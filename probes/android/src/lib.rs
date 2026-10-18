//! Android platform probe: a NativeActivity with no Java at all.
//!
//! Proves the pieces baba-port needs on Android: fullscreen (immersive) GLES2
//! through EGL + glow, a textured quad, an AAudio beep, touch and gamepad
//! input from the NDK input queue, and running code on the Java UI thread
//! through raw JNI (no Java classes in the APK). Everything is logged under
//! the logcat tag `babaprobe`.
#![cfg(target_os = "android")]

use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use android_activity::input::{Axis, InputEvent, KeyAction, MotionAction};
use android_activity::{AndroidApp, InputStatus, MainEvent, PollEvent, WindowManagerFlags};
use glow::HasContext;

// ---------------------------------------------------------------- logging

const TAG: &CStr = c"babaprobe";

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(prio: i32, tag: *const c_char, text: *const c_char) -> i32;
}

struct Logger;
impl log::Log for Logger {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, r: &log::Record) {
        let prio = match r.level() {
            log::Level::Error => 6,
            log::Level::Warn => 5,
            log::Level::Info => 4,
            log::Level::Debug => 3,
            log::Level::Trace => 2,
        };
        let msg = CString::new(format!("{}", r.args()).replace('\0', "?")).unwrap();
        unsafe { __android_log_write(prio, TAG.as_ptr(), msg.as_ptr()) };
    }
    fn flush(&self) {}
}
static LOGGER: Logger = Logger;

macro_rules! info { ($($t:tt)*) => { log::info!($($t)*) } }

// ---------------------------------------------------------------- EGL (just what we use)

type EglDisplay = *mut c_void;
type EglConfig = *mut c_void;
type EglContext = *mut c_void;
type EglSurface = *mut c_void;

#[link(name = "EGL")]
unsafe extern "C" {
    fn eglGetDisplay(id: *mut c_void) -> EglDisplay;
    fn eglInitialize(d: EglDisplay, major: *mut i32, minor: *mut i32) -> u32;
    fn eglChooseConfig(
        d: EglDisplay,
        attrs: *const i32,
        cfgs: *mut EglConfig,
        n: i32,
        num: *mut i32,
    ) -> u32;
    fn eglGetConfigAttrib(d: EglDisplay, c: EglConfig, attr: i32, v: *mut i32) -> u32;
    fn eglCreateContext(
        d: EglDisplay,
        c: EglConfig,
        share: EglContext,
        attrs: *const i32,
    ) -> EglContext;
    fn eglCreateWindowSurface(
        d: EglDisplay,
        c: EglConfig,
        win: *mut c_void,
        attrs: *const i32,
    ) -> EglSurface;
    fn eglDestroySurface(d: EglDisplay, s: EglSurface) -> u32;
    fn eglMakeCurrent(d: EglDisplay, draw: EglSurface, read: EglSurface, ctx: EglContext) -> u32;
    fn eglSwapBuffers(d: EglDisplay, s: EglSurface) -> u32;
    fn eglQuerySurface(d: EglDisplay, s: EglSurface, attr: i32, v: *mut i32) -> u32;
    fn eglGetError() -> i32;
}
#[link(name = "android")]
unsafe extern "C" {
    fn ANativeWindow_setBuffersGeometry(
        w: *mut c_void,
        width: i32,
        height: i32,
        format: i32,
    ) -> i32;
}

const EGL_NONE: i32 = 0x3038;
const EGL_RED_SIZE: i32 = 0x3024;
const EGL_GREEN_SIZE: i32 = 0x3023;
const EGL_BLUE_SIZE: i32 = 0x3022;
const EGL_SURFACE_TYPE: i32 = 0x3033;
const EGL_WINDOW_BIT: i32 = 0x0004;
const EGL_RENDERABLE_TYPE: i32 = 0x3040;
const EGL_OPENGL_ES2_BIT: i32 = 0x0004;
const EGL_NATIVE_VISUAL_ID: i32 = 0x302E;
const EGL_CONTEXT_CLIENT_VERSION: i32 = 0x3098;
const EGL_WIDTH: i32 = 0x3057;
const EGL_HEIGHT: i32 = 0x3056;

struct Egl {
    display: EglDisplay,
    config: EglConfig,
    context: EglContext,
    surface: EglSurface,
}

impl Egl {
    fn new() -> Option<Egl> {
        unsafe {
            let display = eglGetDisplay(std::ptr::null_mut());
            let (mut maj, mut min) = (0, 0);
            if eglInitialize(display, &mut maj, &mut min) == 0 {
                log::error!("eglInitialize failed: {:#x}", eglGetError());
                return None;
            }
            let attrs = [
                EGL_SURFACE_TYPE,
                EGL_WINDOW_BIT,
                EGL_RENDERABLE_TYPE,
                EGL_OPENGL_ES2_BIT,
                EGL_RED_SIZE,
                8,
                EGL_GREEN_SIZE,
                8,
                EGL_BLUE_SIZE,
                8,
                EGL_NONE,
            ];
            let mut config = std::ptr::null_mut();
            let mut n = 0;
            if eglChooseConfig(display, attrs.as_ptr(), &mut config, 1, &mut n) == 0 || n == 0 {
                log::error!("eglChooseConfig failed: {:#x}", eglGetError());
                return None;
            }
            let ctx_attrs = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
            let context =
                eglCreateContext(display, config, std::ptr::null_mut(), ctx_attrs.as_ptr());
            if context.is_null() {
                log::error!("eglCreateContext failed: {:#x}", eglGetError());
                return None;
            }
            info!("EGL {maj}.{min} initialised");
            Some(Egl {
                display,
                config,
                context,
                surface: std::ptr::null_mut(),
            })
        }
    }

    fn attach(&mut self, window: &ndk::native_window::NativeWindow) -> bool {
        unsafe {
            let mut visual = 0;
            eglGetConfigAttrib(self.display, self.config, EGL_NATIVE_VISUAL_ID, &mut visual);
            let w = window.ptr().as_ptr() as *mut c_void;
            ANativeWindow_setBuffersGeometry(w, 0, 0, visual);
            self.surface = eglCreateWindowSurface(self.display, self.config, w, std::ptr::null());
            if self.surface.is_null() {
                log::error!("eglCreateWindowSurface failed: {:#x}", eglGetError());
                return false;
            }
            eglMakeCurrent(self.display, self.surface, self.surface, self.context) != 0
        }
    }

    fn detach(&mut self) {
        unsafe {
            let none = std::ptr::null_mut();
            eglMakeCurrent(self.display, none, none, none);
            if !self.surface.is_null() {
                eglDestroySurface(self.display, self.surface);
            }
        }
        self.surface = std::ptr::null_mut();
    }

    fn size(&self) -> (i32, i32) {
        let (mut w, mut h) = (0, 0);
        unsafe {
            eglQuerySurface(self.display, self.surface, EGL_WIDTH, &mut w);
            eglQuerySurface(self.display, self.surface, EGL_HEIGHT, &mut h);
        }
        (w, h)
    }
}

/// glow loader: core GLES2 entry points come straight from libGLESv2.so.
fn load_gl() -> glow::Context {
    unsafe {
        let lib = libc::dlopen(c"libGLESv2.so".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        glow::Context::from_loader_function(|name| {
            let c = CString::new(name).unwrap();
            libc::dlsym(lib, c.as_ptr()) as *const c_void
        })
    }
}

// ---------------------------------------------------------------- renderer

const VS: &str = "attribute vec2 a_pos;
uniform vec4 u_rect; // centre xy, half size xy (clip space)
uniform float u_scroll;
varying vec2 v_uv;
void main() {
    v_uv = a_pos * 0.5 + 0.5 + vec2(u_scroll, 0.0);
    gl_Position = vec4(u_rect.xy + a_pos * u_rect.zw, 0.0, 1.0);
}";
const FS: &str = "precision mediump float;
varying vec2 v_uv;
uniform sampler2D u_tex;
void main() { gl_FragColor = texture2D(u_tex, v_uv); }";

/// Probe colours (also checked in the screencap by scripts/android/deploy.sh).
const CLEAR: [f32; 3] = [0.1, 0.2, 0.45]; // rgb(26,51,115)
const FLASH: [f32; 3] = [0.9, 0.45, 0.1];
const TEX_A: [u8; 4] = [255, 200, 0, 255];
const TEX_B: [u8; 4] = [230, 40, 120, 255];

struct Gfx {
    gl: glow::Context,
    prog: glow::Program,
    vbo: glow::Buffer,
    tex: glow::Texture,
    u_rect: Option<glow::UniformLocation>,
    u_scroll: Option<glow::UniformLocation>,
}

impl Gfx {
    fn new() -> Gfx {
        let gl = load_gl();
        unsafe {
            let s = |p: u32| gl.get_parameter_string(p);
            info!(
                "GL_VERSION={} GL_RENDERER={} GL_VENDOR={}",
                s(glow::VERSION),
                s(glow::RENDERER),
                s(glow::VENDOR)
            );
            let prog = gl.create_program().unwrap();
            for (kind, src) in [(glow::VERTEX_SHADER, VS), (glow::FRAGMENT_SHADER, FS)] {
                let sh = gl.create_shader(kind).unwrap();
                gl.shader_source(sh, src);
                gl.compile_shader(sh);
                if !gl.get_shader_compile_status(sh) {
                    log::error!("shader: {}", gl.get_shader_info_log(sh));
                }
                gl.attach_shader(prog, sh);
            }
            gl.bind_attrib_location(prog, 0, "a_pos");
            gl.link_program(prog);
            if !gl.get_program_link_status(prog) {
                log::error!("link: {}", gl.get_program_info_log(prog));
            }
            let quad: [f32; 8] = [-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0];
            let bytes: Vec<u8> = quad.iter().flat_map(|f| f.to_ne_bytes()).collect();
            let vbo = gl.create_buffer().unwrap();
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &bytes, glow::STATIC_DRAW);

            // 64x64 checkerboard, 8 px cells.
            let mut px = Vec::with_capacity(64 * 64 * 4);
            for y in 0..64 {
                for x in 0..64 {
                    px.extend_from_slice(if (x / 8 + y / 8) % 2 == 0 {
                        &TEX_A
                    } else {
                        &TEX_B
                    });
                }
            }
            let tex = gl.create_texture().unwrap();
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                64,
                64,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&px)),
            );
            for (p, v) in [
                (glow::TEXTURE_MIN_FILTER, glow::NEAREST),
                (glow::TEXTURE_MAG_FILTER, glow::NEAREST),
                (glow::TEXTURE_WRAP_S, glow::REPEAT),
                (glow::TEXTURE_WRAP_T, glow::REPEAT),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, p, v as i32);
            }
            let u_rect = gl.get_uniform_location(prog, "u_rect");
            let u_scroll = gl.get_uniform_location(prog, "u_scroll");
            Gfx {
                gl,
                prog,
                vbo,
                tex,
                u_rect,
                u_scroll,
            }
        }
    }

    fn draw(&self, w: i32, h: i32, flash: bool, t: f32, touch: Option<(f32, f32)>) {
        let gl = &self.gl;
        let c = if flash { FLASH } else { CLEAR };
        unsafe {
            gl.viewport(0, 0, w, h);
            gl.clear_color(c[0], c[1], c[2], 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.use_program(Some(self.prog));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 0, 0);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.tex));
            // Square quad, half the shorter side, centred; texture scrolls slowly.
            let aspect = w as f32 / h.max(1) as f32;
            let (hx, hy) = if aspect > 1.0 {
                (0.5 / aspect, 0.5)
            } else {
                (0.5, 0.5 * aspect)
            };
            gl.uniform_4_f32(self.u_rect.as_ref(), 0.0, 0.0, hx, hy);
            gl.uniform_1_f32(self.u_scroll.as_ref(), (t * 0.1).fract());
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            if let Some((x, y)) = touch {
                let (cx, cy) = (x / w as f32 * 2.0 - 1.0, 1.0 - y / h as f32 * 2.0);
                gl.uniform_4_f32(self.u_rect.as_ref(), cx, cy, hx * 0.15, hy * 0.15);
                gl.uniform_1_f32(self.u_scroll.as_ref(), 0.0);
                gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            }
        }
    }
}

// ---------------------------------------------------------------- audio (AAudio)

static BEEP_FRAMES: AtomicU32 = AtomicU32::new(0);
static BEEP_HZ: AtomicU32 = AtomicU32::new(440);

fn beep(hz: u32, ms: u32) {
    BEEP_HZ.store(hz, Ordering::Relaxed);
    BEEP_FRAMES.store(48 * ms, Ordering::Relaxed);
}

fn open_audio() -> Option<ndk::audio::AudioStream> {
    use ndk::audio::*;
    let mut phase = 0f32;
    let stream = AudioStreamBuilder::new()
        .ok()?
        .direction(AudioDirection::Output)
        .format(AudioFormat::PCM_Float)
        .channel_count(1)
        .sample_rate(48000)
        .performance_mode(AudioPerformanceMode::LowLatency)
        .data_callback(Box::new(move |s, data, frames| {
            let out = unsafe { std::slice::from_raw_parts_mut(data as *mut f32, frames as usize) };
            let rate = s.sample_rate() as f32;
            let hz = BEEP_HZ.load(Ordering::Relaxed) as f32;
            for o in out.iter_mut() {
                let left = BEEP_FRAMES.load(Ordering::Relaxed);
                if left > 0 {
                    BEEP_FRAMES.store(left - 1, Ordering::Relaxed);
                    phase = (phase + hz / rate).fract();
                    *o = (phase * std::f32::consts::TAU).sin() * 0.25;
                } else {
                    *o = 0.0;
                }
            }
            AudioCallbackResult::Continue
        }))
        .open_stream();
    match stream {
        Ok(s) => {
            if let Err(e) = s.request_start() {
                log::error!("AAudio start: {e:?}");
            }
            info!(
                "AAudio open: rate={} burst={} mode={:?}",
                s.sample_rate(),
                s.frames_per_burst(),
                s.performance_mode()
            );
            Some(s)
        }
        Err(e) => {
            log::error!("AAudio open: {e:?}");
            None
        }
    }
}

// ---------------------------------------------------------------- raw JNI (UI thread)

/// Calls entry `idx` of a JNI function table (`JNINativeInterface` /
/// `JNIInvokeInterface`; indices are fixed by the JNI spec).
unsafe fn jfn<F: Copy>(obj: *mut c_void, idx: usize) -> F {
    unsafe {
        let table = *(obj as *const *const *const c_void);
        std::mem::transmute_copy(&*table.add(idx))
    }
}

type JObj = *mut c_void;
type GetObjectClass = unsafe extern "C" fn(JObj, JObj) -> JObj;
type GetMethodId = unsafe extern "C" fn(JObj, JObj, *const c_char, *const c_char) -> JObj;
type CallObjectA = unsafe extern "C" fn(JObj, JObj, JObj, *const u64) -> JObj;
type CallVoidA = unsafe extern "C" fn(JObj, JObj, JObj, *const u64);
type ExcCheck = unsafe extern "C" fn(JObj) -> u8;
type ExcVoid = unsafe extern "C" fn(JObj);
type GetEnv = unsafe extern "C" fn(JObj, *mut JObj, i32) -> i32;

unsafe fn call(env: JObj, obj: JObj, name: &CStr, sig: &CStr, arg: Option<i32>) -> Option<JObj> {
    unsafe {
        let class = jfn::<GetObjectClass>(env, 31)(env, obj);
        let m = jfn::<GetMethodId>(env, 33)(env, class, name.as_ptr(), sig.as_ptr());
        let ok = |env| {
            if jfn::<ExcCheck>(env, 228)(env) != 0 {
                jfn::<ExcVoid>(env, 16)(env); // ExceptionDescribe -> logcat
                jfn::<ExcVoid>(env, 17)(env); // ExceptionClear
                false
            } else {
                true
            }
        };
        if m.is_null() || !ok(env) {
            return None;
        }
        let r = match arg {
            None => jfn::<CallObjectA>(env, 36)(env, obj, m, std::ptr::null()),
            Some(v) => {
                let args = [v as u32 as u64];
                jfn::<CallVoidA>(env, 63)(env, obj, m, args.as_ptr());
                std::ptr::null_mut()
            }
        };
        ok(env).then_some(r)
    }
}

/// Immersive fullscreen. Must run on the Java UI thread.
fn hide_system_bars(app: &AndroidApp) {
    let (vm, activity) = (app.vm_as_ptr(), app.activity_as_ptr());
    let sdk = AndroidApp::sdk_version();
    unsafe {
        let mut env = std::ptr::null_mut();
        if jfn::<GetEnv>(vm, 6)(vm, &mut env, 0x0001_0006) != 0 {
            log::error!("GetEnv failed");
            return;
        }
        let Some(window) = call(
            env,
            activity,
            c"getWindow",
            c"()Landroid/view/Window;",
            None,
        ) else {
            return;
        };
        let done = if sdk >= 30 {
            call(
                env,
                window,
                c"getInsetsController",
                c"()Landroid/view/WindowInsetsController;",
                None,
            )
            .filter(|c| !c.is_null())
            .and_then(|ctl| {
                // BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE; WindowInsets.Type.systemBars()
                call(env, ctl, c"setSystemBarsBehavior", c"(I)V", Some(2))?;
                call(env, ctl, c"hide", c"(I)V", Some(7))
            })
            .is_some()
        } else {
            call(env, window, c"getDecorView", c"()Landroid/view/View;", None)
                .and_then(|v| call(env, v, c"setSystemUiVisibility", c"(I)V", Some(0x1706)))
                .is_some()
        };
        info!(
            "immersive fullscreen (sdk {sdk}): {}",
            if done { "ok" } else { "failed" }
        );
    }
}

// ---------------------------------------------------------------- main loop

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Info);
    std::panic::set_hook(Box::new(|p| log::error!("panic: {p}")));
    info!(
        "android_main: sdk={} pid={} probe={}",
        AndroidApp::sdk_version(),
        std::process::id(),
        env!("CARGO_PKG_VERSION")
    );
    app.set_window_flags(
        WindowManagerFlags::KEEP_SCREEN_ON | WindowManagerFlags::FULLSCREEN,
        WindowManagerFlags::empty(),
    );
    let audio = open_audio();
    beep(440, 300);

    let start = Instant::now();
    let mut egl: Option<Egl> = None;
    let mut gfx: Option<Gfx> = None;
    let mut has_surface = false;
    let mut quit = false;
    let mut flash_until = start;
    let mut touch: Option<(f32, f32)> = None;
    let mut frames = 0u64;
    let mut moves = 0u32;
    let mut last_report = start;

    while !quit {
        let timeout = if has_surface {
            Some(Duration::ZERO)
        } else {
            None
        };
        app.poll_events(timeout, |ev| {
            if let PollEvent::Main(ev) = ev {
                match ev {
                    MainEvent::InitWindow { .. } => {
                        if let Some(win) = app.native_window() {
                            let e = egl.get_or_insert_with(|| Egl::new().expect("EGL"));
                            has_surface = e.attach(&win);
                            if has_surface && gfx.is_none() {
                                gfx = Some(Gfx::new());
                            }
                            info!(
                                "window: {}x{} surface={has_surface}",
                                win.width(),
                                win.height()
                            );
                        }
                    }
                    MainEvent::TerminateWindow { .. } => {
                        if let Some(e) = egl.as_mut() {
                            e.detach();
                        }
                        has_surface = false;
                        info!("window terminated");
                    }
                    MainEvent::GainedFocus => {
                        info!("focus gained");
                        let a = app.clone();
                        app.run_on_java_main_thread(Box::new(move || hide_system_bars(&a)));
                    }
                    MainEvent::LostFocus => info!("focus lost"),
                    MainEvent::Pause => info!("pause"),
                    MainEvent::Resume { .. } => info!("resume"),
                    MainEvent::Destroy => {
                        info!("destroy");
                        quit = true;
                    }
                    _ => {}
                }
            }
        });

        // Input: touch, keys, gamepad buttons and sticks.
        if let Ok(mut iter) = app.input_events_iter() {
            loop {
                let more = iter.next(|ev| match ev {
                    InputEvent::MotionEvent(m) => {
                        let src = m.source();
                        let p = m.pointer_at_index(m.pointer_index());
                        match src {
                            s if s.is_joystick_class() => {
                                info!(
                                    "gamepad axes dev={} x={:.2} y={:.2} rx={:.2} ry={:.2} hat=({:.0},{:.0}) lt={:.2} rt={:.2}",
                                    m.device_id(),
                                    p.axis_value(Axis::X),
                                    p.axis_value(Axis::Y),
                                    p.axis_value(Axis::Z),
                                    p.axis_value(Axis::Rz),
                                    p.axis_value(Axis::HatX),
                                    p.axis_value(Axis::HatY),
                                    p.axis_value(Axis::Ltrigger),
                                    p.axis_value(Axis::Rtrigger),
                                );
                                InputStatus::Handled
                            }
                            _ => {
                                let (x, y) = (p.x(), p.y());
                                match m.action() {
                                    MotionAction::Down | MotionAction::PointerDown => {
                                        info!(
                                            "touch down id={} at ({x:.0},{y:.0}) pointers={} src={:#x}",
                                            p.pointer_id(),
                                            m.pointer_count(),
                                            u32::from(src)
                                        );
                                        flash_until = Instant::now() + Duration::from_millis(150);
                                        beep(660, 80);
                                        touch = Some((x, y));
                                    }
                                    MotionAction::Up | MotionAction::PointerUp => {
                                        info!(
                                            "touch up id={} at ({x:.0},{y:.0}) after {moves} moves",
                                            p.pointer_id()
                                        );
                                        moves = 0;
                                    }
                                    MotionAction::Move => {
                                        moves += 1;
                                        let p0 = m.pointer_at_index(0);
                                        touch = Some((p0.x(), p0.y()));
                                    }
                                    a => info!("motion {a:?} src={:#x}", u32::from(src)),
                                }
                                InputStatus::Handled
                            }
                        }
                    }
                    InputEvent::KeyEvent(k) => {
                        let code: u32 = k.key_code().into();
                        let down = k.action() == KeyAction::Down;
                        info!(
                            "key {} code={code} ({:?}) dev={} src={:#x} repeat={}",
                            if down { "down" } else { "up" },
                            k.key_code(),
                            k.device_id(),
                            u32::from(k.source()),
                            k.repeat_count()
                        );
                        // Let the system have BACK (finishes the activity) and volume keys.
                        const BACK: u32 = 4;
                        if code == BACK || (24..=25).contains(&code) {
                            InputStatus::Unhandled
                        } else {
                            if down && k.repeat_count() == 0 {
                                flash_until = Instant::now() + Duration::from_millis(150);
                                beep(880, 80);
                            }
                            InputStatus::Handled
                        }
                    }
                    _ => InputStatus::Unhandled,
                });
                if !more {
                    break;
                }
            }
        }

        if has_surface && let (Some(e), Some(g)) = (egl.as_ref(), gfx.as_ref()) {
            let (w, h) = e.size();
            let now = Instant::now();
            g.draw(w, h, now < flash_until, (now - start).as_secs_f32(), touch);
            unsafe { eglSwapBuffers(e.display, e.surface) };
            frames += 1;
            if now - last_report >= Duration::from_secs(5) {
                let fps = frames as f32 / (now - last_report).as_secs_f32();
                info!("frames: {fps:.1} fps, surface {w}x{h}");
                if let Some(a) = audio.as_ref() {
                    info!(
                        "audio: state={:?} frames_written={} xruns={}",
                        a.state(),
                        a.frames_written(),
                        a.x_run_count()
                    );
                }
                frames = 0;
                last_report = now;
            }
        }
    }
    info!("android_main returns");
}
