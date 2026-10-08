//! COSMIC's own dock: switched off when Quai takes its place, given back on request.
//! The panel list lives in `~/.config/cosmic/com.system76.CosmicPanel/v1/entries`
//! (a RON list such as `["Panel", "Dock"]`); COSMIC's panel reloads it by itself.

use anyhow::{Context, Result};
use std::{fs, path::PathBuf};

fn entries() -> PathBuf {
    crate::config::home().join(".config/cosmic/com.system76.CosmicPanel/v1/entries")
}

/// True when COSMIC's dock is in the panel list.
pub fn is_on() -> bool {
    fs::read_to_string(entries()).is_ok_and(|t| t.contains("\"Dock\""))
}

/// Puts COSMIC's dock in the panel list, or takes it out. The list as it was the first
/// time is kept next to it (`entries.before-quai`) for `uninstall.sh`.
pub fn set(on: bool) -> Result<()> {
    let path = entries();
    let text = fs::read_to_string(&path).with_context(|| format!("COSMIC panel settings not found: {}", path.display()))?;
    let backup = path.with_file_name("entries.before-quai");
    if !backup.exists() {
        fs::write(&backup, &text)?;
    }
    let new = rewrite(&text, on);
    if new != text {
        fs::write(&path, &new)?;
    }
    log::info!("COSMIC dock switched {}", if on { "on" } else { "off" });
    Ok(())
}

/// The list with or without the `"Dock"` line; the other entries stay as they are.
fn rewrite(text: &str, on: bool) -> String {
    let has = text.contains("\"Dock\"");
    if on == has {
        return text.to_string();
    }
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    if on {
        let close = lines.iter().rposition(|l| l.trim_start().starts_with(']')).unwrap_or(lines.len());
        lines.insert(close, "    \"Dock\",".to_string());
    } else {
        lines.retain(|l| !l.contains("\"Dock\""));
    }
    let mut out = lines.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// At start, with `hide_cosmic_dock` set: COSMIC's dock goes away so that only Quai shows.
pub fn hide_if_wanted(config: &crate::config::Config) {
    if config.hide_cosmic_dock
        && is_on()
        && let Err(e) = set(false)
    {
        log::warn!("cannot switch COSMIC's dock off: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::rewrite;

    #[test]
    fn dock_line_comes_and_goes() {
        let both = "[\n    \"Panel\",\n    \"Dock\",\n]";
        let panel = "[\n    \"Panel\",\n]";
        assert_eq!(rewrite(both, false), panel);
        assert_eq!(rewrite(panel, true), both);
        assert_eq!(rewrite(both, true), both);
        assert_eq!(rewrite(panel, false), panel);
        // other panels stay
        let three = "[\n    \"Panel\",\n    \"Dock\",\n    \"Side\",\n]";
        assert_eq!(rewrite(three, false), "[\n    \"Panel\",\n    \"Side\",\n]");
    }
}
