//! `baba-port`: the desktop game binary (SDL2). Without options it runs
//! the game; `--drive` runs a scripted headless session; `--level` shows a
//! single level (headless screenshots, comparisons); `--list-devices` and
//! `--audio-test` check the platform.
//!
//! Always run with `source env.sh` (sets `BABA_GAME_DIR` and the data dir).

// Windows release builds open no console window next to the game (output
// still reaches a pipe or a redirected file, e.g. the Warpzone launcher's
// log); debug builds keep the console.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod gamedir;
mod logger;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use baba_game::level_view::{self, GameData, LoadedLevel};
use baba_platform::audio::{self, Audio, PlayParams, SoundData};
use baba_platform::{Event, Key, Platform, WindowConfig};
use baba_render::{
    Atlas, PostEffect, Rect, Renderer, RgbaImage, ScaleMode, TileScene, View, diff, diff_image,
};

/// Wobble frame period. Placeholder until captured
/// (`docs/spec/presentation.md` §3.3).
const WOBBLE_MS: u64 = 200;

const USAGE: &str = "\
usage: baba-port [options]
  (no options)          play the game in a window
  --version             print the version and exit
  --game-dir DIR        the game install (default: $BABA_GAME_DIR, $WARPZONE_DATA_BABA,
                        gamedata/ next to the binary, <data dir>/game, Steam, GOG)
  --data-dir DIR        saves and settings (default: $BABA_PORT_DATA_DIR or the
                        platform's user data dir + baba-port)
  --skip-boot           no attribution card
  --fullscreen          start fullscreen (PortMaster's launcher)
  --drive SCRIPT        run a scripted session headless (see crates/game/src/drive.rs)
  --window              --drive: on the real SDL window (--size) instead of headless
  --render-every N      --drive --window: present every N ticks (default 1)
  --out DIR             --drive: screenshot directory (default scratch/m4/drive)
  --corpus FILE         --drive: solution corpus (default $BABA_CORPUS or
                        $BABA_ROOT/ref/tas/corpus/corpus.tsv)
  --warpzone-host       open the online menu to host a room (Warpzone launcher)
  --warpzone-join ROOM  join a room (id or warpzone://join/... invite link)
  --level WORLD/FILE    show one level (viewer; default baba/0level)
  --headless            no window: render through EGL into an image
  --screenshot OUT.png  save a frame (headless, or windowed after --frames)
  --size WxH            output size (default 854x480, the original's window)
  --views N             N splitscreen views of the level
  --wobble N            wobble counter for the frame (default 0)
  --seed N              cosmetic seed for wobble phases (default 1)
  --uniform-wobble      every unit on wobble phase 0 (for comparisons)
  --scale MODE          integer | integerish | fit (default integerish)
  --rotate DEG          effect pass rotation (degrees)
  --compare REF.png     compare the screenshot with a capture of the original
  --threshold N         per-channel difference that counts as differing (8)
  --frames N            windowed: quit after N frames
  --music NAME          play Data/Music/NAME.ogg
  --list-devices        print video/audio/controller/GL information
  --virtual-pads N      with --list-devices: hot-plug N virtual controllers
  --audio-test          play music + a sound through SDL audio
  --seconds S           audio test length (default 3)
  --offline OUT.wav     audio test through the offline mixer into a WAV";

#[derive(Debug)]
struct Args {
    game_dir: PathBuf,
    level: String,
    headless: bool,
    screenshot: Option<PathBuf>,
    size: (u32, u32),
    views: usize,
    wobble: u32,
    seed: u64,
    uniform_wobble: bool,
    scale: ScaleMode,
    rotate: f32,
    compare: Option<PathBuf>,
    threshold: u8,
    frames: Option<u64>,
    music: Option<String>,
    list_devices: bool,
    virtual_pads: u32,
    audio_test: bool,
    seconds: f64,
    offline: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    skip_boot: bool,
    fullscreen: bool,
    window: bool,
    render_every: u32,
    drive: Option<PathBuf>,
    out: PathBuf,
    corpus: Option<PathBuf>,
    level_given: bool,
    /// `--warpzone-host`, `--warpzone-join ROOM`, invite links.
    online: Vec<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        // The Warpzone launcher hands the install over as
        // WARPZONE_DATA_BABA (packaging/warpzone/baba-port.toml `[[data]]`).
        game_dir: std::env::var_os("BABA_GAME_DIR")
            .or_else(|| std::env::var_os("WARPZONE_DATA_BABA"))
            .map(PathBuf::from)
            .unwrap_or_default(),
        level: "baba/0level".into(),
        headless: false,
        screenshot: None,
        size: (854, 480),
        views: 1,
        wobble: 0,
        seed: 1,
        uniform_wobble: false,
        scale: ScaleMode::default(),
        rotate: 0.0,
        compare: None,
        threshold: 8,
        frames: None,
        music: None,
        list_devices: false,
        virtual_pads: 0,
        audio_test: false,
        seconds: 3.0,
        offline: None,
        data_dir: baba_progress::default_data_dir(),
        skip_boot: false,
        fullscreen: false,
        window: false,
        render_every: 1,
        drive: None,
        out: PathBuf::from("scratch/m4/drive"),
        corpus: std::env::var_os("BABA_CORPUS")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("BABA_ROOT")
                    .map(|r| PathBuf::from(r).join("ref/tas/corpus/corpus.tsv"))
            }),
        level_given: false,
        online: Vec::new(),
    };
    let mut game_dir_given = false;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut val = || it.next().ok_or_else(|| format!("{arg} needs a value"));
        let num = |s: String| s.parse::<f64>().map_err(|e| format!("{s}: {e}"));
        match arg.as_str() {
            "--game-dir" => {
                a.game_dir = val()?.into();
                game_dir_given = true;
            }
            "--version" => {
                println!("baba-port {}", version());
                std::process::exit(0);
            }
            "--level" => {
                a.level = val()?;
                a.level_given = true;
            }
            "--data-dir" => a.data_dir = Some(val()?.into()),
            "--skip-boot" => a.skip_boot = true,
            "--fullscreen" => a.fullscreen = true,
            "--window" => a.window = true,
            "--render-every" => a.render_every = num(val()?)? as u32,
            "--drive" => a.drive = Some(val()?.into()),
            "--out" => a.out = val()?.into(),
            "--corpus" => a.corpus = Some(val()?.into()),
            "--headless" => a.headless = true,
            "--screenshot" => a.screenshot = Some(val()?.into()),
            "--size" => {
                let v = val()?;
                let (w, h) = v.split_once('x').ok_or("--size WxH")?;
                a.size = (
                    w.parse().map_err(|_| "--size WxH")?,
                    h.parse().map_err(|_| "--size WxH")?,
                );
            }
            "--views" => a.views = num(val()?)?.max(1.0) as usize,
            "--wobble" => a.wobble = num(val()?)? as u32,
            "--seed" => a.seed = num(val()?)? as u64,
            "--uniform-wobble" => a.uniform_wobble = true,
            "--scale" => {
                a.scale = match val()?.as_str() {
                    "integer" => ScaleMode::Integer,
                    "fit" => ScaleMode::Fit,
                    _ => ScaleMode::default(),
                }
            }
            "--rotate" => a.rotate = num(val()?)? as f32,
            "--compare" => a.compare = Some(val()?.into()),
            "--threshold" => a.threshold = num(val()?)? as u8,
            "--frames" => a.frames = Some(num(val()?)? as u64),
            "--music" => a.music = Some(val()?),
            "--list-devices" => a.list_devices = true,
            "--virtual-pads" => a.virtual_pads = num(val()?)? as u32,
            "--audio-test" => a.audio_test = true,
            "--seconds" => a.seconds = num(val()?)?,
            "--offline" => a.offline = Some(val()?.into()),
            // Online: the launcher / invite links (`App::launch_online`).
            "--warpzone-host" => a.online.push(arg.clone()),
            "--warpzone-join" => {
                let v = val()?;
                a.online.push(arg.clone());
                a.online.push(v);
            }
            s if s.starts_with("--warpzone-join=") || s.starts_with("warpzone://") => {
                a.online.push(arg.clone())
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => return Err(format!("unknown option {other}\n{USAGE}")),
        }
    }
    // Release builds start without env.sh: look for the install
    // (gamedir.rs). When nothing is found, keep $BABA_GAME_DIR (PortMaster's
    // gamedata/) or suggest <data dir>/game, which the no-data screen shows.
    if game_dir_given {
        // `--game-dir` may name a macOS `.app`, `Data/`, ...
        if let Some(root) = gamedir::resolve(&a.game_dir) {
            a.game_dir = root;
        }
    } else {
        a.game_dir = gamedir::find(a.data_dir.as_deref()).unwrap_or_else(|| {
            if a.game_dir.as_os_str().is_empty() {
                a.data_dir
                    .as_ref()
                    .map(|d| d.join("game"))
                    .unwrap_or_default()
            } else {
                a.game_dir.clone()
            }
        });
    }
    Ok(a)
}

/// `CARGO_PKG_VERSION`, plus the build id CI passes in `BABA_BUILD_ID`
/// (the commit), if any.
fn version() -> String {
    match option_env!("BABA_BUILD_ID") {
        Some(id) if !id.is_empty() => format!("{} ({id})", env!("CARGO_PKG_VERSION")),
        _ => env!("CARGO_PKG_VERSION").to_string(),
    }
}

fn main() {
    logger::init();
    let result = parse_args().and_then(|args| {
        if args.list_devices {
            list_devices(&args)
        } else if args.audio_test {
            audio_test(&args)
        } else if let Some(script) = &args.drive {
            drive(&args, script)
        } else if args.headless {
            headless(&args)
        } else if args.level_given {
            windowed(&args)
        } else {
            play(&args)
        }
    });
    if let Err(e) = result {
        eprintln!("baba-port: {e}");
        std::process::exit(1);
    }
}

fn app_config(args: &Args) -> Result<baba_game::AppConfig, String> {
    if args.game_dir.as_os_str().is_empty() {
        return Err("no game directory: set BABA_GAME_DIR (source env.sh) or --game-dir".into());
    }
    if args.data_dir.is_none() {
        return Err(
            "no data directory: set BABA_PORT_DATA_DIR (source env.sh) or --data-dir".into(),
        );
    }
    Ok(baba_game::AppConfig {
        game_dir: args.game_dir.clone(),
        data_dir: args.data_dir.clone(),
        skip_boot: args.skip_boot,
    })
}

/// The game in a window.
fn play(args: &Args) -> Result<(), String> {
    let cfg = app_config(args)?;
    let settings = cfg
        .data_dir
        .as_ref()
        .map(|d| baba_progress::Profile::load(d).settings)
        .unwrap_or_default();
    let wc = WindowConfig {
        fullscreen: args.fullscreen || settings.toggle(baba_progress::Toggle::Fullscreen),
        ..window_config(args)
    };
    let (mut p, gl) = baba_platform::sdl::SdlPlatform::new(&wc)?;
    log::info!("{}", p.describe());
    if !cfg.game_dir.join("Data/values.lua").is_file() {
        return no_data(
            &mut p,
            gl,
            &cfg.game_dir,
            args.frames,
            args.screenshot.as_deref(),
        );
    }
    let mut app = baba_game::App::new(gl, cfg)?;
    app.launch_online(&args.online);
    for d in p.devices() {
        if matches!(d.id, baba_platform::DeviceId::Controller(_)) {
            app.handle_event(&Event::ControllerAdded(d));
        }
    }
    let mut n = 0u64;
    while app.frame(&mut p) {
        n += 1;
        if args.frames.is_some_and(|f| n >= f) {
            let img = app.read_screen();
            p.swap_buffers();
            return finish_screenshot(args, img, None);
        }
        p.swap_buffers();
    }
    Ok(())
}

/// Where the game data is missing (a fresh PortMaster install): says
/// where to put it, in the renderer's built-in font, until any key or
/// button (or `frames`, then `screenshot`).
fn no_data(
    p: &mut baba_platform::sdl::SdlPlatform,
    gl: baba_platform::glow::Context,
    game_dir: &std::path::Path,
    frames: Option<u64>,
    screenshot: Option<&std::path::Path>,
) -> Result<(), String> {
    use baba_render::ui::{text_quads, wrap};
    // The last two folders (ports/babaport/gamedata on PortMaster): long
    // absolute paths do not wrap.
    let parts: Vec<String> = game_dir
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let short = match parts.len() {
        0..=2 => game_dir.display().to_string(),
        n => format!(".../{}", parts[n - 2..].join("/")),
    };
    let msg = format!(
        "baba-port needs your own copy of Baba Is You.\n\nCopy Data/ and Assets.dat \
         (and gamecontrollerdb.txt) from your install (Steam/GOG, any OS) into\n{}\n\n\
         Press any button to quit.",
        short
    );
    log::error!("no game data in {}", game_dir.display());
    let mut r = Renderer::new(gl).map_err(|e| e.to_string())?;
    let mut events = Vec::new();
    let mut n = 0u64;
    loop {
        n += 1;
        events.clear();
        p.poll_events(&mut events);
        let done = events.iter().any(|e| {
            matches!(
                e,
                Event::Quit | Event::Key { down: true, .. } | Event::Button { down: true, .. }
            )
        });
        let (w, h) = p.drawable_size();
        let scale = (w.min(h) as f32 / 240.0).floor().max(1.0);
        let lines = wrap(&msg, ((w as f32 * 0.9) / (6.0 * scale)) as usize);
        let top = (h as f32 - lines.len() as f32 * 11.0 * scale) / 2.0;
        let quads: Vec<_> = lines
            .iter()
            .enumerate()
            .flat_map(|(i, l)| {
                text_quads(
                    l,
                    w as f32 * 0.05,
                    top + i as f32 * 11.0 * scale,
                    scale,
                    [255; 4],
                )
            })
            .collect();
        r.begin_frame(w, h, [12, 12, 20, 255]);
        r.draw_ui(&quads);
        if done || frames.is_some_and(|f| n >= f) {
            let img = r.read_screen();
            p.swap_buffers();
            if let Some(out) = screenshot {
                img.save_png(out).map_err(|e| e.to_string())?;
            }
            return Ok(());
        }
        p.swap_buffers();
    }
}

/// A scripted session: headless, or on the SDL window with `--window`.
fn drive(args: &Args, script: &std::path::Path) -> Result<(), String> {
    let cfg = app_config(args)?;
    let text = std::fs::read_to_string(script).map_err(|e| format!("{}: {e}", script.display()))?;
    let report = if args.window {
        let (mut p, gl) = baba_platform::sdl::SdlPlatform::new(&window_config(args))?;
        log::info!("{}", p.describe());
        baba_game::drive::run_on(
            &mut p,
            gl,
            args.render_every,
            &text,
            cfg,
            &args.out,
            args.corpus.clone(),
        )?
    } else {
        baba_game::drive::run(&text, cfg, &args.out, args.corpus.clone())?
    };
    println!(
        "drive: {} screenshots, {} failures",
        report.shots.len(),
        report.failures.len()
    );
    if report.failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} failures:\n{}",
            report.failures.len(),
            report.failures.join("\n")
        ))
    }
}

fn open_data(args: &Args) -> Result<GameData, String> {
    if args.game_dir.as_os_str().is_empty() {
        return Err("no game directory: set BABA_GAME_DIR (source env.sh) or --game-dir".into());
    }
    GameData::open(&args.game_dir)
}

/// The level, its scene and a filled atlas.
fn prepare(args: &Args, page_size: u32) -> Result<(LoadedLevel, TileScene, Atlas), String> {
    let data = open_data(args)?;
    let level = LoadedLevel::load(&data, &args.level)?;
    let mut atlas = Atlas::new(page_size);
    let mut sprites = level_view::SpriteSource::new(&data, &level.world);
    let scene = level.scene(&mut sprites, &mut atlas, args.seed, args.uniform_wobble);
    log::info!(
        "{} \"{}\": {}x{} cells, {} sprites, atlas {} images on {} page(s)",
        args.level,
        level.info.general.name,
        scene.cols,
        scene.rows,
        scene.sprites.len(),
        atlas.len(),
        atlas.page_count()
    );
    Ok((level, scene, atlas))
}

/// How views are laid out and composited.
#[derive(Clone, Copy)]
struct DrawOpts {
    views: usize,
    scale: ScaleMode,
    rotate: f32,
}

impl From<&Args> for DrawOpts {
    fn from(a: &Args) -> Self {
        Self {
            views: a.views,
            scale: a.scale,
            rotate: a.rotate,
        }
    }
}

fn draw(
    r: &mut Renderer,
    args: &DrawOpts,
    scene: &TileScene,
    atlas: &Atlas,
    wobble: u32,
    size: (u32, u32),
    time: f32,
) {
    let (w, h) = size;
    r.begin_frame(w, h, scene.outside_bg);
    let (lw, lh) = scene.pixel_size();
    let rects = if args.views <= 1 {
        vec![Rect::new(0, 0, w as i32, h as i32)]
    } else {
        baba_render::tile(w as i32, h as i32, args.views, lw, lh, 2)
    };
    for (i, rect) in rects.into_iter().enumerate() {
        let view = View {
            scene,
            wobble: wobble + i as u32,
            rect,
            scale: args.scale,
            effect: PostEffect {
                rotation: args.rotate.to_radians(),
                time,
                ..Default::default()
            },
        };
        r.draw_view(i, atlas, &view);
    }
}

fn headless(args: &Args) -> Result<(), String> {
    use baba_render::headless::{HeadlessApi, HeadlessContext};
    let (_ctx, gl) = HeadlessContext::new(HeadlessApi::from_env())?;
    let mut r = Renderer::new(gl).map_err(|e| e.to_string())?;
    let (_level, scene, mut atlas) = prepare(args, r.atlas_page_size())?;
    r.sync_atlas(&mut atlas);
    r.use_offscreen_screen(args.size.0, args.size.1)
        .map_err(|e| e.to_string())?;
    draw(
        &mut r,
        &DrawOpts::from(args),
        &scene,
        &atlas,
        args.wobble,
        args.size,
        0.0,
    );
    let img = r.read_screen();
    finish_screenshot(args, img, Some((&scene, &atlas)))
}

fn finish_screenshot(
    args: &Args,
    img: RgbaImage,
    scene: Option<(&TileScene, &Atlas)>,
) -> Result<(), String> {
    if let Some(out) = &args.screenshot {
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        img.save_png(out).map_err(|e| e.to_string())?;
        println!("wrote {} ({}x{})", out.display(), img.width, img.height);
    }
    if let Some(reference) = &args.compare {
        compare(args, &img, reference, scene)?;
    }
    Ok(())
}

/// Compares with a capture of the original: whole frame, plus a
/// wobble-agnostic score (per cell, the best of the three uniform wobble
/// phases, since the original picks phases at random).
fn compare(
    args: &Args,
    img: &RgbaImage,
    reference: &PathBuf,
    scene: Option<(&TileScene, &Atlas)>,
) -> Result<(), String> {
    let refimg =
        RgbaImage::load_png(reference).map_err(|e| format!("{}: {e}", reference.display()))?;
    if (refimg.width, refimg.height) != (img.width, img.height) {
        return Err(format!(
            "reference is {}x{}, screenshot {}x{} (use --size)",
            refimg.width, refimg.height, img.width, img.height
        ));
    }
    let d = diff(img, &refimg, args.threshold);
    println!("frame vs {}: {d}", reference.display());
    if let Some(out) = &args.screenshot {
        let p = out.with_extension("diff.png");
        diff_image(img, &refimg, args.threshold)
            .save_png(&p)
            .map_err(|e| e.to_string())?;
        println!("diff image {}", p.display());
    }
    if let Some((scene, atlas)) = scene
        && args.views <= 1
        && args.rotate == 0.0
    {
        let (lw, lh) = scene.pixel_size();
        let f = baba_render::fit(
            Rect::new(0, 0, img.width as i32, img.height as i32),
            lw,
            lh,
            args.scale,
        );
        if !f.integer {
            return Ok(());
        }
        let k = f.scale as u32;
        let mut uniform = scene.clone();
        for s in &mut uniform.sprites {
            s.wobble = 0;
        }
        let phases: Vec<RgbaImage> = (0..3)
            .map(|p| baba_render::cpu::render_scene(&uniform, atlas, p, k))
            .collect();
        let target = refimg.crop(
            f.rect.x as u32,
            f.rect.y as u32,
            f.rect.w as u32,
            f.rect.h as u32,
        );
        let cell = 24 * k;
        let mut best = RgbaImage::new(target.width, target.height);
        for cy in 0..scene.rows {
            for cx in 0..scene.cols {
                let (x, y) = (cx * cell, cy * cell);
                let t = target.crop(x, y, cell, cell);
                let pick = phases
                    .iter()
                    .min_by(|a, b| {
                        let da = diff(&a.crop(x, y, cell, cell), &t, 0).mean_abs;
                        let db = diff(&b.crop(x, y, cell, cell), &t, 0).mean_abs;
                        da.total_cmp(&db)
                    })
                    .unwrap();
                let c = pick.crop(x, y, cell, cell);
                for yy in 0..c.height {
                    for xx in 0..c.width {
                        best.put(x + xx, y + yy, c.get(xx, yy));
                    }
                }
            }
        }
        let d = diff(&best, &target, args.threshold);
        println!("level area, wobble-matched per cell: {d}");
        if let Some(out) = &args.screenshot {
            let p = out.with_extension("wobble-matched.diff.png");
            diff_image(&best, &target, args.threshold)
                .save_png(&p)
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn window_config(args: &Args) -> WindowConfig {
    WindowConfig {
        width: args.size.0,
        height: args.size.1,
        controller_db: [args.game_dir.join("gamecontrollerdb.txt")]
            .into_iter()
            .filter(|p| p.is_file())
            .collect(),
        ..Default::default()
    }
}

fn windowed(args: &Args) -> Result<(), String> {
    let (mut p, gl) = baba_platform::sdl::SdlPlatform::new(&window_config(args))?;
    log::info!("{}", p.describe());
    let mut r = Renderer::new(gl).map_err(|e| e.to_string())?;
    let (_level, scene, mut atlas) = prepare(args, r.atlas_page_size())?;
    r.sync_atlas(&mut atlas);
    if let Some(name) = &args.music {
        let bytes = std::fs::read(args.game_dir.join(format!("Data/Music/{name}.ogg")))
            .map_err(|e| format!("music {name}: {e}"))?;
        if let Some(a) = p.audio() {
            a.play_music(Arc::from(bytes), true, 0.8)?;
        }
    }
    let mut events = Vec::new();
    let mut frame = 0u64;
    let mut views = args.views;
    loop {
        events.clear();
        p.poll_events(&mut events);
        for e in &events {
            match e {
                Event::Quit
                | Event::Key {
                    key: Key::Escape,
                    down: true,
                    ..
                } => return Ok(()),
                Event::Key {
                    key: Key::Char(c @ b'1'..=b'9'),
                    down: true,
                    ..
                } => {
                    views = (c - b'0') as usize;
                }
                Event::ControllerAdded(_) | Event::ControllerRemoved(_) | Event::Button { .. } => {
                    log::info!("{e:?}")
                }
                _ => {}
            }
        }
        let t = p.time();
        // Scripted runs (--frames) hold the wobble so captures are stable.
        let wobble = if args.frames.is_some() {
            args.wobble
        } else {
            (t.as_millis() as u64 / WOBBLE_MS) as u32
        };
        let size = p.drawable_size();
        let opts = DrawOpts {
            views,
            ..DrawOpts::from(args)
        };
        draw(&mut r, &opts, &scene, &atlas, wobble, size, t.as_secs_f32());
        frame += 1;
        if args.frames.is_some_and(|n| frame >= n) {
            let img = r.read_screen();
            p.swap_buffers();
            return finish_screenshot(args, img, None);
        }
        p.swap_buffers();
    }
}

fn list_devices(args: &Args) -> Result<(), String> {
    for line in baba_platform::sdl::describe_system()? {
        println!("{line}");
    }
    match baba_platform::sdl::SdlPlatform::new(&WindowConfig {
        audio: false,
        ..window_config(args)
    }) {
        Ok((mut p, gl)) => {
            println!("platform: {}", p.describe());
            for _ in 0..args.virtual_pads {
                p.attach_virtual_controller()?;
            }
            let mut events = Vec::new();
            p.poll_events(&mut events);
            for e in &events {
                if let Event::ControllerAdded(d) = e {
                    println!("hot-plug: {:?} {}", d.id, d.name);
                }
            }
            let r = Renderer::new(gl).map_err(|e| e.to_string())?;
            print_gl(&r);
            for d in p.devices() {
                println!(
                    "input: {:?} {}{}{}",
                    d.id,
                    d.name,
                    if d.guid.is_empty() {
                        String::new()
                    } else {
                        format!(" [{}]", d.guid)
                    },
                    if d.mapped {
                        ""
                    } else {
                        " (no controller mapping)"
                    }
                );
            }
        }
        Err(e) => println!("window: {e}"),
    }
    match baba_render::headless::HeadlessContext::new(baba_render::headless::HeadlessApi::from_env())
    {
        Ok((ctx, gl)) => {
            println!("headless: EGL {} display", ctx.platform);
            print_gl(&Renderer::new(gl).map_err(|e| e.to_string())?);
        }
        Err(e) => println!("headless: {e}"),
    }
    Ok(())
}

fn print_gl(r: &Renderer) {
    let i = r.info();
    println!(
        "  GL {} | {} | {:?} {}.{} | max texture {} | atlas page {}",
        i.version,
        i.renderer,
        i.flavor,
        i.major,
        i.minor,
        i.max_texture_size,
        r.atlas_page_size()
    );
}

fn audio_test(args: &Args) -> Result<(), String> {
    let music_name = args.music.clone().unwrap_or_else(|| "baba".into());
    let music_path = args.game_dir.join(format!("Data/Music/{music_name}.ogg"));
    let music: Arc<[u8]> = std::fs::read(&music_path)
        .map_err(|e| format!("{}: {e}", music_path.display()))?
        .into();
    // Sound effects live in Assets.dat (Ogg Vorbis); take a few.
    let mut sounds: Vec<Arc<SoundData>> = Vec::new();
    match baba_formats::AssetsDat::load(&args.game_dir.join("Assets.dat")) {
        Ok(assets) => {
            for e in assets
                .of_kind(baba_formats::AssetKind::Sound)
                .take(40)
                .step_by(8)
            {
                if let Ok(snd) = assets.sound(e)
                    && let Ok(d) = audio::decode_ogg(std::io::Cursor::new(snd.data))
                {
                    sounds.push(Arc::new(d));
                }
            }
            println!("sounds: {} decoded from Assets.dat", sounds.len());
        }
        Err(e) => println!("Assets.dat: {e}"),
    }
    if sounds.is_empty() {
        sounds.push(Arc::new(SoundData::sine(
            660.0,
            0.3,
            audio::SAMPLE_RATE,
            0.3,
        )));
    }
    let run = |a: &mut Audio, wait: &mut dyn FnMut(f64)| -> Result<(), String> {
        println!("audio: {}", a.description());
        a.play_music(music.clone(), true, 0.7)?;
        let steps = (args.seconds * 4.0).ceil() as u32;
        for i in 0..steps {
            if i % 2 == 1 {
                let sound = &sounds[(i as usize / 2) % sounds.len()];
                let pan = if i % 4 == 1 { -0.5 } else { 0.5 };
                a.play(
                    sound,
                    PlayParams {
                        pan,
                        volume: 0.6,
                        ..Default::default()
                    },
                );
            }
            wait(0.25);
        }
        a.stop_all();
        wait(0.1);
        Ok(())
    };
    if let Some(out) = &args.offline {
        let mut a = Audio::offline(Some(out)).map_err(|e| e.to_string())?;
        let mut pending = 0.0;
        run(&mut a, &mut |s| pending += s)?;
        a.advance_offline(pending).map_err(|e| e.to_string())?;
        a.finish().map_err(|e| e.to_string())?;
        println!("wrote {}", out.display());
    } else {
        let mut dev = baba_platform::sdl::SdlAudio::open()?;
        run(&mut dev.audio, &mut |s| {
            std::thread::sleep(Duration::from_secs_f64(s))
        })?;
        if let Ok(path) = std::env::var("SDL_DISKAUDIOFILE") {
            drop(dev);
            let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
            let (peak, rms) = audio::raw_levels(&bytes, true);
            println!(
                "disk audio {path}: {} bytes ({:.2} s f32 stereo), peak {peak:.3}, RMS {rms:.4}",
                bytes.len(),
                bytes.len() as f64 / 8.0 / audio::SAMPLE_RATE as f64
            );
        }
    }
    Ok(())
}
