//! Quai — a two-column dock in the manner of Ubuntu's Unity, for COSMIC.
//!
//! Left column: the pinned applications. Right column: what is open and not
//! pinned. Every tile is lit with the colour of its icon.

mod apps;
mod config;
mod dock;
mod i18n;
mod launch;
mod model;
mod preview;
mod render;
mod testpanel;
mod text;
mod wallpaper;
mod wayland;
mod windows;

use anyhow::{Result, bail};
use std::{fs::File, path::PathBuf};

const HELP: &str = "\
quai — two-column Unity-style dock for COSMIC

Usage:
  quai                      run the dock
  quai --preview FILE.png   draw the dock into an image, without a display
        [--height N] [--scale S]
  quai --windows            list the open windows and the application of each
  quai --version
  quai --help

Settings: ~/.config/quai/config.toml (also by right-clicking the dock).
Set QUAI_LOG=debug for more messages.";

struct Logger(log::LevelFilter);

impl log::Log for Logger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= self.0 && m.target().starts_with("quai")
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            eprintln!("quai: {}: {}", r.level().as_str().to_lowercase(), r.args());
        }
    }
    fn flush(&self) {}
}

/// Holds a lock for as long as the dock runs, so that there is only one.
fn single_instance() -> Result<File> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let display = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".into());
    let path = dir.join(format!("quai-{}.lock", display.replace('/', "_")));
    let file = File::options().create(true).truncate(false).write(true).open(&path)?;
    if rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive).is_err() {
        bail!("the dock is already running");
    }
    Ok(file)
}

fn main() -> Result<()> {
    let level = match std::env::var("QUAI_LOG").as_deref() {
        Ok("debug") => log::LevelFilter::Debug,
        Ok("error") => log::LevelFilter::Error,
        _ => log::LevelFilter::Info,
    };
    let _ = log::set_boxed_logger(Box::new(Logger(level))).map(|()| log::set_max_level(level));

    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    match args.first().map(String::as_str) {
        None => {}
        Some("--help" | "-h") => {
            println!("{HELP}");
            return Ok(());
        }
        Some("--version" | "-V") => {
            println!("quai {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("--preview") => {
            let Some(out) = value("--preview") else { bail!("--preview needs a file name") };
            let height = value("--height").and_then(|v| v.parse().ok()).unwrap_or(1048.0);
            let scale = value("--scale").and_then(|v| v.parse().ok()).unwrap_or(1.0);
            return preview::write(&PathBuf::from(out), height, scale);
        }
        Some("--windows") => return windows::run(),
        Some("--test-panel") => {
            let seconds = value("--test-panel").and_then(|v| v.parse().ok()).unwrap_or(10);
            return testpanel::run(seconds);
        }
        Some(other) => bail!("unknown argument {other}\n\n{HELP}"),
    }

    let _lock = single_instance()?;
    wayland::run(config::Config::load_or_init())
}
