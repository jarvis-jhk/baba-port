//! Level background particles (`docs/spec/presentation.md` §10): a level's
//! `particles` style creates a fixed number of drifting particles at random
//! places, moving `velocity × 0.2` px per frame (measured, like every
//! engine particle) and wrapping around the level. Sizes and colours per
//! style follow the spec's table; the images are the closest engine art.

use baba_sim::CosmeticRng;

use super::kinds::STEP;

/// One background particle.
#[derive(Clone, Debug)]
pub struct BgParticle {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub size: f32,
    pub colour: (u8, u8),
    pub images: &'static [u32; 3],
    pub phase: u8,
    /// Sway phase (radians) and amplitude (px per frame).
    sway: f32,
    sway_amp: f32,
    pub front: bool,
}

const RING: [u32; 3] = [1304, 1305, 1306];
const DOT: [u32; 3] = [456, 457, 456];
const SPARK: [u32; 3] = [1297, 1299, 1300];
const PUFF: [u32; 3] = [701, 702, 703];
const STAR: [u32; 3] = [1302, 1303, 1302];

/// Creates the particles of `style` for a level of `w`×`h` pixels.
pub fn create(style: &str, w: f32, h: f32, rng: &mut CosmeticRng) -> Vec<BgParticle> {
    let mut f = |lo: f32, hi: f32| lo + (hi - lo) * rng.range(0, 1000) as f32 / 1000.0;
    let amount = match style {
        "bubbles" | "soot" | "snow" | "smoke" | "leaves" => 30,
        "sparks" | "stars" | "world" => 40,
        "dust" | "rain" => 50,
        "clouds" => 90,
        "pollen" => 20,
        "glitter" => 60,
        _ => return Vec::new(),
    };
    let mut out = Vec::with_capacity(amount);
    for i in 0..amount {
        let (x, y) = (f(0.0, w), f(0.0, h));
        let first_half = i < amount / 2;
        let (vx, vy, size, colour, images, front): (
            f32,
            f32,
            f32,
            (u8, u8),
            &'static [u32; 3],
            bool,
        ) = match style {
            "bubbles" => {
                let vy = -(f(1.0, 3.99).floor());
                (0.0, vy, -vy * 0.33, (1, 0), &RING, false)
            }
            "soot" => {
                let vy = -(f(1.0, 3.99).floor());
                (0.0, vy, -vy * 0.33, (0, 1), &PUFF, false)
            }
            "sparks" => {
                let vy = -(f(1.0, 3.99).floor());
                (
                    0.0,
                    vy,
                    -vy * 0.23 * 2.0,
                    (2, f(2.0, 4.99) as u8),
                    &SPARK,
                    false,
                )
            }
            "dust" => {
                let vy = -(f(1.0, 3.99).floor());
                (0.0, vy, -vy * 0.363, (1, 0), &DOT, false)
            }
            "snow" => {
                let vx = f(5.0, 9.0);
                (vx, vx.abs() * f(0.1, 0.5), f(0.2, 0.9), (0, 3), &DOT, true)
            }
            "clouds" => {
                let s = f(0.7, 1.3);
                (-s, -s, s * 2.0, (4, 3), &PUFF, false)
            }
            "smoke" => (0.0, -1.0, f(0.7, 1.3), (1, 0), &PUFF, false),
            "pollen" => (f(-2.0, 2.0), f(2.0, 4.0), f(0.4, 1.0), (1, 0), &DOT, false),
            "stars" => {
                let anim = if first_half { 0.0 } else { 1.0 };
                let c = if first_half { (3, 2) } else { (1, 3) };
                (anim + f(0.0, 0.5) + 0.05, 0.0, f(0.4, 1.0), c, &STAR, false)
            }
            "glitter" => {
                let c = match rng_pick(&mut f) {
                    0 => (4, 2),
                    1 => (0, 3),
                    _ => (3, 1),
                };
                (0.0, 0.0, f(0.4, 1.0), c, &SPARK, false)
            }
            "leaves" => {
                let c = if rng_pick(&mut f) == 0 {
                    (6, 3)
                } else {
                    (6, 0)
                };
                (f(-3.0, -1.0), f(0.0, 0.5), f(0.6, 1.2), c, &DOT, false)
            }
            "rain" => (0.0, 80.0, f(0.6, 1.0), (3, 2), &DOT, true),
            _ => (0.0, 0.0, f(0.2, 1.0), (4, 0), &RING, false),
        };
        out.push(BgParticle {
            x,
            y,
            vx,
            vy,
            size,
            colour,
            images,
            phase: f(0.0, 2.99) as u8,
            sway: f(0.0, std::f32::consts::TAU),
            sway_amp: if matches!(style, "bubbles" | "soot" | "dust" | "sparks") {
                0.3
            } else {
                0.0
            },
            front,
        });
    }
    out
}

fn rng_pick(f: &mut impl FnMut(f32, f32) -> f32) -> u8 {
    f(0.0, 3.99) as u8
}

/// One frame of motion with wrap-around in a `w`×`h` area.
pub fn step(ps: &mut [BgParticle], w: f32, h: f32) {
    for p in ps {
        p.sway += 0.05;
        p.x += p.vx * STEP + p.sway.sin() * p.sway_amp;
        p.y += p.vy * STEP;
        let m = 16.0;
        if p.x < -m {
            p.x += w + 2.0 * m;
        } else if p.x > w + m {
            p.x -= w + 2.0 * m;
        }
        if p.y < -m {
            p.y += h + 2.0 * m;
        } else if p.y > h + m {
            p.y -= h + 2.0 * m;
        }
    }
}
