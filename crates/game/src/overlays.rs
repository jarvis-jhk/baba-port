//! Overlays shared by the full-screen view and every splitscreen view
//! (`docs/presentation.md`): the level title card in the large bitmap
//! font, the undo/restart hint, the blob iris and the 3D (vision) view.
//! Each takes the [`Ui`] (or renderer rectangle) of its view, so single
//! player and the multiplayer views draw the same thing at their own size.

use baba_progress::Action;
use baba_render::{Quad, Rect, RegionId, Renderer};

use crate::fx::transition::{BIG_SCALE, BLOB_IMAGE, Iris};
use crate::input::key_name;
use crate::lang::Texts;
use crate::play::Current;
use crate::ui::{Align, Gfx, Ui};

/// Frames without a player before the undo hint shows (measured).
pub const UNDO_HINT_FRAMES: u32 = 160;

/// HUD counter images (`presentation.md` §15.25, matched on captures):
/// spore, flower and orb icons (three wobble frames each) and the 8×16
/// digits 0–9.
const HUD_SPORE: [u32; 3] = [2719, 2721, 2723];
const HUD_FLOWER: [u32; 3] = [1165, 1168, 1221];
const HUD_ORB: [u32; 3] = [2722, 2728, 2729];
const HUD_DIGITS: [u32; 10] = [442, 517, 518, 519, 521, 522, 523, 524, 526, 527];

/// The map HUD counters at the top right (measured on the main map at
/// 854×480): icon centres at x 829/830, rows 32 px apart from y 20; the
/// count in the 8×16 digits right-aligned to x 809. Falls back to text
/// without engine art. `counts`: spores, flowers, orbs (orbs only when
/// any).
pub fn hud_counters(ui: &mut Ui, g: &mut Gfx, counts: [i64; 3], wob: u32) {
    let white = g.pal(0, 3);
    let rows: Vec<(i64, [u32; 3], &str)> = [
        (counts[0], HUD_SPORE, "\u{273D}"),
        (counts[1], HUD_FLOWER, "\u{273F}"),
        (counts[2], HUD_ORB, "\u{25CF}"),
    ]
    .into_iter()
    .enumerate()
    .filter(|(i, (n, _, _))| *i < 2 || *n > 0)
    .map(|(_, r)| r)
    .collect();
    let art = g.art.available();
    for (i, (n, icon, glyph)) in rows.into_iter().enumerate() {
        let cy = 20.0 + 32.0 * i as f32;
        let cx = if i == 1 { 830.0 } else { 829.0 };
        if !art {
            ui.text(
                g,
                None,
                &format!("{n} {glyph}"),
                846.0,
                cy - 13.0,
                16.0,
                white,
                Align::Right,
            );
            continue;
        }
        if let Some(a) = g.art.get(&mut g.atlas, icon[(wob % 3) as usize]) {
            ui.sprite(
                a.region,
                cx - a.hx as f32,
                cy - a.hy as f32,
                a.w as f32,
                a.h as f32,
                white,
            );
        }
        let digits: Vec<u32> = n
            .max(0)
            .to_string()
            .bytes()
            .map(|b| (b - b'0') as u32)
            .collect();
        let mut x = 801.0 - 10.0 * (digits.len() as f32 - 1.0);
        for d in digits {
            if let Some(a) = g.art.get(&mut g.atlas, HUD_DIGITS[d as usize]) {
                ui.sprite(a.region, x, cy - 9.0, a.w as f32, a.h as f32, white);
            }
            x += 10.0;
        }
    }
}

/// The win / map-clear banners as the original's art (pixel-font
/// languages, `ui.md` §7): the words written on from the centre (2 frames
/// per image), then three wobble frames; a copy 4 px lower in (1,2) and the
/// growing dark backing behind, centred at (427, 236) (measured on the
/// `0level` win capture). False when the art is missing.
pub fn art_banner(ui: &mut Ui, g: &mut Gfx, clear: bool, age: u32, wob: u32) -> bool {
    const CONGRATS: [u32; 11] = [607, 611, 614, 613, 616, 617, 618, 620, 621, 622, 623];
    const CONGRATS_W: [u32; 3] = [624, 625, 626];
    const CLEAR: [u32; 10] = [1476, 1475, 1478, 1479, 1482, 1483, 1484, 1481, 1486, 1467];
    const CLEAR_W: [u32; 3] = [1485, 1395, 1474];
    const BACKING: [u32; 13] = [
        1612, 1615, 1617, 1619, 1618, 1620, 1621, 1622, 1623, 1624, 1625, 1626, 1627,
    ];
    if !g.art.available() || !g.fonts.pixel {
        return false;
    }
    let (seq, full): (&[u32], &[u32; 3]) = if clear {
        (&CLEAR, &CLEAR_W)
    } else {
        (&CONGRATS, &CONGRATS_W)
    };
    let k = (age / 2) as usize;
    let img = if k < seq.len() {
        seq[k]
    } else {
        full[(wob % 3) as usize]
    };
    let b = BACKING[k.min(BACKING.len() - 1)];
    let (cx, cy) = (427.0, 236.0);
    // The backing and the lower copy in the menu colour (1,0) (measured on
    // the `0level` win capture: (21,24,31) on the default palette).
    let dark = g.pal(1, 0);
    if let Some(a) = g.art.silhouette(&mut g.atlas, b) {
        ui.sprite(
            a.region,
            cx - a.hx as f32,
            cy + 8.0 - a.hy as f32,
            a.w as f32,
            a.h as f32,
            dark,
        );
    }
    let Some(a) = g.art.get(&mut g.atlas, img) else {
        return false;
    };
    for (dy, c) in [(4.0, dark), (0.0, g.pal(0, 3))] {
        ui.sprite(
            a.region,
            cx - a.hx as f32,
            cy + dy - a.hy as f32,
            a.w as f32,
            a.h as f32,
            c,
        );
    }
    true
}

/// The lines of the sign texts to show now (`progression.md` §3.6): a
/// `sign,<a>=<b>=<c>` (literal) or `sign_lang=<k1>=<k2>=<k3>` (language
/// keys) special whose tile holds or touches (orthogonally) a YOU / YOU2
/// unit.
pub fn sign_lines(
    texts: &Texts,
    info: &baba_formats::LevelInfo,
    level: &baba_sim::Level,
) -> Vec<String> {
    let signs: Vec<_> = info
        .specials
        .iter()
        .filter(|s| s.data.starts_with("sign,") || s.data.starts_with("sign_lang="))
        .collect();
    if signs.is_empty() {
        return Vec::new();
    }
    let units = level.units();
    let mut players = level.effect_sources("you").units;
    players.extend(level.effect_sources("you2").units);
    let cells: Vec<(i32, i32)> = units
        .iter()
        .filter(|u| players.contains(&u.id))
        .map(|u| (u.x, u.y))
        .collect();
    let mut out = Vec::new();
    for s in signs {
        let near = cells
            .iter()
            .any(|&(x, y)| (x - s.x).abs() + (y - s.y).abs() <= 1);
        if !near {
            continue;
        }
        let (lang, rest) = match s.data.split_once(['=', ',']) {
            Some(("sign_lang", r)) => (true, r),
            Some((_, r)) => (false, r),
            None => continue,
        };
        for part in rest.split('=').filter(|p| !p.is_empty()) {
            out.push(if lang {
                texts.get(part)
            } else {
                part.to_string()
            });
        }
    }
    out
}

/// Draws sign lines at the bottom of the level view on a dark band
/// (placement our own; the original's is not captured yet).
pub fn signs(ui: &mut Ui, g: &mut Gfx, texts: &Texts, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    let h = 20.0 * lines.len() as f32 + 12.0;
    let y0 = 452.0 - h;
    let mut band = g.pal(1, 0);
    band[3] = 220;
    ui.rect(g, 60.0, y0, 734.0, h, band);
    for (i, l) in lines.iter().enumerate() {
        ui.text(
            g,
            Some(texts),
            l,
            427.0,
            y0 + 6.0 + 20.0 * i as f32,
            16.0,
            g.pal(0, 3),
            Align::Center,
        );
    }
}

/// The level title card (`presentation.md` §15.10, measured): background
/// (1,0); the label and the subtitle in the large bitmap font at half size,
/// one letter every 24 px; the name at full size. The TrueType font stands
/// in where the bitmap font has no image.
pub fn title_card(
    ui: &mut Ui,
    bg: Option<&mut Ui>,
    g: &mut Gfx,
    label: Option<&str>,
    name: &str,
    subtitle: &str,
) {
    let white = g.pal(0, 3);
    let fill = g.pal(1, 0);
    match bg {
        Some(b) => b.fill(g, fill),
        None => ui.fill(g, fill),
    }
    if let Some(l) = label
        && !ui.art_line(g, l, 427.0, 179.0, 0.5, 24.0, g.pal(4, 2))
    {
        let spaced: String = l
            .chars()
            .flat_map(|c| [c, ' '])
            .collect::<String>()
            .trim_end()
            .to_string();
        ui.text(
            g,
            None,
            &spaced,
            427.0,
            178.0,
            18.0,
            g.pal(4, 2),
            Align::Center,
        );
    }
    if !ui.art_line(g, name, 427.0, 216.0, 1.0, 24.0, white) {
        ui.text(g, None, name, 427.0, 205.0, 40.0, white, Align::Center);
    }
    if !subtitle.is_empty() && !ui.art_line(g, subtitle, 427.0, 277.0, 0.5, 24.0, g.pal(1, 3)) {
        ui.text(
            g,
            None,
            subtitle,
            427.0,
            262.0,
            16.0,
            g.pal(1, 3),
            Align::Center,
        );
    }
}

/// The undo/restart hint at the top of the screen (`ui.md` §6.3, measured
/// on captures): the bound keys' caps at 6 UI tiles left and 4.5 right of
/// the centre, 0.8 tiles from the top, each with its label 1.6 tiles to
/// the right (English: the `button_undo`/`button_restart` sprites).
pub fn undo_hint(ui: &mut Ui, g: &mut Gfx, texts: &Texts, s: &baba_progress::Settings, wob: u32) {
    let white = g.pal(0, 3);
    let y = 24.0 * 0.8;
    for (action, x, sprite, word) in [
        (Action::Undo, 427.0 - 24.0 * 6.0, "button_undo", "undo"),
        (
            Action::Restart,
            427.0 + 24.0 * 4.5,
            "button_restart",
            "restart",
        ),
    ] {
        let key = key_name(s.key(action, false));
        if let Some(a) = crate::keycaps::image(&key).and_then(|n| g.art.get(&mut g.atlas, n)) {
            ui.sprite(
                a.region,
                x - a.hx as f32,
                y - a.hy as f32,
                a.w as f32,
                a.h as f32,
                white,
            );
        } else {
            ui.text(
                g,
                None,
                &key.to_uppercase(),
                x,
                y - 8.0,
                16.0,
                white,
                Align::Center,
            );
        }
        let label = if texts.current.code == "en" {
            g.sprites
                .get(&mut g.atlas, sprite, 0)
                .map(|f| f[(wob % 3) as usize])
        } else {
            None
        };
        match label {
            Some(r) => {
                let (w, h) = g.region_size(r);
                let lx = x + 24.0 * 1.6;
                ui.sprite(
                    r,
                    lx - w as f32 / 2.0,
                    y - h as f32 / 2.0,
                    w as f32,
                    h as f32,
                    white,
                );
            }
            None => {
                ui.text(
                    g,
                    Some(texts),
                    &texts.get(word),
                    x + 24.0 * 0.6,
                    y - 4.0,
                    16.0,
                    white,
                    Align::Left,
                );
            }
        }
    }
}

/// The blob iris of a transition (`presentation.md` §11.1, §15.9) in a
/// view's [`Ui`]; the blobs were laid out for that view's pixel size.
pub fn iris(ui: &mut Ui, g: &mut Gfx, iris: &Iris, cover: bool) {
    let col = g.pal(1, 0);
    if iris.covered() {
        if cover {
            ui.fill(g, col);
        }
        return;
    }
    let blob = g.art.get(&mut g.atlas, BLOB_IMAGE);
    for b in &iris.blobs {
        let k = ui.scale * if b.big { BIG_SCALE } else { 1.0 };
        match blob {
            Some(a) => ui.quads.push(Quad {
                x: b.x - a.hx as f32 * k,
                y: b.y - a.hy as f32 * k,
                w: a.w as f32 * k,
                h: a.h as f32 * k,
                region: a.region,
                color: col,
            }),
            None => ui.disc_px(g, b.x, b.y, 64.0 * k, col),
        }
    }
}

/// The 3D first-person view (`presentation.md` §8) over a level view
/// placed in `area` (screen pixels) at `view_scale`: a 22×18-cell window
/// centred on the area (measured), drawn with the renderer's clipping.
pub fn vision(
    r: &mut Renderer,
    g: &Gfx,
    cur: &Current,
    area: Rect,
    view_scale: f32,
    level_bg: [u8; 4],
    wob: u32,
) {
    let Some(cam) = cur.level.camera() else {
        return;
    };
    let (cw, ch) = (528.0 * view_scale, 432.0 * view_scale);
    let clip = Rect::new(
        (area.x as f32 + (area.w as f32 - cw) / 2.0).round() as i32,
        (area.y as f32 + (area.h as f32 - ch) / 2.0).round() as i32,
        cw.round() as i32,
        ch.round() as i32,
    );
    let atlas = &g.atlas;
    let size = |rg: RegionId| {
        let x = atlas.region(rg);
        (x.w as u32, x.h as u32)
    };
    let mut quads = vec![Quad {
        x: area.x as f32,
        y: area.y as f32,
        w: area.w as f32,
        h: area.h as f32,
        region: g.white(),
        color: level_bg,
    }];
    quads.extend(crate::fx::vision::quads(
        cam,
        &cur.gfx.vision_units,
        (clip.x as f32, clip.y as f32, clip.w as f32, clip.h as f32),
        24.0 * view_scale,
        wob,
        g.pal(1, 2),
        g.white(),
        &size,
    ));
    r.draw_overlay_clipped(&g.atlas, &quads, clip);
}
