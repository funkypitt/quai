//! `quai --doctor`: what the session, the service and the settings look like, in one go,
//! for the day the dock does not show on a machine.

use std::process::Command;

fn systemctl(args: &[&str]) -> String {
    Command::new("systemctl")
        .arg("--user")
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "systemctl not found".into())
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| "(unset)".into())
}

pub fn run() {
    let desktop = env("XDG_CURRENT_DESKTOP");
    println!("session");
    println!("  XDG_CURRENT_DESKTOP  {desktop}{}", if desktop.contains("COSMIC") { "" } else { "   <- Quai needs COSMIC (layer shell + its window protocols)" });
    println!("  WAYLAND_DISPLAY      {}", env("WAYLAND_DISPLAY"));
    println!("  user                 {}{}", env("USER"), if rustix::process::getuid().is_root() { "   <- installed as root: run install.sh as yourself, without sudo" } else { "" });
    println!("startup (systemd --user)");
    let target = systemctl(&["is-active", "cosmic-session.target"]);
    println!("  cosmic-session.target  {target}{}", if target == "active" { "" } else { "   <- the session does not start this target: the service has nothing to hang on" });
    println!("  quai.service enabled   {}", systemctl(&["is-enabled", "quai.service"]));
    println!("  quai.service active    {}", systemctl(&["is-active", "quai.service"]));
    let unit = systemctl(&["show", "quai.service", "-p", "FragmentPath", "--value"]);
    println!("  unit file              {}", if unit.is_empty() { "(none)".to_string() } else { unit });
    println!("  binary                 {}", std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default());
    println!("settings");
    println!("  config                 {}", crate::config::config_dir().join("config.toml").display());
    println!("  COSMIC dock            {}", if crate::cosmicdock::is_on() { "on (switched off when Quai starts, unless hide_cosmic_dock = false)" } else { "off" });
    println!("\nStart by hand: systemctl --user start quai   (log: journalctl --user -u quai)");
}
