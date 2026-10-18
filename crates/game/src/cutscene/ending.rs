//! The base world's endings (`progression.md` §10.1–§10.2, measured in
//! `presentation.md` §15.22–§15.24).
//!
//! * [`EndScene`] — END, in the level: the END object vanishes in smoke,
//!   the words tremble, a large flower buds at the END tile; at 220 every
//!   word turns into a floating blossom (the conversion poof) and the
//!   player walks around collecting them (each one sparkles with a flower
//!   note). When none are left the flower grows over the screen, closes,
//!   and the screen closes (the presenter submits `Proceed`).
//! * [`EndShow`] — what follows on the credits screen: the island with the
//!   ending flower and a flower for every completed area, small flowers
//!   spreading over it, THE END written on, then the logo credits (big
//!   tiles: BABA IS YOU, MADE BY ARVI TEIKARI, PORT BY MP2 GAMES, MADE WITH
//!   MMF2 CLICK TEAM with a walking Baba) before the scroll, whose
//!   decorations and the closing Baba it also draws.
//! * [`DoneScene`] — All Is Done, in the level: after 350 frames the view
//!   scrolls up to the island seen from afar, smoke rises, a dark blob
//!   swallows it, and the credits follow.

use std::collections::{BTreeMap, BTreeSet};

use baba_render::{Quad, RegionId};
use baba_sim::{CosmeticRng, Level};

use super::{Cue, Flower, FlowerKind, Place, Tex};
use crate::fx::transition::Iris;
use crate::fx::{CELL, LevelFx};
use crate::scene::Overrides;
use crate::ui::{Gfx, Ui};

// ------------------------------------------------------------------ END

/// A blossom (a word turned into a flower) drifting around its cell.
#[derive(Clone, Debug)]
struct Blossom {
    cell: (i32, i32),
    ph: (f32, f32),
}

impl Blossom {
    fn pos(&self, t: u32) -> (f32, f32) {
        let tt = t as f32;
        (
            (self.cell.0 as f32 + 0.5) * CELL + 4.0 * (tt / 37.0 + self.ph.0).sin(),
            (self.cell.1 as f32 + 0.5) * CELL + 5.0 * (tt / 29.0 + self.ph.1).sin(),
        )
    }
}

/// A player's unit during END: id, cell now, drawn offset (tween), start
/// cell.
type Walker = (i32, (i32, i32), (f32, f32), (i32, i32));

/// END in the level (phases 1 and 2).
pub struct EndScene {
    phase: u8,
    t: u32,
    frame: u32,
    end_id: Option<i32>,
    end_cell: (i32, i32),
    texts: Vec<(i32, (i32, i32))>,
    jitter: BTreeMap<i32, (f32, f32)>,
    converted: bool,
    blossoms: Vec<Blossom>,
    /// The player's units: id, cell now, drawn offset (tween), start cell.
    you: Vec<Walker>,
    blocked: BTreeSet<(i32, i32)>,
    size: (i32, i32),
    flower: Option<Flower>,
    rng: CosmeticRng,
    /// Set at frame 430 of phase 2: close the screen and go on.
    pub proceed: bool,
}

impl EndScene {
    /// On the turn END was reached.
    pub fn new(level: &Level, seed: u64) -> EndScene {
        let units = level.units();
        let you_ids = level.effect_sources("you").units;
        let you: Vec<_> = units
            .iter()
            .filter(|u| you_ids.contains(&u.id))
            .map(|u| (u.id, (u.x, u.y), (0.0, 0.0), (u.x, u.y)))
            .collect();
        let ends = level.effect_sources("end").units;
        let end = units
            .iter()
            .filter(|u| ends.contains(&u.id))
            .min_by_key(|u| {
                you.iter()
                    .map(|y| (y.1.0 - u.x).abs() + (y.1.1 - u.y).abs())
                    .min()
                    .unwrap_or(0)
            });
        let end_cell = end
            .map(|u| (u.x, u.y))
            .or_else(|| you.first().map(|y| y.1))
            .unwrap_or((level.width() / 2, level.height() / 2));
        let mut blocked = BTreeSet::new();
        for p in ["stop", "push", "pull"] {
            for id in level.effect_sources(p).units {
                if let Some(u) = units.iter().find(|u| u.id == id)
                    && !u.name.starts_with("text_")
                {
                    blocked.insert((u.x, u.y));
                }
            }
        }
        EndScene {
            phase: 1,
            t: 0,
            frame: 0,
            end_id: end.map(|u| u.id),
            end_cell,
            texts: units
                .iter()
                .filter(|u| u.name.starts_with("text_") && u.visible)
                .map(|u| (u.id, (u.x, u.y)))
                .collect(),
            jitter: BTreeMap::new(),
            converted: false,
            blossoms: Vec::new(),
            you,
            blocked,
            size: (level.width(), level.height()),
            flower: None,
            rng: CosmeticRng::new(seed),
            proceed: false,
        }
    }

    /// An arrow press: the player's units step (STOP, PUSH and PULL
    /// objects and the edge block; words no longer push).
    pub fn input(&mut self, dir: u8) -> Vec<Cue> {
        let (dx, dy) = match dir {
            0 => (1, 0),
            1 => (0, -1),
            2 => (-1, 0),
            3 => (0, 1),
            _ => return Vec::new(),
        };
        let mut moved = false;
        for (_, cell, tw, _) in &mut self.you {
            let n = (cell.0 + dx, cell.1 + dy);
            let inside = n.0 > 0 && n.1 > 0 && n.0 < self.size.0 - 1 && n.1 < self.size.1 - 1;
            if inside && !self.blocked.contains(&n) {
                *tw = (
                    tw.0 - dx as f32 * CELL * 0.75,
                    tw.1 - dy as f32 * CELL * 0.75,
                );
                *cell = n;
                moved = true;
            }
        }
        if moved {
            vec![("move".into(), 1.0)]
        } else {
            Vec::new()
        }
    }

    pub fn tick(&mut self, fx: &mut LevelFx) -> Vec<Cue> {
        let mut cues = Vec::new();
        self.frame += 1;
        self.t += 1;
        let t = self.t;
        for (_, _, tw, _) in &mut self.you {
            for v in [&mut tw.0, &mut tw.1] {
                *v *= 0.7;
                if v.abs() < 2.0 {
                    *v = 0.0;
                }
            }
        }
        if let Some(f) = &mut self.flower {
            f.tick();
        }
        let centre = (
            (self.end_cell.0 as f32 + 0.5) * CELL,
            (self.end_cell.1 as f32 + 0.5) * CELL,
        );
        match self.phase {
            1 => {
                if t == 2 {
                    fx.burst("smoke", self.end_cell, 20, (0, 3), true);
                }
                if t == 13 {
                    cues.push(("ending_reverse_baba_loop_start".into(), 1.0));
                }
                if t == 52 {
                    let seed = self.rng.range(0, 1 << 30) as u64;
                    self.flower = Some(Flower::new(
                        FlowerKind::You,
                        centre.0,
                        centre.1,
                        (4, 1),
                        "you",
                        seed,
                    ));
                }
                if !self.converted {
                    // The words tremble (a random walk, measured ±1 px a
                    // frame).
                    for (id, _) in &self.texts {
                        let j = self.jitter.entry(*id).or_insert((0.0, 0.0));
                        j.0 = (j.0 + self.rng.range(-1, 1) as f32).clamp(-6.0, 6.0);
                        j.1 = (j.1 + self.rng.range(-1, 1) as f32).clamp(-6.0, 6.0);
                    }
                }
                if t == 220 {
                    // Every word becomes a blossom (removed with the
                    // conversion poof).
                    self.converted = true;
                    for (id, cell) in &self.texts {
                        fx.poof(*id, *cell);
                        self.blossoms.push(Blossom {
                            cell: *cell,
                            ph: (
                                self.rng.range(0, 628) as f32 / 100.0,
                                self.rng.range(0, 628) as f32 / 100.0,
                            ),
                        });
                    }
                }
                if t > 220 {
                    let cells: Vec<(i32, i32)> = self.you.iter().map(|y| y.1).collect();
                    let before = self.blossoms.len();
                    let mut gone = Vec::new();
                    self.blossoms.retain(|b| {
                        let hit = cells.contains(&b.cell);
                        if hit {
                            gone.push(b.cell);
                        }
                        !hit
                    });
                    for c in gone {
                        fx.burst("bling", c, 10, (0, 3), true);
                    }
                    if self.blossoms.len() < before {
                        let n = self.rng.range(1, 7);
                        cues.push((format!("intro_flower_{n}"), 1.0));
                    }
                    if self.blossoms.is_empty() {
                        self.phase = 2;
                        self.t = 0;
                    }
                }
            }
            _ => {
                if t == 30
                    && let Some(f) = &mut self.flower
                {
                    f.open();
                    f.grow_to(4.2);
                }
                if t == 250 {
                    if let Some(f) = &mut self.flower {
                        f.close();
                        f.grow_to(1.0);
                    }
                    cues.push(("ending_reverse_baba_loop_end".into(), 1.0));
                }
                if t == 430 {
                    self.proceed = true;
                }
            }
        }
        cues
    }

    /// The END object hidden, words trembling (then hidden), the player's
    /// units at their cutscene cells.
    pub fn overrides(&self) -> Overrides {
        let mut o = Overrides::default();
        if let Some(id) = self.end_id {
            o.hide.insert(id);
        }
        for (id, _) in &self.texts {
            if self.converted {
                o.hide.insert(*id);
            } else if let Some(j) = self.jitter.get(id) {
                o.offset.insert(*id, *j);
            }
        }
        for (id, cell, tw, start) in &self.you {
            o.offset.insert(
                *id,
                (
                    (cell.0 - start.0) as f32 * CELL + tw.0.round(),
                    (cell.1 - start.1) as f32 * CELL + tw.1.round(),
                ),
            );
        }
        o
    }

    /// Blossoms and the flower.
    pub fn quads(
        &self,
        blossom: Option<[RegionId; 3]>,
        region_size: &dyn Fn(RegionId) -> (u32, u32),
        wobble: u32,
        tex: &Tex,
        pal: &dyn Fn((u8, u8)) -> [u8; 4],
        p: &Place,
    ) -> Vec<(Quad, f32)> {
        let mut out = Vec::new();
        if let Some(frames) = blossom {
            for (i, b) in self.blossoms.iter().enumerate() {
                let (x, y) = b.pos(self.frame);
                let r = frames[((wobble as usize) + i) % 3];
                let (w, h) = region_size(r);
                let (sx, sy) = p.at(x, y);
                let (w, h) = (w as f32 * p.k, h as f32 * p.k);
                out.push((
                    Quad {
                        x: sx - w / 2.0,
                        y: sy - h / 2.0,
                        w,
                        h,
                        region: r,
                        color: pal((0, 3)),
                    },
                    0.0,
                ));
            }
        }
        if let Some(f) = &self.flower {
            out.extend(f.quads(tex, pal, p));
        }
        out
    }
}

// --------------------------------------------------------- ALL IS DONE

/// All Is Done in the level.
pub struct DoneScene {
    phase: u8,
    t: u32,
    /// Top of the island picture in logical px (phase 2: jumps far below
    /// the screen, then rises; the level goes with it, out of view).
    pub room_y: f32,
    /// Radius of the dark blob over the island (logical px).
    pub blob: f32,
    /// Smoke puffs: position (logical px relative to the picture), age,
    /// dark.
    smoke: Vec<(f32, f32, u32, bool)>,
    rng: CosmeticRng,
    /// The credits follow.
    pub proceed: bool,
}

/// Room offset at the start of phase 2 and where it rises to (measured:
/// 3 % of the remaining way per frame toward −536 for 120 frames, then
/// −480).
const DONE_START: f32 = 1024.0;
const DONE_AIM: f32 = -536.0;
const DONE_END: f32 = -480.0;

impl DoneScene {
    pub fn new(seed: u64) -> DoneScene {
        DoneScene {
            phase: 1,
            t: 0,
            room_y: 0.0,
            blob: 0.0,
            smoke: Vec::new(),
            rng: CosmeticRng::new(seed),
            proceed: false,
        }
    }

    /// The level is out of view (phase 2 on).
    pub fn level_gone(&self) -> bool {
        self.phase >= 2
    }

    pub fn tick(&mut self) -> Vec<Cue> {
        let mut cues = Vec::new();
        self.t += 1;
        let t = self.t;
        for s in &mut self.smoke {
            s.1 -= 0.5;
            s.2 += 1;
        }
        self.smoke.retain(|s| s.2 < 80);
        match self.phase {
            1 => {
                if t == 350 {
                    self.phase = 2;
                    self.t = 0;
                    self.room_y = DONE_START;
                }
            }
            2 => {
                if t < 120 {
                    self.room_y += (DONE_AIM - self.room_y) * 0.03;
                } else if t == 120 {
                    self.room_y = DONE_END;
                }
                if t == 10 {
                    cues.push(("whoooooosh".into(), 1.0));
                }
                if t == 200 {
                    cues.push(("burn3_short".into(), 1.0));
                    for _ in 0..10 {
                        self.puff(false);
                    }
                }
                if t > 220 && t < 420 {
                    self.puff(true);
                }
                if t == 400 {
                    cues.push(("ending_rumble_quiet".into(), 1.0));
                }
                if t > 400 {
                    // Measured: the blob's edge moves out about 5 px a
                    // frame until the screen is dark.
                    self.blob = (t - 400) as f32 * 5.0;
                }
                if t == 1140 {
                    self.phase = 3;
                    self.t = 0;
                }
            }
            _ => {
                if t == 350 {
                    self.proceed = true;
                }
            }
        }
        cues
    }

    /// Smoke from the mountain top (cell 16, 23 of the picture's room).
    fn puff(&mut self, dark: bool) {
        let x = 7.0 + 16.5 * CELL + self.rng.range(-12, 12) as f32;
        let y = 23.5 * CELL + self.rng.range(-12, 12) as f32 - 480.0;
        self.smoke.push((x, y + 480.0, 0, dark));
    }

    /// Phase 2 on: the background, the island picture (`island`, three
    /// wobble frames), the smoke and the blob, over the whole screen.
    pub fn draw(&self, ui: &mut Ui, g: &mut Gfx, island: &[RegionId], tex: &Tex, wobble: u32) {
        if self.phase < 2 {
            return;
        }
        ui.fill(g, g.pal(0, 4));
        if !island.is_empty() {
            let r = island[(wobble as usize) % island.len()];
            let (iw, ih) = g.region_size(r);
            ui.sprite(r, 7.0 - 24.0, self.room_y, iw as f32, ih as f32, [255; 4]);
        }
        let k = ui.scale;
        for &(x, y, age, dark) in &self.smoke {
            let (sx, sy) = (ui.x(x), ui.y(y + self.room_y));
            let r = (5.0 + age as f32 * 0.12) * k;
            let mut c = if dark { g.pal(0, 4) } else { g.pal(0, 3) };
            c[3] = (255.0 * (1.0 - age as f32 / 80.0)) as u8;
            ui.quads.push(Quad {
                x: sx - r,
                y: sy - r,
                w: 2.0 * r,
                h: 2.0 * r,
                region: tex.disc,
                color: c,
            });
        }
        if self.blob > 0.0 {
            let (bx, by) = (
                ui.x(7.0 + 372.0 + 24.0),
                ui.y(62.0 + 24.0 + self.room_y + 480.0),
            );
            let r = self.blob * k;
            ui.quads.push(Quad {
                x: bx - r,
                y: by - r,
                w: 2.0 * r,
                h: 2.0 * r,
                region: tex.disc,
                color: g.pal(0, 4),
            });
        }
    }
}

// ----------------------------------------------------- the island + logos

/// A big credits tile (96 px images, three wobble frames).
#[derive(Clone, Debug)]
struct Tile {
    name: &'static str,
    /// Target and drawn centre (logical 854×480 px).
    at: (f32, f32),
    pos: (f32, f32),
    frames: [u32; 3],
    flip: bool,
    colour: (u8, u8),
    age: u32,
    leaving: Option<u32>,
}

/// Engine images of the credits tiles (`presentation.md` §15.23).
fn tile_frames(name: &str) -> ([u32; 3], (u8, u8)) {
    match name {
        "baba" => ([1610, 1667, 1668], (4, 1)),
        "is" => ([1611, 1653, 1669], (0, 3)),
        "you" => ([1632, 1670, 1671], (4, 1)),
        "made" => ([1644, 1691, 1692], (1, 3)),
        "by" => ([1636, 1673, 1675], (1, 3)),
        "arvi" => ([1635, 1678, 1679], (0, 3)),
        "teikari" => ([1677, 1680, 1682], (0, 3)),
        "port" => ([1638, 1681, 1683], (1, 3)),
        "mp2_logo" => ([1639, 1684, 1685], (4, 1)),
        "mp2" => ([1642, 1687, 1688], (0, 3)),
        "games" => ([1643, 1686, 1689], (0, 3)),
        "with" => ([1646, 1708, 1718], (1, 3)),
        "mmf2" => ([1645, 1690, 1739], (5, 3)),
        "click" => ([1648, 1740, 1753], (5, 3)),
        "team" => ([1649, 1737, 1757], (5, 3)),
        "keke" => ([1781, 1783, 1784], (2, 2)),
        "skull" => ([1782, 1785, 1786], (2, 1)),
        "flag" => ([1787, 1788, 1787], (2, 4)),
        "key" => ([1789, 1790, 1791], (2, 4)),
        "rocket" => ([1792, 1793, 1795], (1, 1)),
        "baba_front" => ([1661, 1662, 1663], (0, 3)),
        _ => ([1647, 1655, 1758], (0, 3)),
    }
}

/// Draws a credits tile by name centred at logical (`x`, `y`) (the scroll
/// decorations and the closing Baba).
pub fn draw_tile(ui: &mut Ui, g: &mut Gfx, name: &str, x: f32, y: f32, wobble: u32) {
    let (frames, c) = tile_frames(name);
    if let Some(a) = g.art.get(&mut g.atlas, frames[(wobble % 3) as usize]) {
        let col = g.pal(c.0 as u32, c.1 as u32);
        ui.sprite(
            a.region,
            x - a.hx as f32,
            y - a.hy as f32,
            a.w as f32,
            a.h as f32,
            col,
        );
    }
}

/// Frames of the walking Baba by the original's frame number.
fn baba_frames(frame: i32) -> ([u32; 3], bool) {
    match frame {
        16 => ([1641, 1768, 1769], false),
        18 => ([1661, 1662, 1663], false),
        17 | 20 | 23 => ([1761, 1762, 1763], true),
        19 | 22 => ([1764, 1765, 1766], false),
        _ => ([1647, 1655, 1758], false),
    }
}

/// The logo credits script (phase 4 frames, measured): `Load(name, x, y)`
/// in 96-px steps from the screen centre, `Remove(name)`, `Move(name, dx,
/// dy, frame)` (frame −1 keeps the image).
#[derive(Clone, Copy, Debug)]
enum Step {
    Load(&'static str, f32, f32),
    Remove(&'static str),
    Move(&'static str, f32, f32, i32),
    Grey(&'static str),
}

const SCRIPT: &[(u32, Step)] = &[
    (80, Step::Load("baba", -1.0, 0.0)),
    (120, Step::Load("is", 0.0, 0.0)),
    (150, Step::Load("you", 1.0, 0.0)),
    (190, Step::Load("baba_obj", -2.5, 2.0)),
    (310, Step::Remove("baba")),
    (330, Step::Remove("is")),
    (345, Step::Load("made", -1.5, 0.0)),
    (350, Step::Remove("you")),
    (375, Step::Load("by", -0.5, 0.0)),
    (426, Step::Load("arvi", 0.5, 0.0)),
    (465, Step::Load("teikari", 1.5, 0.0)),
    (490, Step::Move("baba_obj", 1.0, 0.0, 22)),
    (500, Step::Move("baba_obj", 1.0, 0.0, 19)),
    (510, Step::Move("baba_obj", 1.0, 0.0, 22)),
    (530, Step::Move("baba_obj", 0.0, -1.0, 16)),
    (540, Step::Move("baba_obj", 0.0, -1.0, 16)),
    (540, Step::Move("arvi", 0.0, -1.0, -1)),
    (540, Step::Grey("arvi")),
    (540, Step::Grey("teikari")),
    (605, Step::Remove("made")),
    (615, Step::Load("port", -2.0, 0.0)),
    (625, Step::Move("by", -0.5, 0.0, -1)),
    (625, Step::Move("baba_obj", -0.5, 0.0, 23)),
    (645, Step::Move("baba_obj", 0.0, -1.0, 16)),
    (645, Step::Remove("arvi")),
    (665, Step::Move("baba_obj", -1.0, 0.0, 23)),
    (665, Step::Remove("teikari")),
    (673, Step::Load("mp2_logo", 0.0, 0.0)),
    (695, Step::Load("mp2", 1.0, 0.0)),
    (725, Step::Load("games", 2.0, 0.0)),
    (816, Step::Remove("port")),
    (816, Step::Move("baba_obj", 0.0, 1.0, 18)),
    (816, Step::Move("by", 0.0, 1.0, -1)),
    (826, Step::Load("made", -1.0, -1.0)),
    (836, Step::Move("baba_obj", -1.0, 0.0, 23)),
    (840, Step::Remove("mp2_logo")),
    (840, Step::Load("with", 0.0, -1.0)),
    (846, Step::Move("baba_obj", -1.0, 0.0, 17)),
    (854, Step::Load("mmf2", 1.2, -1.0)),
    (854, Step::Remove("mp2")),
    (856, Step::Move("baba_obj", -1.0, 0.0, 20)),
    (866, Step::Move("baba_obj", -1.0, 0.0, 17)),
    (876, Step::Remove("games")),
    (876, Step::Move("baba_obj", -1.0, 0.0, 23)),
    (881, Step::Load("click", 0.2, 1.0)),
    (906, Step::Remove("baba_obj")),
    (926, Step::Load("team", 1.2, 1.0)),
    (1040, Step::Remove("made")),
    (1050, Step::Remove("with")),
    (1060, Step::Remove("mmf2")),
    (1070, Step::Remove("by")),
    (1080, Step::Remove("click")),
    (1090, Step::Remove("team")),
];

/// Decorations along the credits scroll (phase 5): name, x and y in 96-px
/// steps from the centre of the first screen.
pub const DECORATIONS: &[(&str, f32, f32)] = &[
    ("keke", -3.75, 12.4),
    ("skull", 3.5, 20.4),
    ("flag", -3.5, 34.5),
    ("key", 3.5, 39.0),
    ("rocket", -3.5, 55.1),
];

/// THE END written on (images in drawing order), then three wobble frames;
/// the dark backing growing behind it.
const THE_END: [u32; 15] = [
    1354, 1351, 1353, 1350, 1347, 1352, 1346, 1345, 1349, 1344, 1315, 1331, 1341, 1295, 1343,
];
const THE_END_WOBBLE: [u32; 3] = [1274, 1261, 1282];
const BACKING: [u32; 13] = [
    1612, 1615, 1617, 1619, 1618, 1620, 1621, 1622, 1623, 1624, 1625, 1626, 1627,
];

/// Area flowers on the island: level that completes the area, cell, colour
/// (`progression.md` §10.1).
const AREAS: [(&str, i32, i32, (u8, u8)); 10] = [
    ("232level", 10, 3, (1, 4)),
    ("179level", 11, 5, (2, 2)),
    ("180level", 13, 8, (4, 1)),
    ("206level", 18, 6, (2, 4)),
    ("87level", 20, 4, (3, 2)),
    ("169level", 21, 6, (5, 2)),
    ("207level", 18, 9, (5, 3)),
    ("177level", 16, 10, (1, 4)),
    ("16level", 21, 13, (2, 4)),
    ("182level", 24, 10, (0, 2)),
];

/// Colours of the spreading flowers (`presentation.md` §5.5).
const SPREAD: [(u8, u8); 6] = [(1, 4), (2, 4), (2, 3), (1, 3), (4, 2), (5, 3)];

/// Phases 3–6 of END on the credits screen.
pub struct EndShow {
    phase: u8,
    /// Frames of the current phase (negative: waiting).
    t: i32,
    flowers: Vec<Flower>,
    spread: Vec<(f32, f32)>,
    spread_shown: usize,
    tiles: Vec<Tile>,
    iris: Option<Iris>,
    rng: CosmeticRng,
    /// Images of the island (three wobble frames each: back, decor).
    pub island: Vec<RegionId>,
    pub decor: Vec<RegionId>,
    /// The logo credits are over: the scroll runs.
    pub scrolling: bool,
}

impl EndShow {
    /// `areas`: which of the ten areas are complete; `spread`: the points
    /// of `endingflowers.txt` (screen px of the 854×480 window).
    pub fn new(areas: &[bool], spread: Vec<(f32, f32)>, seed: u64) -> EndShow {
        let mut rng = CosmeticRng::new(seed);
        let mut flowers = vec![
            Flower::new(
                FlowerKind::Island,
                16.0 * CELL,
                14.0 * CELL,
                (4, 1),
                "endingflower",
                1,
            )
            .opened(),
        ];
        let (ox, oy) = ((854.0 - 33.0 * CELL) / 2.0, (480.0 - 18.0 * CELL) / 2.0);
        for (i, (_, x, y, c)) in AREAS.iter().enumerate() {
            if areas.get(i).copied().unwrap_or(false) {
                let seed = rng.range(0, 1 << 30) as u64;
                flowers.push(
                    Flower::new(
                        FlowerKind::Small,
                        ox + (*x as f32 + 0.5) * CELL,
                        oy + (*y as f32 + 0.5) * CELL,
                        *c,
                        "otherarea",
                        seed,
                    )
                    .opened(),
                );
            }
        }
        EndShow {
            phase: 3,
            t: -50,
            flowers,
            spread,
            spread_shown: 0,
            tiles: Vec::new(),
            iris: None,
            rng,
            island: Vec::new(),
            decor: Vec::new(),
            scrolling: false,
        }
    }

    /// The levels whose completed map makes an area flower appear.
    pub fn area_levels() -> [&'static str; 10] {
        AREAS.map(|a| a.0)
    }

    /// Reads `endingflowers.txt` (one `x,y` per line).
    pub fn spread_points(text: &str) -> Vec<(f32, f32)> {
        text.lines()
            .filter_map(|l| {
                let (a, b) = l.trim().split_once(',')?;
                Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
            })
            .collect()
    }

    /// One frame; `w`×`h` = the screen (for the transition). Sounds and
    /// `music:<name>` requests come back as cues.
    pub fn tick(&mut self, w: f32, h: f32) -> Vec<Cue> {
        let mut cues = Vec::new();
        self.t += 1;
        for f in &mut self.flowers {
            f.tick();
        }
        if let Some(i) = &mut self.iris {
            if i.tick() {
                // Covered: the logo credits start.
                self.phase = 4;
                self.t = -100;
                cues.push(("music:ending".into(), 1.0));
            }
            if i.covered() && self.phase == 4 {
                i.release();
            }
            if i.done() {
                self.iris = None;
            }
        }
        let t = self.t;
        match self.phase {
            3 => {
                if t == 30 {
                    cues.push(("dididi".into(), 1.0));
                }
                // Small flowers spread over the island from 250 on, one
                // every 4 frames, nearest first.
                if t >= 250 && (t - 250) % 4 == 0 && self.spread_shown < self.spread.len() {
                    let (x, y) = self.spread[self.spread_shown];
                    let c = SPREAD[self.rng.range(0, 5) as usize];
                    let seed = self.rng.range(0, 1 << 30) as u64;
                    let mut f = Flower::new(FlowerKind::Small, x, y, c, "spread", seed);
                    f.grow_to(0.5);
                    f.open();
                    self.flowers.push(f);
                    self.spread_shown += 1;
                }
                if t == 750 {
                    cues.push(("themetune".into(), 1.0));
                }
                if t == 1150 && self.iris.is_none() {
                    self.iris = Some(Iris::new(w, h, false, &mut self.rng));
                }
            }
            4 => {
                for tile in &mut self.tiles {
                    tile.age += 1;
                    if let Some(l) = &mut tile.leaving {
                        *l += 1;
                    }
                    tile.pos.0 += (tile.at.0 - tile.pos.0) * 0.16;
                    tile.pos.1 += (tile.at.1 - tile.pos.1) * 0.16;
                }
                self.tiles.retain(|t| t.leaving.is_none_or(|l| l < 15));
                if t >= 0 {
                    for (at, step) in SCRIPT {
                        if *at as i32 == t {
                            self.step(*step);
                        }
                    }
                }
                if t == 1100 {
                    self.phase = 5;
                    self.t = 0;
                    self.scrolling = true;
                }
            }
            _ => {}
        }
        cues
    }

    fn step(&mut self, s: Step) {
        let centre = |x: f32, y: f32| (427.0 + x * 96.0, 240.0 + y * 96.0);
        match s {
            Step::Load(name, x, y) => {
                let (frames, colour) = tile_frames(name);
                let at = centre(x, y);
                self.tiles.push(Tile {
                    name,
                    at,
                    pos: at,
                    frames,
                    flip: false,
                    colour,
                    age: 0,
                    leaving: None,
                });
            }
            Step::Remove(name) => {
                if let Some(t) = self
                    .tiles
                    .iter_mut()
                    .find(|t| t.name == name && t.leaving.is_none())
                {
                    t.leaving = Some(0);
                }
            }
            Step::Move(name, dx, dy, frame) => {
                if let Some(t) = self
                    .tiles
                    .iter_mut()
                    .find(|t| t.name == name && t.leaving.is_none())
                {
                    t.at = (t.at.0 + dx * 96.0, t.at.1 + dy * 96.0);
                    if frame >= 0 {
                        let (f, flip) = baba_frames(frame);
                        t.frames = f;
                        t.flip = flip;
                    }
                }
            }
            Step::Grey(name) => {
                for t in &mut self.tiles {
                    if t.name == name {
                        t.colour = (0, 1);
                    }
                }
            }
        }
    }

    /// Draws the island (phase 3) or the logo tiles (phase 4) under the
    /// flowers.
    pub fn draw_back(&self, ui: &mut Ui, g: &mut Gfx, wobble: u32) {
        match self.phase {
            3 => {
                ui.fill(g, g.pal(1, 0));
                let (ox, oy) = ((854.0 - 33.0 * CELL) / 2.0, (480.0 - 18.0 * CELL) / 2.0);
                for set in [&self.island, &self.decor] {
                    if set.is_empty() {
                        continue;
                    }
                    let r = set[(wobble as usize) % set.len()];
                    let (iw, ih) = g.region_size(r);
                    ui.sprite(r, ox, oy, iw as f32, ih as f32, [255; 4]);
                }
            }
            4 => {
                ui.fill(g, g.pal(0, 4));
                for t in &self.tiles {
                    let img = t.frames[(wobble % 3) as usize];
                    let Some(a) = g.art.get(&mut g.atlas, img) else {
                        continue;
                    };
                    // Pops in vertically (1 − 0.9·0.8ⁿ) and shrinks away
                    // (×0.8 a frame), measured.
                    let sy = match t.leaving {
                        Some(l) => 0.8f32.powi(l as i32 + 1),
                        None => 1.0 - 0.9 * 0.8f32.powi(t.age as i32),
                    };
                    let (w, h) = (a.w as f32, a.h as f32 * sy);
                    let (x, y) = (t.pos.0 - a.hx as f32, t.pos.1 - a.hy as f32 * sy);
                    let c = g.pal(t.colour.0 as u32, t.colour.1 as u32);
                    if t.flip {
                        ui.sprite(a.region, x + w, y, -w, h, c);
                    } else {
                        ui.sprite(a.region, x, y, w, h, c);
                    }
                }
            }
            _ => {}
        }
    }

    /// The island's flowers (screen pixels, rotations).
    pub fn flowers(&self, ui: &Ui, g: &Gfx, tex: &Tex) -> Vec<(Quad, f32)> {
        if self.phase != 3 {
            return Vec::new();
        }
        let p = Place {
            x0: ui.x(0.0),
            y0: ui.y(0.0),
            k: ui.scale,
        };
        let pal = |c: (u8, u8)| g.pal(c.0 as u32, c.1 as u32);
        self.flowers
            .iter()
            .flat_map(|f| f.quads(tex, &pal, &p))
            .collect()
    }

    /// THE END and the transition, over the flowers.
    pub fn draw_front(&self, ui: &mut Ui, g: &mut Gfx, wobble: u32) {
        if self.phase == 3 && self.t >= 700 {
            let k = ((self.t - 700) / 3) as usize;
            let img = if k < THE_END.len() {
                THE_END[k]
            } else {
                THE_END_WOBBLE[(wobble % 3) as usize]
            };
            if self.t >= 750 {
                let b = BACKING[(((self.t - 750) / 2) as usize).min(BACKING.len() - 1)];
                if let Some(a) = g.art.get(&mut g.atlas, b) {
                    ui.sprite(
                        a.region,
                        427.0 - a.hx as f32,
                        240.0 + 8.0 - 12.0 - a.hy as f32,
                        a.w as f32,
                        a.h as f32,
                        [255; 4],
                    );
                }
            }
            if let Some(a) = g.art.get(&mut g.atlas, img) {
                for (dy, c) in [(4.0, g.pal(1, 2)), (0.0, g.pal(0, 3))] {
                    ui.sprite(
                        a.region,
                        427.0 - a.hx as f32,
                        240.0 + dy - 12.0 - a.hy as f32,
                        a.w as f32,
                        a.h as f32,
                        c,
                    );
                }
            }
        }
        if let Some(i) = &self.iris {
            crate::overlays::iris(ui, g, i, true);
        }
    }
}
