//! Text measuring and drawing on a pixmap.

use cosmic_text::{
    Attrs, Buffer, Color, Ellipsize, EllipsizeHeightLimit, Family, FontSystem, Metrics, Shaping,
    SwashCache, Weight, Wrap,
};
use std::cell::RefCell;
use tiny_skia::Pixmap;

use crate::apps::Rgb;

pub struct Text {
    fonts: FontSystem,
    cache: SwashCache,
    family: Option<String>,
}

thread_local! {
    static TEXT: RefCell<Option<Text>> = const { RefCell::new(None) };
}

/// Runs `f` with the shared text renderer, created on first use.
pub fn with_text<R>(f: impl FnOnce(&mut Text) -> R) -> R {
    TEXT.with(|t| f(t.borrow_mut().get_or_insert_with(Text::new)))
}

impl Text {
    fn new() -> Self {
        let fonts = FontSystem::new();
        // Unity's typeface when it is installed, the system's sans-serif otherwise.
        let family = ["Ubuntu", "Ubuntu Sans"]
            .into_iter()
            .find(|name| fonts.db().faces().any(|f| f.families.iter().any(|(n, _)| n == name)))
            .map(str::to_string);
        Self { fonts, cache: SwashCache::new(), family }
    }

    fn buffer(&mut self, text: &str, size: f32, bold: bool, max_width: Option<f32>) -> Buffer {
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(size, (size * 1.35).ceil()));
        buffer.set_wrap(Wrap::None);
        if max_width.is_some() {
            buffer.set_ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)));
        }
        buffer.set_size(max_width, None);
        let family = self.family.clone();
        let attrs = Attrs::new()
            .family(family.as_deref().map_or(Family::SansSerif, Family::Name))
            .weight(if bold { Weight::BOLD } else { Weight::NORMAL });
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        buffer
    }

    /// Width and height of one line of text.
    pub fn measure(&mut self, text: &str, size: f32, bold: bool, max_width: Option<f32>) -> (f32, f32) {
        let buffer = self.buffer(text, size, bold, max_width);
        let mut w = 0f32;
        let mut h = 0f32;
        for run in buffer.layout_runs() {
            w = w.max(run.line_w);
            h = h.max(run.line_top + run.line_height);
        }
        (w.ceil(), h.max((size * 1.35).ceil()))
    }

    /// Draws one line of text with its top-left corner at (x, y).
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        pix: &mut Pixmap,
        text: &str,
        x: f32,
        y: f32,
        size: f32,
        bold: bool,
        color: Rgb,
        alpha: f32,
        max_width: Option<f32>,
    ) {
        let mut buffer = self.buffer(text, size, bold, max_width);
        let (pw, ph) = (pix.width() as i32, pix.height() as i32);
        let (ox, oy) = (x.round() as i32, y.round() as i32);
        let base = Color::rgba(
            (color.r * 255.0) as u8,
            (color.g * 255.0) as u8,
            (color.b * 255.0) as u8,
            255,
        );
        let data = pix.data_mut();
        buffer.draw(&mut self.fonts, &mut self.cache, base, |gx, gy, w, h, c| {
            let a = c.a() as f32 / 255.0 * alpha;
            if a <= 0.0 {
                return;
            }
            let src = [c.r() as f32 * a, c.g() as f32 * a, c.b() as f32 * a, 255.0 * a];
            for py in gy..gy + h as i32 {
                for px in gx..gx + w as i32 {
                    let (tx, ty) = (ox + px, oy + py);
                    if tx < 0 || ty < 0 || tx >= pw || ty >= ph {
                        continue;
                    }
                    let i = (ty * pw + tx) as usize * 4;
                    for k in 0..4 {
                        let d = data[i + k] as f32;
                        data[i + k] = (src[k] + d * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
        });
    }
}
