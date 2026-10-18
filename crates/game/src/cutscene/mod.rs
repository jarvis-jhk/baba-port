//! Scripted sequences of the base world (`docs/spec/progression.md` §10,
//! the measurements in `presentation.md` §15.20–§15.24): the intro on the
//! first start of `0level`, the END sequence (blossoms, the closing flower,
//! the island, THE END, the logo credits) and the All Is Done sequence.
//!
//! A cutscene runs on top of a level view: it hides or detaches units of
//! the level's scene (`scene::Overrides`), draws its own pieces (scattered
//! objects, flowers) as rotated quads over the view, and asks the
//! presenter for sounds and particle bursts. Nothing here changes the
//! simulation; the presenter applies the save flags and turns.

pub mod ending;
pub mod intro;

use baba_render::{Atlas, Quad, RegionId, RgbaImage};
use baba_sim::CosmeticRng;

pub use ending::{DoneScene, EndScene, EndShow};
pub use intro::Intro;

/// A sound to play now: name and playback rate (1 = normal).
pub type Cue = (String, f32);

/// Textures the cutscenes draw with (our own shapes, not game art): a
/// petal (white tip, grey base, tinted like sprites) and a hard disc.
#[derive(Clone, Copy, Debug)]
pub struct Tex {
    pub petal: RegionId,
    pub disc: RegionId,
}

impl Tex {
    /// Inserts the textures into `atlas` (once; keep the result).
    pub fn new(atlas: &mut Atlas) -> Tex {
        Tex {
            petal: atlas.insert(&petal_image()),
            disc: atlas.insert(&disc_image(32)),
        }
    }
}

/// A petal 12×32 px pointing up: an elongated rounded shape, white towards
/// the tip and grey (115) at the base, so tinting gives the two tones of
/// the original's flowers.
fn petal_image() -> RgbaImage {
    let (w, h) = (12u32, 32u32);
    let mut img = RgbaImage::new(w, h);
    for y in 0..h {
        let t = (y as f32 + 0.5) / h as f32;
        let half = 6.0 * (std::f32::consts::PI * t).sin().powf(0.6);
        let shade = if t > 0.72 { 115 } else { 255 };
        for x in 0..w {
            let dx = (x as f32 + 0.5 - w as f32 / 2.0).abs();
            if dx <= half {
                let i = ((y * w + x) * 4) as usize;
                img.data[i..i + 4].copy_from_slice(&[shade, shade, shade, 255]);
            }
        }
    }
    img
}

/// A filled disc of diameter `d` px.
fn disc_image(d: u32) -> RgbaImage {
    let mut img = RgbaImage::new(d, d);
    let r = d as f32 / 2.0;
    for y in 0..d {
        for x in 0..d {
            let (dx, dy) = (x as f32 + 0.5 - r, y as f32 + 0.5 - r);
            if dx * dx + dy * dy <= r * r {
                let i = ((y * d + x) * 4) as usize;
                img.data[i..i + 4].copy_from_slice(&[255; 4]);
            }
        }
    }
    img
}

/// Maps level pixels to screen pixels: `(x0, y0, k)` = screen position of
/// the level's top-left corner and its scale.
#[derive(Clone, Copy, Debug)]
pub struct Place {
    pub x0: f32,
    pub y0: f32,
    pub k: f32,
}

impl Place {
    pub fn at(&self, x: f32, y: f32) -> (f32, f32) {
        (self.x0 + x * self.k, self.y0 + y * self.k)
    }
}

/// Which flower (`progression.md` §10, measured on captures).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowerKind {
    /// Intro: over BABA / WALL / ROCK / FLAG (8 petals).
    Word,
    /// Intro: over IS / STOP / PUSH / WIN (6 small petals).
    Small,
    /// The many-petalled flower over YOU (intro) and at the END tile.
    You,
    /// The flower of the island screen (16 petals) and the area flowers.
    Island,
}

/// Petal count, petal length/width, inner radius, bud radius, centre
/// radius (source pixels).
fn shape(k: FlowerKind) -> (usize, f32, f32, f32, f32, f32) {
    match k {
        FlowerKind::Word => (8, 28.0, 10.0, 8.0, 13.0, 5.0),
        FlowerKind::Small => (6, 15.0, 7.0, 4.0, 8.0, 3.0),
        FlowerKind::You => (24, 36.0, 10.0, 9.0, 15.0, 6.0),
        FlowerKind::Island => (16, 26.0, 9.0, 7.0, 12.0, 5.0),
    }
}

/// Frames the petals take to open.
pub const OPEN_FRAMES: u32 = 20;

/// A flower of the cutscenes: a bud that opens, may grow, and finally
/// scatters its petals ("reveal") while the centre shrinks away.
#[derive(Clone, Debug)]
pub struct Flower {
    pub kind: FlowerKind,
    /// Centre in level pixels.
    pub x: f32,
    pub y: f32,
    pub colour: (u8, u8),
    /// Name used by the scripts (`baba`, `is`, `you`, …).
    pub tag: String,
    age: u32,
    open: Option<u32>,
    reveal: Option<u32>,
    /// Growth (END phase 2): target scale and current scale.
    grow: (f32, f32),
    spin: f32,
    seed: u64,
}

impl Flower {
    pub fn new(kind: FlowerKind, x: f32, y: f32, colour: (u8, u8), tag: &str, seed: u64) -> Flower {
        Flower {
            kind,
            x,
            y,
            colour,
            tag: tag.to_string(),
            age: 0,
            open: None,
            reveal: None,
            grow: (1.0, 1.0),
            spin: 0.0,
            seed,
        }
    }

    /// Already open (island flowers).
    pub fn opened(mut self) -> Flower {
        self.open = Some(OPEN_FRAMES);
        self
    }

    /// Drawn at `s` times its size.
    pub fn scaled(mut self, s: f32) -> Flower {
        self.grow = (s, s);
        self
    }

    pub fn open(&mut self) {
        if self.open.is_none() {
            self.open = Some(0);
        }
    }

    /// Back to a bud (the END flower closing).
    pub fn close(&mut self) {
        self.open = None;
    }

    pub fn reveal(&mut self) {
        if self.reveal.is_none() {
            self.reveal = Some(0);
        }
    }

    /// Grows toward `scale` (eased, a tenth of the way per frame).
    pub fn grow_to(&mut self, scale: f32) {
        self.grow.0 = scale;
    }

    /// Gone after its reveal.
    pub fn finished(&self) -> bool {
        self.reveal.is_some_and(|r| r > 40)
    }

    pub fn tick(&mut self) {
        self.age += 1;
        if let Some(o) = &mut self.open {
            *o += 1;
        }
        if let Some(r) = &mut self.reveal {
            *r += 1;
        }
        self.grow.1 += (self.grow.0 - self.grow.1) * 0.1;
        self.spin += 0.2;
    }

    /// Quads (screen pixels, rotation) of the flower.
    pub fn quads(
        &self,
        tex: &Tex,
        pal: &dyn Fn((u8, u8)) -> [u8; 4],
        p: &Place,
    ) -> Vec<(Quad, f32)> {
        let (n, len, wid, inner, bud, centre) = shape(self.kind);
        let mut out = Vec::new();
        let col = pal(self.colour);
        let o = self
            .open
            .map_or(0.0, |o| (o as f32 / OPEN_FRAMES as f32).min(1.0));
        let s = self.grow.1;
        let (cx, cy) = p.at(self.x, self.y);
        let k = p.k * s;
        let disc = |out: &mut Vec<(Quad, f32)>, r: f32, c: [u8; 4]| {
            if r > 0.2 {
                out.push((
                    Quad {
                        x: cx - r * k,
                        y: cy - r * k,
                        w: 2.0 * r * k,
                        h: 2.0 * r * k,
                        region: tex.disc,
                        color: c,
                    },
                    0.0,
                ));
            }
        };
        let rv = self.reveal.map(|r| r as f32);
        // Petals: two rings for the many-petalled flower.
        if o > 0.0 {
            let mut rng = CosmeticRng::new(self.seed);
            for i in 0..n {
                let ring2 = self.kind == FlowerKind::You && i % 2 == 1;
                let a = (i as f32 / n as f32) * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                let l = if ring2 { len * 0.8 } else { len };
                let mut d = (inner + l / 2.0) * o;
                let mut turn = a + std::f32::consts::FRAC_PI_2;
                let mut c = col;
                if ring2 {
                    c = [
                        (c[0] as f32 * 0.8) as u8,
                        (c[1] as f32 * 0.8) as u8,
                        (c[2] as f32 * 0.8) as u8,
                        255,
                    ];
                }
                if let Some(r) = rv {
                    // Petals fly out, spinning, and fade (measured: gone
                    // after about 30 frames).
                    let v = 6.0 + rng.range(0, 40) as f32 / 10.0;
                    d += v * (1.0 - 0.93f32.powf(r)) / 0.07;
                    turn += r * (rng.range(-10, 10) as f32 / 100.0);
                    let alpha = (1.0 - r / 30.0).clamp(0.0, 1.0);
                    c[3] = (alpha * 255.0) as u8;
                    if c[3] == 0 {
                        continue;
                    }
                }
                let (w, h) = (wid * o * k, l * o * k);
                let (px, py) = (cx + a.cos() * d * k, cy + a.sin() * d * k);
                out.push((
                    Quad {
                        x: px - w / 2.0,
                        y: py - h / 2.0,
                        w,
                        h,
                        region: tex.petal,
                        color: c,
                    },
                    turn,
                ));
            }
        }
        // The bud closes over the centre until the petals open.
        if rv.is_none() {
            disc(&mut out, bud * (1.0 - o) + centre * o, col);
        }
        let cr = match rv {
            Some(r) => centre * (1.0 - r * 0.03).max(0.0),
            None if o > 0.0 => centre,
            None => 2.0,
        };
        disc(&mut out, cr, pal((2, 4)));
        out
    }
}

/// The decorative flowers of a level (`[specials] flower,<x>,<y>,<size>`,
/// `progression.md` §3.6: palette colour (x, y), size 1 or 2; the meta
/// maps): open many-petalled flowers centred on their cells (measured on
/// `304level`: radius ≈ 28 / 55 px).
pub fn decor_flowers(info: &baba_formats::LevelInfo) -> Vec<Flower> {
    info.specials
        .iter()
        .filter_map(|s| {
            let mut f = s.data.strip_prefix("flower,")?.split(',');
            let mut n = || f.next().and_then(|v| v.trim().parse::<i32>().ok());
            let (cx, cy, size) = (n()?, n()?, n().unwrap_or(1));
            let (x, y) = ((s.x as f32 + 0.5) * 24.0, (s.y as f32 + 0.5) * 24.0);
            let seed = (s.x * 31 + s.y) as u64;
            Some(
                Flower::new(
                    FlowerKind::You,
                    x,
                    y,
                    (cx.clamp(0, 6) as u8, cy.clamp(0, 4) as u8),
                    "decor",
                    seed,
                )
                .opened()
                .scaled(if size >= 2 { 1.2 } else { 0.6 }),
            )
        })
        .collect()
}

/// The running cutscene of the single-player view.
pub enum Cutscene {
    Intro(Box<Intro>),
    End(Box<EndScene>),
    Done(Box<DoneScene>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flower_opens_and_scatters() {
        let mut f = Flower::new(FlowerKind::Word, 100.0, 100.0, (1, 3), "baba", 1);
        let tex = Tex {
            petal: RegionId(0),
            disc: RegionId(0),
        };
        let pal = |_c: (u8, u8)| [255u8; 4];
        let p = Place {
            x0: 0.0,
            y0: 0.0,
            k: 1.0,
        };
        assert_eq!(f.quads(&tex, &pal, &p).len(), 2, "bud and centre");
        f.open();
        for _ in 0..OPEN_FRAMES {
            f.tick();
        }
        assert_eq!(f.quads(&tex, &pal, &p).len(), 8 + 2);
        f.reveal();
        for _ in 0..45 {
            f.tick();
        }
        assert!(f.finished());
    }

    #[test]
    fn textures_have_shape() {
        let p = petal_image();
        let ink = p.data.chunks(4).filter(|c| c[3] > 0).count();
        assert!(ink > 150 && ink < 12 * 32, "{ink}");
        let d = disc_image(32);
        let ink = d.data.chunks(4).filter(|c| c[3] > 0).count();
        assert!((ink as f32 - std::f32::consts::PI * 256.0).abs() < 40.0);
    }
}
