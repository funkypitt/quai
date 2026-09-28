//! Starting applications, each in its own systemd scope so that none of
//! them lives or dies with the dock.

use freedesktop_desktop_entry::DesktopEntry;
use std::{
    path::Path,
    process::{Command, Stdio},
    sync::atomic::{AtomicU32, Ordering},
};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// Launches a desktop entry, or one of its actions.
pub fn launch(entry: &DesktopEntry, action: Option<&str>, token: Option<&str>) -> bool {
    let args = match action {
        Some(a) => entry.parse_exec_action(a),
        None => entry.parse_exec(),
    };
    let mut args = match args {
        Ok(a) if !a.is_empty() => a,
        Ok(_) => return false,
        Err(e) => {
            log::error!("{}: {e}", entry.id());
            return false;
        }
    };
    if entry.terminal() {
        let mut with_term = terminal_prefix();
        with_term.append(&mut args);
        args = with_term;
    }
    run(&args, entry.id(), entry.path(), token)
}

fn terminal_prefix() -> Vec<String> {
    for (term, flag) in [("cosmic-term", "-e"), ("x-terminal-emulator", "-e"), ("gnome-terminal", "--"), ("xterm", "-e")] {
        if in_path(term) {
            return vec![term.to_string(), flag.to_string()];
        }
    }
    Vec::new()
}

pub fn in_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()))
}

/// systemd's escaping for the application id inside a unit name.
fn escape_unit(id: &str) -> String {
    let mut out = String::new();
    for b in id.bytes() {
        if b.is_ascii_alphanumeric() || b == b'_' || b == b'.' {
            out.push(b as char);
        } else {
            out.push_str(&format!("\\x{b:02x}"));
        }
    }
    if out.is_empty() { "app".into() } else { out }
}

/// Runs a command detached from the dock.
pub fn run(args: &[String], id: &str, cwd: Option<&str>, token: Option<&str>) -> bool {
    if args.is_empty() {
        return false;
    }
    let mut cmd = if in_path("systemd-run") {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let unit = format!("app-quai-{}-{}x{}.scope", escape_unit(id), std::process::id(), n);
        let mut c = Command::new("systemd-run");
        c.args(["--user", "--scope", "--collect", "--quiet", "--slice=app.slice"]);
        c.arg(format!("--unit={unit}"));
        c.arg("--").args(args);
        c
    } else {
        use std::os::unix::process::CommandExt;
        let mut c = Command::new(&args[0]);
        c.args(&args[1..]).process_group(0);
        c
    };
    if let Some(dir) = cwd.filter(|d| Path::new(d).is_dir()) {
        cmd.current_dir(dir);
    } else if let Some(home) = std::env::var_os("HOME") {
        cmd.current_dir(home);
    }
    if let Some(t) = token {
        cmd.env("XDG_ACTIVATION_TOKEN", t).env("DESKTOP_STARTUP_ID", t);
    } else {
        cmd.env_remove("XDG_ACTIVATION_TOKEN").env_remove("DESKTOP_STARTUP_ID");
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    match cmd.spawn() {
        Ok(mut child) => {
            let name = args[0].clone();
            // Collect the exit status so that no zombie is left behind.
            std::thread::spawn(move || match child.wait() {
                Ok(s) if !s.success() => log::debug!("{name} ended with {s}"),
                _ => {}
            });
            true
        }
        Err(e) => {
            log::error!("cannot start {}: {e}", args[0]);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_names() {
        assert_eq!(escape_unit("org.telegram.desktop"), "org.telegram.desktop");
        assert_eq!(escape_unit("brave-browser"), "brave\\x2dbrowser");
        assert_eq!(escape_unit("a b/c"), "a\\x20b\\x2fc");
        assert_eq!(escape_unit(""), "app");
    }
}
