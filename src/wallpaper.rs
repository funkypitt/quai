//! The dock's tint, drawn from the wallpaper as Unity's launcher did.

use std::path::{Path, PathBuf};

use crate::{
    apps::{Rgb, hsv_to_rgb, rgb_to_hsv},
    config::home,
};

/// The dock's colour when there is no wallpaper to draw one from.
pub const NEUTRAL: Rgb = Rgb::new(0.07, 0.07, 0.08);

/// Where COSMIC notes the wallpaper shown on each output.
pub fn state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".local/state"))
        .join("cosmic/com.system76.CosmicBackground/v1")
}

#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    Image(PathBuf),
    Color(Rgb),
}

/// Reads COSMIC's list, such as `[("HDMI-A-4", Path("/usr/share/…jpg")),]`.
pub fn parse_state(text: &str) -> Vec<(String, Source)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("(\"") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find('"') else { break };
        let name = rest[..end].to_string();
        rest = &rest[end + 1..];
        let body = rest.trim_start_matches([',', ' ', '\n', '\t']);
        if let Some(p) = body.strip_prefix("Path(\"") {
            let Some(end) = p.find("\")") else { break };
            out.push((name, Source::Image(PathBuf::from(p[..end].replace("\\\"", "\"")))));
            rest = &p[end..];
        } else if let Some(c) = body.strip_prefix("Color(") {
            // A plain colour or a gradient: the mean of the colours named.
            let end = c.find("))),").or_else(|| c.find("\n")).unwrap_or(c.len());
            let nums: Vec<f32> = c[..end]
                .split(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
                .filter(|s| s.contains('.'))
                .filter_map(|s| s.parse().ok())
                .filter(|v| (0.0..=1.0).contains(v))
                .collect();
            let colors: Vec<&[f32]> = nums.chunks_exact(3).collect();
            if !colors.is_empty() {
                let n = colors.len() as f32;
                let sum = colors.iter().fold([0.0; 3], |a, c| [a[0] + c[0], a[1] + c[1], a[2] + c[2]]);
                out.push((name, Source::Color(Rgb::new(sum[0] / n, sum[1] / n, sum[2] / n))));
            }
            rest = &c[end.min(c.len())..];
        }
    }
    out
}

/// The wallpaper of one of `outputs`, or failing that the first one listed.
pub fn current(outputs: &[String]) -> Option<Source> {
    let text = std::fs::read_to_string(state_dir().join("wallpapers")).ok()?;
    let list = parse_state(&text);
    list.iter()
        .find(|(name, _)| outputs.iter().any(|o| o.eq_ignore_ascii_case(name)))
        .or_else(|| list.first())
        .map(|(_, s)| s.clone())
}

/// The mean colour of an image, each pixel counting for more as it is
/// brighter and more saturated: the wallpaper's mood rather than its mud.
fn mean_color(path: &Path) -> Option<Rgb> {
    let img = image::open(path).ok()?.thumbnail(96, 96).to_rgb8();
    let (mut r, mut g, mut b, mut total) = (0f64, 0f64, 0f64, 0f64);
    for p in img.pixels() {
        let [pr, pg, pb] = p.0.map(|c| c as f64 / 255.0);
        let max = pr.max(pg).max(pb);
        let min = pr.min(pg).min(pb);
        let saturation = if max > 0.0 { (max - min) / max } else { 0.0 };
        let weight = 0.1 + 0.9 * saturation * max;
        r += pr * weight;
        g += pg * weight;
        b += pb * weight;
        total += weight;
    }
    (total > 0.0).then(|| Rgb::new((r / total) as f32, (g / total) as f32, (b / total) as f32))
}

/// From a wallpaper's colour to the dock's: same hue, deep enough for the
/// lit tiles to stand out, and grey wallpapers stay grey.
pub fn tint_from(color: Rgb) -> Rgb {
    let (h, s, _) = rgb_to_hsv(color);
    let s = if s < 0.08 { s } else { (s * 1.7).clamp(0.38, 0.62) };
    hsv_to_rgb(h, s, 0.40)
}

pub fn tint_of(source: &Source) -> Option<Rgb> {
    match source {
        Source::Image(path) => mean_color(path).map(tint_from),
        Source::Color(c) => Some(tint_from(*c)),
    }
}

/// `#rrggbb`, as written in the settings.
pub fn parse_hex(text: &str) -> Option<Rgb> {
    let hex = text.trim().strip_prefix('#')?;
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let c = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok().map(|v| v as f32 / 255.0);
    Some(Rgb::new(c(0)?, c(2)?, c(4)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_file() {
        let text = "[\n    (\"HDMI-A-4\", Path(\"/usr/share/backgrounds/cosmic/orion.jpg\")),\n    (\"DP-1\", Color(Single((0.2, 0.4, 0.6)))),\n]";
        let list = parse_state(text);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], ("HDMI-A-4".into(), Source::Image("/usr/share/backgrounds/cosmic/orion.jpg".into())));
        assert_eq!(list[1], ("DP-1".into(), Source::Color(Rgb::new(0.2, 0.4, 0.6))));
        assert!(parse_state("[]").is_empty());
        assert!(parse_state("(\"broken").is_empty());
    }

    #[test]
    fn tints() {
        let (h, s, v) = rgb_to_hsv(tint_from(Rgb::new(0.41, 0.29, 0.29)));
        assert!(h < 2.0 || h > 358.0, "hue kept, got {h}");
        assert!((0.38..=0.62).contains(&s) && (v - 0.40).abs() < 1e-3);
        let (_, s, _) = rgb_to_hsv(tint_from(Rgb::new(0.5, 0.5, 0.5)));
        assert!(s < 0.01, "a grey wallpaper gives a grey dock");
    }

    #[test]
    fn hex() {
        assert_eq!(parse_hex("#ff8000"), Some(Rgb::new(1.0, 128.0 / 255.0, 0.0)));
        assert_eq!(parse_hex("ff8000"), None);
        assert_eq!(parse_hex("#ff80"), None);
        assert_eq!(parse_hex("#ff80zz"), None);
    }
}
