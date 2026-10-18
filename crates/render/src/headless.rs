//! Offscreen GL context without any window system: EGL loaded at run time
//! (`libEGL.so.1`), preferring Mesa's surfaceless platform (works without
//! X, Wayland or a GPU through llvmpipe), then the device or default
//! display with a pbuffer. Rendering goes to an FBO (see
//! `Renderer::use_offscreen_screen`).

use khronos_egl as egl;

type Egl = egl::DynamicInstance<egl::EGL1_4>;

const PLATFORM_SURFACELESS_MESA: egl::Enum = 0x31DD;
const PLATFORM_DEVICE_EXT: egl::Enum = 0x313F;

/// Which API to request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeadlessApi {
    /// OpenGL ES 2.0 (the feature level the renderer targets; default).
    Gles2,
    /// Desktop OpenGL 3.3 core.
    GlCore33,
}

impl HeadlessApi {
    /// `BABA_HEADLESS_GL=gl33` selects desktop GL; anything else ES 2.
    pub fn from_env() -> Self {
        match std::env::var("BABA_HEADLESS_GL").as_deref() {
            Ok("gl33") | Ok("gl") => HeadlessApi::GlCore33,
            _ => HeadlessApi::Gles2,
        }
    }
}

/// A current EGL context; dropping it releases the context.
pub struct HeadlessContext {
    egl: Egl,
    display: egl::Display,
    context: egl::Context,
    surface: Option<egl::Surface>,
    /// How the display was obtained (for logs).
    pub platform: &'static str,
}

impl HeadlessContext {
    /// Creates a context and makes it current on this thread.
    pub fn new(api: HeadlessApi) -> Result<(Self, glow::Context), String> {
        // SAFETY: loading the system EGL library.
        let egl = unsafe { Egl::load_required() }.map_err(|e| format!("loading libEGL: {e}"))?;
        let client_ext = egl
            .query_string(None, egl::EXTENSIONS)
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut tried = Vec::new();
        let mut candidates: Vec<(&'static str, egl::Enum)> = Vec::new();
        if client_ext.contains("EGL_MESA_platform_surfaceless") {
            candidates.push(("surfaceless", PLATFORM_SURFACELESS_MESA));
        }
        if client_ext.contains("EGL_EXT_platform_device") {
            candidates.push(("device", PLATFORM_DEVICE_EXT));
        }
        let mut found = None;
        for (name, platform) in candidates {
            match platform_display(&egl, platform) {
                Some(d) if egl.initialize(d).is_ok() => {
                    found = Some((name, d));
                    break;
                }
                _ => tried.push(name),
            }
        }
        if found.is_none() {
            // SAFETY: EGL_DEFAULT_DISPLAY.
            if let Some(d) = unsafe { egl.get_display(egl::DEFAULT_DISPLAY) }
                && egl.initialize(d).is_ok()
            {
                found = Some(("default", d));
            }
            tried.push("default");
        }
        let (platform, display) =
            found.ok_or_else(|| format!("no usable EGL display (tried {tried:?})"))?;

        let (api_enum, renderable) = match api {
            HeadlessApi::Gles2 => (egl::OPENGL_ES_API, egl::OPENGL_ES2_BIT),
            HeadlessApi::GlCore33 => (egl::OPENGL_API, egl::OPENGL_BIT),
        };
        egl.bind_api(api_enum)
            .map_err(|e| format!("eglBindAPI: {e}"))?;
        let display_ext = egl
            .query_string(Some(display), egl::EXTENSIONS)
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let surfaceless = display_ext.contains("EGL_KHR_surfaceless_context");
        let mut config = None;
        for surface_type in [egl::PBUFFER_BIT, 0] {
            let attrs = [
                egl::RENDERABLE_TYPE,
                renderable,
                egl::SURFACE_TYPE,
                surface_type,
                egl::RED_SIZE,
                8,
                egl::GREEN_SIZE,
                8,
                egl::BLUE_SIZE,
                8,
                egl::ALPHA_SIZE,
                8,
                egl::NONE,
            ];
            if let Ok(Some(c)) = egl.choose_first_config(display, &attrs) {
                config = Some((c, surface_type));
                break;
            }
        }
        let (config, surface_type) =
            config.ok_or_else(|| format!("no EGL config for {api:?} on {platform}"))?;
        let ctx_attrs: Vec<egl::Int> = match api {
            HeadlessApi::Gles2 => vec![egl::CONTEXT_CLIENT_VERSION, 2, egl::NONE],
            HeadlessApi::GlCore33 => vec![
                egl::CONTEXT_MAJOR_VERSION,
                3,
                egl::CONTEXT_MINOR_VERSION,
                3,
                egl::CONTEXT_OPENGL_PROFILE_MASK,
                egl::CONTEXT_OPENGL_CORE_PROFILE_BIT,
                egl::NONE,
            ],
        };
        let context = egl
            .create_context(display, config, None, &ctx_attrs)
            .map_err(|e| format!("eglCreateContext {api:?}: {e}"))?;
        let surface = if surfaceless {
            None
        } else if surface_type != 0 {
            Some(
                egl.create_pbuffer_surface(
                    display,
                    config,
                    &[egl::WIDTH, 16, egl::HEIGHT, 16, egl::NONE],
                )
                .map_err(|e| format!("eglCreatePbufferSurface: {e}"))?,
            )
        } else {
            return Err("EGL display has neither surfaceless contexts nor pbuffers".into());
        };
        egl.make_current(display, surface, surface, Some(context))
            .map_err(|e| format!("eglMakeCurrent: {e}"))?;
        // SAFETY: the context is current; function pointers come from EGL.
        let gl = unsafe {
            glow::Context::from_loader_function(|name| {
                egl.get_proc_address(name)
                    .map_or(std::ptr::null(), |f| f as *const std::ffi::c_void)
            })
        };
        log::info!("headless EGL context: {platform} display, {api:?}, surfaceless={surfaceless}");
        Ok((
            Self {
                egl,
                display,
                context,
                surface,
                platform,
            },
            gl,
        ))
    }
}

fn platform_display(egl: &Egl, platform: egl::Enum) -> Option<egl::Display> {
    if let Some(egl15) = egl.upcast::<egl::EGL1_5>() {
        // SAFETY: no native display for these platforms.
        if let Ok(d) = unsafe {
            egl15.get_platform_display(platform, egl::DEFAULT_DISPLAY, &[egl::ATTRIB_NONE])
        } {
            return Some(d);
        }
    }
    // EGL 1.4 + EGL_EXT_platform_base.
    let f = egl.get_proc_address("eglGetPlatformDisplayEXT")?;
    type GetPlatformDisplayExt = unsafe extern "system" fn(
        egl::Enum,
        *mut std::ffi::c_void,
        *const egl::Int,
    ) -> *mut std::ffi::c_void;
    // SAFETY: the documented signature of eglGetPlatformDisplayEXT.
    let f: GetPlatformDisplayExt = unsafe { std::mem::transmute(f) };
    let attrs = [egl::NONE];
    let d = unsafe { f(platform, std::ptr::null_mut(), attrs.as_ptr()) };
    if d.is_null() {
        None
    } else {
        // SAFETY: a display handle returned by EGL.
        Some(unsafe { egl::Display::from_ptr(d) })
    }
}

impl Drop for HeadlessContext {
    fn drop(&mut self) {
        let _ = self.egl.make_current(self.display, None, None, None);
        if let Some(s) = self.surface.take() {
            let _ = self.egl.destroy_surface(self.display, s);
        }
        let _ = self.egl.destroy_context(self.display, self.context);
        let _ = self.egl.terminate(self.display);
    }
}
