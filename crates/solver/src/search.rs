//! Graph search over sim states: BFS, greedy best-first and best-first
//! width search (novelty + heuristic), with state-key duplicate detection.
//!
//! Expansion uses macro moves where the walking model applies
//! ([`crate::state::walk_model`]): the YOU unit's walkable region is flood
//! filled without the sim, and only the moves that push or touch something
//! are simulated (after replaying the walk to their start cell through the
//! sim, so every state is a real one). Elsewhere a node expands into the
//! five single inputs.
//!
//! Nodes keep their `Level` only while waiting in the open list (within a
//! memory budget); expanded nodes keep one every few plies as a checkpoint,
//! and a node without a level is rebuilt by replaying moves from its nearest
//! ancestor that has one.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
use std::hash::{BuildHasherDefault, Hasher};
use std::time::{Duration, Instant};

use baba_sim::{Input, Level};

use crate::state::{Goal, KeyOpts, atoms, heuristic, holds, key, key_without, walk_model};

/// Identity hasher for keys that already are hashes.
#[derive(Default)]
pub struct IdHash(u64);
impl Hasher for IdHash {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, b: &[u8]) {
        for &x in b {
            self.0 = (self.0 << 8) ^ x as u64;
        }
    }
    fn write_u64(&mut self, v: u64) {
        self.0 = v;
    }
}
type IdSet = HashSet<u64, BuildHasherDefault<IdHash>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategy {
    Bfs,
    Greedy,
    Bfws,
}

impl Strategy {
    pub fn parse(s: &str) -> Option<Strategy> {
        Some(match s {
            "bfs" => Strategy::Bfs,
            "greedy" => Strategy::Greedy,
            "bfws" => Strategy::Bfws,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Strategy::Bfs => "bfs",
            Strategy::Greedy => "greedy",
            Strategy::Bfws => "bfws",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Limits {
    pub time: Duration,
    pub nodes: usize,
    pub mem_bytes: usize,
    /// Try the wait input too.
    pub wait: bool,
    /// Use macro moves where the walking model applies.
    pub macros: bool,
    /// Sub-goals only count while some player exists.
    pub alive: bool,
    pub key: KeyOpts,
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub expanded: usize,
    pub macro_expanded: usize,
    pub generated: usize,
    pub steps: usize,
    pub secs: f64,
    pub stop: &'static str,
    /// Lowest heuristic value generated and the moves to it.
    pub best_h: i32,
    pub best: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct Found {
    /// Moves 0..3 directions, 4 wait.
    pub moves: Vec<u8>,
    /// The level was won (not only the sub-goals reached).
    pub won: bool,
}

struct Node {
    parent: u32,
    /// Moves from the parent (one input, or a walk plus a push).
    mv: Vec<u8>,
    depth: u32,
    noplayer: bool,
    level: Option<Box<Level>>,
}

pub fn input(m: u8) -> Input {
    if m == 4 { Input::wait() } else { Input::dir(m) }
}

fn est_bytes(l: &Level) -> usize {
    l.unit_records().len() * 200 + (l.width() * l.height()) as usize * 40 + 8192
}

fn path(nodes: &[Node], mut id: u32) -> Vec<u8> {
    let mut parts = Vec::new();
    while id != 0 {
        parts.push(&nodes[id as usize].mv);
        id = nodes[id as usize].parent;
    }
    parts.iter().rev().flat_map(|v| v.iter().copied()).collect()
}

/// One successor of an expansion.
struct Child {
    moves: Vec<u8>,
    level: Level,
    won: bool,
    noplayer: bool,
}

/// What an expansion produced.
enum Expansion {
    Children(Vec<Child>),
    /// The node's canonical (macro) key was seen already.
    Duplicate,
}

const DIRS: [(i32, i32); 4] = [(1, 0), (0, -1), (-1, 0), (0, 1)];

/// Macro expansion: flood fill the walker's region, then simulate the walk to
/// every cell next to a blocked/occupied cell and the move into it.
fn expand_macro(
    level: &Level,
    lim: &Limits,
    canon: &mut IdSet,
    st: &mut Stats,
) -> Option<Expansion> {
    let wm = walk_model(level)?;
    let w = wm.w;
    let idx = |p: (i32, i32)| (p.1 * w + p.0) as usize;
    let n = wm.free.len();
    if wm.pos.0 < 0 || wm.pos.1 < 0 || idx(wm.pos) >= n {
        return None;
    }
    // BFS over free cells.
    let mut parent: Vec<i32> = vec![-2; n];
    let mut pdir: Vec<u8> = vec![0; n];
    let mut order = vec![wm.pos];
    parent[idx(wm.pos)] = -1;
    let mut i = 0;
    while i < order.len() {
        let p = order[i];
        i += 1;
        for (d, (dx, dy)) in DIRS.iter().enumerate() {
            let q = (p.0 + dx, p.1 + dy);
            let qi = idx(q);
            if wm.free[qi] && parent[qi] == -2 {
                parent[qi] = idx(p) as i32;
                pdir[qi] = d as u8;
                order.push(q);
            }
        }
    }
    // Standing on something that is not inert: the exact cell matters.
    let region_min = if wm.free[idx(wm.pos)] {
        order.iter().map(|&p| idx(p)).min().unwrap_or(0)
    } else {
        n + idx(wm.pos)
    };
    let mut m = crate::state::Mix::new();
    m.add(key_without(level, wm.you, &lim.key));
    m.add(region_min as u64);
    if !canon.insert(m.finish()) {
        return Some(Expansion::Duplicate);
    }
    st.macro_expanded += 1;
    let base_key = key_without(level, wm.you, &lim.key);
    // Levels at region cells, materialised on demand along the BFS tree.
    let mut at: HashMap<usize, Option<Level>> = HashMap::new();
    at.insert(idx(wm.pos), Some(level.clone()));
    let mut out = Vec::new();
    let walk_path = |c: usize| -> Vec<u8> {
        let mut v = Vec::new();
        let mut c = c;
        while parent[c] >= 0 {
            v.push(pdir[c]);
            c = parent[c] as usize;
        }
        v.reverse();
        v
    };
    for &p in &order {
        for (d, (dx, dy)) in DIRS.iter().enumerate() {
            let q = (p.0 + dx, p.1 + dy);
            if wm.free[idx(q)] || wm.wall[idx(q)] {
                continue;
            }
            // A row of push-only things against a blocker does not move.
            if wm.pushy[idx(q)] {
                let mut r = q;
                while wm.pushy[idx(r)] {
                    r = (r.0 + dx, r.1 + dy);
                }
                if wm.block[idx(r)] {
                    continue;
                }
            }
            // Materialise the level at p.
            let pi = idx(p);
            if !at.contains_key(&pi) {
                let mut chain = vec![pi];
                let mut c = pi;
                while !at.contains_key(&(parent[c] as usize)) {
                    c = parent[c] as usize;
                    chain.push(c);
                }
                for &c in chain.iter().rev() {
                    let prev = at.get(&(parent[c] as usize)).cloned().flatten();
                    let r = prev.map(|mut l| {
                        let o = l.step(input(pdir[c]));
                        st.steps += 1;
                        (l, o)
                    });
                    let v = match r {
                        Some((l, o)) => {
                            let deviated = o.won
                                || o.ended
                                || o.all_done
                                || key_without(&l, wm.you, &lim.key) != base_key;
                            if deviated {
                                // The walk itself changed something: a successor.
                                out.push(Child {
                                    moves: walk_path(c),
                                    won: o.won || o.ended || o.all_done,
                                    noplayer: o.no_player,
                                    level: l,
                                });
                                None
                            } else {
                                Some(l)
                            }
                        }
                        None => None,
                    };
                    at.insert(c, v);
                }
            }
            let Some(Some(lp)) = at.get(&pi) else {
                continue;
            };
            let mut c = lp.clone();
            let o = c.step(input(d as u8));
            st.steps += 1;
            let won = o.won || o.ended || o.all_done;
            if !won && key_without(&c, wm.you, &lim.key) == base_key {
                // Nothing else changed: a bump (walker did not move) is
                // useless; stepping onto an occupied cell is a real move.
                let moved = c
                    .unit_records()
                    .iter()
                    .any(|u| u.handle == wm.you && (u.x, u.y) == q);
                if !moved {
                    continue;
                }
            }
            let mut mv = walk_path(pi);
            mv.push(d as u8);
            out.push(Child {
                moves: mv,
                won: o.won || o.ended || o.all_done,
                noplayer: o.no_player,
                level: c,
            });
        }
    }
    if debug() {
        eprintln!(
            "macro: you at {:?} region {} children {}",
            wm.pos,
            order.len(),
            out.len()
        );
    }
    Some(Expansion::Children(out))
}

fn debug() -> bool {
    static D: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *D.get_or_init(|| std::env::var_os("SOLVER_DEBUG").is_some())
}

fn expand_plain(level: &Level, noplayer: bool, lim: &Limits, st: &mut Stats) -> Expansion {
    let ms: &[u8] = if noplayer {
        &[4]
    } else if lim.wait {
        &[0, 1, 2, 3, 4]
    } else {
        &[0, 1, 2, 3]
    };
    let mut out = Vec::new();
    for &m in ms {
        let mut c = level.clone();
        let o = c.step(input(m));
        st.steps += 1;
        out.push(Child {
            moves: vec![m],
            won: o.won || o.ended || o.all_done,
            noplayer: o.no_player,
            level: c,
        });
    }
    Expansion::Children(out)
}

/// Searches from `root` for a state satisfying all `goals`.
pub fn search(
    root: &Level,
    goals: &[Goal],
    strat: Strategy,
    lim: &Limits,
) -> (Option<Found>, Stats) {
    let t0 = Instant::now();
    let mut st = Stats {
        best_h: i32::MAX,
        ..Stats::default()
    };
    let need_win = goals.iter().any(|g| matches!(g, Goal::Win));
    let others: Vec<Goal> = goals
        .iter()
        .filter(|g| !matches!(g, Goal::Win))
        .cloned()
        .collect();
    let per = est_bytes(root);
    let cap_levels = (lim.mem_bytes / per).max(64);
    let mut stored = 1usize;

    let mut nodes: Vec<Node> = vec![Node {
        parent: 0,
        mv: vec![],
        depth: 0,
        noplayer: root.no_player(),
        level: Some(Box::new(root.clone())),
    }];
    let mut seen = IdSet::default();
    let mut canon = IdSet::default();
    seen.insert(key(root, &lim.key));
    if !need_win && !others.is_empty() && others.iter().all(|g| holds(root, g)) {
        return (
            Some(Found {
                moves: vec![],
                won: false,
            }),
            st,
        );
    }

    let mut fifo: VecDeque<u32> = VecDeque::new();
    let mut heap: BinaryHeap<Reverse<(u32, i32, u32, u32)>> = BinaryHeap::new();
    let mut novelty: HashMap<i32, IdSet> = HashMap::new();
    let mut abuf = Vec::new();
    let mut hbuf = Vec::new();
    match strat {
        Strategy::Bfs => fifo.push_back(0),
        _ => heap.push(Reverse((0, 0, 0, 0))),
    }

    loop {
        let id = match strat {
            Strategy::Bfs => fifo.pop_front(),
            _ => heap.pop().map(|Reverse((_, _, _, i))| i),
        };
        let Some(id) = id else {
            st.stop = "exhausted";
            break;
        };
        if st.expanded.is_multiple_of(16) {
            if t0.elapsed() > lim.time {
                st.stop = "time";
                break;
            }
            if nodes.len() >= lim.nodes {
                st.stop = "nodes";
                break;
            }
        }
        st.expanded += 1;
        // Materialise the node's level.
        let level: Level = match nodes[id as usize].level.take() {
            Some(l) => {
                stored -= 1;
                *l
            }
            None => {
                let mut chain = Vec::new();
                let mut cur = id;
                while nodes[cur as usize].level.is_none() {
                    chain.push(cur);
                    cur = nodes[cur as usize].parent;
                }
                let mut l = (**nodes[cur as usize].level.as_ref().unwrap()).clone();
                for &c in chain.iter().rev() {
                    for &m in &nodes[c as usize].mv {
                        l.step(input(m));
                        st.steps += 1;
                    }
                }
                l
            }
        };
        let (depth, noplayer) = (nodes[id as usize].depth, nodes[id as usize].noplayer);
        let exp = if lim.macros && !noplayer {
            expand_macro(&level, lim, &mut canon, &mut st)
        } else {
            None
        };
        let exp = exp.unwrap_or_else(|| expand_plain(&level, noplayer, lim, &mut st));
        let children = match exp {
            Expansion::Duplicate => continue,
            Expansion::Children(c) => c,
        };
        for ch in children {
            st.generated += 1;
            let c = ch.level;
            if ch.won
                || (!need_win
                    && (!lim.alive || !c.no_player())
                    && others.iter().all(|g| holds(&c, g)))
            {
                let mut p = path(&nodes, id);
                p.extend(ch.moves);
                st.secs = t0.elapsed().as_secs_f64();
                st.stop = "found";
                return (
                    Some(Found {
                        moves: p,
                        won: ch.won,
                    }),
                    st,
                );
            }
            if c.destroyed() {
                continue;
            }
            if !seen.insert(key(&c, &lim.key)) {
                continue;
            }
            let cid = nodes.len() as u32;
            let prio = match strat {
                Strategy::Bfs => (0, 0),
                Strategy::Greedy => (0, heuristic(&c, goals, &mut hbuf)),
                Strategy::Bfws => {
                    let h = heuristic(&c, goals, &mut hbuf);
                    atoms(&c, &mut abuf);
                    let tab = novelty.entry(h).or_default();
                    let mut novel = false;
                    for &a in &abuf {
                        novel |= tab.insert(a);
                    }
                    (if novel { 0 } else { 1 }, h)
                }
            };
            if strat != Strategy::Bfs && prio.1 < st.best_h {
                st.best_h = prio.1;
                st.best = path(&nodes, id);
                st.best.extend(&ch.moves);
            }
            let keep = stored < cap_levels;
            if keep {
                stored += 1;
            }
            nodes.push(Node {
                parent: id,
                mv: ch.moves,
                depth: depth + 1,
                noplayer: ch.noplayer,
                level: keep.then(|| Box::new(c)),
            });
            match strat {
                Strategy::Bfs => fifo.push_back(cid),
                _ => heap.push(Reverse((prio.0, prio.1, depth + 1, cid))),
            }
        }
        // Keep a checkpoint every few plies (and always the root).
        if id == 0 || (depth % 4 == 0 && stored < cap_levels) {
            nodes[id as usize].level = Some(Box::new(level));
            stored += 1;
        }
    }
    st.secs = t0.elapsed().as_secs_f64();
    (None, st)
}
