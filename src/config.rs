//! User configuration, stored in `~/.config/quai/config.toml`.

use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backlight {
    /// Every tile is lit with its icon colour (Unity's default).
    Always,
    /// Only running applications are lit.
    Running,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClickActive {
    /// Clicking the focused application minimizes it (or cycles its windows).
    Minimize,
    /// Clicking the focused application only cycles its windows.
    Cycle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Desktop file ids of the pinned applications, in order.
    pub pinned: Vec<String>,
    /// Right column: one tile per application (true) or per window (false).
    pub group_windows: bool,
    pub backlight: Backlight,
    pub click_active: ClickActive,
    /// Show the applications and workspaces buttons on the first row.
    pub buttons: bool,
    /// Opacity of the dock background, 0.0 to 1.0.
    pub opacity: f32,
    /// Colour of the dock: "wallpaper" (drawn from it, as Unity did),
    /// "none" (near black) or a colour such as "#402030".
    pub tint: String,
    /// Blur what lies behind the dock, where the compositor can.
    pub blur: bool,
    /// "all", or the name of one output (for example "HDMI-A-4").
    pub output: String,
    /// Icon theme; empty means the desktop's theme.
    pub icon_theme: String,
    /// Delay before a tooltip shows, in milliseconds.
    pub tooltip_delay_ms: u64,
    /// Switch COSMIC's own dock off when Quai starts (`quai --cosmic-dock on` gives it back).
    pub hide_cosmic_dock: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            pinned: Vec::new(),
            group_windows: true,
            backlight: Backlight::Always,
            click_active: ClickActive::Minimize,
            buttons: true,
            opacity: 0.27,
            tint: "wallpaper".into(),
            blur: true,
            output: "all".into(),
            icon_theme: String::new(),
            tooltip_delay_ms: 350,
            hide_cosmic_dock: true,
        }
    }
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".config"))
        .join("quai")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

const HEADER: &str = "\
# Quai — two-column Unity-style dock.
# This file is rewritten when you pin, unpin or reorder from the dock;
# edits made here are picked up at once.
#
# pinned           desktop file ids, top to bottom
# group_windows    right column: one tile per application (true) or per window (false)
# backlight        \"always\" (Unity's default) or \"running\"
# click_active     \"minimize\" or \"cycle\"
# buttons          applications and workspaces buttons on the first row
# opacity          dock background, 0.0 to 1.0
# tint             \"wallpaper\" (colour drawn from the wallpaper), \"none\", or \"#rrggbb\"
# blur             blur what lies behind the dock
# output           \"all\" or an output name such as \"HDMI-A-4\"
# icon_theme       empty = the desktop's theme
# hide_cosmic_dock switch COSMIC's own dock off when Quai starts (quai --cosmic-dock on gives it back)

";

impl Config {
    /// Loads the configuration; on first run, imports COSMIC's pinned apps.
    pub fn load_or_init() -> Self {
        let path = config_path();
        match fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<Config>(&text) {
                Ok(c) => c.sanitized(),
                Err(e) => {
                    log::error!("{}: {e}; using defaults", path.display());
                    Config::default()
                }
            },
            Err(_) => {
                let c = Config { pinned: cosmic_favorites(), ..Config::default() };
                if let Err(e) = c.save() {
                    log::warn!("cannot write {}: {e}", path.display());
                }
                c
            }
        }
    }

    /// Reloads from disk; `None` when the file is missing or invalid.
    pub fn reload() -> Option<Self> {
        let text = fs::read_to_string(config_path()).ok()?;
        toml::from_str::<Config>(&text).ok().map(Config::sanitized)
    }

    fn sanitized(mut self) -> Self {
        self.opacity = self.opacity.clamp(0.0, 1.0);
        let mut seen = std::collections::HashSet::new();
        self.pinned.retain(|p| !p.is_empty() && seen.insert(p.to_ascii_lowercase()));
        self
    }

    pub fn save(&self) -> std::io::Result<()> {
        let body = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        let dir = config_dir();
        fs::create_dir_all(&dir)?;
        let tmp = dir.join("config.toml.tmp");
        fs::write(&tmp, format!("{HEADER}{body}"))?;
        fs::rename(tmp, config_path())
    }
}

/// Reads the pinned applications of COSMIC's own dock (a RON list of strings).
pub fn cosmic_favorites() -> Vec<String> {
    let path = home().join(".config/cosmic/com.system76.CosmicAppList/v1/favorites");
    let Ok(text) = fs::read_to_string(path) else { return Vec::new() };
    parse_ron_string_list(&text)
}

fn parse_ron_string_list(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut s = String::new();
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    if let Some(n) = chars.next() {
                        s.push(n)
                    }
                }
                '"' => break,
                _ => s.push(c),
            }
        }
        out.push(s);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ron_list() {
        let v = parse_ron_string_list("[\n    \"org.telegram.desktop\",\n    \"a\\\"b\",\n]");
        assert_eq!(v, vec!["org.telegram.desktop".to_string(), "a\"b".to_string()]);
        assert!(parse_ron_string_list("[]").is_empty());
    }

    #[test]
    fn roundtrip_and_defaults() {
        let c: Config = toml::from_str("pinned = [\"a\", \"A\", \"b\"]\nopacity = 3.0").unwrap();
        let c = c.sanitized();
        assert_eq!(c.pinned, vec!["a", "b"]);
        assert_eq!(c.opacity, 1.0);
        assert!(c.group_windows);
        let text = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back, c);
    }
}
