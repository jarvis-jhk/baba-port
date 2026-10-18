//! The level state and its unit primitives: registration, creation,
//! removal, position updates and the tile map (`docs/spec/turn.md` §1–§2).

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::conditions::Caches;
use crate::defs::{Defs, KindId, NameId};
use crate::events::{Cause, DestroyStyle, Event};
use crate::rng::Rng;
use crate::rules::RuleSet;
use crate::turn::Pending;
use crate::undo::{CreateSource, Line, Record};
use crate::unit::{Handle, IconData, Unit};

/// How a unit is registered (`turn.md` §2.2 step 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegMode {
    /// Normal: single-unit status pass in full mode.
    Normal,
    /// Undo restore: status pass in undoing mode (FLOAT only).
    Undo,
    /// Level load: no status pass (the level-start passes run later).
    Load,
}

/// Level-icon surroundings on the parent map (`conditions.md` §5).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Surroundings {
    /// Icon direction (4 = none).
    pub dir: u8,
    /// Names per cell: index 0 r, 1 u, 2 l, 3 d, 4 dr, 5 ur, 6 ul, 7 dl, 8 o.
    pub cells: [Vec<String>; 9],
}

/// Game-flow context of a level session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Context {
    /// Playing inside the editor (test play).
    pub in_editor: bool,
    /// Surroundings of the level's icon on the parent map.
    pub surroundings: Option<Surroundings>,
    /// Saved map path states by `[paths]` index (game flow).
    pub path_states: Vec<(usize, i32)>,
}

/// Turn-scoped scratch state (`turn.md` §1.4).
#[derive(Clone, Debug, Default)]
pub(crate) struct Scratch {
    pub deleted: BTreeSet<Handle>,
    pub deleted_empty: BTreeSet<i32>,
    pub teleported: BTreeSet<Handle>,
    pub fell: BTreeMap<Handle, u8>,
    pub pending: Vec<Pending>,
    pub vacated: BTreeSet<i32>,
    pub push_count: u32,
    pub move_count: u32,
    pub level_conversions: Vec<crate::convert::ConvEntry>,
    pub empty_bonus: bool,
}

/// A level being played: the complete deterministic state.
#[derive(Clone, Debug)]
pub struct Level {
    pub(crate) defs: Arc<Defs>,
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) wall: Arc<Vec<bool>>,
    pub(crate) units: BTreeMap<Handle, Unit>,
    pub(crate) next_handle: Handle,
    pub(crate) order: Vec<Handle>,
    pub(crate) by_name: Vec<Vec<Handle>>,
    pub(crate) tiles: Vec<Vec<Handle>>,
    pub(crate) next_id: i32,
    pub(crate) known: Arc<BTreeSet<String>>,
    pub(crate) rules: Arc<RuleSet>,
    pub(crate) dirty: bool,
    pub(crate) recompute_count: u32,
    pub(crate) conds_counter: u32,
    pub(crate) word_watch: Arc<BTreeSet<Handle>>,
    pub(crate) word_kinds: Arc<BTreeSet<NameId>>,
    pub(crate) active_prev: Arc<BTreeSet<Handle>>,
    // level-wide state
    pub(crate) mapdir: u8,
    pub(crate) rotation: i32,
    pub(crate) offset: (i32, i32),
    pub(crate) bonus: bool,
    pub(crate) last_key: u8,
    pub(crate) auto_count: u32,
    /// Stored AUTO directions per player (5 = none stored).
    pub(crate) auto_dirs: [u8; 2],
    pub(crate) vision: crate::vision::Vision,
    /// No player exists (music state 1), from the last player check.
    pub(crate) noplayer: bool,
    /// Presentation bookkeeping (not hashed, not undone).
    pub(crate) fx: crate::fx::Fx,
    pub(crate) rng: Cell<Rng>,
    pub(crate) caches: RefCell<Caches>,
    pub(crate) destroy: Cell<Option<DestroyStyle>>,
    pub(crate) destroyed: bool,
    // undo
    pub(crate) history: Vec<Arc<Record>>,
    pub(crate) record: Record,
    pub(crate) turn_changed: bool,
    pub(crate) recording: bool,
    // per turn
    pub(crate) scratch: Scratch,
    pub(crate) events: Vec<Event>,
    pub(crate) won: bool,
    pub(crate) ended: bool,
    pub(crate) all_done: bool,
    pub(crate) ending_running: bool,
    pub(crate) first_turn: bool,
    pub(crate) transition: Option<crate::events::Transition>,
    pub(crate) cursor_link: BTreeMap<Handle, Handle>,
    pub(crate) paths: Arc<Vec<crate::load::PathObj>>,
    pub(crate) ctx: Context,
    pub(crate) source: Arc<crate::load::Source>,
}

impl Level {
    // ----------------------------------------------------------------
    // Geometry
    // ----------------------------------------------------------------

    /// Room width in tiles (border included).
    pub fn width(&self) -> i32 {
        self.w
    }

    /// Room height in tiles (border included).
    pub fn height(&self) -> i32 {
        self.h
    }

    /// On the grid.
    pub(crate) fn on_grid(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.w && y < self.h
    }

    /// Strictly inside (the interior).
    pub(crate) fn inside(&self, x: i32, y: i32) -> bool {
        x >= 1 && y >= 1 && x <= self.w - 2 && y <= self.h - 2
    }

    /// Edge: off-grid or static wall.
    pub(crate) fn is_edge(&self, x: i32, y: i32) -> bool {
        !self.on_grid(x, y) || self.wall[(x + y * self.w) as usize]
    }

    /// Static map tile present (conditions' "map tile not empty").
    pub(crate) fn map_tile(&self, x: i32, y: i32) -> bool {
        self.is_edge(x, y)
    }

    pub(crate) fn tile_index(&self, x: i32, y: i32) -> i32 {
        x + y * self.w
    }

    /// Units on a tile in stack order (empty for off-grid).
    pub(crate) fn at(&self, x: i32, y: i32) -> &[Handle] {
        if self.on_grid(x, y) {
            &self.tiles[(x + y * self.w) as usize]
        } else {
            &[]
        }
    }

    /// Tile has no units.
    pub(crate) fn tile_empty(&self, x: i32, y: i32) -> bool {
        self.at(x, y).is_empty()
    }

    // ----------------------------------------------------------------
    // Unit access
    // ----------------------------------------------------------------

    pub(crate) fn unit(&self, h: Handle) -> &Unit {
        self.units.get(&h).expect("known unit")
    }

    pub(crate) fn unit_mut(&mut self, h: Handle) -> &mut Unit {
        self.units.get_mut(&h).expect("known unit")
    }

    /// The handle refers to a unit record (alive, or removed this turn).
    pub(crate) fn exists(&self, h: Handle) -> bool {
        self.units.contains_key(&h)
    }

    /// The handle refers to a registered (live) unit.
    pub(crate) fn alive(&self, h: Handle) -> bool {
        self.units.get(&h).is_some_and(|u| !u.dead)
    }

    /// Drops the records of removed units (end of turn).
    pub(crate) fn prune(&mut self) {
        self.units.retain(|_, u| !u.dead);
    }

    /// Unit name (`baba`, `text_baba`).
    pub(crate) fn uname(&self, h: Handle) -> &str {
        &self.defs.kinds[self.unit(h).kind as usize].name
    }

    /// Rule name: `text` for every text unit.
    pub(crate) fn rname(&self, h: Handle) -> &str {
        if self.is_text(h) {
            "text"
        } else {
            self.uname(h)
        }
    }

    pub(crate) fn is_text(&self, h: Handle) -> bool {
        self.defs.kinds[self.unit(h).kind as usize].is_text
    }

    pub(crate) fn kind(&self, h: Handle) -> &crate::defs::Kind {
        &self.defs.kinds[self.unit(h).kind as usize]
    }

    /// Units with a unit name (per-name list order).
    pub(crate) fn units_named(&self, name: &str) -> &[Handle] {
        match self.defs.name_id(name) {
            Some(n) => &self.by_name[n as usize],
            None => &[],
        }
    }

    /// First unit in unit-list order with a persistent id (`undo.md` §2).
    pub(crate) fn find_id(&self, id: i32) -> Option<Handle> {
        self.order.iter().copied().find(|&h| self.unit(h).id == id)
    }

    /// Fresh persistent id.
    pub(crate) fn new_id(&mut self) -> i32 {
        let i = self.next_id;
        self.next_id += 1;
        i
    }

    // ----------------------------------------------------------------
    // Registration
    // ----------------------------------------------------------------

    fn alloc(&mut self) -> Handle {
        self.next_handle += 1;
        self.next_handle
    }

    /// Registers a unit (`turn.md` §2.2). Returns its handle.
    pub(crate) fn register(&mut self, mut u: Unit, mode: RegMode) -> Handle {
        let h = self.alloc();
        u.handle = h;
        u.dead = false;
        if mode != RegMode::Undo {
            u.back_start = 0;
            u.broken = false;
            u.followed = -1;
            u.holder = 0;
        }
        u.new = mode != RegMode::Undo;
        u.active = false;
        u.visible = true;
        u.disabled = false;
        let k = u.kind;
        let (x, y) = (u.x, u.y);
        self.units.insert(h, u);
        self.order.push(h);
        if x > 0 && y > 0 && self.on_grid(x, y) {
            let i = self.tile_index(x, y) as usize;
            self.tiles[i].push(h);
        }
        let kd = &self.defs.kinds[k as usize];
        let (nid, is_text, wt, word) = (kd.name_id, kd.is_text, kd.word_type, kd.word.clone());
        self.by_name[nid as usize].push(h);
        if is_text {
            let t = self.defs.text_name as usize;
            self.by_name[t].push(h);
            self.dirty = true;
            if wt == 0 {
                self.add_known(&word);
            }
        } else {
            self.add_known(&word);
        }
        match mode {
            RegMode::Normal => crate::effects::status::status_pass_unit(self, h, false),
            RegMode::Undo => crate::effects::status::status_pass_unit(self, h, true),
            RegMode::Load => {}
        }
        h
    }

    pub(crate) fn add_known(&mut self, name: &str) {
        if !self.known.contains(name) {
            Arc::make_mut(&mut self.known).insert(name.to_string());
        }
    }

    /// Generic creation (`turn.md` §2.1). Returns the new unit's handle.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn create(
        &mut self,
        kind: KindId,
        x: i32,
        y: i32,
        dir: u8,
        float: u8,
        icon: Option<Arc<IconData>>,
        id: Option<i32>,
        base: i32,
        source: CreateSource,
    ) -> Handle {
        let id = id.unwrap_or_else(|| self.new_id());
        let icon = if kind == self.defs.level_kind {
            let mut d = icon.as_deref().cloned().unwrap_or_default();
            let min = if d.file.is_empty() { 1 } else { 2 };
            d.completion = d.completion.max(min);
            if d.colour.is_empty() {
                d.colour = "1,2".into();
            }
            if d.clear_colour.is_empty() {
                d.clear_colour = "1,3".into();
            }
            Some(Arc::new(d))
        } else {
            icon
        };
        let nid = self.defs.kinds[kind as usize].name_id;
        let u = Unit {
            handle: 0,
            id,
            kind,
            x,
            y,
            dir: if dir > 3 { 0 } else { dir },
            float,
            dead: false,
            converted: true,
            new: true,
            original: nid,
            followed: -1,
            back_start: 0,
            holder: 0,
            broken: false,
            active: false,
            visible: true,
            disabled: false,
            paint: 0,
            icon,
        };
        self.line(Line::Created {
            id,
            kind,
            base,
            source,
            at: (x, y, u.dir),
        });
        self.turn_changed = true;
        let h = self.register(u, RegMode::Normal);
        self.unit_mut(h).converted = true;
        if self.word_kinds.contains(&nid) {
            self.dirty = true;
        }
        let name = self.uname(h).to_string();
        let dir = self.unit(h).dir;
        let cause = match source {
            CreateSource::Create => crate::events::CreateCause::Make,
            CreateSource::Convert => crate::events::CreateCause::Convert,
            CreateSource::EmptyConvert => crate::events::CreateCause::EmptyConvert,
            CreateSource::Back => crate::events::CreateCause::Back,
        };
        self.events.push(Event::Created {
            id,
            name,
            at: (x, y),
            dir,
            cause,
        });
        h
    }

    /// Removes a unit from every list without HAS ("delunit",
    /// `turn.md` §2.4). Marks it dead.
    pub(crate) fn unregister(&mut self, h: Handle) {
        if !self.alive(h) {
            return;
        }
        let (x, y, k) = {
            let u = self.unit(h);
            (u.x, u.y, u.kind)
        };
        if let Some(p) = self.order.iter().position(|&o| o == h) {
            self.order.remove(p);
        }
        let kd = &self.defs.kinds[k as usize];
        let (nid, is_text) = (kd.name_id as usize, kd.is_text);
        if let Some(p) = self.by_name[nid].iter().position(|&o| o == h) {
            self.by_name[nid].remove(p);
        }
        if is_text {
            let t = self.defs.text_name as usize;
            if let Some(p) = self.by_name[t].iter().position(|&o| o == h) {
                self.by_name[t].remove(p);
            }
            self.dirty = true;
        }
        if self.on_grid(x, y) {
            let i = self.tile_index(x, y) as usize;
            if let Some(p) = self.tiles[i].iter().position(|&o| o == h) {
                self.tiles[i].remove(p);
            }
        }
        if self.word_watch.contains(&h) {
            self.dirty = true;
            let id = self.unit(h).id;
            if self.recording && !self.record.word_ids.contains(&id) {
                self.record.word_ids.push(id);
            }
        }
        crate::vision::forget(self, h);
        self.unit_mut(h).dead = true;
    }

    /// Destroys a unit ("delete", `turn.md` §2.4): HAS (unless `no_has`),
    /// undo line, unregister. Returns false if it was already removed.
    pub(crate) fn destroy(&mut self, h: Handle, no_has: bool, cause: Cause) -> bool {
        if self.scratch.deleted.contains(&h) || !self.alive(h) {
            return false;
        }
        let (x, y) = {
            let u = self.unit(h);
            (u.x, u.y)
        };
        if !no_has && self.inside(x, y) {
            crate::convert::run_has(self, crate::conditions::Subj::Unit(h));
        }
        if !self.alive(h) {
            return false;
        }
        crate::vision::on_destroy(self, h);
        let snap = self.unit(h).clone();
        let colour = self.unit_colour(h);
        self.remove_recorded(h, cause);
        self.scratch.deleted.insert(h);
        self.fx_removed(&snap, colour, cause);
        true
    }

    /// Records a "removed" line and unregisters (no HAS).
    pub(crate) fn remove_recorded(&mut self, h: Handle, cause: Cause) {
        let snap = self.unit(h).clone();
        self.events.push(Event::Removed {
            id: snap.id,
            name: self.uname(h).to_string(),
            at: (snap.x, snap.y),
            cause,
        });
        self.line(Line::Removed(Box::new(snap)));
        self.turn_changed = true;
        self.unregister(h);
    }

    /// Destroys an empty tile: only EMPTY HAS runs (`turn.md` §2.4).
    pub(crate) fn destroy_empty(&mut self, x: i32, y: i32) {
        let t = self.tile_index(x, y);
        if self.scratch.deleted_empty.contains(&t) {
            return;
        }
        if self.inside(x, y) {
            crate::convert::run_has(self, crate::conditions::Subj::Empty(x, y));
        }
        self.scratch.deleted_empty.insert(t);
    }

    // ----------------------------------------------------------------
    // Position updates
    // ----------------------------------------------------------------

    /// Sets position and facing at once, updating the tile map (no line).
    pub(crate) fn place_unit(&mut self, h: Handle, x: i32, y: i32, dir: u8) {
        let (ox, oy) = {
            let u = self.unit(h);
            (u.x, u.y)
        };
        if (ox, oy) != (x, y) {
            if self.on_grid(ox, oy) {
                let i = self.tile_index(ox, oy) as usize;
                if let Some(p) = self.tiles[i].iter().position(|&o| o == h) {
                    self.tiles[i].remove(p);
                }
            }
            if self.on_grid(x, y) {
                let i = self.tile_index(x, y) as usize;
                self.tiles[i].push(h);
            }
        }
        let u = self.unit_mut(h);
        u.x = x;
        u.y = y;
        u.dir = dir;
        if self.is_text(h) || self.word_watch.contains(&h) {
            self.dirty = true;
        }
    }

    /// The standard update: move/turn with an undo line when something
    /// changes (`turn.md` §5).
    pub(crate) fn update_unit(&mut self, h: Handle, x: i32, y: i32, dir: u8) {
        if !self.alive(h) {
            return;
        }
        let (id, ox, oy, od) = {
            let u = self.unit(h);
            (u.id, u.x, u.y, u.dir)
        };
        let dir = if dir > 3 { od } else { dir };
        if (ox, oy, od) == (x, y, dir) {
            return;
        }
        self.line(Line::Moved {
            id,
            old: (ox, oy, od),
            new: (x, y, dir),
        });
        self.turn_changed = true;
        self.place_unit(h, x, y, dir);
        let kind = self.move_kind();
        self.events.push(Event::Moved {
            id,
            from: (ox, oy),
            to: (x, y),
            from_dir: od,
            dir,
            kind,
        });
    }

    /// Turn a unit (recorded).
    pub(crate) fn set_dir(&mut self, h: Handle, dir: u8) {
        if !self.alive(h) || dir > 3 {
            return;
        }
        let (x, y) = {
            let u = self.unit(h);
            (u.x, u.y)
        };
        self.update_unit(h, x, y, dir);
    }

    /// Rebuilds the tile map from the unit list, trimming overflow
    /// (`turn.md` §4.2 step 1, §11).
    pub(crate) fn rebuild_tiles(&mut self) {
        for t in self.tiles.iter_mut() {
            t.clear();
        }
        let limit = self.defs.stack_limit;
        let mut trim: Vec<Handle> = Vec::new();
        let mut counts: BTreeMap<(usize, NameId), usize> = BTreeMap::new();
        let order = self.order.clone();
        for h in order {
            let (x, y, k, lf) = {
                let u = self.unit(h);
                (u.x, u.y, u.kind, u.has_level_file())
            };
            if !self.on_grid(x, y) || x <= 0 || y <= 0 {
                continue;
            }
            let i = self.tile_index(x, y) as usize;
            let c = counts
                .entry((i, self.defs.kinds[k as usize].name_id))
                .or_insert(0);
            if *c < limit {
                *c += 1;
            } else if !lf {
                trim.push(h);
                continue;
            }
            self.tiles[i].push(h);
        }
        // `turn.md` §11.2: removed line, no HAS, not in `deleted`.
        for h in trim {
            let snap = self.unit(h).clone();
            self.events.push(Event::Removed {
                id: snap.id,
                name: self.uname(h).to_string(),
                at: (snap.x, snap.y),
                cause: Cause::Silent,
            });
            self.line(Line::Removed(Box::new(snap)));
            self.turn_changed = true;
            self.unregister(h);
        }
    }

    /// Requests level destruction (`turn.md` §11.1).
    pub(crate) fn request_destroy(&self, style: DestroyStyle) {
        if self.destroy.get().is_none() {
            self.destroy.set(Some(style));
        }
    }

    /// Rules PRNG draw.
    pub(crate) fn rand(&self, lo: i32, hi: i32) -> i32 {
        let mut r = self.rng.get();
        let v = r.range(lo, hi);
        self.rng.set(r);
        v
    }

    /// Text units (creation order).
    pub(crate) fn text_units(&self) -> &[Handle] {
        &self.by_name[self.defs.text_name as usize]
    }
}
