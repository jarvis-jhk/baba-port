//! Block effects (`docs/spec/properties.md` §4, order `turn.md` §7).
//!
//! [`run`] calls one function per step in the fixed order; each step owns
//! its victim list and removal mode. Steps are independent functions so
//! families can be refined separately.

use std::collections::BTreeSet;

use crate::conditions::Subj;
use crate::events::{Cause, DestroyStyle, Particle};
use crate::level::Level;
use crate::movement::Thing;
use crate::rules::Cond;
use crate::undo::Line;
use crate::unit::{Handle, dir_delta};

/// Victims since the last removal point.
#[derive(Default)]
pub(crate) struct Victims {
    list: Vec<Handle>,
    set: BTreeSet<Handle>,
}

impl Victims {
    /// Adds a victim. Duplicates are kept: in simultaneous removal HAS
    /// runs once per listing (`turn.md` §2.5).
    pub fn add(&mut self, h: Handle) {
        self.set.insert(h);
        self.list.push(h);
    }
    pub fn has(&self, h: Handle) -> bool {
        self.set.contains(&h)
    }
}

/// Removes victims (`turn.md` §2.5).
pub(crate) fn remove(lvl: &mut Level, v: Victims, simultaneous: bool, cause: Cause) {
    if simultaneous {
        for &h in &v.list {
            if lvl.alive(h) && !lvl.scratch.deleted.contains(&h) {
                let u = lvl.unit(h);
                if lvl.inside(u.x, u.y) {
                    crate::convert::run_has(lvl, Subj::Unit(h));
                }
            }
        }
        for h in v.list {
            lvl.destroy(h, true, cause);
        }
    } else {
        for h in v.list {
            lvl.destroy(h, false, cause);
        }
    }
}

/// Rules `(subject, conds, cid)` with verb IS and this object, subject not
/// negated.
pub(crate) fn is_rules(lvl: &Level, p: &str) -> Vec<(String, Vec<Cond>, u32)> {
    lvl.rules
        .list(p)
        .iter()
        .map(|&ri| &lvl.rules.features[ri])
        .filter(|r| r.verb == "is" && r.object == p && !r.subject.starts_with("not "))
        .map(|r| (r.subject.clone(), r.conds.clone(), r.cid))
        .collect()
}

/// Unit `h` matches a rule subject with conditions.
pub(crate) fn matches(lvl: &Level, h: Handle, subject: &str, conds: &[Cond], cid: u32) -> bool {
    let ok = lvl.rname(h) == subject || lvl.uname(h) == subject;
    ok && lvl.alive(h) && lvl.holds(conds, cid, Subj::Unit(h))
}

/// Player units: YOU, then YOU2, then 3D (duplicates possible).
pub(crate) fn players(lvl: &Level) -> Vec<Handle> {
    let mut v = lvl.units_with("you");
    v.extend(lvl.units_with("you2"));
    v.extend(lvl.units_with("3d"));
    v
}

/// The block (`turn.md` §7).
pub(crate) fn run(lvl: &mut Level) {
    done(lvl);
    more(lvl);
    play(lvl);
    crate::movement::extra::hold(lvl, true);
    sink(lvl);
    boom(lvl);
    weak(lvl);
    melt(lvl);
    defeat(lvl);
    open_shut(lvl);
    eat(lvl);
    crate::convert::make(lvl);
    crate::movement::extra::hold(lvl, false);
    player_checks(lvl);
    bounds(lvl);
    crash(lvl);
}

fn done(lvl: &mut Level) {
    if lvl.ending_running {
        return;
    }
    for h in lvl.units_with("done") {
        if !lvl.alive(h) || lvl.unit(h).float == 2 {
            continue;
        }
        leave(lvl, h);
    }
}

/// A unit leaves through DONE (no HAS, ignores SAFE).
pub(crate) fn leave(lvl: &mut Level, h: Handle) {
    let snap = lvl.unit(h).clone();
    lvl.events.push(crate::events::Event::Removed {
        id: snap.id,
        name: lvl.uname(h).to_string(),
        at: (snap.x, snap.y),
        cause: Cause::Done,
    });
    crate::vision::on_done(lvl, h);
    lvl.turn_sound(crate::events::sounds::DONE, false);
    lvl.line(Line::Done(Box::new(snap)));
    lvl.turn_changed = true;
    lvl.unit_mut(h).float = 2;
    lvl.unregister(h);
}

fn more(lvl: &mut Level) {
    let list = lvl.units_with("more");
    for h in list {
        if !lvl.alive(h) {
            continue;
        }
        for nd in 0..4u8 {
            if !lvl.alive(h) {
                break;
            }
            let (x, y, k, dir, fl, icon) = {
                let u = lvl.unit(h);
                (u.x, u.y, u.kind, u.dir, u.float, u.icon.clone())
            };
            let (dx, dy) = dir_delta(nd);
            let (nx, ny) = (x + dx, y + dy);
            if lvl.is_edge(nx, ny) {
                continue;
            }
            let here = lvl.at(nx, ny).to_vec();
            let blocked = if here.is_empty() {
                lvl.empty_prop("stop", nx, ny)
                    || lvl.empty_prop("push", nx, ny)
                    || lvl.empty_prop("pull", nx, ny)
            } else {
                here.iter().any(|&o| {
                    lvl.unit(o).kind == k
                        || lvl.prop(o, "stop")
                        || lvl.prop(o, "push")
                        || lvl.prop(o, "pull")
                })
            };
            if blocked {
                continue;
            }
            lvl.create(
                k,
                nx,
                ny,
                dir,
                fl,
                icon,
                None,
                -1,
                crate::undo::CreateSource::Create,
            );
        }
    }
}

/// X PLAY note (`audio.md` §4.3): one sound per (subject, note) with
/// sounding units, which suppresses the move sound. Presentation only, but
/// the conditions are tested in the original's order (OFTEN/SELDOM).
fn play(lvl: &mut Level) {
    for (subject, rules) in lvl.verb_groups("play") {
        if subject == "empty" {
            continue;
        }
        let members = lvl.units_named(&subject).to_vec();
        if members.is_empty() {
            continue;
        }
        for (note, c, cid) in rules {
            let units: Vec<i32> = members
                .iter()
                .copied()
                .filter(|&h| lvl.exists(h) && lvl.holds(&c, cid, Subj::Unit(h)) && lvl.alive(h))
                .map(|h| lvl.unit(h).id)
                .collect();
            if units.is_empty() {
                continue;
            }
            lvl.turn_sound(crate::events::sounds::SILENT, false);
            for &id in &units {
                if let Some(h) = lvl.find_id(id) {
                    let at = (lvl.unit(h).x, lvl.unit(h).y);
                    lvl.burst(Particle::Music, None, at, 1, (0, 3), 0, Some(id));
                }
            }
            lvl.events.push(crate::events::Event::Play {
                name: subject.clone(),
                note,
                units,
            });
        }
    }
}

fn sink(lvl: &mut Level) {
    let mut v = Victims::default();
    for u in lvl.units_with("sink") {
        if !lvl.alive(u) || v.has(u) {
            continue;
        }
        let (x, y, fl) = {
            let w = lvl.unit(u);
            (w.x, w.y, w.float)
        };
        let usafe = lvl.safe(Thing::Unit(u));
        let mut sunk = false;
        for b in lvl.at(x, y).to_vec() {
            if b == u || v.has(b) || lvl.unit(b).float != fl {
                continue;
            }
            let bsafe = lvl.safe(Thing::Unit(b));
            if usafe && bsafe {
                continue;
            }
            if !bsafe {
                v.add(b);
            }
            if !usafe {
                sunk = true;
            }
        }
        if sunk {
            v.add(u);
        }
    }
    remove(lvl, v, false, Cause::Sink);
}

fn boom(lvl: &mut Level) {
    let mut v = Victims::default();
    let list = lvl.units_with("boom");
    let mut done = BTreeSet::new();
    for u in list {
        if !done.insert(u) || !lvl.exists(u) {
            continue;
        }
        let (x, y, fl) = {
            let w = lvl.unit(u);
            (w.x, w.y, w.float)
        };
        let n = lvl.count(lvl.rname(u), "is", "boom", Subj::Unit(u)).max(1) as i32;
        let rho = (n - 1).min(lvl.w.max(lvl.h));
        for f in lvl.order.clone() {
            if f == u || v.has(f) {
                continue;
            }
            let w = lvl.unit(f);
            if w.float != fl || !lvl.inside(w.x, w.y) {
                continue;
            }
            if (w.x - x).abs() <= rho && (w.y - y).abs() <= rho && !lvl.safe(Thing::Unit(f)) {
                v.add(f);
            }
        }
        if !lvl.safe(Thing::Unit(u)) {
            v.add(u);
        }
        // `presentation.md` §9.3: one smoke burst per exploding unit.
        let colour = lvl.unit_colour(u);
        lvl.burst(Particle::Smoke, Some("boom"), (x, y), 15, colour, 6, None);
    }
    remove(lvl, v, false, Cause::Boom);
}

fn weak(lvl: &mut Level) {
    let mut v = Victims::default();
    for u in lvl.units_with("weak") {
        if !lvl.alive(u) || lvl.safe(Thing::Unit(u)) || lvl.unit(u).new {
            continue;
        }
        let (x, y, fl) = {
            let w = lvl.unit(u);
            (w.x, w.y, w.float)
        };
        if lvl
            .at(x, y)
            .iter()
            .any(|&o| o != u && lvl.unit(o).float == fl)
        {
            v.add(u);
        }
    }
    remove(lvl, v, true, Cause::Weak);
}

fn melt(lvl: &mut Level) {
    let mut v = Victims::default();
    let hot = is_rules(lvl, "hot");
    if hot.is_empty() {
        return;
    }
    for u in lvl.units_with("melt") {
        if !lvl.alive(u) || lvl.safe(Thing::Unit(u)) {
            continue;
        }
        let (x, y, fl) = {
            let w = lvl.unit(u);
            (w.x, w.y, w.float)
        };
        for (s, c, cid) in &hot {
            if s == "empty" {
                continue;
            }
            let ms: Vec<Handle> = lvl
                .at(x, y)
                .iter()
                .copied()
                .filter(|&o| matches(lvl, o, s, c, *cid))
                .collect();
            if ms.iter().any(|&o| lvl.unit(o).float == fl) {
                v.add(u);
            }
        }
    }
    remove(lvl, v, true, Cause::Melt);
}

fn defeat(lvl: &mut Level) {
    let mut v = Victims::default();
    let rules = is_rules(lvl, "defeat");
    if rules.is_empty() {
        return;
    }
    for u in players(lvl) {
        if !lvl.alive(u) || v.has(u) {
            continue;
        }
        let (x, y, fl) = {
            let w = lvl.unit(u);
            (w.x, w.y, w.float)
        };
        for (s, c, cid) in &rules {
            if s == "empty" {
                continue;
            }
            let ms: Vec<Handle> = lvl
                .at(x, y)
                .iter()
                .copied()
                .filter(|&o| matches(lvl, o, s, c, *cid))
                .collect();
            if ms.is_empty() || lvl.safe(Thing::Unit(u)) {
                continue;
            }
            let hit = ms.contains(&u)
                || ms.iter().any(|&d| {
                    d != u
                        && lvl.unit(d).float == fl
                        && (!lvl.prop(d, "weak") || lvl.safe(Thing::Unit(d)))
                });
            if hit {
                v.add(u);
                break;
            }
        }
    }
    remove(lvl, v, false, Cause::Defeat);
}

fn open_shut(lvl: &mut Level) {
    let mut v = Victims::default();
    let open = is_rules(lvl, "open");
    if open.is_empty() {
        return;
    }
    for u in lvl.units_with("shut") {
        if !lvl.alive(u) {
            continue;
        }
        let (x, y, fl) = {
            let w = lvl.unit(u);
            (w.x, w.y, w.float)
        };
        for (s, c, cid) in &open {
            if s == "empty" {
                continue;
            }
            let k = lvl
                .at(x, y)
                .iter()
                .copied()
                .find(|&o| matches(lvl, o, s, c, *cid) && lvl.unit(o).float == fl);
            if let Some(k) = k {
                if !lvl.safe(Thing::Unit(u)) {
                    v.add(u);
                }
                if k != u && !lvl.safe(Thing::Unit(k)) {
                    v.add(k);
                }
                break;
            }
        }
    }
    remove(lvl, v, false, Cause::Unlock);
}

fn eat(lvl: &mut Level) {
    let mut v = Victims::default();
    let groups = lvl.verb_groups("eat");
    for (x_name, rules) in groups {
        if x_name == "empty" || x_name == "level" {
            continue;
        }
        for (y_name, c, cid) in rules {
            if y_name == "level" || y_name == "empty" {
                continue;
            }
            for u in lvl.units_named(&x_name).to_vec() {
                if !lvl.alive(u) || v.has(u) || !lvl.holds(&c, cid, Subj::Unit(u)) {
                    continue;
                }
                let (x, y, fl) = {
                    let w = lvl.unit(u);
                    (w.x, w.y, w.float)
                };
                for b in lvl.at(x, y).to_vec() {
                    if b != u
                        && lvl.rname(b) == y_name
                        && !lvl.safe(Thing::Unit(b))
                        && lvl.unit(b).float == fl
                    {
                        v.add(b);
                    }
                }
            }
        }
    }
    remove(lvl, v, true, Cause::Eaten);
}

fn player_checks(lvl: &mut Level) {
    let mut v = Victims::default();
    let bonus = is_rules(lvl, "bonus");
    let end = is_rules(lvl, "end");
    let win = is_rules(lvl, "win");
    if bonus.is_empty() && end.is_empty() && win.is_empty() {
        return;
    }
    for u in players(lvl) {
        if !lvl.alive(u) {
            continue;
        }
        let (x, y, fl) = {
            let w = lvl.unit(u);
            (w.x, w.y, w.float)
        };
        for (s, c, cid) in &bonus {
            if s == "empty" {
                continue;
            }
            for d in lvl.at(x, y).to_vec() {
                if lvl.unit(d).float == fl && matches(lvl, d, s, c, *cid) {
                    collect_bonus(lvl);
                    if !lvl.safe(Thing::Unit(d)) {
                        v.add(d);
                    }
                }
            }
        }
        for (s, c, cid) in &end {
            if s == "empty" {
                continue;
            }
            if lvl
                .at(x, y)
                .iter()
                .any(|&d| lvl.unit(d).float == fl && matches(lvl, d, s, c, *cid))
            {
                lvl.ended = true;
                if !lvl.defs.base_world || lvl.ctx.in_editor {
                    lvl.won = true;
                    lvl.burst(Particle::Win, Some("win"), (x, y), 10, (2, 4), 0, None);
                } else {
                    lvl.burst(Particle::Unlock, None, (x, y), 10, (1, 4), 0, None);
                }
                break;
            }
        }
        for (s, c, cid) in &win {
            if s == "empty" {
                continue;
            }
            let found = lvl.at(x, y).iter().any(|&d| {
                lvl.unit(d).float == fl
                    && matches(lvl, d, s, c, *cid)
                    && !lvl.has(s, "is", "done", Subj::Unit(d))
                    && !lvl.has(s, "is", "end", Subj::Unit(d))
            });
            if found {
                lvl.won = true;
                lvl.burst(Particle::Win, Some("win"), (x, y), 10, (2, 4), 0, None);
                break;
            }
        }
    }
    remove(lvl, v, false, Cause::Bonus);
}

/// Collects the level bonus (undo line).
pub(crate) fn collect_bonus(lvl: &mut Level) {
    if !lvl.bonus {
        lvl.bonus = true;
        lvl.line(Line::Bonus);
        lvl.events.push(crate::events::Event::BonusCollected);
        lvl.events
            .push(crate::events::Event::Sound(crate::events::sounds::BONUS));
    }
}

fn bounds(lvl: &mut Level) {
    for h in lvl.order.clone() {
        let u = lvl.unit(h);
        if !lvl.inside(u.x, u.y) {
            lvl.destroy(h, false, Cause::Destroyed);
        }
    }
}

fn crash(lvl: &mut Level) {
    if lvl.any_is("crash") && !lvl.units_with("crash").is_empty() {
        lvl.request_destroy(DestroyStyle::Infinity);
    }
}
