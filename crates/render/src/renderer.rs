//! The GL renderer. OpenGL ES 2.0 feature level only: one texture unit,
//! client-side attribute setup per draw, u16 indices, NPOT textures with
//! clamp-to-edge and no mipmaps, RGBA8 texture render targets. Runs on
//! GLES 2/3 and desktop GL 2.1 / 3.x core (a VAO is bound once there).
//!
//! Each level view is drawn into its own offscreen texture at an integer
//! multiple of the source resolution (nearest sampling, so sprites stay
//! crisp), then composited into the viewport by the effect pass (scale,
//! rotation, zoom, tint, optional custom fragment shader). At an integer
//! final scale the composite is a 1:1 copy, i.e. pixel-exact.

use glow::HasContext;

use crate::atlas::Atlas;
use crate::image::RgbaImage;
use crate::scene::{Quad, TileScene};
use crate::viewport::{Fit, Rect, ScaleMode, fit};

/// Quads per draw call (u16 indices: 4 vertices each).
const BATCH_QUADS: usize = 4096;

/// Which GL flavour the context is; selects the GLSL dialect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlFlavor {
    /// OpenGL ES 2.0 or later (GLSL ES 1.00 shaders).
    Es,
    /// Desktop GL 3.2+ core.
    Core,
    /// Desktop GL 2.x / 3.0-3.1 compatibility.
    Legacy,
}

/// Facts about the context, for logs and `--list-devices`.
#[derive(Clone, Debug)]
pub struct GlInfo {
    pub flavor: GlFlavor,
    pub major: u32,
    pub minor: u32,
    pub version: String,
    pub renderer: String,
    pub vendor: String,
    pub max_texture_size: u32,
    /// Fragment shaders have highp floats (always true on desktop).
    pub frag_highp: bool,
}

#[derive(Debug)]
pub struct RenderError(pub String);

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RenderError {}

impl From<String> for RenderError {
    fn from(s: String) -> Self {
        RenderError(s)
    }
}

type Result<T> = std::result::Result<T, RenderError>;

#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    pos: [f32; 2],
    uv: [f32; 2],
    color: [u8; 4],
}

const VERTEX_SIZE: i32 = std::mem::size_of::<Vertex>() as i32;

const VS: &str = r#"
ATTR vec2 a_pos;
ATTR vec2 a_uv;
ATTR vec4 a_color;
uniform vec4 u_proj;
VOUT vec2 v_uv;
VOUT vec4 v_color;
void main() {
    gl_Position = vec4(a_pos * u_proj.xy + u_proj.zw, 0.0, 1.0);
    v_uv = a_uv;
    v_color = a_color;
}
"#;

const FS_SPRITE: &str = r#"
uniform sampler2D u_tex;
VIN vec2 v_uv;
VIN vec4 v_color;
void main() {
    FRAG = TEX(u_tex, v_uv) * v_color;
}
"#;

/// Uniforms available to custom effect shaders (see
/// [`Renderer::register_effect`]): `u_tex` (the level image), `u_time`
/// (seconds), `u_size` (level texture size in pixels), varyings `v_uv` and
/// `v_color` (the tint). The body must write `FRAG`.
pub const EFFECT_PRELUDE: &str = r#"
uniform sampler2D u_tex;
uniform float u_time;
uniform vec2 u_size;
VIN vec2 v_uv;
VIN vec4 v_color;
"#;

fn header(flavor: GlFlavor, major: u32, minor: u32, fragment: bool) -> String {
    match flavor {
        GlFlavor::Es => {
            let mut s = String::from("#version 100\n");
            if fragment {
                s.push_str(
                    "#ifdef GL_FRAGMENT_PRECISION_HIGH\nprecision highp float;\n#else\nprecision mediump float;\n#endif\n",
                );
            }
            s.push_str("#define ATTR attribute\n#define VOUT varying\n#define VIN varying\n#define TEX texture2D\n#define FRAG gl_FragColor\n");
            s
        }
        GlFlavor::Core => {
            let v = if (major, minor) >= (3, 3) {
                "330 core"
            } else {
                "150"
            };
            let mut s = format!(
                "#version {v}\n#define ATTR in\n#define VOUT out\n#define VIN in\n#define TEX texture\n"
            );
            if fragment {
                s.push_str("out vec4 o_frag;\n#define FRAG o_frag\n");
            }
            s
        }
        GlFlavor::Legacy => {
            let v = if major >= 3 { "130" } else { "120" };
            format!(
                "#version {v}\n#define ATTR attribute\n#define VOUT varying\n#define VIN varying\n#define TEX texture2D\n#define FRAG gl_FragColor\n"
            )
        }
    }
}

struct Program {
    program: glow::Program,
    u_proj: Option<glow::UniformLocation>,
    u_tex: Option<glow::UniformLocation>,
    u_time: Option<glow::UniformLocation>,
    u_size: Option<glow::UniformLocation>,
}

/// An offscreen colour target (RGBA8 texture + framebuffer).
pub struct Offscreen {
    pub texture: glow::Texture,
    pub framebuffer: glow::Framebuffer,
    pub width: u32,
    pub height: u32,
}

/// Handle of a registered custom effect shader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectId(usize);

/// Full-view effect applied when a level view is composited (the hook for
/// level rotation, zoom, fades and later shader effects).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostEffect {
    /// Clockwise rotation around the level centre, radians.
    pub rotation: f32,
    /// Extra zoom around the level centre (1 = none).
    pub zoom: f32,
    /// Offset in viewport pixels.
    pub offset: [f32; 2],
    /// Multiplied with the level image.
    pub tint: [u8; 4],
    /// Custom fragment shader, if any.
    pub shader: Option<EffectId>,
    /// Seconds, passed as `u_time` to custom shaders.
    pub time: f32,
}

impl Default for PostEffect {
    fn default() -> Self {
        Self {
            rotation: 0.0,
            zoom: 1.0,
            offset: [0.0, 0.0],
            tint: [255; 4],
            shader: None,
            time: 0.0,
        }
    }
}

/// One level view to draw.
pub struct View<'a> {
    pub scene: &'a TileScene,
    /// Wobble counter (frame = counter + per-sprite offset, mod 3).
    pub wobble: u32,
    /// Viewport on the target, origin top-left.
    pub rect: Rect,
    pub scale: ScaleMode,
    pub effect: PostEffect,
}

/// The renderer; owns the GL context handle.
pub struct Renderer {
    gl: glow::Context,
    info: GlInfo,
    sprite: Program,
    effects: Vec<Program>,
    vbo: glow::Buffer,
    ibo: glow::Buffer,
    _vao: Option<glow::VertexArray>,
    pages: Vec<glow::Texture>,
    /// Level render targets, one per view slot (reused across frames).
    level_targets: Vec<Option<Offscreen>>,
    /// Optional offscreen "screen" (headless); `None` = default framebuffer.
    screen: Option<Offscreen>,
    target_size: (u32, u32),
    verts: Vec<Vertex>,
    quads: Vec<Quad>,
    /// Built-in UI textures (icons, font), created on first `draw_ui`.
    ui_textures: Option<[(glow::Texture, u32, u32); 2]>,
}

impl Renderer {
    /// Wraps a current GL context. Compiles the shaders, creates buffers.
    pub fn new(gl: glow::Context) -> Result<Self> {
        let info = unsafe { query_info(&gl) };
        log::info!(
            "GL: {} ({}, {}), {:?} {}.{}, max texture {}, frag highp {}",
            info.version,
            info.renderer,
            info.vendor,
            info.flavor,
            info.major,
            info.minor,
            info.max_texture_size,
            info.frag_highp
        );
        unsafe {
            let vao = if info.flavor == GlFlavor::Core {
                let vao = gl.create_vertex_array()?;
                gl.bind_vertex_array(Some(vao));
                Some(vao)
            } else {
                None
            };
            let sprite = compile(&gl, &info, FS_SPRITE, "")?;
            let vbo = gl.create_buffer()?;
            let ibo = gl.create_buffer()?;
            let mut idx: Vec<u16> = Vec::with_capacity(BATCH_QUADS * 6);
            for q in 0..BATCH_QUADS as u16 {
                let b = q * 4;
                idx.extend_from_slice(&[b, b + 1, b + 2, b + 2, b + 3, b]);
            }
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ibo));
            gl.buffer_data_u8_slice(
                glow::ELEMENT_ARRAY_BUFFER,
                bytes_of(&idx),
                glow::STATIC_DRAW,
            );
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_size(
                glow::ARRAY_BUFFER,
                BATCH_QUADS as i32 * 4 * VERTEX_SIZE,
                glow::STREAM_DRAW,
            );
            Ok(Self {
                gl,
                info,
                sprite,
                effects: Vec::new(),
                vbo,
                ibo,
                _vao: vao,
                pages: Vec::new(),
                level_targets: Vec::new(),
                screen: None,
                target_size: (1, 1),
                verts: Vec::with_capacity(BATCH_QUADS * 4),
                quads: Vec::new(),
                ui_textures: None,
            })
        }
    }

    pub fn gl(&self) -> &glow::Context {
        &self.gl
    }

    pub fn info(&self) -> &GlInfo {
        &self.info
    }

    /// Atlas page size suited to this context: at most 2048, and 1024
    /// where fragment shaders lack highp (texture coordinates in mediump
    /// cannot address single texels of larger pages).
    pub fn atlas_page_size(&self) -> u32 {
        let cap = if self.info.frag_highp { 2048 } else { 1024 };
        self.info.max_texture_size.min(cap)
    }

    /// Compiles a custom effect shader. `body` is GLSL (ES 1.00 style with
    /// the `TEX`/`FRAG`/`VIN` macros) appended after [`EFFECT_PRELUDE`]
    /// and must define `main`.
    pub fn register_effect(&mut self, body: &str) -> Result<EffectId> {
        let p = unsafe { compile(&self.gl, &self.info, body, EFFECT_PRELUDE)? };
        self.effects.push(p);
        Ok(EffectId(self.effects.len() - 1))
    }

    /// Uploads atlas pages that changed since the last call (creates
    /// textures for new pages).
    pub fn sync_atlas(&mut self, atlas: &mut Atlas) {
        let gl = &self.gl;
        unsafe {
            while self.pages.len() < atlas.page_count() {
                let size = atlas.page_size() as i32;
                let tex = new_texture(gl, size, size, None).expect("atlas texture");
                self.pages.push(tex);
            }
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
            for (page, y0, y1) in atlas.take_dirty() {
                let img = atlas.page(page);
                let row = img.width as usize * 4;
                gl.bind_texture(glow::TEXTURE_2D, Some(self.pages[page]));
                gl.tex_sub_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    0,
                    y0 as i32,
                    img.width as i32,
                    (y1 - y0) as i32,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(
                        &img.data[y0 as usize * row..y1 as usize * row],
                    )),
                );
            }
        }
    }

    /// Forgets every GL object (after the platform reports a lost context,
    /// e.g. Android pause); the caller re-creates the renderer and calls
    /// `Atlas::mark_all_dirty`.
    pub fn into_gl(self) -> glow::Context {
        self.gl
    }

    /// Renders into an offscreen "screen" of `w` x `h` instead of the
    /// default framebuffer (headless screenshots).
    pub fn use_offscreen_screen(&mut self, w: u32, h: u32) -> Result<()> {
        if let Some(s) = &self.screen
            && (s.width, s.height) == (w, h)
        {
            return Ok(());
        }
        if let Some(old) = self.screen.take() {
            unsafe { delete_offscreen(&self.gl, old) };
        }
        self.screen = Some(unsafe { new_offscreen(&self.gl, w, h)? });
        Ok(())
    }

    fn bind_screen(&self) {
        unsafe {
            self.gl.bind_framebuffer(
                glow::FRAMEBUFFER,
                self.screen.as_ref().map(|s| s.framebuffer),
            );
        }
    }

    /// Starts a frame on the screen target of `w` x `h` pixels, cleared to
    /// `clear`.
    pub fn begin_frame(&mut self, w: u32, h: u32, clear: [u8; 4]) {
        self.target_size = (w, h);
        let gl = &self.gl;
        unsafe {
            self.bind_screen();
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::CULL_FACE);
            gl.disable(glow::SCISSOR_TEST);
            gl.enable(glow::BLEND);
            gl.blend_func_separate(
                glow::SRC_ALPHA,
                glow::ONE_MINUS_SRC_ALPHA,
                glow::ONE,
                glow::ONE_MINUS_SRC_ALPHA,
            );
            gl.viewport(0, 0, w as i32, h as i32);
            let c = clear.map(|v| v as f32 / 255.0);
            gl.clear_color(c[0], c[1], c[2], c[3]);
            gl.clear(glow::COLOR_BUFFER_BIT);
        }
    }

    /// Draws a level view; `slot` selects the reusable offscreen target
    /// (one per simultaneously visible view). Returns the placement.
    pub fn draw_view(&mut self, slot: usize, atlas: &Atlas, view: &View) -> Fit {
        let (cw, ch) = view.scene.pixel_size();
        let placed = fit(view.rect, cw, ch, view.scale);
        // Render at the integer scale at or above the final one (crisp
        // pixels, then at most a mild filtered down-scale), within the
        // texture size limit.
        let max = self.info.max_texture_size;
        let mut k = (placed.scale - 1e-4).ceil().max(1.0) as u32;
        while k > 1 && (cw * k > max || ch * k > max) {
            k -= 1;
        }
        let (tw, th) = (cw * k, ch * k);
        if self.level_targets.len() <= slot {
            self.level_targets.resize_with(slot + 1, || None);
        }
        let reuse = matches!(&self.level_targets[slot], Some(t) if (t.width, t.height) == (tw, th));
        if !reuse {
            if let Some(old) = self.level_targets[slot].take() {
                unsafe { delete_offscreen(&self.gl, old) };
            }
            self.level_targets[slot] =
                Some(unsafe { new_offscreen(&self.gl, tw, th).expect("level target") });
        }
        let target = self.level_targets[slot].as_ref().unwrap();
        let (fb, tex) = (target.framebuffer, target.texture);

        // 1. The level at k x into its texture.
        let mut quads = std::mem::take(&mut self.quads);
        quads.clear();
        view.scene.quads(atlas, view.wobble, k, &mut quads);
        unsafe {
            let gl = &self.gl;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fb));
            gl.viewport(0, 0, tw as i32, th as i32);
            let c = view.scene.outside_bg.map(|v| v as f32 / 255.0);
            gl.clear_color(c[0], c[1], c[2], 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            let (ix, iy, iw, ih) = view.scene.interior(k);
            gl.enable(glow::SCISSOR_TEST);
            // Image row y is framebuffer row y here (see the projection).
            gl.scissor(ix as i32, iy as i32, iw as i32, ih as i32);
            let c = view.scene.level_bg.map(|v| v as f32 / 255.0);
            gl.clear_color(c[0], c[1], c[2], 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.disable(glow::SCISSOR_TEST);
            gl.use_program(Some(self.sprite.program));
            gl.uniform_1_i32(self.sprite.u_tex.as_ref(), 0);
            // Pixel y = 0 maps to NDC -1, i.e. framebuffer/texture row 0,
            // so the texture holds the image top-down in memory order.
            gl.uniform_4_f32(
                self.sprite.u_proj.as_ref(),
                2.0 / tw as f32,
                2.0 / th as f32,
                -1.0,
                -1.0,
            );
        }
        self.draw_quads(atlas, &quads);
        self.quads = quads;

        // 2. Composite into the viewport (effect pass).
        let (sw, sh) = self.target_size;
        let fx = &view.effect;
        let r = view.rect;
        unsafe {
            let gl = &self.gl;
            self.bind_screen();
            gl.viewport(0, 0, sw as i32, sh as i32);
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(r.x, sh as i32 - r.y - r.h, r.w, r.h);
            let c = view.scene.outside_bg.map(|v| v as f32 / 255.0);
            gl.clear_color(c[0], c[1], c[2], c[3]);
            gl.clear(glow::COLOR_BUFFER_BIT);
            // Quarter turns keep pixels whole: sample nearest then too.
            let quarter = fx.rotation / std::f32::consts::FRAC_PI_2;
            let square = (quarter - quarter.round()).abs() < 1e-4;
            let filter = if placed.integer && placed.scale as u32 == k && square && fx.zoom == 1.0 {
                glow::NEAREST
            } else {
                glow::LINEAR
            };
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, filter as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, filter as i32);
            let p = match fx.shader {
                Some(EffectId(i)) => &self.effects[i],
                None => &self.sprite,
            };
            gl.use_program(Some(p.program));
            gl.uniform_1_i32(p.u_tex.as_ref(), 0);
            gl.uniform_1_f32(p.u_time.as_ref(), fx.time);
            gl.uniform_2_f32(p.u_size.as_ref(), tw as f32, th as f32);
            gl.uniform_4_f32(
                p.u_proj.as_ref(),
                2.0 / sw as f32,
                -2.0 / sh as f32,
                -1.0,
                1.0,
            );
        }
        let pr = placed.rect;
        let cx = pr.x as f32 + pr.w as f32 * 0.5 + fx.offset[0];
        let cy = pr.y as f32 + pr.h as f32 * 0.5 + fx.offset[1];
        let (hw, hh) = (pr.w as f32 * 0.5 * fx.zoom, pr.h as f32 * 0.5 * fx.zoom);
        let (s, c) = fx.rotation.sin_cos();
        let corner = |dx: f32, dy: f32| [cx + dx * c - dy * s, cy + dx * s + dy * c];
        self.verts.clear();
        // Pass 1 put image row 0 at texture row 0 (v = 0).
        for (d, uv) in [
            ((-hw, -hh), [0.0, 0.0]),
            ((hw, -hh), [1.0, 0.0]),
            ((hw, hh), [1.0, 1.0]),
            ((-hw, hh), [0.0, 1.0]),
        ] {
            self.verts.push(Vertex {
                pos: corner(d.0, d.1),
                uv,
                color: fx.tint,
            });
        }
        self.flush();
        unsafe {
            self.gl.disable(glow::SCISSOR_TEST);
        }
        placed
    }

    /// Draws quads directly on the screen target, in screen pixels (y
    /// down), over whatever was drawn before (menus, text, HUD,
    /// transitions). Quads may come from any atlas page.
    pub fn draw_overlay(&mut self, atlas: &Atlas, quads: &[Quad]) {
        if quads.is_empty() {
            return;
        }
        self.overlay_setup();
        self.draw_quads(atlas, quads);
    }

    /// Binds the screen target and the sprite program with a screen-pixel
    /// projection (y down).
    fn overlay_setup(&mut self) {
        let (sw, sh) = self.target_size;
        unsafe {
            let gl = &self.gl;
            self.bind_screen();
            gl.viewport(0, 0, sw as i32, sh as i32);
            gl.disable(glow::SCISSOR_TEST);
            gl.use_program(Some(self.sprite.program));
            gl.uniform_1_i32(self.sprite.u_tex.as_ref(), 0);
            gl.uniform_4_f32(
                self.sprite.u_proj.as_ref(),
                2.0 / sw as f32,
                -2.0 / sh as f32,
                -1.0,
                1.0,
            );
        }
    }

    /// Like [`Renderer::draw_overlay`] with each quad turned by its angle
    /// (radians, clockwise on screen) about its centre: rotated sprites of
    /// cutscenes (scattered objects, flower petals). Sampled linearly.
    pub fn draw_overlay_rotated(&mut self, atlas: &Atlas, quads: &[(Quad, f32)]) {
        if quads.is_empty() {
            return;
        }
        self.overlay_setup();
        let plain: Vec<Quad> = quads.iter().map(|(q, _)| *q).collect();
        let angles: Vec<f32> = quads.iter().map(|(_, a)| *a).collect();
        self.draw_quads_turned(atlas, &plain, Some(&angles));
    }

    /// Like [`Renderer::draw_overlay`], clipped to `clip` (screen pixels,
    /// top-left origin).
    pub fn draw_overlay_clipped(&mut self, atlas: &Atlas, quads: &[Quad], clip: Rect) {
        let sh = self.target_size.1 as i32;
        self.draw_overlay(atlas, &[]);
        unsafe {
            self.bind_screen();
            self.gl.enable(glow::SCISSOR_TEST);
            self.gl
                .scissor(clip.x, sh - clip.y - clip.h, clip.w, clip.h);
        }
        if !quads.is_empty() {
            let (sw, sh) = self.target_size;
            unsafe {
                let gl = &self.gl;
                gl.viewport(0, 0, sw as i32, sh as i32);
                gl.use_program(Some(self.sprite.program));
                gl.uniform_1_i32(self.sprite.u_tex.as_ref(), 0);
                gl.uniform_4_f32(
                    self.sprite.u_proj.as_ref(),
                    2.0 / sw as f32,
                    -2.0 / sh as f32,
                    -1.0,
                    1.0,
                );
            }
            self.draw_quads(atlas, quads);
        }
        unsafe {
            self.gl.disable(glow::SCISSOR_TEST);
        }
    }

    /// Draws quads (target pixels, y down) with the bound program and
    /// projection, batching by atlas page.
    fn draw_quads(&mut self, atlas: &Atlas, quads: &[Quad]) {
        self.draw_quads_turned(atlas, quads, None);
    }

    /// [`Renderer::draw_quads`] with optional per-quad rotations.
    fn draw_quads_turned(&mut self, atlas: &Atlas, quads: &[Quad], angles: Option<&[f32]>) {
        let size = atlas.page_size() as f32;
        let mut page = u16::MAX;
        self.verts.clear();
        for (i, q) in quads.iter().enumerate() {
            let r = atlas.region(q.region);
            if r.page != page || self.verts.len() >= BATCH_QUADS * 4 {
                self.flush();
                page = r.page;
                unsafe {
                    self.gl
                        .bind_texture(glow::TEXTURE_2D, Some(self.pages[page as usize]));
                }
            }
            let (u0, v0) = (r.x as f32 / size, r.y as f32 / size);
            let (u1, v1) = ((r.x + r.w) as f32 / size, (r.y + r.h) as f32 / size);
            let (x0, y0, x1, y1) = (q.x, q.y, q.x + q.w, q.y + q.h);
            let mut pos = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
            if let Some(a) = angles.and_then(|a| a.get(i)).filter(|a| **a != 0.0) {
                let (s, c) = a.sin_cos();
                let (cx, cy) = (q.x + q.w * 0.5, q.y + q.h * 0.5);
                for p in &mut pos {
                    let (dx, dy) = (p[0] - cx, p[1] - cy);
                    *p = [cx + dx * c - dy * s, cy + dx * s + dy * c];
                }
            }
            self.verts.extend_from_slice(&[
                Vertex {
                    pos: pos[0],
                    uv: [u0, v0],
                    color: q.color,
                },
                Vertex {
                    pos: pos[1],
                    uv: [u1, v0],
                    color: q.color,
                },
                Vertex {
                    pos: pos[2],
                    uv: [u1, v1],
                    color: q.color,
                },
                Vertex {
                    pos: pos[3],
                    uv: [u0, v1],
                    color: q.color,
                },
            ]);
        }
        self.flush();
    }

    fn flush(&mut self) {
        if self.verts.is_empty() {
            return;
        }
        let gl = &self.gl;
        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(self.ibo));
            gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, bytes_of(&self.verts));
            gl.enable_vertex_attrib_array(0);
            gl.enable_vertex_attrib_array(1);
            gl.enable_vertex_attrib_array(2);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, VERTEX_SIZE, 0);
            gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, VERTEX_SIZE, 8);
            gl.vertex_attrib_pointer_f32(2, 4, glow::UNSIGNED_BYTE, true, VERTEX_SIZE, 16);
            gl.draw_elements(
                glow::TRIANGLES,
                (self.verts.len() / 4 * 6) as i32,
                glow::UNSIGNED_SHORT,
                0,
            );
        }
        self.verts.clear();
    }

    /// Draws built-in UI quads (`crate::ui`: touch control icons, the
    /// fallback font) over the screen target, in order, alpha-blended.
    /// Call after the views, before presenting.
    pub fn draw_ui(&mut self, quads: &[crate::ui::UiQuad]) {
        use crate::ui::{UiSprite, font_texture, icon_texture, sprite_rect};
        if quads.is_empty() {
            return;
        }
        if self.ui_textures.is_none() {
            let make = |img: RgbaImage, filter: u32| unsafe {
                let t = new_texture(
                    &self.gl,
                    img.width as i32,
                    img.height as i32,
                    Some(&img.data),
                )
                .expect("ui texture");
                for p in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                    self.gl
                        .tex_parameter_i32(glow::TEXTURE_2D, p, filter as i32);
                }
                (t, img.width, img.height)
            };
            // Icons are scaled smoothly, the 5x7 font stays crisp.
            self.ui_textures = Some([
                make(icon_texture(), glow::LINEAR),
                make(font_texture(), glow::NEAREST),
            ]);
        }
        let textures = self.ui_textures.unwrap();
        let (sw, sh) = self.target_size;
        unsafe {
            let gl = &self.gl;
            self.bind_screen();
            gl.viewport(0, 0, sw as i32, sh as i32);
            gl.disable(glow::SCISSOR_TEST);
            gl.enable(glow::BLEND);
            gl.use_program(Some(self.sprite.program));
            gl.uniform_1_i32(self.sprite.u_tex.as_ref(), 0);
            gl.uniform_4_f32(
                self.sprite.u_proj.as_ref(),
                2.0 / sw as f32,
                -2.0 / sh as f32,
                -1.0,
                1.0,
            );
        }
        self.verts.clear();
        let mut bound = usize::MAX;
        for q in quads {
            let which = matches!(q.sprite, UiSprite::Glyph(_)) as usize;
            if which != bound || self.verts.len() >= BATCH_QUADS * 4 {
                self.flush();
                bound = which;
                unsafe {
                    self.gl
                        .bind_texture(glow::TEXTURE_2D, Some(textures[which].0));
                }
            }
            let (_, tw, th) = textures[which];
            let (rx, ry, rw, rh) = sprite_rect(q.sprite);
            let (u0, v0) = (rx as f32 / tw as f32, ry as f32 / th as f32);
            let (u1, v1) = ((rx + rw) as f32 / tw as f32, (ry + rh) as f32 / th as f32);
            let (x0, y0, x1, y1) = (q.x, q.y, q.x + q.w, q.y + q.h);
            for (pos, uv) in [
                ([x0, y0], [u0, v0]),
                ([x1, y0], [u1, v0]),
                ([x1, y1], [u1, v1]),
                ([x0, y1], [u0, v1]),
            ] {
                self.verts.push(Vertex {
                    pos,
                    uv,
                    color: q.color,
                });
            }
        }
        self.flush();
    }

    /// Reads the screen target back (top-down rows).
    pub fn read_screen(&self) -> RgbaImage {
        self.bind_screen();
        let (w, h) = self.target_size;
        unsafe { read_bound(&self.gl, w, h) }
    }

    /// Reads a view slot's level texture (the level at its integer render
    /// scale, before the effect pass).
    pub fn read_level_target(&self, slot: usize) -> Option<RgbaImage> {
        let t = self.level_targets.get(slot)?.as_ref()?;
        unsafe {
            self.gl
                .bind_framebuffer(glow::FRAMEBUFFER, Some(t.framebuffer));
            let mut img = read_bound(&self.gl, t.width, t.height);
            // Memory order is already top-down (see `draw_view`); undo the
            // flip `read_bound` does for y-up framebuffers.
            img.flip_vertical();
            self.bind_screen();
            Some(img)
        }
    }
}

unsafe fn read_bound(gl: &glow::Context, w: u32, h: u32) -> RgbaImage {
    let mut img = RgbaImage::new(w, h);
    unsafe {
        gl.pixel_store_i32(glow::PACK_ALIGNMENT, 4);
        gl.read_pixels(
            0,
            0,
            w as i32,
            h as i32,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut img.data)),
        );
    }
    img.flip_vertical();
    img
}

fn bytes_of<T: Copy>(v: &[T]) -> &[u8] {
    // SAFETY: plain-old-data slices (u16 indices, #[repr(C)] vertices
    // without padding) viewed as bytes.
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}

unsafe fn query_info(gl: &glow::Context) -> GlInfo {
    let v = gl.version();
    let flavor = if v.is_embedded {
        GlFlavor::Es
    } else if (v.major, v.minor) >= (3, 2) {
        // A 3.2+ context may still be compatibility; core-style GLSL works
        // in both as long as a VAO is bound.
        GlFlavor::Core
    } else {
        GlFlavor::Legacy
    };
    unsafe {
        let frag_highp = !v.is_embedded
            || gl
                .get_shader_precision_format(glow::FRAGMENT_SHADER, glow::HIGH_FLOAT)
                .is_some_and(|p| p.precision > 0);
        GlInfo {
            flavor,
            major: v.major,
            minor: v.minor,
            version: gl.get_parameter_string(glow::VERSION),
            renderer: gl.get_parameter_string(glow::RENDERER),
            vendor: gl.get_parameter_string(glow::VENDOR),
            max_texture_size: gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE).max(64) as u32,
            frag_highp,
        }
    }
}

unsafe fn compile(
    gl: &glow::Context,
    info: &GlInfo,
    fs_body: &str,
    prelude: &str,
) -> Result<Program> {
    unsafe {
        let program = gl.create_program()?;
        let sources = [
            (
                glow::VERTEX_SHADER,
                header(info.flavor, info.major, info.minor, false) + VS,
            ),
            (
                glow::FRAGMENT_SHADER,
                header(info.flavor, info.major, info.minor, true) + prelude + fs_body,
            ),
        ];
        let mut shaders = Vec::new();
        for (kind, src) in &sources {
            let sh = gl.create_shader(*kind)?;
            gl.shader_source(sh, src);
            gl.compile_shader(sh);
            if !gl.get_shader_compile_status(sh) {
                return Err(RenderError(format!(
                    "shader compile failed: {}\n{src}",
                    gl.get_shader_info_log(sh)
                )));
            }
            gl.attach_shader(program, sh);
            shaders.push(sh);
        }
        gl.bind_attrib_location(program, 0, "a_pos");
        gl.bind_attrib_location(program, 1, "a_uv");
        gl.bind_attrib_location(program, 2, "a_color");
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            return Err(RenderError(format!(
                "shader link failed: {}",
                gl.get_program_info_log(program)
            )));
        }
        for sh in shaders {
            gl.detach_shader(program, sh);
            gl.delete_shader(sh);
        }
        Ok(Program {
            u_proj: gl.get_uniform_location(program, "u_proj"),
            u_tex: gl.get_uniform_location(program, "u_tex"),
            u_time: gl.get_uniform_location(program, "u_time"),
            u_size: gl.get_uniform_location(program, "u_size"),
            program,
        })
    }
}

unsafe fn new_texture(
    gl: &glow::Context,
    w: i32,
    h: i32,
    data: Option<&[u8]>,
) -> Result<glow::Texture> {
    unsafe {
        let tex = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::NEAREST as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::NEAREST as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_S,
            glow::CLAMP_TO_EDGE as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_T,
            glow::CLAMP_TO_EDGE as i32,
        );
        // GLES2: internal format must equal the format (unsized RGBA).
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA as i32,
            w,
            h,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(data),
        );
        Ok(tex)
    }
}

unsafe fn new_offscreen(gl: &glow::Context, w: u32, h: u32) -> Result<Offscreen> {
    unsafe {
        let texture = new_texture(gl, w as i32, h as i32, None)?;
        let framebuffer = gl.create_framebuffer()?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(texture),
            0,
        );
        let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
        if status != glow::FRAMEBUFFER_COMPLETE {
            return Err(RenderError(format!(
                "offscreen {w}x{h} incomplete: 0x{status:x}"
            )));
        }
        Ok(Offscreen {
            texture,
            framebuffer,
            width: w,
            height: h,
        })
    }
}

unsafe fn delete_offscreen(gl: &glow::Context, o: Offscreen) {
    unsafe {
        gl.delete_framebuffer(o.framebuffer);
        gl.delete_texture(o.texture);
    }
}
