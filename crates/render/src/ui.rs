//! Screen-space UI that does not come from the game's data: the on-screen
//! touch controls (icons drawn procedurally here) and a tiny built-in 5x7
//! font for messages shown before any game data exists (e.g. "copy your
//! game files here"). Drawn over the frame with [`Renderer::draw_ui`]
//! (`crate::Renderer`), in drawable pixels, origin top-left.

use crate::image::RgbaImage;

/// Procedural icons (white on transparent, 64x64, anti-aliased).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UiIcon {
    /// Filled disc: button background.
    Disc,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    /// A dot (wait / idle).
    Wait,
    /// Half-circle arrow (undo).
    Undo,
    /// Almost full circle arrow (restart).
    Restart,
    /// Three bars (menu).
    Menu,
    /// Opaque white square (solid rectangles).
    Solid,
}

impl UiIcon {
    pub const ALL: [UiIcon; 10] = [
        UiIcon::Disc,
        UiIcon::ArrowUp,
        UiIcon::ArrowDown,
        UiIcon::ArrowLeft,
        UiIcon::ArrowRight,
        UiIcon::Wait,
        UiIcon::Undo,
        UiIcon::Restart,
        UiIcon::Menu,
        UiIcon::Solid,
    ];

    fn index(self) -> u32 {
        UiIcon::ALL.iter().position(|&i| i == self).unwrap() as u32
    }
}

/// What a [`UiQuad`] shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UiSprite {
    Icon(UiIcon),
    /// A character of the built-in font (ASCII; lower case is drawn upper
    /// case, unknown characters as `?`).
    Glyph(char),
}

/// One UI rectangle in drawable pixels, tinted by `color` (straight alpha).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiQuad {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub sprite: UiSprite,
    pub color: [u8; 4],
}

/// Icon cell size in the icon texture.
pub const ICON: u32 = 64;
/// Glyph cell (5x7 glyph + 1 px spacing right and below).
pub const GLYPH_W: u32 = 6;
pub const GLYPH_H: u32 = 8;

const FONT_CHARS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.,:/-'()!?_> ";
#[rustfmt::skip]
const FONT: [[u8; 7]; 49] = [
    [0x0E,0x11,0x11,0x1F,0x11,0x11,0x11], [0x1E,0x11,0x11,0x1E,0x11,0x11,0x1E],
    [0x0E,0x11,0x10,0x10,0x10,0x11,0x0E], [0x1C,0x12,0x11,0x11,0x11,0x12,0x1C],
    [0x1F,0x10,0x10,0x1E,0x10,0x10,0x1F], [0x1F,0x10,0x10,0x1E,0x10,0x10,0x10],
    [0x0E,0x11,0x10,0x17,0x11,0x11,0x0F], [0x11,0x11,0x11,0x1F,0x11,0x11,0x11],
    [0x0E,0x04,0x04,0x04,0x04,0x04,0x0E], [0x07,0x02,0x02,0x02,0x02,0x12,0x0C],
    [0x11,0x12,0x14,0x18,0x14,0x12,0x11], [0x10,0x10,0x10,0x10,0x10,0x10,0x1F],
    [0x11,0x1B,0x15,0x15,0x11,0x11,0x11], [0x11,0x11,0x19,0x15,0x13,0x11,0x11],
    [0x0E,0x11,0x11,0x11,0x11,0x11,0x0E], [0x1E,0x11,0x11,0x1E,0x10,0x10,0x10],
    [0x0E,0x11,0x11,0x11,0x15,0x12,0x0D], [0x1E,0x11,0x11,0x1E,0x14,0x12,0x11],
    [0x0F,0x10,0x10,0x0E,0x01,0x01,0x1E], [0x1F,0x04,0x04,0x04,0x04,0x04,0x04],
    [0x11,0x11,0x11,0x11,0x11,0x11,0x0E], [0x11,0x11,0x11,0x11,0x11,0x0A,0x04],
    [0x11,0x11,0x11,0x15,0x15,0x15,0x0A], [0x11,0x11,0x0A,0x04,0x0A,0x11,0x11],
    [0x11,0x11,0x11,0x0A,0x04,0x04,0x04], [0x1F,0x01,0x02,0x04,0x08,0x10,0x1F],
    [0x0E,0x11,0x13,0x15,0x19,0x11,0x0E], [0x04,0x0C,0x04,0x04,0x04,0x04,0x0E],
    [0x0E,0x11,0x01,0x02,0x04,0x08,0x1F], [0x1F,0x02,0x04,0x02,0x01,0x11,0x0E],
    [0x02,0x06,0x0A,0x12,0x1F,0x02,0x02], [0x1F,0x10,0x1E,0x01,0x01,0x11,0x0E],
    [0x06,0x08,0x10,0x1E,0x11,0x11,0x0E], [0x1F,0x01,0x02,0x04,0x08,0x08,0x08],
    [0x0E,0x11,0x11,0x0E,0x11,0x11,0x0E], [0x0E,0x11,0x11,0x0F,0x01,0x02,0x0C],
    [0x00,0x00,0x00,0x00,0x00,0x0C,0x0C], [0x00,0x00,0x00,0x00,0x0C,0x04,0x08],
    [0x00,0x0C,0x0C,0x00,0x0C,0x0C,0x00], [0x00,0x01,0x02,0x04,0x08,0x10,0x00],
    [0x00,0x00,0x00,0x1F,0x00,0x00,0x00], [0x0C,0x04,0x08,0x00,0x00,0x00,0x00],
    [0x02,0x04,0x08,0x08,0x08,0x04,0x02], [0x08,0x04,0x02,0x02,0x02,0x04,0x08],
    [0x04,0x04,0x04,0x04,0x04,0x00,0x04], [0x0E,0x11,0x01,0x02,0x04,0x00,0x04],
    [0x00,0x00,0x00,0x00,0x00,0x00,0x1F], [0x08,0x04,0x02,0x01,0x02,0x04,0x08],
    [0x00,0x00,0x00,0x00,0x00,0x00,0x00],
];

/// Index of a character in the font texture.
pub(crate) fn glyph_index(c: char) -> u32 {
    let c = c.to_ascii_uppercase();
    FONT_CHARS
        .chars()
        .position(|f| f == c)
        .unwrap_or_else(|| FONT_CHARS.find('?').unwrap()) as u32
}

/// Texel rectangle `(x, y, w, h)` of a sprite in its texture.
pub(crate) fn sprite_rect(s: UiSprite) -> (u32, u32, u32, u32) {
    match s {
        UiSprite::Icon(i) => (i.index() * ICON, 0, ICON, ICON),
        UiSprite::Glyph(c) => (glyph_index(c) * GLYPH_W, 0, GLYPH_W - 1, GLYPH_H - 1),
    }
}

/// The icon strip (all [`UiIcon`]s side by side).
pub fn icon_texture() -> RgbaImage {
    let n = UiIcon::ALL.len() as u32;
    let mut img = RgbaImage::new(ICON * n, ICON);
    for (i, &icon) in UiIcon::ALL.iter().enumerate() {
        draw_icon(&mut img, i as u32 * ICON, icon);
    }
    img
}

/// The font strip (one 6x8 cell per character).
pub fn font_texture() -> RgbaImage {
    let mut img = RgbaImage::new(GLYPH_W * FONT.len() as u32, GLYPH_H);
    for (i, rows) in FONT.iter().enumerate() {
        for (y, bits) in rows.iter().enumerate() {
            for x in 0..5 {
                if bits & (0x10 >> x) != 0 {
                    img.put(i as u32 * GLYPH_W + x, y as u32, [255; 4]);
                }
            }
        }
    }
    img
}

/// Width in pixels of `text` at integer `scale`.
pub fn text_width(text: &str, scale: f32) -> f32 {
    let n = text.chars().count() as f32;
    (n * GLYPH_W as f32 - 1.0).max(0.0) * scale
}

/// Quads for a line of text with its top-left at `(x, y)`; each font pixel
/// is `scale` drawable pixels (use whole numbers for crisp text).
pub fn text_quads(text: &str, x: f32, y: f32, scale: f32, color: [u8; 4]) -> Vec<UiQuad> {
    text.chars()
        .enumerate()
        .filter(|(_, c)| *c != ' ')
        .map(|(i, c)| UiQuad {
            x: x + i as f32 * GLYPH_W as f32 * scale,
            y,
            w: (GLYPH_W - 1) as f32 * scale,
            h: (GLYPH_H - 1) as f32 * scale,
            sprite: UiSprite::Glyph(c),
            color,
        })
        .collect()
}

/// Breaks `text` into lines of at most `cols` characters (on spaces).
pub fn wrap(text: &str, cols: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            if !line.is_empty() && line.len() + 1 + word.len() > cols {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        lines.push(line);
    }
    lines
}

// ---------------------------------------------------------------- icon shapes

type Shape = Box<dyn Fn(f32, f32) -> bool>;

fn disc(cx: f32, cy: f32, r: f32) -> Shape {
    Box::new(move |x, y| (x - cx).powi(2) + (y - cy).powi(2) <= r * r)
}

fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Shape {
    Box::new(move |x, y| x >= x0 && x <= x1 && y >= y0 && y <= y1)
}

fn tri(a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> Shape {
    Box::new(move |x, y| {
        let s = |p: (f32, f32), q: (f32, f32)| (q.0 - p.0) * (y - p.1) - (q.1 - p.1) * (x - p.0);
        let (d1, d2, d3) = (s(a, b), s(b, c), s(c, a));
        let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
        let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
        !(neg && pos)
    })
}

/// Ring segment around (cx, cy), radius r, thickness t, between visual
/// angles a0..a1 (degrees, counter-clockwise, 0 = right).
fn arc(cx: f32, cy: f32, r: f32, t: f32, a0: f32, a1: f32) -> Shape {
    Box::new(move |x, y| {
        let (dx, dy) = (x - cx, cy - y);
        let d = (dx * dx + dy * dy).sqrt();
        if (d - r).abs() > t / 2.0 {
            return false;
        }
        let mut a = dy.atan2(dx).to_degrees();
        while a < a0 {
            a += 360.0;
        }
        a <= a1
    })
}

/// Arrowhead at the end of an arc (visual angle `a`, travelling
/// counter-clockwise when `ccw`).
fn arc_head(cx: f32, cy: f32, r: f32, a: f32, ccw: bool, size: f32) -> Shape {
    let t = a.to_radians();
    let p = (cx + r * t.cos(), cy - r * t.sin());
    // Screen-space tangent of counter-clockwise travel.
    let mut d = (-t.sin(), -t.cos());
    if !ccw {
        d = (-d.0, -d.1);
    }
    let n = (-d.1, d.0);
    tri(
        (p.0 + d.0 * size, p.1 + d.1 * size),
        (p.0 + n.0 * size - d.0 * 2.0, p.1 + n.1 * size - d.1 * 2.0),
        (p.0 - n.0 * size - d.0 * 2.0, p.1 - n.1 * size - d.1 * 2.0),
    )
}

fn shapes(icon: UiIcon) -> Vec<Shape> {
    let c = ICON as f32 / 2.0;
    match icon {
        UiIcon::Disc => vec![disc(c, c, c - 1.0)],
        UiIcon::Solid => vec![rect(0.0, 0.0, ICON as f32, ICON as f32)],
        UiIcon::ArrowUp => vec![tri(
            (c, c - 17.0),
            (c + 18.0, c + 11.0),
            (c - 18.0, c + 11.0),
        )],
        UiIcon::ArrowDown => vec![tri(
            (c, c + 17.0),
            (c - 18.0, c - 11.0),
            (c + 18.0, c - 11.0),
        )],
        UiIcon::ArrowLeft => vec![tri(
            (c - 17.0, c),
            (c + 11.0, c - 18.0),
            (c + 11.0, c + 18.0),
        )],
        UiIcon::ArrowRight => {
            vec![tri(
                (c + 17.0, c),
                (c - 11.0, c + 18.0),
                (c - 11.0, c - 18.0),
            )]
        }
        UiIcon::Wait => vec![disc(c, c, 8.0)],
        UiIcon::Undo => vec![
            arc(c, c + 4.0, 15.0, 7.0, -30.0, 180.0),
            arc_head(c, c + 4.0, 15.0, 180.0, true, 10.0),
        ],
        UiIcon::Restart => vec![
            arc(c, c, 15.0, 7.0, 110.0, 400.0),
            arc_head(c, c, 15.0, 110.0, false, 10.0),
        ],
        UiIcon::Menu => vec![
            rect(c - 14.0, c - 12.0, c + 14.0, c - 7.0),
            rect(c - 14.0, c - 2.5, c + 14.0, c + 2.5),
            rect(c - 14.0, c + 7.0, c + 14.0, c + 12.0),
        ],
    }
}

/// Rasterises an icon into its cell with 4x4 supersampling.
fn draw_icon(img: &mut RgbaImage, x0: u32, icon: UiIcon) {
    let sh = shapes(icon);
    const SS: u32 = 4;
    for y in 0..ICON {
        for x in 0..ICON {
            let mut hits = 0;
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = x as f32 + (sx as f32 + 0.5) / SS as f32;
                    let py = y as f32 + (sy as f32 + 0.5) / SS as f32;
                    if sh.iter().any(|s| s(px, py)) {
                        hits += 1;
                    }
                }
            }
            if hits > 0 {
                let a = (hits * 255 / (SS * SS)) as u8;
                img.put(x0 + x, y, [255, 255, 255, a]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_are_distinct_and_centred() {
        let img = icon_texture();
        let cover = |i: u32| -> (u32, f32, f32) {
            let (mut n, mut sx, mut sy) = (0u32, 0f32, 0f32);
            for y in 0..ICON {
                for x in 0..ICON {
                    if img.get(i * ICON + x, y)[3] > 127 {
                        n += 1;
                        sx += x as f32;
                        sy += y as f32;
                    }
                }
            }
            (n, sx / n.max(1) as f32, sy / n.max(1) as f32)
        };
        let all: Vec<_> = (0..UiIcon::ALL.len() as u32).map(cover).collect();
        for (i, (n, _, _)) in all.iter().enumerate() {
            assert!(*n > 100, "icon {i} nearly empty");
        }
        // Arrows point where they say: centroid offset opposite the tip.
        assert!(all[1].2 > 32.0 && all[2].2 < 32.0, "up/down");
        assert!(all[3].1 > 32.0 && all[4].1 < 32.0, "left/right");
    }

    #[test]
    fn font_covers_messages() {
        let img = font_texture();
        assert_eq!(img.width, GLYPH_W * FONT.len() as u32);
        assert_eq!(FONT_CHARS.chars().count(), FONT.len());
        assert_eq!(glyph_index('a'), glyph_index('A'));
        assert_eq!(glyph_index('~'), glyph_index('?'));
        let q = text_quads("Hi there", 10.0, 5.0, 2.0, [255; 4]);
        assert_eq!(q.len(), 7);
        assert_eq!(text_width("Hi", 2.0), 22.0);
        assert_eq!(
            wrap("copy the Data folder here please", 12),
            vec!["copy the", "Data folder", "here please"]
        );
    }
}
