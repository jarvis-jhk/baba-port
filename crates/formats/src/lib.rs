//! # baba-formats
//!
//! Readers for the data files of a *Baba Is You* install, so the port can
//! run from the player's own copy of the game. Pure parsing: no rendering,
//! no game rules, no global state. Nothing from the game is embedded here.
//!
//! ## Overview
//!
//! | Data | Module | Entry points |
//! |---|---|---|
//! | Install layout | [`install`] | [`GameInstall`], [`SpriteSet`] |
//! | Worlds (`Data/Worlds/<id>`) | [`world`] | [`World`], [`WorldData`], [`Level`] |
//! | Level grids (`.l`) | [`level`] | [`LevelMap`], [`Layer`], [`Direction`] |
//! | Level metadata (`.ld`) | [`leveldata`] | [`LevelInfo`] |
//! | Object slots and overrides | [`objects`] | [`ObjectTable`], [`TileChanges`], [`EditorObjectList`], [`TileId`] |
//! | Lua table literals | [`lua`] | [`lua::LuaChunk`] |
//! | Palettes | [`palette`] | [`Palette`], [`PalColour`] |
//! | Sprites | [`sprites`] | [`SpriteDir`], [`sprite_variant`] |
//! | Themes, languages | [`text`] | [`Theme`], [`Language`] |
//! | Saves, settings | [`save`] | [`SaveSlot`], [`SaveIndex`], [`Settings`] |
//! | `Assets.dat` | [`assets`] | [`AssetsDat`] |
//! | The game's executable (sound names) | [`exe`] | [`exe::sound_names`] |
//! | INI dialect | [`ini`] | [`Ini`] |
//! | Images | [`image`] | [`RgbaImage`] |
//!
//! The byte layouts are documented in `docs/formats/*.md`.
//!
//! ## Typical use
//!
//! ```no_run
//! use baba_formats::{GameInstall, SpriteSet};
//!
//! let install = GameInstall::open("/path/to/Baba Is You".as_ref())?;
//! let objects = install.load_objects()?;            // base slots (values.lua)
//! let world = install.world("baba")?;
//! let level = world.load_level("0level")?;
//! let effective = objects.with_changes(&level.info.tiles); // per-level overrides
//! let palette = install.load_palette(
//!     Some(&world), &level.info.general.palette, level.info.general.palette_root)?;
//! let sprites = SpriteSet::load(&install, Some(&world))?;
//! for layer in &level.map.layers {
//!     for (x, y, tile, dir) in layer.objects() {
//!         let Some(obj) = effective.by_tile(tile) else { continue };
//!         let rgb = palette.rgb(obj.colour);
//!         let file = sprites.find(&obj.sprite, obj.sprite_in_root, 0, 1);
//!         println!("{x},{y} {} {dir:?} {rgb:?} {file:?}", obj.name);
//!     }
//! }
//! # Ok::<(), baba_formats::Error>(())
//! ```
//!
//! ## Errors
//!
//! Every fallible call returns [`Error`], which names the file (when known),
//! the format and, for binary formats, the byte offset.

#![warn(missing_docs)]

pub mod assets;
mod bytes;
pub mod error;
pub mod exe;
pub mod image;
pub mod ini;
pub mod install;
pub mod level;
pub mod leveldata;
pub mod lua;
mod lz4;
pub mod objects;
pub mod palette;
pub mod save;
pub mod sprites;
pub mod text;
pub mod world;

pub use assets::{AssetKind, AssetsDat, ShaderLanguage};
pub use error::{Error, ErrorKind, Result};
pub use image::RgbaImage;
pub use ini::Ini;
pub use install::{GameInstall, SpriteSet};
pub use level::{Direction, Layer, LevelMap};
pub use leveldata::LevelInfo;
pub use objects::{EditorObjectList, ObjectDef, ObjectTable, TileChanges, TileId, Tiling};
pub use palette::{PalColour, Palette};
pub use save::{SaveIndex, SaveSlot, Settings};
pub use sprites::{SpriteDir, sprite_variant};
pub use text::{Language, Theme};
pub use world::{Level, World, WorldData};
