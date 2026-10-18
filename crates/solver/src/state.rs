//! State keys (duplicate detection), novelty atoms, goals and heuristics.

use baba_sim::Level;

/// 64-bit FxHash-style mixing (no dependencies; solver only, not the sim).
#[derive(Clone, Copy)]
pub struct Mix(pub u64);

impl Mix {
    pub fn new() -> Mix {
        Mix(0xcbf2_9ce4_8422_2325)
    }
    pub fn add(&mut self, v: u64) {
        self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
    pub fn finish(self) -> u64 {
        let mut x = self.0;
        x ^= x >> 33;
        x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
        x ^= x >> 33;
        x
    }
}

pub fn hash_str(s: &str) -> u64 {
    let mut m = Mix::new();
    for b in s.bytes() {
        m.add(b as u64);
    }
    m.finish()
}

/// Per-level settings for keys and heuristics.
#[derive(Clone, Debug)]
pub struct KeyOpts {
    /// Facing is part of the key (levels where direction can matter).
    pub dir: bool,
}

/// Words that make facing relevant to the outcome.
const DIR_WORDS: &[&str] = &[
    "move",
    "shift",
    "facing",
    "turn",
    "deturn",
    "follow",
    "back",
    "auto",
    "up",
    "down",
    "left",
    "right",
    "nudgeup",
    "nudgedown",
    "nudgeleft",
    "nudgeright",
    "fall",
    "fallup",
    "falldown",
    "fallleft",
    "fallright",
    "seeing",
    "select",
    "chill",
    "tele",
    "power",
    "powered",
    "boom",
];

impl KeyOpts {
    /// Facing matters if a direction-related word or letters exist.
    pub fn for_level(lvl: &Level) -> KeyOpts {
        let d = lvl.defs();
        let mut dir = false;
        for u in lvl.unit_records() {
            let k = d.kind(u.kind);
            if k.is_text && (DIR_WORDS.contains(&k.word.as_str()) || k.word_type == 5) {
                dir = true;
            }
        }
        KeyOpts { dir }
    }
}

/// Order-independent key of the game state (ignores ids, undo history and
/// the PRNG).
pub fn key(lvl: &Level, o: &KeyOpts) -> u64 {
    key_without(lvl, u32::MAX, o)
}

/// Like [`key`] but without one unit (the walker in macro search).
pub fn key_without(lvl: &Level, skip: u32, o: &KeyOpts) -> u64 {
    let mut v: Vec<u64> = Vec::with_capacity(256);
    for u in lvl.unit_records() {
        if u.handle == skip {
            continue;
        }
        let mut p = (u.kind as u64) << 40
            | ((u.x as u64) & 0x3ff) << 30
            | ((u.y as u64) & 0x3ff) << 20
            | (u.float as u64) << 16
            | (u.broken as u64) << 15;
        if o.dir {
            p |= (u.dir as u64) << 12;
        }
        if u.followed > 0 || u.holder > 0 || u.back_start > 0 || u.completion() != 0 {
            let mut m = Mix::new();
            m.add(p);
            m.add(u.followed as u64);
            m.add(u.holder as u64);
            m.add(u.back_start as u64);
            m.add(u.completion() as u64);
            p = m.finish();
        }
        v.push(p);
    }
    v.sort_unstable();
    let mut m = Mix::new();
    for p in v {
        m.add(p);
    }
    m.add(lvl.mapdir() as u64);
    m.add(lvl.rotation() as u64);
    m.add(lvl.offset().0 as u64);
    m.add(lvl.offset().1 as u64);
    m.add(lvl.bonus() as u64);
    m.finish()
}

/// Properties/verbs that make things happen without the player pushing
/// anything (walking is then not a pure position change).
const TURN_PROPS: &[&str] = &[
    "move",
    "auto",
    "shift",
    "fall",
    "fallright",
    "fallup",
    "fallleft",
    "falldown",
    "follow",
    "back",
    "tele",
    "chill",
    "more",
    "nudgeright",
    "nudgeup",
    "nudgeleft",
    "nudgedown",
    "turn",
    "deturn",
    "you2",
    "3d",
    "sleep",
];
const TURN_VERBS: &[&str] = &["make", "fear", "follow"];

/// Walking model for macro moves: the single YOU unit and the cells it can
/// walk through without changing anything else (empty or holding only
/// objects no rule mentions). `None` when the model does not apply
/// (several YOU units, conditional rules, things that act every turn).
pub struct Walk {
    pub you: u32,
    pub pos: (i32, i32),
    pub free: Vec<bool>,
    /// Moving into the cell is predicted to change nothing (border, or
    /// only objects whose sole rule is `X is stop`, with a walker that is
    /// only YOU).
    pub wall: Vec<bool>,
    /// Pushed things stop here (border, stop-only objects).
    pub block: Vec<bool>,
    /// Only push-only things here (text, `X is push` objects).
    pub pushy: Vec<bool>,
    pub w: i32,
}

pub fn walk_model(lvl: &Level) -> Option<Walk> {
    let mut subjects: Vec<&str> = Vec::new();
    let mut you_subj: Vec<&str> = Vec::new();
    for r in &lvl.rules().features {
        if r.disabled() {
            continue;
        }
        if !r.conds.is_empty()
            || r.subject == "empty"
            || TURN_VERBS.contains(&r.verb.as_str())
            || (r.verb == "is" && TURN_PROPS.contains(&r.object.as_str()))
        {
            return None;
        }
        if r.verb == "is" && r.object == "you" {
            if r.subject == "level" {
                return None;
            }
            you_subj.push(&r.subject);
        }
        subjects.push(&r.subject);
    }
    let d = lvl.defs();
    let all = subjects.contains(&"all");
    // Kinds whose only rule is `X is stop`.
    let rules = &lvl.rules().features;
    let only = |name: &str, prop: &str| {
        let mut any = false;
        for r in rules.iter().filter(|r| !r.disabled() && r.subject == name) {
            if r.verb != "is" || r.object != prop {
                return false;
            }
            any = true;
        }
        any
    };
    let only_stop = |name: &str| only(name, "stop");
    let text_push = only("text", "push");
    let (w, h) = (lvl.width(), lvl.height());
    let n = (w * h) as usize;
    let mut free = vec![true; n];
    let mut wall = vec![false; n];
    for x in 0..w {
        for y in 0..h {
            if x == 0 || y == 0 || x == w - 1 || y == h - 1 {
                free[(y * w + x) as usize] = false;
                wall[(y * w + x) as usize] = true;
            }
        }
    }
    let mut you = None;
    let mut stopper = vec![false; n];
    let mut pusher = vec![false; n];
    let mut other = vec![false; n];
    let mut block = wall.clone();
    for u in lvl.unit_records() {
        let k = d.kind(u.kind);
        if you_subj
            .iter()
            .any(|s| subject_matches(s, &k.name, k.is_text))
        {
            if you.is_some() {
                return None;
            }
            you = Some((u.handle, (u.x, u.y), k.name.clone()));
            continue;
        }
        let inert = !k.is_text && !subjects.contains(&k.name.as_str()) && !all;
        if !inert && u.x >= 0 && u.y >= 0 && u.x < w && u.y < h {
            let i = (u.y * w + u.x) as usize;
            free[i] = false;
            if !k.is_text && !all && only_stop(&k.name) {
                stopper[i] = true;
            } else if (k.is_text && text_push && !subjects.contains(&k.name.as_str()))
                || (!k.is_text && !all && only(&k.name, "push"))
            {
                pusher[i] = true;
            } else {
                other[i] = true;
            }
        }
    }
    let (you, pos, yname) = you?;
    let walker_plain = rules
        .iter()
        .filter(|r| !r.disabled() && (r.subject == yname || r.subject == "all"))
        .all(|r| r.verb == "is" && r.object == "you");
    let mut pushy = vec![false; n];
    for i in 0..n {
        if stopper[i] && !other[i] && !pusher[i] {
            block[i] = true;
        }
        pushy[i] = pusher[i] && !stopper[i] && !other[i];
    }
    if walker_plain {
        wall.clone_from(&block);
    } else {
        pushy.fill(false);
    }
    Some(Walk {
        you,
        pos,
        free,
        wall,
        block,
        pushy,
        w,
    })
}

/// Novelty atoms: (kind, x, y) per unit plus one per active rule.
pub fn atoms(lvl: &Level, out: &mut Vec<u64>) {
    out.clear();
    for u in lvl.unit_records() {
        out.push((u.kind as u64) << 24 | ((u.x as u64) & 0xfff) << 12 | ((u.y as u64) & 0xfff));
    }
    for r in &lvl.rules().features {
        if !r.disabled() {
            out.push(hash_str(&format!("{} {} {}", r.subject, r.verb, r.object)) | 1 << 63);
        }
    }
}

/// A search goal.
#[derive(Clone, Debug)]
pub enum Goal {
    /// The level is won (also END / ALL IS DONE).
    Win,
    /// A rule `subject verb object` is active (conditions ignored).
    Rule(String),
    /// No such rule is active.
    NoRule(String),
    /// A unit of this kind name is at the cell.
    At(String, i32, i32),
    /// No unit of this kind name exists.
    Gone(String),
    /// Two units of this kind name touch (8-neighbourhood).
    Pair(String),
    /// Some unit of this kind name exists.
    Exists(String),
    /// Exactly this many units of the kind name exist.
    Count(String, usize),
    /// Every unit of this kind name faces this way (0 R, 1 U, 2 L, 3 D).
    Face(String, u8),
    /// No unit of this kind name (`*` = any) at the cell.
    NotAt(String, i32, i32),
    /// A unit of the first kind is next to (or on) one of the second.
    Near(String, String),
}

/// Smallest Chebyshev distance between two units (of kinds `a`, `b`).
fn kind_dist(lvl: &Level, a: &str, b: &str) -> i32 {
    let d = lvl.defs();
    let pa: Vec<(i32, i32)> = lvl
        .unit_records()
        .iter()
        .filter(|u| d.kind(u.kind).name == a)
        .map(|u| (u.x, u.y))
        .collect();
    let pb: Vec<(i32, i32)> = if a == b {
        pa.clone()
    } else {
        lvl.unit_records()
            .iter()
            .filter(|u| d.kind(u.kind).name == b)
            .map(|u| (u.x, u.y))
            .collect()
    };
    let mut m = 99;
    for (i, p) in pa.iter().enumerate() {
        for (j, q) in pb.iter().enumerate() {
            if a == b && i == j {
                continue;
            }
            m = m.min((p.0 - q.0).abs().max((p.1 - q.1).abs()));
        }
    }
    m
}

impl Goal {
    pub fn parse(s: &str) -> Result<Goal, String> {
        if s == "win" {
            return Ok(Goal::Win);
        }
        let (k, v) = s.split_once(':').ok_or(format!("bad goal {s}"))?;
        Ok(match k {
            "rule" => Goal::Rule(v.to_lowercase()),
            "norule" => Goal::NoRule(v.to_lowercase()),
            "gone" => Goal::Gone(v.to_lowercase()),
            "pair" => Goal::Pair(v.to_lowercase()),
            "exists" => Goal::Exists(v.to_lowercase()),
            "count" => {
                let (n, c) = v.rsplit_once(':').ok_or("count:NAME:N")?;
                Goal::Count(n.to_lowercase(), c.parse().map_err(|_| "count:NAME:N")?)
            }
            "face" => {
                let (n, d) = v.rsplit_once(':').ok_or("face:NAME:R|U|L|D")?;
                let d = match d {
                    "R" | "r" => 0,
                    "U" | "u" => 1,
                    "L" | "l" => 2,
                    "D" | "d" => 3,
                    _ => return Err("face:NAME:R|U|L|D".into()),
                };
                Goal::Face(n.to_lowercase(), d)
            }
            "notat" => {
                let (n, xy) = v.rsplit_once(':').ok_or("notat:NAME:X,Y")?;
                let (x, y) = xy.split_once(',').ok_or("notat:NAME:X,Y")?;
                Goal::NotAt(
                    n.to_string(),
                    x.parse().map_err(|_| "x")?,
                    y.parse().map_err(|_| "y")?,
                )
            }
            "near" => {
                let (a, b) = v.split_once(':').ok_or("near:A:B")?;
                Goal::Near(a.to_lowercase(), b.to_lowercase())
            }
            "at" => {
                let (n, xy) = v.rsplit_once(':').ok_or("at:NAME:X,Y")?;
                let (x, y) = xy.split_once(',').ok_or("at:NAME:X,Y")?;
                Goal::At(
                    n.to_string(),
                    x.parse().map_err(|_| "x")?,
                    y.parse().map_err(|_| "y")?,
                )
            }
            _ => return Err(format!("bad goal {s}")),
        })
    }
}

fn has_rule(lvl: &Level, s: &str) -> bool {
    lvl.rules()
        .features
        .iter()
        .any(|r| !r.disabled() && format!("{} {} {}", r.subject, r.verb, r.object) == s)
}

/// State predicate of a (non-win) goal.
pub fn holds(lvl: &Level, g: &Goal) -> bool {
    let d = lvl.defs();
    match g {
        Goal::Win => false,
        Goal::Rule(s) => has_rule(lvl, s),
        Goal::NoRule(s) => !has_rule(lvl, s),
        Goal::At(n, x, y) => lvl
            .unit_records()
            .iter()
            .any(|u| u.x == *x && u.y == *y && d.kind(u.kind).name == *n),
        Goal::Gone(n) => !lvl.unit_records().iter().any(|u| d.kind(u.kind).name == *n),
        Goal::Pair(n) => kind_dist(lvl, n, n) <= 1,
        Goal::Exists(n) => lvl.unit_records().iter().any(|u| d.kind(u.kind).name == *n),
        Goal::Count(n, c) => {
            lvl.unit_records()
                .iter()
                .filter(|u| d.kind(u.kind).name == *n)
                .count()
                == *c
        }
        Goal::Face(n, dir) => lvl
            .unit_records()
            .iter()
            .filter(|u| d.kind(u.kind).name == *n)
            .all(|u| u.dir == *dir),
        Goal::NotAt(n, x, y) => !lvl
            .unit_records()
            .iter()
            .any(|u| u.x == *x && u.y == *y && (n == "*" || d.kind(u.kind).name == *n)),
        Goal::Near(a, b) => kind_dist(lvl, a, b) <= 1,
    }
}

fn subject_matches(subj: &str, name: &str, is_text: bool) -> bool {
    subj == name
        || (is_text && subj == "text")
        || (!is_text && subj == "all" && name != "level" && name != "cursor")
}

/// Positions of units that some active `X is <prop>` rule applies to
/// (conditions ignored). `level` subjects count as "everywhere".
fn units_with(lvl: &Level, prop: &str, out: &mut Vec<(i32, i32)>) -> bool {
    out.clear();
    let d = lvl.defs();
    let subs: Vec<&str> = lvl
        .rules()
        .features
        .iter()
        .filter(|r| !r.disabled() && r.verb == "is" && r.object == prop)
        .map(|r| r.subject.as_str())
        .collect();
    if subs.is_empty() {
        return false;
    }
    let level = subs.contains(&"level");
    for u in lvl.unit_records() {
        let k = d.kind(u.kind);
        if subs.iter().any(|s| subject_matches(s, &k.name, k.is_text)) {
            out.push((u.x, u.y));
        }
    }
    level
}

/// Text units as (word, position).
type Texts<'a> = Vec<(&'a str, (i32, i32))>;

fn text_list(lvl: &Level) -> Texts<'_> {
    let d = lvl.defs();
    lvl.unit_records()
        .into_iter()
        .filter_map(|u| {
            let k = d.kind(u.kind);
            k.is_text.then_some((k.word.as_str(), (u.x, u.y)))
        })
        .collect()
}

fn dist(a: (i32, i32), b: (i32, i32)) -> i32 {
    (a.0 - b.0).abs() + (a.1 - b.1).abs()
}

const FAR: i32 = 40;

/// Text displacement estimates: per word, the number of pushes to bring a
/// text of that word to each cell, where a push from p to p+d needs p-d
/// (the pusher's cell) and p+d to be free of the border and of STOP
/// objects; cells holding a text that can never move are reserved for it.
struct TextCtx<'a> {
    tx: Texts<'a>,
    w: i32,
    h: i32,
    blocked: Vec<bool>,
    fixed: Vec<Option<&'a str>>,
    cache: Vec<(&'a str, Vec<i32>)>,
}

impl<'a> TextCtx<'a> {
    fn new(lvl: &'a Level, tx: Texts<'a>) -> TextCtx<'a> {
        let (w, h) = (lvl.width(), lvl.height());
        let n = (w * h) as usize;
        let mut blocked = vec![false; n];
        for x in 0..w {
            for y in 0..h {
                if x == 0 || y == 0 || x == w - 1 || y == h - 1 {
                    blocked[(y * w + x) as usize] = true;
                }
            }
        }
        let d = lvl.defs();
        let rules = &lvl.rules().features;
        let has = |name: &str, prop: &str| {
            rules.iter().any(|r| {
                !r.disabled()
                    && r.verb == "is"
                    && r.object == prop
                    && subject_matches(&r.subject, name, false)
            })
        };
        let mut stops: Vec<(&str, bool)> = Vec::new();
        for u in lvl.unit_records() {
            let k = d.kind(u.kind);
            if k.is_text || u.x < 0 || u.y < 0 || u.x >= w || u.y >= h {
                continue;
            }
            let s = match stops.iter().find(|s| s.0 == k.name) {
                Some(s) => s.1,
                None => {
                    let s = has(&k.name, "stop") && !has(&k.name, "push");
                    stops.push((&k.name, s));
                    s
                }
            };
            if s {
                blocked[(u.y * w + u.x) as usize] = true;
            }
        }
        // Frozen texts: on each axis one neighbour is a blocker or another
        // frozen text (iterated to a fixed point, Sokoban-style).
        let mut fixed: Vec<Option<&str>> = vec![None; n];
        loop {
            let mut changed = false;
            for &(word, (x, y)) in &tx {
                if x < 0 || y < 0 || x >= w || y >= h || fixed[(y * w + x) as usize].is_some() {
                    continue;
                }
                let free = |x: i32, y: i32| {
                    x >= 0
                        && y >= 0
                        && x < w
                        && y < h
                        && !blocked[(y * w + x) as usize]
                        && fixed[(y * w + x) as usize].is_none()
                };
                let movable = [(1, 0), (0, 1)]
                    .iter()
                    .any(|&(dx, dy)| free(x - dx, y - dy) && free(x + dx, y + dy));
                if !movable {
                    fixed[(y * w + x) as usize] = Some(word);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        for (i, f) in fixed.iter().enumerate() {
            if f.is_some() {
                blocked[i] = true;
            }
        }
        TextCtx {
            tx,
            w,
            h,
            blocked,
            fixed,
            cache: Vec::new(),
        }
    }

    fn map(&mut self, word: &str) -> Option<usize> {
        if let Some(i) = self.cache.iter().position(|c| c.0 == word) {
            return Some(i);
        }
        let (w, h) = (self.w, self.h);
        let mut dm = vec![FAR; (w * h) as usize];
        let mut q = Vec::new();
        let mut key = None;
        for &(wd, (x, y)) in &self.tx {
            if wd == word && x >= 0 && y >= 0 && x < w && y < h {
                dm[(y * w + x) as usize] = 0;
                q.push((x, y));
                key = Some(wd);
            }
        }
        let key = key?;
        let free = |x: i32, y: i32| {
            x >= 0 && y >= 0 && x < w && y < h && !self.blocked[(y * w + x) as usize]
        };
        let mut i = 0;
        while i < q.len() {
            let (x, y) = q[i];
            i += 1;
            let dv = dm[(y * w + x) as usize];
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (nx, ny) = (x + dx, y + dy);
                if !free(nx, ny) || !free(x - dx, y - dy) {
                    continue;
                }
                let j = (ny * w + nx) as usize;
                if dm[j] > dv + 1 {
                    dm[j] = dv + 1;
                    q.push((nx, ny));
                }
            }
        }
        self.cache.push((key, dm));
        Some(self.cache.len() - 1)
    }

    fn cost_at(&mut self, word: &str, (x, y): (i32, i32)) -> i32 {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return FAR;
        }
        let j = (y * self.w + x) as usize;
        if let Some(f) = self.fixed[j]
            && f != word
        {
            return FAR;
        }
        match self.map(word) {
            Some(m) => self.cache[m].1[j],
            None => FAR,
        }
    }

    /// Cheapest displacement to line up `a verb b`, keeping one of the
    /// three texts where it is.
    fn rule_cost(&mut self, a: &str, verb: &str, b: &str) -> i32 {
        let words = [a, verb, b];
        let mut best = FAR;
        for (k, anchor) in words.iter().enumerate() {
            let ps: Vec<(i32, i32)> = self
                .tx
                .iter()
                .filter(|t| t.0 == *anchor)
                .map(|t| t.1)
                .collect();
            for p in ps {
                for (dx, dy) in [(1, 0), (0, 1)] {
                    let mut c = 0;
                    for (j, wd) in words.iter().enumerate() {
                        if j != k {
                            let off = j as i32 - k as i32;
                            c += self.cost_at(wd, (p.0 + off * dx, p.1 + off * dy));
                            if c >= best {
                                break;
                            }
                        }
                    }
                    best = best.min(c);
                }
            }
        }
        best
    }
}

/// Lower is better. Not admissible; a guide for best-first search.
pub fn heuristic(lvl: &Level, goals: &[Goal], buf: &mut Vec<(i32, i32)>) -> i32 {
    // Losing every player is rarely part of a plan.
    let mut h = if lvl.no_player() { 30 } else { 0 };
    for g in goals {
        h += match g {
            Goal::Win => win_h(lvl, buf),
            Goal::Rule(s) => {
                if has_rule(lvl, s) {
                    0
                } else {
                    let w: Vec<&str> = s.split(' ').collect();
                    if w.len() == 3 {
                        1 + TextCtx::new(lvl, text_list(lvl)).rule_cost(w[0], w[1], w[2])
                    } else {
                        1
                    }
                }
            }
            Goal::NoRule(s) => has_rule(lvl, s) as i32,
            Goal::At(n, x, y) => {
                let d = lvl.defs();
                lvl.unit_records()
                    .iter()
                    .filter(|u| d.kind(u.kind).name == *n)
                    .map(|u| dist((u.x, u.y), (*x, *y)))
                    .min()
                    .unwrap_or(60)
            }
            Goal::Pair(n) => kind_dist(lvl, n, n) - 1,
            Goal::NotAt(..) | Goal::Exists(..) | Goal::Face(..) => (!holds(lvl, g)) as i32 * 20,
            Goal::Count(n, c) => {
                let d = lvl.defs();
                let k = lvl
                    .unit_records()
                    .iter()
                    .filter(|u| d.kind(u.kind).name == *n)
                    .count();
                (k as i32 - *c as i32).abs() * 10
            }
            Goal::Near(a, b) => kind_dist(lvl, a, b) - 1,
            Goal::Gone(n) => {
                let d = lvl.defs();
                lvl.unit_records()
                    .iter()
                    .filter(|u| d.kind(u.kind).name == *n)
                    .count() as i32
            }
        };
    }
    h
}

/// Win estimate: over every pair of object kinds (A, B), the cost of making
/// A YOU and B WIN (0 if active, else a text-displacement estimate) plus the
/// distance between their nearest units; the minimum over pairs.
fn win_h(lvl: &Level, buf: &mut Vec<(i32, i32)>) -> i32 {
    let you_any = units_with(lvl, "you", buf) || !buf.is_empty();
    if units_with(lvl, "win", buf) && you_any {
        return 0;
    }
    let d = lvl.defs();
    let mut pos: Vec<(&str, Vec<(i32, i32)>)> = Vec::new();
    let mut words: Vec<&str> = Vec::new();
    let mut tx: Texts<'_> = Vec::new();
    for u in lvl.unit_records() {
        let k = d.kind(u.kind);
        if k.is_text {
            tx.push((&k.word, (u.x, u.y)));
            if !words.contains(&k.word.as_str()) {
                words.push(&k.word);
            }
        } else if k.name != "level" && k.name != "cursor" {
            match pos.iter_mut().find(|(n, _)| *n == k.name) {
                Some((_, v)) => v.push((u.x, u.y)),
                None => pos.push((&k.name, vec![(u.x, u.y)])),
            }
        }
    }
    let rules = &lvl.rules().features;
    let active = |name: &str, prop: &str| {
        rules.iter().any(|r| {
            !r.disabled()
                && r.verb == "is"
                && r.object == prop
                && subject_matches(&r.subject, name, false)
        })
    };
    let mut tc = TextCtx::new(lvl, tx);
    let mut cost = |name: &str, prop: &str| -> Option<i32> {
        if active(name, prop) {
            Some(0)
        } else if words.contains(&name) && words.contains(&prop) {
            let c = tc.rule_cost(name, "is", prop);
            (c < FAR).then_some(2 + 2 * c)
        } else {
            None
        }
    };
    let cy: Vec<Option<i32>> = pos.iter().map(|(n, _)| cost(n, "you")).collect();
    let cw: Vec<Option<i32>> = pos.iter().map(|(n, _)| cost(n, "win")).collect();
    // Cells a walker cannot pass: STOP (unless PUSH), DEFEAT, SINK.
    let prop_of = |name: &str, prop: &str| active(name, prop);
    let (w, h) = (lvl.width(), lvl.height());
    let mut blocked = vec![false; (w * h) as usize];
    for (n, ps) in &pos {
        let b = (prop_of(n, "stop") && !prop_of(n, "push"))
            || prop_of(n, "defeat")
            || prop_of(n, "sink");
        if b {
            for &(x, y) in ps {
                if x >= 0 && y >= 0 && x < w && y < h {
                    blocked[(y * w + x) as usize] = true;
                }
            }
        }
    }
    for x in 0..w {
        for y in 0..h {
            if x == 0 || y == 0 || x == w - 1 || y == h - 1 {
                blocked[(y * w + x) as usize] = true;
            }
        }
    }
    let mut best = 300;
    let mut queue = Vec::new();
    for (b, wb) in cw.iter().enumerate() {
        let Some(wb) = *wb else { continue };
        if cy.iter().flatten().all(|&ya| ya + wb >= best) {
            continue;
        }
        // Path distance from the B units (the goal cells are enterable).
        let mut dm = vec![i32::MAX; (w * h) as usize];
        queue.clear();
        for &(x, y) in &pos[b].1 {
            if x >= 0 && y >= 0 && x < w && y < h {
                dm[(y * w + x) as usize] = 0;
                queue.push((x, y));
            }
        }
        let mut i = 0;
        while i < queue.len() {
            let (x, y) = queue[i];
            i += 1;
            let dv = dm[(y * w + x) as usize];
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= w || ny >= h {
                    continue;
                }
                let j = (ny * w + nx) as usize;
                if dm[j] == i32::MAX {
                    dm[j] = dv + 1;
                    // Blocked cells get a distance but are not expanded.
                    if !blocked[j] {
                        queue.push((nx, ny));
                    }
                }
            }
        }
        for (a, ya) in cy.iter().enumerate() {
            let Some(ya) = *ya else { continue };
            let dd = if a == b {
                0
            } else {
                let m = pos[a]
                    .1
                    .iter()
                    .filter(|&&(x, y)| x >= 0 && y >= 0 && x < w && y < h)
                    .map(|&(x, y)| dm[(y * w + x) as usize])
                    .min()
                    .unwrap_or(i32::MAX);
                if m == i32::MAX { 40 } else { m }
            };
            if ya + wb + dd < best && std::env::var_os("SOLVER_EXPLAIN").is_some() {
                eprintln!(
                    "h {} = you({}) {} + win({}) {} + dist {}",
                    ya + wb + dd,
                    pos[a].0,
                    ya,
                    pos[b].0,
                    wb,
                    dd
                );
            }
            best = best.min(ya + wb + dd);
        }
    }
    best
}
