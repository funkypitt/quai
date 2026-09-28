//! Desktop entries, window → application matching, icons and tile colours.

use freedesktop_desktop_entry as fde;
use fde::{DesktopEntry, unicase::Ascii};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
};
use tiny_skia::Pixmap;

use crate::config::home;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgb {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Rgb {
    pub const fn new(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b }
    }
    pub fn scaled(self, k: f32) -> Self {
        Self::new((self.r * k).clamp(0.0, 1.0), (self.g * k).clamp(0.0, 1.0), (self.b * k).clamp(0.0, 1.0))
    }
    pub fn mix(self, o: Rgb, t: f32) -> Self {
        Self::new(self.r + (o.r - self.r) * t, self.g + (o.g - self.g) * t, self.b + (o.b - self.b) * t)
    }
    pub fn color(self, a: f32) -> tiny_skia::Color {
        tiny_skia::Color::from_rgba(self.r, self.g, self.b, a.clamp(0.0, 1.0))
            .unwrap_or(tiny_skia::Color::BLACK)
    }
}

pub struct DesktopAction {
    pub id: String,
    pub name: String,
}

pub struct App {
    /// Desktop file id, or the window's app id when no desktop file matches.
    pub key: String,
    pub name: String,
    pub entry: Option<DesktopEntry>,
    pub actions: Vec<DesktopAction>,
    pub tile: Rgb,
    pub glow: Rgb,
    /// Share of the 48 px of Unity's icon that this icon is drawn at: icons
    /// that fill their whole canvas are drawn smaller, so that the lit tile
    /// shows around them as it did around Unity's own icons.
    pub fit: f32,
    icon_source: Option<PathBuf>,
    icons: RefCell<HashMap<u32, Rc<Pixmap>>>,
}

impl App {
    pub fn can_launch(&self) -> bool {
        self.entry.as_ref().is_some_and(|e| e.exec().is_some())
    }

    /// The icon rendered at `px` device pixels.
    pub fn icon(&self, px: u32) -> Rc<Pixmap> {
        if let Some(p) = self.icons.borrow().get(&px) {
            return p.clone();
        }
        let pix = self
            .icon_source
            .as_deref()
            .and_then(|p| render_icon_file(p, px))
            .unwrap_or_else(|| letter_icon(&self.name, px));
        let pix = Rc::new(pix);
        self.icons.borrow_mut().insert(px, pix.clone());
        pix
    }
}

pub struct AppDb {
    entries: Vec<DesktopEntry>,
    locales: Vec<String>,
    themes: Vec<String>,
    apps: HashMap<String, Rc<App>>,
    resolved: HashMap<String, String>,
}

impl AppDb {
    pub fn new(configured_theme: &str) -> Self {
        let mut db = Self {
            entries: Vec::new(),
            locales: fde::get_languages_from_env(),
            themes: icon_themes(configured_theme),
            apps: HashMap::new(),
            resolved: HashMap::new(),
        };
        db.reload();
        db
    }

    pub fn set_theme(&mut self, configured_theme: &str) {
        self.themes = icon_themes(configured_theme);
        self.apps.clear();
    }

    /// Re-reads every desktop file and forgets cached matches.
    pub fn reload(&mut self) {
        let mut seen = HashSet::new();
        self.entries = fde::Iter::new(application_dirs().into_iter())
            .filter_map(|p| DesktopEntry::from_path(p, Some(&self.locales)).ok())
            .filter(|e| e.type_().is_none_or(|t| t == "Application"))
            .filter(|e| seen.insert(e.id().to_ascii_lowercase()))
            .collect();
        // Entries meant to be shown come first, so they win over hidden helpers.
        self.entries.sort_by_key(|e| e.no_display() || e.hidden());
        self.apps.clear();
        self.resolved.clear();
    }

    pub fn application_dirs(&self) -> Vec<PathBuf> {
        application_dirs()
    }

    /// The application key for a window.
    pub fn resolve(&mut self, app_id: &str, title: &str) -> String {
        if app_id.is_empty() {
            return if title.is_empty() { "unknown".into() } else { format!("title:{title}") };
        }
        let special = app_id == "steam_app_default" || app_id.ends_with(".exe");
        if !special && let Some(k) = self.resolved.get(app_id) {
            return k.clone();
        }
        let key = match_entry(&self.entries, app_id, title, &self.locales)
            .map(|e| e.id().to_string())
            .unwrap_or_else(|| app_id.to_string());
        if !special {
            self.resolved.insert(app_id.to_string(), key.clone());
        }
        key
    }

    /// The canonical key for a pinned id (fixes case, resolves aliases).
    pub fn canonical(&mut self, id: &str) -> String {
        if let Some(e) = self.entries.iter().find(|e| e.id().eq_ignore_ascii_case(id)) {
            return e.id().to_string();
        }
        self.resolve(id, "")
    }

    pub fn app(&mut self, key: &str) -> Rc<App> {
        if let Some(a) = self.apps.get(key) {
            return a.clone();
        }
        let entry = self.entries.iter().find(|e| e.id() == key).cloned();
        let app = Rc::new(self.build(key, entry));
        self.apps.insert(key.to_string(), app.clone());
        app
    }

    /// A pseudo application for one of the dock's own buttons.
    pub fn builtin(&mut self, key: &str, desktop_id: &str, fallback_name: &str) -> Rc<App> {
        if let Some(a) = self.apps.get(key) {
            return a.clone();
        }
        let entry = self.entries.iter().find(|e| e.id() == desktop_id).cloned();
        let mut app = self.build(key, entry);
        if app.entry.is_none() {
            app.name = fallback_name.to_string();
        }
        let app = Rc::new(app);
        self.apps.insert(key.to_string(), app.clone());
        app
    }

    fn build(&self, key: &str, entry: Option<DesktopEntry>) -> App {
        let name = entry
            .as_ref()
            .and_then(|e| e.name(&self.locales).map(|n| n.into_owned()))
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| pretty_name(key));
        let icon_name = entry.as_ref().and_then(|e| e.icon()).unwrap_or(key).to_string();
        let icon_source = find_icon(&icon_name, &self.themes).or_else(|| {
            // A window without a desktop file may still have a themed icon.
            (icon_name != key).then(|| find_icon(key, &self.themes)).flatten()
        });
        let actions = entry
            .as_ref()
            .map(|e| {
                e.actions()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|a| !a.is_empty() && e.action_exec(a).is_some())
                    .filter_map(|a| {
                        let name = e.action_name(a, &self.locales)?.into_owned();
                        Some(DesktopAction { id: a.to_string(), name })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let sample = icon_source
            .as_deref()
            .and_then(|p| render_icon_file(p, 48))
            .unwrap_or_else(|| letter_icon(&name, 48));
        let (tile, glow) = tile_colors(&sample);
        let fit = icon_fit(&sample);
        log::debug!("{key}: icon {:?}, fit {fit:.2}", icon_source);
        let icons = RefCell::new(HashMap::new());
        if icon_source.is_none() {
            icons.borrow_mut().insert(48, Rc::new(sample));
        }
        App { key: key.to_string(), name, entry, actions, tile, glow, fit, icon_source, icons }
    }
}

fn pretty_name(key: &str) -> String {
    let base = key.strip_prefix("title:").unwrap_or(key);
    let last = if base.contains(' ') { base } else { base.rsplit('.').next().unwrap_or(base) };
    let mut c = last.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => "?".into(),
    }
}

fn application_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = fde::default_paths().collect();
    let h = home();
    for extra in [
        h.join(".local/share/applications"),
        h.join(".local/share/flatpak/exports/share/applications"),
        PathBuf::from("/var/lib/flatpak/exports/share/applications"),
        PathBuf::from("/var/lib/snapd/desktop/applications"),
        PathBuf::from("/usr/local/share/applications"),
        PathBuf::from("/usr/share/applications"),
    ] {
        if !dirs.contains(&extra) {
            dirs.push(extra);
        }
    }
    dirs
}

/// Finds the desktop entry of a window, from the most to the least certain clue.
pub fn match_entry<'a>(
    entries: &'a [DesktopEntry],
    app_id: &str,
    title: &str,
    locales: &[String],
) -> Option<&'a DesktopEntry> {
    // Some windows name their desktop file, extension included; but an id
    // may also end in ".desktop" of its own right (org.telegram.desktop).
    let stripped = app_id.strip_suffix(".desktop").filter(|s| !s.is_empty());
    let ids = || std::iter::once(app_id).chain(stripped);
    // An exact file name is the strongest clue.
    let found = ids()
        .find_map(|id| entries.iter().find(|e| e.id().eq_ignore_ascii_case(id)))
        .or_else(|| ids().find_map(|id| match_id(entries, id)));
    if found.is_some() {
        return found;
    }
    // Wine and Proton windows only carry the game's title.
    if app_id == "steam_app_default" || app_id.ends_with(".exe") {
        return entries.iter().find(|e| {
            e.name(locales).is_some_and(|n| n == title)
                && (app_id != "steam_app_default"
                    || e.categories().unwrap_or_default().contains(&"Game"))
        });
    }
    None
}

/// Last parts of an id that say nothing about the application.
const GENERIC: [&str; 7] = ["desktop", "app", "application", "client", "gui", "main", "bin"];

fn match_id<'a>(entries: &'a [DesktopEntry], id: &str) -> Option<&'a DesktopEntry> {
    if let Some(e) = fde::find_app_by_id(entries, Ascii::new(id)) {
        return Some(e);
    }
    // The executable's name, whatever its directory.
    let by_exec = entries.iter().find(|e| {
        exec_program(e).is_some_and(|p| {
            Path::new(&p).file_name().and_then(|n| n.to_str()).is_some_and(|n| n.eq_ignore_ascii_case(id))
        })
    });
    if by_exec.is_some() {
        return by_exec;
    }
    // "Some App" against some-app.desktop, and the last part of a dotted id.
    let dashed = id.replace([' ', '_'], "-");
    let last = id.rsplit('.').next().unwrap_or(id);
    let telling = |part: &str| part.len() > 2 && !GENERIC.iter().any(|g| g.eq_ignore_ascii_case(part));
    entries.iter().find(|e| {
        let eid = e.id();
        let elast = eid.rsplit('.').next().unwrap_or(eid);
        eid.replace([' ', '_'], "-").eq_ignore_ascii_case(&dashed)
            || (telling(elast) && (elast.eq_ignore_ascii_case(id) || elast.eq_ignore_ascii_case(last)))
    })
}

/// First word of Exec that is a program, skipping `env VAR=x` prefixes.
fn exec_program(e: &DesktopEntry) -> Option<String> {
    let args = e.parse_exec().ok()?;
    let mut it = args.into_iter().peekable();
    if it.peek().is_some_and(|a| a == "env" || a.ends_with("/env")) {
        it.next();
        while it.peek().is_some_and(|a| a.contains('=') || a.starts_with('-')) {
            it.next();
        }
    }
    it.next()
}

// ---------------------------------------------------------------- icons

fn icon_themes(configured: &str) -> Vec<String> {
    let mut themes = Vec::new();
    let mut push = |t: String| {
        let t = t.trim().trim_matches('"').to_string();
        if !t.is_empty() && !themes.contains(&t) {
            themes.push(t);
        }
    };
    push(configured.to_string());
    if let Ok(t) = std::fs::read_to_string(home().join(".config/cosmic/com.system76.CosmicTk/v1/icon_theme")) {
        push(t);
    }
    for t in ["Cosmic", "Pop", "Adwaita", "hicolor"] {
        push(t.to_string());
    }
    themes
}

fn icon_bases() -> Vec<PathBuf> {
    let h = home();
    let mut bases = vec![h.join(".local/share/icons"), h.join(".icons")];
    if let Some(dirs) = std::env::var_os("XDG_DATA_DIRS") {
        bases.extend(std::env::split_paths(&dirs).map(|d| d.join("icons")));
    }
    bases.extend([
        h.join(".local/share/flatpak/exports/share/icons"),
        PathBuf::from("/var/lib/flatpak/exports/share/icons"),
        PathBuf::from("/var/lib/snapd/desktop/icons"),
        PathBuf::from("/usr/local/share/icons"),
        PathBuf::from("/usr/share/icons"),
    ]);
    let mut seen = HashSet::new();
    bases.retain(|b| seen.insert(b.clone()));
    bases
}

const ICON_EXTS: [&str; 5] = ["svg", "png", "webp", "jpg", "jpeg"];

pub fn find_icon(name: &str, themes: &[String]) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    let as_path = Path::new(name);
    if as_path.is_absolute() {
        return as_path.is_file().then(|| as_path.to_path_buf());
    }
    // Some desktop files name the icon with its extension.
    let stem = ICON_EXTS
        .iter()
        .find_map(|ext| name.strip_suffix(&format!(".{ext}")))
        .unwrap_or(name);

    for theme in themes {
        let found = freedesktop_icons::lookup(stem).with_size(64).with_theme(theme).with_cache().find();
        if let Some(p) = found.filter(|p| is_supported(p)) {
            // The lookup may settle for a tiny bitmap; look for a better one.
            return Some(best_in_hicolor(stem).filter(|_| is_small_bitmap(&p)).unwrap_or(p));
        }
    }
    best_in_hicolor(stem).or_else(|| {
        ICON_EXTS.iter().find_map(|ext| {
            let p = PathBuf::from(format!("/usr/share/pixmaps/{stem}.{ext}"));
            p.is_file().then_some(p)
        })
    })
}

fn is_supported(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| ICON_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

fn is_small_bitmap(p: &Path) -> bool {
    if p.extension().is_some_and(|e| e == "svg") {
        return false;
    }
    image::image_dimensions(p).map(|(w, _)| w < 48).unwrap_or(true)
}

/// Searches every hicolor tree (system, user, Flatpak, Snap) for the largest icon.
fn best_in_hicolor(stem: &str) -> Option<PathBuf> {
    const SIZES: [&str; 9] =
        ["scalable", "512x512", "256x256", "192x192", "128x128", "96x96", "72x72", "64x64", "48x48"];
    for size in SIZES {
        for base in icon_bases() {
            for ext in ICON_EXTS {
                let p = base.join("hicolor").join(size).join("apps").join(format!("{stem}.{ext}"));
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// Renders an icon file into a square pixmap of `px` pixels.
pub fn render_icon_file(path: &Path, px: u32) -> Option<Pixmap> {
    let px = px.max(1);
    let is_svg = path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("svg"));
    if is_svg {
        let data = std::fs::read(path).ok()?;
        let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).ok()?;
        let size = tree.size();
        let (w, h) = (size.width(), size.height());
        if w <= 0.0 || h <= 0.0 {
            return None;
        }
        let k = px as f32 / w.max(h);
        let mut pix = Pixmap::new(px, px)?;
        let tf = tiny_skia::Transform::from_scale(k, k)
            .post_translate((px as f32 - w * k) / 2.0, (px as f32 - h * k) / 2.0);
        resvg::render(&tree, tf, &mut pix.as_mut());
        return Some(pix);
    }
    let img = image::open(path).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    let k = px as f32 / w.max(h) as f32;
    let (nw, nh) = (((w as f32 * k).round() as u32).clamp(1, px), ((h as f32 * k).round() as u32).clamp(1, px));
    let scaled = if (nw, nh) == (w, h) {
        img
    } else {
        image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Lanczos3)
    };
    let mut pix = Pixmap::new(px, px)?;
    let (ox, oy) = ((px - nw) / 2, (px - nh) / 2);
    let stride = px as usize * 4;
    let data = pix.data_mut();
    for (x, y, p) in scaled.enumerate_pixels() {
        let [r, g, b, a] = p.0;
        let i = (y + oy) as usize * stride + (x + ox) as usize * 4;
        let pre = |c: u8| ((c as u32 * a as u32 + 127) / 255) as u8;
        data[i..i + 4].copy_from_slice(&[pre(r), pre(g), pre(b), a]);
    }
    Some(pix)
}

/// Stand-in for applications without any icon: an initial on a coloured disc.
fn letter_icon(name: &str, px: u32) -> Pixmap {
    let px = px.max(8);
    let mut pix = Pixmap::new(px, px).expect("icon size");
    let hue = name.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32)) % 360;
    let c = hsv_to_rgb(hue as f32, 0.55, 0.80);
    let mut paint = tiny_skia::Paint::default();
    paint.anti_alias = true;
    paint.set_color(c.color(1.0));
    let r = px as f32 / 2.0;
    if let Some(path) = tiny_skia::PathBuilder::from_circle(r, r, r * 0.84) {
        pix.fill_path(&path, &paint, tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
    }
    let letter = name.chars().find(|c| c.is_alphanumeric()).unwrap_or('?').to_uppercase().to_string();
    crate::text::with_text(|t| {
        let size = px as f32 * 0.5;
        let (w, _) = t.measure(&letter, size, true, None);
        t.draw(&mut pix, &letter, (px as f32 - w) / 2.0, px as f32 * 0.5 - size * 0.62, size, true, Rgb::new(1.0, 1.0, 1.0), 1.0, None);
    });
    pix
}

// --------------------------------------------------------------- colours

pub fn rgb_to_hsv(c: Rgb) -> (f32, f32, f32) {
    let max = c.r.max(c.g).max(c.b);
    let min = c.r.min(c.g).min(c.b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == c.r {
        60.0 * (((c.g - c.b) / d).rem_euclid(6.0))
    } else if max == c.g {
        60.0 * ((c.b - c.r) / d + 2.0)
    } else {
        60.0 * ((c.r - c.g) / d + 4.0)
    };
    let s = if max == 0.0 { 0.0 } else { d / max };
    (h, s, max)
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Rgb {
    let h = h.rem_euclid(360.0);
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    Rgb::new(r + m, g + m, b + m)
}

/// How much of its canvas an icon covers, turned into a drawing size:
/// 1.0 (48 px) for an icon with air around it, down to 40 px for one that
/// fills its square.
pub fn icon_fit(icon: &Pixmap) -> f32 {
    let total = icon.pixels().len().max(1) as f32;
    let solid = icon.pixels().iter().filter(|p| p.alpha() > 170).count() as f32;
    let coverage = solid / total;
    let t = ((coverage - 0.62) / (0.86 - 0.62)).clamp(0.0, 1.0);
    1.0 - t * (8.0 / 48.0)
}

/// Unity's tile colour: the icon's average colour, each pixel weighted by its
/// opacity and saturation, then normalised to a fixed saturation and value.
pub fn tile_colors(icon: &Pixmap) -> (Rgb, Rgb) {
    let (mut rt, mut gt, mut bt, mut total) = (0f64, 0f64, 0f64, 0f64);
    for p in icon.pixels() {
        let a = p.alpha() as f32;
        if a == 0.0 {
            continue;
        }
        let (r, g, b) = (
            p.red() as f32 * 255.0 / a,
            p.green() as f32 * 255.0 / a,
            p.blue() as f32 * 255.0 / a,
        );
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let saturation = if max > 0.0 { (max - min) / max } else { 0.0 };
        let relevance = (0.1 + 0.9 * (a / 255.0) * saturation) as f64;
        rt += r as f64 * relevance;
        gt += g as f64 * relevance;
        bt += b as f64 * relevance;
        total += relevance * 255.0;
    }
    if total <= 0.0 {
        let grey = Rgb::new(0.55, 0.55, 0.55);
        return (grey, Rgb::new(0.9, 0.9, 0.9));
    }
    let avg = Rgb::new((rt / total) as f32, (gt / total) as f32, (bt / total) as f32);
    let (h, s, _) = rgb_to_hsv(avg);
    let s = if s > 0.15 { 0.65 } else { s };
    (hsv_to_rgb(h, s, 0.90), hsv_to_rgb(h, (s * 0.85).min(0.55), 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, body: &str) -> DesktopEntry {
        DesktopEntry::from_str(path, &format!("[Desktop Entry]\nType=Application\n{body}"), None::<&[&str]>)
            .unwrap()
    }

    fn sample() -> Vec<DesktopEntry> {
        vec![
            entry("/usr/share/applications/org.telegram.desktop.desktop", "Name=Telegram\nExec=telegram-desktop -- %u\nStartupWMClass=TelegramDesktop"),
            entry("/usr/share/applications/brave-browser.desktop", "Name=Brave\nExec=/usr/bin/brave-browser-stable %U\nStartupWMClass=brave-browser"),
            entry("/usr/share/applications/brave-abcdef-Default.desktop", "Name=Web App\nExec=/opt/brave.com/brave/brave-browser --app-id=abcdef\nStartupWMClass=crx_abcdef"),
            entry("/usr/share/applications/com.system76.CosmicFiles.desktop", "Name=COSMIC Files\nExec=cosmic-files %U"),
            entry("/usr/share/applications/mailbot-app.desktop", "Name=Mailbot\nExec=env GDK_BACKEND=x11 /opt/mailbot/mailbot"),
            entry("/usr/share/applications/game.desktop", "Name=Great Game\nExec=steam steam://run/1\nCategories=Game;"),
        ]
    }

    fn m(app_id: &str, title: &str) -> Option<String> {
        let e = sample();
        match_entry(&e, app_id, title, &[]).map(|e| e.id().to_string())
    }

    #[test]
    fn matching() {
        assert_eq!(m("org.telegram.desktop", "").as_deref(), Some("org.telegram.desktop"));
        assert_eq!(m("TelegramDesktop", "").as_deref(), Some("org.telegram.desktop"));
        assert_eq!(m("telegram-desktop", "").as_deref(), Some("org.telegram.desktop"));
        assert_eq!(m("Brave-browser", "").as_deref(), Some("brave-browser"));
        assert_eq!(m("brave-abcdef-Default", "").as_deref(), Some("brave-abcdef-Default"));
        assert_eq!(m("crx_abcdef", "").as_deref(), Some("brave-abcdef-Default"));
        assert_eq!(m("cosmic-files", "").as_deref(), Some("com.system76.CosmicFiles"));
        assert_eq!(m("com.system76.CosmicFiles.desktop", "").as_deref(), Some("com.system76.CosmicFiles"));
        assert_eq!(m("mailbot", "").as_deref(), Some("mailbot-app"));
        assert_eq!(m("steam_app_default", "Great Game").as_deref(), Some("game"));
        assert_eq!(m("nothing-like-it", ""), None);
        assert_eq!(m("other.vendor.desktop", ""), None, "a generic last part proves nothing");
        assert_eq!(m("desktop", ""), None);
    }

    #[test]
    fn hsv_roundtrip() {
        for c in [Rgb::new(0.9, 0.3, 0.1), Rgb::new(0.1, 0.5, 0.9), Rgb::new(0.4, 0.4, 0.4)] {
            let (h, s, v) = rgb_to_hsv(c);
            let back = hsv_to_rgb(h, s, v);
            assert!((back.r - c.r).abs() < 1e-4 && (back.g - c.g).abs() < 1e-4 && (back.b - c.b).abs() < 1e-4);
        }
    }

    #[test]
    fn tile_color_follows_the_icon() {
        let mut pix = Pixmap::new(8, 8).unwrap();
        pix.fill(tiny_skia::Color::from_rgba8(230, 90, 20, 255));
        let (tile, _) = tile_colors(&pix);
        let (h, s, v) = rgb_to_hsv(tile);
        assert!((h - 20.0).abs() < 4.0, "hue {h}");
        assert!((s - 0.65).abs() < 1e-3 && (v - 0.90).abs() < 1e-3);

        // A grey icon stays grey instead of taking a made-up hue.
        pix.fill(tiny_skia::Color::from_rgba8(120, 120, 120, 255));
        let (_, s, _) = rgb_to_hsv(tile_colors(&pix).0);
        assert!(s < 0.05);

        // A transparent icon must not divide by zero.
        let empty = Pixmap::new(8, 8).unwrap();
        let (tile, _) = tile_colors(&empty);
        assert!(tile.r.is_finite());
    }

    #[test]
    fn full_icons_are_drawn_smaller() {
        let mut full = Pixmap::new(48, 48).unwrap();
        full.fill(tiny_skia::Color::from_rgba8(10, 120, 200, 255));
        assert!((icon_fit(&full) - 40.0 / 48.0).abs() < 1e-3);
        let airy = letter_free_disc(48, 0.30);
        assert_eq!(icon_fit(&airy), 1.0);
    }

    fn letter_free_disc(px: u32, radius: f32) -> Pixmap {
        let mut pix = Pixmap::new(px, px).unwrap();
        let mut paint = tiny_skia::Paint::default();
        paint.set_color(tiny_skia::Color::WHITE);
        let c = px as f32 / 2.0;
        let path = tiny_skia::PathBuilder::from_circle(c, c, px as f32 * radius).unwrap();
        pix.fill_path(&path, &paint, tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
        pix
    }

    #[test]
    fn names() {
        assert_eq!(pretty_name("org.gnome.nautilus"), "Nautilus");
        assert_eq!(pretty_name("title:My window"), "My window");
    }
}
