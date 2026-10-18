//! Editor undo (`docs/spec/editor.md` §11): a stack of groups, one per
//! user action (a stroke, a tool drag, a selection paste, move all, …).
//! Undo reverts the newest group. Each group keeps the grid and the map
//! records as they were before the action, which reverts exactly what the
//! original's per-object records revert (placed, removed, direction,
//! moved, dragged), map icons/paths/specials included. No redo.

use baba_formats::LevelMap;
use baba_formats::leveldata::{LevelLink, MapPath, Special};

use super::model::EdLevel;

#[derive(Clone, PartialEq)]
struct Snap {
    map: LevelMap,
    levels: Vec<LevelLink>,
    paths: Vec<MapPath>,
    specials: Vec<Special>,
}

impl Snap {
    fn of(l: &EdLevel) -> Snap {
        Snap {
            map: l.map.clone(),
            levels: l.info.levels.clone(),
            paths: l.info.paths.clone(),
            specials: l.info.specials.clone(),
        }
    }
}

/// Groups bigger than this many are forgotten from the bottom.
const LIMIT: usize = 200;

#[derive(Default)]
pub struct Undo {
    groups: Vec<Snap>,
    open: Option<Snap>,
}

impl Undo {
    /// Starts an action (if none is open).
    pub fn begin(&mut self, l: &EdLevel) {
        if self.open.is_none() {
            self.open = Some(Snap::of(l));
        }
    }

    /// Ends the open action; it becomes a group if it changed anything.
    /// Returns true if a group was added.
    pub fn commit(&mut self, l: &EdLevel) -> bool {
        let Some(s) = self.open.take() else {
            return false;
        };
        if s == Snap::of(l) {
            return false;
        }
        self.groups.push(s);
        if self.groups.len() > LIMIT {
            self.groups.remove(0);
        }
        true
    }

    /// Reverts the newest group. False if there is nothing to undo.
    pub fn undo(&mut self, l: &mut EdLevel) -> bool {
        if let Some(open) = self.open.take()
            && open != Snap::of(l)
        {
            self.groups.push(open);
        }
        let Some(s) = self.groups.pop() else {
            return false;
        };
        l.map = s.map;
        l.info.levels = s.levels;
        l.info.paths = s.paths;
        l.info.specials = s.specials;
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.groups.is_empty()
    }

    /// Forgets everything (`§11.4`).
    pub fn clear(&mut self) {
        self.groups.clear();
        self.open = None;
    }

    pub fn len(&self) -> usize {
        self.groups.len()
    }

    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }
}
