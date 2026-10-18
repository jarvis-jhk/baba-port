//! The level block (`docs/spec/properties.md` §5): empty-tile pairings,
//! LEVEL rules, ALL IS DONE and the unit-count guard.

use std::collections::BTreeSet;

use super::block::{Victims, collect_bonus, leave, players, remove};
use crate::conditions::Subj;
use crate::events::{Cause, DestroyStyle};
use crate::level::Level;
use crate::movement::Thing;
use crate::movement::extra::{level_reversed, level_scroll_by};
use crate::undo::Line;
use crate::unit::{Handle, opposite};

/// Runs the level block.
pub(crate) fn run(lvl: &mut Level) {
    if lvl.destroy.get().is_none() {
        gallery(lvl);
        all_is_done(lvl);
        if !empties(lvl) {
            level_rules(lvl);
        }
    }
    if lvl.order.len() >= 2000 {
        lvl.request_destroy(DestroyStyle::TooComplex);
    }
}

fn destroy_level(lvl: &mut Level, style: DestroyStyle) -> bool {
    if !lvl.level_prop("safe") {
        lvl.request_destroy(style);
        true
    } else {
        false
    }
}

/// §5.1. Returns true if the level block must stop.
fn empties(lvl: &mut Level) -> bool {
    let rules: Vec<(String, String, Vec<crate::rules::Cond>, u32)> = lvl
        .rules
        .list("empty")
        .iter()
        .map(|&ri| &lvl.rules.features[ri])
        .filter(|r| r.subject == "empty" && (r.verb == "is" || r.verb == "eat"))
        .map(|r| (r.verb.clone(), r.object.clone(), r.conds.clone(), r.cid))
        .collect();
    if rules.is_empty() {
        return false;
    }
    let lf = lvl.level_float();
    let lp = |lvl: &Level, p: &str| lvl.level_prop(p);
    let mut victims = Victims::default();
    let mut dead_empties: Vec<(i32, i32)> = Vec::new();
    let mut victory: Vec<(i32, i32)> = Vec::new();
    let mut ending: Vec<(i32, i32)> = Vec::new();
    let main_world = lvl.defs.base_world && !lvl.ctx.in_editor;
    for x in 1..lvl.w - 1 {
        for y in 1..lvl.h - 1 {
            if !lvl.tile_empty(x, y) {
                continue;
            }
            let subj = Subj::Empty(x, y);
            let esafe = lvl.empty_prop("safe", x, y);
            let efloat = lvl.empty_prop("float", x, y) as u8;
            let same = efloat == lf;
            let mut props: BTreeSet<String> = BTreeSet::new();
            let (mut unlock, mut meltf, mut defeat, mut win, mut bonus, mut end) =
                (false, false, false, false, false, false);
            let mut booms = 0;
            let mut end_seen_before_you = false;
            for (v, o, c, cid) in &rules {
                if !lvl.holds(c, *cid, subj) {
                    continue;
                }
                if v == "eat" {
                    if o == "level" && same && !lvl.level_prop("safe") {
                        lvl.request_destroy(DestroyStyle::Plain);
                        return true;
                    }
                    continue;
                }
                if o == "end" && !props.contains("you") {
                    end_seen_before_you = true;
                }
                if o == "boom" {
                    booms += 1;
                }
                props.insert(o.clone());
                let has = |p: &str| props.contains(p);
                let you = has("you") || has("you2") || has("3d");
                unlock |= has("open") && has("shut");
                meltf |= has("melt") && has("hot");
                defeat |= has("defeat") && you;
                win |= has("win") && you;
                bonus |= has("bonus") && you;
                end |= has("end") && you && end_seen_before_you;
                if same {
                    unlock |= (has("shut") && lp(lvl, "open")) || (has("open") && lp(lvl, "shut"));
                    meltf |= has("melt") && lp(lvl, "hot");
                    defeat |= you && lp(lvl, "defeat");
                    win |= you && lp(lvl, "win");
                    bonus |= has("bonus") && lp(lvl, "you");
                    end |= has("end") && lp(lvl, "you");
                }
            }
            let mut alive = true;
            if booms > 0 {
                let rho = (booms - 1).min(lvl.w.max(lvl.h));
                for h in lvl.order.clone() {
                    let u = lvl.unit(h);
                    if u.float == efloat
                        && (u.x - x).abs() <= rho
                        && (u.y - y).abs() <= rho
                        && !lvl.safe(Thing::Unit(h))
                    {
                        victims.add(h);
                    }
                }
                if !esafe {
                    alive = false;
                }
            }
            if !esafe {
                if alive && (unlock || meltf || defeat || bonus) {
                    alive = false;
                }
                if bonus && !lvl.scratch.empty_bonus {
                    lvl.scratch.empty_bonus = true;
                    collect_bonus(lvl);
                }
            }
            if !alive {
                dead_empties.push((x, y));
            }
            if win && alive {
                lvl.won = true;
                return true;
            }
            if end && alive && !lvl.defs.base_world {
                lvl.ended = true;
                lvl.won = true;
                return true;
            }
            if win {
                victory.push((x, y));
            }
            if end {
                ending.push((x, y));
            }
        }
    }
    remove(lvl, victims, false, Cause::Boom);
    for (x, y) in dead_empties {
        lvl.destroy_empty(x, y);
    }
    if victory.iter().any(|&(x, y)| lvl.tile_empty(x, y)) {
        lvl.won = true;
        return true;
    }
    if !main_world && ending.iter().any(|&(x, y)| lvl.tile_empty(x, y)) {
        lvl.ended = true;
        lvl.won = true;
        return true;
    }
    false
}

fn on_level_layer(lvl: &Level, h: Handle) -> bool {
    lvl.unit(h).float == lvl.level_float()
}

fn empty_on_level_layer(lvl: &Level, p: &str) -> bool {
    let lf = lvl.level_float();
    for y in 1..lvl.h - 1 {
        for x in 1..lvl.w - 1 {
            if lvl.tile_empty(x, y)
                && lvl.empty_prop(p, x, y)
                && (lvl.empty_prop("float", x, y) as u8) == lf
            {
                return true;
            }
        }
    }
    false
}

fn level_rules(lvl: &mut Level) {
    let rules: Vec<(String, String, String, Vec<crate::rules::Cond>, u32)> = lvl
        .rules
        .list("level")
        .iter()
        .map(|&ri| &lvl.rules.features[ri])
        .filter(|r| r.subject == "level" || r.object == "level")
        .map(|r| {
            (
                r.subject.clone(),
                r.verb.clone(),
                r.object.clone(),
                r.conds.clone(),
                r.cid,
            )
        })
        .collect();
    let mut teles = 0;
    for (s, v, o, c, cid) in rules {
        if s.starts_with("not ") || o.starts_with("not ") {
            continue;
        }
        if s == "level" {
            if !lvl.holds(&c, cid, Subj::Level) {
                continue;
            }
        } else if v == "eat" && o == "level" {
            // X EAT LEVEL: conditions on the X side.
            let any = if s == "empty" {
                (1..lvl.h - 1).any(|y| {
                    (1..lvl.w - 1)
                        .any(|x| lvl.tile_empty(x, y) && lvl.holds(&c, cid, Subj::Empty(x, y)))
                })
            } else {
                lvl.units_named(&s)
                    .iter()
                    .any(|&h| lvl.holds(&c, cid, Subj::Unit(h)))
            };
            if any && destroy_level(lvl, DestroyStyle::Plain) {
                return;
            }
            continue;
        } else {
            continue;
        }
        let lf = lvl.level_float();
        if v == "eat" {
            if o == "all" {
                continue;
            }
            if o == "empty" {
                for x in 0..lvl.w {
                    for y in 0..lvl.h {
                        if lvl.tile_empty(x, y) && !lvl.is_edge(x, y) {
                            lvl.destroy_empty(x, y);
                        }
                    }
                }
                continue;
            }
            if o == "level" {
                if !lvl.units_named("level").is_empty() && destroy_level(lvl, DestroyStyle::Plain) {
                    return;
                }
                continue;
            }
            let names: Vec<String> = if o.starts_with("group") {
                lvl.group_members(&o)
            } else {
                vec![o.clone()]
            };
            for n in names {
                for h in lvl.units_named(&n).to_vec() {
                    if !lvl.safe(Thing::Unit(h)) {
                        lvl.destroy(h, false, Cause::Eaten);
                    }
                }
            }
            continue;
        }
        if v != "is" {
            continue;
        }
        match o.as_str() {
            "you" | "you2" | "3d" => {
                let defeat = lvl
                    .units_with("defeat")
                    .iter()
                    .any(|&h| on_level_layer(lvl, h))
                    || lvl.level_prop("defeat")
                    || empty_on_level_layer(lvl, "defeat");
                if defeat && destroy_level(lvl, DestroyStyle::Plain) {
                    return;
                }
                let canwin = lvl
                    .units_with("win")
                    .iter()
                    .any(|&h| on_level_layer(lvl, h))
                    || lvl.level_prop("win")
                    || empty_on_level_layer(lvl, "win");
                let canend = lvl
                    .units_with("end")
                    .iter()
                    .any(|&h| on_level_layer(lvl, h))
                    || lvl.level_prop("end")
                    || empty_on_level_layer(lvl, "end");
                let mut any = false;
                for h in lvl.units_with("bonus") {
                    if lvl.alive(h) && on_level_layer(lvl, h) && !lvl.safe(Thing::Unit(h)) {
                        lvl.destroy(h, false, Cause::Bonus);
                        any = true;
                    }
                }
                if any {
                    collect_bonus(lvl);
                }
                if canwin {
                    lvl.won = true;
                    return;
                }
                if canend && !lvl.defs.base_world {
                    lvl.ended = true;
                    lvl.won = true;
                    return;
                }
            }
            "defeat" => {
                for h in players(lvl) {
                    if lvl.alive(h) && on_level_layer(lvl, h) && !lvl.safe(Thing::Unit(h)) {
                        lvl.destroy(h, false, Cause::Defeat);
                    }
                }
                if lvl.level_prop("you") && destroy_level(lvl, DestroyStyle::Plain) {
                    return;
                }
            }
            "weak" => {
                if lvl.order.iter().any(|&h| lvl.unit(h).float == lf)
                    && destroy_level(lvl, DestroyStyle::Plain)
                {
                    return;
                }
            }
            "hot" => {
                for h in lvl.units_with("melt") {
                    if lvl.alive(h) && on_level_layer(lvl, h) && !lvl.safe(Thing::Unit(h)) {
                        lvl.destroy(h, false, Cause::Melt);
                    }
                }
            }
            "melt" => {
                let hot = lvl
                    .units_with("hot")
                    .iter()
                    .any(|&h| on_level_layer(lvl, h))
                    || lvl.level_prop("hot");
                if hot && destroy_level(lvl, DestroyStyle::Plain) {
                    return;
                }
                if empty_on_level_layer(lvl, "hot") && destroy_level(lvl, DestroyStyle::Plain) {
                    return;
                }
            }
            "open" | "shut" => {
                let other = if o == "open" { "shut" } else { "open" };
                let us: Vec<Handle> = lvl
                    .units_with(other)
                    .into_iter()
                    .filter(|&h| on_level_layer(lvl, h))
                    .collect();
                if !us.is_empty() || lvl.level_prop(other) {
                    if destroy_level(lvl, DestroyStyle::Plain) {
                        return;
                    }
                    for h in us {
                        if lvl.alive(h) && !lvl.safe(Thing::Unit(h)) {
                            lvl.destroy(h, false, Cause::Unlock);
                        }
                    }
                }
                if empty_on_level_layer(lvl, other) && destroy_level(lvl, DestroyStyle::Plain) {
                    return;
                }
            }
            "sink" | "boom" => {
                let us: Vec<Handle> = lvl
                    .order
                    .iter()
                    .copied()
                    .filter(|&h| lvl.unit(h).float == lf)
                    .collect();
                if !us.is_empty() {
                    if destroy_level(lvl, DestroyStyle::Plain) {
                        return;
                    }
                    for h in us {
                        if lvl.alive(h) && !lvl.safe(Thing::Unit(h)) {
                            lvl.destroy(
                                h,
                                false,
                                if o == "sink" {
                                    Cause::Sink
                                } else {
                                    Cause::Boom
                                },
                            );
                        }
                    }
                }
            }
            "done" => {
                for h in lvl.order.clone() {
                    if lvl.alive(h) {
                        leave(lvl, h);
                    }
                }
            }
            "bonus" => {
                let p = players(lvl).iter().any(|&h| on_level_layer(lvl, h))
                    || lvl.level_prop("you")
                    || empty_on_level_layer(lvl, "you");
                if p {
                    if destroy_level(lvl, DestroyStyle::Bonus) {
                        return;
                    }
                    collect_bonus(lvl);
                }
            }
            "win" | "end" => {
                let p = players(lvl).iter().any(|&h| on_level_layer(lvl, h))
                    || lvl.level_prop("you")
                    || lvl.level_prop("you2")
                    || lvl.level_prop("3d")
                    || empty_on_level_layer(lvl, "you")
                    || empty_on_level_layer(lvl, "you2")
                    || empty_on_level_layer(lvl, "3d");
                if p {
                    if o == "win" {
                        lvl.won = true;
                        return;
                    } else if !lvl.defs.base_world {
                        lvl.ended = true;
                        lvl.won = true;
                        return;
                    }
                }
            }
            "tele" => {
                teles += 1;
                if teles > 3 || lvl.level_prop("still") {
                    continue;
                }
                lvl.fx.move_kind = Some(crate::events::MoveKind::Teleport);
                for h in lvl.order.clone() {
                    let x = lvl.rand(1, lvl.w - 2);
                    let y = lvl.rand(1, lvl.h - 2);
                    if lvl.alive(h) && lvl.unit(h).float == lf {
                        let d = lvl.unit(h).dir;
                        lvl.update_unit(h, x, y, d);
                    }
                }
                lvl.fx.move_kind = None;
                let (_, short) = lvl.fx.check("tele");
                lvl.turn_sound(crate::events::sounds::TELE, short);
            }
            "move" => {
                if !lvl.level_prop("still") && !lvl.level_prop("sleep") {
                    let md = lvl.mapdir;
                    let d = if level_reversed(lvl) {
                        opposite(md)
                    } else {
                        md
                    };
                    level_scroll_by(lvl, d, 1, false, d);
                }
            }
            "chill" => {
                let d = lvl.rand(0, 3) as u8;
                lvl.line(Line::MapDir { old: lvl.mapdir });
                lvl.mapdir = d;
                if !lvl.level_prop("still") && !lvl.level_prop("sleep") {
                    level_scroll_by(lvl, d, 1, false, d);
                }
            }
            "nudgeright" | "nudgeup" | "nudgeleft" | "nudgedown" => {
                if !lvl.level_prop("still") && !lvl.level_prop("sleep") {
                    let d = match o.as_str() {
                        "nudgeright" => 0,
                        "nudgeup" => 1,
                        "nudgeleft" => 2,
                        _ => 3,
                    };
                    let d = if level_reversed(lvl) { opposite(d) } else { d };
                    let md = lvl.mapdir;
                    level_scroll_by(lvl, d, 1, false, md);
                }
            }
            "fall" | "fallup" | "fallright" | "fallleft" => {
                if !lvl.level_prop("still") {
                    let (d, n) = match o.as_str() {
                        "fall" => (3, 20),
                        "fallup" => (1, 20),
                        "fallright" => (0, 35),
                        _ => (2, 35),
                    };
                    let d = if level_reversed(lvl) { opposite(d) } else { d };
                    level_scroll_by(lvl, d, n, false, d);
                }
            }
            "turn" | "deturn" => {
                let old = lvl.mapdir;
                lvl.line(Line::MapDir { old });
                lvl.line(Line::MapRotation { old: lvl.rotation });
                lvl.mapdir = if o == "turn" {
                    (old + 3) % 4
                } else {
                    (old + 1) % 4
                };
                lvl.rotation = ((old as i32 + 1) % 4) * 90;
                lvl.turn_changed = true;
            }
            "empty" => {
                lvl.request_destroy(DestroyStyle::Empty);
            }
            _ => {}
        }
    }
}

fn all_is_done(lvl: &mut Level) {
    let lit = lvl.rules.list("done").iter().any(|&ri| {
        let r = &lvl.rules.features[ri];
        r.subject == "all" && r.verb == "is" && r.object == "done" && r.conds.is_empty()
    });
    if !lit || lvl.won {
        return;
    }
    let d = lvl.units_with("done");
    let lk = lvl.defs.level_kind;
    if !d.iter().any(|&h| lvl.unit(h).kind != lk) {
        return;
    }
    if lvl.defs.base_world && !lvl.ctx.in_editor {
        lvl.all_done = true;
    } else {
        let total = lvl.order.len();
        let text = lvl.text_units().len();
        let icons = lvl.units_named("level").len();
        if d.len() >= total.saturating_sub(text + icons) {
            lvl.all_done = true;
            lvl.won = true;
        }
    }
    if lvl.all_done {
        lvl.events
            .push(crate::events::Event::Sound(crate::events::sounds::DONE_ALL));
    }
}

/// Gallery special (§5.5): base world `305level` only.
fn gallery(lvl: &mut Level) {
    if !lvl.defs.base_world || lvl.defs.stem != "305level" {
        return;
    }
    let win = lvl.rules.list("win").iter().any(|&ri| {
        let r = &lvl.rules.features[ri];
        r.subject == "image" && r.verb == "is" && r.object == "win" && r.conds.is_empty()
    });
    if win && players(lvl).iter().any(|&h| lvl.unit(h).x > lvl.w - 16) {
        lvl.won = true;
    }
}
