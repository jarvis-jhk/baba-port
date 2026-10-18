//! The intro (`progression.md` §10.4, timings in `presentation.md`
//! §15.21): `0level` starts with its objects scattered over the screen,
//! bobbing and swaying in a dark colour, and its words hidden. Flowers
//! open over BABA, WALL, ROCK and FLAG in turn, scatter their petals and
//! reveal the word; the objects of that word shake and glide to their
//! cells in their own colour. Small flowers then reveal IS / STOP / PUSH /
//! WIN, a large flower YOU, the floor tiles shake into place and input is
//! released.

use std::collections::{BTreeMap, BTreeSet};

use baba_render::{Quad, RegionId};
use baba_sim::{CosmeticRng, Level};

use super::{Cue, Flower, FlowerKind, Place, Tex};
use crate::fx::{CELL, LevelFx};
use crate::scene::Overrides;

/// The four word flowers: word, first frame, sound number.
const WORDS: [(&str, u32); 4] = [("baba", 160), ("wall", 530), ("rock", 900), ("flag", 1220)];
/// After a word flower appears: it opens, reveals, the objects glide.
const OPEN_AT: u32 = 70;
const REVEAL_AT: u32 = 200;
const PLACE_AT: u32 = 270;
/// Colour of the scattered objects (measured: palette (1,1)).
const DARK: (u8, u8) = (1, 1);

/// A scattered object (level pixels, centre).
#[derive(Clone, Debug)]
struct Scatter {
    name: String,
    x: f32,
    y: f32,
    period: f32,
    phase: f32,
    rot_amp: f32,
    rot_period: f32,
    rot_phase: f32,
    /// Gliding to the cell: current position and angle.
    glide: Option<(f32, f32, f32)>,
    shaking: bool,
    cell: (f32, f32),
}

impl Scatter {
    /// Drawn centre and angle (radians) at frame `t`.
    fn pose(&self, t: u32, jitter: (f32, f32)) -> (f32, f32, f32) {
        if let Some((x, y, a)) = self.glide {
            return (x, y, a);
        }
        let tt = t as f32;
        let dy = 12.0 * (std::f32::consts::TAU * tt / self.period + self.phase).sin();
        let a =
            self.rot_amp * (std::f32::consts::TAU * tt / self.rot_period + self.rot_phase).sin();
        (self.x + jitter.0, self.y + dy + jitter.1, a.to_radians())
    }
}

/// The intro cutscene.
pub struct Intro {
    /// Frames since the intro started (all phases).
    pub frame: u32,
    phase: u8,
    t: u32,
    scatter: BTreeMap<i32, Scatter>,
    hidden: BTreeSet<i32>,
    texts: Vec<(i32, String, (f32, f32))>,
    flowers: Vec<Flower>,
    rng: CosmeticRng,
    /// Input released (the level is playable).
    pub released: bool,
    pub done: bool,
}

impl Intro {
    /// Starts on a freshly loaded `0level`.
    pub fn new(level: &Level, seed: u64) -> Intro {
        let mut rng = CosmeticRng::new(seed);
        let (w, h) = (level.width() as f32 * CELL, level.height() as f32 * CELL);
        let mut scatter = BTreeMap::new();
        let mut hidden = BTreeSet::new();
        let mut texts = Vec::new();
        for u in level.units() {
            let cell = ((u.x as f32 + 0.5) * CELL, (u.y as f32 + 0.5) * CELL);
            if u.name.starts_with("text_") {
                hidden.insert(u.id);
                texts.push((u.id, u.name.clone(), cell));
                continue;
            }
            let x = rng.range(CELL as i32, (w - CELL) as i32) as f32;
            let y = rng.range(CELL as i32, (h - CELL) as i32) as f32;
            scatter.insert(
                u.id,
                Scatter {
                    name: u.name.clone(),
                    x,
                    y,
                    period: rng.range(320, 600) as f32,
                    phase: rng.range(0, 628) as f32 / 100.0,
                    rot_amp: rng.range(10, 35) as f32,
                    rot_period: rng.range(300, 600) as f32,
                    rot_phase: rng.range(0, 628) as f32 / 100.0,
                    glide: None,
                    shaking: false,
                    cell,
                },
            );
        }
        Intro {
            frame: 0,
            phase: 2,
            t: 0,
            scatter,
            hidden,
            texts,
            flowers: Vec::new(),
            rng,
            released: false,
            done: false,
        }
    }

    fn texts_named(&self, name: &str) -> Vec<(i32, (f32, f32))> {
        self.texts
            .iter()
            .filter(|(_, n, _)| n == name)
            .map(|(id, _, c)| (*id, *c))
            .collect()
    }

    fn flowers_tagged(&mut self, tag: &str) -> impl Iterator<Item = &mut Flower> {
        let tag = tag.to_string();
        self.flowers.iter_mut().filter(move |f| f.tag == tag)
    }

    fn glide(&mut self, name: &str) {
        for s in self.scatter.values_mut() {
            if s.name == name && s.glide.is_none() {
                s.shaking = false;
                let (x, y, a) = s.pose(self.frame, (0.0, 0.0));
                s.glide = Some((x, y, a));
            }
        }
    }

    /// Skips to the end (hold to skip).
    pub fn skip(&mut self) {
        self.scatter.clear();
        self.hidden.clear();
        self.flowers.clear();
        self.released = true;
        self.done = true;
    }

    /// One frame; returns the sounds to play. Particle bursts go to `fx`.
    pub fn tick(&mut self, fx: &mut LevelFx) -> Vec<Cue> {
        let mut cues = Vec::new();
        if self.done {
            return cues;
        }
        self.frame += 1;
        self.t += 1;
        let t = self.t;
        for f in &mut self.flowers {
            f.tick();
        }
        self.flowers.retain(|f| !f.finished());
        // Gliding objects close 30 % of the way per frame (measured).
        let mut placed = Vec::new();
        for (id, s) in &mut self.scatter {
            if let Some((x, y, a)) = &mut s.glide {
                *x += (s.cell.0 - *x) * 0.3;
                *y += (s.cell.1 - *y) * 0.3;
                *a *= 0.7;
                if (s.cell.0 - *x).abs() < 0.5 && (s.cell.1 - *y).abs() < 0.5 {
                    placed.push(*id);
                }
            }
        }
        for id in placed {
            self.scatter.remove(&id);
        }
        match self.phase {
            2 => {
                for (i, (word, start)) in WORDS.iter().enumerate() {
                    let n = i + 1;
                    if t == *start {
                        for (_, (x, y)) in self.texts_named(&format!("text_{word}")) {
                            let seed = self.rng.range(0, 1 << 30) as u64;
                            self.flowers.push(Flower::new(
                                FlowerKind::Word,
                                x,
                                y,
                                (1, 3),
                                word,
                                seed,
                            ));
                        }
                        cues.push((format!("intro_reveal_{n}f"), 1.0));
                    } else if t == start + OPEN_AT {
                        self.flowers_tagged(word).for_each(|f| f.open());
                    } else if t == start + REVEAL_AT {
                        self.flowers_tagged(word).for_each(|f| f.reveal());
                        for (id, _) in self.texts_named(&format!("text_{word}")) {
                            self.hidden.remove(&id);
                        }
                        for s in self.scatter.values_mut() {
                            if s.name == *word {
                                s.shaking = true;
                            }
                        }
                        cues.push((format!("intro_disappear_{n}"), 1.0));
                    } else if t == start + PLACE_AT {
                        self.glide(word);
                        let pitch = (44000 + self.rng.range(-9000, 9000)) as f32 / 44000.0;
                        cues.push(("whoooooosh_loud".into(), pitch));
                        if *word == "flag" {
                            self.phase = 3;
                            self.t = 0;
                        }
                    }
                    if t > start + REVEAL_AT && t < start + PLACE_AT && t.is_multiple_of(3) {
                        self.bling(word, fx);
                    }
                }
            }
            3 => {
                // Small flowers every 20 frames on IS/STOP/PUSH/WIN, with a
                // pause after the fourth (measured: 160–220, 300–340).
                if t.is_multiple_of(20) {
                    let mut k = (t / 20) as i32 - 7;
                    if k > 7 {
                        k -= 3;
                    } else if (5..=7).contains(&k) {
                        k = -1;
                    }
                    if (1..8).contains(&k) {
                        let small = self.smalls();
                        if let Some(&(_, (x, y))) = small.get(k as usize - 1) {
                            let seed = self.rng.range(0, 1 << 30) as u64;
                            self.flowers.push(Flower::new(
                                FlowerKind::Small,
                                x,
                                y,
                                (1, 3),
                                "is",
                                seed,
                            ));
                        }
                        cues.push((format!("intro_flower_{k}"), 1.0));
                        if k == 7 {
                            self.phase = 4;
                            self.t = 0;
                        }
                    }
                }
            }
            4 => {
                if t.is_multiple_of(20) {
                    let k = (t / 20) as i32 - 1;
                    if (1..8).contains(&k) {
                        if let Some(f) = self
                            .flowers
                            .iter_mut()
                            .filter(|f| f.tag == "is")
                            .nth(k as usize - 1)
                        {
                            f.open();
                        }
                        if k == 7 {
                            self.phase = 5;
                            self.t = 0;
                        }
                    }
                }
            }
            5 => {
                if t == 100 {
                    self.flowers_tagged("is").for_each(|f| f.reveal());
                    for (id, _) in self.smalls() {
                        self.hidden.remove(&id);
                    }
                    cues.push(("intro_disappear_1".into(), 1.0));
                    self.phase = 6;
                    self.t = 0;
                }
            }
            6 => {
                if t == 80 {
                    cues.push(("intro_reverse_baba".into(), 1.0));
                }
                if t == 110 {
                    for (_, (x, y)) in self.texts_named("text_you") {
                        let seed = self.rng.range(0, 1 << 30) as u64;
                        self.flowers
                            .push(Flower::new(FlowerKind::You, x, y, (4, 1), "you", seed));
                    }
                } else if t == 280 {
                    self.flowers_tagged("you").for_each(|f| f.open());
                }
                if t > 380 {
                    for s in self.scatter.values_mut() {
                        if s.name == "tile" {
                            s.shaking = true;
                        }
                    }
                    if t.is_multiple_of(5) {
                        self.bling("tile", fx);
                    }
                }
                if t == 620 {
                    self.flowers_tagged("you").for_each(|f| f.reveal());
                    self.glide("tile");
                    for (id, _) in self.texts_named("text_you") {
                        self.hidden.remove(&id);
                    }
                    // Anything left (names without a flower) settles too.
                    let rest: Vec<String> = self.scatter.values().map(|s| s.name.clone()).collect();
                    for n in rest {
                        self.glide(&n);
                    }
                    self.hidden.clear();
                    self.released = true;
                    self.phase = 7;
                    self.t = 0;
                }
            }
            _ => {
                if self.scatter.is_empty() && self.flowers.is_empty() {
                    self.done = true;
                }
            }
        }
        cues
    }

    /// IS / STOP / PUSH / WIN texts in unit order.
    fn smalls(&self) -> Vec<(i32, (f32, f32))> {
        self.texts
            .iter()
            .filter(|(_, n, _)| {
                matches!(
                    n.as_str(),
                    "text_is" | "text_stop" | "text_push" | "text_win"
                )
            })
            .map(|(id, _, c)| (*id, *c))
            .collect()
    }

    /// One bling particle at each shaking object named `name` (colour
    /// (1,1), measured).
    fn bling(&mut self, name: &str, fx: &mut LevelFx) {
        let frame = self.frame;
        let pts: Vec<(f32, f32)> = self
            .scatter
            .values()
            .filter(|s| s.name == name && s.glide.is_none())
            .map(|s| {
                let (x, y, _) = s.pose(frame, (0.0, 0.0));
                (x, y)
            })
            .collect();
        for p in pts {
            fx.burst_at("bling", p, 1, (1, 1));
        }
    }

    /// How the level's units are drawn now: words hidden until revealed,
    /// scattered objects detached (drawn by [`Intro::quads`]) and dark
    /// until they glide.
    pub fn overrides(&self) -> Overrides {
        let mut o = Overrides {
            hide: self.hidden.clone(),
            ..Default::default()
        };
        for (id, s) in &self.scatter {
            o.detach.insert(*id);
            if s.glide.is_none() {
                o.colour.insert(*id, DARK);
            }
        }
        o
    }

    /// Scattered objects (from the scene's detached sprites) and flowers,
    /// in screen pixels with their rotations.
    pub fn quads(
        &mut self,
        detached: &[(i32, baba_render::SceneSprite)],
        region_size: &dyn Fn(RegionId) -> (u32, u32),
        wobble: u32,
        tex: &Tex,
        pal: &dyn Fn((u8, u8)) -> [u8; 4],
        p: &Place,
    ) -> Vec<(Quad, f32)> {
        let mut out = Vec::new();
        for (id, sp) in detached {
            let Some(s) = self.scatter.get(id) else {
                continue;
            };
            let jitter = if s.shaking {
                (self.rng.range(-2, 2) as f32, self.rng.range(-2, 2) as f32)
            } else {
                (0.0, 0.0)
            };
            let (x, y, a) = s.pose(self.frame, jitter);
            let region = sp.frames[((wobble + sp.wobble as u32) % 3) as usize];
            let (w, h) = region_size(region);
            let (sx, sy) = p.at(x, y);
            let (w, h) = (w as f32 * p.k, h as f32 * p.k);
            out.push((
                Quad {
                    x: sx - w / 2.0,
                    y: sy - h / 2.0,
                    w,
                    h,
                    region,
                    color: sp.color,
                },
                a,
            ));
        }
        for f in &self.flowers {
            out.extend(f.quads(tex, pal, p));
        }
        out
    }
}
