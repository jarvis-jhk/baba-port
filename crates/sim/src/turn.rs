//! The turn pipeline (`docs/spec/turn.md` §4), the pending-action queue
//! (§5), undo turns (`undo.md` §4–§5) and the level-start passes.

use std::sync::Arc;

use crate::effects::{self, status};
use crate::events::{Cause, DestroyStyle, TurnKind, TurnOutcome};
use crate::level::Level;
use crate::movement::{self, Keys, extra};
use crate::rules;
use crate::undo::Line;
use crate::unit::{Handle, Unit};

/// A queued action (`turn.md` §5).
#[derive(Clone, Debug)]
pub(crate) enum Pending {
    /// Move to (x, y), optionally turning.
    Move {
        h: Handle,
        x: i32,
        y: i32,
        dir: Option<u8>,
    },
    /// Turn.
    Dir { h: Handle, dir: u8 },
    /// A unit conversion.
    Convert {
        h: Handle,
        outs: Vec<crate::convert::Output>,
        snap: Box<Unit>,
    },
    /// An empty-tile conversion.
    EmptyConvert {
        x: i32,
        y: i32,
        outs: Vec<crate::convert::Output>,
    },
    /// A float change (already applied): record the line, drop HOLD.
    Float { h: Handle, old: u8 },
}

/// One input to the level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Input {
    /// A direction (0 right, 1 up, 2 left, 3 down) or 4 = wait, for player
    /// 1 (drives YOU) or 2 (drives YOU2, falling back to YOU).
    Key {
        /// Direction 0..=4.
        dir: u8,
        /// Player 1 or 2.
        player: u8,
    },
    /// The AUTO timer fired (LEVEL IS AUTO, `turn.md` §3): one auto turn
    /// with the directions the players stored since the last one (keys
    /// only store while AUTO holds). Nothing happens while AUTO does not
    /// hold. The game layer sends it periodically while
    /// [`Level::auto_active`].
    Tick,
    /// Undo.
    Undo,
    /// Restart (reload the level).
    Restart,
}

impl Input {
    /// Player-1 direction key.
    pub fn dir(d: u8) -> Input {
        Input::Key { dir: d, player: 1 }
    }
    /// Player-1 wait.
    pub fn wait() -> Input {
        Input::Key { dir: 4, player: 1 }
    }
    /// Parses a corpus character: `R U L D W` (also `S`/`.`/` ` for wait),
    /// `Z` undo, `r` restart, `A` AUTO timer tick.
    pub fn from_char(c: char) -> Option<Input> {
        Some(match c {
            'R' => Input::dir(0),
            'U' => Input::dir(1),
            'L' => Input::dir(2),
            'D' => Input::dir(3),
            'W' | 'S' | '.' | ' ' => Input::wait(),
            'Z' => Input::Undo,
            'r' => Input::Restart,
            'A' => Input::Tick,
            _ => return None,
        })
    }
}

/// Flushes the pending queue (`turn.md` §5).
pub(crate) fn flush(lvl: &mut Level) {
    while !lvl.scratch.pending.is_empty() {
        let q = std::mem::take(&mut lvl.scratch.pending);
        for p in q {
            match p {
                Pending::Move { h, x, y, dir } => {
                    if lvl.alive(h) {
                        let d = dir.unwrap_or(lvl.unit(h).dir);
                        lvl.update_unit(h, x, y, d);
                    }
                }
                Pending::Dir { h, dir } => {
                    if lvl.alive(h) {
                        lvl.set_dir(h, dir);
                    }
                }
                Pending::Convert { h, outs, snap } => crate::convert::execute(lvl, h, &outs, &snap),
                Pending::EmptyConvert { x, y, outs } => {
                    crate::convert::execute_empty(lvl, x, y, &outs)
                }
                Pending::Float { h, old } => {
                    if lvl.exists(h) {
                        let id = lvl.unit(h).id;
                        lvl.line(Line::Float { id, old });
                        let hold = lvl.unit(h).holder;
                        if hold != 0 {
                            lvl.line(Line::Holder { id, old: hold });
                            lvl.unit_mut(h).holder = 0;
                        }
                    }
                }
            }
        }
    }
}

fn reset_scratch(lvl: &mut Level) {
    let vacated = std::mem::take(&mut lvl.scratch.vacated);
    let fell = std::mem::take(&mut lvl.scratch.fell);
    let convs = std::mem::take(&mut lvl.scratch.level_conversions);
    let eb = lvl.scratch.empty_bonus;
    lvl.scratch = Default::default();
    lvl.scratch.vacated = vacated;
    lvl.scratch.level_conversions = convs;
    lvl.scratch.empty_bonus = eb;
    let _ = fell;
    lvl.caches.borrow_mut().powered.clear();
}

/// The move phase (`turn.md` §4.1).
fn move_phase(lvl: &mut Level, keys: Keys) {
    lvl.fx.moving = true;
    status::status_pass(lvl, status::Mode::NoTurn);
    extra::level_you(lvl, keys);
    movement::takes(lvl, keys);
    if lvl.scratch.push_count >= 10000 {
        lvl.request_destroy(DestroyStyle::Infinity);
    }
    flush(lvl);
    rules::refresh(lvl);
    crate::convert::collect(lvl);
    flush(lvl);
    rules::refresh(lvl);
    extra::moveblock(lvl, false);
    let key = if keys.you < 5 { keys.you } else { keys.you2 };
    extra::select(lvl, key);
    if keys.cam < 5 {
        crate::vision::check(lvl);
    }
    lvl.fx.moving = false;
}

/// "Down" in 3D mode switches the camera target and runs the move phase
/// without the camera mover (`turn.md` §3). Returns the camera key.
fn camera_key(lvl: &mut Level, key: u8) -> u8 {
    if lvl.vision.on && key == 3 {
        if !lvl.order.is_empty() {
            crate::vision::change_target(lvl, None);
        }
        5
    } else {
        key
    }
}

fn guard_units(lvl: &mut Level) {
    if lvl.order.len() > 2000 {
        lvl.request_destroy(DestroyStyle::TooComplex);
    }
}

/// End-of-turn update (`turn.md` §4.2).
fn end_of_turn(lvl: &mut Level) {
    // 1
    flush(lvl);
    let fell = std::mem::take(&mut lvl.scratch.fell);
    reset_scratch(lvl);
    lvl.scratch.fell = fell;
    guard_units(lvl);
    lvl.rebuild_tiles();
    // 2
    rules::refresh(lvl);
    // 3
    lvl.fx.decay();
    extra::fall_pass(lvl);
    flush(lvl);
    rules::refresh(lvl);
    status::status_pass(lvl, status::Mode::Full);
    status::level_rotation(lvl, true);
    // 4
    crate::convert::level_collect(lvl);
    extra::fall_pass(lvl);
    // 5
    flush(lvl);
    rules::refresh(lvl);
    // 6
    effects::level::run(lvl);
    effects::block::run(lvl);
    crate::convert::level_apply(lvl);
    // 7
    for h in lvl.order.clone() {
        lvl.unit_mut(h).converted = false;
    }
    // 8
    flush(lvl);
    reset_scratch(lvl);
    lvl.rebuild_tiles();
    // 9
    rules::refresh(lvl);
    lvl.noplayer = !crate::vision::find_player(lvl, false);
    if lvl.fx.turn_sound.is_none() && lvl.turn_changed {
        lvl.turn_sound(crate::events::sounds::MOVE, false);
    }
    // 10
    effects::visual_refresh(lvl);
    lvl.caches.borrow_mut().dice.clear();
    // 11
    party(lvl);
    group_check(lvl);
    execute_destroy(lvl);
    // 12
    lvl.recompute_count = 0;
}

/// One-shot PARTY confetti (`presentation.md` §9.6): 6 particles at every
/// visible unit of each X IS PARTY rule (level, empty and ALL subjects
/// skipped). The units' conditions are tested in the original's order.
fn party(lvl: &mut Level) {
    let rules = lvl.rules.clone();
    for &ri in rules.list("party") {
        let r = &rules.features[ri];
        if r.verb != "is"
            || r.object != "party"
            || ["empty", "all", "level"].contains(&r.subject.as_str())
        {
            continue;
        }
        for h in lvl.units_named(&r.subject).to_vec() {
            if lvl.holds(&r.conds, r.cid, crate::conditions::Subj::Unit(h)) && lvl.unit(h).visible {
                let (x, y, id) = {
                    let u = lvl.unit(h);
                    (u.x, u.y, u.id)
                };
                lvl.burst(
                    crate::events::Particle::Confetti,
                    None,
                    (x, y),
                    6,
                    crate::events::CONFETTI[0],
                    0,
                    Some(id),
                );
            }
        }
    }
}

/// Group membership refresh (`turn.md` §4.2 step 11): the members of
/// every known group name are resolved once more. Not pure: a recursive
/// membership that does not settle within 10 rounds destroys the level
/// (`conditions.md` §2.6), and OFTEN/SELDOM conditions draw.
fn group_check(lvl: &mut Level) {
    if lvl.rules.members.is_empty() {
        return;
    }
    let groups: Vec<String> = lvl
        .known
        .iter()
        .filter(|n| n.starts_with("group"))
        .cloned()
        .collect();
    for g in groups {
        let _ = lvl.group_members(&g);
    }
}

/// Level destruction executes (`turn.md` §11.1).
fn execute_destroy(lvl: &mut Level) {
    let Some(style) = lvl.destroy.take() else {
        return;
    };
    lvl.events.push(crate::events::Event::LevelDestroyed(style));
    // `audio.md` §4.1: silent for the guards (their message plays the
    // infinity sound), a pop otherwise.
    match style {
        DestroyStyle::Infinity | DestroyStyle::TooComplex => {
            lvl.removal_sound(crate::events::Sound::SILENCE, false);
            lvl.events
                .push(crate::events::Event::Sound(crate::events::sounds::INFINITY));
        }
        _ => lvl.removal_sound(crate::events::sounds::POP, false),
    }
    for h in lvl.order.clone() {
        if lvl.alive(h) {
            lvl.remove_recorded(h, Cause::Silent);
        }
    }
    lvl.rules = Arc::new(Default::default());
    lvl.dirty = true;
    lvl.destroyed = true;
    lvl.noplayer = true;
}

/// Post-undo update (`undo.md` §5).
fn undo_update(lvl: &mut Level) {
    flush(lvl);
    reset_scratch(lvl);
    lvl.scratch.fell.clear();
    guard_units(lvl);
    lvl.rebuild_tiles();
    rules::refresh(lvl);
    status::status_pass(lvl, status::Mode::Undoing);
    flush(lvl);
    rules::refresh(lvl);
    for h in lvl.order.clone() {
        lvl.unit_mut(h).converted = false;
    }
    flush(lvl);
    reset_scratch(lvl);
    lvl.rebuild_tiles();
    rules::refresh(lvl);
    lvl.noplayer = !crate::vision::find_player(lvl, true);
    effects::visual_refresh(lvl);
    lvl.caches.borrow_mut().dice.clear();
    lvl.recompute_count = 0;
    lvl.scratch.vacated.clear();
}

/// Level-start passes (the original's start block, `turn.md` §4.2a).
pub(crate) fn level_start(lvl: &mut Level) {
    lvl.recording = false;
    lvl.rebuild_tiles();
    lvl.recording = true;
    rules::refresh(lvl);
    lvl.recompute_count = 0;
    for light in [false, true] {
        // Each start pass begins by clearing the OFTEN/SELDOM cache.
        lvl.caches.borrow_mut().dice.clear();
        if !light {
            lvl.auto_count =
                lvl.count("level", "is", "auto", crate::conditions::Subj::Level) as u32;
            lvl.rotation = 0;
            status::level_rotation(lvl, false);
        }
        // The start pass tests every rule indexed under the unit's rule
        // name (subject or object), in index order, and keeps those that
        // hold; FLOAT among them floats the unit. Testing all of them
        // matters for OFTEN/SELDOM draws (`turn.md` §12).
        for h in lvl.order.clone() {
            let name = lvl.rname(h).to_string();
            let rules = lvl.rules.clone();
            let mut float = false;
            for &ri in rules.list(&name) {
                let r = &rules.features[ri];
                if lvl.holds(&r.conds, r.cid, crate::conditions::Subj::Unit(h))
                    && r.object == "float"
                    && r.verb == "is"
                {
                    float = true;
                }
            }
            if float {
                lvl.unit_mut(h).float = 1;
            }
        }
        lvl.recording = false;
        extra::moveblock(lvl, true);
        lvl.recording = true;
        effects::visual_refresh(lvl);
        if !light {
            // Between the passes the level state is cleared and the rules
            // are parsed again from scratch (fresh condition lists, so
            // OFTEN/SELDOM re-roll); `mapdir` goes back to down until the
            // first turn's rotation pass (`turn.md` §4.2a).
            lvl.caches.borrow_mut().powered.clear();
            lvl.rules = Arc::new(Default::default());
            lvl.dirty = true;
            lvl.mapdir = 3;
            rules::refresh(lvl);
        }
        lvl.recompute_count = 0;
    }
    crate::vision::level_start(lvl);
    lvl.history.clear();
    lvl.record = crate::undo::Record {
        lines: Vec::new(),
        last_key: lvl.last_key,
        rng: Some(lvl.rng.get()),
        vision_ids: lvl.vision_ids(),
        word_ids: lvl.word_ids(),
    };
    lvl.turn_changed = false;
    lvl.events.clear();
    lvl.scratch = Default::default();
    lvl.prune();
}

impl Level {
    /// Applies one input and returns what happened.
    pub fn step(&mut self, input: Input) -> TurnOutcome {
        self.events.clear();
        self.won = false;
        self.ended = false;
        self.all_done = false;
        self.transition = None;
        self.first_turn = self.history.is_empty();
        self.fx.begin_turn();
        let camera_before = self.camera();
        let mut undo_step = false;
        let mut kind = TurnKind::Turn;
        match input {
            Input::Restart => {
                self.restart();
                return TurnOutcome {
                    kind: TurnKind::Restart,
                    no_player: self.no_player(),
                    ..Default::default()
                };
            }
            Input::Undo => {
                kind = TurnKind::Undo;
                self.fx.reset();
                self.events
                    .push(crate::events::Event::Sound(crate::events::sounds::UNDO));
                self.undo_record();
                undo_update(self);
                self.prune();
            }
            Input::Key { dir, player } => {
                let d = dir.min(4);
                if self.auto_count > 0 {
                    // AUTO: the key is only stored for the next auto turn.
                    let p = if player == 2 { 1 } else { 0 };
                    self.auto_dirs[p] = d;
                    if self.auto_dirs[0] == 5 && !self.rules.index.contains_key("you2") {
                        self.auto_dirs[0] = d;
                    }
                    return TurnOutcome {
                        kind: TurnKind::Stored,
                        no_player: self.no_player(),
                        ..Default::default()
                    };
                }
                self.last_key = d;
                let cam = camera_key(self, d);
                let keys = if player == 2 {
                    Keys {
                        you: 5,
                        you2: d,
                        cam,
                    }
                } else {
                    Keys {
                        you: d,
                        you2: 5,
                        cam,
                    }
                };
                move_phase(self, keys);
                end_of_turn(self);
                undo_step = self.commit_record();
                self.scratch.vacated.clear();
                self.prune();
            }
            Input::Tick => {
                let stored = std::mem::replace(&mut self.auto_dirs, [5, 5]);
                let (p1, p2) = (stored[0].min(4), stored[1].min(4));
                let n = self.auto_count;
                if n == 0 {
                    return TurnOutcome {
                        kind: TurnKind::Stored,
                        no_player: self.no_player(),
                        ..Default::default()
                    };
                }
                for _ in 0..n {
                    self.last_key = if p1 < 4 {
                        p1
                    } else if p2 < 4 {
                        p2
                    } else {
                        4
                    };
                    let cam = camera_key(self, p1);
                    move_phase(
                        self,
                        Keys {
                            you: p1,
                            you2: p2,
                            cam,
                        },
                    );
                }
                end_of_turn(self);
                undo_step = self.commit_record();
                self.scratch.vacated.clear();
                self.prune();
            }
        }
        let camera = self.camera();
        if camera.map(|c| c.target) != camera_before.map(|c| c.target)
            || camera.is_some() != camera_before.is_some()
        {
            self.events.push(crate::events::Event::Camera {
                on: camera.is_some(),
                target: camera.and_then(|c| c.target),
            });
        }
        TurnOutcome {
            events: std::mem::take(&mut self.events),
            kind,
            turn_sound: self.fx.turn_sound.take(),
            removal_sound: self.fx.removal_sound.take(),
            won: self.won,
            ended: self.ended,
            all_done: self.all_done,
            no_player: self.no_player(),
            undo_step,
            transition: self.transition.take(),
        }
    }

    /// LEVEL IS AUTO holds: direction keys only store, and the game layer
    /// should fire [`Input::Tick`] on its AUTO timer.
    pub fn auto_active(&self) -> bool {
        self.auto_count > 0
    }

    /// No player exists (music state 1, `audio.md` §3) as of the last
    /// turn's player check; false right after the level start.
    pub fn no_player(&self) -> bool {
        self.noplayer
    }

    /// Reloads the level from its source (`undo.md` §8): history cleared.
    pub fn restart(&mut self) {
        let mut fresh = Level::start(self.source.clone(), self.ctx.clone());
        // The no-player state is engine state that the level start does not
        // reset; the next turn's player check updates it (oracle).
        fresh.noplayer = self.noplayer;
        *self = fresh;
    }
}
