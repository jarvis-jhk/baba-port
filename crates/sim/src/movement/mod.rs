//! Movement (`docs/spec/movement.md`): takes, resolution passes, obstacle
//! checks, pushing, pulling, swapping and contact specials. FALL, the
//! move-block (FOLLOW/BACK/TELE/SHIFT facing), HOLD, SELECT and the level as
//! a mover live in [`extra`].

pub(crate) mod extra;

use std::collections::{BTreeMap, BTreeSet};

use crate::conditions::Subj;
use crate::events::{Cause, DestroyStyle};
use crate::level::Level;
use crate::turn::Pending;
use crate::unit::{Handle, dir_delta, dir_name, opposite};

/// A unit or an empty tile taking part in movement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Thing {
    Unit(Handle),
    Empty(i32, i32),
}

impl Thing {
    pub fn subj(self) -> Subj {
        match self {
            Thing::Unit(h) => Subj::Unit(h),
            Thing::Empty(x, y) => Subj::Empty(x, y),
        }
    }
}

/// Why a record moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reason {
    You,
    Move,
    Auto,
    Chill,
    Nudge(u8),
    Fear,
    Shift,
}

#[derive(Clone, Debug)]
pub(crate) struct Mover {
    pub thing: Thing,
    pub reason: Reason,
    pub state: u8,
    pub moves: u32,
    pub dir: u8,
    pub x: i32,
    pub y: i32,
    finished: bool,
    drop: bool,
}

impl Mover {
    pub fn new(thing: Thing, reason: Reason, dir: u8, x: i32, y: i32) -> Mover {
        Mover {
            thing,
            reason,
            state: 0,
            moves: 1,
            dir,
            x,
            y,
            finished: false,
            drop: false,
        }
    }
}

/// Obstacle entries (§4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ob {
    Free,
    Stop(Option<Thing>),
    Edge,
    Push(Thing),
    Pull(Thing),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sp {
    Lock,
    Eat,
    Weak,
}

#[derive(Clone, Debug)]
pub(crate) struct MoveEntry {
    thing: Thing,
    dx: i32,
    dy: i32,
    facing: u8,
    specials: Vec<(Thing, Sp)>,
    ox: i32,
    oy: i32,
}

/// Per-pass caches (§5, §6).
#[derive(Default)]
pub(crate) struct PassCache {
    test: BTreeMap<(i32, String, u8), u8>,
    outcome: BTreeMap<(i32, String, u8), u8>,
    pulled_once: BTreeSet<(i32, String, u8)>,
    pushed_once: BTreeMap<(i32, String, u8), u8>,
    moves: Vec<MoveEntry>,
    level_pushed: Option<u8>,
    level_pulled: Option<u8>,
}

// --------------------------------------------------------------------
// Predicates on things
// --------------------------------------------------------------------

impl Level {
    pub(crate) fn tprop(&self, t: Thing, p: &str) -> bool {
        match t {
            Thing::Unit(h) => self.prop(h, p),
            Thing::Empty(x, y) => self.empty_prop(p, x, y),
        }
    }

    pub(crate) fn tpos(&self, t: Thing) -> (i32, i32) {
        match t {
            Thing::Unit(h) => {
                let u = self.unit(h);
                (u.x, u.y)
            }
            Thing::Empty(x, y) => (x, y),
        }
    }

    pub(crate) fn tname(&self, t: Thing) -> String {
        match t {
            Thing::Unit(h) => self.uname(h).to_string(),
            Thing::Empty(..) => "empty".into(),
        }
    }

    pub(crate) fn trname(&self, t: Thing) -> String {
        match t {
            Thing::Unit(h) => self.rname(h).to_string(),
            Thing::Empty(..) => "empty".into(),
        }
    }

    /// Float layer of a thing (§1.3 of `turn.md`).
    pub(crate) fn tfloat(&self, t: Thing) -> u8 {
        match t {
            Thing::Unit(h) => self.unit(h).float,
            Thing::Empty(x, y) => self.empty_prop("float", x, y) as u8,
        }
    }

    pub(crate) fn level_float(&self) -> u8 {
        self.level_prop("float") as u8
    }

    pub(crate) fn same_layer(&self, a: Thing, b: Thing) -> bool {
        self.tfloat(a) == self.tfloat(b)
    }

    pub(crate) fn safe(&self, t: Thing) -> bool {
        self.tprop(t, "safe")
    }

    /// Odd number of applicable REVERSE rules (§11).
    pub(crate) fn reversed(&self, t: Thing) -> bool {
        let n = match t {
            Thing::Unit(h) => self.count(self.rname(h), "is", "reverse", Subj::Unit(h)),
            Thing::Empty(x, y) => self.count("empty", "is", "reverse", Subj::Empty(x, y)),
        };
        n % 2 == 1
    }

    /// "T cannot move in d" (§10).
    pub(crate) fn cantmove(&self, t: Thing, d: u8) -> bool {
        if let Thing::Unit(h) = t
            && self.unit(h).float == self.level_float()
            && self.level_prop("hold")
        {
            return true;
        }
        if self.tprop(t, "still") {
            return true;
        }
        if d < 4 {
            let dd = if self.reversed(t) { opposite(d) } else { d };
            if self.tprop(t, &format!("locked{}", dir_name(dd))) {
                return true;
            }
        }
        false
    }

    /// The thing is asleep (the original's helper only works for the level
    /// and empty tiles, §10).
    pub(crate) fn asleep_mover(&self, t: Thing) -> bool {
        self.tprop(t, "sleep")
    }

    /// Last queued position change of a unit in the pending queue.
    pub(crate) fn queued_pos(&self, h: Handle) -> Option<(i32, i32)> {
        self.scratch.pending.iter().rev().find_map(|p| match *p {
            Pending::Move { h: ph, x, y, .. } if ph == h => Some((x, y)),
            _ => None,
        })
    }

    fn eats(&self, t: Thing, o: Thing) -> bool {
        let on = self.trname(o);
        match t {
            Thing::Unit(h) => self.has(self.uname(h), "eat", &on, Subj::Unit(h)),
            Thing::Empty(x, y) => self.has("empty", "eat", &on, Subj::Empty(x, y)),
        }
    }
}

// --------------------------------------------------------------------
// Obstacle check (§4)
// --------------------------------------------------------------------

/// Obstacles of `t` at (x, y) looking in d (pull mode: behind).
pub(crate) fn obstacles(
    lvl: &Level,
    t: Thing,
    x: i32,
    y: i32,
    d: u8,
    pull: bool,
    shift: bool,
) -> (Vec<Ob>, Vec<(Thing, Sp)>) {
    let mut out = Vec::new();
    let mut sp = Vec::new();
    if !pull && lvl.tprop(t, "phantom") {
        return (vec![Ob::Free], sp);
    }
    let partner = if lvl.tprop(t, "open") {
        Some("shut")
    } else if lvl.tprop(t, "shut") {
        Some("open")
    } else {
        None
    };
    let (dx, dy) = dir_delta(d);
    let (tx, ty) = if pull {
        (x - dx, y - dy)
    } else {
        (x + dx, y + dy)
    };
    if lvl.is_edge(tx, ty) {
        out.push(Ob::Edge);
    }
    let here: Vec<Handle> = lvl.at(tx, ty).to_vec();
    let t_safe = lvl.safe(t);
    for o in here {
        if lvl.unit(o).dead {
            continue;
        }
        let ot = Thing::Unit(o);
        let mut ignored = false;
        if let Some((nx, ny)) = lvl.queued_pos(o) {
            if shift && !pull && nx != x && ny != y {
                ignored = true;
            }
            if (nx, ny) == (x, y) || (nx, ny) == (x + 2 * dx, y + 2 * dy) {
                ignored = true;
            }
        }
        // Specials are recorded even for already-moving obstacles.
        if !pull {
            let same = lvl.same_layer(t, ot);
            if let Some(pp) = partner
                && lvl.prop(o, pp)
                && (!t_safe || !lvl.safe(ot))
                && same
            {
                sp.push((ot, Sp::Lock));
                ignored = true;
            }
            if lvl.eats(t, ot) && !lvl.safe(ot) && same {
                sp.push((ot, Sp::Eat));
                ignored = true;
            }
            if lvl.prop(o, "weak") && !lvl.safe(ot) && same {
                sp.push((ot, Sp::Weak));
            }
        }
        if ignored {
            out.push(Ob::Free);
            continue;
        }
        let mut stop = lvl.prop(o, "stop");
        let mut push = lvl.prop(o, "push");
        let mut pullp = lvl.prop(o, "pull");
        let mut swap = lvl.prop(o, "swap");
        if lvl.cantmove(ot, d) {
            if push {
                push = false;
                stop = true;
            }
            if pullp {
                pullp = false;
                stop = true;
            }
            swap = false;
        }
        if lvl.unit(o).kind == lvl.defs.level_kind && !lvl.unit(o).visible {
            stop = false;
        }
        let weak_here = lvl.prop(o, "weak") && lvl.unit(o).float == lvl.tfloat(t);
        let blocks = ((stop && !push && (!pullp || !pull)) || (pullp && !pull && !push)) && !swap;
        if blocks && !weak_here {
            out.push(Ob::Stop(Some(ot)));
        } else if push && !pull && !swap {
            out.push(Ob::Push(ot));
        } else if pullp && pull {
            out.push(Ob::Pull(ot));
        } else {
            out.push(Ob::Free);
        }
    }
    if lvl.tile_empty(tx, ty) && !lvl.is_edge(tx, ty) {
        let e = Thing::Empty(tx, ty);
        if lvl.rules.index.contains_key("empty") {
            let same = lvl.same_layer(t, e);
            let e_safe = lvl.safe(e);
            if !pull && lvl.eats(t, e) && !e_safe && same {
                sp.push((e, Sp::Eat));
                out.push(Ob::Free);
            } else if !pull
                && partner.is_some_and(|pp| lvl.empty_prop(pp, tx, ty))
                && (!e_safe || !t_safe)
                && same
            {
                sp.push((e, Sp::Lock));
                out.push(Ob::Free);
            } else if !pull && lvl.empty_prop("weak", tx, ty) && !e_safe && same {
                sp.push((e, Sp::Weak));
                out.push(Ob::Free);
            } else {
                let push = lvl.empty_prop("push", tx, ty);
                let pullp = lvl.empty_prop("pull", tx, ty);
                let swap = lvl.empty_prop("swap", tx, ty);
                let stop = lvl.empty_prop("stop", tx, ty);
                let cm = (push || swap || pullp) && lvl.cantmove(e, d);
                let blocks = (!push
                    && !swap
                    && ((pullp && !pull) || (pullp && pull && cm) || (!pullp && stop)))
                    || ((push || swap) && cm);
                if blocks {
                    out.push(Ob::Stop(Some(e)));
                } else if push && !pull && !swap {
                    out.push(Ob::Push(e));
                } else if pullp && pull {
                    out.push(Ob::Pull(e));
                } else {
                    out.push(Ob::Free);
                }
            }
        }
    }
    if out.is_empty() {
        out.push(Ob::Free);
    }
    (out, sp)
}

fn key(lvl: &Level, t: Thing, d: u8) -> (i32, String, u8) {
    let (x, y) = lvl.tpos(t);
    (lvl.tile_index(x, y), lvl.tname(t), d)
}

/// Push chain test (§5): 0 can move, 1 blocked.
fn chain_test(
    lvl: &Level,
    pc: &mut PassCache,
    p: Thing,
    d: u8,
    pull: bool,
    pusher: Option<Thing>,
    shift: bool,
) -> u8 {
    let k = key(lvl, p, d);
    if let Some(&v) = pc.test.get(&k) {
        return v;
    }
    if lvl.tprop(p, "weak") && !lvl.safe(p) && !pull {
        pc.test.insert(k, 0);
        return 0;
    }
    let (x, y) = lvl.tpos(p);
    let (obs, _) = obstacles(lvl, p, x, y, d, false, shift);
    let mut r = 0;
    for e in obs {
        let v = match e {
            Ob::Free | Ob::Pull(_) => 0,
            Ob::Stop(who) => {
                if pull && who.is_some() && who == pusher {
                    0
                } else {
                    1
                }
            }
            Ob::Edge => 1,
            Ob::Push(q) => {
                if pull {
                    0
                } else {
                    chain_test(lvl, pc, q, d, false, Some(p), shift)
                }
            }
        };
        r = r.max(v);
    }
    pc.test.insert(k, r);
    r
}

/// Push execution (§6). Returns the outcome (0 moved, 2 blocked).
#[allow(clippy::too_many_arguments)]
fn execute(
    lvl: &mut Level,
    pc: &mut PassCache,
    p: Thing,
    d: u8,
    pull: bool,
    pusher: Option<Thing>,
    memo: &mut BTreeSet<(Thing, u8)>,
    shift: bool,
) -> u8 {
    memo.insert((p, d));
    let (px, py) = lvl.tpos(p);
    let (dx, dy) = dir_delta(d);
    let (nx, ny) = (px + dx, py + dy);
    // 2. swaps at the destination
    if matches!(p, Thing::Unit(_)) || !pull {
        for b in lvl.at(nx, ny).to_vec() {
            let bt = Thing::Unit(b);
            if pull && Some(bt) == pusher {
                continue;
            }
            if lvl.prop(b, "swap") && !lvl.prop(b, "still") && lvl.queued_pos(b).is_none() {
                lvl.scratch.pending.push(Pending::Move {
                    h: b,
                    x: px,
                    y: py,
                    dir: None,
                });
            }
        }
    }
    // 3. pulled swapper
    if pull && lvl.tprop(p, "swap") && !lvl.tprop(p, "still") {
        for b in lvl.at(nx, ny).to_vec() {
            if Some(Thing::Unit(b)) == pusher {
                continue;
            }
            if lvl.queued_pos(b).is_none() {
                lvl.scratch.pending.push(Pending::Move {
                    h: b,
                    x: px,
                    y: py,
                    dir: None,
                });
            }
        }
    }
    // 4. outcome
    let k = key(lvl, p, d);
    let (obs, specials) = obstacles(lvl, p, px, py, d, false, shift);
    let mut outcome = match pc.outcome.get(&k) {
        Some(&v) => v,
        None => {
            let mut r = 0;
            for e in &obs {
                let v = match *e {
                    Ob::Free | Ob::Pull(_) => 0,
                    Ob::Stop(w) => {
                        if pull && w.is_some() && w == pusher {
                            0
                        } else {
                            2
                        }
                    }
                    Ob::Edge => 2,
                    Ob::Push(q) => {
                        if pull && Some(q) == pusher {
                            0
                        } else {
                            1
                        }
                    }
                };
                r = r.max(v);
            }
            pc.outcome.insert(k.clone(), r);
            r
        }
    };
    loop {
        match outcome {
            0 => {
                if matches!(p, Thing::Unit(_)) {
                    // Pushed and pulled things face the push direction.
                    pc.moves.push(MoveEntry {
                        thing: p,
                        dx,
                        dy,
                        facing: d,
                        specials: specials.clone(),
                        ox: px,
                        oy: py,
                    });
                }
                if !pull && pc.pulled_once.insert(k.clone()) {
                    let (behind, _) = obstacles(lvl, p, px, py, d, true, false);
                    for e in behind {
                        if let Ob::Pull(q) = e {
                            if Some(q) == pusher {
                                continue;
                            }
                            if let Thing::Unit(_) = q {
                                let (qx, qy) = lvl.tpos(q);
                                pc.moves.push(MoveEntry {
                                    thing: q,
                                    dx,
                                    dy,
                                    facing: d,
                                    specials: vec![],
                                    ox: qx,
                                    oy: qy,
                                });
                            }
                            if !memo.contains(&(q, d)) {
                                execute(lvl, pc, q, d, true, Some(p), memo, shift);
                            }
                        }
                    }
                }
                break;
            }
            1 => {
                let res = if let Some(&v) = pc.pushed_once.get(&k) {
                    v
                } else {
                    let mut last = 0;
                    for e in &obs {
                        if let Ob::Push(q) = *e {
                            if pull && memo.contains(&(q, d)) {
                                continue;
                            }
                            last = execute(lvl, pc, q, d, false, Some(p), memo, shift);
                        }
                    }
                    pc.pushed_once.insert(k.clone(), last);
                    last
                };
                outcome = if res == 0 { 0 } else { 2 };
            }
            _ => {
                if lvl.tprop(p, "weak") {
                    if let Thing::Unit(h) = p {
                        lvl.destroy(h, false, Cause::Weak);
                    }
                    outcome = 0;
                }
                break;
            }
        }
    }
    if pull {
        let (bx, by) = lvl.tpos(p);
        let (behind, _) = obstacles(lvl, p, bx, by, d, true, false);
        for e in behind {
            if let Ob::Pull(q) = e
                && !memo.contains(&(q, d))
            {
                if let Thing::Unit(_) = q {
                    let (qx, qy) = lvl.tpos(q);
                    pc.moves.push(MoveEntry {
                        thing: q,
                        dx,
                        dy,
                        facing: d,
                        specials: vec![],
                        ox: qx,
                        oy: qy,
                    });
                }
                execute(lvl, pc, q, d, true, Some(p), memo, shift);
            }
        }
    }
    lvl.scratch.push_count += 1;
    if lvl.fx.turn_sound.is_none() {
        lvl.turn_sound(crate::events::sounds::MOVE_HI, false);
    }
    if outcome == 2 { 2 } else { 0 }
}

// --------------------------------------------------------------------
// Pass loop (§3)
// --------------------------------------------------------------------

/// Runs one take (with its final takes) over the given records.
pub(crate) fn run_take(lvl: &mut Level, mut recs: Vec<Mover>) {
    while !recs.is_empty() {
        recs = pass_loop(lvl, recs);
        if lvl.scratch.move_count >= 15000 {
            return;
        }
    }
}

fn pass_loop(lvl: &mut Level, mut recs: Vec<Mover>) -> Vec<Mover> {
    let mut s: u8 = 0;
    let mut carry: Vec<Mover> = Vec::new();
    let mut pc = PassCache::default();
    loop {
        let mut m = u8::MAX;
        for i in 0..recs.len() {
            m = m.min(recs[i].state);
            // Records of units destroyed earlier in the take still act
            // (the original reads removed objects until the frame ends).
            if let Thing::Unit(h) = recs[i].thing
                && !lvl.exists(h)
            {
                recs[i].drop = true;
                continue;
            }
            if recs[i].state == s && recs[i].moves > 0 && !recs[i].drop {
                attempt(lvl, &mut recs, i, s, &mut pc);
                if lvl.scratch.move_count >= 15000 {
                    lvl.request_destroy(DestroyStyle::TooComplex);
                    return Vec::new();
                }
            }
        }
        let mut keep = Vec::new();
        for mut r in recs.drain(..) {
            if r.drop {
                continue;
            }
            if r.finished {
                if r.moves > 0 {
                    r.finished = false;
                    carry.push(r);
                }
            } else {
                keep.push(r);
            }
        }
        recs = keep;
        apply_moves(lvl, &mut pc);
        s = if m > s { s.saturating_add(1) } else { m };
        if recs.is_empty() {
            if let Some(d) = pc.level_pushed {
                extra::level_scroll(lvl, d, true);
            }
            if let Some(d) = pc.level_pulled {
                extra::level_scroll(lvl, d, true);
            }
            crate::turn::flush(lvl);
            break;
        }
        let lp = (pc.level_pushed, pc.level_pulled);
        pc = PassCache::default();
        pc.level_pushed = lp.0;
        pc.level_pulled = lp.1;
        if s > 20 {
            break;
        }
    }
    carry
}

fn attempt(lvl: &mut Level, recs: &mut [Mover], i: usize, s: u8, pc: &mut PassCache) {
    lvl.scratch.move_count += 1;
    let r = recs[i].clone();
    let (x, y) = match r.thing {
        Thing::Unit(h) => {
            let u = lvl.unit(h);
            if u.holder != 0 {
                finish(recs, i, s);
                return;
            }
            (u.x, u.y)
        }
        Thing::Empty(..) => (r.x, r.y),
    };
    if !lvl.on_grid(x, y) {
        finish(recs, i, s);
        return;
    }
    let is_unit = matches!(r.thing, Thing::Unit(_));
    let mut d = match r.thing {
        Thing::Unit(h) => lvl.unit(h).dir,
        Thing::Empty(..) => r.dir,
    };
    if r.state == 0
        && !is_unit
        && ((r.reason == Reason::Move && d > 3) || r.reason == Reason::Chill)
    {
        d = lvl.rand(0, 3) as u8;
        recs[i].dir = d;
        if lvl.cantmove(r.thing, d) {
            finish(recs, i, s);
            return;
        }
    }
    if r.state == 3 && matches!(r.reason, Reason::Move | Reason::Chill) {
        let dd = opposite(d);
        let can = !lvl.cantmove(r.thing, dd);
        if can {
            d = dd;
        }
        if let Thing::Unit(h) = r.thing {
            lvl.set_dir(h, dd);
        } else {
            recs[i].dir = dd;
        }
        if !can {
            finish(recs, i, s);
            return;
        }
    }
    if r.state == 0 && r.reason == Reason::Shift {
        if let Thing::Unit(h) = r.thing {
            lvl.set_dir(h, r.dir);
        }
        d = r.dir;
    }
    if d > 3 {
        d = lvl.rand(0, 3) as u8;
    }
    let facing = d;
    if let Reason::Nudge(nd) = r.reason {
        d = nd;
    }
    let facing = if matches!(r.reason, Reason::Nudge(_)) {
        match r.thing {
            Thing::Unit(h) => lvl.unit(h).dir,
            _ => facing,
        }
    } else {
        facing
    };
    if lvl.reversed(r.thing) {
        d = opposite(d);
    }
    let (obs, specials) = obstacles(lvl, r.thing, x, y, d, false, r.reason == Reason::Shift);
    let (behind, _) = obstacles(lvl, r.thing, x, y, d, true, false);
    let swapper = lvl.tprop(r.thing, "swap") && !lvl.cantmove(r.thing, d);
    let mut res = 0u8;
    let mut pushes: Vec<Thing> = Vec::new();
    for e in &obs {
        match *e {
            Ob::Free | Ob::Pull(_) => {}
            Ob::Edge => {
                res = res.max(2);
                if lvl.level_prop("push") {
                    pc.level_pushed = Some(d);
                }
            }
            Ob::Stop(_) => {
                if !swapper {
                    res = res.max(2);
                    if s < 2 {
                        recs[i].state = recs[i].state.max(2);
                    }
                }
            }
            Ob::Push(q) => {
                if !swapper {
                    res = res.max(1);
                    if s < 1 {
                        recs[i].state = recs[i].state.max(1);
                    } else {
                        pushes.push(q);
                    }
                }
            }
        }
    }
    loop {
        match res {
            0 => {
                success(
                    lvl, recs, i, s, pc, r.thing, x, y, d, facing, specials, &behind,
                );
                return;
            }
            1 => {
                if s < 1 {
                    recs[i].state = 1;
                    return;
                }
                let blocked = pushes.iter().any(|&q| {
                    chain_test(
                        lvl,
                        pc,
                        q,
                        d,
                        false,
                        Some(r.thing),
                        r.reason == Reason::Shift,
                    ) != 0
                });
                if blocked {
                    res = 2;
                } else {
                    for &q in &pushes {
                        let mut memo = BTreeSet::new();
                        execute(
                            lvl,
                            pc,
                            q,
                            d,
                            false,
                            None,
                            &mut memo,
                            r.reason == Reason::Shift,
                        );
                    }
                    res = 0;
                }
            }
            _ => {
                if s < 2 {
                    recs[i].state = 2;
                    return;
                } else if s < 3 {
                    recs[i].state = 3;
                    return;
                } else if matches!(r.reason, Reason::Move | Reason::Chill) && s < 4 {
                    recs[i].state = 4;
                    return;
                }
                if lvl.tprop(r.thing, "weak") && !lvl.safe(r.thing) {
                    if let Thing::Unit(h) = r.thing {
                        lvl.destroy(h, false, Cause::Weak);
                    }
                    recs[i].moves = 1;
                }
                finish(recs, i, s);
                return;
            }
        }
    }
}

fn finish(recs: &mut [Mover], i: usize, s: u8) {
    let r = &mut recs[i];
    r.moves = r.moves.saturating_sub(1);
    r.finished = true;
    if s > 0 {
        r.state = 10;
    } else {
        r.state = 0;
    }
}

#[allow(clippy::too_many_arguments)]
fn success(
    lvl: &mut Level,
    recs: &mut [Mover],
    i: usize,
    s: u8,
    pc: &mut PassCache,
    t: Thing,
    x: i32,
    y: i32,
    d: u8,
    facing: u8,
    specials: Vec<(Thing, Sp)>,
    behind: &[Ob],
) {
    if s > 0 {
        for (j, r) in recs.iter_mut().enumerate() {
            if j != i && r.state >= 2 && r.state < 10 {
                r.state = 0;
            }
        }
    }
    let (dx, dy) = dir_delta(d);
    pc.moves.push(MoveEntry {
        thing: t,
        dx,
        dy,
        facing,
        specials,
        ox: x,
        oy: y,
    });
    // §8 swaps
    let (nx, ny) = (x + dx, y + dy);
    let mover_swap = lvl.tprop(t, "swap") && !lvl.cantmove(t, d);
    // Neither swap path checks for an already queued move (§8).
    let mut swapped = BTreeSet::new();
    if mover_swap {
        for b in lvl.at(nx, ny).to_vec() {
            if Thing::Unit(b) != t && !lvl.prop(b, "still") {
                lvl.scratch.pending.push(Pending::Move {
                    h: b,
                    x,
                    y,
                    dir: None,
                });
                swapped.insert(b);
            }
        }
    }
    for b in lvl.at(nx, ny).to_vec() {
        if Thing::Unit(b) != t
            && !swapped.contains(&b)
            && lvl.prop(b, "swap")
            && !lvl.prop(b, "still")
        {
            lvl.scratch.pending.push(Pending::Move {
                h: b,
                x,
                y,
                dir: None,
            });
        }
    }
    // §7 pulls
    for e in behind {
        match *e {
            Ob::Pull(q) => {
                let mut tc = PassCache::default();
                let blocked = chain_test(lvl, &mut tc, q, d, true, Some(t), false);
                if blocked == 0 {
                    if let Thing::Unit(_) = q {
                        let (qx, qy) = lvl.tpos(q);
                        pc.moves.push(MoveEntry {
                            thing: q,
                            dx,
                            dy,
                            facing: d,
                            specials: vec![],
                            ox: qx,
                            oy: qy,
                        });
                    }
                    let mut memo = BTreeSet::new();
                    execute(lvl, pc, q, d, true, Some(t), &mut memo, false);
                }
            }
            Ob::Edge if lvl.level_prop("pull") => {
                pc.level_pulled = Some(d);
            }
            _ => {}
        }
    }
    let r = &mut recs[i];
    if let Thing::Empty(..) = t {
        r.x += dx;
        r.y += dy;
        r.thing = Thing::Empty(r.x, r.y);
    }
    finish(recs, i, s);
}

/// Applying queued moves (§9).
fn apply_moves(lvl: &mut Level, pc: &mut PassCache) {
    let moves = std::mem::take(&mut pc.moves);
    for e in &moves {
        let (tx, ty) = (e.ox + e.dx, e.oy + e.dy);
        let mut gone = false;
        for &(b, kind) in &e.specials {
            if gone {
                break;
            }
            match b {
                Thing::Unit(bh) => {
                    if lvl.scratch.deleted.contains(&bh) || !lvl.alive(bh) {
                        continue;
                    }
                    if kind != Sp::Weak {
                        let u = lvl.unit(bh);
                        if (u.x, u.y) != (tx, ty) {
                            continue;
                        }
                        if moves
                            .iter()
                            .any(|m| m.thing == b && (m.dx, m.dy) != (tx, ty))
                        {
                            continue;
                        }
                    }
                }
                Thing::Empty(x, y) => {
                    if lvl.scratch.deleted_empty.contains(&lvl.tile_index(x, y)) {
                        continue;
                    }
                }
            }
            match kind {
                Sp::Lock => {
                    if !lvl.safe(b)
                        && let Thing::Unit(bh) = b
                    {
                        lvl.destroy(bh, false, Cause::Unlock);
                    } else if !lvl.safe(b)
                        && let Thing::Empty(x, y) = b
                    {
                        lvl.destroy_empty(x, y);
                    }
                    if let Thing::Unit(th) = e.thing {
                        if !lvl.alive(th) {
                            gone = true;
                        } else if !lvl.safe(e.thing) {
                            lvl.destroy(th, false, Cause::Unlock);
                            gone = true;
                        }
                    }
                }
                Sp::Eat => match b {
                    Thing::Unit(bh) => {
                        lvl.destroy(bh, false, Cause::Eaten);
                    }
                    Thing::Empty(x, y) => lvl.destroy_empty(x, y),
                },
                Sp::Weak => {
                    if let (Thing::Empty(x, y), Thing::Unit(_)) = (b, e.thing) {
                        lvl.destroy_empty(x, y);
                    }
                }
            }
        }
        if let Thing::Unit(h) = e.thing
            && !gone
            && lvl.alive(h)
        {
            lvl.scratch.pending.push(Pending::Move {
                h,
                x: tx,
                y: ty,
                dir: Some(e.facing),
            });
        }
    }
}

// --------------------------------------------------------------------
// Gathering (§2) and the move phase's takes
// --------------------------------------------------------------------

fn add_unit(recs: &mut Vec<Mover>, m: Mover, stack: bool) {
    if let Some(r) = recs
        .iter_mut()
        .find(|r| r.thing == m.thing && matches!(m.thing, Thing::Unit(_)))
    {
        if stack {
            r.moves += 1;
        }
        return;
    }
    recs.push(m);
}

/// Empty tiles (interior, ascending tile index) where an `EMPTY IS p` rule
/// (the given one) holds.
fn empty_tiles_for(lvl: &Level, conds: &[crate::rules::Cond], cid: u32) -> Vec<(i32, i32)> {
    let mut v = Vec::new();
    for y in 1..lvl.h - 1 {
        for x in 1..lvl.w - 1 {
            if lvl.tile_empty(x, y) && lvl.holds(conds, cid, Subj::Empty(x, y)) {
                v.push((x, y));
            }
        }
    }
    v
}

fn empty_rules(lvl: &Level, p: &str) -> Vec<(Vec<crate::rules::Cond>, u32)> {
    lvl.rules
        .list(p)
        .iter()
        .map(|&ri| &lvl.rules.features[ri])
        .filter(|r| r.subject == "empty" && r.verb == "is" && r.object == p)
        .map(|r| (r.conds.clone(), r.cid))
        .collect()
}

/// Players' key directions: (YOU dir, YOU2 dir) per mode.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Keys {
    /// Direction for YOU units (4 = idle, 5 = not driven).
    pub you: u8,
    /// Direction for YOU2 units (5 = not driven).
    pub you2: u8,
    /// The command's key for the 3D camera unit (5 = no camera mover:
    /// "down" in 3D mode switched the target instead).
    pub cam: u8,
}

/// The takes of the move phase (`turn.md` §4.1 step 4).
pub(crate) fn takes(lvl: &mut Level, keys: Keys) {
    // Take 1: YOU.
    let mut recs: Vec<Mover> = Vec::new();
    let mut groups: Vec<(&str, u8)> = Vec::new();
    if keys.you < 5 {
        groups.push(("you", keys.you));
    }
    if keys.you2 < 5 {
        let any_you2 = !lvl.units_with("you2").is_empty();
        if any_you2 {
            groups.push(("you2", keys.you2));
        } else if keys.you >= 5 {
            groups.push(("you", keys.you2));
        }
    }
    let mut seen: BTreeSet<Handle> = BTreeSet::new();
    for (p, d) in groups {
        for h in lvl.units_with(p) {
            if !lvl.alive(h) {
                continue;
            }
            let t = Thing::Unit(h);
            if lvl.asleep_mover(t) {
                continue;
            }
            if d < 4 {
                lvl.set_dir(h, d);
            }
            if lvl.cantmove(t, d) {
                continue;
            }
            if d < 4 && seen.insert(h) {
                let (x, y) = lvl.tpos(t);
                recs.push(Mover::new(t, Reason::You, d, x, y));
            }
        }
        if d < 4
            && let Some((conds, cid)) = empty_rules(lvl, p).into_iter().next()
        {
            for (x, y) in empty_tiles_for(lvl, &conds, cid) {
                let t = Thing::Empty(x, y);
                if lvl.asleep_mover(t) || lvl.cantmove(t, d) {
                    continue;
                }
                recs.push(Mover::new(t, Reason::You, d, x, y));
            }
        }
    }
    camera_mover(lvl, keys.cam, &mut seen, &mut recs);
    run_take(lvl, recs);
    if lvl.destroy.get() == Some(DestroyStyle::TooComplex) {
        return;
    }

    // Take 2: MOVE, AUTO, CHILL.
    let mut recs: Vec<Mover> = Vec::new();
    for (p, reason) in [
        ("move", Reason::Move),
        ("auto", Reason::Auto),
        ("chill", Reason::Chill),
    ] {
        for h in lvl.units_with(p) {
            if !lvl.alive(h) {
                continue;
            }
            let t = Thing::Unit(h);
            if lvl.asleep_mover(t) {
                continue;
            }
            if reason == Reason::Chill {
                let d = lvl.rand(0, 3) as u8;
                lvl.set_dir(h, d);
            }
            let d = lvl.unit(h).dir;
            if lvl.cantmove(t, d) {
                continue;
            }
            let (x, y) = lvl.tpos(t);
            add_unit(&mut recs, Mover::new(t, reason, d, x, y), true);
        }
        for (conds, cid) in empty_rules(lvl, p) {
            for (x, y) in empty_tiles_for(lvl, &conds, cid) {
                let t = Thing::Empty(x, y);
                if lvl.asleep_mover(t) {
                    continue;
                }
                let d = lvl.empty_dir(x, y);
                if d < 4 && lvl.cantmove(t, d) {
                    continue;
                }
                recs.push(Mover::new(t, reason, d, x, y));
            }
        }
    }
    run_take(lvl, recs);

    // Takes 3–6: NUDGE.
    for (p, nd) in [
        ("nudgeright", 0u8),
        ("nudgeup", 1),
        ("nudgeleft", 2),
        ("nudgedown", 3),
    ] {
        let mut recs: Vec<Mover> = Vec::new();
        for h in lvl.units_with(p) {
            if !lvl.alive(h) {
                continue;
            }
            let t = Thing::Unit(h);
            if lvl.asleep_mover(t) || lvl.cantmove(t, nd) {
                continue;
            }
            let (x, y) = lvl.tpos(t);
            add_unit(&mut recs, Mover::new(t, Reason::Nudge(nd), nd, x, y), true);
        }
        for (conds, cid) in empty_rules(lvl, p) {
            for (x, y) in empty_tiles_for(lvl, &conds, cid) {
                let t = Thing::Empty(x, y);
                if lvl.asleep_mover(t) || lvl.cantmove(t, nd) {
                    continue;
                }
                recs.push(Mover::new(t, Reason::Nudge(nd), nd, x, y));
            }
        }
        if !recs.is_empty() {
            run_take(lvl, recs);
        }
    }

    // Take 7: FEAR.
    let recs = gather_fear(lvl);
    run_take(lvl, recs);

    // Take 8: SHIFT.
    let recs = gather_shift(lvl);
    run_take(lvl, recs);
}

/// The 3D camera unit in take 1 (`movement.md` §2.1, §19.2): "up" moves
/// it forward, "right" turns it clockwise, "left" counter-clockwise.
fn camera_mover(lvl: &mut Level, key: u8, seen: &mut BTreeSet<Handle>, recs: &mut Vec<Mover>) {
    if key >= 5 || !lvl.rules.index.contains_key("3d") {
        return;
    }
    let Some(h) = lvl.cam_unit() else {
        return;
    };
    let udir = lvl.unit(h).dir;
    let (forward, turn) = match key {
        1 => (true, 0),
        0 => (false, 3),
        2 => (false, 1),
        _ => (false, 0),
    };
    let d = (udir + turn) % 4;
    let t = Thing::Unit(h);
    if lvl.asleep_mover(t) {
        return;
    }
    let still = lvl.cantmove(t, d);
    lvl.set_dir(h, d);
    if !still && forward && seen.insert(h) {
        let (x, y) = lvl.tpos(t);
        recs.push(Mover::new(t, Reason::You, d, x, y));
    }
}

fn gather_fear(lvl: &mut Level) -> Vec<Mover> {
    let mut recs: Vec<Mover> = Vec::new();
    let groups = lvl.verb_groups("fear");
    for (subject, rules) in groups {
        let things: Vec<Thing> = if subject == "empty" {
            let mut v = Vec::new();
            for y in 1..lvl.h - 1 {
                for x in 1..lvl.w - 1 {
                    if lvl.tile_empty(x, y) {
                        v.push(Thing::Empty(x, y));
                    }
                }
            }
            v
        } else {
            lvl.units_named(&subject)
                .iter()
                .map(|&h| Thing::Unit(h))
                .collect()
        };
        for t in things {
            if let Thing::Unit(h) = t
                && !lvl.alive(h)
            {
                continue;
            }
            let targets: Vec<String> = rules
                .iter()
                .filter(|(_, c, cid)| lvl.holds(c, *cid, t.subj()))
                .map(|(o, _, _)| o.clone())
                .collect();
            if targets.is_empty() {
                continue;
            }
            let (x, y) = lvl.tpos(t);
            let f = match t {
                Thing::Unit(h) => lvl.unit(h).dir,
                Thing::Empty(..) => lvl.empty_dir(x, y),
            };
            let f = if f > 3 { 0 } else { f };
            let order = [(f + 2) % 4, (f + 3) % 4, f, (f + 1) % 4];
            let mut counts = [0usize; 4];
            let mut provisional: Option<u8> = None;
            for &nd in &order {
                let (dx, dy) = dir_delta(nd);
                let (nx, ny) = (x + dx, y + dy);
                let mut c = 0;
                for tn in &targets {
                    let present = if tn == "empty" {
                        lvl.inside(nx, ny) && lvl.tile_empty(nx, ny)
                    } else {
                        lvl.at(nx, ny).iter().any(|&o| lvl.rname(o) == tn)
                    };
                    if present {
                        c += 1;
                    }
                }
                counts[nd as usize] = c;
                if c > 0 {
                    provisional = Some(opposite(nd));
                }
            }
            let max = *counts.iter().max().unwrap();
            if max == 0 {
                continue;
            }
            for c in counts.iter_mut() {
                if *c < max {
                    *c = 0;
                }
            }
            let nmax = counts.iter().filter(|&&c| c == max).count();
            let flee = if nmax == 1 {
                provisional
            } else {
                let mut res = None;
                for cand in [f, (f + 3) % 4, (f + 1) % 4, (f + 2) % 4] {
                    if counts[cand as usize] == 1 {
                        continue;
                    }
                    let (obs, _) = obstacles(lvl, t, x, y, cand, false, false);
                    let mut pc = PassCache::default();
                    let blocked = obs.iter().any(|e| match *e {
                        Ob::Edge | Ob::Stop(_) => true,
                        Ob::Push(q) => {
                            chain_test(lvl, &mut pc, q, cand, false, Some(t), false) != 0
                        }
                        _ => false,
                    });
                    if !blocked {
                        res = Some(cand);
                        break;
                    }
                }
                res
            };
            let Some(fd) = flee else { continue };
            if lvl.asleep_mover(t) {
                continue;
            }
            if let Thing::Unit(h) = t {
                lvl.set_dir(h, fd);
            }
            if lvl.cantmove(t, fd) {
                continue;
            }
            let mut m = Mover::new(t, Reason::Fear, fd, x, y);
            m.moves = max as u32;
            if let Some(r) = recs.iter_mut().find(|r| r.thing == t) {
                r.moves += 1;
            } else {
                recs.push(m);
            }
        }
    }
    recs
}

fn gather_shift(lvl: &mut Level) -> Vec<Mover> {
    let mut recs: Vec<Mover> = Vec::new();
    for s in lvl.units_with("shift") {
        if !lvl.alive(s) {
            continue;
        }
        let (x, y, d, fl) = {
            let u = lvl.unit(s);
            (u.x, u.y, u.dir, u.float)
        };
        let here: Vec<Handle> = lvl.at(x, y).to_vec();
        if here.len() <= 1 {
            continue;
        }
        for o in here {
            if o == s || lvl.unit(o).float != fl {
                continue;
            }
            let t = Thing::Unit(o);
            if lvl.cantmove(t, d) {
                continue;
            }
            if let Some(r) = recs.iter_mut().find(|r| r.thing == t) {
                r.moves += 1;
            } else {
                recs.push(Mover::new(t, Reason::Shift, d, x, y));
            }
        }
    }
    if lvl.level_prop("shift") {
        let lf = lvl.level_float();
        let md = lvl.mapdir;
        for h in lvl.order.clone() {
            if lvl.unit(h).float != lf {
                continue;
            }
            lvl.set_dir(h, md);
            let t = Thing::Unit(h);
            if !lvl.cantmove(t, md) {
                let (x, y) = lvl.tpos(t);
                recs.push(Mover::new(t, Reason::Shift, md, x, y));
            }
        }
    }
    recs
}
