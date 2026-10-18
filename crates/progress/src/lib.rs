//! # baba-progress
//!
//! The save model and progression rules of baba-port, without any I/O
//! beyond reading/writing its own files and without rendering:
//!
//! | module | what |
//! |---|---|
//! | [`store`] | ordered ini store in the original's dialect |
//! | [`settings`] | `SettingsC.txt`: options, key/pad bindings, slot names |
//! | [`slot`] | one save slot: per-world states, counters, tree, conversions |
//! | [`profile`] | the data directory with settings, `ba.ba` and three slots |
//! | [`map`] | icon/path states of a map, unlocking, gates |
//! | [`tree`] | level tree, level ids and the "Level X" label |
//!
//! The file names and key layout follow the original game (private spec
//! `saves.md`), so an importer can copy an existing save group by group.

pub mod map;
pub mod profile;
pub mod settings;
pub mod slot;
pub mod store;
pub mod tree;

pub use map::{Change, Counters, Icon, MapLayout, Path, Unlock};
pub use profile::{Profile, SLOTS, default_data_dir};
pub use settings::{Action, PadAction, Settings, Toggle, Zoom};
pub use slot::{Slot, Stats};
pub use store::Store;
pub use tree::{LevelTree, Node, level_id};
