//! Level icons on maps (`presentation.md` §12.1): a rounded square in the
//! icon colour (filled lighter while locked, dark once open) and, once
//! open, the symbol: two digits, a letter, a dot, or the map picture of
//! `[icons]`; completed icons use the clear colour. The square and dot
//! are our own shapes (the original's are engine art); digits and letters
//! are the game's text sprites cropped and scaled down.

use std::path::Path;

use baba_formats::palette::PalColour;
use baba_render::{Atlas, RegionId, RgbaImage, SceneSprite, TileScene};

use super::{LevelGfx, phase};
use crate::art::Art;

/// Engine images of a level icon: the rounded body in its three wobble
/// frames (`presentation.md` §12.1, identified on captures).
const ICON_BODY: [u32; 3] = [24, 590, 591];

/// Two-digit number symbols 00–99.
const NUMBERS: [u32; 100] = [
    10, 12, 16, 17, 22, 28, 29, 32, 36, 42, 44, 45, 46, 61, 64, 66, 68, 69, 70, 73, 74, 75, 6, 78,
    80, 84, 92, 93, 98, 100, 102, 106, 110, 111, 118, 128, 133, 137, 142, 148, 149, 152, 155, 157,
    158, 159, 160, 161, 162, 164, 167, 169, 172, 173, 174, 175, 189, 192, 194, 196, 223, 224, 227,
    230, 231, 241, 253, 254, 255, 258, 259, 261, 272, 274, 307, 308, 309, 316, 319, 322, 323, 327,
    329, 330, 335, 343, 345, 346, 347, 352, 382, 391, 393, 394, 395, 397, 398, 400, 401, 450,
];

/// Letter symbols a–z.
const LETTERS: [u32; 26] = [
    552, 559, 796, 1034, 1036, 1037, 1123, 1124, 1125, 1126, 31, 62, 65, 67, 2, 81, 79, 82, 83, 86,
    88, 90, 99, 89, 116, 125,
];

/// Dot symbols: number n shows n + 1 dots (1–9; matched on the meta map
/// `304level`, `presentation.md` §15.31), 9 and above one big dot.
const DOTS: [u32; 10] = [462, 463, 464, 465, 466, 470, 471, 76, 501, 502];

fn lit(p: [u8; 4]) -> bool {
    p[3] > 0 && (p[0] as u32 + p[1] as u32 + p[2] as u32) > 60
}

impl LevelGfx {
    /// The rounded square (fill mask, border mask), generated once.
    fn squares(&mut self, atlas: &mut Atlas) -> (RegionId, RegionId) {
        if let Some(s) = self.square {
            return s;
        }
        let n = 20u32;
        let mut fill = RgbaImage::new(n, n);
        let mut border = RgbaImage::new(n, n);
        for y in 0..n {
            for x in 0..n {
                let dx = x.min(n - 1 - x);
                let dy = y.min(n - 1 - y);
                if dx + dy < 3 {
                    continue; // cut corner
                }
                if dx < 2 || dy < 2 || dx + dy < 5 {
                    border.put(x, y, [255; 4]);
                } else {
                    fill.put(x, y, [255; 4]);
                }
            }
        }
        let s = (atlas.insert(&fill), atlas.insert(&border));
        self.square = Some(s);
        s
    }

    fn dot(&mut self, atlas: &mut Atlas) -> RegionId {
        if let Some(d) = self.dot {
            return d;
        }
        let n = 8u32;
        let mut img = RgbaImage::new(n, n);
        for y in 0..n {
            for x in 0..n {
                let (dx, dy) = (x as f32 - 3.5, y as f32 - 3.5);
                if dx * dx + dy * dy <= 13.0 {
                    img.put(x, y, [255; 4]);
                }
            }
        }
        let d = atlas.insert(&img);
        self.dot = Some(d);
        d
    }

    /// A digit or letter: the text sprite cropped to its lit pixels and
    /// scaled to 14 px height. Returns the region and its width.
    fn glyph(&mut self, atlas: &mut Atlas, data_dir: &Path, c: char) -> Option<(RegionId, u32)> {
        if let Some(g) = self.glyphs.get(&c) {
            return *g;
        }
        let g = RgbaImage::load_png(data_dir.join("Sprites").join(format!("text_{c}_0_1.png")))
            .ok()
            .and_then(|img| {
                let (mut x0, mut y0, mut x1, mut y1) = (img.width, img.height, 0, 0);
                for y in 0..img.height {
                    for x in 0..img.width {
                        if lit(img.get(x, y)) {
                            x0 = x0.min(x);
                            y0 = y0.min(y);
                            x1 = x1.max(x + 1);
                            y1 = y1.max(y + 1);
                        }
                    }
                }
                if x1 <= x0 || y1 <= y0 {
                    return None;
                }
                let (w, h) = (x1 - x0, y1 - y0);
                let th = 14u32;
                let tw = ((w * th) as f32 / h as f32).round().max(1.0) as u32;
                let mut out = RgbaImage::new(tw, th);
                for y in 0..th {
                    for x in 0..tw {
                        if lit(img.get(x0 + x * w / tw, y0 + y * h / th)) {
                            out.put(x, y, [255; 4]);
                        }
                    }
                }
                Some((atlas.insert(&out), tw))
            });
        self.glyphs.insert(c, g);
        g
    }

    pub(super) fn push_icon(
        &mut self,
        atlas: &mut Atlas,
        data_dir: &Path,
        scene: &mut TileScene,
        u: &baba_sim::Unit,
        art: &mut Art,
    ) {
        let Some(icon) = u.icon.clone() else { return };
        if icon.completion <= 0 {
            return;
        }
        let done = icon.completion >= 3;
        let locked = icon.completion == 1;
        let col = if done {
            &icon.clear_colour
        } else {
            &icon.colour
        };
        let c = self.colour(PalColour::parse(col).unwrap_or(PalColour::new(0, 3)));
        let shade = |k: f32| {
            [
                (c[0] as f32 * k) as u8,
                (c[1] as f32 * k) as u8,
                (c[2] as f32 * k) as u8,
                255,
            ]
        };
        let (x, y) = (u.x as f32, u.y as f32);
        let sprite = |r: RegionId, dx: f32, colour: [u8; 4], layer: i16| SceneSprite {
            x: x + dx,
            y,
            frames: [r; 3],
            color: colour,
            wobble: 0,
            layer,
        };
        if art.available() && self.push_icon_art(atlas, data_dir, scene, u, &icon, c, art) {
            return;
        }
        if icon.style == -1 && !locked {
            if let Some(r) = self.icon_pic(atlas, icon.number, data_dir) {
                scene.sprites.push(sprite(r, 0.0, c, 15));
            }
            return;
        }
        let (fill, border) = self.squares(atlas);
        let fill_c = if locked { shade(0.55) } else { shade(0.22) };
        scene.sprites.push(sprite(fill, 0.0, fill_c, 14));
        scene.sprites.push(sprite(border, 0.0, c, 15));
        if locked {
            return;
        }
        let sym = if done { c } else { self.pal(0, 3) };
        let mut glyphs: Vec<(RegionId, u32)> = Vec::new();
        match icon.style {
            0 => {
                let n = icon.number.clamp(0, 99);
                for d in [n / 10, n % 10] {
                    if let Some(g) = self.glyph(atlas, data_dir, char::from(b'0' + d as u8)) {
                        glyphs.push(g);
                    }
                }
            }
            1 => {
                let ch = char::from(b'a' + icon.number.clamp(0, 25) as u8);
                if let Some(g) = self.glyph(atlas, data_dir, ch) {
                    glyphs.push(g);
                }
            }
            _ => {
                let d = self.dot(atlas);
                scene.sprites.push(sprite(d, 0.0, sym, 16));
                return;
            }
        }
        let total: u32 =
            glyphs.iter().map(|g| g.1).sum::<u32>() + 2 * glyphs.len().saturating_sub(1) as u32;
        let mut left = -(total as f32) / 2.0;
        for (r, w) in glyphs {
            let centre = left + w as f32 / 2.0;
            scene.sprites.push(sprite(r, centre / 24.0, sym, 16));
            left += w as f32 + 2.0;
        }
    }

    /// The icon from engine art; false when an image is missing.
    #[allow(clippy::too_many_arguments)]
    fn push_icon_art(
        &mut self,
        atlas: &mut Atlas,
        data_dir: &Path,
        scene: &mut TileScene,
        u: &baba_sim::Unit,
        icon: &baba_sim::IconData,
        c: [u8; 4],
        art: &mut Art,
    ) -> bool {
        let done = icon.completion >= 3;
        let locked = icon.completion == 1;
        let (x, y) = (u.x as f32, u.y as f32);
        let place = |a: crate::art::ArtImage| {
            let dx = (24 - a.w as i32).div_euclid(2) as f32;
            let dy = (24 - a.h as i32).div_euclid(2) as f32;
            (
                x + (12.0 - a.hx as f32 - dx) / 24.0,
                y + (12.0 - a.hy as f32 - dy) / 24.0,
            )
        };
        if icon.style == -1 && !locked {
            if let Some(r) = self.icon_pic(atlas, icon.number, data_dir) {
                scene.sprites.push(SceneSprite {
                    x,
                    y,
                    frames: [r; 3],
                    color: c,
                    wobble: 0,
                    layer: 15,
                });
            }
            return true;
        }
        let mut frames = [RegionId(0); 3];
        let mut first = None;
        for (i, n) in ICON_BODY.iter().enumerate() {
            let Some(a) = art.get(atlas, *n) else {
                return false;
            };
            frames[i] = a.region;
            first.get_or_insert(a);
        }
        let (bx, by) = place(first.unwrap());
        scene.sprites.push(SceneSprite {
            x: bx,
            y: by,
            frames,
            color: c,
            wobble: phase(u.id),
            layer: 14,
        });
        if locked {
            return true;
        }
        let sym = match icon.style {
            0 => NUMBERS.get(icon.number.clamp(0, 99) as usize).copied(),
            1 => LETTERS.get(icon.number.clamp(0, 25) as usize).copied(),
            2 => DOTS.get(icon.number.clamp(0, 9) as usize).copied(),
            _ => None,
        };
        if let Some(a) = sym.and_then(|n| art.get(atlas, n)) {
            let (sx, sy) = place(a);
            scene.sprites.push(SceneSprite {
                x: sx,
                y: sy,
                frames: [a.region; 3],
                color: if done { c } else { self.pal(0, 3) },
                wobble: 0,
                layer: 16,
            });
        }
        true
    }
}
