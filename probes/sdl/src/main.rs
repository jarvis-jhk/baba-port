//! SDL2 platform probe for Linux, PortMaster (aarch64), Windows and macOS.
//!
//! Opens a window with a GLES 2.0 context (falls back to desktop GL 2.1),
//! clears it, draws a textured quad through glow, plays a short beep, logs
//! controller/touch/key events, and in `--headless` mode renders a few frames
//! offscreen, reads the pixels back and checks them (exit code 1 on failure).
//!
//! Options: --headless  --frames N  --shot out.ppm  --virtual-pad
//!          --desktop-gl  --size WxH
use std::time::{Duration, Instant};

use glow::HasContext;
use sdl2::audio::{AudioCallback, AudioSpecDesired};
use sdl2::controller::GameController;
use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use sdl2::video::GLProfile;

const CLEAR: [f32; 3] = [0.1, 0.2, 0.45]; // rgb(26,51,115)
const TEX_A: [u8; 4] = [255, 200, 0, 255];
const TEX_B: [u8; 4] = [230, 40, 120, 255];

const VS: &str = "attribute vec2 a_pos;
uniform vec4 u_rect;
varying vec2 v_uv;
void main() {
    v_uv = a_pos * 0.5 + 0.5;
    gl_Position = vec4(u_rect.xy + a_pos * u_rect.zw, 0.0, 1.0);
}";
const FS: &str = "varying vec2 v_uv;
uniform sampler2D u_tex;
void main() { gl_FragColor = texture2D(u_tex, v_uv); }";

struct Beep {
    phase: f32,
    step: f32,
    left: u32,
}
impl AudioCallback for Beep {
    type Channel = f32;
    fn callback(&mut self, out: &mut [f32]) {
        for o in out.iter_mut() {
            if self.left > 0 {
                self.left -= 1;
                self.phase = (self.phase + self.step).fract();
                *o = (self.phase * std::f32::consts::TAU).sin() * 0.2;
            } else {
                *o = 0.0;
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    let val = |f: &str| {
        args.iter()
            .position(|a| a == f)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let headless = has("--headless");
    let frames: Option<u32> = val("--frames")
        .map(|v| v.parse().unwrap())
        .or(headless.then_some(30));
    let (w, h) = val("--size")
        .and_then(|s| {
            s.split_once('x')
                .map(|(a, b)| (a.parse().unwrap(), b.parse().unwrap()))
        })
        .unwrap_or((640u32, 480u32));
    if headless && std::env::var_os("SDL_VIDEODRIVER").is_none() {
        // SAFETY: single-threaded at this point.
        unsafe { std::env::set_var("SDL_VIDEODRIVER", "offscreen") };
    }

    // Controller events also without window focus (hidden window when headless).
    sdl2::hint::set("SDL_JOYSTICK_ALLOW_BACKGROUND_EVENTS", "1");
    // BABA_SDL_<X>=v becomes SDL hint SDL_<X>=v: Proton's Wine drops SDL_*
    // variables from the environment (scripts/windows/run-wine.sh maps them).
    for (k, v) in std::env::vars() {
        if let Some(name) = k.strip_prefix("BABA_SDL_") {
            sdl2::hint::set(&format!("SDL_{name}"), &v);
        }
    }
    let sdl = sdl2::init().expect("SDL_Init");
    let v = sdl2::version::version();
    println!(
        "SDL {}.{}.{} on {} (built against {}.{}.{})",
        v.major,
        v.minor,
        v.patch,
        sdl2::get_platform(),
        sdl2::sys::SDL_MAJOR_VERSION,
        sdl2::sys::SDL_MINOR_VERSION,
        sdl2::sys::SDL_PATCHLEVEL
    );
    let video = sdl.video().expect("video");
    println!("video driver: {}", video.current_video_driver());

    // GLES 2.0 first (what Android/PortMaster have), then desktop GL 2.1.
    let mut es = !has("--desktop-gl");
    let (window, _ctx) = loop {
        let attr = video.gl_attr();
        if es {
            attr.set_context_profile(GLProfile::GLES);
            attr.set_context_version(2, 0);
        } else {
            attr.set_context_profile(GLProfile::Compatibility);
            attr.set_context_version(2, 1);
        }
        let mut b = video.window("baba-port probe", w, h);
        b.opengl().position_centered();
        if headless {
            b.hidden();
        }
        let made = b.build().map_err(|e| e.to_string()).and_then(|win| {
            let ctx = win.gl_create_context()?;
            Ok((win, ctx))
        });
        match made {
            Ok(x) => break x,
            Err(e) if es => {
                println!("GLES 2.0 context failed ({e}), trying desktop GL 2.1");
                es = false;
            }
            Err(e) => panic!("no GL context: {e}"),
        }
    };
    let gl = unsafe {
        glow::Context::from_loader_function(|s| video.gl_get_proc_address(s) as *const _)
    };
    let s = |p| unsafe { gl.get_parameter_string(p) };
    println!(
        "GL ({}): {} | {} | {}",
        if es {
            "ES 2.0 requested"
        } else {
            "desktop 2.1"
        },
        s(glow::VERSION),
        s(glow::RENDERER),
        s(glow::VENDOR)
    );

    let header = if es {
        "#version 100\nprecision mediump float;\n"
    } else {
        "#version 120\n"
    };
    let (prog, u_rect) = unsafe {
        let prog = gl.create_program().unwrap();
        for (kind, src) in [(glow::VERTEX_SHADER, VS), (glow::FRAGMENT_SHADER, FS)] {
            let sh = gl.create_shader(kind).unwrap();
            gl.shader_source(sh, &format!("{header}{src}"));
            gl.compile_shader(sh);
            assert!(
                gl.get_shader_compile_status(sh),
                "shader: {}",
                gl.get_shader_info_log(sh)
            );
            gl.attach_shader(prog, sh);
        }
        gl.bind_attrib_location(prog, 0, "a_pos");
        gl.link_program(prog);
        assert!(
            gl.get_program_link_status(prog),
            "link: {}",
            gl.get_program_info_log(prog)
        );
        let quad: [f32; 8] = [-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0];
        let bytes: Vec<u8> = quad.iter().flat_map(|f| f.to_ne_bytes()).collect();
        let vbo = gl.create_buffer().unwrap();
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &bytes, glow::STATIC_DRAW);
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
        let u = gl.get_uniform_location(prog, "u_rect");
        (prog, u)
    };

    // Audio: a 300 ms beep (SDL_AUDIODRIVER=dummy works headless).
    let _audio = sdl.audio().ok().and_then(|a| {
        let want = AudioSpecDesired {
            freq: Some(48000),
            channels: Some(1),
            samples: Some(512),
        };
        match a.open_playback(None, &want, |spec| {
            println!(
                "audio: {} Hz, {} ch, buffer {} ({})",
                spec.freq,
                spec.channels,
                spec.samples,
                a.current_audio_driver()
            );
            Beep {
                phase: 0.0,
                step: 440.0 / spec.freq as f32,
                left: spec.freq as u32 * 3 / 10,
            }
        }) {
            Ok(dev) => {
                dev.resume();
                Some(dev)
            }
            Err(e) => {
                println!("audio unavailable: {e}");
                None
            }
        }
    });

    let gc = sdl.game_controller().expect("gamecontroller");
    println!("joysticks at start: {}", gc.num_joysticks().unwrap_or(0));
    let mut pads: Vec<GameController> = Vec::new();
    let mut virtual_idx = None;
    if has("--virtual-pad") {
        let i = unsafe {
            sdl2::sys::SDL_JoystickAttachVirtual(
                sdl2::sys::SDL_JoystickType::SDL_JOYSTICK_TYPE_GAMECONTROLLER,
                6,
                15,
                0,
            )
        };
        println!("virtual controller attached at index {i}");
        virtual_idx = (i >= 0).then_some(i);
    }

    let mut pump = sdl.event_pump().unwrap();
    let start = Instant::now();
    let mut frame = 0u32;
    let mut last = start;
    let mut since = 0u32;
    'run: loop {
        // Scripted input on the virtual controller (headless tests).
        if virtual_idx.is_some() && !pads.is_empty() {
            let joy =
                unsafe { sdl2::sys::SDL_JoystickFromInstanceID(pads[0].instance_id() as i32) };
            unsafe {
                match frame {
                    5 => {
                        sdl2::sys::SDL_JoystickSetVirtualButton(joy, 0, 1);
                    }
                    7 => {
                        sdl2::sys::SDL_JoystickSetVirtualButton(joy, 0, 0);
                    }
                    9 => {
                        sdl2::sys::SDL_JoystickSetVirtualAxis(joy, 0, 32767);
                    }
                    11 => {
                        sdl2::sys::SDL_JoystickSetVirtualAxis(joy, 0, 0);
                    }
                    _ => {}
                }
            }
        }
        for ev in pump.poll_iter() {
            match ev {
                Event::Quit { .. }
                | Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => break 'run,
                Event::ControllerDeviceAdded { which, .. } => match gc.open(which) {
                    Ok(c) => {
                        println!(
                            "controller added: #{which} '{}' mapping: {}",
                            c.name(),
                            c.mapping()
                        );
                        pads.push(c);
                    }
                    Err(e) => println!("controller #{which} failed to open: {e}"),
                },
                Event::ControllerDeviceRemoved { which, .. } => {
                    println!("controller removed: {which}")
                }
                Event::ControllerButtonDown { which, button, .. } => {
                    println!("pad {which} button down {button:?}")
                }
                Event::ControllerButtonUp { which, button, .. } => {
                    println!("pad {which} button up {button:?}")
                }
                Event::ControllerAxisMotion {
                    which, axis, value, ..
                } => println!("pad {which} axis {axis:?} = {value}"),
                Event::KeyDown { keycode, .. } => println!("key down {keycode:?}"),
                Event::FingerDown {
                    x, y, finger_id, ..
                } => println!("finger {finger_id} down at {x:.3},{y:.3}"),
                Event::MouseButtonDown {
                    x, y, mouse_btn, ..
                } => println!("mouse {mouse_btn:?} at {x},{y}"),
                _ => {}
            }
        }
        let (dw, dh) = window.drawable_size();
        unsafe {
            gl.viewport(0, 0, dw as i32, dh as i32);
            gl.clear_color(CLEAR[0], CLEAR[1], CLEAR[2], 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.use_program(Some(prog));
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 0, 0);
            let aspect = dw as f32 / dh.max(1) as f32;
            let (hx, hy) = if aspect > 1.0 {
                (0.5 / aspect, 0.5)
            } else {
                (0.5, 0.5 * aspect)
            };
            gl.uniform_4_f32(u_rect.as_ref(), 0.0, 0.0, hx, hy);
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        }
        frame += 1;
        since += 1;
        if let Some(n) = frames
            && frame >= n
        {
            let ok = check_pixels(&gl, dw as i32, dh as i32, val("--shot").as_deref());
            println!(
                "{frame} frames in {:.2}s; pixels {}",
                start.elapsed().as_secs_f32(),
                if ok { "OK" } else { "WRONG" }
            );
            std::process::exit(if ok { 0 } else { 1 });
        }
        window.gl_swap_window();
        if last.elapsed() >= Duration::from_secs(5) {
            println!(
                "{:.1} fps, drawable {dw}x{dh}",
                since as f32 / last.elapsed().as_secs_f32()
            );
            last = Instant::now();
            since = 0;
        }
        if headless {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// Reads back the frame (before the swap), checks a corner and the centre and
/// optionally writes a PPM.
fn check_pixels(gl: &glow::Context, w: i32, h: i32, shot: Option<&str>) -> bool {
    let mut px = vec![0u8; (w * h * 4) as usize];
    unsafe {
        gl.read_pixels(
            0,
            0,
            w,
            h,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut px)),
        );
    }
    let at = |x: i32, y: i32| {
        let i = ((y * w + x) * 4) as usize;
        [px[i], px[i + 1], px[i + 2]]
    };
    let near = |a: [u8; 3], b: [u8; 3]| {
        a.iter()
            .zip(b)
            .all(|(x, y)| (*x as i32 - y as i32).abs() <= 3)
    };
    let corner = at(2, 2);
    let centre = at(w / 2 + 3, h / 2 + 3);
    let clear = [26, 51, 115];
    let ok = near(corner, clear) && (near(centre, [255, 200, 0]) || near(centre, [230, 40, 120]));
    println!("readback {w}x{h}: corner {corner:?} centre {centre:?}");
    if let Some(path) = shot {
        let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
        for y in (0..h).rev() {
            for x in 0..w {
                out.extend_from_slice(&at(x, y));
            }
        }
        std::fs::write(path, out).expect("write shot");
        println!("wrote {path}");
    }
    ok
}
