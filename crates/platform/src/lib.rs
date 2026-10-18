//! `baba-platform`: the boundary between the game and the OS.
//!
//! The game sees only [`Platform`] (window + GL context, events, timing,
//! audio) and the backend-neutral [`Event`]s below. The SDL2 backend
//! ([`sdl::SdlPlatform`], feature `sdl2`) covers Windows, Linux, macOS and
//! PortMaster (KMSDRM + GLES2); the Android backend ([`android`], feature
//! `android`: NativeActivity + EGL + AAudio, no Java) implements the same
//! trait, reuses [`audio`] as is and runs the on-screen [`touch`] controls.

#[cfg(all(feature = "android", target_os = "android"))]
pub mod android;
pub mod audio;
#[cfg(all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"))]
mod glibc_compat;
#[cfg(feature = "sdl2")]
pub mod sdl;
pub mod touch;

use std::time::Duration;

pub use audio::Audio;
pub use glow;

/// Which GL context to ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlRequest {
    /// Platform default order (see `docs/platform.md`): desktop tries GL
    /// 3.3 core, then GLES 2, then GL 2.1; ARM Linux / Android try GLES 2
    /// first.
    Auto,
    Gles2,
    GlCore33,
    Gl21,
}

impl GlRequest {
    /// `BABA_GL=gles2|gl33|gl21` overrides `Auto`.
    pub fn from_env() -> Self {
        match std::env::var("BABA_GL").as_deref() {
            Ok("gles2") | Ok("gles") => GlRequest::Gles2,
            Ok("gl33") => GlRequest::GlCore33,
            Ok("gl21") => GlRequest::Gl21,
            _ => GlRequest::Auto,
        }
    }
}

/// Window / context options.
#[derive(Clone, Debug)]
pub struct WindowConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub fullscreen: bool,
    pub vsync: bool,
    pub gl: GlRequest,
    /// Open an audio device.
    pub audio: bool,
    /// Extra controller mapping files (SDL `gamecontrollerdb.txt` format),
    /// loaded after SDL's built-in mappings, later files win.
    pub controller_db: Vec<std::path::PathBuf>,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "baba-port".into(),
            width: 1280,
            height: 720,
            fullscreen: false,
            vsync: true,
            gl: GlRequest::from_env(),
            audio: true,
            controller_db: Vec::new(),
        }
    }
}

/// Identifies an input device for as long as it is connected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DeviceId {
    Keyboard,
    Mouse,
    Touch,
    /// Backend instance id (SDL joystick instance id; never reused while
    /// the process runs).
    Controller(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub name: String,
    /// SDL GUID or similar, for mapping files.
    pub guid: String,
    /// False for joysticks without a controller mapping (no events).
    pub mapped: bool,
}

/// Keys the game cares about, by physical position (scancode); anything
/// else arrives as `Other(backend scancode)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    W,
    A,
    S,
    D,
    Z,
    R,
    Q,
    E,
    P,
    M,
    Space,
    Enter,
    Escape,
    Backspace,
    Tab,
    Ctrl,
    Shift,
    Alt,
    /// Letters other than the above, `Char(b'a'..=b'z')`, and digits.
    Char(u8),
    F(u8),
    /// Android back / menu buttons.
    Back,
    Menu,
    Other(u32),
}

/// SDL-style controller buttons (positional names).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Button {
    South,
    East,
    West,
    North,
    Back,
    Guide,
    Start,
    LeftStick,
    RightStick,
    LeftShoulder,
    RightShoulder,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
    Misc,
    Other(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Axis {
    LeftX,
    LeftY,
    RightX,
    RightY,
    TriggerLeft,
    TriggerRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchPhase {
    Down,
    Move,
    Up,
}

/// Backend-neutral events. Positions are in drawable pixels; touch in
/// 0..1 of the window.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Quit,
    /// New drawable size in pixels.
    Resized {
        width: u32,
        height: u32,
    },
    Focus(bool),
    /// The app goes to the background (Android: GL context may be lost).
    Suspend,
    /// The app is back; if `gl_lost`, re-create the renderer from
    /// `Platform::take_restored_gl`.
    Resume {
        gl_lost: bool,
    },
    Key {
        key: Key,
        down: bool,
        repeat: bool,
    },
    ControllerAdded(DeviceInfo),
    ControllerRemoved(DeviceId),
    Button {
        device: DeviceId,
        button: Button,
        down: bool,
    },
    /// -1..1 for sticks (y down positive), 0..1 for triggers.
    Axis {
        device: DeviceId,
        axis: Axis,
        value: f32,
    },
    MouseMove {
        x: f32,
        y: f32,
    },
    MouseButton {
        button: u8,
        down: bool,
        x: f32,
        y: f32,
    },
    MouseWheel {
        dx: f32,
        dy: f32,
    },
    Touch {
        finger: u64,
        phase: TouchPhase,
        x: f32,
        y: f32,
    },
}

/// What the game needs from the OS. The GL context is created by the
/// backend's constructor (which returns a `glow::Context`) and stays
/// current on the main thread.
pub trait Platform {
    /// Backend and driver names for logs (e.g. "SDL 2.30.0 x11, GLES 2").
    fn describe(&self) -> String;
    /// Drawable size in pixels (may differ from window size on HiDPI).
    fn drawable_size(&self) -> (u32, u32);
    /// Appends pending events.
    fn poll_events(&mut self, out: &mut Vec<Event>);
    /// Presents the frame (blocks for vsync when enabled).
    fn swap_buffers(&mut self);
    fn set_fullscreen(&mut self, on: bool);
    /// Monotonic time since the platform started.
    fn time(&self) -> Duration;
    /// The audio front end, if a device could be opened.
    fn audio(&mut self) -> Option<&mut Audio>;
    /// Currently connected input devices.
    fn devices(&self) -> Vec<DeviceInfo>;
    /// Rumble a controller (no-op where unsupported).
    fn rumble(&mut self, _device: DeviceId, _low: f32, _high: f32, _ms: u32) {}
    /// After `Event::Resume { gl_lost: true }`: the new context.
    fn take_restored_gl(&mut self) -> Option<glow::Context> {
        None
    }
    /// On-screen touch controls, where the backend runs them (Android).
    /// Touches on them already arrive as key events; the game draws
    /// `items()` over its frame (`baba_render::ui`) before `swap_buffers`.
    fn touch_controls(&self) -> Option<&touch::TouchControls> {
        None
    }
    /// Mutable access, e.g. to disable the controls on screens that take
    /// raw touches.
    fn touch_controls_mut(&mut self) -> Option<&mut touch::TouchControls> {
        None
    }
}
