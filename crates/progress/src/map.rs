//! Map progression data: icon and path states on one map, saved-state
//! overrides, re-propagation, gates and the unlock flood fill
//! (`docs/spec/progression.md` §3.2, §4, §5.1). Pure data; the game turns
//! the result into units and animations.

use std::collections::VecDeque;

use baba_formats::LevelInfo;

use crate::slot::Slot;

/// An icon (a unit with a target level) on the map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Icon {
    pub x: i32,
    pub y: i32,
    pub target: String,
    /// 0 hidden, 1 visible, 2 open, 3 completed.
    pub state: i32,
}

/// A `[paths]` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Path {
    /// Index in `[paths]`.
    pub index: usize,
    pub x: i32,
    pub y: i32,
    /// 0 none, 1 spores, 2 flowers, 3 orbs, 4 local spores.
    pub gate: i32,
    pub requirement: i32,
    /// 0 hidden; 1 revealed closed gate; 2+ visible; 3 revealed path /
    /// opened gate.
    pub state: i32,
}

impl Path {
    /// A gate that has not been opened.
    pub fn closed_gate(&self) -> bool {
        self.gate != 0 && self.state < 3
    }
}

/// What changed at one tile during an unlock (for the animation).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Icon {
        target: String,
        x: i32,
        y: i32,
        from: i32,
        to: i32,
    },
    Path {
        index: usize,
        x: i32,
        y: i32,
        from: i32,
        to: i32,
    },
}

/// The result of completing a level on a map.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Unlock {
    /// Neighbours opened directly (§5.1 step 2), in right/up/left/down
    /// order of discovery.
    pub opened: Vec<Change>,
    /// Hidden things revealed by the flood fill, one ring per BFS depth.
    pub rings: Vec<Vec<Change>>,
}

/// Counter values gates compare against (§4.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub prize: i64,
    pub clears: i64,
    pub bonus: i64,
    pub local_prize: i64,
}

impl Counters {
    pub fn read(slot: &Slot, world: &str, map: &str) -> Self {
        Self {
            prize: slot.prize_total(world),
            clears: slot.clears_total(world),
            bonus: slot.bonus_total(world),
            local_prize: slot.prize(world, map),
        }
    }

    pub fn for_gate(&self, gate: i32) -> i64 {
        match gate {
            1 => self.prize,
            2 => self.clears,
            3 => self.bonus,
            4 => self.local_prize,
            _ => i64::MAX,
        }
    }
}

/// Icon and path states of one map.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapLayout {
    pub map: String,
    pub icons: Vec<Icon>,
    pub paths: Vec<Path>,
}

const DIRS: [(i32, i32); 4] = [(1, 0), (0, -1), (-1, 0), (0, 1)];

impl MapLayout {
    /// From the map's `.ld` (initial states) plus extra icons made from
    /// objects by `[specials]` (`(x, y, target, state)`).
    pub fn from_info(map: &str, info: &LevelInfo, extra: Vec<Icon>) -> Self {
        let mut icons: Vec<Icon> = info
            .levels
            .iter()
            .filter(|l| !l.file.is_empty())
            .map(|l| Icon {
                x: l.x,
                y: l.y,
                target: l.file.clone(),
                state: l.state.clamp(0, 3),
            })
            .collect();
        icons.extend(extra);
        let paths = info
            .paths
            .iter()
            .enumerate()
            .map(|(i, p)| Path {
                index: i,
                x: p.x,
                y: p.y,
                gate: p.gate,
                requirement: p.requirement,
                state: if p.style == 1 { 2 } else { 0 },
            })
            .collect();
        Self {
            map: map.to_string(),
            icons,
            paths,
        }
    }

    /// §3.2 step 4: saved states override (icons: saved > 0 replaces;
    /// paths: the larger of initial and saved).
    pub fn apply_save(&mut self, slot: &Slot, world: &str) {
        for i in &mut self.icons {
            let s = slot.level_state(world, &i.target);
            if s > 0 {
                i.state = s.min(3);
            }
        }
        for p in &mut self.paths {
            let s = slot.path_state(world, &self.map, p.x, p.y);
            p.state = p.state.max(s.min(3));
        }
    }

    /// §3.2 step 5: every completed icon (and opened gate) re-runs the
    /// opening logic. Unlike a win, this applies the new states at once,
    /// so chains already opened are not revealed again.
    pub fn repropagate(&mut self) {
        let starts: Vec<(i32, i32)> = self
            .icons
            .iter()
            .filter(|i| i.state >= 3)
            .map(|i| (i.x, i.y))
            .chain(
                self.paths
                    .iter()
                    .filter(|p| p.gate != 0 && p.state >= 3)
                    .map(|p| (p.x, p.y)),
            )
            .collect();
        for (x, y) in starts {
            let mut sink = Vec::new();
            self.unlock_around(x, y, true, &mut sink);
            self.flood(x, y, true, &mut Vec::new());
        }
    }

    fn icons_at(&self, x: i32, y: i32) -> Vec<usize> {
        (0..self.icons.len())
            .filter(|&k| self.icons[k].x == x && self.icons[k].y == y)
            .collect()
    }

    fn paths_at(&self, x: i32, y: i32) -> Vec<usize> {
        (0..self.paths.len())
            .filter(|&k| self.paths[k].x == x && self.paths[k].y == y)
            .collect()
    }

    fn icon_change(&self, k: usize, to: i32) -> Change {
        let i = &self.icons[k];
        Change::Icon {
            target: i.target.clone(),
            x: i.x,
            y: i.y,
            from: i.state,
            to,
        }
    }

    fn path_change(&self, k: usize, to: i32) -> Change {
        let p = &self.paths[k];
        Change::Path {
            index: p.index,
            x: p.x,
            y: p.y,
            from: p.state,
            to,
        }
    }

    /// Applies a change, keeping the larger state (saves only ever grow).
    /// Every icon with the same target shares the state.
    pub fn apply(&mut self, c: &Change) {
        match c {
            Change::Icon { target, to, .. } => {
                for i in &mut self.icons {
                    if i.target == *target {
                        i.state = i.state.max(*to);
                    }
                }
            }
            Change::Path { index, to, .. } => {
                for p in &mut self.paths {
                    if p.index == *index {
                        p.state = p.state.max(*to);
                    }
                }
            }
        }
    }

    /// §5.1 step 2 around (x, y): icons below "open" become open, paths
    /// become revealed (gates: revealed closed). With `immediate` the
    /// states change at once; otherwise they are only reported (the
    /// original writes the save and leaves the units for the animation).
    fn unlock_around(&mut self, x: i32, y: i32, immediate: bool, out: &mut Vec<Change>) {
        for (dx, dy) in DIRS {
            let (nx, ny) = (x + dx, y + dy);
            for k in self.icons_at(nx, ny) {
                if self.icons[k].state < 2 {
                    let c = self.icon_change(k, 2);
                    if immediate {
                        self.apply(&c);
                    }
                    out.push(c);
                }
            }
            for k in self.paths_at(nx, ny) {
                if self.paths[k].state < 2 {
                    let to = if self.paths[k].gate == 0 { 3 } else { 1 };
                    let c = self.path_change(k, to);
                    if immediate {
                        let i = self.paths[k].index;
                        for p in &mut self.paths {
                            if p.index == i {
                                // In memory a revealed path is "open" (2),
                                // a gate stays closed (1).
                                p.state = p.state.max(if p.gate == 0 { 2 } else { 1 });
                            }
                        }
                    }
                    out.push(c);
                }
            }
        }
    }

    /// §5.1 step 3: the hidden-chain fill from (x, y). Tiles are visited
    /// in discovery order; hidden things (state 0 in the current states)
    /// next to the visited tile are queued with the kind of the tile they
    /// were found from (start, icon or path), except next to a closed gate
    /// other than the start. A queued icon is revealed only if found from
    /// the start or an icon tile, a queued path only from the start or a
    /// path tile; revealed things become the next visited tile, and a
    /// revealed path also unlocks around itself (step 2). Returns the
    /// changes grouped by discovery depth (rings for the animation).
    fn flood(&mut self, x: i32, y: i32, immediate: bool, rings: &mut Vec<Vec<Change>>) {
        #[derive(Clone, Copy, PartialEq)]
        enum Kind {
            Start,
            Icon,
            Path,
        }
        enum Item {
            Icon(usize),
            Path(usize),
        }
        let mut done: Vec<(i32, i32)> = Vec::new();
        let mut queue: VecDeque<(Item, Kind, usize)> = VecDeque::new();
        let mut cur = (x, y, 0usize);
        loop {
            let (cx, cy, depth) = cur;
            if !done.contains(&(cx, cy)) {
                let start = (cx, cy) == (x, y);
                let mut kind = Kind::Icon;
                let mut gate_here = false;
                if !self.icons_at(cx, cy).is_empty() {
                    kind = Kind::Icon;
                }
                for k in self.paths_at(cx, cy) {
                    kind = Kind::Path;
                    if self.paths[k].gate > 0 && !start {
                        gate_here = true;
                    }
                }
                if start {
                    kind = Kind::Start;
                }
                if !gate_here {
                    for (dx, dy) in DIRS {
                        let (nx, ny) = (cx + dx, cy + dy);
                        for k in self.icons_at(nx, ny) {
                            if self.icons[k].state == 0 {
                                queue.push_back((Item::Icon(k), kind, depth + 1));
                            }
                        }
                        for k in self.paths_at(nx, ny) {
                            if self.paths[k].state == 0 {
                                queue.push_back((Item::Path(k), kind, depth + 1));
                            }
                        }
                    }
                }
                done.push((cx, cy));
            }
            let Some((item, from, d)) = queue.pop_front() else {
                break;
            };
            let mut changes = Vec::new();
            match item {
                Item::Icon(k) => {
                    if matches!(from, Kind::Icon | Kind::Start) {
                        let c = self.icon_change(k, 1);
                        if immediate {
                            self.apply(&c);
                        }
                        changes.push(c);
                        cur = (self.icons[k].x, self.icons[k].y, d);
                    }
                }
                Item::Path(k) => {
                    if matches!(from, Kind::Path | Kind::Start) {
                        let to = if self.paths[k].gate == 0 { 3 } else { 1 };
                        let c = self.path_change(k, to);
                        if immediate {
                            let i = self.paths[k].index;
                            for p in &mut self.paths {
                                if p.index == i {
                                    p.state = p.state.max(1);
                                }
                            }
                        }
                        changes.push(c);
                        let (px, py) = (self.paths[k].x, self.paths[k].y);
                        cur = (px, py, d);
                        self.unlock_around(px, py, immediate, &mut changes);
                    }
                }
            }
            if !changes.is_empty() {
                let ring = d.saturating_sub(1);
                while rings.len() <= ring {
                    rings.push(Vec::new());
                }
                rings[ring].extend(changes);
            }
        }
    }

    /// §5.1 steps 1–3: the level `target` at (x, y) was won. The fill runs
    /// on the states from before the win (as the original, which writes
    /// the save first and changes the units during the animation); the
    /// combined result is applied to the layout before returning.
    pub fn complete(&mut self, target: &str, x: i32, y: i32) -> Unlock {
        let mut opened = Vec::new();
        self.unlock_around(x, y, false, &mut opened);
        let mut rings = Vec::new();
        self.flood(x, y, false, &mut rings);
        if let Some(k) = self.icons.iter().position(|i| i.target == target) {
            let c = self.icon_change(k, 3);
            self.apply(&c);
        }
        for c in opened.iter().chain(rings.iter().flatten()) {
            self.apply(c);
        }
        Unlock { opened, rings }
    }

    /// The unlock of a win without the icon itself (an opened gate,
    /// §5.4): steps 2–3 from (x, y), applied before returning.
    pub fn unlock_from(&mut self, x: i32, y: i32) -> Unlock {
        let mut opened = Vec::new();
        self.unlock_around(x, y, false, &mut opened);
        let mut rings = Vec::new();
        self.flood(x, y, false, &mut rings);
        for c in opened.iter().chain(rings.iter().flatten()) {
            self.apply(c);
        }
        Unlock { opened, rings }
    }

    /// Gates whose counter meets the requirement (§4.2): `(index, newly)`;
    /// `newly` gates were never opened before and get the animation.
    /// Their state becomes 3.
    pub fn open_gates(&mut self, counters: &Counters) -> Vec<(usize, bool)> {
        let mut out = Vec::new();
        for p in &mut self.paths {
            if p.gate != 0 && p.state > 0 && counters.for_gate(p.gate) >= p.requirement as i64 {
                out.push((p.index, p.state < 3));
                p.state = 3;
            }
        }
        out
    }

    /// Writes icon and path states into the slot (§5.1 "written to the
    /// save immediately").
    pub fn write(&self, slot: &mut Slot, world: &str) {
        for i in &self.icons {
            if i.state > slot.level_state(world, &i.target) {
                slot.set_level_state(world, &i.target, i.state);
            }
        }
        for p in &self.paths {
            let v = match (p.gate, p.state) {
                (0, s) if s >= 2 => 3,
                (_, 3) => 3,
                (_, 1) => 1,
                _ => 0,
            };
            if v > slot.path_state(world, &self.map, p.x, p.y) {
                slot.set_path_state(world, &self.map, p.x, p.y, v);
            }
        }
    }

    /// Number of completed levels among this map's icons (distinct
    /// targets).
    pub fn completed_count(&self) -> usize {
        let mut seen: Vec<&str> = Vec::new();
        for i in &self.icons {
            if i.state >= 3 && !seen.contains(&i.target.as_str()) {
                seen.push(&i.target);
            }
        }
        seen.len()
    }

    /// State shown for the icon targeting `stem`.
    pub fn icon_state(&self, stem: &str) -> Option<i32> {
        self.icons
            .iter()
            .find(|i| i.target == stem)
            .map(|i| i.state)
    }

    /// Path state by `[paths]` index.
    pub fn path_state(&self, index: usize) -> i32 {
        self.paths
            .iter()
            .find(|p| p.index == index)
            .map_or(0, |p| p.state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> MapLayout {
        // A row: icon a at (1,1), path (2,1), path (3,1), icon b (4,1),
        // gate (5,1) needing 2 spores, icon c (6,1); icon d below a (1,2).
        let ld = LevelInfo::parse(
            "[general]\nlevels=4\npaths=3\nleveltype=1\n\
             [levels]\n0file=a\n0X=1\n0Y=1\n0state=2\n\
             1file=b\n1X=4\n1Y=1\n1state=0\n\
             2file=c\n2X=6\n2Y=1\n2state=0\n\
             3file=d\n3X=1\n3Y=2\n3state=0\n\
             [paths]\n0X=2\n0Y=1\n0object=object117\n\
             1X=3\n1Y=1\n1object=object117\n\
             2X=5\n2Y=1\n2gate=1\n2requirement=2\n2object=object117\n",
        );
        MapLayout::from_info("m", &ld, Vec::new())
    }

    #[test]
    fn completing_opens_neighbours_and_reveals_chain() {
        let mut m = layout();
        let u = m.complete("a", 1, 1);
        assert_eq!(m.icon_state("a"), Some(3));
        // d is directly below: opened (from hidden).
        assert_eq!(m.icon_state("d"), Some(2));
        // path (2,1) next to a: opened directly to 3.
        assert_eq!(m.path_state(0), 3);
        // the reveal continues along the path: (3,1) revealed, b beside
        // it opened by step 2 around the revealed path.
        assert_eq!(m.path_state(1), 3);
        assert_eq!(m.icon_state("b"), Some(2));
        // the gate is not adjacent to a revealed path: it stays hidden;
        // c is behind it.
        assert_eq!(m.path_state(2), 0);
        assert_eq!(m.icon_state("c"), Some(0));
        assert!(!u.opened.is_empty());
    }

    #[test]
    fn gates_stop_the_fill_and_open_with_counters() {
        let mut m = layout();
        m.complete("a", 1, 1);
        m.complete("b", 4, 1);
        // b reveals the gate next to it as a closed gate (1); revealing a
        // path unlocks around it, so c behind the gate is open in the data
        // (as in the original) but unreachable while the gate is closed.
        assert_eq!(m.path_state(2), 1);
        assert_eq!(m.icon_state("c"), Some(2));
        let mut c = Counters {
            prize: 1,
            ..Default::default()
        };
        assert!(m.open_gates(&c).is_empty());
        c.prize = 2;
        assert_eq!(m.open_gates(&c), vec![(2, true)]);
        assert_eq!(m.open_gates(&c), vec![(2, false)]);
    }

    #[test]
    fn closed_gates_stop_the_fill() {
        // icon a (1,1), gate (2,1) already revealed, path (3,1) hidden.
        let ld = LevelInfo::parse(
            "[general]\nlevels=1\npaths=2\nleveltype=1\n\
             [levels]\n0file=a\n0X=1\n0Y=1\n0state=2\n\
             [paths]\n0X=2\n0Y=1\n0gate=1\n0requirement=9\n\
             1X=3\n1Y=1\n",
        );
        let mut m = MapLayout::from_info("m", &ld, Vec::new());
        let mut slot = Slot::default();
        slot.set_path_state("w", "m", 2, 1, 1);
        m.apply_save(&slot, "w");
        m.complete("a", 1, 1);
        assert_eq!(m.path_state(0), 1);
        assert_eq!(m.path_state(1), 0);
    }

    #[test]
    fn save_roundtrip_and_repropagation() {
        let mut m = layout();
        m.complete("a", 1, 1);
        let mut slot = Slot::default();
        m.write(&mut slot, "w");
        assert_eq!(slot.level_state("w", "a"), 3);
        assert_eq!(slot.path_state("w", "m", 2, 1), 3);
        // Loading the written save restores everything.
        let mut m1 = layout();
        m1.apply_save(&slot, "w");
        m1.repropagate();
        assert_eq!(m1.icon_state("b"), Some(2));
        assert_eq!(m1.path_state(1), 3);
        // A save that only knows "a completed" re-opens the direct
        // neighbours (the original's load does not reveal chains again).
        let mut only = Slot::default();
        only.set_level_state("w", "a", 3);
        let mut m2 = layout();
        m2.apply_save(&only, "w");
        m2.repropagate();
        assert_eq!(m2.icon_state("d"), Some(2));
        assert_eq!(m2.path_state(0), 2);
        assert_eq!(m2.icon_state("b"), Some(0));
    }
}
