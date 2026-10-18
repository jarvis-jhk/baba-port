//! `baba-render`: an OpenGL ES 2.0-level renderer (via `glow`) for Baba
//! levels: run-time sprite atlas, palette colours, tile grid with the
//! three-frame wobble, N viewports with letterboxing and integer-ish
//! scaling, an effect pass per view, and a headless EGL path plus a CPU
//! reference rasterizer for screenshots and tests.
//!
//! The renderer knows nothing about windows: the platform layer (or
//! [`headless::HeadlessContext`]) hands it a current `glow::Context`.

pub mod atlas;
pub mod cpu;
#[cfg(feature = "headless")]
pub mod headless;
pub mod image;
pub mod renderer;
pub mod scene;
pub mod sprites;
pub mod ui;
pub mod viewport;

pub use atlas::{Atlas, Region, RegionId};
pub use glow;
pub use image::{DiffStats, RgbaImage, diff, diff_image};
pub use renderer::{EffectId, GlFlavor, GlInfo, PostEffect, RenderError, Renderer, View};
pub use scene::{Quad, SceneSprite, TileScene, WOBBLE_FRAMES};
pub use sprites::{Palette, SpriteBank};
pub use viewport::{Fit, Rect, ScaleMode, fit, tile};
