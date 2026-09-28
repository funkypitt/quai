//! Draws the dock into an image, without any display: the pinned
//! applications of the settings, with a plausible set of open windows.

use anyhow::{Context, Result};
use std::path::Path;
use tiny_skia::{Color, Pixmap, PixmapPaint, Transform};

use crate::{
    apps::AppDb,
    config::{Backlight, Config, cosmic_favorites},
    i18n::{Msg, close_all, tr},
    wallpaper,
    render::{
        self, COLUMNS, DOCK_W, HEADER_H, MenuRow, PITCH, Scene, TILE, TOP_PAD, TileDraw, tile_x,
    },
};

pub fn write(out: &Path, height: f32, scale: f32) -> Result<()> {
    let config = Config::reload()
        .unwrap_or_else(|| Config { pinned: cosmic_favorites(), ..Config::default() });
    let mut db = AppDb::new(&config.icon_theme);
    let top = if config.buttons { HEADER_H } else { TOP_PAD };
    let mut tiles = Vec::new();
    let mut tile = |key: &str, col: usize, y: f32, windows: usize, active: bool, pulse: f32, fixed: bool, db: &mut AppDb| {
        let app = if let Some(name) = key.strip_prefix("button:") {
            let (desktop, label) = if name == "applications" {
                ("com.system76.CosmicAppLibrary", tr(Msg::Applications))
            } else {
                ("com.system76.CosmicWorkspaces", tr(Msg::Workspaces))
            };
            db.builtin(key, desktop, label)
        } else {
            let key = db.canonical(key);
            db.app(&key)
        };
        tiles.push(TileDraw {
            app,
            x: tile_x(col),
            y,
            col,
            lit: config.backlight == Backlight::Always || windows > 0,
            windows,
            active,
            hover: 0.0,
            pressed: 0.0,
            pulse,
            alpha: 1.0,
            zoom: 1.0,
            fixed,
            lifted: false,
        });
    };
    if config.buttons {
        tile("button:applications", 0, TOP_PAD, 0, false, 0.0, true, &mut db);
        tile("button:workspaces", 1, TOP_PAD, 0, false, 0.0, true, &mut db);
    }
    for (i, key) in config.pinned.iter().enumerate() {
        let (windows, active, pulse) = match i {
            0 => (1, false, 0.0),
            2 => (3, true, 0.0),
            4 => (2, false, 0.0),
            5 => (0, false, 0.9),
            7 => (2, false, 0.0),
            _ => (0, false, 0.0),
        };
        tile(key, 0, top + i as f32 * PITCH, windows, active, pulse, false, &mut db);
    }
    let open = ["com.system76.CosmicTerm", "com.system76.CosmicEdit", "com.system76.CosmicSettings", "no-desktop-file"];
    for (i, key) in open.iter().enumerate() {
        tile(key, 1, top + i as f32 * PITCH, 1 + i % 2, false, 0.0, false, &mut db);
    }
    // The tooltip and the menu are those of the pinned tiles they point at.
    let pinned_name = |row: usize| {
        let at = tiles.iter().find(|t| t.col == 0 && !t.fixed && t.y == top + row as f32 * PITCH);
        at.map(|t| t.app.name.clone()).unwrap_or_default()
    };
    let (name, menu_name) = (pinned_name(4), pinned_name(7));

    let source = wallpaper::current(&[]);
    let tint = match config.tint.trim().to_ascii_lowercase().as_str() {
        "wallpaper" => source.as_ref().and_then(wallpaper::tint_of).unwrap_or(wallpaper::NEUTRAL),
        other => wallpaper::parse_hex(other).unwrap_or(wallpaper::NEUTRAL),
    };
    let scene = Scene {
        height,
        scale,
        opacity: config.opacity,
        tint,
        header: config.buttons,
        tiles,
        overflow: [(false, false); COLUMNS],
        drop_mark: None,
    };
    let dock = render::render_dock(&scene).context("cannot draw the dock")?;

    // The wallpaper as the screen shows it, blurred under the dock; a
    // gradient stands in when there is none.
    let width = ((DOCK_W + 420.0) * scale) as u32;
    let mut pix = Pixmap::new(width, dock.height()).context("image too large")?;
    let shot = match &source {
        Some(wallpaper::Source::Image(path)) => image::open(path).ok(),
        _ => None,
    };
    if let Some(img) = shot {
        let (sw, sh) = ((1920.0 * scale) as u32, (1080.0 * scale) as u32);
        let full = img.resize_to_fill(sw, sh, image::imageops::FilterType::Triangle).to_rgba8();
        let top = ((sh.saturating_sub(dock.height())) / 2).min(sh - 1);
        let view = image::imageops::crop_imm(&full, 0, top, width.min(sw), dock.height().min(sh - top)).to_image();
        let blurred = image::imageops::blur(&view, 14.0 * scale);
        let edge = (DOCK_W * scale) as u32;
        for (x, y, p) in view.enumerate_pixels() {
            let src = if x < edge { blurred.get_pixel(x, y) } else { p };
            if let Some(c) = tiny_skia::PremultipliedColorU8::from_rgba(src[0], src[1], src[2], 255) {
                pix.pixels_mut()[(y * width + x) as usize] = c;
            }
        }
    } else {
        for (i, p) in pix.pixels_mut().iter_mut().enumerate() {
            let (x, y) = ((i as u32 % width) as f32 / width as f32, (i as u32 / width) as f32 / height.max(1.0) / scale);
            let c = Color::from_rgba(0.16 + 0.30 * x, 0.22 + 0.25 * y, 0.42 + 0.20 * (1.0 - y), 1.0).unwrap();
            *p = c.premultiply().to_color_u8();
        }
    }
    let paint = PixmapPaint::default();
    pix.draw_pixmap(0, 0, dock.as_ref(), &paint, Transform::identity(), None);

    let at = |x: f32, y: f32| (((DOCK_W + 3.0 + x) * scale) as i32, (y * scale) as i32);
    let (w, h) = render::tooltip_size(&name);
    if let Some(t) = render::render_tooltip(&name, w, h, h / 2.0, scale) {
        let (x, y) = at(0.0, top + 4.0 * PITCH + TILE / 2.0 - h / 2.0);
        pix.draw_pixmap(x, y, t.as_ref(), &paint, Transform::identity(), None);
    }
    let rows: Vec<MenuRow<u8>> = vec![
        MenuRow::Header(menu_name.clone()),
        MenuRow::Separator,
        MenuRow::Entry { label: format!("Documents — {menu_name}"), action: 0, check: Some(true) },
        MenuRow::Entry { label: format!("Images — {menu_name}"), action: 0, check: Some(false) },
        MenuRow::Separator,
        MenuRow::Entry { label: tr(Msg::NewWindow).into(), action: 0, check: None },
        MenuRow::Separator,
        MenuRow::Entry { label: tr(Msg::Unpin).into(), action: 0, check: None },
        MenuRow::Entry { label: close_all(2), action: 0, check: None },
    ];
    let layout = render::layout_menu(&rows);
    if let Some(m) = render::render_menu(&rows, &layout, Some(3), 40.0, scale) {
        let (x, y) = at(0.0, top + 7.0 * PITCH);
        pix.draw_pixmap(x, y, m.as_ref(), &paint, Transform::identity(), None);
    }
    pix.save_png(out).with_context(|| format!("cannot write {}", out.display()))?;
    println!("{}", out.display());
    Ok(())
}
