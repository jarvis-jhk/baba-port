//! Per-view presentation effects of a running level
//! (`docs/spec/presentation.md` §2.4, §4.6, §5.6–§5.8, §6.4, §7, §9):
//! movement tweens, the FLOAT bob, colour blinking, the background colour
//! fade, screen shake, particle bursts from the sim's events, continuous
//! property particles and the move puffs. One [`LevelFx`] belongs to one
//! level view, so splitscreen views animate independently. Everything is
//! driven by 60 Hz ticks and the cosmetic generator; nothing feeds back
//! into the simulation.
//!
//! Coordinates are level pixels at the 24 px source cell size; the view
//! scales them with the level.

pub mod background;
pub mod kinds;
pub mod transition;
pub mod vision;

use std::collections::BTreeMap;

use baba_render::{Atlas, SceneSprite};
use baba_sim::{CosmeticRng, Event, Level, MoveKind, TurnKind, TurnOutcome};

use crate::art::Art;
use kinds::{Anim, CONTINUOUS, FADE_FRAMES, Kind, STEP, kind};

/// Source cell size in pixels.
pub const CELL: f32 = 24.0;
/// Frames per wobble frame (measured; all units switch together).
pub const WOBBLE_TICKS: u64 = 10;
/// Frames per blink colour step (measured).
pub const BLINK_TICKS: u64 = 10;
/// FLOAT bob period in frames and amplitude in pixels (measured).
pub const FLOAT_PERIOD: f32 = 220.0;
pub const FLOAT_AMPLITUDE: f32 = 3.0;
/// Draw layer of particles (above every unit layer).
pub const PARTICLE_LAYER: i16 = 1000;

/// Options from the settings and the level's flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FxOptions {
    pub particles: bool,
    pub shake: bool,
    pub rule_effect: bool,
}

impl Default for FxOptions {
    fn default() -> Self {
        Self {
            particles: true,
            shake: true,
            rule_effect: true,
        }
    }
}

/// First origin, last target and kind of a unit's moves in one turn.
type NetMove = ((i32, i32), (i32, i32), MoveKind);

/// One live particle.
#[derive(Clone, Debug)]
struct Particle {
    kind: &'static Kind,
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    age: u16,
    life: u16,
    colour: (u8, u8),
    /// Follows this unit (offset kept relative to its drawn centre).
    follow: Option<i32>,
    /// Wobble phase.
    phase: u8,
}

/// Effects state of one level view.
pub struct LevelFx {
    rng: CosmeticRng,
    /// Frames since the level started (drives the effect timers).
    pub frame: u64,
    /// Drawn offset of moving units from their cells, in pixels.
    tweens: BTreeMap<i32, (f32, f32)>,
    particles: Vec<Particle>,
    shake: i32,
    shake_offset: (i32, i32),
    /// Cell centres of units as last drawn (for following particles).
    centres: BTreeMap<i32, (f32, f32)>,
    /// Background colour shown (fading toward the target), RGB.
    bg: Option<[f32; 3]>,
    /// Animation counters per unit: (walk steps, turns) (§4.5).
    anim: BTreeMap<i32, (u8, u8)>,
    /// Units that are SLEEP.
    sleeping: std::collections::BTreeSet<i32>,
    /// Background particle style of the level (`particles`, §10).
    pub style: String,
    back: Vec<background::BgParticle>,
    /// Level size in pixels.
    area: (f32, f32),
    /// Shown level rotation (degrees) and scroll (cells), easing toward
    /// the level's (measured: half the remaining difference per frame).
    rot: f32,
    scroll: (f32, f32),
    /// Rotating rings of counter symbols at opening gates (§15.26).
    rings: Vec<Ring>,
    /// Units removed by conversion showing the "poof" (id, cell, age;
    /// `presentation.md` §15.24).
    poofs: Vec<(i32, (i32, i32), u8)>,
    /// Level flags.
    pub options: FxOptions,
    /// Player settings.
    pub user: FxOptions,
}

/// Frames a gate ring shows (it ends with the gate's burst).
const RING_FRAMES: u16 = 120;

/// A ring of counter symbols around an opening gate.
#[derive(Clone, Debug)]
struct Ring {
    cx: f32,
    cy: f32,
    n: u32,
    images: [u32; 3],
    age: u16,
}

fn cell_centre(x: i32, y: i32) -> (f32, f32) {
    ((x as f32 + 0.5) * CELL, (y as f32 + 0.5) * CELL)
}

impl LevelFx {
    pub fn new(seed: u64) -> LevelFx {
        LevelFx {
            rng: CosmeticRng::new(seed),
            frame: 0,
            tweens: BTreeMap::new(),
            particles: Vec::new(),
            shake: 0,
            shake_offset: (0, 0),
            centres: BTreeMap::new(),
            bg: None,
            anim: BTreeMap::new(),
            sleeping: Default::default(),
            style: String::new(),
            back: Vec::new(),
            area: (0.0, 0.0),
            rot: 0.0,
            scroll: (0.0, 0.0),
            poofs: Vec::new(),
            rings: Vec::new(),
            options: FxOptions::default(),
            user: FxOptions::default(),
        }
    }

    /// State at level start (sleeping units show their sleep frame).
    pub fn start(&mut self, level: &Level) {
        self.reset();
        self.sleeping = level.effect_sources("sleep").units.into_iter().collect();
        self.area = (level.width() as f32 * CELL, level.height() as f32 * CELL);
        self.rot = level.rotation() as f32;
        let (ox, oy) = level.offset();
        self.scroll = (ox as f32, oy as f32);
        self.back = if self.options.particles && self.user.particles {
            background::create(&self.style, self.area.0, self.area.1, &mut self.rng)
        } else {
            Vec::new()
        };
    }

    /// Drops everything in flight (restart, level change).
    pub fn reset(&mut self) {
        self.tweens.clear();
        self.particles.clear();
        self.shake = 0;
        self.shake_offset = (0, 0);
        self.frame = 0;
        self.anim.clear();
        self.sleeping.clear();
        self.poofs.clear();
    }

    fn unit(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.rng.range(0, 10_000) as f32 / 10_000.0
    }

    /// Spawns one particle of `k` for the cell centred at (`cx`, `cy`).
    fn spawn(
        &mut self,
        k: &'static Kind,
        cx: f32,
        cy: f32,
        colour: (u8, u8),
        jitter: bool,
        follow: Option<i32>,
    ) {
        if !(self.options.particles && self.user.particles) || self.particles.len() > 4000 {
            return;
        }
        let half = (CELL / 2.0) as i32;
        let (ox, oy) = if jitter {
            (
                self.rng.range(-half, half) as f32,
                self.rng.range(-half, half) as f32,
            )
        } else {
            (0.0, 0.0)
        };
        let base = if (ox, oy) == (0.0, 0.0) || k.jitter >= 180.0 {
            self.unit(0.0, 360.0)
        } else {
            (-oy).atan2(ox).to_degrees()
        };
        let dir = if k.jitter >= 180.0 {
            base
        } else {
            base + self.unit(-k.jitter, k.jitter)
        }
        .to_radians();
        let speed = self.unit(k.speed.0, k.speed.1);
        let life = self.rng.range(k.life.0 as i32, k.life.1 as i32) as u16;
        let phase = self.rng.range(0, 2) as u8;
        self.particles.push(Particle {
            kind: k,
            // Following particles keep their position relative to the unit.
            x: if follow.is_some() { ox } else { cx + ox },
            y: if follow.is_some() { oy } else { cy + oy },
            vx: dir.cos() * speed,
            vy: -dir.sin() * speed,
            age: 0,
            life,
            colour,
            follow,
            phase,
        });
    }

    /// Bursts, tweens, puffs and shake of one turn.
    pub fn on_turn(&mut self, out: &TurnOutcome, level: &Level) {
        if out.kind == TurnKind::Stored {
            return;
        }
        if out.kind == TurnKind::Restart {
            self.reset();
            return;
        }
        let units: BTreeMap<i32, baba_sim::UnitView> =
            level.units().into_iter().map(|u| (u.id, u)).collect();
        let vision = level.vision_on();
        // Net movement per unit this turn (first origin, last target).
        let mut moves: BTreeMap<i32, NetMove> = BTreeMap::new();
        let mut puffs = 0;
        for e in &out.events {
            match e {
                Event::Moved {
                    id, from, to, kind, ..
                } => {
                    let entry = moves.entry(*id).or_insert((*from, *to, *kind));
                    entry.1 = *to;
                    if matches!(kind, MoveKind::Teleport | MoveKind::Undo) {
                        entry.2 = *kind;
                    }
                    if from != to
                        && matches!(kind, MoveKind::Step | MoveKind::Fall)
                        && !vision
                        && puffs < 700
                        && units.get(id).is_some_and(|u| u.visible)
                    {
                        puffs += 1;
                        let colour = units[id].colour;
                        self.puff(*from, *to, colour);
                    }
                }
                Event::Particles(b) => {
                    let n = b.count as u32 * b.scale as u32 / 100;
                    let (cx, cy) = cell_centre(b.at.0, b.at.1);
                    let k = kind(b.kind.name());
                    for _ in 0..n {
                        if !self.rng.one_in(b.chance) {
                            continue;
                        }
                        let colour = if b.kind == baba_sim::Particle::Confetti {
                            baba_sim::events::CONFETTI[self.rng.range(0, 4) as usize]
                        } else {
                            b.colour
                        };
                        self.spawn(k, cx, cy, colour, true, b.attach);
                        if b.kind == baba_sim::Particle::Confetti
                            && let Some(p) = self.particles.last_mut()
                        {
                            // Lua-set speeds (§9.6), engine units per frame.
                            p.vx = self.rng.range(-20, 20) as f32;
                            p.vy = self.rng.range(-25, -10) as f32;
                        }
                    }
                    if b.shake > 0 && (self.options.shake && self.user.shake) {
                        self.shake = self.shake.max(b.shake as i32);
                    }
                }
                Event::RuleFormed { texts, .. }
                    if self.options.rule_effect && self.user.rule_effect =>
                {
                    for id in texts {
                        if let Some(u) = units.get(id) {
                            let (cx, cy) = cell_centre(u.x, u.y);
                            for _ in 0..5 {
                                self.spawn(kind("bling"), cx, cy, u.colour, true, Some(*id));
                            }
                        }
                    }
                }
                Event::Removed {
                    id,
                    at,
                    cause: baba_sim::events::Cause::Converted,
                    ..
                } if !vision => self.poof(*id, *at),
                Event::Play { units: ids, .. } => {
                    for id in ids {
                        if let Some(u) = units.get(id) {
                            let (cx, cy) = cell_centre(u.x, u.y);
                            self.spawn(kind("music"), cx, cy, (0, 3), true, None);
                        }
                    }
                }
                _ => {}
            }
        }
        for (id, (from, to, k)) in &moves {
            if from != to && matches!(k, MoveKind::Step | MoveKind::Fall | MoveKind::Back) {
                let a = self.anim.entry(*id).or_default();
                a.0 = a.0.wrapping_add(1);
            }
        }
        let sleeping: std::collections::BTreeSet<i32> =
            level.effect_sources("sleep").units.into_iter().collect();
        if out.undo_step {
            for u in units.values() {
                if !sleeping.contains(&u.id) {
                    let a = self.anim.entry(u.id).or_default();
                    a.1 = a.1.wrapping_add(1);
                }
            }
        }
        // Waking up: back to step 0 with a small puff (§4.5).
        let woken: Vec<i32> = self.sleeping.difference(&sleeping).copied().collect();
        for id in woken {
            self.anim.insert(id, (0, 0));
            if let Some(u) = units.get(&id) {
                let (cx, cy) = cell_centre(u.x, u.y);
                self.spawn(kind("destroy"), cx, cy, (3, 1), true, None);
            }
        }
        self.sleeping = sleeping;
        for (id, (from, to, k)) in moves {
            let tween = matches!(k, MoveKind::Step | MoveKind::Fall | MoveKind::Back);
            if !tween || from == to {
                if matches!(k, MoveKind::Teleport | MoveKind::Undo) {
                    self.tweens.remove(&id);
                }
                continue;
            }
            let prev = self.tweens.get(&id).copied().unwrap_or((0.0, 0.0));
            let dx = prev.0 + (from.0 - to.0) as f32 * CELL;
            let dy = prev.1 + (from.1 - to.1) as f32 * CELL;
            // The first quarter of the way is jumped at once (§4.6).
            self.tweens.insert(id, (dx * 0.75, dy * 0.75));
        }
    }

    /// A burst of `n` particles of `kind` at a cell (scripted sequences).
    pub fn burst(
        &mut self,
        kind_name: &str,
        cell: (i32, i32),
        n: u32,
        colour: (u8, u8),
        jitter: bool,
    ) {
        let (cx, cy) = cell_centre(cell.0, cell.1);
        let k = kind(kind_name);
        for _ in 0..n {
            self.spawn(k, cx, cy, colour, jitter, None);
        }
    }

    /// A burst of `n` particles of `kind` centred at level pixel `at`
    /// (scripted sequences with objects off the grid).
    pub fn burst_at(&mut self, kind_name: &str, at: (f32, f32), n: u32, colour: (u8, u8)) {
        let k = kind(kind_name);
        for _ in 0..n {
            self.spawn(k, at.0, at.1, colour, true, None);
        }
    }

    /// The conversion "poof" of unit `id` at `cell`: for two frames its
    /// last sprite is drawn cut by white dashed lines (measured, §15.24).
    pub fn poof(&mut self, id: i32, cell: (i32, i32)) {
        self.poofs.push((id, cell, 0));
    }

    /// The ring of counter symbols around a gate that opens (`gate` = the
    /// gate type 1–4, `count` = its requirement; measured on captures:
    /// the symbols fly in from far away, spiral to a 24 px ring and turn
    /// until the gate opens 120 frames later).
    pub fn gate_ring(&mut self, cell: (i32, i32), gate: i32, count: i32) {
        let images = match gate {
            2 => [1161, 1162, 1167],
            3 => [2722, 2728, 2729],
            _ => [2719, 2721, 2723],
        };
        let (cx, cy) = cell_centre(cell.0, cell.1);
        self.rings.push(Ring {
            cx,
            cy,
            n: count.clamp(1, 60) as u32,
            images,
            age: 0,
        });
    }

    /// Poofs showing now: unit id, cell, frame (0 or 1).
    pub fn poofs(&self) -> impl Iterator<Item = (i32, (i32, i32), u8)> + '_ {
        self.poofs.iter().copied()
    }

    /// Sets the screen-shake strength (if stronger than the current one).
    pub fn shake(&mut self, s: u8) {
        if self.options.shake && self.user.shake {
            self.shake = self.shake.max(s as i32);
        }
    }

    /// The move puff: one bling particle three quarters down the old
    /// cell, drifting against the move (§9.3).
    fn puff(&mut self, from: (i32, i32), to: (i32, i32), colour: (u8, u8)) {
        if !(self.options.particles && self.user.particles) {
            return;
        }
        let sx = (to.0 - from.0).signum() as f32;
        let sy = (to.1 - from.1).signum() as f32;
        let cx = (from.0 as f32 + 0.5) * CELL;
        let cy = (from.1 as f32 + 0.75) * CELL;
        let k = kind("bling");
        let a = self.rng.range(10, 30) as f32 * 0.1;
        let b = self.rng.range(10, 30) as f32 * 0.1;
        self.particles.push(Particle {
            kind: k,
            x: cx,
            y: cy,
            vx: -sx * a,
            vy: -sy * b,
            age: 0,
            life: k.life.1,
            colour,
            follow: None,
            phase: 0,
        });
    }

    /// One 60 Hz frame.
    pub fn tick(&mut self, level: &Level) {
        self.frame += 1;
        for p in &mut self.poofs {
            p.2 += 1;
        }
        self.poofs.retain(|p| p.2 < 2);
        for r in &mut self.rings {
            r.age += 1;
        }
        self.rings.retain(|r| r.age < RING_FRAMES);
        for t in self.tweens.values_mut() {
            t.0 *= 0.7;
            t.1 *= 0.7;
            if t.0.abs() < 2.0 {
                t.0 = 0.0;
            }
            if t.1.abs() < 2.0 {
                t.1 = 0.0;
            }
        }
        self.tweens.retain(|_, t| *t != (0.0, 0.0));
        if self.shake > 0 {
            self.shake -= 1;
            let s = self.shake;
            self.shake_offset = if s >= 1 {
                (self.rng.range(-s, s), self.rng.range(-s, s))
            } else {
                (0, 0)
            };
        } else {
            self.shake_offset = (0, 0);
        }
        let units = level.units();
        self.centres.clear();
        for u in &units {
            let (cx, cy) = cell_centre(u.x, u.y);
            let off = self.tweens.get(&u.id).copied().unwrap_or((0.0, 0.0));
            self.centres.insert(u.id, (cx + off.0, cy + off.1));
        }
        for p in &mut self.particles {
            p.age += 1;
            p.vx *= p.kind.decay;
            p.vy = p.vy * p.kind.decay + p.kind.gravity;
            p.x += p.vx * STEP;
            p.y += p.vy * STEP;
        }
        self.particles.retain(|p| p.age < p.life);
        background::step(&mut self.back, self.area.0, self.area.1);
        // Rotation takes the short way round.
        let target = level.rotation() as f32;
        let mut d = (target - self.rot) % 360.0;
        if d > 180.0 {
            d -= 360.0;
        } else if d < -180.0 {
            d += 360.0;
        }
        self.rot = if d.abs() < 0.01 {
            target
        } else {
            self.rot + d * 0.5
        };
        let (ox, oy) = level.offset();
        for (cur, t) in [
            (&mut self.scroll.0, ox as f32),
            (&mut self.scroll.1, oy as f32),
        ] {
            let d = t - *cur;
            *cur = if d.abs() < 0.01 { t } else { *cur + d * 0.5 };
        }
        if self.options.particles && self.user.particles {
            self.continuous(level, &units);
        }
    }

    /// Continuous property particles (§9.4) on this frame.
    fn continuous(&mut self, level: &Level, units: &[baba_sim::UnitView]) {
        for c in CONTINUOUS {
            if !self.frame.is_multiple_of(c.period as u64) {
                continue;
            }
            let src = level.effect_sources(c.prop);
            if src.units.is_empty() && src.cells.is_empty() {
                continue;
            }
            let k = kind(c.kind);
            let colour_end = (1u8, self.rng.range(2, 4) as u8);
            for id in &src.units {
                let Some(u) = units.iter().find(|u| u.id == *id) else {
                    continue;
                };
                if !u.visible || u.name == "empty" {
                    continue;
                }
                let colour = if c.inwards {
                    colour_end
                } else {
                    c.colour.unwrap_or(u.colour)
                };
                let (cx, cy) = cell_centre(u.x, u.y);
                for _ in 0..c.count {
                    if !self.rng.one_in(c.chance) {
                        continue;
                    }
                    if c.inwards {
                        self.inwards(k, cx, cy, colour);
                    } else {
                        self.spawn(k, cx, cy, colour, !c.fixed, c.fixed.then_some(*id));
                    }
                }
            }
            for &(x, y) in &src.cells {
                let (cx, cy) = cell_centre(x, y);
                let colour = c.colour.unwrap_or(colour_end);
                for _ in 0..c.count {
                    if self.rng.one_in(c.chance) {
                        self.spawn(k, cx, cy, colour, !c.fixed, None);
                    }
                }
            }
        }
    }

    /// An END particle: placed within 1.5 cells and converging (§9.5).
    fn inwards(&mut self, k: &'static Kind, cx: f32, cy: f32, colour: (u8, u8)) {
        let r = (CELL * 1.5) as i32;
        let ox = self.rng.range(-r, r) as f32;
        let oy = self.rng.range(-r, r) as f32;
        let life = self.rng.range(k.life.0 as i32, k.life.1 as i32) as u16;
        self.particles.push(Particle {
            kind: k,
            x: cx + ox,
            y: cy + oy,
            vx: -ox * 0.2,
            vy: -oy * 0.2,
            age: 0,
            life,
            colour,
            follow: None,
            phase: 0,
        });
    }

    /// Drawn offset of a unit from its cell: movement tween and FLOAT bob
    /// (`float` = the unit's float layer), in pixels.
    pub fn unit_offset(&self, id: i32, float: u8) -> (f32, f32) {
        let (mut dx, mut dy) = self.tweens.get(&id).copied().unwrap_or((0.0, 0.0));
        dx = dx.round();
        dy = dy.round();
        if float == 1 {
            let t = self.frame as f32 / FLOAT_PERIOD * std::f32::consts::TAU;
            dy += (FLOAT_AMPLITUDE * t.sin() - 0.5).round();
        }
        (dx, dy)
    }

    /// Animation state of a unit: walk steps, turns, asleep.
    pub fn anim(&self, id: i32) -> (u8, u8, bool) {
        let (a, b) = self.anim.get(&id).copied().unwrap_or((0, 0));
        (a, b, self.sleeping.contains(&id))
    }

    /// Level rotation to show, degrees (counter-clockwise on screen).
    pub fn view_rotation(&self) -> f32 {
        self.rot
    }

    /// Level scroll to show, in cells.
    pub fn view_scroll(&self) -> (f32, f32) {
        self.scroll
    }

    /// Current screen-shake offset in level pixels.
    pub fn shake_offset(&self) -> (i32, i32) {
        self.shake_offset
    }

    /// Index into a blinking colour list (§5.8).
    pub fn blink(&self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            ((self.frame / BLINK_TICKS) as usize) % n
        }
    }

    /// The background colour shown: fades toward `target` (measured: half
    /// the difference per frame).
    pub fn background(&mut self, target: [u8; 4]) -> [u8; 4] {
        let t = [target[0] as f32, target[1] as f32, target[2] as f32];
        let cur = self.bg.get_or_insert(t);
        for i in 0..3 {
            cur[i] += (t[i] - cur[i]) * 0.5;
            if (t[i] - cur[i]).abs() < 1.0 {
                cur[i] = t[i];
            }
        }
        [cur[0] as u8, cur[1] as u8, cur[2] as u8, 255]
    }

    /// Particle sprites for the scene (`colour` resolves palette
    /// coordinates; `wobble` is the global wobble counter).
    pub fn sprites(
        &self,
        atlas: &mut Atlas,
        art: &mut Art,
        wobble: u32,
        colour: &dyn Fn((u8, u8)) -> [u8; 4],
        out: &mut Vec<SceneSprite>,
        scaled: &mut Vec<(SceneSprite, f32)>,
    ) {
        for b in &self.back {
            let Some(a) = art.get(atlas, b.images[((wobble + b.phase as u32) % 3) as usize]) else {
                continue;
            };
            let dx = (CELL as i32 - a.w as i32).div_euclid(2) as f32;
            let dy = (CELL as i32 - a.h as i32).div_euclid(2) as f32;
            scaled.push((
                SceneSprite {
                    x: (b.x.round() - a.hx as f32 - dx) / CELL,
                    y: (b.y.round() - a.hy as f32 - dy) / CELL,
                    frames: [a.region; 3],
                    color: colour(b.colour),
                    wobble: 0,
                    layer: if b.front { PARTICLE_LAYER - 1 } else { -50 },
                },
                b.size,
            ));
        }
        for r in &self.rings {
            let rad = 24.0 + 200.0 * 0.95f32.powi(r.age as i32);
            for i in 0..r.n {
                let a = (r.age as f32 * 3.0 + i as f32 * 360.0 / r.n as f32).to_radians();
                let img = r.images[((wobble + i) % 3) as usize];
                let Some(art_img) = art.get(atlas, img) else {
                    continue;
                };
                let (px, py) = (r.cx + a.cos() * rad, r.cy - a.sin() * rad);
                let dx = (CELL as i32 - art_img.w as i32).div_euclid(2) as f32;
                let dy = (CELL as i32 - art_img.h as i32).div_euclid(2) as f32;
                out.push(SceneSprite {
                    x: (px.round() - art_img.hx as f32 - dx) / CELL,
                    y: (py.round() - art_img.hy as f32 - dy) / CELL,
                    frames: [art_img.region; 3],
                    color: colour((0, 3)),
                    wobble: 0,
                    layer: PARTICLE_LAYER,
                });
            }
        }
        for p in &self.particles {
            if p.age < p.kind.hidden {
                continue;
            }
            let k = p.kind;
            let n = k.images.len() as u16;
            let img = match k.anim {
                Anim::Seq => {
                    k.images[((p.age as u32 * n as u32) / p.life.max(1) as u32).min(n as u32 - 1)
                        as usize]
                }
                Anim::Wobble => {
                    let fade = n.saturating_sub(3);
                    let left = p.life.saturating_sub(p.age);
                    if fade > 0 && left <= fade * FADE_FRAMES {
                        let i = fade - 1 - (left.saturating_sub(1) / FADE_FRAMES).min(fade - 1);
                        k.images[(3 + i) as usize]
                    } else {
                        k.images[((wobble + p.phase as u32) % 3) as usize]
                    }
                }
            };
            let Some(a) = art.get(atlas, img) else {
                continue;
            };
            let (px, py) = match p.follow {
                Some(id) => match self.centres.get(&id) {
                    Some(&(cx, cy)) => (cx + p.x, cy + p.y),
                    None => continue,
                },
                None => (p.x, p.y),
            };
            let dx = (CELL as i32 - a.w as i32).div_euclid(2) as f32;
            let dy = (CELL as i32 - a.h as i32).div_euclid(2) as f32;
            out.push(SceneSprite {
                x: (px.round() - a.hx as f32 - dx) / CELL,
                y: (py.round() - a.hy as f32 - dy) / CELL,
                frames: [a.region; 3],
                color: colour(p.colour),
                wobble: 0,
                layer: PARTICLE_LAYER,
            });
        }
    }

    /// Number of live particles (tests, logs).
    pub fn particle_count(&self) -> usize {
        self.particles.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tween_matches_the_measured_curve() {
        // One cell to the right: 18, 13, 9, 6, 4, 3, 2, then 0 px left.
        let mut fx = LevelFx::new(1);
        fx.tweens.insert(1, (-24.0 * 0.75, 0.0));
        let mut seen = Vec::new();
        for _ in 0..9 {
            seen.push(-fx.unit_offset(1, 0).0 as i32);
            for t in fx.tweens.values_mut() {
                t.0 *= 0.7;
                if t.0.abs() < 2.0 {
                    t.0 = 0.0;
                }
            }
        }
        assert_eq!(seen, vec![18, 13, 9, 6, 4, 3, 2, 0, 0]);
    }

    #[test]
    fn every_kind_has_images_and_sane_ranges() {
        for k in kinds::KINDS {
            assert!(k.images.len() >= 3, "{}", k.name);
            assert!(k.life.0 <= k.life.1 && k.life.0 > 0, "{}", k.name);
            assert!(k.speed.0 <= k.speed.1, "{}", k.name);
        }
    }
}
