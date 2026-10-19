//! baba-port's game layer as a library: the platform-agnostic [`App`]
//! (title, menus, settings, saves, map and level flow, audio rules,
//! credits) driven by any `baba_platform::Platform`. The desktop binary
//! (`src/main.rs`) and other entry points (Android) only create a
//! platform and a GL context and call [`App::frame`] in their loop.
//!
//! | module | what |
//! |---|---|
//! | [`app`] | the game: modes, menus, transitions, drawing |
//! | [`play`] | a play session: level tree, map loading, entering, winning, unlock animations |
//! | [`scene`] | sim level → tile scene (units, icons, paths, backgrounds) |
//! | [`menu`] | generic menu grid |
//! | [`osk`] | on-screen keyboard for slot names and level/room codes |
//! | [`input`] | bindings, key repeat, rebinding capture |
//! | [`audio`] | music selection, turn sounds, volumes |
//! | [`sounds`] | sound effects by name from the install |
//! | [`lang`], [`font`], [`ui`] | language lookup, TrueType glyphs, overlay drawing |
//! | [`world`] | worlds and level metadata |
//! | [`credits`] | credits and endings |
//! | [`drive`] | scripted headless runs |
//! | [`editor`] | the level editor: model, tools, undo, drawing, gamepad, tutorial |
//! | [`level_view`] | static level preview from the `.l` file (screenshots) |
//! | [`import`] | importing the original's save and settings |
//! | [`levelcodes`] | online level codes: download, featured list, history (upload behind a feature) |
//! | [`packs`] | custom world packs and the user `levels` world |
//! | [`session`] | turns (the one path that changes an instance), multiplayer sessions in three modes |
//! | [`join`] | local players: devices, keyboard halves, the join screen |
//! | [`split`] | splitscreen layout for 1–12 viewports |
//! | [`fx`], [`art`], [`pixelfont`], [`keycaps`], [`title`] | presentation (M5): per-view level effects and particles, engine art, bitmap fonts, key caps, title screen |

pub mod app;
pub mod art;
pub mod audio;
pub mod credits;
pub mod cutscene;
#[cfg(not(target_os = "android"))]
pub mod drive;
pub mod editor;
pub mod font;
pub mod fx;
pub mod import;
pub mod input;
pub mod join;
pub mod keycaps;
pub mod lang;
pub mod level_view;
pub mod levelcodes;
pub mod menu;
pub mod osk;
pub mod overlays;
pub mod packs;
pub mod pixelfont;
pub mod play;
pub mod scene;
pub mod session;
pub mod sounds;
pub mod split;
pub mod title;
pub mod ui;
#[cfg(all(target_os = "linux", feature = "sdl"))]
pub mod vpads;
pub mod world;

pub use app::{App, AppConfig};
