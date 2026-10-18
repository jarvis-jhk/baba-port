//! SDL2 backend: Windows, Linux (X11/Wayland/KMSDRM), macOS, PortMaster.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use sdl2::controller::{Axis as SdlAxis, Button as SdlButton, GameController};
use sdl2::event::{Event as SdlEvent, WindowEvent};
use sdl2::keyboard::Scancode;
use sdl2::video::{FullscreenType, GLContext, GLProfile, SwapInterval, Window};

use crate::audio::{Audio, DeviceMixer, SAMPLE_RATE};
use crate::{
    Axis, Button, DeviceId, DeviceInfo, Event, GlRequest, Key, Platform, TouchPhase, WindowConfig,
};

fn e(s: impl std::fmt::Display) -> String {
    s.to_string()
}

/// SDL hints set before subsystems start.
pub fn set_hints() {
    use sdl2::hint::set;
    // Controllers keep working when the window is unfocused.
    set("SDL_JOYSTICK_ALLOW_BACKGROUND_EVENTS", "1");
    // Windows: RawInput + HIDAPI lift XInput's 4-controller limit (12
    // local players).
    set("SDL_JOYSTICK_RAWINPUT", "1");
    set("SDL_JOYSTICK_HIDAPI", "1");
    set("SDL_VIDEO_X11_NET_WM_BYPASS_COMPOSITOR", "0");
    if !cfg!(windows) {
        set("SDL_VIDEO_MINIMIZE_ON_FOCUS_LOSS", "0");
    }
    // Touch is reported as touch, not as a synthetic mouse.
    set("SDL_TOUCH_MOUSE_EVENTS", "0");
    // BABA_SDL_<X>=v becomes hint SDL_<X>=v (Proton's Wine drops SDL_*
    // variables, scripts/windows/run-wine.sh passes them renamed).
    for (k, v) in std::env::vars() {
        if let Some(name) = k.strip_prefix("BABA_SDL_") {
            set(&format!("SDL_{name}"), &v);
        }
    }
}

/// Attempts for `GlRequest::Auto`, in order.
fn gl_attempts(req: GlRequest) -> Vec<GlRequest> {
    match req {
        GlRequest::Auto
            if cfg!(any(target_arch = "aarch64", target_arch = "arm"))
                && !cfg!(target_os = "macos") =>
        {
            vec![GlRequest::Gles2, GlRequest::GlCore33, GlRequest::Gl21]
        }
        GlRequest::Auto => vec![GlRequest::GlCore33, GlRequest::Gles2, GlRequest::Gl21],
        r => vec![r],
    }
}

/// The SDL audio callback: runs our mixer.
pub struct DeviceCallback(DeviceMixer);

impl sdl2::audio::AudioCallback for DeviceCallback {
    type Channel = f32;

    fn callback(&mut self, out: &mut [f32]) {
        self.0.render(out);
    }
}

/// The SDL2 platform. Owns the window, GL context, controllers and audio
/// device.
pub struct SdlPlatform {
    sdl: sdl2::Sdl,
    video: sdl2::VideoSubsystem,
    window: Window,
    _gl_context: GLContext,
    gl_kind: GlRequest,
    pump: sdl2::EventPump,
    controllers_sys: sdl2::GameControllerSubsystem,
    controllers: BTreeMap<u32, GameController>,
    audio: Option<Audio>,
    _audio_device: Option<sdl2::audio::AudioDevice<DeviceCallback>>,
    start: Instant,
}

impl SdlPlatform {
    /// Opens the window with a GL context (made current) and returns the
    /// `glow` handle for the renderer.
    pub fn new(cfg: &WindowConfig) -> Result<(Self, glow::Context), String> {
        set_hints();
        let sdl = sdl2::init()?;
        let video = sdl.video()?;
        let mut last_err = String::new();
        let mut made = None;
        for req in gl_attempts(cfg.gl) {
            {
                let attr = video.gl_attr();
                attr.set_double_buffer(true);
                attr.set_depth_size(0);
                attr.set_stencil_size(0);
                attr.set_red_size(8);
                attr.set_green_size(8);
                attr.set_blue_size(8);
                match req {
                    GlRequest::Gles2 => {
                        attr.set_context_profile(GLProfile::GLES);
                        attr.set_context_version(2, 0);
                    }
                    GlRequest::GlCore33 => {
                        attr.set_context_profile(GLProfile::Core);
                        attr.set_context_version(3, 3);
                    }
                    GlRequest::Gl21 | GlRequest::Auto => {
                        attr.set_context_profile(GLProfile::Compatibility);
                        attr.set_context_version(2, 1);
                    }
                }
            }
            let mut b = video.window(&cfg.title, cfg.width, cfg.height);
            b.opengl().resizable().allow_highdpi().position_centered();
            if cfg.fullscreen {
                b.fullscreen_desktop();
            }
            let window = match b.build() {
                Ok(w) => w,
                Err(err) => {
                    last_err = format!("{req:?} window: {err}");
                    continue;
                }
            };
            match window.gl_create_context() {
                Ok(ctx) => {
                    made = Some((window, ctx, req));
                    break;
                }
                Err(err) => last_err = format!("{req:?} context: {err}"),
            }
        }
        let (window, gl_context, gl_kind) =
            made.ok_or_else(|| format!("no GL context ({last_err})"))?;
        window.gl_make_current(&gl_context)?;
        let interval = if cfg.vsync {
            SwapInterval::VSync
        } else {
            SwapInterval::Immediate
        };
        if let Err(err) = video.gl_set_swap_interval(interval) {
            log::warn!("swap interval: {err}");
        }
        // SAFETY: the context is current on this thread.
        let gl = unsafe {
            glow::Context::from_loader_function(|s| video.gl_get_proc_address(s) as *const _)
        };

        let controllers_sys = sdl.game_controller()?;
        for path in &cfg.controller_db {
            match controllers_sys.load_mappings(path) {
                Ok(n) => log::info!("{}: {n} controller mappings", path.display()),
                Err(err) => log::warn!("{}: {err}", path.display()),
            }
        }
        let (audio, audio_device) = if cfg.audio {
            match open_audio(&sdl) {
                Ok((a, d)) => (Some(a), Some(d)),
                Err(err) => {
                    log::warn!("no audio: {err}");
                    (None, None)
                }
            }
        } else {
            (None, None)
        };
        let pump = sdl.event_pump()?;
        let mut p = Self {
            sdl,
            video,
            window,
            _gl_context: gl_context,
            gl_kind,
            pump,
            controllers_sys,
            controllers: BTreeMap::new(),
            audio,
            _audio_device: audio_device,
            start: Instant::now(),
        };
        // Controllers present at start arrive as ControllerDeviceAdded
        // events too; open them now so `devices()` is complete at once.
        if let Ok(n) = p.controllers_sys.num_joysticks() {
            for i in 0..n {
                p.open_controller(i);
            }
        }
        Ok((p, gl))
    }

    pub fn sdl(&self) -> &sdl2::Sdl {
        &self.sdl
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    fn open_controller(&mut self, index: u32) -> Option<DeviceInfo> {
        if !self.controllers_sys.is_game_controller(index) {
            return None;
        }
        let c = self.controllers_sys.open(index).ok()?;
        let id = c.instance_id();
        let info = controller_info(&c);
        self.controllers.insert(id, c);
        Some(info)
    }

    fn scale(&self) -> (f32, f32) {
        let (w, h) = self.window.size();
        let (dw, dh) = self.window.drawable_size();
        (dw as f32 / w.max(1) as f32, dh as f32 / h.max(1) as f32)
    }
}

impl SdlPlatform {
    /// Dev/test helper: plugs in a virtual controller (SDL >= 2.0.14; SDL
    /// maps it like an Xbox pad). It arrives as `Event::ControllerAdded` on
    /// the next poll, like a real hot-plug. Returns its device index.
    pub fn attach_virtual_controller(&mut self) -> Result<u32, String> {
        use sdl2::sys::{SDL_JoystickAttachVirtual, SDL_JoystickType};
        // SAFETY: plain SDL call; the joystick subsystem is initialised.
        let i = unsafe {
            SDL_JoystickAttachVirtual(SDL_JoystickType::SDL_JOYSTICK_TYPE_GAMECONTROLLER, 6, 15, 0)
        };
        if i < 0 {
            return Err(sdl2::get_error());
        }
        Ok(i as u32)
    }

    /// Unplugs a virtual controller by its current device index.
    pub fn detach_virtual_controller(&mut self, device_index: u32) -> Result<(), String> {
        // SAFETY: plain SDL call.
        if unsafe { sdl2::sys::SDL_JoystickDetachVirtual(device_index as i32) } < 0 {
            return Err(sdl2::get_error());
        }
        Ok(())
    }

    /// Presses / releases a button (SDL controller button number, 0 = A /
    /// South) on an open virtual controller.
    pub fn set_virtual_button(
        &mut self,
        device: DeviceId,
        button: u8,
        down: bool,
    ) -> Result<(), String> {
        let DeviceId::Controller(id) = device else {
            return Err("not a controller".into());
        };
        // SAFETY: the instance id belongs to a joystick we keep open.
        unsafe {
            let j = sdl2::sys::SDL_JoystickFromInstanceID(id as i32);
            if j.is_null()
                || sdl2::sys::SDL_JoystickSetVirtualButton(j, button as i32, down as u8) < 0
            {
                return Err(sdl2::get_error());
            }
        }
        Ok(())
    }
}

fn controller_info(c: &GameController) -> DeviceInfo {
    DeviceInfo {
        id: DeviceId::Controller(c.instance_id()),
        name: c.name(),
        guid: c
            .mapping()
            .split(',')
            .next()
            .unwrap_or_default()
            .to_string(),
        mapped: true,
    }
}

fn open_audio(
    sdl: &sdl2::Sdl,
) -> Result<(Audio, sdl2::audio::AudioDevice<DeviceCallback>), String> {
    let sys = sdl.audio()?;
    let desired = sdl2::audio::AudioSpecDesired {
        freq: Some(SAMPLE_RATE as i32),
        channels: Some(2),
        samples: Some(1024),
    };
    let mut front = None;
    let device = sys.open_playback(None, &desired, |spec| {
        let (a, mixer) = Audio::device(
            spec.freq as u32,
            format!(
                "SDL audio ({}), {} Hz, {} ch, {} frames",
                sys.current_audio_driver(),
                spec.freq,
                spec.channels,
                spec.samples
            ),
        );
        front = Some(a);
        DeviceCallback(mixer)
    })?;
    if device.spec().channels != 2 {
        return Err(format!(
            "device opened with {} channels",
            device.spec().channels
        ));
    }
    device.resume();
    let front = front.expect("callback constructed");
    log::info!("{}", front.description());
    Ok((front, device))
}

impl Platform for SdlPlatform {
    fn describe(&self) -> String {
        let v = sdl2::version::version();
        format!(
            "SDL {}.{}.{} ({} video), {:?} context",
            v.major,
            v.minor,
            v.patch,
            self.video.current_video_driver(),
            self.gl_kind
        )
    }

    fn drawable_size(&self) -> (u32, u32) {
        self.window.drawable_size()
    }

    fn poll_events(&mut self, out: &mut Vec<Event>) {
        let (sx, sy) = self.scale();
        let events: Vec<SdlEvent> = self.pump.poll_iter().collect();
        for ev in events {
            match ev {
                SdlEvent::Quit { .. } => out.push(Event::Quit),
                SdlEvent::Window { win_event, .. } => match win_event {
                    WindowEvent::SizeChanged(..) | WindowEvent::Resized(..) => {
                        let (width, height) = self.window.drawable_size();
                        out.push(Event::Resized { width, height });
                    }
                    WindowEvent::FocusGained => out.push(Event::Focus(true)),
                    WindowEvent::FocusLost => out.push(Event::Focus(false)),
                    _ => {}
                },
                SdlEvent::AppWillEnterBackground { .. } => out.push(Event::Suspend),
                SdlEvent::AppDidEnterForeground { .. } => {
                    out.push(Event::Resume { gl_lost: false })
                }
                SdlEvent::KeyDown {
                    scancode: Some(sc),
                    repeat,
                    ..
                } => out.push(Event::Key {
                    key: map_key(sc),
                    down: true,
                    repeat,
                }),
                SdlEvent::KeyUp {
                    scancode: Some(sc), ..
                } => out.push(Event::Key {
                    key: map_key(sc),
                    down: false,
                    repeat: false,
                }),
                SdlEvent::ControllerDeviceAdded { which, .. } => {
                    if let Some(info) = self.open_controller(which) {
                        log::info!("controller added: {} ({:?})", info.name, info.id);
                        out.push(Event::ControllerAdded(info));
                    }
                }
                SdlEvent::ControllerDeviceRemoved { which, .. } => {
                    if self.controllers.remove(&which).is_some() {
                        log::info!("controller removed: {which}");
                        out.push(Event::ControllerRemoved(DeviceId::Controller(which)));
                    }
                }
                SdlEvent::ControllerButtonDown { which, button, .. } => out.push(Event::Button {
                    device: DeviceId::Controller(which),
                    button: map_button(button),
                    down: true,
                }),
                SdlEvent::ControllerButtonUp { which, button, .. } => out.push(Event::Button {
                    device: DeviceId::Controller(which),
                    button: map_button(button),
                    down: false,
                }),
                SdlEvent::ControllerAxisMotion {
                    which, axis, value, ..
                } => {
                    let (axis, v) = match axis {
                        SdlAxis::LeftX => (Axis::LeftX, value as f32 / 32767.0),
                        SdlAxis::LeftY => (Axis::LeftY, value as f32 / 32767.0),
                        SdlAxis::RightX => (Axis::RightX, value as f32 / 32767.0),
                        SdlAxis::RightY => (Axis::RightY, value as f32 / 32767.0),
                        SdlAxis::TriggerLeft => (Axis::TriggerLeft, value as f32 / 32767.0),
                        SdlAxis::TriggerRight => (Axis::TriggerRight, value as f32 / 32767.0),
                    };
                    out.push(Event::Axis {
                        device: DeviceId::Controller(which),
                        axis,
                        value: v.clamp(-1.0, 1.0),
                    });
                }
                SdlEvent::MouseMotion { x, y, .. } => out.push(Event::MouseMove {
                    x: x as f32 * sx,
                    y: y as f32 * sy,
                }),
                SdlEvent::MouseButtonDown {
                    mouse_btn, x, y, ..
                } => out.push(Event::MouseButton {
                    button: mouse_btn as u8,
                    down: true,
                    x: x as f32 * sx,
                    y: y as f32 * sy,
                }),
                SdlEvent::MouseButtonUp {
                    mouse_btn, x, y, ..
                } => out.push(Event::MouseButton {
                    button: mouse_btn as u8,
                    down: false,
                    x: x as f32 * sx,
                    y: y as f32 * sy,
                }),
                SdlEvent::MouseWheel {
                    precise_x,
                    precise_y,
                    ..
                } => out.push(Event::MouseWheel {
                    dx: precise_x,
                    dy: precise_y,
                }),
                SdlEvent::FingerDown {
                    finger_id, x, y, ..
                } => out.push(Event::Touch {
                    finger: finger_id as u64,
                    phase: TouchPhase::Down,
                    x,
                    y,
                }),
                SdlEvent::FingerMotion {
                    finger_id, x, y, ..
                } => out.push(Event::Touch {
                    finger: finger_id as u64,
                    phase: TouchPhase::Move,
                    x,
                    y,
                }),
                SdlEvent::FingerUp {
                    finger_id, x, y, ..
                } => out.push(Event::Touch {
                    finger: finger_id as u64,
                    phase: TouchPhase::Up,
                    x,
                    y,
                }),
                _ => {}
            }
        }
    }

    fn swap_buffers(&mut self) {
        self.window.gl_swap_window();
    }

    fn set_fullscreen(&mut self, on: bool) {
        let t = if on {
            FullscreenType::Desktop
        } else {
            FullscreenType::Off
        };
        if let Err(err) = self.window.set_fullscreen(t) {
            log::warn!("fullscreen: {err}");
        }
    }

    fn time(&self) -> Duration {
        self.start.elapsed()
    }

    fn audio(&mut self) -> Option<&mut Audio> {
        self.audio.as_mut()
    }

    fn devices(&self) -> Vec<DeviceInfo> {
        let mut out = vec![DeviceInfo {
            id: DeviceId::Keyboard,
            name: "Keyboard".into(),
            guid: String::new(),
            mapped: true,
        }];
        out.extend(self.controllers.values().map(controller_info));
        // Joysticks SDL knows but cannot map (no events; listed so the user
        // can add a mapping).
        if let Ok(n) = self.controllers_sys.num_joysticks() {
            for i in 0..n {
                if !self.controllers_sys.is_game_controller(i) {
                    out.push(DeviceInfo {
                        id: DeviceId::Controller(u32::MAX - i),
                        name: self.controllers_sys.name_for_index(i).unwrap_or_else(e),
                        guid: String::new(),
                        mapped: false,
                    });
                }
            }
        }
        out
    }

    fn rumble(&mut self, device: DeviceId, low: f32, high: f32, ms: u32) {
        if let DeviceId::Controller(id) = device
            && let Some(c) = self.controllers.get_mut(&id)
        {
            let q = |v: f32| (v.clamp(0.0, 1.0) * 65535.0) as u16;
            let _ = c.set_rumble(q(low), q(high), ms);
        }
    }
}

fn map_key(sc: Scancode) -> Key {
    use Scancode as S;
    match sc {
        S::Up => Key::Up,
        S::Down => Key::Down,
        S::Left => Key::Left,
        S::Right => Key::Right,
        S::W => Key::W,
        S::A => Key::A,
        S::S => Key::S,
        S::D => Key::D,
        S::Z => Key::Z,
        S::R => Key::R,
        S::Q => Key::Q,
        S::E => Key::E,
        S::P => Key::P,
        S::M => Key::M,
        S::Space => Key::Space,
        S::Return | S::KpEnter => Key::Enter,
        S::Escape => Key::Escape,
        S::Backspace => Key::Backspace,
        S::Tab => Key::Tab,
        S::LCtrl | S::RCtrl => Key::Ctrl,
        S::LShift | S::RShift => Key::Shift,
        S::LAlt | S::RAlt => Key::Alt,
        S::AcBack => Key::Back,
        S::Menu | S::Application => Key::Menu,
        _ => {
            let n = sc as i32;
            // SDL scancodes: A=4..Z=29, 1=30..9=38, 0=39, F1=58..F12=69.
            match n {
                4..=29 => Key::Char(b'a' + (n - 4) as u8),
                30..=38 => Key::Char(b'1' + (n - 30) as u8),
                39 => Key::Char(b'0'),
                58..=69 => Key::F((n - 57) as u8),
                _ => Key::Other(n as u32),
            }
        }
    }
}

fn map_button(b: SdlButton) -> Button {
    use SdlButton as B;
    match b {
        B::A => Button::South,
        B::B => Button::East,
        B::X => Button::West,
        B::Y => Button::North,
        B::Back => Button::Back,
        B::Guide => Button::Guide,
        B::Start => Button::Start,
        B::LeftStick => Button::LeftStick,
        B::RightStick => Button::RightStick,
        B::LeftShoulder => Button::LeftShoulder,
        B::RightShoulder => Button::RightShoulder,
        B::DPadUp => Button::DPadUp,
        B::DPadDown => Button::DPadDown,
        B::DPadLeft => Button::DPadLeft,
        B::DPadRight => Button::DPadRight,
        B::Misc1 => Button::Misc,
        other => Button::Other(other as u8),
    }
}

/// Facts for `--list-devices` that need SDL but no window.
pub fn describe_system() -> Result<Vec<String>, String> {
    set_hints();
    let sdl = sdl2::init()?;
    let v = sdl2::version::version();
    let mut out = vec![format!("SDL {}.{}.{}", v.major, v.minor, v.patch)];
    let drivers: Vec<_> = sdl2::video::drivers().collect();
    out.push(format!("video drivers: {}", drivers.join(", ")));
    match sdl.video() {
        Ok(video) => {
            out.push(format!("video driver: {}", video.current_video_driver()));
            for d in 0..video.num_video_displays().unwrap_or(0) {
                let name = video.display_name(d).unwrap_or_default();
                let mode = video
                    .current_display_mode(d)
                    .map(|m| format!("{}x{} @{} Hz", m.w, m.h, m.refresh_rate))
                    .unwrap_or_default();
                out.push(format!("display {d}: {name} {mode}"));
            }
        }
        Err(err) => out.push(format!("video: {err}")),
    }
    let adrivers: Vec<_> = sdl2::audio::drivers().collect();
    out.push(format!("audio drivers: {}", adrivers.join(", ")));
    match sdl.audio() {
        Ok(a) => {
            out.push(format!("audio driver: {}", a.current_audio_driver()));
            for i in 0..a.num_audio_playback_devices().unwrap_or(0) {
                out.push(format!(
                    "audio device {i}: {}",
                    a.audio_playback_device_name(i).unwrap_or_default()
                ));
            }
        }
        Err(err) => out.push(format!("audio: {err}")),
    }
    Ok(out)
}

/// SDL audio without a window (audio tests, `--audio-test`).
pub struct SdlAudio {
    _sdl: sdl2::Sdl,
    _device: sdl2::audio::AudioDevice<DeviceCallback>,
    pub audio: Audio,
}

impl SdlAudio {
    pub fn open() -> Result<Self, String> {
        set_hints();
        let sdl = sdl2::init()?;
        let (audio, device) = open_audio(&sdl)?;
        Ok(Self {
            _sdl: sdl,
            _device: device,
            audio,
        })
    }
}
