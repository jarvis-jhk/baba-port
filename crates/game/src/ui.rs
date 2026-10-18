//! Screen-space drawing: rectangles, framed buttons, text with the
//! language markup (`$x,y` colour switches, `#key` insertions; `ui.md`
//! §9.3) and sprites, collected as quads for `Renderer::draw_overlay`.

use std::collections::HashMap;
use std::path::PathBuf;

use baba_formats::Palette;
use baba_render::{Atlas, Quad, RegionId, RgbaImage, SpriteBank};

use crate::font::Fonts;
use crate::lang::Texts;

/// Horizontal text alignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// Shared drawing resources (one atlas for levels, UI and glyphs).
pub struct Gfx {
    pub atlas: Atlas,
    pub fonts: Fonts,
    white: RegionId,
    disc: RegionId,
    /// The default palette (UI colours, `presentation.md` §5.5).
    pub palette: Palette,
    /// Global sprites (`Data/Sprites`).
    pub sprites: SpriteBank,
    /// Engine art from `Assets.dat`.
    pub art: crate::art::Art,
    images: HashMap<PathBuf, Option<RegionId>>,
    /// Upper-case all UI text (pixel-font languages: the original's pixel
    /// font draws lower-case letters as small capitals).
    pub upper: bool,
}

impl Gfx {
    pub fn new(mut atlas: Atlas, data_dir: &std::path::Path) -> Gfx {
        let white = atlas.insert(&RgbaImage::filled(4, 4, [255, 255, 255, 255]));
        // A soft-edged disc for transition blobs.
        let n = 64u32;
        let mut img = RgbaImage::new(n, n);
        let r = n as f32 / 2.0;
        for y in 0..n {
            for x in 0..n {
                let d = ((x as f32 + 0.5 - r).powi(2) + (y as f32 + 0.5 - r).powi(2)).sqrt();
                let a = (r - d).clamp(0.0, 1.0);
                img.put(x, y, [255, 255, 255, (a * 255.0) as u8]);
            }
        }
        let disc = atlas.insert(&img);
        let palette =
            Palette::load(&data_dir.join("Palettes").join("default.png")).unwrap_or_else(|_| {
                Palette {
                    width: 7,
                    height: 5,
                    colours: vec![[200, 200, 200]; 35],
                }
            });
        Gfx {
            atlas,
            fonts: Fonts::load(data_dir),
            white,
            disc,
            palette,
            sprites: SpriteBank::new(vec![data_dir.join("Sprites")]),
            art: crate::art::Art::default(),
            images: HashMap::new(),
            upper: true,
        }
    }

    /// Palette colour `(x, y)` of the default palette.
    pub fn pal(&self, x: u32, y: u32) -> [u8; 4] {
        let [r, g, b] = self
            .palette
            .rgb(baba_formats::PalColour::new(x as u8, y as u8));
        [r, g, b, 255]
    }

    /// A plain image file (cached).
    pub fn image(&mut self, path: PathBuf) -> Option<RegionId> {
        if let Some(r) = self.images.get(&path) {
            return *r;
        }
        let r = RgbaImage::load_png(&path)
            .ok()
            .filter(|i| i.width < self.atlas.page_size() && i.height < self.atlas.page_size())
            .map(|i| self.atlas.insert(&i));
        self.images.insert(path, r);
        r
    }

    /// A solid white region (fills, lines).
    pub fn white(&self) -> RegionId {
        self.white
    }

    pub fn region_size(&self, r: RegionId) -> (u32, u32) {
        let g = self.atlas.region(r);
        (g.w as u32, g.h as u32)
    }
}

/// An overlay being built.
pub struct Ui {
    pub quads: Vec<Quad>,
    /// UI scale (1 at the original's 854×480).
    pub scale: f32,
    pub width: f32,
    pub height: f32,
    /// Screen x of logical x 0 when set (edge-anchored overlays on narrow
    /// screens); otherwise the 854-wide canvas is centred.
    pub x0: Option<f32>,
}

impl Ui {
    /// Narrower than 16:9 (4:3, 1:1 handhelds).
    pub fn narrow(&self) -> bool {
        self.width / self.height < 854.0 / 480.0 - 0.01
    }

    /// For narrow screens: the same canvas at a larger scale for content
    /// that only spans the middle `span` logical pixels (title, title
    /// card): up to the screen height, as far as that span fits.
    pub fn widened(&self, span: f32) -> Ui {
        let mut u = Ui {
            quads: Vec::new(),
            scale: self.scale,
            width: self.width,
            height: self.height,
            x0: self.x0,
        };
        if self.narrow() {
            let s = (self.height / 480.0).min(self.width / span);
            if s > u.scale {
                u.scale = s;
            }
        }
        u
    }

    /// For narrow screens: an overlay whose logical x `anchor` sits at the
    /// same distance from the screen edge as on 16:9 (left edge for
    /// `anchor` < 427, right edge otherwise), at a readable scale (the
    /// height's, at most what `span` logical pixels allow).
    pub fn edge(&self, anchor_right: bool, span: f32) -> Ui {
        let mut u = self.widened(span);
        if self.narrow() {
            u.x0 = Some(if anchor_right {
                self.width - 854.0 * u.scale
            } else {
                0.0
            });
        }
        u
    }
    pub fn new(width: u32, height: u32) -> Ui {
        let scale = (width as f32 / 854.0).min(height as f32 / 480.0).max(0.5);
        // Integer scales keep text and frames crisp when possible.
        // (2560 wide is 2.998 × 854: within 1 % of the next integer
        // counts as that integer, a pixel or two of overhang is fine.)
        let scale = if scale >= 1.0 {
            (scale * 1.01).floor()
        } else {
            scale
        };
        Ui {
            quads: Vec::new(),
            scale,
            width: width as f32,
            height: height as f32,
            x0: None,
        }
    }

    /// Logical (854×480-based) coordinates → screen pixels, centred.
    pub fn x(&self, lx: f32) -> f32 {
        match self.x0 {
            Some(o) => o + lx * self.scale,
            None => (self.width - 854.0 * self.scale) / 2.0 + lx * self.scale,
        }
    }

    pub fn y(&self, ly: f32) -> f32 {
        (self.height - 480.0 * self.scale) / 2.0 + ly * self.scale
    }

    /// A filled rectangle in screen pixels.
    pub fn rect_px(&mut self, g: &Gfx, x: f32, y: f32, w: f32, h: f32, color: [u8; 4]) {
        self.quads.push(Quad {
            x,
            y,
            w,
            h,
            region: g.white,
            color,
        });
    }

    /// A filled rectangle in logical units.
    pub fn rect(&mut self, g: &Gfx, x: f32, y: f32, w: f32, h: f32, color: [u8; 4]) {
        let s = self.scale;
        self.rect_px(g, self.x(x), self.y(y), w * s, h * s, color);
    }

    /// The whole screen.
    pub fn fill(&mut self, g: &Gfx, color: [u8; 4]) {
        let (w, h) = (self.width, self.height);
        self.rect_px(g, 0.0, 0.0, w, h, color);
    }

    /// A button frame: fill with a 2-unit border.
    #[allow(clippy::too_many_arguments)]
    pub fn frame(
        &mut self,
        g: &Gfx,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        fill: [u8; 4],
        border: [u8; 4],
    ) {
        // The original's button frames (`Editor_button_*` pieces) have a
        // 2 px border with the corner pixels cut (measured).
        self.rect(g, x + 1.0, y, w - 2.0, h, border);
        self.rect(g, x, y + 1.0, 1.0, h - 2.0, border);
        self.rect(g, x + w - 1.0, y + 1.0, 1.0, h - 2.0, border);
        self.rect(g, x + 2.0, y + 2.0, w - 4.0, h - 4.0, fill);
    }

    /// A disc centred at screen pixel (cx, cy) with radius r.
    pub fn disc_px(&mut self, g: &Gfx, cx: f32, cy: f32, r: f32, color: [u8; 4]) {
        self.quads.push(Quad {
            x: cx - r,
            y: cy - r,
            w: 2.0 * r,
            h: 2.0 * r,
            region: g.disc,
            color,
        });
    }

    /// An atlas region at logical (x, y), size in logical units.
    pub fn sprite(&mut self, r: RegionId, x: f32, y: f32, w: f32, h: f32, color: [u8; 4]) {
        let s = self.scale;
        self.quads.push(Quad {
            x: self.x(x),
            y: self.y(y),
            w: w * s,
            h: h * s,
            region: r,
            color,
        });
    }

    /// Draws text with markup at logical (x, y) = top of the line, `px`
    /// logical size. Returns the logical width.
    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &mut self,
        g: &mut Gfx,
        texts: Option<&Texts>,
        s: &str,
        x: f32,
        y: f32,
        px: f32,
        color: [u8; 4],
        align: Align,
    ) -> f32 {
        let runs = markup(texts, s, g.upper);
        let spx = px * self.scale;
        let width: f32 = runs
            .iter()
            .map(|(t, _)| g.fonts.measure(&mut g.atlas, t, spx))
            .sum::<f32>()
            / self.scale;
        let x0 = match align {
            Align::Left => x,
            Align::Center => x - width / 2.0,
            Align::Right => x - width,
        };
        let mut pen = self.x(x0).round();
        let base = (self.y(y) + g.fonts.ascent(spx)).round();
        for (t, col) in runs {
            let c = match col {
                Some((cx, cy)) => g.pal(cx, cy),
                None => color,
            };
            let c = [c[0], c[1], c[2], color[3]];
            for ch in t.chars() {
                let gl = g.fonts.glyph(&mut g.atlas, ch, spx);
                if let Some(r) = gl.region {
                    self.quads.push(Quad {
                        x: (pen + gl.dx as f32).round(),
                        y: base + gl.dy as f32,
                        w: gl.w as f32,
                        h: gl.h as f32,
                        region: r,
                        color: c,
                    });
                }
                pen += gl.advance;
            }
        }
        width
    }

    /// A line in the large bitmap font at `scale` (1 or 0.5), one
    /// character every `advance` logical pixels, centred on `cx`, image
    /// tops at `top` (title cards, `ui.md` §7). False when a character
    /// has no image (the caller falls back to [`Ui::text`]).
    #[allow(clippy::too_many_arguments)]
    pub fn art_line(
        &mut self,
        g: &mut Gfx,
        s: &str,
        cx: f32,
        top: f32,
        scale: f32,
        advance: f32,
        color: [u8; 4],
    ) -> bool {
        use crate::pixelfont::{LARGE, image};
        if !g.fonts.pixel || !g.art.available() {
            return false;
        }
        let chars: Vec<char> = s.chars().collect();
        if chars.iter().any(|&c| c != ' ' && image(LARGE, c).is_none()) {
            return false;
        }
        let x0 = cx - chars.len() as f32 * advance / 2.0;
        for (i, &c) in chars.iter().enumerate() {
            let Some(n) = image(LARGE, c) else { continue };
            let Some(a) = g.art.get(&mut g.atlas, n) else {
                return false;
            };
            let s2 = self.scale * scale;
            // Small centring correction for the half-size letters.
            let dx = (advance - a.w as f32 * scale) / 2.0;
            self.quads.push(Quad {
                x: self.x(x0 + i as f32 * advance + dx).round(),
                y: self.y(top).round(),
                w: a.w as f32 * s2,
                h: a.h as f32 * s2,
                region: a.region,
                color,
            });
        }
        true
    }

    /// Logical width of `s` (markup applied).
    pub fn measure(&self, g: &mut Gfx, texts: Option<&Texts>, s: &str, px: f32) -> f32 {
        let spx = px * self.scale;
        markup(texts, s, g.upper)
            .iter()
            .map(|(t, _)| g.fonts.measure(&mut g.atlas, t, spx))
            .sum::<f32>()
            / self.scale
    }
}

/// Splits text into runs with their colour switches (`$x,y`), replacing
/// `#key` with the language text and dropping `@` glyph codes and `^`.
pub fn markup(texts: Option<&Texts>, s: &str, upper: bool) -> Vec<(String, Option<(u32, u32)>)> {
    let mut out: Vec<(String, Option<(u32, u32)>)> = vec![(String::new(), None)];
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '$'
            && i + 3 < chars.len()
            && chars[i + 1].is_ascii_digit()
            && chars[i + 2] == ','
            && chars[i + 3].is_ascii_digit()
        {
            let col = (
                chars[i + 1].to_digit(10).unwrap(),
                chars[i + 3].to_digit(10).unwrap(),
            );
            out.push((String::new(), Some(col)));
            i += 4;
            continue;
        }
        if (c == '#' || c == '@') && i + 1 < chars.len() && !chars[i + 1].is_whitespace() {
            let mut j = i + 1;
            while j < chars.len() && !chars[j].is_whitespace() {
                j += 1;
            }
            let key: String = chars[i + 1..j].iter().collect();
            if c == '#'
                && let Some(t) = texts.and_then(|t| t.opt(&key))
            {
                let sub = markup(texts, &t, upper);
                for (t, col) in sub {
                    match col {
                        Some(_) => out.push((t, col)),
                        None => out.last_mut().unwrap().0.push_str(&t),
                    }
                }
            }
            i = j;
            continue;
        }
        if c == '^' {
            i += 1;
            continue;
        }
        let last = out.last_mut().unwrap();
        if upper {
            last.0.extend(c.to_uppercase());
        } else {
            last.0.push(c);
        }
        i += 1;
    }
    out.retain(|(t, _)| !t.is_empty());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_colours() {
        let r = markup(None, "$4,1Baba $0,3is you", false);
        assert_eq!(
            r,
            vec![
                ("Baba ".to_string(), Some((4, 1))),
                ("is you".to_string(), Some((0, 3)))
            ]
        );
        let r = markup(None, "a #missing b^c", true);
        assert_eq!(r, vec![("A  BC".to_string(), None)]);
    }
}
