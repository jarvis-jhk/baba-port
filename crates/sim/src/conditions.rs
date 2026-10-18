//! Condition evaluation (`docs/spec/conditions.md`).

use std::collections::{BTreeMap, BTreeSet};

use crate::defs::in_short;
use crate::events::DestroyStyle;
use crate::level::Level;
use crate::rules::{Cond, RuleSet};
use crate::unit::{Handle, dir_delta};

/// The subject of a rule query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subj {
    /// A unit.
    Unit(Handle),
    /// An empty tile.
    Empty(i32, i32),
    /// The level pseudo-unit.
    Level,
}

/// Per-turn caches (`conditions.md` §7).
#[derive(Clone, Debug, Default)]
pub struct Caches {
    /// OFTEN/SELDOM rolls by (conds id, entry index, subject key).
    pub dice: BTreeMap<(u32, usize, i64), i32>,
    /// POWERED results by subtype ("", "2", "3").
    pub powered: BTreeMap<String, bool>,
}

/// Recursion guard set (`conditions.md` §2.8).
#[derive(Clone, Debug, Default)]
pub(crate) struct Guard {
    ids: Vec<u32>,
    seeing: Vec<u32>,
}

impl Guard {
    fn with(&self, cid: u32) -> Guard {
        let mut g = self.clone();
        if !g.ids.contains(&cid) {
            g.ids.push(cid);
        }
        if !g.seeing.contains(&cid) {
            g.seeing.push(cid);
        }
        g
    }
    fn has(&self, cid: u32) -> bool {
        self.ids.contains(&cid)
    }
    fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

/// Evaluation options.
#[derive(Clone, Copy, Default)]
pub(crate) struct Opts<'a> {
    pub depth: u32,
    pub ignore_broken: bool,
    /// FACING/SEEING auto-fail (empty-direction computation).
    pub autofail: bool,
    /// Override member list for group params (§2.7).
    pub members: Option<&'a [String]>,
}

/// Evaluate with the level's current rules (recompute step 2 helper).
pub(crate) fn holds_with(lvl: &Level, _rs: &RuleSet, conds: &[Cond], cid: u32, subj: Subj) -> bool {
    eval(lvl, conds, cid, subj, &Guard::default(), Opts::default())
}

struct Info {
    name: String,
    x: i32,
    y: i32,
    dir: u8,
    unit: Option<Handle>,
}

fn info(lvl: &Level, subj: Subj, guard: &Guard, cid: u32) -> Info {
    match subj {
        Subj::Unit(h) => {
            let u = lvl.unit(h);
            Info {
                name: lvl.rname(h).to_string(),
                x: u.x,
                y: u.y,
                dir: u.dir,
                unit: Some(h),
            }
        }
        Subj::Empty(x, y) => {
            let _ = (guard, cid);
            Info {
                name: "empty".into(),
                x,
                y,
                dir: 4,
                unit: None,
            }
        }
        Subj::Level => Info {
            name: "level".into(),
            x: 0,
            y: 0,
            dir: lvl.ctx.surroundings.as_ref().map_or(4, |s| s.dir),
            unit: None,
        },
    }
}

fn subj_key(lvl: &Level, subj: Subj) -> i64 {
    match subj {
        Subj::Unit(h) => h as i64,
        Subj::Empty(x, y) => -(lvl.tile_index(x, y) as i64) - 2,
        Subj::Level => -1,
    }
}

/// `holds(conds, subject)` (§2).
pub(crate) fn eval(
    lvl: &Level,
    conds: &[Cond],
    cid: u32,
    subj: Subj,
    caller: &Guard,
    o: Opts<'_>,
) -> bool {
    let depth = o.depth + 1;
    if depth > 80 {
        lvl.request_destroy(DestroyStyle::Infinity);
        return false;
    }
    if lvl.rules.features.is_empty() {
        return false;
    }
    // BROKEN gate.
    if !o.ignore_broken {
        let broken = match subj {
            Subj::Unit(h) => lvl.unit(h).broken,
            Subj::Empty(_, _) | Subj::Level => {
                if lvl.rules.index.contains_key("broken") && !caller.has(cid) {
                    let g = caller.with(cid);
                    let n = if matches!(subj, Subj::Level) {
                        "level"
                    } else {
                        "empty"
                    };
                    has_g(
                        lvl,
                        n,
                        "is",
                        "broken",
                        subj,
                        &g,
                        Opts {
                            depth,
                            ignore_broken: true,
                            ..Default::default()
                        },
                    )
                } else {
                    false
                }
            }
        };
        if broken {
            return false;
        }
    }
    if conds.is_empty() {
        return true;
    }
    let me = info(lvl, subj, caller, cid);
    let mut g = caller.with(cid);
    let mut result = true;
    let mut valid = false;
    let mut in_or = false;
    let mut or_any = false;
    for (i, c) in conds.iter().enumerate() {
        let mut name = c.name.as_str();
        if let Some(r) = name.strip_prefix('(') {
            name = r;
            in_or = true;
            or_any = false;
        }
        if let Some(r) = name.strip_suffix(')') {
            name = r;
        }
        let (neg, mut base) = match name.strip_prefix("not ") {
            Some(r) => (true, r),
            None => (false, name),
        };
        if o.autofail && (name == "facing" || name == "seeing") {
            result = false;
            valid = true;
            break;
        }
        // §2.5 param order.
        let mut params: Vec<String> = Vec::new();
        let mut handle_group = false;
        for p in &c.params {
            if p.starts_with("not ") {
                params.push(p.clone());
            } else {
                params.insert(0, p.clone());
            }
            if p.starts_with("group") || p.starts_with("not group") {
                handle_group = true;
            }
        }
        if handle_group {
            params = group_params(lvl, params, &g, Opts { depth, ..o });
        }
        let mut subtype = "";
        if base.starts_with("powered") && base.len() > 7 {
            subtype = &base[7..];
            base = "powered";
        }
        let Some(kind) = CondKind::parse(base) else {
            result = false;
            break;
        };
        valid = true;
        let ctx = Ctx {
            lvl,
            me: &me,
            subj,
            cid,
            index: i,
            caller,
            depth,
            neg,
        };
        let (mut r, reset) = ctx.run(kind, &params, subtype, &mut g, o);
        if neg {
            r = !r;
        }
        if r && reset {
            g = caller.with(cid);
        }
        if !r {
            if !in_or {
                result = false;
                break;
            }
        } else if in_or {
            or_any = true;
        }
        if c.name.ends_with(')') {
            in_or = false;
            if !or_any {
                result = false;
                break;
            }
            result = true;
        }
    }
    if !valid {
        return true;
    }
    result
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CondKind {
    Never,
    On,
    Near,
    NextTo,
    Facing,
    FacedBy,
    Seeing,
    Without,
    Above,
    Below,
    BesideLeft,
    BesideRight,
    Feeling,
    Lonely,
    Powered,
    Idle,
    Often,
    Seldom,
}

impl CondKind {
    fn parse(s: &str) -> Option<CondKind> {
        Some(match s {
            "never" => CondKind::Never,
            "on" => CondKind::On,
            "near" => CondKind::Near,
            "nextto" => CondKind::NextTo,
            "facing" => CondKind::Facing,
            "facedby" => CondKind::FacedBy,
            "seeing" => CondKind::Seeing,
            "without" => CondKind::Without,
            "above" => CondKind::Above,
            "below" => CondKind::Below,
            "besideleft" => CondKind::BesideLeft,
            "besideright" => CondKind::BesideRight,
            "feeling" => CondKind::Feeling,
            "lonely" => CondKind::Lonely,
            "powered" => CondKind::Powered,
            "idle" => CondKind::Idle,
            "often" => CondKind::Often,
            "seldom" => CondKind::Seldom,
            _ => return None,
        })
    }
}

/// §2.6 group parameter transform (literal procedure).
fn group_params(lvl: &Level, mut p: Vec<String>, g: &Guard, o: Opts<'_>) -> Vec<String> {
    let n = p.len();
    let mut remove: Vec<usize> = Vec::new();
    let mut shift: isize = 0;
    for a in 0..n {
        let b = p[a].clone();
        let mut negated = false;
        let mut mem: Option<Vec<String>> = o.members.map(|m| m.to_vec());
        if b.starts_with("group") {
            if mem.is_none() {
                mem = Some(members(lvl, &b, g, o));
            }
            remove.push(a);
        } else if let Some(gn) = b.strip_prefix("not ").filter(|x| x.starts_with("group")) {
            negated = true;
            mem = Some(match mem {
                None => non_members(lvl, gn, g, o),
                Some(m) => lvl
                    .known
                    .iter()
                    .filter(|k| !in_short(k) && !m.contains(k))
                    .cloned()
                    .collect(),
            });
            remove.push(a);
        }
        if let Some(m) = &mem {
            for x in m {
                if negated {
                    p.push(x.clone());
                } else {
                    p.insert(0, x.clone());
                    shift -= 1;
                }
            }
        }
        if mem.as_ref().is_none_or(|m| m.is_empty()) {
            p.push("_NONE_".into());
            break;
        }
    }
    for r in remove {
        let idx = r as isize - shift;
        if idx >= 0 && (idx as usize) < p.len() {
            p.remove(idx as usize);
        }
        shift += 1;
    }
    p
}

/// members(G) (§2.6).
pub(crate) fn members(lvl: &Level, group: &str, g: &Guard, o: Opts<'_>) -> Vec<String> {
    let mut out = Vec::new();
    let mut recursive = Vec::new();
    for m in lvl.rules.members.iter().filter(|m| m.group == group) {
        if m.conds.is_empty() {
            out.push(m.name.clone());
        } else if !m.recursive {
            let units = lvl.units_named(&m.name);
            if units.iter().any(|&h| {
                eval(
                    lvl,
                    &m.conds,
                    m.cid,
                    Subj::Unit(h),
                    g,
                    Opts {
                        depth: o.depth + 1,
                        members: None,
                        ..o
                    },
                )
            }) {
                out.push(m.name.clone());
            }
        } else {
            recursive.push(m);
        }
    }
    if !recursive.is_empty() {
        let mut cand = out.clone();
        let mut ok = false;
        for _ in 0..10 {
            let mut next = out.clone();
            for m in &recursive {
                let units = lvl.units_named(&m.name);
                let sat = units.iter().any(|&h| {
                    eval(
                        lvl,
                        &m.conds,
                        m.cid,
                        Subj::Unit(h),
                        g,
                        Opts {
                            depth: o.depth + 1,
                            members: Some(&cand),
                            ..o
                        },
                    )
                });
                if sat {
                    next.push(m.name.clone());
                }
            }
            if next == cand {
                ok = true;
                break;
            }
            cand = next;
        }
        if !ok {
            lvl.request_destroy(DestroyStyle::Infinity);
            return Vec::new();
        }
        out.extend(cand);
    }
    out
}

fn non_members(lvl: &Level, group: &str, g: &Guard, o: Opts<'_>) -> Vec<String> {
    let m = members(lvl, group, g, o);
    lvl.known
        .iter()
        .filter(|k| !in_short(k) && !m.contains(k))
        .cloned()
        .collect()
}

struct Ctx<'a> {
    lvl: &'a Level,
    me: &'a Info,
    subj: Subj,
    cid: u32,
    index: usize,
    caller: &'a Guard,
    depth: u32,
    neg: bool,
}

/// Distinct-match bookkeeping (§3).
#[derive(Default)]
struct Found {
    units: BTreeSet<Handle>,
    cells: BTreeSet<i32>,
    level: bool,
    count: usize,
}

fn split(p: &str) -> (bool, &str) {
    match p.strip_prefix("not ") {
        Some(r) => (true, r),
        None => (false, p),
    }
}

impl Ctx<'_> {
    fn run(
        &self,
        k: CondKind,
        params: &[String],
        subtype: &str,
        g: &mut Guard,
        o: Opts<'_>,
    ) -> (bool, bool) {
        let lvl = self.lvl;
        match k {
            CondKind::Never => (false, false),
            CondKind::Idle => (lvl.last_key == 4, false),
            CondKind::Often | CondKind::Seldom => {
                let key = (self.cid, self.index, subj_key(lvl, self.subj));
                let mut c = lvl.caches.borrow_mut();
                let v = *c.dice.entry(key).or_insert_with(|| {
                    if k == CondKind::Often {
                        lvl.rand(1, 4)
                    } else {
                        lvl.rand(1, 6)
                    }
                });
                (if k == CondKind::Often { v > 1 } else { v == 1 }, false)
            }
            CondKind::Lonely => (
                match self.subj {
                    Subj::Unit(h) => !lvl.at(self.me.x, self.me.y).iter().any(|&o| o != h),
                    Subj::Empty(x, y) => lvl.tile_empty(x, y),
                    Subj::Level => false,
                },
                false,
            ),
            CondKind::Powered => self.powered(subtype, o),
            CondKind::Feeling => self.feeling(params, g, o),
            CondKind::On => (self.on(params), false),
            CondKind::Near => (self.area(params, true), false),
            CondKind::NextTo => (self.area(params, false), false),
            CondKind::Facing => (self.facing(params, g), false),
            CondKind::FacedBy => (self.facedby(params), false),
            CondKind::Seeing => self.seeing(params, g),
            CondKind::Without => (self.without(params), false),
            CondKind::Above | CondKind::Below | CondKind::BesideLeft | CondKind::BesideRight => {
                (self.line(k, params), false)
            }
        }
    }

    /// Unit name match: X matches rule name X; `not X` a non-text unit
    /// named otherwise.
    fn name_ok(&self, h: Handle, pnot: bool, pname: &str) -> bool {
        let n = self.lvl.rname(h);
        if pnot {
            n != pname && n != "text"
        } else {
            n == pname
        }
    }

    /// Try to match a name param against the units of the given cells.
    fn match_units(&self, f: &mut Found, cells: &[(i32, i32)], pnot: bool, pname: &str) -> bool {
        for &(x, y) in cells {
            for &d in self.lvl.at(x, y) {
                if Some(d) == self.me.unit || f.units.contains(&d) {
                    continue;
                }
                if self.name_ok(d, pnot, pname) {
                    f.units.insert(d);
                    return true;
                }
            }
        }
        false
    }

    fn cell_empty(&self, x: i32, y: i32) -> bool {
        self.lvl.tile_empty(x, y) && !self.lvl.map_tile(x, y)
    }

    // ---------------------------------------------------------- level (§5)

    fn snapshot(&self, idx: &[usize]) -> Vec<String> {
        let mut v = Vec::new();
        if let Some(s) = &self.lvl.ctx.surroundings {
            for &i in idx {
                if s.cells[i].is_empty() {
                    v.push("-".to_string());
                } else {
                    v.extend(s.cells[i].iter().cloned());
                }
            }
        }
        v
    }

    fn level_params(&self, params: &[String], snap: &[usize]) -> bool {
        let lvl = self.lvl;
        let mut f = Found::default();
        let snapshot = self.snapshot(snap);
        for p in params {
            let (pnot, pname) = split(p);
            if pname.starts_with("group") {
                return false;
            }
            let mut ok = false;
            if p != "empty" && p != "level" {
                if !pnot {
                    if let Some(&d) = lvl.units_named(p).iter().find(|d| !f.units.contains(d)) {
                        f.units.insert(d);
                        ok = true;
                    }
                } else {
                    // Deviation 5: names ascending.
                    'outer: for n in 0..lvl.defs.name_count() {
                        let nm = lvl.defs.name(n as u16);
                        if nm == pname {
                            continue;
                        }
                        for &d in &lvl.by_name[n] {
                            if !f.units.contains(&d) {
                                f.units.insert(d);
                                ok = true;
                                break 'outer;
                            }
                        }
                    }
                }
            } else if p == "empty" {
                for y in 1..lvl.h - 1 {
                    for x in 1..lvl.w - 1 {
                        let t = lvl.tile_index(x, y);
                        if !ok && lvl.tile_empty(x, y) && !f.cells.contains(&t) {
                            f.cells.insert(t);
                            ok = true;
                        }
                    }
                }
            } else {
                if let Some(&d) = lvl
                    .units_named("level")
                    .iter()
                    .find(|d| !f.units.contains(d))
                {
                    f.units.insert(d);
                    ok = true;
                }
            }
            if p != "text" && !ok {
                for d in &snapshot {
                    if (!pnot && d == pname) || (pnot && d != pname) {
                        ok = true;
                    }
                }
            }
            if ok || p == "text" {
                f.count += 1;
            }
        }
        f.count == params.len()
    }

    // ---------------------------------------------------------- ON (§4.2)

    fn on(&self, params: &[String]) -> bool {
        let lvl = self.lvl;
        if params.is_empty() {
            return false;
        }
        match self.subj {
            Subj::Level => return self.level_params(params, &[8]),
            Subj::Empty(_, _) => return params.iter().all(|p| p == "level"),
            Subj::Unit(_) => {}
        }
        let (x, y) = (self.me.x, self.me.y);
        let mut f = Found::default();
        for p in params {
            let (pnot, pname) = split(p);
            if pname.starts_with("group") {
                return false;
            }
            if (pname != "empty" && p != "level") || (p == "level" && f.level) {
                if self.match_units(&mut f, &[(x, y)], pnot, pname) {
                    f.count += 1;
                }
            } else if pname == "empty" {
                if !pnot {
                    return false;
                }
                // Quirk: every other unconsumed unit counts once.
                let others: Vec<Handle> = lvl
                    .at(x, y)
                    .iter()
                    .copied()
                    .filter(|&d| Some(d) != self.me.unit && !f.units.contains(&d))
                    .collect();
                for d in others {
                    f.units.insert(d);
                    f.count += 1;
                }
            } else if !f.level {
                f.level = true;
                f.count += 1;
            }
        }
        f.count == params.len()
    }

    // ------------------------------------------------ NEAR / NEXTTO (§4.3–4)

    fn area(&self, params: &[String], near: bool) -> bool {
        let lvl = self.lvl;
        if params.is_empty() {
            return false;
        }
        if self.subj == Subj::Level {
            return if near {
                self.level_params(params, &[0, 1, 2, 3, 4, 5, 6, 7, 8])
            } else {
                self.level_params(params, &[0, 1, 2, 3, 4, 5, 6, 7])
            };
        }
        let (x, y) = (self.me.x, self.me.y);
        let cells: Vec<(i32, i32)> = if near {
            let mut v = Vec::new();
            for gx in -1..=1 {
                for gy in -1..=1 {
                    v.push((x + gx, y + gy));
                }
            }
            v
        } else {
            vec![(x + 1, y), (x, y - 1), (x - 1, y), (x, y + 1)]
        };
        let mut f = Found::default();
        for p in params {
            let (pnot, pname) = split(p);
            if pname.starts_with("group") {
                return false;
            }
            if p != "level" || f.level {
                if pname != "empty" {
                    if self.match_units(&mut f, &cells, pnot, pname) {
                        f.count += 1;
                    }
                } else {
                    let mut ok = false;
                    for &(cx, cy) in &cells {
                        if ok {
                            break;
                        }
                        let t = cx + cy * lvl.w;
                        let mut ne =
                            lvl.on_grid(cx, cy) && self.cell_empty(cx, cy) && !f.cells.contains(&t);
                        if (cx, cy) == (x, y) {
                            if matches!(self.subj, Subj::Empty(_, _)) {
                                if !pnot {
                                    ne = false;
                                }
                            } else if pnot && lvl.at(x, y).len() <= 1 {
                                ne = true;
                            }
                        }
                        if ne != pnot {
                            f.cells.insert(t);
                            ok = true;
                        }
                    }
                    if ok {
                        f.count += 1;
                    }
                }
            } else {
                f.level = true;
                f.count += 1;
            }
        }
        f.count == params.len()
    }

    // ---------------------------------------------------------- FACING

    fn facing(&self, params: &[String], g: &Guard) -> bool {
        let lvl = self.lvl;
        if self.subj == Subj::Level {
            let d = self.me.dir;
            if d > 3 || params.is_empty() {
                return false;
            }
            return self.level_params(params, &[d as usize]);
        }
        let d = match self.subj {
            Subj::Empty(x, y) => {
                if self.caller.has(self.cid) {
                    4
                } else {
                    empty_dir(lvl, x, y, g, self.depth)
                }
            }
            _ => self.me.dir,
        };
        if d > 3 || params.is_empty() {
            return false;
        }
        let (dx, dy) = dir_delta(d);
        let (tx, ty) = (self.me.x + dx, self.me.y + dy);
        let extras: Vec<String> = lvl
            .defs
            .word_args("facing")
            .map(|(_, e)| e.to_vec())
            .unwrap_or_default();
        let mut f = Found::default();
        for p in params {
            let (pnot, pname) = split(p);
            if pname.starts_with("group") {
                return false;
            }
            let ok = if extras.iter().any(|e| e == pname) {
                let want = match pname {
                    "right" => 0,
                    "up" => 1,
                    "left" => 2,
                    "down" => 3,
                    _ => 9,
                };
                (d == want) != pnot
            } else if pname == "empty" {
                let e = lvl.on_grid(tx, ty) && self.cell_empty(tx, ty);
                e != pnot
            } else if p == "level" && !f.level {
                f.level = true;
                true
            } else {
                self.match_units(&mut f, &[(tx, ty)], pnot, pname)
            };
            if ok {
                f.count += 1;
            }
        }
        f.count == params.len()
    }

    // ---------------------------------------------------------- FACEDBY

    fn facedby(&self, params: &[String]) -> bool {
        let lvl = self.lvl;
        if params.is_empty() {
            return false;
        }
        let (x, y) = (self.me.x, self.me.y);
        let mut done = vec![false; params.len()];
        let mut f = Found::default();
        for nd in 0..4u8 {
            let (dx, dy) = dir_delta(nd);
            let (nx, ny) = (x + dx, y + dy);
            for (i, p) in params.iter().enumerate() {
                if done[i] {
                    continue;
                }
                let (pnot, pname) = split(p);
                if pname.starts_with("group") {
                    return false;
                }
                if p == "level" && !f.level {
                    f.level = true;
                    done[i] = true;
                    continue;
                }
                if pname == "empty" {
                    let e = lvl.on_grid(nx, ny) && self.cell_empty(nx, ny);
                    let points = e && {
                        let ed = empty_dir(lvl, nx, ny, &Guard::default(), self.depth);
                        ed == crate::unit::opposite(nd)
                    };
                    if points != pnot {
                        done[i] = true;
                    }
                    continue;
                }
                for &d in lvl.at(nx, ny) {
                    if Some(d) == self.me.unit || f.units.contains(&d) {
                        continue;
                    }
                    let u = lvl.unit(d);
                    let faces = u.dir == crate::unit::opposite(nd);
                    let n = lvl.rname(d);
                    let ok = if !pnot {
                        n == pname && faces
                    } else {
                        n != "text" && !(n == pname && faces)
                    };
                    if ok {
                        f.units.insert(d);
                        done[i] = true;
                        break;
                    }
                }
            }
        }
        done.iter().all(|&d| d)
    }

    // ---------------------------------------------------------- SEEING

    fn seeing(&self, params: &[String], g: &Guard) -> (bool, bool) {
        let lvl = self.lvl;
        if self.caller.seeing.contains(&self.cid) {
            return (false, false);
        }
        if self.subj == Subj::Level {
            let d = self.me.dir;
            if d > 3 || params.is_empty() {
                return (false, false);
            }
            let r = self.level_params(params, &[d as usize]);
            return (r, r);
        }
        let d = match self.subj {
            Subj::Empty(x, y) => empty_dir(lvl, x, y, g, self.depth),
            _ => self.me.dir,
        };
        if params.is_empty() || d > 3 {
            return (false, false);
        }
        let (dx, dy) = dir_delta(d);
        let (mut cx, mut cy) = (self.me.x + dx, self.me.y + dy);
        let mut seen: Vec<Handle> = Vec::new();
        let mut empties = 0usize;
        while lvl.inside(cx, cy) {
            let here = lvl.at(cx, cy);
            let mut visible = 0;
            let mut solid = lvl.map_tile(cx, cy);
            for &h in here {
                let hide = lvl.has_g(lvl.uname(h), "is", "hide", Subj::Unit(h), g);
                if !hide {
                    visible += 1;
                    seen.push(h);
                }
                let blocks = (lvl.has_g(lvl.uname(h), "is", "stop", Subj::Unit(h), g)
                    || lvl.has_g(lvl.uname(h), "is", "push", Subj::Unit(h), g)
                    || lvl.has_g(lvl.uname(h), "is", "pull", Subj::Unit(h), g))
                    && !hide
                    && !lvl.has_g(lvl.uname(h), "is", "phantom", Subj::Unit(h), g);
                if blocks {
                    solid = true;
                }
            }
            if visible == 0 {
                empties += 1;
            }
            if solid {
                break;
            }
            cx += dx;
            cy += dy;
        }
        let mut used = BTreeSet::new();
        let mut level_used = false;
        let mut empty_used = 0;
        let mut count = 0;
        for p in params {
            let (pnot, pname) = split(p);
            if pname.starts_with("group") {
                return (false, false);
            }
            let ok = if pname == "empty" {
                if empty_used < empties {
                    empty_used += 1;
                    true
                } else {
                    false
                }
            } else if p == "level" && !level_used {
                level_used = true;
                true
            } else if let Some(&h) = seen.iter().find(|&&h| {
                Some(h) != self.me.unit && !used.contains(&h) && self.name_ok(h, pnot, pname)
            }) {
                used.insert(h);
                true
            } else {
                false
            };
            if ok {
                count += 1;
            }
        }
        let r = count == params.len();
        (r, r)
    }

    // ---------------------------------------------------------- WITHOUT

    fn without(&self, params: &[String]) -> bool {
        let lvl = self.lvl;
        let mut count: BTreeMap<&str, usize> = BTreeMap::new();
        for p in params {
            *count.entry(p.as_str()).or_insert(0) += 1;
        }
        if let Some(c) = count.get_mut("level") {
            *c -= 1;
        }
        let mut any = false;
        let mut all = true;
        for p in params {
            let (pnot, pname) = split(p);
            if pname.starts_with("group") {
                return false;
            }
            let sat = if p == "level" && count["level"] == 0 {
                return false;
            } else if p == "empty" {
                let mut n = 0;
                for y in 1..lvl.h - 1 {
                    for x in 1..lvl.w - 1 {
                        if self.cell_empty(x, y) {
                            n += 1;
                        }
                    }
                }
                let lim = count["empty"] + matches!(self.subj, Subj::Empty(_, _)) as usize;
                n < lim
            } else if pnot {
                let lim = count[p.as_str()];
                let mut n = 0;
                for &h in &lvl.order {
                    if n >= lim {
                        break;
                    }
                    if Some(h) != self.me.unit && lvl.rname(h) != pname && !lvl.is_text(h) {
                        n += 1;
                    }
                }
                n < lim
            } else {
                let units = lvl.units_named(pname).len();
                let lim = count[p.as_str()] + (self.me.name == pname) as usize;
                units == 0 || units < lim
            };
            any |= sat;
            all &= sat;
        }
        if self.neg { any } else { all }
    }

    // ------------------------------------------- ABOVE / BELOW / BESIDE

    fn line(&self, k: CondKind, params: &[String]) -> bool {
        let lvl = self.lvl;
        if params.is_empty() {
            return false;
        }
        if self.subj == Subj::Level {
            let idx = match k {
                CondKind::Above => 3,
                CondKind::Below => 1,
                CondKind::BesideRight => 2,
                _ => 0,
            };
            return self.level_params(params, &[idx]);
        }
        let (x, y) = (self.me.x, self.me.y);
        let cells: Vec<(i32, i32)> = match k {
            CondKind::Above => (y + 1..=lvl.h - 2).map(|yy| (x, yy)).collect(),
            CondKind::Below => (1..y).rev().map(|yy| (x, yy)).collect(),
            CondKind::BesideRight => (1..x).rev().map(|xx| (xx, y)).collect(),
            _ => (x + 1..=lvl.w - 2).map(|xx| (xx, y)).collect(),
        };
        let mut f = Found::default();
        for p in params {
            let (pnot, pname) = split(p);
            if pname.starts_with("group") {
                return false;
            }
            let ok = if pname == "empty" {
                let mut ok = false;
                for &(cx, cy) in &cells {
                    let t = cx + cy * lvl.w;
                    if !f.cells.contains(&t) && self.cell_empty(cx, cy) != pnot {
                        f.cells.insert(t);
                        ok = true;
                        break;
                    }
                }
                ok
            } else if p == "level" && !f.level {
                f.level = true;
                true
            } else {
                self.match_units(&mut f, &cells, pnot, pname)
            };
            if ok {
                f.count += 1;
            }
        }
        f.count == params.len()
    }

    // ---------------------------------------------------------- FEELING

    fn feeling(&self, params: &[String], g: &mut Guard, o: Opts<'_>) -> (bool, bool) {
        let lvl = self.lvl;
        if params.is_empty() {
            return (false, false);
        }
        let n = &self.me.name;
        let mut all = true;
        for p in params {
            let (pnot, pname) = split(p);
            let mut sat = false;
            for &ri in lvl.rules.list(n) {
                let r = &lvl.rules.features[ri];
                if r.subject != *n || r.verb != "is" || g.has(r.cid) {
                    continue;
                }
                let target_ok = if !pnot {
                    r.object == pname
                } else {
                    !r.object.starts_with("not ")
                        && r.object != pname
                        && lvl.defs.word_type_of(&r.object) == Some(2)
                };
                if !target_ok {
                    continue;
                }
                g.ids.push(r.cid);
                let gg = g.clone();
                if eval(
                    lvl,
                    &r.conds,
                    r.cid,
                    self.subj,
                    &gg,
                    Opts {
                        depth: self.depth,
                        members: None,
                        ..o
                    },
                ) {
                    sat = true;
                    break;
                }
            }
            all &= sat;
        }
        (all, all)
    }

    // ---------------------------------------------------------- POWERED

    fn powered(&self, subtype: &str, o: Opts<'_>) -> (bool, bool) {
        let lvl = self.lvl;
        let top = self.caller.is_empty();
        if top && let Some(&v) = lvl.caches.borrow().powered.get(subtype) {
            return (v, true);
        }
        let prop = format!("power{subtype}");
        let mut res = false;
        for &ri in lvl.rules.list(&prop) {
            let r = &lvl.rules.features[ri];
            if r.subject.starts_with("not ")
                || r.verb != "is"
                || r.object != prop
                || self.caller.has(r.cid)
            {
                continue;
            }
            let g = self.caller.with(r.cid);
            let op = Opts {
                depth: self.depth,
                members: None,
                ..o
            };
            let ok = match r.subject.as_str() {
                "empty" => {
                    let mut ok = false;
                    'e: for y in 1..lvl.h - 1 {
                        for x in 1..lvl.w - 1 {
                            if lvl.tile_empty(x, y)
                                && eval(lvl, &r.conds, r.cid, Subj::Empty(x, y), &g, op)
                            {
                                ok = true;
                                break 'e;
                            }
                        }
                    }
                    ok
                }
                "level" => eval(lvl, &r.conds, r.cid, Subj::Level, &g, op),
                s => lvl
                    .units_named(s)
                    .iter()
                    .any(|&h| eval(lvl, &r.conds, r.cid, Subj::Unit(h), &g, op)),
            };
            if ok {
                res = true;
                break;
            }
        }
        if top {
            lvl.caches
                .borrow_mut()
                .powered
                .insert(subtype.to_string(), res);
        }
        (res, true)
    }
}

/// Direction of an empty tile (§6.1).
pub(crate) fn empty_dir(lvl: &Level, x: i32, y: i32, g: &Guard, depth: u32) -> u8 {
    let mut d = 4u8;
    if !lvl.rules.index.contains_key("empty") {
        return d;
    }
    let subj = Subj::Empty(x, y);
    let op = Opts {
        depth,
        autofail: true,
        ..Default::default()
    };
    let sleep = |lvl: &Level| {
        has_g(
            lvl,
            "empty",
            "is",
            "sleep",
            subj,
            g,
            Opts {
                depth,
                ..Default::default()
            },
        )
    };
    let mut chosen = false;
    for &ri in lvl.rules.list("empty") {
        let r = &lvl.rules.features[ri];
        if r.subject != "empty" {
            continue;
        }
        if r.verb == "is" {
            let nd = match r.object.as_str() {
                "right" => 0,
                "up" => 1,
                "left" => 2,
                "down" => 3,
                _ => continue,
            };
            if eval(lvl, &r.conds, r.cid, subj, g, op) && !sleep(lvl) {
                d = nd;
            }
        } else if r.verb == "follow" {
            let t = r.object.as_str();
            if matches!(t, "all" | "empty" | "level") || t.starts_with("not ") {
                continue;
            }
            let names: Vec<String> = if t.starts_with("group") {
                members(lvl, t, g, op)
            } else {
                vec![t.to_string()]
            };
            for n in names {
                if chosen {
                    break;
                }
                let units = lvl.units_named(&n);
                if !lvl.known.contains(&n) || units.is_empty() {
                    continue;
                }
                if sleep(lvl) || !eval(lvl, &r.conds, r.cid, subj, g, op) {
                    continue;
                }
                let mut best = i32::MAX;
                let mut bd = 4u8;
                for &h in units {
                    let u = lvl.unit(h);
                    let (dx, dy) = (u.x - x, u.y - y);
                    let dist = dx.abs() + dy.abs();
                    if dist > 0 && dist <= best {
                        best = dist;
                        bd = if dx.abs() <= dy.abs() {
                            if dy >= 0 { 3 } else { 1 }
                        } else if dx > 0 {
                            0
                        } else {
                            2
                        };
                    }
                }
                if bd < 4 {
                    d = bd;
                    chosen = true;
                }
            }
        }
    }
    d
}

/// `hasfeature` with a guard (§18).
pub(crate) fn has_g(
    lvl: &Level,
    s: &str,
    v: &str,
    o: &str,
    subj: Subj,
    g: &Guard,
    op: Opts<'_>,
) -> bool {
    for &ri in lvl.rules.list(s) {
        let r = &lvl.rules.features[ri];
        if r.subject == s && r.verb == v && r.object == o && eval(lvl, &r.conds, r.cid, subj, g, op)
        {
            return true;
        }
    }
    if s.starts_with("text_") {
        for &ri in lvl.rules.list("text") {
            let r = &lvl.rules.features[ri];
            if r.subject == "text"
                && r.verb == v
                && r.object == o
                && eval(lvl, &r.conds, r.cid, subj, g, op)
            {
                return true;
            }
        }
    }
    false
}

impl Level {
    /// Condition list holds for a subject (top level).
    pub(crate) fn holds(&self, conds: &[Cond], cid: u32, subj: Subj) -> bool {
        eval(self, conds, cid, subj, &Guard::default(), Opts::default())
    }

    /// `hasfeature` (top level).
    pub(crate) fn has(&self, s: &str, v: &str, o: &str, subj: Subj) -> bool {
        has_g(self, s, v, o, subj, &Guard::default(), Opts::default())
    }

    /// `hasfeature` inside a condition (guard passed along, depth reset).
    pub(crate) fn has_g(&self, s: &str, v: &str, o: &str, subj: Subj, g: &Guard) -> bool {
        has_g(self, s, v, o, subj, g, Opts::default())
    }

    /// Empty-tile direction (top level).
    pub(crate) fn empty_dir(&self, x: i32, y: i32) -> u8 {
        empty_dir(self, x, y, &Guard::default(), 0)
    }

    /// Group members (top level).
    pub(crate) fn group_members(&self, g: &str) -> Vec<String> {
        members(self, g, &Guard::default(), Opts::default())
    }
}
