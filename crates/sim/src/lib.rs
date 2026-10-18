//! # baba-sim
//!
//! A pure, deterministic turn engine for a Baba Is You reimplementation:
//! rule parsing, conditions, movement, every property and verb,
//! conversions, undo and the map cursor. No I/O, no rendering, no clocks,
//! no threads, no floats; all randomness comes from a PRNG stored in the
//! level state. Many independent [`Level`]s may live in one process, and
//! cloning one is cheap enough for online views.
//!
//! ```no_run
//! use std::sync::Arc;
//! use baba_formats::GameInstall;
//! use baba_sim::{Context, Input, Level, Source};
//!
//! let install = GameInstall::open("/path/to/Baba Is You".as_ref())?;
//! let objects = install.load_objects()?;
//! let world = install.world("baba")?;
//! let data = world.load_level("1level")?;
//! let src = Arc::new(Source::from_level(&objects, &data, "baba"));
//! let mut level = Level::start(src, Context::default());
//! let out = level.step(Input::dir(0));
//! println!("won: {}, hash {:016x}", out.won, level.state_hash());
//! # Ok::<(), baba_formats::Error>(())
//! ```
//!
//! Module map (design notes: `docs/sim-design.md`):
//!
//! | module | what |
//! |---|---|
//! | `defs` | per-level object definitions and name lookups |
//! | `level`, `unit` | state, unit primitives, tile map |
//! | `rules` | parsing, ALL/NOT/MIMIC/groups/WORD, queries |
//! | `conditions` | condition evaluation, OFTEN/SELDOM, POWERED |
//! | `movement` | takes, pushing/pulling/swapping, FALL, move-block, SELECT |
//! | `effects` | status pass, block effects, level block, visual refresh |
//! | `convert` | conversions, HAS, MAKE, level conversion |
//! | `turn` | the turn pipeline, pending queue, undo turn, level start |
//! | `undo` | undo lines, records, commit and replay |

mod conditions;
mod convert;
pub mod defs;
mod effects;
pub mod events;
pub mod fx;
mod hash;
mod level;
mod load;
mod movement;
mod present;
pub mod rng;
pub mod rules;
mod turn;
mod undo;
mod unit;
mod vision;

pub use conditions::Subj;
pub use defs::{Defs, SimpleDef};
pub use events::{
    Burst, Cause, Colour, CreateCause, DestroyStyle, Event, MoveKind, Particle, Sound, TextState,
    Transition, TurnKind, TurnOutcome,
};
pub use level::{Context, Level, Surroundings};
pub use load::{PathEntry, PathObj, Placed, Source, load_order};
pub use present::EffectSources;
pub use rng::CosmeticRng;
pub use rules::{Cond, Rule, RuleSet};
pub use turn::Input;
pub use unit::{Handle, IconData, Unit, dir_delta};
pub use vision::{Cam, Camera};

#[doc(hidden)]
pub mod testkit;

/// A read-only view of one unit (presentation, tests, oracle diffs).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitView {
    /// Persistent id.
    pub id: i32,
    /// Unit name.
    pub name: String,
    /// Tile x.
    pub x: i32,
    /// Tile y.
    pub y: i32,
    /// Facing.
    pub dir: u8,
    /// Float layer.
    pub float: u8,
    /// Part of a valid rule (text).
    pub active: bool,
    /// Visible.
    pub visible: bool,
    /// Text display state (inactive / active / disabled with a cross).
    pub text: TextState,
    /// Colour to draw (first colour property, else the definition's
    /// colour; text: active colour when active).
    pub colour: Colour,
    /// All colour properties in blink order (empty: none; several:
    /// the unit blinks through them, `presentation.md` §5.6).
    pub colours: Vec<Colour>,
    /// Draw layer of the definition.
    pub layer: i32,
}

impl Level {
    /// Units in unit-list order.
    pub fn units(&self) -> Vec<UnitView> {
        self.order
            .iter()
            .map(|&h| {
                let u = self.unit(h);
                UnitView {
                    id: u.id,
                    name: self.uname(h).to_string(),
                    x: u.x,
                    y: u.y,
                    dir: u.dir,
                    float: u.float,
                    active: u.active,
                    visible: u.visible,
                    text: if self.is_text(h) {
                        self.text_state(u)
                    } else {
                        TextState::Inactive
                    },
                    colour: self.unit_colour(h),
                    colours: fx::colours_of(u.paint),
                    layer: self.kind(h).layer,
                }
            })
            .collect()
    }

    /// Full unit records in unit-list order.
    pub fn unit_records(&self) -> Vec<&Unit> {
        self.order.iter().map(|&h| self.unit(h)).collect()
    }

    /// The current rule set.
    pub fn rules(&self) -> &RuleSet {
        &self.rules
    }

    /// Rules as display strings (`baba is you [on(tile)]`).
    pub fn rule_strings(&self) -> Vec<String> {
        self.rules.display()
    }

    /// The definitions.
    pub fn defs(&self) -> &Defs {
        &self.defs
    }

    /// Number of committed undo steps.
    pub fn undo_depth(&self) -> usize {
        self.history.len()
    }

    /// Map direction (0..3).
    pub fn mapdir(&self) -> u8 {
        self.mapdir
    }

    /// Level rotation in degrees.
    pub fn rotation(&self) -> i32 {
        self.rotation
    }

    /// Level screen offset in tiles.
    pub fn offset(&self) -> (i32, i32) {
        self.offset
    }

    /// The level bonus has been collected.
    pub fn bonus(&self) -> bool {
        self.bonus
    }

    /// The level was destroyed (and not undone).
    pub fn destroyed(&self) -> bool {
        self.order.is_empty() && self.destroyed
    }

    /// Raw state of the rules PRNG (oracle diffs).
    pub fn rng_state(&self) -> u64 {
        self.rng.get().state()
    }

    /// Stable hash of the simulation state (identical on every platform).
    pub fn state_hash(&self) -> u64 {
        let mut f = hash::Fnv::new();
        f.i32(self.w);
        f.i32(self.h);
        f.u32(self.order.len() as u32);
        for &h in &self.order {
            let u = self.unit(h);
            f.i32(u.id);
            f.str(&self.defs.kinds[u.kind as usize].name);
            f.i32(u.x);
            f.i32(u.y);
            f.u8(u.dir);
            f.u8(u.float);
            f.str(self.defs.name(u.original));
            f.i32(u.followed);
            f.u32(u.back_start);
            f.i32(u.holder);
            f.bool(u.broken);
            f.i32(u.completion());
        }
        f.i32(self.next_id);
        f.u8(self.mapdir);
        f.i32(self.rotation);
        f.i32(self.offset.0);
        f.i32(self.offset.1);
        f.bool(self.bonus);
        f.u8(self.last_key);
        f.u64(self.rng.get().state());
        f.bool(self.vision.on);
        match self.vision.cam {
            Cam::None => f.u8(0),
            Cam::Unit(id) => {
                f.u8(1);
                f.i32(id);
            }
            Cam::Fixed { x, y, dir } => {
                f.u8(2);
                f.i32(x);
                f.i32(y);
                f.u8(dir);
            }
        }
        let vids = self.vision_ids();
        f.u32(vids.len() as u32);
        for id in vids {
            f.i32(id);
        }
        f.u8(self.auto_dirs[0]);
        f.u8(self.auto_dirs[1]);
        f.u32(self.known.len() as u32);
        for k in self.known.iter() {
            f.str(k);
        }
        f.u32(self.history.len() as u32);
        f.bool(self.turn_changed);
        f.finish()
    }
}
