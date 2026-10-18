//! Text glyphs from the game's TrueType fonts (`Data/merged.ttf`, and
//! `mergedsc.ttf` for Simplified Chinese), rasterised on demand with
//! `ab_glyph` (pure Rust; outlines are parsed lazily, so the 8 MB CJK
//! fonts cost little until a glyph is used) into the shared sprite atlas.
//! Glyphs are white with coverage as alpha and get tinted per quad.
//!
//! Pixel-font languages draw with the original's bitmap fonts from
//! `Assets.dat` instead (`crate::pixelfont`), falling back to the TTF for
//! characters without an image.

use std::collections::HashMap;
use std::path::Path;

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use baba_render::{Atlas, RegionId, RgbaImage};

/// A rasterised glyph.
#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    pub region: Option<RegionId>,
    pub w: u32,
    pub h: u32,
    /// Offset of the bitmap's top-left from the pen position on the
    /// baseline.
    pub dx: i32,
    pub dy: i32,
    pub advance: f32,
}

/// Fonts plus the glyph cache.
pub struct Fonts {
    regular: Option<FontVec>,
    sc: Option<FontVec>,
    /// Use the Simplified-Chinese variant.
    pub use_sc: bool,
    /// Draw with the bitmap fonts where they have the character.
    pub pixel: bool,
    /// Engine art (the bitmap fonts).
    pub art: crate::art::Art,
    cache: HashMap<(char, u16, bool, bool), Glyph>,
}

fn load(path: &Path) -> Option<FontVec> {
    let bytes = std::fs::read(path)
        .map_err(|e| log::warn!("{}: {e}", path.display()))
        .ok()?;
    FontVec::try_from_vec(bytes)
        .map_err(|e| log::warn!("{}: {e}", path.display()))
        .ok()
}

impl Fonts {
    pub fn load(data_dir: &Path) -> Fonts {
        Fonts {
            regular: load(&data_dir.join("merged.ttf")),
            sc: load(&data_dir.join("mergedsc.ttf")),
            use_sc: false,
            pixel: false,
            art: crate::art::Art::default(),
            cache: HashMap::new(),
        }
    }

    pub fn available(&self) -> bool {
        self.regular.is_some()
    }

    fn font(&self) -> Option<&FontVec> {
        if self.use_sc {
            self.sc.as_ref().or(self.regular.as_ref())
        } else {
            self.regular.as_ref()
        }
    }

    /// Ascent in pixels at size `px`.
    pub fn ascent(&self, px: f32) -> f32 {
        self.font()
            .map(|f| f.as_scaled(PxScale::from(px)).ascent())
            .unwrap_or(px * 0.8)
    }

    /// The glyph for `c` at `px`, rasterising it on first use.
    pub fn glyph(&mut self, atlas: &mut Atlas, c: char, px: f32) -> Glyph {
        let key = (c, (px * 4.0) as u16, self.use_sc, self.pixel);
        if let Some(g) = self.cache.get(&key) {
            return *g;
        }
        let g = match self.pixel_glyph(atlas, c, px) {
            Some(g) => g,
            None => self.raster(atlas, c, px),
        };
        self.cache.insert(key, g);
        g
    }

    /// A bitmap-font glyph: the small font below 28 px (integer scale
    /// `px / 16`), the large one above (`px / 40`). The image top is 2
    /// scaled pixels above the text line's top for the small font and at
    /// the line top for the large one (measured on captures).
    fn pixel_glyph(&mut self, atlas: &mut Atlas, c: char, px: f32) -> Option<Glyph> {
        use crate::pixelfont::{LARGE, LARGE_ADVANCE, SMALL, SMALL_ADVANCE, image};
        if !self.pixel {
            return None;
        }
        let large = px >= 28.0;
        let (set, adv, unit, lift) = if large {
            (LARGE, LARGE_ADVANCE, 40.0, 0)
        } else {
            (SMALL, SMALL_ADVANCE, 16.0, 2)
        };
        // Bitmap glyphs only scale up by whole steps; far below their
        // size (small splitscreen views) they would overflow the layout,
        // so the TTF draws there.
        if px / unit < 0.7 {
            return None;
        }
        let s = (px / unit).round().max(1.0) as u32;
        let asc = self.ascent(px).round() as i32;
        if c == ' ' {
            return Some(Glyph {
                region: None,
                w: 0,
                h: 0,
                dx: 0,
                dy: 0,
                advance: (adv * s) as f32,
            });
        }
        let n = image(set, c)?;
        let a = if s == 1 {
            self.art.get(atlas, n)?
        } else {
            let (img, hx, hy) = self.art.pixels(n)?;
            let mut big = RgbaImage::new(img.width * s, img.height * s);
            for y in 0..big.height {
                for x in 0..big.width {
                    big.put(x, y, img.get(x / s, y / s));
                }
            }
            crate::art::ArtImage {
                region: atlas.insert(&big),
                w: big.width,
                h: big.height,
                hx: hx * s as i32,
                hy: hy * s as i32,
            }
        };
        Some(Glyph {
            region: Some(a.region),
            w: a.w,
            h: a.h,
            dx: 0,
            dy: -asc - (lift * s) as i32,
            advance: (adv * s) as f32,
        })
    }

    fn raster(&self, atlas: &mut Atlas, c: char, px: f32) -> Glyph {
        let empty = Glyph {
            region: None,
            w: 0,
            h: 0,
            dx: 0,
            dy: 0,
            advance: px * 0.5,
        };
        let Some(font) = self.font() else {
            return empty;
        };
        let mut id = font.glyph_id(c);
        // Full-width forms missing from the font: use the ASCII ones.
        if id.0 == 0 && ('\u{FF01}'..='\u{FF5E}').contains(&c) {
            id = font.glyph_id(char::from_u32(c as u32 - 0xFEE0).unwrap_or('?'));
        }
        if id.0 == 0 && !c.is_whitespace() {
            id = font.glyph_id('?');
        }
        let scale = PxScale::from(px);
        let scaled = font.as_scaled(scale);
        let advance = scaled.h_advance(id);
        let glyph = id.with_scale_and_position(scale, ab_glyph::point(0.0, 0.0));
        let Some(outline) = font.outline_glyph(glyph) else {
            return Glyph { advance, ..empty };
        };
        let b = outline.px_bounds();
        let (w, h) = (b.width().ceil() as u32, b.height().ceil() as u32);
        if w == 0 || h == 0 {
            return Glyph { advance, ..empty };
        }
        let mut img = RgbaImage::new(w, h);
        outline.draw(|x, y, cov| {
            if x < w && y < h {
                img.put(x, y, [255, 255, 255, (cov.clamp(0.0, 1.0) * 255.0) as u8]);
            }
        });
        Glyph {
            region: Some(atlas.insert(&img)),
            w,
            h,
            dx: b.min.x.floor() as i32,
            dy: b.min.y.floor() as i32,
            advance,
        }
    }

    /// Width of `text` at `px` (no markup).
    pub fn measure(&mut self, atlas: &mut Atlas, text: &str, px: f32) -> f32 {
        text.chars().map(|c| self.glyph(atlas, c, px).advance).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lists the private-use / astral code points of the game's TTF
    /// (where the icon glyphs live). Skips without `BABA_GAME_DIR`.
    #[test]
    fn icon_code_points() {
        let Some(dir) = std::env::var_os("BABA_GAME_DIR") else {
            return;
        };
        let f = load(&Path::new(&dir).join("Data/merged.ttf")).expect("font");
        let mut odd: Vec<u32> = f
            .codepoint_ids()
            .map(|(_, c)| c as u32)
            .filter(|&c| c > 0xffff || (0xe000..0xf900).contains(&c))
            .collect();
        odd.sort();
        eprintln!(
            "{} code points; astral/PUA: {:x?}",
            f.codepoint_ids().count(),
            odd
        );
        for c in [
            '✿', '❀', '⏲', '⌚', '•', '●', '★', '♣', '¤', '£', '⚘', '✽', '❁',
        ] {
            eprintln!("{c} U+{:04X}: {}", c as u32, f.glyph_id(c).0);
        }
    }
}
