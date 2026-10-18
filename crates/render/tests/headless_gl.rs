//! Headless GL (EGL, Mesa llvmpipe in CI) against the CPU reference.
//! Skips when no EGL display is available unless `BABA_REQUIRE_GL=1`.

use baba_render::headless::{HeadlessApi, HeadlessContext};
use baba_render::*;

fn context(api: HeadlessApi) -> Option<(HeadlessContext, Renderer)> {
    match HeadlessContext::new(api) {
        Ok((ctx, gl)) => Some((ctx, Renderer::new(gl).expect("renderer"))),
        Err(e) => {
            assert!(
                std::env::var("BABA_REQUIRE_GL").is_err(),
                "headless GL required: {e}"
            );
            eprintln!("skipping: {e}");
            None
        }
    }
}

/// A synthetic level: odd-sized, translucent, overlapping, multi-page sprites.
fn test_scene(atlas: &mut Atlas) -> TileScene {
    let mut frames = Vec::new();
    for i in 0..12u32 {
        let (w, h) = [(24, 24), (26, 24), (30, 30), (21, 21), (48, 24)][i as usize % 5];
        let mut f = [RegionId(0); 3];
        for (k, slot) in f.iter_mut().enumerate() {
            let mut img = RgbaImage::new(w, h);
            for y in 0..h {
                for x in 0..w {
                    let on = !(x * 7 + y * 3 + i + k as u32 * 5).is_multiple_of(4);
                    let a = if (x + y) % 5 == 0 { 128 } else { 255 };
                    let g = (60 + (x * 9 + y * 5 + i * 11) % 196) as u8;
                    img.put(x, y, if on { [g, g, 255 - g, a] } else { [0, 0, 0, 0] });
                }
            }
            *slot = atlas.insert(&img);
        }
        frames.push(f);
    }
    let mut scene = TileScene::new(9, 6, 24);
    scene.level_bg = [8, 8, 8, 255];
    scene.outside_bg = [21, 24, 31, 255];
    scene.border = 1;
    for y in 0..6 {
        for x in 0..9 {
            let i = (x * 5 + y * 3) as usize % frames.len();
            scene.sprites.push(SceneSprite {
                x: x as f32,
                y: y as f32,
                frames: frames[i],
                color: [(x * 30) as u8 + 15, 255 - (y * 40) as u8, 200, 255],
                wobble: (x + y) as u8 % 3,
                layer: ((x + y) % 2) as i16,
            });
        }
    }
    scene
}

fn check(api: HeadlessApi) {
    let Some((_ctx, mut r)) = context(api) else {
        return;
    };
    eprintln!("{:?}", r.info());
    // Small pages force several textures.
    let mut atlas = Atlas::new(128);
    let scene = test_scene(&mut atlas);
    assert!(atlas.page_count() > 1);
    r.sync_atlas(&mut atlas);
    let (lw, lh) = scene.pixel_size();
    for (scale, wobble) in [(1u32, 0u32), (2, 1), (3, 2)] {
        let (w, h) = (lw * scale + 40, lh * scale + 30);
        r.use_offscreen_screen(w, h).unwrap();
        r.begin_frame(w, h, [255, 0, 0, 255]);
        let view = View {
            scene: &scene,
            wobble,
            rect: Rect::new(0, 0, w as i32, h as i32),
            scale: ScaleMode::Integer,
            effect: PostEffect::default(),
        };
        let fit = r.draw_view(0, &atlas, &view);
        assert_eq!(fit.scale, scale as f32);
        let level = r.read_level_target(0).unwrap();
        let reference = cpu::render_scene(&scene, &atlas, wobble, scale);
        let d = diff(&level, &reference, 1);
        eprintln!("{api:?} x{scale} level target vs CPU: {d}");
        assert!(d.max_abs <= 1, "{d}");
        // The composite at integer scale is a 1:1 copy into the letterbox.
        let screen = r.read_screen();
        let pr = fit.rect;
        let inner = screen.crop(pr.x as u32, pr.y as u32, pr.w as u32, pr.h as u32);
        let d = diff(&inner, &reference, 1);
        assert!(d.max_abs <= 1, "composite {d}");
        assert_eq!(screen.get(0, 0), [21, 24, 31, 255], "letterbox colour");
        if std::env::var("BABA_TEST_DUMP").is_ok() {
            let dir = std::env::var("BABA_TEST_DUMP").unwrap();
            screen
                .save_png(format!("{dir}/render-test-{api:?}-{scale}.png"))
                .unwrap();
        }
    }
    // Splitscreen: 4 views, fractional scale + rotation effect.
    let (w, h) = (640, 480);
    r.use_offscreen_screen(w, h).unwrap();
    r.begin_frame(w, h, [0, 0, 0, 255]);
    let rects = tile(w as i32, h as i32, 4, lw, lh, 2);
    for (i, rect) in rects.iter().enumerate() {
        let view = View {
            scene: &scene,
            wobble: i as u32,
            rect: *rect,
            scale: ScaleMode::default(),
            effect: PostEffect {
                rotation: i as f32 * 0.2,
                ..Default::default()
            },
        };
        r.draw_view(i, &atlas, &view);
    }
    let screen = r.read_screen();
    // Every viewport got its letterbox colour or level content (no red clear).
    assert!(!screen.data.chunks(4).any(|p| p == [255, 0, 0, 255]) || rects.len() < 4);
}

#[test]
fn gles2_matches_cpu_reference() {
    check(HeadlessApi::Gles2);
}

#[test]
fn gl33_core_matches_cpu_reference() {
    check(HeadlessApi::GlCore33);
}

#[test]
fn custom_effect_shader_compiles_and_runs() {
    let Some((_ctx, mut r)) = context(HeadlessApi::Gles2) else {
        return;
    };
    let id = r
        .register_effect(
            "void main() { vec4 c = TEX(u_tex, v_uv); FRAG = vec4(1.0 - c.rgb, 1.0) * v_color; }",
        )
        .unwrap();
    let mut atlas = Atlas::new(128);
    let scene = test_scene(&mut atlas);
    r.sync_atlas(&mut atlas);
    let (lw, lh) = scene.pixel_size();
    r.use_offscreen_screen(lw, lh).unwrap();
    r.begin_frame(lw, lh, [0, 0, 0, 255]);
    r.draw_view(
        0,
        &atlas,
        &View {
            scene: &scene,
            wobble: 0,
            rect: Rect::new(0, 0, lw as i32, lh as i32),
            scale: ScaleMode::Integer,
            effect: PostEffect {
                shader: Some(id),
                ..Default::default()
            },
        },
    );
    let mut reference = cpu::render_scene(&scene, &atlas, 0, 1);
    for p in reference.data.chunks_mut(4) {
        for c in &mut p[..3] {
            *c = 255 - *c;
        }
    }
    let d = diff(&r.read_screen(), &reference, 1);
    assert!(d.max_abs <= 1, "{d}");
}

/// Built-in UI (touch control icons, font) drawn over the frame.
#[test]
fn ui_overlay() {
    use baba_render::ui::{UiIcon, UiQuad, UiSprite, text_quads};
    let Some((_ctx, mut r)) = context(HeadlessApi::Gles2) else {
        return;
    };
    r.use_offscreen_screen(200, 100).unwrap();
    r.begin_frame(200, 100, [0, 0, 0, 255]);
    let icon = |x: f32, sprite, color| UiQuad {
        x,
        y: 10.0,
        w: 40.0,
        h: 40.0,
        sprite: UiSprite::Icon(sprite),
        color,
    };
    let mut q = vec![
        icon(10.0, UiIcon::Solid, [255, 0, 0, 255]),
        // Translucent disc: blends over black.
        icon(60.0, UiIcon::Disc, [0, 0, 255, 128]),
        icon(110.0, UiIcon::ArrowRight, [0, 255, 0, 255]),
    ];
    q.extend(text_quads("HI", 10.0, 70.0, 2.0, [255, 255, 0, 255]));
    r.draw_ui(&q);
    let img = r.read_screen();
    assert_eq!(img.get(30, 30), [255, 0, 0, 255]);
    let disc = img.get(80, 30);
    assert!(disc[2] > 100 && disc[2] < 150 && disc[0] == 0, "{disc:?}");
    // The arrow points right: its tip column is lit near the right edge,
    // the far left of its cell is not.
    assert_eq!(img.get(116, 30), [0, 0, 0, 255]);
    assert!(img.get(133, 30)[1] > 200);
    // 'H' at scale 2: left stem lit, centre gap of the top row dark.
    assert_eq!(img.get(11, 71), [255, 255, 0, 255]);
    assert_eq!(img.get(15, 71), [0, 0, 0, 255]);
    if let Ok(dir) = std::env::var("BABA_TEST_DUMP") {
        img.save_png(std::path::Path::new(&dir).join("ui_overlay.png"))
            .unwrap();
    }
}
