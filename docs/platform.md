# Platform layer and renderer

How baba-port talks to the OS and the GPU, how to run it headless, and how
its output compares with the original. Per-target build notes (Android,
PortMaster, Windows, macOS) live in their own sections as they land.

## Overview

```
game (crates/game) ──► baba-platform::Platform  (window + GL context, events, audio, time)
        │                    └── sdl::SdlPlatform   (Windows, Linux, macOS, PortMaster)
        │                    └── android::AndroidPlatform (NativeActivity + EGL + AAudio, touch controls)
        └────────────► baba-render::Renderer      (glow, GLES 2.0 feature level)
                             └── headless::HeadlessContext (EGL, no window system)
```

- **Why not wgpu** (the sibling ports use it): PortMaster devices only have
  GLES2 over KMSDRM, and a minimal Android build should not carry a Vulkan/GL
  abstraction layer. `glow` is a thin GL loader; the renderer uses only
  OpenGL ES 2.0 features, so the same code runs on GLES 2/3, desktop GL 3.3
  core and GL 2.1.
- The renderer never sees a window: it gets a current `glow::Context` from
  the platform backend or from `HeadlessContext`.
- The game sees only the `Platform` trait and backend-neutral `Event`s, so an
  Android backend (e.g. NativeActivity + EGL + AAudio, no SDL/Java) can be
  added without touching game code. It reuses `baba_platform::audio`
  (mixer, Ogg decoding) unchanged: a backend only has to call
  `DeviceMixer::render` from its audio callback.

## `baba-platform`

`Platform` (crates/platform/src/lib.rs): `describe`, `drawable_size`,
`poll_events`, `swap_buffers`, `set_fullscreen`, `time` (monotonic),
`audio`, `devices`, `rumble`, `touch_controls` (on-screen controls where the backend runs them: the
game draws `items()` with `Renderer::draw_ui`, `baba_render::ui`; touches
on them already arrive as key events), `take_restored_gl` (after
`Event::Resume { gl_lost: true }`, for Android's context loss; the atlas
keeps CPU copies of its pages and can re-upload with `mark_all_dirty`).

Events: `Quit`, `Resized`, `Focus`, `Suspend`/`Resume`, `Key` (by physical
position; `Key::Char`, `Key::F`, `Key::Other(scancode)` for the rest),
`ControllerAdded/Removed` (hot-plug, stable `DeviceId::Controller(instance
id)`), `Button`/`Axis` (positional names: South = A/Cross), mouse (drawable
pixels, HiDPI-scaled), `Touch` (finger id, phase, 0..1 coordinates).

### SDL2 backend (`sdl::SdlPlatform`, feature `sdl2`, default on)

- **Linking.** `crates/platform/build.rs` adds `tools/lib` to the link path;
  `scripts/bootstrap-tools.sh` points `tools/lib/libSDL2.so` at the system's
  `libSDL2-2.0.so.0` (no -dev package needed), or unpacks Ubuntu's runtime
  package into `tools/sdl2-root` when the system has none. Windows cross
  builds look in `tools/windows/sdl2/x86_64-w64-mingw32/lib`;
  `BABA_SDL2_LIB_DIR` overrides. Runtime: the system/bundled SDL 2.0.14+
  (2.30 here).
- **GL context order** (`GlRequest::Auto`): desktop tries GL 3.3 core, then
  GLES 2.0, then GL 2.1; ARM Linux (PortMaster) tries GLES 2.0 first.
  `BABA_GL=gles2|gl33|gl21` forces one. The renderer picks the GLSL dialect
  from the context (`#version 100` for any ES, `330 core`/`150` for core,
  `120`/`130` for legacy) with a small macro prelude so there is one shader
  source.
- **Hints:** background joystick events, RawInput + HIDAPI (more than 4
  pads on Windows, for 12 local players), no minimise on focus loss (Linux),
  touch not mirrored as mouse.
- **Controllers:** every SDL game controller is opened, at start and on
  hot-plug; removal closes it. Mapping files from `WindowConfig::controller_db`
  are loaded after SDL's built-ins (the game binary passes the install's
  `gamecontrollerdb.txt`; a port-shipped copy can be added the same way).
  Joysticks without a mapping are listed by `devices()` with `mapped: false`.
  Rumble via `Platform::rumble`.
- **Virtual controllers** (`attach_virtual_controller`, `set_virtual_button`,
  `detach_virtual_controller`): SDL virtual joysticks, used by the hot-plug
  test (`crates/platform/tests/virtual_controllers.rs`: two pads plugged,
  button on the second, first unplugged) and by
  `baba-port --list-devices --virtual-pads N`. Runs on the private Xvfb or
  with SDL's `offscreen` video driver (no X at all).
- **Audio:** stereo `f32` at 44.1 kHz (the game's Ogg files are 44.1 kHz,
  so normally no resampling), 1024-frame buffers. The SDL callback owns the
  mixer; the game sends commands over a channel, so the callback never waits
  on the game thread.

### Audio (`baba_platform::audio`, pure Rust)

- `Mixer` (copied from our bopl-port mixer and extended): voices with
  volume, pitch, pan (constant power), looping, delayed start, buses
  (music / sfx / ui) and master volume, click-free gain ramps.
- Sources: decoded samples (`SoundData`, for sounds) or a `FrameStream`
  (music). `OggStream` decodes Ogg Vorbis (`lewton`) packet by packet while
  it plays and loops by rewinding: a song costs its compressed size in
  memory (~2-4 MB) instead of ~50-90 MB decoded, which matters on 1 GB
  handhelds.
- `Audio` is the game's front end; the same API drives a device or an
  offline mixer that writes a WAV (headless runs, tests).
- Sounds: the game's sound effects are Ogg files inside `Assets.dat`
  (`baba_formats::AssetsDat`); `--audio-test` streams a song and plays a
  few of those over it. Checked through SDL's `disk` driver (2.9 s of
  f32 stereo written, peak 0.32, RMS 0.061) and the offline WAV path.

## `baba-render`

- **Atlas** (`atlas.rs`): sprites are shelf-packed at run time from the
  player's PNGs into pages of `Renderer::atlas_page_size()` (≤ 2048; 1024 on
  ES devices without `highp` in fragment shaders, where `mediump` texture
  coordinates cannot address single texels of bigger pages). 1 px transparent
  gutter. CPU copies are kept (context loss, CPU reference).
- **Sprites/palettes** (`sprites.rs`): `<sprite>_<frame>_<wobble>.png`,
  world folder first, loaded on demand; RGB sprites' black `tRNS` colour key
  becomes alpha 0. Palette colours are looked up on the CPU and passed as
  vertex colours (sprite texel × colour, straight alpha).
- **Scene** (`scene.rs`): `TileScene` = grid size, tile size (24), level
  and outside colours, border ring width, sprites with tile position (float,
  for later tweening), three wobble frames, colour, per-sprite wobble phase
  and layer. `quads()` flattens it into a draw list (stable sort by layer;
  sprites not 24×24 are centred on their cell, rounded down).
- **Views** (`renderer.rs`, `viewport.rs`): each view renders its level into
  its own offscreen texture at the integer scale k = ceil(final scale) with
  nearest sampling, then the *effect pass* composites it into the viewport:
  1:1 (nearest, pixel-exact) when the final scale is the integer k, else
  linear filtering (mild downscale from k, i.e. sharp-bilinear). The effect
  pass is the hook for level rotation, zoom, offset (shake / level
  movement), tint (fades) and custom fragment shaders
  (`Renderer::register_effect`, uniforms `u_tex`, `u_time`, `u_size`).
  Viewports are scissored, so rotated levels never spill into neighbours.
- **Layout:** `tile(w, h, n, …)` picks the column count that gives n views
  the largest fitted scale (incomplete last row centred, gap in px);
  `fit(view, …, ScaleMode)`: `Integer`, `IntegerIsh { min_fill }` (default
  0.85: integer when it fills ≥ 85 % of the exact fit; down-scales always
  fractional) or `Fit`.
- **ES 2 constraints honoured:** u16 indices (4096 quads per draw), one
  texture unit, NPOT textures with clamp-to-edge and no mipmaps, unsized
  `RGBA` textures as render targets, no VAOs on ES (one VAO bound once on
  core profiles), `glReadPixels` RGBA/UNSIGNED_BYTE only.

### Headless (`headless.rs`)

`HeadlessContext` loads `libEGL.so.1` at run time (`khronos-egl`, dynamic)
and creates a context without any window system, in this order: Mesa
**surfaceless** platform (`EGL_MESA_platform_surfaceless`), EGL device
platform, default display; surfaceless contexts if
`EGL_KHR_surfaceless_context`, else a 16×16 pbuffer. Rendering goes to an
FBO (`Renderer::use_offscreen_screen`). Default API GLES 2
(`BABA_HEADLESS_GL=gl33` for desktop core). On this laptop it picks the
Intel GPU (iris); `LIBGL_ALWAYS_SOFTWARE=1` forces llvmpipe (what CI gets);
`MESA_GLES_VERSION_OVERRIDE=2.0` gives a strict ES 2.0 context. All three
were checked. CI needs only `libegl1` + `libegl-mesa0` (Mesa) — no X, no
Xvfb. Chosen over OSMesa (deprecated/removed in recent Mesa) and Xvfb+GLX
(needs a server).

### Tests

- `crates/render/tests/headless_gl.rs`: a synthetic level (odd-sized,
  translucent, overlapping sprites over several atlas pages, border ring)
  rendered on GLES 2 and GL 3.3 core at scales 1–3 and compared with the
  **CPU reference rasterizer** (`cpu.rs`: same quads, nearest sampling,
  straight-alpha over): max channel error 1 (rounding), both for the level
  texture and the composited screen. Also: 4 tiled rotated views, a custom
  effect shader (inverted colours vs CPU). Skips without EGL unless
  `BABA_REQUIRE_GL=1`; `BABA_TEST_DUMP=<dir>` writes the frames.
- `image.rs`: `diff` (mean/max error, differing pixels over a threshold,
  PSNR), `diff_image` (red = differing), `crop`, `resize_area`,
  `scale_nearest` for comparing captures.
- `crates/platform`: mixer unit tests (sum, resample, pitch, pan, buses,
  loop, delay, voice stealing, streams) and the virtual controller test
  (`BABA_REQUIRE_SDL=1` to fail instead of skip).
- `baba_platform::touch`: on-screen controls (layout, keys, slide, repeat,
  hide on controller use); `headless_gl::ui_overlay`: `Renderer::draw_ui`
  (icons, translucent disc, built-in font) on GLES 2.
- `crates/game`: every level of every world in the install (600) builds a
  scene with a sprite for every unit (skips without `BABA_GAME_DIR`).

## `baba-port` (M2 viewer; the game itself: `docs/game-flow.md`)

Without options `baba-port` runs the game (`--data-dir`, `--skip-boot`,
`--frames N --screenshot OUT` for scripted window captures); `--drive
SCRIPT` runs a scripted headless session. The level viewer below is
selected by `--level` or `--headless`.

Always `source env.sh` first. Never on the desktop: `--headless`, or
`DISPLAY=:68` (`scripts/dev/xvfb.sh`), and `SDL_AUDIODRIVER=disk` (or
`dummy`) so nothing plays on the laptop's speakers.

```
baba-port --headless --level baba/0level --screenshot scratch/m2/l00.png [--size 854x480]
          [--views N] [--rotate DEG] [--scale integer|integerish|fit] [--wobble N] [--seed N]
          [--compare REF.png [--threshold N]] [--uniform-wobble]
DISPLAY=:68 baba-port --level baba/89level [--frames N --screenshot out.png] [--music baba]
DISPLAY=:68 baba-port --list-devices [--virtual-pads N]
SDL_AUDIODRIVER=disk SDL_DISKAUDIOFILE=scratch/m2/a.raw baba-port --audio-test [--music NAME] [--seconds S]
baba-port --audio-test --offline scratch/m2/a.wav
```

Windowed mode: Esc quits, 1–9 sets the number of splitscreen views,
controller events are logged. With `--frames` the wobble is held (stable
captures) and the last frame is read back before the swap.

Level → scene (`crates/game/src/level_view.rs`, presentation spec §1–§5):
object table from `values.lua` with the level's `[tiles]` overrides; the
edge ring is not drawn and shows `(1,0)` like the area around the level;
interior `(0,4)`; tiled objects join same-name neighbours and the border
(§4.3); variants via `baba_formats::sprite_variant`; colours from the
level's palette; random wobble phase per unit from a cosmetic RNG. **Active
text** is a placeholder guess (`NOUN VERB NOUN|PROPERTY` in a row or column)
until `baba-sim` reports rule state.

## Comparison with the original

Reference: `scratch/ref/l00_start.png` (the original, `baba/0level` "baba
is you", 854×480 window, headless Xvfb capture by the reference agent).

| metric (854×480, ours vs original) | value |
|---|---|
| whole frame, our random wobble phases | mean \|d\| 0.96, 0.71 % px differ (> 8), PSNR 25.0 dB |
| level area (840×480), wobble matched per cell (best of 3 phases) | mean \|d\| 0.67, 0.27 % px differ |
| …of which outside the tutorial "controls" special (cells 15–19 × 2–5) | **0 px differ at threshold 0** (pixel-exact) |

So, confirmed on the original: the 854×480 window shows the full 35×20 room
(edge ring included) at scale 1, centred (x offset 7); interior `(0,4)`,
ring and surroundings `(1,0)`, edge not drawn; tint = texel × palette colour,
black colour key on RGB sprites; sprite placement, wall tiling masks, draw
order and the active colours of the four rules on this level. The only
differences are the level's `[specials]` "controls" entries (arrow-key icons
and the MOVE label), drawn by the engine from `keyboardupper.png` etc.: not
implemented yet. Run it again with
`baba-port --headless --level baba/0level --screenshot scratch/m2/l00.png --compare scratch/ref/l00_start.png --threshold 0`
(writes `l00.diff.png` and `l00.wobble-matched.diff.png`).

Windowed output on Xvfb (llvmpipe via GLX: GL 3.3 core, GLES 2, GL 2.1) is
byte-identical to the headless EGL output (iris, ES 3.2).

## Gaps / next

- Engine-drawn level extras: `[specials]` (tutorial controls), map
  background `[images]`, level icons/paths/UI on maps (M4), particles,
  transitions, movement tween, blinking (M5).
- Zoom modes and window sizes other than 854×480 (spec §1.4): our default
  is integer-ish fit of the whole room; on 640×480 the 840×480 room is
  downscaled to 0.76 (linear) — consider cropping the edge ring on small
  screens (M10).
- Wobble period and global phases: measured (10 frames, one global timer;
  `docs/presentation.md`).
- Text input (editor, level codes), on-screen keyboard: later.
- Android backend: done (`baba_platform::android`, `docs/platform-android.md`).
- Fullscreen toggle and window icon UX, display selection: M4.

## Target pipelines (cross-platform probes)

Per-target build, toolchain size and verification: `platform-android.md`
(NativeActivity, no Java, tested on the tablet), `platform-portmaster.md`
(aarch64 glibc ≥ 2.17, qemu-user + llvmpipe GLES2), `platform-windows.md`
(llvm-mingw, Wine), `platform-macos.md` (CI workflow, `cargo check` from
Linux). Probes: `probes/android`, `probes/sdl` (own cargo workspaces).
Findings that concern this crate pair:

- Android has no SDL2: the `android-activity` backend (EGL + glow, AAudio,
  NDK input queue, raw JNI) implements the `Platform` trait next to the
  SDL2 one (`baba_platform::android`, feature `android`).
- APK size: 270 KB (probe) → 1.71 MB (interim player) → 1.95 MB (the
  full game `App`, `docs/platform-android.md`). The growth
  is the game itself (sim, formats, renderer, PNG/Ogg decoders, menus,
  progression, TrueType rasteriser) plus the
  Warpzone client with a pure-Rust TLS stack (rustls, RustCrypto, webpki
  roots), which online play needs; no Java, no extra native libraries.
  The toolchain did not grow (still rust-std + the 1.6 MB NDK pieces).
  Since M7 the game library itself links `baba-net` (online play on
  every platform); `cargo check --target aarch64-linux-android -p
  baba-port --no-default-features` passes with `CC=/bin/false` (no C
  compiler, no ring). APK not re-measured yet.
- No GLES context on Windows under Wine (no WGL ES profile, no ANGLE) and
  never on macOS: desktop GL 2.1/3.3 must keep working there (3.3 core on
  macOS needs a VAO).
- Proton's Wine drops `SDL_*` environment variables; the probes read
  `BABA_SDL_<X>` as SDL hint `SDL_<X>` (`scripts/windows/run-wine.sh`
  forwards them that way); so does the game (`baba_platform::sdl::set_hints`).
- `SDL_JOYSTICK_ALLOW_BACKGROUND_EVENTS=1` is needed for controller events
  in hidden/headless windows; SDL's virtual joystick drives headless
  controller tests.
