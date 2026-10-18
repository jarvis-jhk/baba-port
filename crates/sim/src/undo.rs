//! Undo history (`docs/spec/undo.md`): per-turn records of lines, commit
//! rules and the undo operation itself.

use std::sync::Arc;

use crate::defs::KindId;
use crate::events::{Cause, CreateCause, Event, MoveKind};
use crate::level::Level;
use crate::rng::Rng;
use crate::unit::Unit;

/// How a "created" line came about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CreateSource {
    /// Generic creation (MAKE, HAS, MORE, ALL, ...).
    Create,
    /// Product of a unit conversion.
    Convert,
    /// Product of an EMPTY conversion.
    EmptyConvert,
    /// BACK recreation.
    Back,
}

/// One undo line (`undo.md` §2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    /// Position and/or facing changed.
    Moved {
        /// Unit id.
        id: i32,
        /// Old (x, y, dir).
        old: (i32, i32, u8),
        /// New (x, y, dir).
        new: (i32, i32, u8),
    },
    /// A unit was removed (full snapshot).
    Removed(Box<Unit>),
    /// A unit was created.
    Created {
        /// Unit id.
        id: i32,
        /// Kind.
        kind: KindId,
        /// Source unit id (-1 none).
        base: i32,
        /// Origin.
        source: CreateSource,
        /// Position and facing at creation.
        at: (i32, i32, u8),
    },
    /// A unit left through DONE (snapshot before leaving).
    Done(Box<Unit>),
    /// Float changed.
    Float {
        /// Unit id.
        id: i32,
        /// Old float.
        old: u8,
    },
    /// BACK start changed.
    BackStart {
        /// Unit id.
        id: i32,
        /// Old value.
        old: u32,
    },
    /// Level screen offset changed.
    LevelOffset {
        /// Old offset.
        old: (i32, i32),
        /// Old map direction.
        old_mapdir: u8,
    },
    /// Level rotation changed.
    MapRotation {
        /// Old rotation (degrees).
        old: i32,
    },
    /// Map direction changed.
    MapDir {
        /// Old value.
        old: u8,
    },
    /// Map cursor moved.
    MapCursor {
        /// Unit id.
        id: i32,
        /// Old (x, y, dir).
        old: (i32, i32, u8),
    },
    /// BROKEN cache changed to `new_state`.
    Broken {
        /// Unit id.
        id: i32,
        /// New state.
        new_state: bool,
    },
    /// Bonus collected.
    Bonus,
    /// FOLLOW memory changed.
    Followed {
        /// Unit id.
        id: i32,
        /// Old value.
        old: i32,
    },
    /// HOLD link changed.
    Holder {
        /// Unit id.
        id: i32,
        /// Old value.
        old: i32,
    },
    /// Informational conversion line.
    Converted,
    /// 3D mode started with this camera target.
    StartVision {
        /// Camera target.
        cam: crate::vision::Cam,
    },
    /// 3D mode stopped.
    StopVision {
        /// Camera target before stopping.
        cam: crate::vision::Cam,
        /// Camera position and facing before stopping.
        pos: (i32, i32, u8),
    },
    /// The camera target changed.
    VisionTarget {
        /// Old target.
        old: crate::vision::Cam,
        /// New target.
        new: crate::vision::Cam,
    },
}

/// One undo record (`undo.md` §1).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Record {
    /// Lines in execution order (replayed in reverse).
    pub lines: Vec<Line>,
    /// "Last key" when the record was opened.
    pub last_key: u8,
    /// Rules PRNG when the record was opened.
    pub rng: Option<Rng>,
    /// Ids of the 3D target cycle when the record was opened.
    pub vision_ids: Vec<i32>,
    /// Ids of the WORD units and WORD-related units when the record was
    /// opened, plus WORD units removed during the turn: undoing a line
    /// of one of them forces a rule recomputation (`undo.md` §4).
    pub word_ids: Vec<i32>,
}

impl Level {
    /// Appends a line to the in-progress record.
    pub(crate) fn line(&mut self, l: Line) {
        if self.recording {
            self.record.lines.push(l);
        }
    }

    /// End-of-turn commit (`undo.md` §3). Returns whether a step was kept.
    pub(crate) fn commit_record(&mut self) -> bool {
        let kept = self.turn_changed;
        let fresh = Record {
            lines: Vec::new(),
            last_key: self.last_key,
            rng: Some(self.rng.get()),
            vision_ids: self.vision_ids(),
            word_ids: self.word_ids(),
        };
        let old = std::mem::replace(&mut self.record, fresh);
        if kept {
            self.history.push(Arc::new(old));
        }
        self.turn_changed = false;
        kept
    }

    /// Applies the undo of the last committed record (`undo.md` §4 steps
    /// 1–6). Returns false if there was nothing to undo.
    pub(crate) fn undo_record(&mut self) -> bool {
        let Some(rec) = self.history.pop() else {
            return false;
        };
        self.last_key = rec.last_key;
        if let Some(r) = rec.rng {
            self.rng.set(r);
        }
        let was = self.recording;
        // Undo's own changes go nowhere (they are not recorded), but the
        // create/remove primitives still set the turn-changed flag (§7.3).
        self.recording = false;
        for l in rec.lines.iter().rev() {
            let id = match l {
                Line::Moved { id, .. } => Some(*id),
                Line::Removed(u) | Line::Done(u) => Some(u.id),
                _ => None,
            };
            if id.is_some_and(|id| rec.word_ids.contains(&id)) {
                self.dirty = true;
            }
            self.undo_line(l);
        }
        self.recording = was;
        self.record.rng = rec.rng;
        if !rec.vision_ids.is_empty() {
            self.vision.targets = rec
                .vision_ids
                .iter()
                .filter_map(|&id| self.find_id(id))
                .collect();
        }
        self.record.vision_ids = rec.vision_ids.clone();
        self.record.word_ids = rec.word_ids.clone();
        true
    }

    /// Ids of the units the WORD machinery watches.
    pub(crate) fn word_ids(&self) -> Vec<i32> {
        self.word_watch
            .iter()
            .filter(|&&h| self.alive(h))
            .map(|&h| self.unit(h).id)
            .collect()
    }

    fn undo_line(&mut self, l: &Line) {
        match l {
            Line::Moved { id, old, .. } => {
                if let Some(h) = self.find_id(*id) {
                    let (fx, fy, fd) = {
                        let u = self.unit(h);
                        (u.x, u.y, u.dir)
                    };
                    self.place_unit(h, old.0, old.1, old.2);
                    self.events.push(Event::Moved {
                        id: *id,
                        from: (fx, fy),
                        to: (old.0, old.1),
                        from_dir: fd,
                        dir: old.2,
                        kind: MoveKind::Undo,
                    });
                }
            }
            Line::Removed(u) => {
                self.restore_unit(u, None);
            }
            Line::Done(u) => {
                self.restore_unit(u, Some(u.float));
            }
            Line::Created { id, .. } => {
                if let Some(h) = self.find_id(*id) {
                    let (x, y) = (self.unit(h).x, self.unit(h).y);
                    let name = self.uname(h).to_string();
                    self.unregister(h);
                    self.turn_changed = true;
                    self.events.push(Event::Removed {
                        id: *id,
                        name,
                        at: (x, y),
                        cause: Cause::Undo,
                    });
                }
            }
            Line::Float { id, old } => {
                if let Some(h) = self.find_id(*id) {
                    self.unit_mut(h).float = *old;
                }
            }
            Line::BackStart { id, old } => {
                if let Some(h) = self.find_id(*id) {
                    self.unit_mut(h).back_start = *old;
                }
            }
            Line::LevelOffset { old, old_mapdir } => {
                self.offset = *old;
                self.mapdir = *old_mapdir;
            }
            Line::MapRotation { old } => self.rotation = *old,
            Line::MapDir { old } => self.mapdir = *old,
            Line::MapCursor { id, old } => {
                if let Some(h) = self.find_id(*id) {
                    self.place_unit(h, old.0, old.1, old.2);
                }
            }
            Line::Broken { id, new_state } => {
                if let Some(h) = self.find_id(*id) {
                    self.unit_mut(h).broken = !*new_state;
                }
            }
            Line::Bonus => self.bonus = false,
            Line::Followed { id, old } => {
                if let Some(h) = self.find_id(*id) {
                    self.unit_mut(h).followed = *old;
                }
            }
            Line::Holder { id, old } => {
                if let Some(h) = self.find_id(*id) {
                    self.unit_mut(h).holder = *old;
                }
            }
            Line::Converted => {}
            Line::StartVision { .. } | Line::StopVision { .. } | Line::VisionTarget { .. } => {
                crate::vision::undo_line(self, l)
            }
        }
    }

    /// Recreates a removed unit from its snapshot (appended to the unit
    /// list, registered in undoing mode).
    fn restore_unit(&mut self, snap: &Unit, float: Option<u8>) {
        let mut u = snap.clone();
        u.dead = false;
        u.converted = false;
        u.new = false;
        if let Some(f) = float {
            u.float = f;
        }
        let h = self.register(u, crate::level::RegMode::Undo);
        self.turn_changed = true;
        let u = self.unit(h);
        let ev = Event::Created {
            id: u.id,
            name: self.uname(h).to_string(),
            at: (u.x, u.y),
            dir: u.dir,
            cause: CreateCause::Undo,
        };
        self.events.push(ev);
    }
}
