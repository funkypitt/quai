//! Drawing: the dock, its Unity-style tiles, tooltips and menus.
//!
//! Every measure is in logical pixels, in Unity 7's default proportions: a
//! 65 px launcher holding 54 px tiles around 48 px icons, 5 px apart.

use std::{cell::RefCell, collections::HashMap, rc::Rc};
use tiny_skia::{
    BlendMode, Color, FillRule, FilterQuality, GradientStop, LinearGradient, Mask, Paint, Path,
    PathBuilder, Pixmap, PixmapPaint, Point, RadialGradient, Rect, Shader, SpreadMode, Stroke,
    Transform,
};

use crate::apps::{App, Rgb};
use crate::text::with_text;

pub const COL_W: f32 = 65.0;
pub const COLUMNS: usize = 2;
pub const DOCK_W: f32 = COL_W * COLUMNS as f32;
pub const TILE: f32 = 54.0;
pub const ICON: f32 = 48.0;
pub const GAP: f32 = 5.0;
/// Room beside a tile for the pips of its open windows.
pub const PIP_W: f32 = 5.0;
pub const TOP_PAD: f32 = 7.0;
pub const BOTTOM_PAD: f32 = 7.0;
pub const RADIUS: f32 = 5.0;
pub const PITCH: f32 = TILE + GAP;
/// Height of the buttons row, separator included.
pub const HEADER_H: f32 = TOP_PAD + TILE + GAP + 1.0 + GAP;
/// Margin around a tile sprite, leaving room for the glow.
const SPRITE_M: f32 = 12.0;
const FADE: f32 = 18.0;

/// Left edge of a column's tiles. The right column mirrors the left one:
/// pips on the outer sides, and in the middle the arrow of the application
/// in use, so that the two never meet.
pub fn tile_x(col: usize) -> f32 {
    let x = col as f32 * COL_W;
    if col == 0 { x + PIP_W } else { x + COL_W - 1.0 - PIP_W - TILE }
}

const WHITE: Rgb = Rgb::new(1.0, 1.0, 1.0);
const BLACK: Rgb = Rgb::new(0.0, 0.0, 0.0);
const PANEL_BG: Rgb = Rgb::new(0.11, 0.11, 0.12);

pub struct TileDraw {
    pub app: Rc<App>,
    /// Top-left corner of the tile in the dock.
    pub x: f32,
    pub y: f32,
    pub col: usize,
    pub lit: bool,
    /// Number of open windows shown by the pips on the left.
    pub windows: usize,
    pub active: bool,
    pub hover: f32,
    pub pressed: f32,
    pub pulse: f32,
    pub alpha: f32,
    pub zoom: f32,
    /// Buttons of the first row stay put when a column scrolls.
    pub fixed: bool,
    /// The tile being dragged is drawn last, above the others.
    pub lifted: bool,
}

pub struct Scene {
    pub height: f32,
    pub scale: f32,
    pub opacity: f32,
    /// Colour of the dock's background.
    pub tint: Rgb,
    pub header: bool,
    pub tiles: Vec<TileDraw>,
    /// For each column, whether tiles are hidden above and below.
    pub overflow: [(bool, bool); COLUMNS],
    /// Where a dragged tile would land: column and y of the gap.
    pub drop_mark: Option<(usize, f32)>,
}

fn paint_color(c: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.anti_alias = true;
    p.set_color(c);
    p
}

fn paint_shader(shader: Shader<'static>) -> Paint<'static> {
    let mut p = Paint::default();
    p.anti_alias = true;
    p.shader = shader;
    p
}

fn vgrad(y0: f32, y1: f32, stops: &[(f32, Color)]) -> Option<Shader<'static>> {
    LinearGradient::new(
        Point::from_xy(0.0, y0),
        Point::from_xy(0.0, y1),
        stops.iter().map(|(p, c)| GradientStop::new(*p, *c)).collect(),
        SpreadMode::Pad,
        Transform::identity(),
    )
}

pub fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    // Control point distance for a quarter circle drawn with a cubic.
    let k = r * 0.552_284_8;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

fn fill(pix: &mut Pixmap, path: &Path, paint: &Paint, tf: Transform, mask: Option<&Mask>) {
    pix.fill_path(path, paint, FillRule::Winding, tf, mask);
}

fn stroke(pix: &mut Pixmap, path: &Path, paint: &Paint, width: f32, tf: Transform) {
    let stroke = Stroke { width, ..Stroke::default() };
    pix.stroke_path(path, paint, &stroke, tf, None);
}

fn fill_rect(pix: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, paint: &Paint, tf: Transform) {
    if let Some(r) = Rect::from_xywh(x, y, w, h) {
        pix.fill_rect(r, paint, tf, None);
    }
}

// ----------------------------------------------------------------- tiles

/// What a sprite's look depends on; animated values in sixteenths.
type SpriteKey = (String, bool, u8, u8, u8, u32);

thread_local! {
    static SPRITES: RefCell<HashMap<SpriteKey, Rc<Pixmap>>> = RefCell::new(HashMap::new());
}

/// Forgets drawn tiles, when icons or their theme change.
pub fn clear_sprites() {
    SPRITES.with(|c| c.borrow_mut().clear());
}

fn cached_sprite(t: &TileDraw, s: f32) -> Option<Rc<Pixmap>> {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 16.0).round() as u8;
    let key = (t.app.key.clone(), t.lit, q(t.hover), q(t.pressed), q(t.pulse), (s * 120.0).round() as u32);
    if let Some(p) = SPRITES.with(|c| c.borrow().get(&key).cloned()) {
        return Some(p);
    }
    let steady = TileDraw {
        app: t.app.clone(),
        hover: key.2 as f32 / 16.0,
        pressed: key.3 as f32 / 16.0,
        pulse: key.4 as f32 / 16.0,
        ..*t
    };
    let pix = Rc::new(tile_sprite(&steady, s)?);
    SPRITES.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 600 {
            c.clear();
        }
        c.insert(key, pix.clone());
    });
    Some(pix)
}

/// One tile with its glow, as a sprite `TILE + 2 × SPRITE_M` wide.
fn tile_sprite(t: &TileDraw, s: f32) -> Option<Pixmap> {
    let side = ((TILE + 2.0 * SPRITE_M) * s).ceil() as u32;
    let mut pix = Pixmap::new(side, side)?;
    let tf = Transform::from_scale(s, s);
    let (x, y) = (SPRITE_M, SPRITE_M);
    let (cx, cy) = (x + TILE / 2.0, y + TILE / 2.0);
    let body = rounded_rect(x, y, TILE, TILE, RADIUS)?;

    // Glow behind the tile: a launch in progress, or the pointer over it.
    let glow = (t.pulse * 0.85 + t.hover * 0.22).min(1.0);
    if glow > 0.01 {
        let g = t.app.glow;
        if let Some(shader) = RadialGradient::new(
            Point::from_xy(cx, cy),
            0.0,
            Point::from_xy(cx, cy),
            TILE * 0.72,
            vec![
                GradientStop::new(0.0, g.color(glow)),
                GradientStop::new(0.70, g.color(0.62 * glow)),
                GradientStop::new(1.0, g.color(0.0)),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        ) {
            let side_l = TILE + 2.0 * SPRITE_M;
            fill_rect(&mut pix, 0.0, 0.0, side_l, side_l, &paint_shader(shader), tf);
        }
    }

    // Backlight, in the colour drawn from the icon.
    if t.lit {
        let c = t.app.tile.mix(t.app.glow, t.pulse * 0.45);
        let stops = [
            (0.0, c.mix(WHITE, 0.16).color(0.96)),
            (0.5, c.color(0.96)),
            (1.0, c.scaled(0.68).color(0.96)),
        ];
        if let Some(shader) = vgrad(y, y + TILE, &stops) {
            fill(&mut pix, &body, &paint_shader(shader), tf, None);
        }
    } else {
        let stops = [(0.0, WHITE.color(0.11)), (1.0, WHITE.color(0.04))];
        if let Some(shader) = vgrad(y, y + TILE, &stops) {
            fill(&mut pix, &body, &paint_shader(shader), tf, None);
        }
    }

    // Icon.
    let icon_px = (ICON * t.app.fit * s).round() as u32;
    let icon = t.app.icon(icon_px);
    let off = ((SPRITE_M + TILE / 2.0) * s - icon_px as f32 / 2.0).round() as i32;
    pix.draw_pixmap(off, off, icon.as_ref().as_ref(), &PixmapPaint::default(), Transform::identity(), None);

    // Glass shine over the upper half, clipped to the tile.
    let mut clip = Mask::new(side, side)?;
    clip.fill_path(&body, FillRule::Winding, true, tf);
    let mut pb = PathBuilder::new();
    pb.move_to(x, y);
    pb.line_to(x + TILE, y);
    pb.line_to(x + TILE, y + TILE * 0.36);
    pb.cubic_to(x + TILE * 0.70, y + TILE * 0.50, x + TILE * 0.30, y + TILE * 0.56, x, y + TILE * 0.60);
    pb.close();
    if let (Some(shine), Some(shader)) = (
        pb.finish(),
        vgrad(y, y + TILE * 0.60, &[(0.0, WHITE.color(0.36)), (1.0, WHITE.color(0.07))]),
    ) {
        fill(&mut pix, &shine, &paint_shader(shader), tf, Some(&clip));
    }

    // Pointer feedback.
    if t.hover > 0.01 {
        fill(&mut pix, &body, &paint_color(WHITE.color(0.10 * t.hover)), tf, None);
    }
    if t.pressed > 0.01 {
        fill(&mut pix, &body, &paint_color(BLACK.color(0.22 * t.pressed)), tf, None);
    }

    // Edge: a dark line outside, a light one inside, brighter at the top.
    if let Some(outer) = rounded_rect(x - 0.5, y - 0.5, TILE + 1.0, TILE + 1.0, RADIUS + 0.5) {
        stroke(&mut pix, &outer, &paint_color(BLACK.color(0.45)), 1.0, tf);
    }
    if let (Some(inner), Some(shader)) = (
        rounded_rect(x + 0.5, y + 0.5, TILE - 1.0, TILE - 1.0, RADIUS - 0.5),
        vgrad(
            y,
            y + TILE,
            &[
                (0.0, WHITE.color(if t.lit { 0.62 } else { 0.34 })),
                (0.5, WHITE.color(if t.lit { 0.26 } else { 0.14 })),
                (1.0, WHITE.color(if t.lit { 0.16 } else { 0.10 })),
            ],
        ),
    ) {
        stroke(&mut pix, &inner, &paint_shader(shader), 1.0, tf);
    }
    Some(pix)
}

fn triangle(pix: &mut Pixmap, pts: [(f32, f32); 3], paint: &Paint, tf: Transform) {
    let mut pb = PathBuilder::new();
    pb.move_to(pts[0].0, pts[0].1);
    pb.line_to(pts[1].0, pts[1].1);
    pb.line_to(pts[2].0, pts[2].1);
    pb.close();
    if let Some(p) = pb.finish() {
        fill(pix, &p, paint, tf, None);
    }
}

/// Unity's arrows: pips on the outer side for open windows, and an arrow on
/// the inner side for the application in use.
fn indicators(pix: &mut Pixmap, t: &TileDraw, tf: Transform) {
    let cy = t.y + TILE / 2.0;
    let paint = paint_color(WHITE.color(0.95 * t.alpha));
    let shadow = paint_color(BLACK.color(0.35 * t.alpha));
    // `edge` is where the base of the triangle sits, `dir` where it points.
    let mut arrow = |edge: f32, dir: f32, y: f32, half: f32| {
        triangle(pix, [(edge, y - half - 1.0), (edge + 5.0 * dir, y), (edge, y + half + 1.0)], &shadow, tf);
        triangle(pix, [(edge, y - half), (edge + 4.0 * dir, y), (edge, y + half)], &paint, tf);
    };
    let left = t.col == 0;
    let (outer, inner) = if left { (0.0, COL_W) } else { (DOCK_W - 1.0, COL_W) };
    let n = t.windows.min(3);
    let (half, step) = if n <= 1 { (4.5, 0.0) } else { (3.0, 8.0) };
    for i in 0..n {
        let y = cy + (i as f32 - (n as f32 - 1.0) / 2.0) * step;
        arrow(outer, if left { 1.0 } else { -1.0 }, y, half);
    }
    if t.active {
        // The base sits mid-gutter, the point against the tile.
        let base = if left { inner - 1.5 } else { inner + 1.5 };
        arrow(base, if left { -1.0 } else { 1.0 }, cy, 4.5);
    }
}

fn draw_tile(pix: &mut Pixmap, t: &TileDraw, s: f32) {
    if t.alpha <= 0.01 {
        return;
    }
    let Some(sprite) = cached_sprite(t, s) else { return };
    let paint = PixmapPaint {
        opacity: t.alpha.clamp(0.0, 1.0),
        blend_mode: BlendMode::SourceOver,
        quality: if (t.zoom - 1.0).abs() > 0.001 { FilterQuality::Bilinear } else { FilterQuality::Nearest },
    };
    let (dx, dy) = (((t.x - SPRITE_M) * s).round(), ((t.y - SPRITE_M) * s).round());
    let half = sprite.width() as f32 / 2.0;
    let tf = Transform::from_translate(-half, -half)
        .post_scale(t.zoom, t.zoom)
        .post_translate(dx + half, dy + half);
    pix.draw_pixmap(0, 0, sprite.as_ref().as_ref(), &paint, tf, None);
    if t.windows > 0 || t.active {
        indicators(pix, t, Transform::from_scale(s, s));
    }
}

// ------------------------------------------------------------------ dock

pub fn render_dock(scene: &Scene) -> Option<Pixmap> {
    let s = scene.scale;
    let (w, h) = ((DOCK_W * s).round() as u32, (scene.height * s).round() as u32);
    let tf = Transform::from_scale(s, s);
    let top = if scene.header { HEADER_H } else { TOP_PAD };

    // Scrolling tiles go on their own layer so they can fade at the ends
    // without piercing the translucent background.
    let mut layer = Pixmap::new(w, h.max(1))?;
    for t in scene.tiles.iter().filter(|t| !t.fixed && !t.lifted) {
        draw_tile(&mut layer, t, s);
    }
    let mut erase = Paint::default();
    erase.blend_mode = BlendMode::Clear;
    fill_rect(&mut layer, 0.0, 0.0, DOCK_W, top - 1.0, &erase, tf);
    for (col, (above, below)) in scene.overflow.iter().enumerate() {
        let x = col as f32 * COL_W;
        let mut fade = |y0: f32, y1: f32| {
            let stops = [(0.0, BLACK.color(1.0)), (1.0, BLACK.color(0.0))];
            if let Some(shader) = vgrad(y0, y1, &stops) {
                let mut p = paint_shader(shader);
                p.blend_mode = BlendMode::DestinationOut;
                fill_rect(&mut layer, x, y0.min(y1), COL_W, (y1 - y0).abs(), &p, tf);
            }
        };
        if *above {
            fade(top - 1.0, top - 1.0 + FADE);
        }
        if *below {
            fade(scene.height, scene.height - FADE);
        }
    }

    let mut pix = Pixmap::new(w, h.max(1))?;
    // A touch lighter at the top, as if lit from above.
    let t = scene.tint;
    let stops = [(0.0, t.mix(WHITE, 0.07).color(scene.opacity)), (1.0, t.scaled(0.78).color(scene.opacity))];
    match vgrad(0.0, scene.height, &stops) {
        Some(shader) => {
            let mut p = paint_shader(shader);
            p.blend_mode = BlendMode::Source;
            fill_rect(&mut pix, 0.0, 0.0, DOCK_W, scene.height, &p, tf);
        }
        None => pix.fill(t.color(scene.opacity)),
    }
    // Unity's side line, and faint rules between the areas.
    fill_rect(&mut pix, DOCK_W - 1.0, 0.0, 1.0, scene.height, &paint_color(WHITE.color(0.16)), tf);
    if scene.header {
        let y = TOP_PAD + TILE + GAP;
        fill_rect(&mut pix, 6.0, y, DOCK_W - 13.0, 1.0, &paint_color(WHITE.color(0.12)), tf);
    }
    pix.draw_pixmap(0, 0, layer.as_ref(), &PixmapPaint::default(), Transform::identity(), None);

    if let Some((col, y)) = scene.drop_mark {
        fill_rect(&mut pix, tile_x(col), y - 1.5, TILE, 3.0, &paint_color(WHITE.color(0.85)), tf);
    }
    for t in scene.tiles.iter().filter(|t| t.fixed && !t.lifted) {
        draw_tile(&mut pix, t, s);
    }
    for t in scene.tiles.iter().filter(|t| t.lifted) {
        draw_tile(&mut pix, t, s);
    }
    Some(pix)
}

// ---------------------------------------------------------------- popups

pub const ARROW_W: f32 = 8.0;
const PANEL_R: f32 = 6.0;
const FONT: f32 = 13.5;

#[derive(Debug, Clone, PartialEq)]
pub enum MenuRow<A> {
    Header(String),
    Separator,
    Entry { label: String, action: A, check: Option<bool> },
}

pub struct MenuLayout {
    pub width: f32,
    pub height: f32,
    /// Top and height of each row.
    pub rows: Vec<(f32, f32)>,
    checks: bool,
}

const MENU_PAD: f32 = 6.0;
const ROW_H: f32 = 28.0;
const HEADER_ROW_H: f32 = 30.0;
const SEP_H: f32 = 9.0;
const MENU_MIN_W: f32 = 170.0;
const MENU_MAX_W: f32 = 380.0;

impl MenuLayout {
    pub fn row_at(&self, x: f32, y: f32) -> Option<usize> {
        if x < ARROW_W || x > self.width {
            return None;
        }
        self.rows.iter().position(|(top, h)| y >= *top && y < top + h)
    }
}

pub fn layout_menu<A>(rows: &[MenuRow<A>]) -> MenuLayout {
    let checks = rows.iter().any(|r| matches!(r, MenuRow::Entry { check: Some(_), .. }));
    let left = if checks { 34.0 } else { 14.0 };
    let mut y = MENU_PAD;
    let mut text_w = 0f32;
    let mut out = Vec::with_capacity(rows.len());
    with_text(|t| {
        for row in rows {
            let h = match row {
                MenuRow::Header(label) => {
                    text_w = text_w.max(t.measure(label, FONT, true, None).0 + 14.0 - left);
                    HEADER_ROW_H
                }
                MenuRow::Separator => SEP_H,
                MenuRow::Entry { label, .. } => {
                    text_w = text_w.max(t.measure(label, FONT, false, None).0);
                    ROW_H
                }
            };
            out.push((y, h));
            y += h;
        }
    });
    let body = (left + text_w + 18.0).clamp(MENU_MIN_W, MENU_MAX_W);
    MenuLayout { width: (ARROW_W + body).ceil(), height: (y + MENU_PAD).ceil(), rows: out, checks }
}

/// A dark panel with Unity's arrow on its left side, pointing at the tile.
fn panel(pix: &mut Pixmap, w: f32, h: f32, arrow_y: f32, tf: Transform) {
    let ay = arrow_y.clamp(PANEL_R + 8.0, (h - PANEL_R - 8.0).max(PANEL_R + 8.0));
    let build = |inset: f32| -> Option<Path> {
        let (x0, y0, x1, y1) = (ARROW_W + inset, inset, w - inset, h - inset);
        let r = PANEL_R - inset;
        let k = r * 0.552_284_8;
        let mut pb = PathBuilder::new();
        pb.move_to(x0 + r, y0);
        pb.line_to(x1 - r, y0);
        pb.cubic_to(x1 - r + k, y0, x1, y0 + r - k, x1, y0 + r);
        pb.line_to(x1, y1 - r);
        pb.cubic_to(x1, y1 - r + k, x1 - r + k, y1, x1 - r, y1);
        pb.line_to(x0 + r, y1);
        pb.cubic_to(x0 + r - k, y1, x0, y1 - r + k, x0, y1 - r);
        if h > 2.0 * (PANEL_R + 8.0) - 0.5 {
            pb.line_to(x0, ay + 7.0);
            pb.line_to(inset * 1.6, ay);
            pb.line_to(x0, ay - 7.0);
        }
        pb.line_to(x0, y0 + r);
        pb.cubic_to(x0, y0 + r - k, x0 + r - k, y0, x0 + r, y0);
        pb.close();
        pb.finish()
    };
    if let Some(p) = build(0.0) {
        fill(pix, &p, &paint_color(PANEL_BG.color(0.97)), tf, None);
    }
    if let Some(p) = build(0.5) {
        stroke(pix, &p, &paint_color(WHITE.color(0.24)), 1.0, tf);
    }
}

pub fn tooltip_size(text: &str) -> (f32, f32) {
    let (w, _) = with_text(|t| t.measure(text, FONT, false, Some(420.0)));
    ((ARROW_W + w + 26.0).ceil(), 32.0)
}

pub fn render_tooltip(text: &str, w: f32, h: f32, arrow_y: f32, s: f32) -> Option<Pixmap> {
    let mut pix = Pixmap::new((w * s).round() as u32, (h * s).round() as u32)?;
    panel(&mut pix, w, h, arrow_y, Transform::from_scale(s, s));
    with_text(|t| {
        let size = FONT * s;
        let (_, th) = t.measure(text, size, false, Some(420.0 * s));
        t.draw(&mut pix, text, (ARROW_W + 13.0) * s, (h * s - th) / 2.0, size, false, WHITE, 1.0, Some(420.0 * s));
    });
    Some(pix)
}

pub fn render_menu<A>(
    rows: &[MenuRow<A>],
    layout: &MenuLayout,
    hover: Option<usize>,
    arrow_y: f32,
    s: f32,
) -> Option<Pixmap> {
    let (w, h) = (layout.width, layout.height);
    let mut pix = Pixmap::new((w * s).round() as u32, (h * s).round() as u32)?;
    let tf = Transform::from_scale(s, s);
    panel(&mut pix, w, h, arrow_y, tf);
    let left = ARROW_W + if layout.checks { 34.0 } else { 14.0 };
    let max_text = (w - left - 14.0).max(20.0) * s;

    for (i, row) in rows.iter().enumerate() {
        let (y, rh) = layout.rows[i];
        match row {
            MenuRow::Separator => {
                fill_rect(&mut pix, ARROW_W + 1.0, y + (rh / 2.0).floor(), w - ARROW_W - 2.0, 1.0, &paint_color(WHITE.color(0.14)), tf);
            }
            MenuRow::Header(label) => with_text(|t| {
                let size = FONT * s;
                let (_, th) = t.measure(label, size, true, Some(max_text));
                t.draw(&mut pix, label, (ARROW_W + 14.0) * s, y * s + (rh * s - th) / 2.0, size, true, WHITE, 1.0, Some((w - ARROW_W - 28.0) * s));
            }),
            MenuRow::Entry { label, check, .. } => {
                if hover == Some(i)
                    && let Some(p) = rounded_rect(ARROW_W + 4.0, y, w - ARROW_W - 8.0, rh, 4.0)
                {
                    fill(&mut pix, &p, &paint_color(WHITE.color(0.16)), tf, None);
                }
                if *check == Some(true) {
                    let (cx, cy) = (ARROW_W + 19.0, y + rh / 2.0);
                    let mut pb = PathBuilder::new();
                    pb.move_to(cx - 5.0, cy);
                    pb.line_to(cx - 1.5, cy + 3.5);
                    pb.line_to(cx + 5.0, cy - 4.0);
                    if let Some(p) = pb.finish() {
                        stroke(&mut pix, &p, &paint_color(WHITE.color(0.95)), 1.8, tf);
                    }
                }
                with_text(|t| {
                    let size = FONT * s;
                    let (_, th) = t.measure(label, size, false, Some(max_text));
                    t.draw(&mut pix, label, left * s, y * s + (rh * s - th) / 2.0, size, false, WHITE, 0.96, Some(max_text));
                });
            }
        }
    }
    Some(pix)
}

/// Copies a premultiplied RGBA pixmap into a wl_shm ARGB8888 buffer.
pub fn copy_to_shm(pix: &Pixmap, canvas: &mut [u8]) {
    for (dst, src) in canvas.chunks_exact_mut(4).zip(pix.data().chunks_exact(4)) {
        dst[0] = src[2];
        dst[1] = src[1];
        dst[2] = src[0];
        dst[3] = src[3];
    }
}
