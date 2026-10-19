//! Android backend: `NativeActivity` through `android-activity`, no Java.
//!
//! - **Window/GL**: EGL 1.4 + a GLES 2 context on the activity's
//!   `ANativeWindow`, GL entry points from `libGLESv2.so` (glow). The context
//!   survives surface loss (home, screen off); only the window surface is
//!   re-created. If the context itself is lost (`EGL_CONTEXT_LOST`, or the
//!   test switch `setprop debug.babaport.losectx 1`), a new one is made and
//!   the game gets `Event::Resume { gl_lost: true }` + `take_restored_gl`.
//! - **Lifecycle**: window terminated → `Suspend` (and `poll_events` then
//!   blocks until the window is back); pause/resume stop/start the audio
//!   stream; destroy → `Quit`. Returning from `android_main` finishes the
//!   activity.
//! - **Input** from the NDK input queue: touch (multi-pointer) through the
//!   on-screen [`TouchControls`], gamepads (buttons, sticks, triggers,
//!   hat → d-pad) as controllers keyed by Android device id (hot-plug through
//!   a periodic `InputDevice.getDeviceIds()` scan), keyboards as keys, the
//!   back button as [`Key::Back`] (the game decides; it is not passed on to
//!   the system), mouse as mouse events. Volume/power keys go to the system.
//! - **Audio**: AAudio float stereo output stream whose callback runs the
//!   shared [`DeviceMixer`]; re-opened after a disconnect (headphones
//!   unplugged, output changed).
//! - **Immersive fullscreen** through raw JNI on the Java UI thread.

use std::collections::BTreeMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use android_activity::input::{Axis as AAxis, InputEvent, KeyAction, MotionAction};
use android_activity::{InputStatus, MainEvent, PollEvent, WindowManagerFlags};

/// Re-exported for the entry point (`android_main(app: AndroidApp)`).
pub use android_activity::{self, AndroidApp};

use crate::audio::{Audio, DeviceMixer, SAMPLE_RATE};
use crate::touch::TouchControls;
use crate::{Axis, Button, DeviceId, DeviceInfo, Event, Key, Platform, TouchPhase, WindowConfig};

// ---------------------------------------------------------------- EGL

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
    fn eglDestroyContext(d: EglDisplay, c: EglContext) -> u32;
    fn eglCreateWindowSurface(
        d: EglDisplay,
        c: EglConfig,
        win: *mut c_void,
        attrs: *const i32,
    ) -> EglSurface;
    fn eglDestroySurface(d: EglDisplay, s: EglSurface) -> u32;
    fn eglMakeCurrent(d: EglDisplay, draw: EglSurface, read: EglSurface, ctx: EglContext) -> u32;
    fn eglSwapBuffers(d: EglDisplay, s: EglSurface) -> u32;
    fn eglSwapInterval(d: EglDisplay, interval: i32) -> u32;
    fn eglQuerySurface(d: EglDisplay, s: EglSurface, attr: i32, v: *mut i32) -> u32;
    fn eglGetError() -> i32;
    fn eglGetProcAddress(name: *const c_char) -> *mut c_void;
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
unsafe extern "C" {
    fn __system_property_get(name: *const c_char, value: *mut c_char) -> i32;
}

const EGL_NONE: i32 = 0x3038;
const EGL_RED_SIZE: i32 = 0x3024;
const EGL_GREEN_SIZE: i32 = 0x3023;
const EGL_BLUE_SIZE: i32 = 0x3022;
const EGL_DEPTH_SIZE: i32 = 0x3025;
const EGL_SURFACE_TYPE: i32 = 0x3033;
const EGL_WINDOW_BIT: i32 = 0x0004;
const EGL_RENDERABLE_TYPE: i32 = 0x3040;
const EGL_OPENGL_ES2_BIT: i32 = 0x0004;
const EGL_NATIVE_VISUAL_ID: i32 = 0x302E;
const EGL_CONTEXT_CLIENT_VERSION: i32 = 0x3098;
const EGL_WIDTH: i32 = 0x3057;
const EGL_HEIGHT: i32 = 0x3056;
const EGL_CONTEXT_LOST: i32 = 0x300E;

/// Reads an Android system property (`getprop`).
pub fn system_property(name: &str) -> Option<String> {
    let name = CString::new(name).ok()?;
    let mut buf = [0 as c_char; 92]; // PROP_VALUE_MAX
    let n = unsafe { __system_property_get(name.as_ptr(), buf.as_mut_ptr()) };
    (n > 0).then(|| {
        unsafe { CStr::from_ptr(buf.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    })
}

struct Egl {
    display: EglDisplay,
    config: EglConfig,
    context: EglContext,
    surface: EglSurface,
    vsync: bool,
}

fn egl_err(what: &str) -> String {
    format!("{what} failed: EGL error {:#x}", unsafe { eglGetError() })
}

impl Egl {
    fn new(vsync: bool) -> Result<Egl, String> {
        unsafe {
            let display = eglGetDisplay(std::ptr::null_mut());
            let (mut maj, mut min) = (0, 0);
            if eglInitialize(display, &mut maj, &mut min) == 0 {
                return Err(egl_err("eglInitialize"));
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
                EGL_DEPTH_SIZE,
                0,
                EGL_NONE,
            ];
            let mut config = std::ptr::null_mut();
            let mut n = 0;
            if eglChooseConfig(display, attrs.as_ptr(), &mut config, 1, &mut n) == 0 || n == 0 {
                return Err(egl_err("eglChooseConfig"));
            }
            log::info!("EGL {maj}.{min}");
            let mut egl = Egl {
                display,
                config,
                context: std::ptr::null_mut(),
                surface: std::ptr::null_mut(),
                vsync,
            };
            egl.create_context()?;
            Ok(egl)
        }
    }

    fn create_context(&mut self) -> Result<(), String> {
        let attrs = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
        let ctx = unsafe {
            eglCreateContext(
                self.display,
                self.config,
                std::ptr::null_mut(),
                attrs.as_ptr(),
            )
        };
        if ctx.is_null() {
            return Err(egl_err("eglCreateContext"));
        }
        self.context = ctx;
        Ok(())
    }

    fn destroy_context(&mut self) {
        unsafe {
            let none = std::ptr::null_mut();
            eglMakeCurrent(self.display, none, none, none);
            if !self.context.is_null() {
                eglDestroyContext(self.display, self.context);
            }
        }
        self.context = std::ptr::null_mut();
    }

    /// Creates the window surface and makes the context current on it.
    /// `Err(true)` = the context was lost.
    fn attach(&mut self, window: &ndk::native_window::NativeWindow) -> Result<(), bool> {
        unsafe {
            let mut visual = 0;
            eglGetConfigAttrib(self.display, self.config, EGL_NATIVE_VISUAL_ID, &mut visual);
            let w = window.ptr().as_ptr() as *mut c_void;
            ANativeWindow_setBuffersGeometry(w, 0, 0, visual);
            self.surface = eglCreateWindowSurface(self.display, self.config, w, std::ptr::null());
            if self.surface.is_null() {
                log::error!("{}", egl_err("eglCreateWindowSurface"));
                return Err(false);
            }
            if eglMakeCurrent(self.display, self.surface, self.surface, self.context) == 0 {
                let e = eglGetError();
                log::error!("eglMakeCurrent failed: {e:#x}");
                return Err(e == EGL_CONTEXT_LOST);
            }
            eglSwapInterval(self.display, self.vsync as i32);
            Ok(())
        }
    }

    fn make_current(&self) -> bool {
        unsafe { eglMakeCurrent(self.display, self.surface, self.surface, self.context) != 0 }
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

    fn size(&self) -> (u32, u32) {
        let (mut w, mut h) = (0, 0);
        unsafe {
            eglQuerySurface(self.display, self.surface, EGL_WIDTH, &mut w);
            eglQuerySurface(self.display, self.surface, EGL_HEIGHT, &mut h);
        }
        (w.max(0) as u32, h.max(0) as u32)
    }
}

/// glow loader: core GLES2 entry points from libGLESv2.so, extensions
/// through eglGetProcAddress.
pub fn load_gl() -> glow::Context {
    unsafe {
        let lib = libc::dlopen(c"libGLESv2.so".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        glow::Context::from_loader_function(|name| {
            let c = CString::new(name).unwrap();
            let p = libc::dlsym(lib, c.as_ptr());
            if p.is_null() {
                eglGetProcAddress(c.as_ptr()) as *const c_void
            } else {
                p as *const c_void
            }
        })
    }
}

// ---------------------------------------------------------------- JNI

/// Raw JNI helpers (no Java classes in the APK; system classes only).
pub mod jni {
    use std::ffi::{CStr, CString, c_void};
    use std::marker::PhantomData;

    use jni_sys::{JNI_OK, JNI_VERSION_1_6, JNIEnv, JavaVM, jobject, jvalue};

    /// A JNI environment for the current (attached) thread, inside a local
    /// reference frame that is popped when [`with_env`]'s closure returns.
    pub struct Env(*mut JNIEnv);

    /// A local reference, valid while its [`Env`] frame lives (may be
    /// null).
    #[derive(Clone, Copy, Debug)]
    pub struct JObject<'e>(jobject, PhantomData<&'e Env>);

    impl JObject<'_> {
        pub fn is_null(self) -> bool {
            self.0.is_null()
        }
    }

    /// Runs `f` with the current thread's JNI env (the android_main thread
    /// and the Java UI thread are attached already).
    pub fn with_env<R>(vm: *mut c_void, f: impl FnOnce(&Env) -> Option<R>) -> Option<R> {
        if vm.is_null() {
            return None;
        }
        unsafe {
            let vm = vm as *mut JavaVM;
            let mut env: *mut c_void = std::ptr::null_mut();
            if ((**vm).GetEnv?)(vm, &mut env, JNI_VERSION_1_6) != JNI_OK {
                log::error!("JNI GetEnv failed");
                return None;
            }
            let env = env as *mut JNIEnv;
            if ((**env).PushLocalFrame?)(env, 64) != 0 {
                return None;
            }
            let e = Env(env);
            let r = f(&e);
            e.check();
            ((**env).PopLocalFrame?)(env, std::ptr::null_mut());
            r
        }
    }

    /// Argument for a method call.
    #[derive(Clone, Copy)]
    pub enum Arg<'e> {
        Int(i32),
        Bool(bool),
        Obj(JObject<'e>),
    }

    fn values(args: &[Arg]) -> Vec<jvalue> {
        args.iter()
            .map(|a| match *a {
                Arg::Int(i) => jvalue { i },
                Arg::Bool(b) => jvalue { z: b as u8 },
                Arg::Obj(o) => jvalue { l: o.0 },
            })
            .collect()
    }

    impl Env {
        /// Wraps a raw reference (e.g. `AndroidApp::activity_as_ptr`).
        ///
        /// # Safety
        /// `raw` must be a valid JNI reference (or null) for as long as the
        /// returned object is used.
        pub unsafe fn wrap(&self, raw: *mut c_void) -> JObject<'_> {
            JObject(raw as jobject, PhantomData)
        }

        fn obj(&self, raw: jobject) -> JObject<'_> {
            JObject(raw, PhantomData)
        }

        /// Clears a pending exception (logged); true if there was one.
        pub fn check(&self) -> bool {
            unsafe {
                let env = self.0;
                if ((**env).ExceptionCheck.unwrap())(env) != 0 {
                    ((**env).ExceptionDescribe.unwrap())(env);
                    ((**env).ExceptionClear.unwrap())(env);
                    true
                } else {
                    false
                }
            }
        }

        fn ok<T>(&self, v: T) -> Option<T> {
            (!self.check()).then_some(v)
        }

        /// A system class (`android/view/InputDevice`).
        pub fn class(&self, name: &CStr) -> Option<JObject<'_>> {
            unsafe {
                let c = ((**self.0).FindClass?)(self.0, name.as_ptr());
                self.ok(self.obj(c)).filter(|c| !c.is_null())
            }
        }

        fn method(&self, obj: JObject, name: &CStr, sig: &CStr) -> Option<jni_sys::jmethodID> {
            if obj.is_null() {
                return None;
            }
            unsafe {
                let env = self.0;
                let class = ((**env).GetObjectClass?)(env, obj.0);
                let m = ((**env).GetMethodID?)(env, class, name.as_ptr(), sig.as_ptr());
                self.ok(m).filter(|m| !m.is_null())
            }
        }

        fn static_method(
            &self,
            class: JObject,
            name: &CStr,
            sig: &CStr,
        ) -> Option<jni_sys::jmethodID> {
            unsafe {
                let env = self.0;
                let m = ((**env).GetStaticMethodID?)(env, class.0, name.as_ptr(), sig.as_ptr());
                self.ok(m).filter(|m| !m.is_null())
            }
        }

        /// `obj.name(args)` returning an object (may be null).
        pub fn call_obj(
            &self,
            obj: JObject,
            name: &CStr,
            sig: &CStr,
            args: &[Arg],
        ) -> Option<JObject<'_>> {
            let m = self.method(obj, name, sig)?;
            let a = values(args);
            unsafe {
                let r = ((**self.0).CallObjectMethodA?)(self.0, obj.0, m, a.as_ptr());
                self.ok(self.obj(r))
            }
        }

        pub fn call_int(&self, obj: JObject, name: &CStr, sig: &CStr, args: &[Arg]) -> Option<i32> {
            let m = self.method(obj, name, sig)?;
            let a = values(args);
            unsafe {
                let r = ((**self.0).CallIntMethodA?)(self.0, obj.0, m, a.as_ptr());
                self.ok(r)
            }
        }

        pub fn call_bool(
            &self,
            obj: JObject,
            name: &CStr,
            sig: &CStr,
            args: &[Arg],
        ) -> Option<bool> {
            let m = self.method(obj, name, sig)?;
            let a = values(args);
            unsafe {
                let r = ((**self.0).CallBooleanMethodA?)(self.0, obj.0, m, a.as_ptr());
                self.ok(r != 0)
            }
        }

        pub fn call_void(&self, obj: JObject, name: &CStr, sig: &CStr, args: &[Arg]) -> Option<()> {
            let m = self.method(obj, name, sig)?;
            let a = values(args);
            unsafe {
                ((**self.0).CallVoidMethodA?)(self.0, obj.0, m, a.as_ptr());
                self.ok(())
            }
        }

        pub fn call_static_obj(
            &self,
            class: JObject,
            name: &CStr,
            sig: &CStr,
            args: &[Arg],
        ) -> Option<JObject<'_>> {
            let m = self.static_method(class, name, sig)?;
            let a = values(args);
            unsafe {
                let r = ((**self.0).CallStaticObjectMethodA?)(self.0, class.0, m, a.as_ptr());
                self.ok(self.obj(r))
            }
        }

        pub fn new_string(&self, s: &str) -> Option<JObject<'_>> {
            let c = CString::new(s).ok()?;
            unsafe {
                let r = ((**self.0).NewStringUTF?)(self.0, c.as_ptr());
                self.ok(self.obj(r))
            }
        }

        /// A Java string as Rust (None for null).
        pub fn string(&self, s: JObject) -> Option<String> {
            if s.is_null() {
                return None;
            }
            unsafe {
                let env = self.0;
                let p = ((**env).GetStringUTFChars?)(env, s.0, std::ptr::null_mut());
                if p.is_null() {
                    return None;
                }
                let r = CStr::from_ptr(p).to_string_lossy().into_owned();
                ((**env).ReleaseStringUTFChars?)(env, s.0, p);
                Some(r)
            }
        }

        /// A Java `String[]` as a Vec (null elements become empty strings).
        pub fn string_array(&self, a: JObject) -> Option<Vec<String>> {
            if a.is_null() {
                return None;
            }
            unsafe {
                let env = self.0;
                let n = ((**env).GetArrayLength?)(env, a.0);
                let mut v = Vec::with_capacity(n.max(0) as usize);
                for i in 0..n {
                    let s = ((**env).GetObjectArrayElement?)(env, a.0, i);
                    let s = self.ok(self.obj(s))?;
                    v.push(self.string(s).unwrap_or_default());
                    ((**env).DeleteLocalRef?)(env, s.0);
                }
                Some(v)
            }
        }

        /// A Java `int[]` as a Vec.
        pub fn int_array(&self, a: JObject) -> Option<Vec<i32>> {
            if a.is_null() {
                return None;
            }
            unsafe {
                let env = self.0;
                let n = ((**env).GetArrayLength?)(env, a.0);
                let mut v = vec![0i32; n.max(0) as usize];
                ((**env).GetIntArrayRegion?)(env, a.0, 0, n, v.as_mut_ptr());
                self.ok(v)
            }
        }
    }
}

use jni::Arg;

/// Immersive fullscreen (status and navigation bars hidden, swipe shows
/// them transiently). Must run on the Java UI thread.
fn hide_system_bars(vm: *mut c_void, activity: *mut c_void) {
    let sdk = AndroidApp::sdk_version();
    let done = jni::with_env(vm, |e| {
        let window = e.call_obj(
            unsafe { e.wrap(activity) },
            c"getWindow",
            c"()Landroid/view/Window;",
            &[],
        )?;
        if sdk >= 30 {
            let ctl = e
                .call_obj(
                    window,
                    c"getInsetsController",
                    c"()Landroid/view/WindowInsetsController;",
                    &[],
                )
                .filter(|c| !c.is_null())?;
            // BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE; WindowInsets.Type.systemBars()
            e.call_void(ctl, c"setSystemBarsBehavior", c"(I)V", &[Arg::Int(2)])?;
            e.call_void(ctl, c"hide", c"(I)V", &[Arg::Int(7)])
        } else {
            let view = e.call_obj(window, c"getDecorView", c"()Landroid/view/View;", &[])?;
            // IMMERSIVE_STICKY | FULLSCREEN | HIDE_NAVIGATION | LAYOUT_*
            e.call_void(view, c"setSystemUiVisibility", c"(I)V", &[Arg::Int(0x1706)])
        }
    });
    log::info!(
        "immersive fullscreen (sdk {sdk}): {}",
        if done.is_some() { "ok" } else { "failed" }
    );
}

/// An input device as Android reports it.
struct AndroidDevice {
    id: i32,
    name: String,
    descriptor: String,
    sources: i32,
}

const SOURCE_GAMEPAD: i32 = 0x0000_0401;
const SOURCE_JOYSTICK: i32 = 0x0100_0010;

fn is_pad_source(s: i32) -> bool {
    s & SOURCE_GAMEPAD == SOURCE_GAMEPAD || s & SOURCE_JOYSTICK == SOURCE_JOYSTICK
}

fn input_device(e: &jni::Env, class: jni::JObject, id: i32) -> Option<AndroidDevice> {
    let dev = e
        .call_static_obj(
            class,
            c"getDevice",
            c"(I)Landroid/view/InputDevice;",
            &[Arg::Int(id)],
        )
        .filter(|d| !d.is_null())?;
    let name = e
        .call_obj(dev, c"getName", c"()Ljava/lang/String;", &[])
        .and_then(|s| e.string(s))
        .unwrap_or_default();
    let descriptor = e
        .call_obj(dev, c"getDescriptor", c"()Ljava/lang/String;", &[])
        .and_then(|s| e.string(s))
        .unwrap_or_default();
    let sources = e.call_int(dev, c"getSources", c"()I", &[]).unwrap_or(0);
    Some(AndroidDevice {
        id,
        name,
        descriptor,
        sources,
    })
}

/// Every input device (`InputDevice.getDeviceIds()`).
fn scan_input_devices(vm: *mut c_void) -> Option<Vec<AndroidDevice>> {
    jni::with_env(vm, |e| {
        let class = e.class(c"android/view/InputDevice")?;
        let ids = e.call_static_obj(class, c"getDeviceIds", c"()[I", &[])?;
        let ids = e.int_array(ids)?;
        Some(
            ids.into_iter()
                .filter_map(|id| input_device(e, class, id))
                .collect(),
        )
    })
}

fn describe_device(vm: *mut c_void, id: i32) -> Option<AndroidDevice> {
    jni::with_env(vm, |e| {
        let class = e.class(c"android/view/InputDevice")?;
        input_device(e, class, id)
    })
}

// ---------------------------------------------------------------- input mapping

/// Android key codes (`KeyEvent.KEYCODE_*`, stable values).
fn map_key(code: u32) -> Option<Key> {
    Some(match code {
        19 => Key::Up,
        20 => Key::Down,
        21 => Key::Left,
        22 => Key::Right,
        23 | 66 | 160 => Key::Enter, // DPAD_CENTER, ENTER, NUMPAD_ENTER
        62 => Key::Space,
        111 => Key::Escape,
        67 => Key::Backspace,
        61 => Key::Tab,
        113 | 114 => Key::Ctrl,
        59 | 60 => Key::Shift,
        57 | 58 => Key::Alt,
        4 => Key::Back,
        82 => Key::Menu,
        29..=54 => match b'a' + (code - 29) as u8 {
            b'w' => Key::W,
            b'a' => Key::A,
            b's' => Key::S,
            b'd' => Key::D,
            b'z' => Key::Z,
            b'r' => Key::R,
            b'q' => Key::Q,
            b'e' => Key::E,
            b'p' => Key::P,
            b'm' => Key::M,
            c => Key::Char(c),
        },
        7..=16 => Key::Char(b'0' + (code - 7) as u8),
        131..=142 => Key::F((code - 130) as u8),
        _ => return None,
    })
}

/// Gamepad key codes → positional buttons.
fn map_button(code: u32) -> Option<Button> {
    Some(match code {
        96 => Button::South,          // BUTTON_A
        97 => Button::East,           // BUTTON_B
        99 => Button::West,           // BUTTON_X
        100 => Button::North,         // BUTTON_Y
        102 => Button::LeftShoulder,  // L1
        103 => Button::RightShoulder, // R1
        106 => Button::LeftStick,     // THUMBL
        107 => Button::RightStick,    // THUMBR
        108 => Button::Start,         // START
        109 | 4 => Button::Back,      // SELECT (and BACK from a pad)
        110 => Button::Guide,         // MODE
        19 => Button::DPadUp,
        20 => Button::DPadDown,
        21 => Button::DPadLeft,
        22 => Button::DPadRight,
        98 | 101 | 104 | 105 => Button::Other(code as u8), // C, Z, L2, R2
        188..=203 => Button::Other(code as u8),            // BUTTON_1..16
        _ => return None,
    })
}

/// Keys the system keeps (volume, power, home, ...).
fn system_key(code: u32) -> bool {
    matches!(code, 3 | 24 | 25 | 26 | 164 | 187 | 220 | 221 | 223 | 224)
}

#[derive(Default)]
struct PadState {
    axes: [f32; 6],
    hat: (i8, i8),
}

// ---------------------------------------------------------------- audio

struct AudioOut {
    stream: ndk::audio::AudioStream,
}

/// The mixer shared by successive AAudio streams (re-opened after a
/// disconnect).
type SharedMixer = Arc<Mutex<DeviceMixer>>;

fn open_stream(
    mixer: SharedMixer,
    rate: i32,
    lost: Arc<AtomicBool>,
) -> Result<ndk::audio::AudioStream, String> {
    use ndk::audio::*;
    let stream = AudioStreamBuilder::new()
        .map_err(|e| format!("AAudio builder: {e:?}"))?
        .direction(AudioDirection::Output)
        .format(AudioFormat::PCM_Float)
        .channel_count(2)
        .sample_rate(rate)
        .sharing_mode(AudioSharingMode::Shared)
        .performance_mode(AudioPerformanceMode::LowLatency)
        .data_callback(Box::new(move |_s, data, frames| {
            let out =
                unsafe { std::slice::from_raw_parts_mut(data as *mut f32, frames as usize * 2) };
            match mixer.try_lock() {
                Ok(mut m) => m.render(out),
                Err(_) => out.fill(0.0),
            }
            AudioCallbackResult::Continue
        }))
        .error_callback(Box::new(move |_s, e| {
            log::warn!("AAudio error: {e:?}");
            lost.store(true, Ordering::Relaxed);
        }))
        .open_stream()
        .map_err(|e| format!("AAudio open: {e:?}"))?;
    Ok(stream)
}

// ---------------------------------------------------------------- platform

/// The Android platform (see the module docs).
pub struct AndroidPlatform {
    app: AndroidApp,
    egl: Egl,
    start: Instant,
    queued: Vec<Event>,
    size: (u32, u32),
    has_surface: bool,
    suspended_reported: bool,
    paused: bool,
    quit: bool,
    lose_context_on_suspend: bool,
    restored_gl: Option<glow::Context>,
    audio: Option<Audio>,
    mixer: Option<SharedMixer>,
    audio_out: Option<AudioOut>,
    audio_rate: i32,
    audio_lost: Arc<AtomicBool>,
    touch: TouchControls,
    pads: BTreeMap<i32, PadState>,
    devices: BTreeMap<i32, DeviceInfo>,
    next_scan: Duration,
}

impl AndroidPlatform {
    /// Waits for the activity's window, creates the EGL context on it and
    /// returns the `glow` handle (the context stays current on this
    /// thread). Errors if the activity is destroyed first.
    pub fn new(app: AndroidApp, cfg: &WindowConfig) -> Result<(Self, glow::Context), String> {
        app.set_window_flags(
            WindowManagerFlags::KEEP_SCREEN_ON | WindowManagerFlags::FULLSCREEN,
            WindowManagerFlags::empty(),
        );
        let egl = Egl::new(cfg.vsync)?;
        let mut p = Self {
            app,
            egl,
            start: Instant::now(),
            queued: Vec::new(),
            size: (1, 1),
            has_surface: false,
            suspended_reported: false,
            paused: false,
            quit: false,
            lose_context_on_suspend: system_property("debug.babaport.losectx").as_deref()
                == Some("1"),
            restored_gl: None,
            audio: None,
            mixer: None,
            audio_out: None,
            audio_rate: SAMPLE_RATE as i32,
            audio_lost: Arc::new(AtomicBool::new(false)),
            touch: TouchControls::new(1, 1),
            pads: BTreeMap::new(),
            devices: BTreeMap::new(),
            next_scan: Duration::ZERO,
        };
        // Lifecycle events until the first window.
        while !p.has_surface {
            if p.quit {
                return Err("activity destroyed before its window appeared".into());
            }
            p.pump(None);
        }
        p.queued.retain(|e| !matches!(e, Event::Resume { .. }));
        let gl = load_gl();
        if cfg.audio {
            p.open_audio();
        }
        log::info!("{}", p.describe());
        Ok((p, gl))
    }

    pub fn app(&self) -> &AndroidApp {
        &self.app
    }

    fn open_audio(&mut self) {
        let mixer = match &self.mixer {
            Some(m) => m.clone(),
            None => {
                let (audio, dev) = Audio::device(self.audio_rate as u32, String::new());
                let m = Arc::new(Mutex::new(dev));
                self.audio = Some(audio);
                self.mixer = Some(m.clone());
                m
            }
        };
        self.audio_lost.store(false, Ordering::Relaxed);
        match open_stream(mixer, self.audio_rate, self.audio_lost.clone()) {
            Ok(stream) => {
                let rate = stream.sample_rate();
                if rate != self.audio_rate {
                    log::warn!(
                        "AAudio gave {rate} Hz instead of {} Hz; audio plays at the wrong speed",
                        self.audio_rate
                    );
                }
                let desc = format!(
                    "AAudio {} Hz stereo f32, burst {} frames, {:?}",
                    rate,
                    stream.frames_per_burst(),
                    stream.performance_mode()
                );
                log::info!("{desc}");
                if let Some(a) = &mut self.audio {
                    a.set_description(desc);
                }
                if !self.paused
                    && let Err(e) = stream.request_start()
                {
                    log::error!("AAudio start: {e:?}");
                }
                self.audio_out = Some(AudioOut { stream });
            }
            Err(e) => log::error!("{e}"),
        }
    }

    fn set_audio_running(&mut self, on: bool) {
        if let Some(a) = &self.audio_out {
            let r = if on {
                a.stream.request_start()
            } else {
                a.stream.request_pause()
            };
            if let Err(e) = r {
                log::warn!("AAudio {}: {e:?}", if on { "start" } else { "pause" });
            }
        }
    }

    fn on_window(&mut self) {
        let Some(win) = self.app.native_window() else {
            return;
        };
        let mut gl_lost = false;
        if self.egl.context.is_null() {
            gl_lost = true;
        }
        loop {
            if self.egl.context.is_null()
                && let Err(e) = self.egl.create_context()
            {
                log::error!("{e}");
                return;
            }
            match self.egl.attach(&win) {
                Ok(()) => break,
                Err(true) => {
                    // Context lost: start over with a new one.
                    self.egl.detach();
                    self.egl.destroy_context();
                    gl_lost = true;
                }
                Err(false) => return,
            }
        }
        self.has_surface = true;
        self.suspended_reported = false;
        let size = self.egl.size();
        log::info!(
            "window {}x{}{}",
            size.0,
            size.1,
            if gl_lost { ", new GL context" } else { "" }
        );
        if gl_lost {
            self.restored_gl = Some(load_gl());
        }
        self.queued.push(Event::Resume { gl_lost });
        self.set_size(size);
    }

    fn set_size(&mut self, size: (u32, u32)) {
        if size != self.size && size.0 > 0 && size.1 > 0 {
            self.size = size;
            self.queued.push(Event::Resized {
                width: size.0,
                height: size.1,
            });
        }
    }

    /// Processes lifecycle events (blocking up to `timeout`).
    fn pump(&mut self, timeout: Option<Duration>) {
        let app = self.app.clone();
        app.poll_events(timeout, |ev| {
            let PollEvent::Main(ev) = ev else {
                return;
            };
            match ev {
                MainEvent::InitWindow { .. } => self.on_window(),
                MainEvent::TerminateWindow { .. } => {
                    self.egl.detach();
                    self.has_surface = false;
                    if self.lose_context_on_suspend {
                        log::info!("debug.babaport.losectx: dropping the GL context");
                        self.egl.destroy_context();
                    }
                    self.queued.push(Event::Suspend);
                }
                MainEvent::WindowResized { .. } | MainEvent::ContentRectChanged { .. } => {
                    if self.has_surface {
                        let s = self.egl.size();
                        self.set_size(s);
                    }
                }
                MainEvent::GainedFocus => {
                    let (vm, act) = (
                        self.app.vm_as_ptr() as usize,
                        self.app.activity_as_ptr() as usize,
                    );
                    self.app.run_on_java_main_thread(Box::new(move || {
                        hide_system_bars(vm as *mut c_void, act as *mut c_void)
                    }));
                    self.queued.push(Event::Focus(true));
                }
                MainEvent::LostFocus => self.queued.push(Event::Focus(false)),
                MainEvent::Pause => {
                    self.paused = true;
                    self.set_audio_running(false);
                }
                MainEvent::Resume { .. } => {
                    self.paused = false;
                    self.set_audio_running(true);
                }
                MainEvent::Destroy => {
                    self.quit = true;
                    self.queued.push(Event::Quit);
                }
                _ => {}
            }
        });
    }

    fn controller(&mut self, dev: i32, out: &mut Vec<Event>) -> DeviceId {
        let id = DeviceId::Controller(dev as u32);
        if let std::collections::btree_map::Entry::Vacant(v) = self.devices.entry(dev) {
            let d = describe_device(self.app.vm_as_ptr(), dev);
            let info = DeviceInfo {
                id,
                name: d
                    .as_ref()
                    .map(|d| d.name.clone())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| format!("Android input device {dev}")),
                guid: d.map(|d| d.descriptor).unwrap_or_default(),
                mapped: true,
            };
            v.insert(info.clone());
            out.push(Event::ControllerAdded(info));
        }
        id
    }

    /// Hot-plug: compares the system's gamepads with ours.
    fn scan_devices(&mut self, out: &mut Vec<Event>) {
        let Some(list) = scan_input_devices(self.app.vm_as_ptr()) else {
            return;
        };
        for d in &list {
            if is_pad_source(d.sources) && !self.devices.contains_key(&d.id) {
                let info = DeviceInfo {
                    id: DeviceId::Controller(d.id as u32),
                    name: d.name.clone(),
                    guid: d.descriptor.clone(),
                    mapped: true,
                };
                self.devices.insert(d.id, info.clone());
                out.push(Event::ControllerAdded(info));
            }
        }
        let gone: Vec<i32> = self
            .devices
            .keys()
            .copied()
            .filter(|&id| id > 0 && !list.iter().any(|d| d.id == id))
            .collect();
        for id in gone {
            self.devices.remove(&id);
            self.pads.remove(&id);
            out.push(Event::ControllerRemoved(DeviceId::Controller(id as u32)));
        }
    }

    fn read_input(&mut self, out: &mut Vec<Event>) {
        let app = self.app.clone();
        let Ok(mut iter) = app.input_events_iter() else {
            return;
        };
        let (w, h) = (self.size.0.max(1) as f32, self.size.1.max(1) as f32);
        loop {
            let more = iter.next(|ev| match ev {
                InputEvent::MotionEvent(m) => {
                    let src: u32 = m.source().into();
                    let src = src as i32;
                    if src & SOURCE_JOYSTICK == SOURCE_JOYSTICK {
                        let dev = m.device_id();
                        let id = self.controller(dev, out);
                        let p = m.pointer_at_index(0);
                        let pad = self.pads.entry(dev).or_default();
                        let vals = [
                            p.axis_value(AAxis::X),
                            p.axis_value(AAxis::Y),
                            p.axis_value(AAxis::Z),
                            p.axis_value(AAxis::Rz),
                            p.axis_value(AAxis::Ltrigger)
                                .max(p.axis_value(AAxis::Brake)),
                            p.axis_value(AAxis::Rtrigger).max(p.axis_value(AAxis::Gas)),
                        ];
                        let names = [
                            Axis::LeftX,
                            Axis::LeftY,
                            Axis::RightX,
                            Axis::RightY,
                            Axis::TriggerLeft,
                            Axis::TriggerRight,
                        ];
                        for i in 0..6 {
                            if (vals[i] - pad.axes[i]).abs() > 1e-3 {
                                pad.axes[i] = vals[i];
                                out.push(Event::Axis {
                                    device: id,
                                    axis: names[i],
                                    value: vals[i],
                                });
                            }
                        }
                        let hat = (
                            p.axis_value(AAxis::HatX).round() as i8,
                            p.axis_value(AAxis::HatY).round() as i8,
                        );
                        let old = pad.hat;
                        pad.hat = hat;
                        for (o, n, neg, pos) in [
                            (old.0, hat.0, Button::DPadLeft, Button::DPadRight),
                            (old.1, hat.1, Button::DPadUp, Button::DPadDown),
                        ] {
                            if o != n {
                                if o != 0 {
                                    out.push(Event::Button {
                                        device: id,
                                        button: if o < 0 { neg } else { pos },
                                        down: false,
                                    });
                                }
                                if n != 0 {
                                    out.push(Event::Button {
                                        device: id,
                                        button: if n < 0 { neg } else { pos },
                                        down: true,
                                    });
                                }
                            }
                        }
                        return InputStatus::Handled;
                    }
                    const SOURCE_MOUSE: i32 = 0x2002;
                    if src & SOURCE_MOUSE == SOURCE_MOUSE {
                        let p = m.pointer_at_index(0);
                        let (x, y) = (p.x(), p.y());
                        match m.action() {
                            MotionAction::Down | MotionAction::Up => out.push(Event::MouseButton {
                                button: 1,
                                down: m.action() == MotionAction::Down,
                                x,
                                y,
                            }),
                            MotionAction::Scroll => out.push(Event::MouseWheel {
                                dx: p.axis_value(AAxis::Hscroll),
                                dy: p.axis_value(AAxis::Vscroll),
                            }),
                            _ => out.push(Event::MouseMove { x, y }),
                        }
                        return InputStatus::Handled;
                    }
                    // Touchscreen (and anything else pointer-like).
                    let touch = |p: android_activity::input::Pointer<'_>, phase| Event::Touch {
                        finger: p.pointer_id() as u64,
                        phase,
                        x: (p.x() / w).clamp(0.0, 1.0),
                        y: (p.y() / h).clamp(0.0, 1.0),
                    };
                    match m.action() {
                        MotionAction::Down | MotionAction::PointerDown => out.push(touch(
                            m.pointer_at_index(m.pointer_index()),
                            TouchPhase::Down,
                        )),
                        MotionAction::Up | MotionAction::PointerUp => {
                            out.push(touch(m.pointer_at_index(m.pointer_index()), TouchPhase::Up))
                        }
                        MotionAction::Move => {
                            for p in m.pointers() {
                                out.push(touch(p, TouchPhase::Move));
                            }
                        }
                        MotionAction::Cancel => {
                            for p in m.pointers() {
                                out.push(touch(p, TouchPhase::Up));
                            }
                        }
                        _ => {}
                    }
                    InputStatus::Handled
                }
                InputEvent::KeyEvent(k) => {
                    let code: u32 = k.key_code().into();
                    if system_key(code) {
                        return InputStatus::Unhandled;
                    }
                    let down = match k.action() {
                        KeyAction::Down => true,
                        KeyAction::Up => false,
                        _ => return InputStatus::Handled,
                    };
                    let repeat = k.repeat_count() > 0;
                    let src: u32 = k.source().into();
                    let from_pad = (src as i32) & SOURCE_GAMEPAD == SOURCE_GAMEPAD
                        || (96..=110).contains(&code)
                        || (188..=203).contains(&code);
                    if from_pad && let Some(button) = map_button(code) {
                        if !repeat {
                            let device = self.controller(k.device_id(), out);
                            out.push(Event::Button {
                                device,
                                button,
                                down,
                            });
                        }
                    } else if let Some(key) = map_key(code) {
                        out.push(Event::Key { key, down, repeat });
                    } else {
                        out.push(Event::Key {
                            key: Key::Other(code),
                            down,
                            repeat,
                        });
                    }
                    InputStatus::Handled
                }
                _ => InputStatus::Unhandled,
            });
            if !more {
                break;
            }
        }
    }
}

impl Platform for AndroidPlatform {
    fn describe(&self) -> String {
        format!(
            "Android (API {}), NativeActivity, EGL GLES 2, {}x{}, {}",
            AndroidApp::sdk_version(),
            self.size.0,
            self.size.1,
            self.audio
                .as_ref()
                .map(|a| a.description().to_string())
                .unwrap_or_else(|| "no audio".into())
        )
    }

    fn drawable_size(&self) -> (u32, u32) {
        self.size
    }

    fn poll_events(&mut self, out: &mut Vec<Event>) {
        // In the background: wait (no busy loop, no drawing) until the
        // window is back or the activity goes away.
        while !self.has_surface && self.suspended_reported && !self.quit {
            self.pump(None);
        }
        self.pump(Some(Duration::ZERO));
        if !self.has_surface {
            self.suspended_reported = true;
        }
        if self.audio_lost.swap(false, Ordering::Relaxed) {
            log::info!("AAudio stream disconnected; re-opening");
            self.audio_out = None;
            self.open_audio();
        }
        let mut raw = std::mem::take(&mut self.queued);
        self.read_input(&mut raw);
        let now = self.time();
        if now >= self.next_scan {
            self.next_scan = now + Duration::from_secs(2);
            self.scan_devices(&mut raw);
        }
        for ev in raw {
            self.touch.filter(ev, out);
        }
        self.touch.tick(now, out);
    }

    fn swap_buffers(&mut self) {
        if !self.has_surface {
            return;
        }
        unsafe {
            if eglSwapBuffers(self.egl.display, self.egl.surface) == 0 {
                let e = eglGetError();
                if e == EGL_CONTEXT_LOST {
                    log::warn!("GL context lost; creating a new one");
                    self.egl.destroy_context();
                    if self.egl.create_context().is_ok() && self.egl.make_current() {
                        self.restored_gl = Some(load_gl());
                        self.queued.push(Event::Resume { gl_lost: true });
                    }
                } else {
                    log::warn!("eglSwapBuffers: {e:#x}");
                }
            }
        }
        let s = self.egl.size();
        self.set_size(s);
    }

    fn set_fullscreen(&mut self, _on: bool) {
        // Always immersive fullscreen.
    }

    fn time(&self) -> Duration {
        self.start.elapsed()
    }

    fn audio(&mut self) -> Option<&mut Audio> {
        self.audio.as_mut()
    }

    fn devices(&self) -> Vec<DeviceInfo> {
        self.devices.values().cloned().collect()
    }

    fn take_restored_gl(&mut self) -> Option<glow::Context> {
        self.restored_gl.take()
    }

    fn touch_controls(&self) -> Option<&TouchControls> {
        Some(&self.touch)
    }

    fn touch_controls_mut(&mut self) -> Option<&mut TouchControls> {
        Some(&mut self.touch)
    }
}

impl Drop for AndroidPlatform {
    fn drop(&mut self) {
        if let Some(a) = self.audio_out.take() {
            let _ = a.stream.request_stop();
        }
        self.egl.detach();
        self.egl.destroy_context();
    }
}
