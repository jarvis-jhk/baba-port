//! A deterministic toy puzzle game for tests and `netsim`: small grid levels
//! with walls, pushable rocks, several YOU units and a flag; every turn moves
//! all YOU units (pushing rocks), undo and restart are turns too, touching
//! the flag clears the level. [`ToyReplica`] runs it in all three modes the
//! way baba-port's sim is meant to run under baba-net (docs/net.md):
//!
//! * mode 1: one level instance in the shared stream, every player's input
//!   moves it;
//! * mode 2: one instance per player stream, clears are shared events
//!   (the "save"); a player's next level is the first one not cleared in
//!   the save *as of the turn's `sync` point* plus their own clears;
//! * mode 3: one instance and one save per player.
//!
//! Integer only, no hash maps in logic, all randomness from the seed.

use crate::session::Replica;
use crate::wire::{Entry, EntryKind, Mode, PlayerId, SessionStart, StreamId};
use std::collections::BTreeMap;

pub const W: i16 = 10;
pub const H: i16 = 7;
/// Inputs: 0..=3 right/up/left/down, 4 wait, 5 undo, 6 restart.
pub const UNDO: u8 = 5;
pub const RESTART: u8 = 6;
const UNDO_DEPTH: usize = 512;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cell {
    Empty,
    Wall,
}

fn splitmix(x: &mut u64) -> u64 {
    *x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn fnv(h: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *h ^= *b as u64;
        *h = h.wrapping_mul(0x100_0000_01b3);
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct Units {
    you: Vec<(i16, i16)>,
    rocks: Vec<(i16, i16)>,
}

/// One level instance.
#[derive(Clone, Debug)]
pub struct ToyLevel {
    pub level: u16,
    grid: Vec<Cell>,
    flag: (i16, i16),
    start: Units,
    now: Units,
    undo: Vec<Units>,
    pub turns: u32,
    pub cleared: u32,
}

impl ToyLevel {
    pub fn new(seed: u64, level: u16) -> ToyLevel {
        let mut r = seed ^ (level as u64).wrapping_mul(0xA24B_AED4_963E_E407);
        let mut grid = vec![Cell::Empty; (W * H) as usize];
        let mut free = Vec::new();
        for y in 0..H {
            for x in 0..W {
                let border = x == 0 || y == 0 || x == W - 1 || y == H - 1;
                if border || splitmix(&mut r) % 100 < 8 {
                    grid[(y * W + x) as usize] = Cell::Wall;
                } else {
                    free.push((x, y));
                }
            }
        }
        let mut take = |r: &mut u64| {
            let i = (splitmix(r) % free.len() as u64) as usize;
            free.swap_remove(i)
        };
        let flag = take(&mut r);
        let you = vec![take(&mut r), take(&mut r)];
        let rocks = (0..4).map(|_| take(&mut r)).collect();
        let start = Units { you, rocks };
        ToyLevel {
            level,
            grid,
            flag,
            now: start.clone(),
            start,
            undo: Vec::new(),
            turns: 0,
            cleared: 0,
        }
    }

    fn wall(&self, (x, y): (i16, i16)) -> bool {
        x < 0 || y < 0 || x >= W || y >= H || self.grid[(y * W + x) as usize] == Cell::Wall
    }

    /// One turn. Returns true when the level was cleared by it.
    pub fn step(&mut self, input: u8) -> bool {
        self.turns += 1;
        match input {
            UNDO => {
                if let Some(u) = self.undo.pop() {
                    self.now = u;
                }
                return false;
            }
            RESTART => {
                self.undo.push(self.now.clone());
                self.now = self.start.clone();
                return false;
            }
            0..=3 => {}
            _ => return false,
        }
        let (dx, dy) = [(1, 0), (0, -1), (-1, 0), (0, 1)][input as usize];
        let before = self.now.clone();
        // YOU units move in a fixed order (index order); each pushes a chain
        // of rocks if the cell after the chain is free.
        for i in 0..self.now.you.len() {
            let (x, y) = self.now.you[i];
            let mut chain = Vec::new();
            let mut p = (x + dx, y + dy);
            while let Some(k) = self.now.rocks.iter().position(|r| *r == p) {
                chain.push(k);
                p = (p.0 + dx, p.1 + dy);
            }
            if self.wall(p) {
                continue;
            }
            for k in chain {
                let r = &mut self.now.rocks[k];
                *r = (r.0 + dx, r.1 + dy);
            }
            self.now.you[i] = (x + dx, y + dy);
        }
        if self.now != before {
            self.undo.push(before);
            if self.undo.len() > UNDO_DEPTH {
                self.undo.remove(0);
            }
        }
        self.now.you.contains(&self.flag)
    }

    pub fn hash_into(&self, h: &mut u64) {
        fnv(h, &self.level.to_le_bytes());
        fnv(h, &self.turns.to_le_bytes());
        fnv(h, &self.cleared.to_le_bytes());
        fnv(h, &(self.undo.len() as u32).to_le_bytes());
        for (x, y) in self.now.you.iter().chain(self.now.rocks.iter()) {
            fnv(h, &x.to_le_bytes());
            fnv(h, &y.to_le_bytes());
        }
    }
}

/// The first level not in `cleared`.
fn next_level(cleared: &[u16]) -> u16 {
    (0u16..).find(|l| !cleared.contains(l)).unwrap()
}

#[derive(Clone, Debug)]
pub struct ToyPlayer {
    pub instance: ToyLevel,
    /// Mode 3: own save. Mode 2: own clears not yet seen in the save.
    pub own_clears: Vec<u16>,
    pub active: bool,
}

/// The toy game as a [`Replica`] of a whole session.
#[derive(Clone, Debug)]
pub struct ToyReplica {
    pub mode: Mode,
    pub seed: u64,
    /// Mode 1: the one level.
    pub coop: ToyLevel,
    /// Mode 2: the shared save, as (shared entry index, level) in order.
    pub save: Vec<(u32, u16)>,
    pub players: BTreeMap<PlayerId, ToyPlayer>,
    /// Clears that happened on this replica (player, level), for the owner
    /// to submit as shared events (mode 2); see [`take_clears`](Self::take_clears).
    clears: Vec<(PlayerId, u16)>,
    roster_hash: u64,
}

impl ToyReplica {
    pub fn new(start: &SessionStart) -> ToyReplica {
        ToyReplica {
            mode: start.mode,
            seed: start.seed,
            coop: ToyLevel::new(start.seed, 0),
            save: Vec::new(),
            players: BTreeMap::new(),
            clears: Vec::new(),
            roster_hash: 0xcbf2_9ce4_8422_2325,
        }
    }

    /// Clears since the last call (every replica sees them; only the owner
    /// of the player turns them into shared events).
    pub fn take_clears(&mut self) -> Vec<(PlayerId, u16)> {
        std::mem::take(&mut self.clears)
    }

    /// Levels cleared in the save as of shared entry `sync`.
    fn save_at(&self, sync: u32) -> Vec<u16> {
        self.save
            .iter()
            .filter(|(i, _)| *i < sync)
            .map(|(_, l)| *l)
            .collect()
    }

    /// Break the replica on purpose (desync tests).
    pub fn corrupt(&mut self, stream: StreamId) {
        match stream {
            StreamId::Shared => match self.mode {
                Mode::Coop => self.coop.turns += 1000,
                _ => self.roster_hash ^= 1,
            },
            StreamId::Player(p) => {
                if let Some(pl) = self.players.get_mut(&p) {
                    pl.instance.turns += 1000;
                }
            }
        }
    }
}

impl Replica for ToyReplica {
    fn apply(&mut self, stream: StreamId, index: u32, e: &Entry) {
        match (stream, e.kind) {
            (StreamId::Shared, EntryKind::Joined) => {
                fnv(&mut self.roster_hash, &[1, e.player.0]);
                let level = match self.mode {
                    Mode::SharedSave => next_level(&self.save_at(index)),
                    _ => 0,
                };
                self.players.insert(
                    e.player,
                    ToyPlayer {
                        instance: ToyLevel::new(self.seed, level),
                        own_clears: Vec::new(),
                        active: true,
                    },
                );
            }
            (StreamId::Shared, EntryKind::Left) => {
                fnv(&mut self.roster_hash, &[2, e.player.0]);
                if let Some(p) = self.players.get_mut(&e.player) {
                    p.active = false;
                }
            }
            (StreamId::Shared, EntryKind::Input) => {
                if e.data.len() == 1 && self.coop.step(e.data[0]) {
                    self.coop.cleared += 1;
                    let next = self.coop.level + 1;
                    let cleared = self.coop.cleared;
                    self.coop = ToyLevel::new(self.seed, next);
                    self.coop.cleared = cleared;
                    self.clears.push((e.player, next - 1));
                }
            }
            (StreamId::Shared, EntryKind::Event) => {
                if self.mode == Mode::SharedSave && e.data.len() == 2 {
                    let l = u16::from_le_bytes([e.data[0], e.data[1]]);
                    if !self.save.iter().any(|(_, x)| *x == l) {
                        self.save.push((index, l));
                    }
                }
            }
            (StreamId::Player(p), EntryKind::Input) => {
                let mode = self.mode;
                let save = match mode {
                    Mode::SharedSave => self.save_at(e.sync),
                    _ => Vec::new(),
                };
                let Some(pl) = self.players.get_mut(&p) else {
                    return;
                };
                if e.data.len() != 1 || !pl.instance.step(e.data[0]) {
                    return;
                }
                let done = pl.instance.level;
                let cleared = pl.instance.cleared + 1;
                if !pl.own_clears.contains(&done) {
                    pl.own_clears.push(done);
                }
                let mut known = save;
                known.extend(pl.own_clears.iter().copied());
                pl.instance = ToyLevel::new(self.seed, next_level(&known));
                pl.instance.cleared = cleared;
                self.clears.push((p, done));
            }
            _ => {}
        }
    }

    fn state_hash(&mut self, stream: StreamId) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        match stream {
            StreamId::Shared => {
                fnv(&mut h, &self.roster_hash.to_le_bytes());
                match self.mode {
                    Mode::Coop => self.coop.hash_into(&mut h),
                    Mode::SharedSave => {
                        for (i, l) in &self.save {
                            fnv(&mut h, &i.to_le_bytes());
                            fnv(&mut h, &l.to_le_bytes());
                        }
                    }
                    Mode::OwnGames => {}
                }
            }
            StreamId::Player(p) => {
                if let Some(pl) = self.players.get(&p) {
                    pl.instance.hash_into(&mut h);
                    for l in &pl.own_clears {
                        fnv(&mut h, &l.to_le_bytes());
                    }
                }
            }
        }
        h
    }
}

/// A bot: random moves held for a few turns, sometimes undo or restart.
#[derive(Clone, Debug)]
pub struct ToyBot {
    rng: u64,
    dir: u8,
    left: u32,
}

impl ToyBot {
    pub fn new(seed: u64) -> ToyBot {
        ToyBot {
            rng: seed | 1,
            dir: 0,
            left: 0,
        }
    }

    fn next(&mut self) -> u64 {
        splitmix(&mut self.rng)
    }

    pub fn input(&mut self) -> u8 {
        let r = self.next() % 100;
        if r < 4 {
            return UNDO;
        }
        if r == 4 {
            return RESTART;
        }
        if self.left == 0 {
            self.dir = (self.next() % 4) as u8;
            self.left = 1 + (self.next() % 4) as u32;
        }
        self.left -= 1;
        self.dir
    }

    /// Milliseconds until the next input (a fast human: 4–12 turns/s).
    pub fn delay_ms(&mut self) -> u64 {
        80 + self.next() % 170
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_are_deterministic_and_clearable() {
        let a = ToyLevel::new(7, 3);
        let b = ToyLevel::new(7, 3);
        let (mut ha, mut hb) = (0u64, 0u64);
        a.hash_into(&mut ha);
        b.hash_into(&mut hb);
        assert_eq!(ha, hb);
        // A bot clears some levels within a few thousand turns.
        let mut l = ToyLevel::new(1, 0);
        let mut bot = ToyBot::new(5);
        let mut clears = 0;
        for _ in 0..20000 {
            if l.step(bot.input()) {
                clears += 1;
                l = ToyLevel::new(1, l.level + 1);
            }
        }
        assert!(clears >= 3, "{clears} clears");
    }

    #[test]
    fn undo_restores() {
        let mut l = ToyLevel::new(3, 0);
        let start = l.now.clone();
        for d in [0, 0, 1, 2, 3, 3] {
            l.step(d);
        }
        for _ in 0..6 {
            l.step(UNDO);
        }
        assert_eq!(l.now, start);
    }
}
