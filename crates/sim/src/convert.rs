//! Conversion and creation (`docs/spec/conversion.md`): X IS Y, TEXT,
//! WRITE, BECOME, REVERT, ALL, EMPTY conversions, LEVEL conversions, HAS and
//! MAKE.

use crate::conditions::Subj;
use crate::defs::{KindId, in_full, is_group};
use crate::events::{Cause, DestroyStyle, Event, Transition};
use crate::level::Level;
use crate::rules::Cond;
use crate::turn::Pending;
use crate::undo::{CreateSource, Line};
use crate::unit::{Handle, Unit};

/// Conversion target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Name(String),
    Paradox,
    All,
    Revert,
}

/// One entry of a subject's conversion list.
#[derive(Clone, Debug)]
pub(crate) struct ConvEntry {
    pub target: Target,
    pub conds: Vec<Cond>,
    pub cid: u32,
    pub verb: String,
}

/// One queued output: target and id.
#[derive(Clone, Debug)]
pub(crate) struct Output {
    pub target: Target,
    pub id: i32,
}

/// Gate of `conversion.md` §3.1.
fn opens(lvl: &Level, s: &str, v: &str, o: &str) -> bool {
    lvl.known.contains(o)
        || o == format!("not {s}")
        || o == "all"
        || lvl.defs.exists(o)
        || (o == "text" && lvl.defs.exists("text_text"))
        || o == "revert"
        || (v == "write" && lvl.defs.active_lookup(&format!("text_{s}")).is_some())
}

/// Conversion list of subject `s` (§3 step 1).
fn conv_list(lvl: &Level, s: &str) -> Vec<ConvEntry> {
    let mut out: Vec<ConvEntry> = Vec::new();
    for &ri in lvl.rules.list(s) {
        let r = &lvl.rules.features[ri];
        if r.subject != s {
            continue;
        }
        let o = r.object.as_str();
        let e = |t: Target| ConvEntry {
            target: t,
            conds: r.conds.clone(),
            cid: r.cid,
            verb: r.verb.clone(),
        };
        if r.verb == "is" || r.verb == "become" {
            if o == "word" || (o == s && r.verb != "become") {
                continue;
            }
            if o == "revert" {
                out.insert(0, e(Target::Revert));
            } else if o == "text" {
                out.push(e(Target::Name(format!("text_{s}"))));
            } else if o == format!("not {s}") {
                out.push(e(Target::Paradox));
            } else if o == "all" {
                out.push(e(Target::All));
            } else if lvl.known.contains(o) && !is_group(o) {
                out.push(e(Target::Name(o.to_string())));
            }
        } else if r.verb == "write" && !o.starts_with("not ") {
            out.push(e(Target::Name(format!("text_{o}"))));
        }
    }
    out
}

/// Target validity (§4 step 1).
fn valid(lvl: &Level, t: &Target) -> bool {
    match t {
        Target::Name(n) => n == "level" || n == "empty" || lvl.defs.exists(n),
        _ => true,
    }
}

/// Move-phase conversion step (`turn.md` §4.1 step 10): queues conversions.
pub(crate) fn collect(lvl: &mut Level) {
    let mut done: Vec<String> = Vec::new();
    let feats: Vec<(String, String, String)> = lvl
        .rules
        .features
        .iter()
        .filter(|r| matches!(r.verb.as_str(), "is" | "become" | "write"))
        .map(|r| (r.subject.clone(), r.verb.clone(), r.object.clone()))
        .collect();
    for (s, v, o) in feats {
        if done.contains(&s) || s.starts_with("not ") || is_group(&s) || s == "all" {
            continue;
        }
        if !opens(lvl, &s, &v, &o) {
            continue;
        }
        done.push(s.clone());
        let list = conv_list(lvl, &s);
        if list.is_empty() {
            continue;
        }
        if s == "empty" {
            empty_collect(lvl, &list);
        } else {
            unit_collect(lvl, &s, &list);
        }
    }
}

fn unit_collect(lvl: &mut Level, s: &str, list: &[ConvEntry]) {
    let has_all = list.iter().any(|e| e.target == Target::All);
    for u in lvl.units_named(s).to_vec() {
        if !lvl.alive(u) || lvl.unit(u).converted {
            continue;
        }
        let mut outs: Vec<Output> = Vec::new();
        for (a, e) in list.iter().enumerate() {
            if !valid(lvl, &e.target) || !lvl.holds(&e.conds, e.cid, Subj::Unit(u)) {
                continue;
            }
            let id = if a == 0 && !has_all {
                lvl.unit(u).id
            } else {
                lvl.new_id()
            };
            if e.target == Target::Revert {
                let un = lvl.unit(u);
                let cur = lvl.defs.kind(un.kind).name_id;
                if cur != un.original {
                    outs = vec![Output {
                        target: Target::Revert,
                        id,
                    }];
                    lvl.unit_mut(u).converted = true;
                    break;
                }
                continue;
            }
            outs.push(Output {
                target: e.target.clone(),
                id,
            });
            lvl.unit_mut(u).converted = true;
        }
        if !outs.is_empty() {
            let snap = Box::new(lvl.unit(u).clone());
            lvl.scratch
                .pending
                .push(Pending::Convert { h: u, outs, snap });
        }
    }
}

fn empty_collect(lvl: &mut Level, list: &[ConvEntry]) {
    let occupied: Vec<bool> = (0..lvl.w * lvl.h)
        .map(|i| !lvl.tiles[i as usize].is_empty())
        .collect();
    for x in 0..lvl.w {
        for y in 0..lvl.h {
            let t = lvl.tile_index(x, y);
            if occupied[t as usize] || lvl.scratch.vacated.contains(&t) || lvl.is_edge(x, y) {
                continue;
            }
            let mut outs = Vec::new();
            for e in list {
                if matches!(e.target, Target::Revert) || e.target == Target::Name("empty".into()) {
                    continue;
                }
                if !valid(lvl, &e.target) || !lvl.holds(&e.conds, e.cid, Subj::Empty(x, y)) {
                    continue;
                }
                outs.push(Output {
                    target: e.target.clone(),
                    id: 0,
                });
            }
            if !outs.is_empty() {
                lvl.scratch
                    .pending
                    .push(Pending::EmptyConvert { x, y, outs });
            }
        }
    }
}

/// Executes a queued unit conversion (§4.1).
pub(crate) fn execute(lvl: &mut Level, h: Handle, outs: &[Output], snap: &Unit) {
    let src = if lvl.alive(h) {
        lvl.unit(h).clone()
    } else {
        snap.clone()
    };
    let mut remove = false;
    let mut keep_by_all = false;
    let mut all_seen = false;
    let from = lvl.defs.kind(src.kind).name.clone();
    for o in outs {
        let tname = match &o.target {
            Target::Name(n) => n.clone(),
            Target::Paradox => {
                // `presentation.md` §9.3: conversion into the error object.
                let colour = if lvl.alive(h) {
                    lvl.unit_colour(h)
                } else {
                    lvl.defs.kind(src.kind).colour
                };
                lvl.burst(
                    crate::events::Particle::Unlock,
                    Some("paradox"),
                    (src.x, src.y),
                    20,
                    colour,
                    0,
                    None,
                );
                remove = true;
                continue;
            }
            Target::All => {
                all_seen = true;
                let made = create_all(
                    lvl,
                    Subj::Unit(h),
                    &from,
                    src.x,
                    src.y,
                    src.dir,
                    src.original,
                );
                let lk = lvl.defs.level_kind;
                if !(lvl.defs.kind(src.kind).is_text || src.kind == lk) || made == 0 {
                    keep_by_all = true;
                }
                continue;
            }
            Target::Revert => {
                let on = lvl.defs.name(src.original).to_string();
                if on == "empty" {
                    "empty".to_string()
                } else if on.is_empty() || lvl.defs.kind_by_name(&on) == Some(src.kind) {
                    break;
                } else {
                    on
                }
            }
        };
        if tname == "empty" {
            lvl.line(Line::Converted);
            let t = lvl.tile_index(src.x, src.y);
            lvl.scratch.vacated.insert(t);
            remove = true;
            continue;
        }
        let Some(k) = lvl.defs.kind_by_name(&tname) else {
            break;
        };
        lvl.line(Line::Converted);
        lvl.events.push(Event::Converted {
            from_id: src.id,
            from: from.clone(),
            to_id: o.id,
            to: tname.clone(),
            at: (src.x, src.y),
        });
        let ph = lvl.create(
            k,
            src.x,
            src.y,
            src.dir,
            0,
            src.icon.clone(),
            Some(o.id),
            src.id,
            CreateSource::Convert,
        );
        let u = lvl.unit_mut(ph);
        u.new = false;
        u.original = src.original;
        remove = true;
    }
    let _ = all_seen;
    if remove && !keep_by_all && lvl.alive(h) {
        lvl.remove_recorded(h, Cause::Converted);
    }
}

/// Executes a queued EMPTY conversion (§10).
pub(crate) fn execute_empty(lvl: &mut Level, x: i32, y: i32, outs: &[Output]) {
    for o in outs {
        match &o.target {
            Target::All => {
                let d = empty_facing(lvl, x, y);
                let en = lvl.defs.empty_name;
                create_all(lvl, Subj::Empty(x, y), "empty", x, y, d, en);
            }
            Target::Paradox | Target::Revert => {}
            Target::Name(n) => {
                let Some(k) = lvl.defs.kind_by_name(n) else {
                    continue;
                };
                let d = empty_facing(lvl, x, y);
                lvl.line(Line::Converted);
                let h = lvl.create(k, x, y, d, 0, None, None, -1, CreateSource::EmptyConvert);
                let to_id = lvl.unit(h).id;
                lvl.events.push(Event::Converted {
                    from_id: -1,
                    from: "empty".into(),
                    to_id,
                    to: n.clone(),
                    at: (x, y),
                });
                let en = lvl.defs.empty_name;
                lvl.unit_mut(h).original = en;
                let t = lvl.tile_index(x, y);
                lvl.scratch.vacated.insert(t);
            }
        }
    }
}

fn empty_facing(lvl: &Level, x: i32, y: i32) -> u8 {
    let d = lvl.empty_dir(x, y);
    if d > 3 { lvl.rand(0, 3) as u8 } else { d }
}

/// X IS ALL creation (§11). Returns the number of products.
fn create_all(
    lvl: &mut Level,
    subj: Subj,
    s: &str,
    x: i32,
    y: i32,
    dir: u8,
    original: crate::defs::NameId,
) -> usize {
    let names: Vec<String> = lvl
        .known
        .iter()
        .filter(|n| !in_full(n) && n.as_str() != s)
        .cloned()
        .collect();
    let mut made = 0;
    for b in names {
        if lvl.has(s, "is", &format!("not {b}"), subj) {
            continue;
        }
        let src = match subj {
            Subj::Unit(h) => Some(h),
            _ => None,
        };
        if lvl
            .at(x, y)
            .iter()
            .any(|&o| Some(o) != src && lvl.uname(o) == b)
        {
            continue;
        }
        let Some(k) = lvl.defs.kind_by_name(&b) else {
            continue;
        };
        lvl.line(Line::Converted);
        let h = lvl.create(k, x, y, dir, 0, None, None, -1, CreateSource::Create);
        lvl.unit_mut(h).original = original;
        made += 1;
    }
    made
}

/// HAS products when a unit or empty tile is destroyed (§7).
pub(crate) fn run_has(lvl: &mut Level, subj: Subj) {
    let (n, x, y, dir, icon) = match subj {
        Subj::Unit(h) => {
            let u = lvl.unit(h);
            (lvl.rname(h).to_string(), u.x, u.y, u.dir, u.icon.clone())
        }
        Subj::Empty(x, y) => {
            let d = lvl.empty_dir(x, y);
            let d = if d > 3 { lvl.rand(0, 3) as u8 } else { d };
            ("empty".to_string(), x, y, d, None)
        }
        Subj::Level => return,
    };
    let rules: Vec<(String, Vec<Cond>, u32)> = lvl
        .rules
        .list(&n)
        .iter()
        .map(|&ri| &lvl.rules.features[ri])
        .filter(|r| r.subject == n && r.verb == "has" && !r.object.starts_with("not "))
        .map(|r| (r.object.clone(), r.conds.clone(), r.cid))
        .collect();
    for (o, c, cid) in rules {
        if !lvl.holds(&c, cid, subj) {
            continue;
        }
        let target = if o == "text" {
            format!("text_{n}")
        } else if lvl.known.contains(&o) && o != "empty" && o != "all" && !is_group(&o) {
            o.clone()
        } else {
            continue;
        };
        if let Some(k) = lvl.defs.kind_by_name(&target) {
            lvl.create(
                k,
                x,
                y,
                dir,
                0,
                icon.clone(),
                None,
                -1,
                CreateSource::Create,
            );
        }
    }
}

/// MAKE (§8), block step 12.
pub(crate) fn make(lvl: &mut Level) {
    let groups = lvl.verb_groups("make");
    for (x_name, rules) in groups {
        for (y_name, c, cid) in rules {
            if !(lvl.known.contains(&y_name) || y_name == "text" || y_name == "all") {
                continue;
            }
            let product: Option<KindId> = if y_name == "text" {
                if x_name == "text" || x_name == "all" {
                    None
                } else {
                    lvl.defs.kind_by_name(&format!("text_{x_name}"))
                }
            } else if y_name == "all" || y_name == "empty" || is_group(&y_name) {
                None
            } else {
                lvl.defs.kind_by_name(&y_name)
            };
            if x_name == "empty" {
                for x in 0..lvl.w {
                    for y in 0..lvl.h {
                        if !lvl.tile_empty(x, y) || lvl.is_edge(x, y) {
                            continue;
                        }
                        if !lvl.holds(&c, cid, Subj::Empty(x, y)) {
                            continue;
                        }
                        let d = empty_facing(lvl, x, y);
                        if let Some(k) = product {
                            lvl.create(k, x, y, d, 0, None, None, -1, CreateSource::Create);
                        }
                    }
                }
                continue;
            }
            for u in lvl.units_named(&x_name).to_vec() {
                if !lvl.alive(u) || !lvl.holds(&c, cid, Subj::Unit(u)) {
                    continue;
                }
                let (x, y, d, icon) = {
                    let w = lvl.unit(u);
                    (w.x, w.y, w.dir, w.icon.clone())
                };
                let present = lvl.at(x, y).iter().any(|&o| {
                    let w = lvl.unit(o);
                    !w.converted
                        && if y_name == "text" {
                            lvl.is_text(o)
                        } else {
                            lvl.uname(o) == y_name
                        }
                });
                if present {
                    continue;
                }
                if let Some(k) = product {
                    let d = if d > 3 { lvl.rand(0, 3) as u8 } else { d };
                    lvl.create(k, x, y, d, 0, icon, None, -1, CreateSource::Create);
                }
            }
        }
    }
}

/// Level conversion collection (§9.1), end-of-turn step 4.
pub(crate) fn level_collect(lvl: &mut Level) {
    let has_rule = lvl.rules.list("level").iter().any(|&ri| {
        let r = &lvl.rules.features[ri];
        r.subject == "level"
            && matches!(r.verb.as_str(), "is" | "become" | "write")
            && opens(lvl, "level", &r.verb, &r.object)
    });
    if !has_rule {
        return;
    }
    let mut list = conv_list(lvl, "level");
    for r in lvl.rules.list("level") {
        let f = &lvl.rules.features[*r];
        if f.subject == "level" && f.verb == "become" && f.object == "level" {
            list.push(ConvEntry {
                target: Target::Revert,
                conds: f.conds.clone(),
                cid: f.cid,
                verb: "become".into(),
            });
        }
    }
    lvl.scratch.level_conversions.extend(list);
}

/// Level conversion execution (§9.2), end-of-turn step 6.
pub(crate) fn level_apply(lvl: &mut Level) {
    let list = std::mem::take(&mut lvl.scratch.level_conversions);
    if list.is_empty() || lvl.rules.features.is_empty() || lvl.won || lvl.destroy.get().is_some() {
        return;
    }
    let mut targets: Vec<String> = Vec::new();
    for e in &list {
        let target = match (&e.target, e.verb.as_str()) {
            (Target::Name(n), "become") if n == "level" => Target::Revert,
            (t, _) => t.clone(),
        };
        let ok_cond = lvl.holds(&e.conds, e.cid, Subj::Level);
        match target {
            Target::Paradox => {
                if ok_cond {
                    lvl.request_destroy(DestroyStyle::Plain);
                    return;
                }
            }
            Target::All => {}
            Target::Revert => {
                if ok_cond {
                    targets = vec!["revert".into()];
                    break;
                }
            }
            Target::Name(n) => {
                if n != "empty" && lvl.defs.exists(&n) && ok_cond {
                    targets.push(n);
                }
            }
        }
    }
    if targets.is_empty() {
        return;
    }
    if targets.len() >= 50 {
        lvl.request_destroy(DestroyStyle::TooComplex);
        return;
    }
    lvl.transition = Some(Transition::LevelConversion(targets));
}
