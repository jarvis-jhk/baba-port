//! FALL (`movement.md` §12), the move-block (§13: FOLLOW, BACK, TELE, SHIFT
//! facing), HOLD (§15), SELECT (§18) and the level as a mover (§19).

use std::collections::BTreeMap;

use super::{Ob, Sp, Thing, obstacles};
use crate::conditions::Subj;
use crate::events::{Cause, DestroyStyle, Event, MoveKind, Particle};
use crate::level::{Level, RegMode};
use crate::undo::{CreateSource, Line};
use crate::unit::{Handle, dir_delta, opposite};

// --------------------------------------------------------------------
// Level scrolling (§17.1, §19.1)
// --------------------------------------------------------------------

/// Shifts the level's screen offset by `amount` tiles in d and records
/// the undo line. `set_mapdir`: also set `mapdir` := d.
pub(crate) fn level_scroll_by(
    lvl: &mut Level,
    d: u8,
    amount: i32,
    set_mapdir: bool,
    stored_mapdir: u8,
) {
    let old = lvl.offset;
    let (dx, dy) = dir_delta(d);
    lvl.offset = (old.0 + dx * amount, old.1 + dy * amount);
    lvl.line(Line::LevelOffset {
        old,
        old_mapdir: stored_mapdir,
    });
    lvl.turn_changed = true;
    if set_mapdir && d < 4 {
        lvl.mapdir = d;
    }
    lvl.events.push(Event::LevelMoved {
        offset: lvl.offset,
        mapdir: lvl.mapdir,
        rotation: lvl.rotation,
    });
}

/// Level pushed / pulled at the edge: one tile, `mapdir` := d.
pub(crate) fn level_scroll(lvl: &mut Level, d: u8, set_mapdir: bool) {
    let md = lvl.mapdir;
    level_scroll_by(lvl, d, 1, set_mapdir, md);
}

/// Level can move in d (not STILL, not LOCKED that way).
pub(crate) fn level_can_move(lvl: &Level, d: u8) -> bool {
    if lvl.level_prop("still") {
        return false;
    }
    if d < 4 {
        let dd = if level_reversed(lvl) { opposite(d) } else { d };
        if lvl.level_prop(&format!("locked{}", crate::unit::dir_name(dd))) {
            return false;
        }
    }
    true
}

pub(crate) fn level_reversed(lvl: &Level) -> bool {
    lvl.count("level", "is", "reverse", Subj::Level) % 2 == 1
}

/// LEVEL IS YOU before the takes (§19.1).
pub(crate) fn level_you(lvl: &mut Level, keys: super::Keys) {
    let mut players: Vec<(&str, u8)> = Vec::new();
    if keys.you < 5 && keys.you2 < 5 {
        // Both-players mode (AUTO turns): one level move with player 1's
        // direction if LEVEL IS YOU or LEVEL IS YOU2 holds (turn.md §3).
        if !lvl.level_prop("you") && !lvl.level_prop("you2") {
            return;
        }
        players.push(("", keys.you));
    } else if keys.you < 5 {
        players.push(("you", keys.you));
    }
    if keys.you2 < 5 && keys.you >= 5 {
        // Player 2 drives LEVEL IS YOU2, or LEVEL IS YOU when no LEVEL IS
        // YOU2 rule exists at all (conditions not tested).
        if !lvl.find(Some("level"), "is", Some("you2")).is_empty() {
            players.push(("you2", keys.you2));
        } else {
            players.push(("you", keys.you2));
        }
    }
    for (p, d) in players {
        if !p.is_empty() && !lvl.level_prop(p) {
            continue;
        }
        let dd = if d < 4 && level_reversed(lvl) {
            opposite(d)
        } else {
            d
        };
        if !level_can_move(lvl, d) {
            continue;
        }
        let md = lvl.mapdir;
        if dd < 4 {
            level_scroll_by(lvl, dd, 1, true, md);
        } else {
            level_scroll_by(lvl, 0, 0, false, md);
        }
    }
}

// --------------------------------------------------------------------
// FALL (§12)
// --------------------------------------------------------------------

/// One fall pass.
pub(crate) fn fall_pass(lvl: &mut Level) {
    lvl.fx.move_kind = Some(MoveKind::Fall);
    fall_pass_inner(lvl);
    lvl.fx.move_kind = None;
}

fn fall_pass_inner(lvl: &mut Level) {
    let mut lists: [Vec<Handle>; 4] = Default::default();
    for (p, d) in [
        ("fall", 3u8),
        ("fallright", 0),
        ("fallup", 1),
        ("fallleft", 2),
    ] {
        for h in lvl.units_with(p) {
            let d = if lvl.reversed(Thing::Unit(h)) {
                opposite(d)
            } else {
                d
            };
            lists[d as usize].push(h);
        }
    }
    if lists.iter().all(|l| l.is_empty()) {
        return;
    }
    let mut cnt: BTreeMap<Handle, [i32; 4]> = BTreeMap::new();
    for &h in &lists[3] {
        cnt.entry(h).or_default()[3] += 1;
    }
    for &h in &lists[0] {
        cnt.entry(h).or_default()[0] += 1;
    }
    for &h in &lists[1] {
        let c = cnt.entry(h).or_default();
        if c[3] > 0 {
            c[3] -= 1;
        } else {
            c[1] += 1;
        }
    }
    for &h in &lists[2] {
        let c = cnt.entry(h).or_default();
        if c[0] > 0 {
            c[0] -= 1;
        } else {
            c[2] += 1;
        }
    }
    let mut entries: Vec<(Handle, u8)> = Vec::new();
    for d in [3u8, 0, 1, 2] {
        for &h in &lists[d as usize] {
            entries.push((h, d));
        }
    }
    let mut first_dir: BTreeMap<Handle, u8> = BTreeMap::new();
    for &(h, d) in &entries {
        first_dir.entry(h).or_insert(d);
    }
    let mut steps = 0u32;
    let mut dropped = vec![false; entries.len()];
    loop {
        let mut moved = false;
        for (ei, &(h, d)) in entries.iter().enumerate() {
            if dropped[ei] || !lvl.alive(h) {
                continue;
            }
            if cnt.get(&h).map_or(0, |c| c[d as usize]) <= 0 {
                continue;
            }
            if lvl.scratch.fell.get(&h).is_some_and(|&f| f != d) {
                continue;
            }
            if lvl.cantmove(Thing::Unit(h), d) {
                continue;
            }
            let (dx, dy) = dir_delta(d);
            loop {
                let (x, y, dir) = {
                    let u = lvl.unit(h);
                    (u.x, u.y, u.dir)
                };
                if !lvl.on_grid(x + dx, y + dy) {
                    break;
                }
                let (obs, sp) = obstacles(lvl, Thing::Unit(h), x, y, d, false, false);
                if obs.iter().any(|e| !matches!(e, Ob::Free)) {
                    break;
                }
                let mut weak_targets = Vec::new();
                for (b, k) in sp {
                    match k {
                        Sp::Lock => {
                            if !lvl.safe(b) {
                                destroy_thing(lvl, b, Cause::Unlock);
                            }
                            if !lvl.safe(Thing::Unit(h)) {
                                lvl.destroy(h, false, Cause::Unlock);
                            }
                        }
                        Sp::Eat => destroy_thing(lvl, b, Cause::Eaten),
                        Sp::Weak => weak_targets.push(b),
                    }
                }
                if !lvl.alive(h) {
                    dropped[ei] = true;
                    break;
                }
                lvl.update_unit(h, x + dx, y + dy, dir);
                for b in weak_targets {
                    destroy_thing(lvl, b, Cause::Weak);
                }
                moved = true;
                steps += 1;
                if steps >= 6000 {
                    lvl.request_destroy(DestroyStyle::Infinity);
                    return;
                }
                if !lvl.alive(h) {
                    dropped[ei] = true;
                    break;
                }
            }
        }
        if !moved {
            break;
        }
    }
    for (h, d) in first_dir {
        lvl.scratch.fell.entry(h).or_insert(d);
    }
}

fn destroy_thing(lvl: &mut Level, t: Thing, cause: Cause) {
    match t {
        Thing::Unit(h) => {
            lvl.destroy(h, false, cause);
        }
        Thing::Empty(x, y) => lvl.destroy_empty(x, y),
    }
}

// --------------------------------------------------------------------
// Move-block (§13)
// --------------------------------------------------------------------

/// `isgone` predicate (§13.5).
pub(crate) fn is_gone(lvl: &Level, u: Handle) -> bool {
    let t = Thing::Unit(u);
    if lvl.safe(t) {
        return false;
    }
    let (x, y, fl) = {
        let v = lvl.unit(u);
        (v.x, v.y, v.float)
    };
    let un = lvl.rname(u).to_string();
    let you = lvl.prop(u, "you") || lvl.prop(u, "you2") || lvl.prop(u, "3d");
    let melt = lvl.prop(u, "melt");
    let shut = lvl.prop(u, "shut");
    let open = lvl.prop(u, "open");
    let weak = lvl.prop(u, "weak");
    for &o in lvl.at(x, y) {
        if o == u {
            continue;
        }
        if lvl.has(lvl.uname(o), "eat", &un, Subj::Unit(o)) {
            return true;
        }
        let same = lvl.unit(o).float == fl;
        if !same {
            continue;
        }
        if lvl.prop(o, "sink")
            || (you && lvl.prop(o, "defeat"))
            || (melt && lvl.prop(o, "hot"))
            || (shut && lvl.prop(o, "open"))
            || (open && lvl.prop(o, "shut"))
            || weak
        {
            return true;
        }
    }
    false
}

/// The move-block (`turn.md` §4.1 step 13). `start`: level-start variant
/// (FOLLOW only, no undo lines).
pub(crate) fn moveblock(lvl: &mut Level, start: bool) {
    follow(lvl, start);
    if start {
        return;
    }
    lvl.fx.move_kind = Some(MoveKind::Back);
    back(lvl);
    crate::turn::flush(lvl);
    lvl.fx.move_kind = Some(MoveKind::Teleport);
    tele(lvl);
    lvl.fx.move_kind = None;
    shift_facing(lvl);
    crate::turn::flush(lvl);
}

fn follow(lvl: &mut Level, start: bool) {
    let rules: Vec<(String, Vec<crate::rules::Cond>, u32)> = lvl
        .rules
        .list("follow")
        .iter()
        .map(|&ri| &lvl.rules.features[ri])
        .filter(|r| {
            r.verb == "follow" && !r.subject.starts_with("not ") && !r.object.starts_with("not ")
        })
        .map(|r| (r.subject.clone(), r.conds.clone(), r.cid))
        .collect();
    for (s, conds, cid) in rules {
        if s == "empty" || s == "level" && false {
            continue;
        }
        for u in lvl.units_named(&s).to_vec() {
            if !lvl.alive(u) || !lvl.holds(&conds, cid, Subj::Unit(u)) {
                continue;
            }
            follow_unit(lvl, u, start);
        }
    }
}

fn follow_unit(lvl: &mut Level, u: Handle, start: bool) {
    let name = lvl.uname(u).to_string();
    let rn = lvl.rname(u).to_string();
    let mut targets: Vec<String> = Vec::new();
    for subj in [name.as_str(), rn.as_str()] {
        for &ri in lvl.rules.list(subj) {
            let r = &lvl.rules.features[ri];
            if r.subject == subj
                && r.verb == "follow"
                && !r.object.starts_with("not ")
                && lvl.holds(&r.conds, r.cid, Subj::Unit(u))
                && !targets.contains(&r.object)
            {
                targets.push(r.object.clone());
            }
        }
        if name == rn {
            break;
        }
    }
    let (ux, uy, f, mem) = {
        let v = lvl.unit(u);
        (v.x, v.y, v.dir, v.followed)
    };
    let mut best = i32::MAX;
    let mut chosen: Option<u8> = None;
    let mut stopped = false;
    let mut adjacent: Option<(Handle, u8)> = None;
    let mut priority: Option<(Handle, u8)> = None;
    let mut lock_front = false;
    let mut lock_ccw = false;
    'outer: for t in &targets {
        let names: Vec<String> = if t.starts_with("group") {
            lvl.group_members(t)
        } else {
            vec![t.clone()]
        };
        for n in names {
            for &b in lvl.units_named(&n) {
                if b == u || lvl.unit(b).dead {
                    continue;
                }
                let (bx, by) = {
                    let v = lvl.unit(b);
                    (v.x, v.y)
                };
                let (dx, dy) = (bx - ux, by - uy);
                let dist = dx.abs() + dy.abs();
                let dir = if dx.abs() <= dy.abs() {
                    if dy >= 0 { 3 } else { 1 }
                } else if dx > 0 {
                    0
                } else {
                    2
                };
                if dist > 0 && dist <= best {
                    best = dist;
                    chosen = Some(dir);
                    if dist == 1 {
                        if mem == lvl.unit(b).id {
                            stopped = true;
                            break 'outer;
                        }
                        adjacent = Some((b, dir));
                        let fr = dir_delta(f);
                        let ccw = dir_delta((f + 1) % 4);
                        let cw = dir_delta((f + 3) % 4);
                        if (dx, dy) == fr {
                            priority = Some((b, dir));
                            lock_front = true;
                        } else if !lock_front && (dx, dy) == ccw {
                            priority = Some((b, dir));
                            lock_ccw = true;
                        } else if !lock_front && !lock_ccw && (dx, dy) == cw {
                            priority = Some((b, dir));
                        }
                    }
                }
            }
        }
    }
    if !stopped {
        let pick = priority.or(adjacent);
        if let Some((b, dir)) = pick {
            let id = lvl.unit(b).id;
            set_followed(lvl, u, id, start);
            chosen = Some(dir);
        } else if mem != -1 {
            set_followed(lvl, u, -1, start);
        }
    }
    if let Some(d) = chosen {
        if start {
            let (x, y) = (lvl.unit(u).x, lvl.unit(u).y);
            lvl.place_unit(u, x, y, d);
        } else {
            lvl.set_dir(u, d);
        }
    }
}

fn set_followed(lvl: &mut Level, u: Handle, id: i32, start: bool) {
    let (uid, old) = {
        let v = lvl.unit(u);
        (v.id, v.followed)
    };
    if old == id {
        return;
    }
    if !start {
        lvl.line(Line::Followed { id: uid, old });
    }
    lvl.unit_mut(u).followed = id;
}

fn back(lvl: &mut Level) {
    if !lvl.any_is("back") {
        return;
    }
    for u in lvl.units_with("back") {
        if !lvl.alive(u) {
            continue;
        }
        let bs = lvl.unit(u).back_start;
        if bs == 0 {
            continue;
        }
        let nrec = lvl.history.len() as i64 + 1;
        let k = nrec - bs as i64;
        let target = 2 * k + 1;
        if target < 1 {
            continue;
        }
        let rec = if target == 1 {
            std::sync::Arc::new(lvl.record.clone())
        } else {
            let i = lvl.history.len() as i64 - (target - 1);
            if i < 0 {
                continue;
            }
            lvl.history[i as usize].clone()
        };
        let id = lvl.unit(u).id;
        // A replay step marks the turn changed even when the record holds
        // nothing for this unit (so idle turns while backing are steps).
        lvl.turn_changed = true;
        let at = (lvl.unit(u).x, lvl.unit(u).y);
        lvl.burst(Particle::Wonder, None, at, 1, (3, 0), 0, None);
        for l in rec.lines.iter().rev() {
            match l {
                Line::Moved { id: lid, old, new } if *lid == id => {
                    if (old.0, old.1) != (new.0, new.1) {
                        lvl.scratch.pending.push(crate::turn::Pending::Move {
                            h: u,
                            x: old.0,
                            y: old.1,
                            dir: Some(old.2),
                        });
                    } else {
                        lvl.scratch
                            .pending
                            .push(crate::turn::Pending::Dir { h: u, dir: old.2 });
                    }
                    lvl.scratch.teleported.insert(u);
                }
                Line::Created { id: lid, base, .. } if *lid == id => {
                    for l2 in rec.lines.iter() {
                        if let Line::Removed(snap) = l2
                            && snap.id == *base
                            && lvl.alive(u)
                        {
                            let (x, y) = (lvl.unit(u).x, lvl.unit(u).y);
                            let mut s = (**snap).clone();
                            s.x = x;
                            s.y = y;
                            let nid = s.id;
                            let kind = s.kind;
                            let dir = s.dir;
                            lvl.line(Line::Created {
                                id: nid,
                                kind,
                                base: -1,
                                source: CreateSource::Back,
                                at: (x, y, dir),
                            });
                            lvl.turn_changed = true;
                            lvl.register(s, RegMode::Normal);
                            lvl.remove_recorded(u, Cause::Silent);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn tele(lvl: &mut Level) {
    let teles = lvl.units_with("tele");
    if teles.is_empty() {
        return;
    }
    for &t in &teles {
        if !lvl.alive(t) || is_gone(lvl, t) {
            continue;
        }
        let (x, y, fl, tn) = {
            let v = lvl.unit(t);
            (v.x, v.y, v.float, v.kind)
        };
        for v in lvl.at(x, y).to_vec() {
            if v == t || !lvl.alive(v) {
                continue;
            }
            if lvl.prop(v, "still")
                || lvl.unit(v).float != fl
                || lvl.scratch.teleported.contains(&v)
                || lvl.unit(v).kind == tn
            {
                continue;
            }
            if teles.len() <= 1 {
                continue;
            }
            let cands: Vec<Handle> = teles
                .iter()
                .copied()
                .filter(|&o| o != t && lvl.alive(o) && lvl.unit(o).kind == tn)
                .collect();
            if cands.is_empty() {
                continue;
            }
            let i = lvl.rand(0, cands.len() as i32 - 1) as usize;
            let dest = cands[i];
            let (dx, dy) = (lvl.unit(dest).x, lvl.unit(dest).y);
            lvl.scratch.teleported.insert(v);
            let (vx, vy, dir) = {
                let w = lvl.unit(v);
                (w.x, w.y, w.dir)
            };
            lvl.update_unit(v, dx, dy, dir);
            // `presentation.md` §9.3, `audio.md` §4.1.
            let short = lvl.burst(Particle::Glow, Some("tele"), (vx, vy), 5, (1, 4), 0, None);
            lvl.burst(Particle::Glow, None, (dx, dy), 5, (1, 4), 0, None);
            lvl.turn_sound(crate::events::sounds::TELE, short);
        }
    }
}

fn shift_facing(lvl: &mut Level) {
    for s in lvl.units_with("shift") {
        if !lvl.alive(s) || is_gone(lvl, s) {
            continue;
        }
        let (x, y, fl, d) = {
            let v = lvl.unit(s);
            (v.x, v.y, v.float, v.dir)
        };
        for o in lvl.at(x, y).to_vec() {
            if o == s || !lvl.alive(o) || lvl.unit(o).float != fl {
                continue;
            }
            let dirty = lvl.dirty;
            lvl.set_dir(o, d);
            lvl.dirty = dirty;
        }
    }
}

// --------------------------------------------------------------------
// HOLD (§15)
// --------------------------------------------------------------------

/// HOLD pass (`first`: block item 4; else item 13).
pub(crate) fn hold(lvl: &mut Level, first: bool) {
    if !first && !lvl.any_is("hold") {
        return;
    }
    for h in lvl.units_with("hold") {
        if !lvl.alive(h) {
            continue;
        }
        let (x, y, fl, id) = {
            let v = lvl.unit(h);
            (v.x, v.y, v.float, v.id)
        };
        for b in lvl.at(x, y).to_vec() {
            if b == h || lvl.unit(b).float != fl {
                continue;
            }
            let (bid, old) = (lvl.unit(b).id, lvl.unit(b).holder);
            if old != id || !first {
                lvl.line(Line::Holder { id: bid, old });
                lvl.unit_mut(b).holder = id;
            }
        }
    }
    for b in lvl.order.clone() {
        let hid = lvl.unit(b).holder;
        if hid == 0 {
            continue;
        }
        let holder = lvl.find_id(hid);
        let still_hold = holder.is_some_and(|h| lvl.prop(h, "hold"));
        let release = |lvl: &mut Level, b: Handle| {
            let (bid, old) = (lvl.unit(b).id, lvl.unit(b).holder);
            lvl.line(Line::Holder { id: bid, old });
            lvl.unit_mut(b).holder = 0;
        };
        let Some(hh) = holder.filter(|_| still_hold) else {
            release(lvl, b);
            continue;
        };
        let (hx, hy, hf) = {
            let v = lvl.unit(hh);
            (v.x, v.y, v.float)
        };
        let (bx, by, bd) = {
            let v = lvl.unit(b);
            (v.x, v.y, v.dir)
        };
        if first {
            let (dx, dy) = (hx - bx, hy - by);
            let d = match (dx, dy) {
                (1, 0) => 0,
                (0, -1) => 1,
                (-1, 0) => 2,
                (0, 1) => 3,
                _ => 4,
            };
            if lvl.cantmove(Thing::Unit(b), d) {
                release(lvl, b);
                continue;
            }
            lvl.update_unit(b, hx, hy, bd);
            if lvl.unit(b).float != hf {
                release(lvl, b);
            }
        } else {
            lvl.update_unit(b, hx, hy, bd);
        }
    }
}

// --------------------------------------------------------------------
// SELECT (§18)
// --------------------------------------------------------------------

/// Map cursors move with the key (§18, §18.1).
pub(crate) fn select(lvl: &mut Level, key: u8) {
    if key > 4 {
        return;
    }
    let cursors: Vec<Handle> = lvl
        .units_with("select")
        .into_iter()
        .filter(|&h| lvl.unit(h).kind != lvl.defs.level_kind)
        .collect();
    let mut seen = Vec::new();
    let mut d = key;
    for c in cursors {
        if seen.contains(&c) || !lvl.alive(c) {
            continue;
        }
        seen.push(c);
        let t = Thing::Unit(c);
        let rev = lvl.reversed(t);
        let (mut ox, mut oy) = dir_delta(key);
        if rev {
            d = opposite(d);
            ox = -ox;
            oy = -oy;
        }
        if lvl.cantmove(t, d) {
            continue;
        }
        let (x, y, od, id) = {
            let v = lvl.unit(c);
            (v.x, v.y, v.dir, v.id)
        };
        let (nx, ny) = (x + ox, y + oy);
        let mut target: Option<Handle> = None;
        for &o in lvl.at(nx, ny) {
            if o == c {
                continue;
            }
            let v = lvl.unit(o);
            if v.visible && !v.dead && v.completion() > 1 {
                target = Some(o);
                break;
            }
        }
        let on_path = target.is_none()
            && lvl
                .paths
                .iter()
                .any(|p| p.x == nx && p.y == ny && p.completion > 1);
        if target.is_none() && !on_path {
            continue;
        }
        // Idle sets the facing to 4 (none), as the original does.
        let nd = d;
        lvl.line(Line::MapCursor {
            id,
            old: (x, y, od),
        });
        lvl.turn_changed = true;
        lvl.place_unit(c, nx, ny, nd);
        match target {
            Some(tg) if lvl.unit(tg).has_level_file() => {
                lvl.cursor_link.insert(c, tg);
            }
            _ => {
                lvl.cursor_link.remove(&c);
            }
        }
    }
}
