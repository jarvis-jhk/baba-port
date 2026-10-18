//! Level setup: from `baba-formats` data to an initial [`Level`].

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::sync::Arc;

use baba_formats::{ObjectTable, TileId};

use crate::defs::{Defs, KindId};
use crate::level::{Context, Level, RegMode};
use crate::rng::Rng;
use crate::unit::{IconData, Unit};

/// One unit placed at level start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placed {
    /// Object type.
    pub kind: KindId,
    /// Tile x.
    pub x: i32,
    /// Tile y.
    pub y: i32,
    /// Facing.
    pub dir: u8,
    /// Level-icon data.
    pub icon: Option<Arc<IconData>>,
}

/// Everything needed to (re)start a level: definitions, room, walls and
/// the initial units in load order.
#[derive(Clone, Debug)]
pub struct Source {
    /// Definitions.
    pub defs: Arc<Defs>,
    /// Width (border included).
    pub w: i32,
    /// Height (border included).
    pub h: i32,
    /// Static wall layer (`w * h`, row-major).
    pub wall: Arc<Vec<bool>>,
    /// Initial units in load order (ids 1, 2, ... in this order).
    pub units: Vec<Placed>,
    /// Map path entries (`[paths]`).
    pub paths: Vec<PathEntry>,
    /// Units that exist when the rules PRNG is seeded (`turn.md` §12.1):
    /// the loaded tiles and level icons, not the map cursor placed after.
    pub seed_units: usize,
}

/// A map path entry (`progression.md` §4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathEntry {
    /// Tile x.
    pub x: i32,
    /// Tile y.
    pub y: i32,
    /// Style (1 = always open).
    pub style: i32,
    /// Gate type (0 none).
    pub gate: i32,
    /// Gate requirement.
    pub requirement: i32,
}

/// A materialised path object (not a unit; only the map cursor uses it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathObj {
    /// Tile x.
    pub x: i32,
    /// Tile y.
    pub y: i32,
    /// 1 = closed gate, 3 = walkable.
    pub completion: i32,
}

impl Source {
    /// Builds a source from a level read by `baba-formats`.
    pub fn from_level(objects: &ObjectTable, level: &baba_formats::Level, world: &str) -> Source {
        let defs = Defs::from_level(objects, &level.info, world, &level.stem);
        let w = level.map.width() as i32;
        let h = level.map.height() as i32;
        let mut wall = vec![false; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let border = x == 0 || y == 0 || x == w - 1 || y == h - 1;
                let edge = level
                    .map
                    .layers
                    .iter()
                    .any(|l| l.tile(x as u32, y as u32) == Some(TileId::EDGE));
                wall[(x + y * w) as usize] = border || edge;
            }
        }
        let (units, seed_units) = load_order_seeded(&defs, level);
        let paths = level
            .info
            .paths
            .iter()
            .map(|p| PathEntry {
                x: p.x,
                y: p.y,
                style: p.style,
                gate: p.gate,
                requirement: p.requirement,
            })
            .collect();
        Source {
            defs: Arc::new(defs),
            w,
            h,
            wall: Arc::new(wall),
            units,
            paths,
            seed_units,
        }
    }

    /// A synthetic source (tests and tools).
    pub fn synthetic(defs: Defs, w: i32, h: i32, units: Vec<Placed>) -> Source {
        let mut wall = vec![false; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                wall[(x + y * w) as usize] = x == 0 || y == 0 || x == w - 1 || y == h - 1;
            }
        }
        Source {
            defs: Arc::new(defs),
            w,
            h,
            wall: Arc::new(wall),
            seed_units: units.len(),
            units,
            paths: Vec::new(),
        }
    }
}

/// The original's load order (`turn.md` §16 O1): grid cells with x as the
/// outer and y as the inner loop, layers bottom first; then the level icons
/// of `[levels]`; then a map cursor at `selectorX/Y`. Kept in one function
/// so it can be corrected against the oracle.
pub fn load_order(defs: &Defs, level: &baba_formats::Level) -> Vec<Placed> {
    load_order_seeded(defs, level).0
}

/// [`load_order`] plus the number of units present when the PRNG is seeded
/// (everything but the map cursor).
fn load_order_seeded(defs: &Defs, level: &baba_formats::Level) -> (Vec<Placed>, usize) {
    let mut out = Vec::new();
    let w = level.map.width();
    let h = level.map.height();
    for x in 0..w {
        for y in 0..h {
            for l in &level.map.layers {
                let Some(t) = l.tile(x, y) else { continue };
                if t.is_empty() || t == TileId::EDGE {
                    continue;
                }
                let Some(k) = defs.kind_by_tile(t) else {
                    continue;
                };
                out.push(Placed {
                    kind: k,
                    x: x as i32,
                    y: y as i32,
                    dir: l.dir(x, y) as u8,
                    icon: None,
                });
            }
        }
    }
    for link in &level.info.levels {
        out.push(Placed {
            kind: defs.level_kind,
            x: link.x,
            y: link.y,
            dir: if (0..4).contains(&link.dir) {
                link.dir as u8
            } else {
                3
            },
            icon: Some(Arc::new(IconData {
                file: link.file.clone(),
                name: link.name.clone(),
                number: link.number,
                style: link.style,
                completion: link.state,
                colour: link
                    .colour
                    .map(|c| format!("{},{}", c.x, c.y))
                    .unwrap_or_default(),
                clear_colour: link
                    .clear_colour
                    .map(|c| format!("{},{}", c.x, c.y))
                    .unwrap_or_default(),
                map_level: false,
            })),
        });
    }
    let n = out.len();
    if let Some((sx, sy)) = level.info.general.selector
        && sx > 0
        && sy > 0
        && let Some(k) = defs.kind_by_name("cursor")
        && !out.iter().any(|p| p.kind == k && p.x == sx && p.y == sy)
    {
        out.push(Placed {
            kind: k,
            x: sx,
            y: sy,
            dir: 0,
            icon: None,
        });
    }
    (out, n)
}

impl Level {
    /// Starts a level session from a source (level setup + start passes).
    pub fn start(source: Arc<Source>, ctx: Context) -> Level {
        let defs = source.defs.clone();
        let n = (source.w * source.h) as usize;
        let mut lvl = Level {
            defs: defs.clone(),
            w: source.w,
            h: source.h,
            wall: source.wall.clone(),
            units: Default::default(),
            next_handle: 0,
            order: Vec::new(),
            by_name: vec![Vec::new(); defs.name_count()],
            tiles: vec![Vec::new(); n],
            next_id: 1,
            known: Arc::new(BTreeSet::new()),
            rules: Arc::new(Default::default()),
            dirty: true,
            recompute_count: 0,
            conds_counter: 0,
            word_watch: Arc::new(BTreeSet::new()),
            word_kinds: Arc::new(BTreeSet::new()),
            active_prev: Arc::new(BTreeSet::new()),
            mapdir: 3,
            rotation: 0,
            offset: (0, 0),
            bonus: false,
            last_key: 0,
            auto_count: 0,
            auto_dirs: [5, 5],
            vision: Default::default(),
            noplayer: false,
            fx: Default::default(),
            rng: Cell::new(Rng::new(0)),
            caches: RefCell::new(Default::default()),
            destroy: Cell::new(None),
            destroyed: false,
            history: Vec::new(),
            record: Default::default(),
            turn_changed: false,
            recording: false,
            scratch: Default::default(),
            events: Vec::new(),
            won: false,
            ended: false,
            all_done: false,
            ending_running: false,
            first_turn: true,
            transition: None,
            cursor_link: Default::default(),
            paths: Arc::new(materialise(&source.paths, &ctx)),
            ctx,
            source: source.clone(),
        };
        for p in &source.units {
            let id = lvl.new_id();
            let nid = defs.kind(p.kind).name_id;
            let icon = if p.kind == defs.level_kind {
                let mut d = p.icon.as_deref().cloned().unwrap_or_default();
                if d.colour.is_empty() {
                    d.colour = "1,2".into();
                }
                if d.clear_colour.is_empty() {
                    d.clear_colour = "1,3".into();
                }
                Some(Arc::new(d))
            } else {
                p.icon.clone()
            };
            let u = Unit {
                handle: 0,
                id,
                kind: p.kind,
                x: p.x,
                y: p.y,
                dir: p.dir,
                float: 0,
                dead: false,
                converted: false,
                new: false,
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
            let h = lvl.register(u, RegMode::Load);
            lvl.unit_mut(h).new = false;
        }
        lvl.rng
            .set(Rng::for_level(&defs.world, &defs.stem, source.seed_units));
        crate::turn::level_start(&mut lvl);
        lvl
    }
}

/// Path objects at level start (`progression.md` §4.2): an entry is
/// materialised when its state (saved, else 2 for style 1, else 0) is > 0;
/// gates stay closed (1) unless game flow opened them (state 3 passed in).
fn materialise(entries: &[PathEntry], ctx: &Context) -> Vec<PathObj> {
    let mut out = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let saved = ctx
            .path_states
            .iter()
            .find(|(j, _)| *j == i)
            .map(|p| p.1)
            .unwrap_or(0);
        let state = if e.style == 1 { saved.max(2) } else { saved };
        if state <= 0 {
            continue;
        }
        let completion = if e.gate == 0 || state >= 3 && saved >= 3 {
            3
        } else {
            1
        };
        out.push(PathObj {
            x: e.x,
            y: e.y,
            completion,
        });
    }
    out
}
