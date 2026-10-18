//! The blob iris between screens (`docs/spec/presentation.md` §11.1,
//! measured on the original): 36 small blobs on an ellipse 1.5× the half
//! screen and 16 big ones (12 on an ellipse 1.8×, 4 in the corners) fly
//! toward the centre for a fixed number of frames, wait while the screen
//! is covered (action, title card), then fly back out with their velocity
//! reversed. Positions are in a screen of `w`×`h` pixels; velocities move
//! `× STEP` per frame like every engine particle.

use baba_sim::CosmeticRng;

use super::kinds::STEP;

/// Frames of the closing flight (normal / fast) and of the opening one.
pub const CLOSE_FRAMES: u32 = 41;
pub const OPEN_FRAMES: u32 = 50;
const FAST: f32 = 22.5 / 50.0;

/// Engine image of a transition blob (a 128 px disc).
pub const BLOB_IMAGE: u32 = 234;
/// Scale of the big blobs.
pub const BIG_SCALE: f32 = 1.15;

#[derive(Clone, Copy, Debug)]
pub struct Blob {
    pub x: f32,
    pub y: f32,
    vx: f32,
    vy: f32,
    pub big: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Closing(u32),
    Covered,
    Opening(u32),
    Done,
}

/// One running iris.
#[derive(Clone, Debug)]
pub struct Iris {
    pub blobs: Vec<Blob>,
    pub phase: Phase,
    close: u32,
    open: u32,
}

impl Iris {
    /// Blobs placed for a `w`×`h` screen (`fast`: the restart variant).
    pub fn new(w: f32, h: f32, fast: bool, rng: &mut CosmeticRng) -> Iris {
        let (sw, sh) = (w * 0.5, h * 0.5);
        let mut blobs = Vec::with_capacity(52);
        let toward = |x: f32, y: f32, spd: f32, xmul: f32| {
            let d = (sh - y).atan2(sw - x);
            (d.cos() * spd * xmul, d.sin() * spd)
        };
        let steps = if fast { 22.5 } else { 50.0 };
        for i in 0..36 {
            let a = (i as f32 * 10.0).to_radians();
            let (x, y) = (sw + a.cos() * sw * 1.5, sh - a.sin() * sh * 1.5);
            let dist = ((y - sh).powi(2) + (x - sw).powi(2)).sqrt();
            let spd = dist / steps * 5.0 + rng.range(-9, 7) as f32;
            let (vx, vy) = toward(x, y, spd, 1.0);
            blobs.push(Blob {
                x,
                y,
                vx,
                vy,
                big: false,
            });
        }
        let steps = if fast { 36.0 } else { 72.0 };
        let big = |x: f32, y: f32, blobs: &mut Vec<Blob>| {
            let dist = ((y - sh).powi(2) + (x - sw).powi(2)).sqrt();
            let (vx, vy) = toward(x, y, dist / steps * 5.0, 1.01);
            blobs.push(Blob {
                x,
                y,
                vx,
                vy,
                big: true,
            });
        };
        for i in 1..=12 {
            let a = (i as f32 * 30.0).to_radians();
            big(sw + a.cos() * sw * 1.8, sh - a.sin() * sh * 1.8, &mut blobs);
        }
        for (lx, ly) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            big(sw + lx * sw * 1.8, sh + ly * sh * 1.8, &mut blobs);
        }
        let k = if fast { FAST } else { 1.0 };
        Iris {
            blobs,
            phase: Phase::Closing(0),
            close: (CLOSE_FRAMES as f32 * k).round() as u32,
            open: (OPEN_FRAMES as f32 * k).round() as u32,
        }
    }

    /// One frame. Returns true on the frame the screen becomes covered.
    pub fn tick(&mut self) -> bool {
        match self.phase {
            Phase::Closing(t) => {
                for b in &mut self.blobs {
                    b.x += b.vx * STEP;
                    b.y += b.vy * STEP;
                }
                if t + 1 >= self.close {
                    self.phase = Phase::Covered;
                    return true;
                }
                self.phase = Phase::Closing(t + 1);
            }
            Phase::Opening(t) => {
                for b in &mut self.blobs {
                    b.x -= b.vx * STEP;
                    b.y -= b.vy * STEP;
                }
                self.phase = if t + 1 >= self.open {
                    Phase::Done
                } else {
                    Phase::Opening(t + 1)
                };
            }
            Phase::Covered | Phase::Done => {}
        }
        false
    }

    /// Starts flying out (from the covered state).
    pub fn release(&mut self) {
        if self.phase == Phase::Covered {
            self.phase = Phase::Opening(0);
        }
    }

    pub fn done(&self) -> bool {
        self.phase == Phase::Done
    }

    pub fn covered(&self) -> bool {
        self.phase == Phase::Covered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closes_covers_and_opens() {
        let mut rng = CosmeticRng::new(1);
        let mut iris = Iris::new(854.0, 480.0, false, &mut rng);
        assert_eq!(iris.blobs.len(), 52);
        let mut frames = 0;
        while !iris.tick() {
            frames += 1;
        }
        assert_eq!(frames + 1, CLOSE_FRAMES);
        // Small blobs end near the centre (41/50 of the way plus jitter).
        let b = iris.blobs[0];
        assert!((b.x - 427.0).abs() < 140.0, "{b:?}");
        iris.release();
        for _ in 0..OPEN_FRAMES {
            iris.tick();
        }
        assert!(iris.done());
    }
}
