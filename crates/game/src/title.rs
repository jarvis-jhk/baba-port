//! The title screen's engine art (`ui.md` §2, measured on captures of the
//! original): a 792×432 background plate tiled over the screen, scrolling
//! a third of a pixel per frame toward the top left, darkened, with the
//! three wobble frames of the plate; the "BABA IS YOU" logo from large
//! letter images (three wobble frames each) and the dashed line under it.
//! All positions are in the 854×480 logical screen.

use baba_render::Quad;

use crate::ui::{Gfx, Ui};

/// Background plate frames.
const PLATE: [u32; 3] = [991, 993, 994];
const PLATE_W: f32 = 792.0;
const PLATE_H: f32 = 432.0;
/// Plate brightness (measured 0.68).
const PLATE_TINT: u8 = 174;
/// Frames per pixel of scroll.
const SCROLL_FRAMES: u64 = 3;

/// Logo letters: images (three wobble frames), image left, colour is
/// pink (4,1) or white (0,3).
const LOGO: [([u32; 3], f32, bool); 9] = [
    ([77, 114, 3087], 235.0, false),
    ([211, 582, 3089], 271.0, false),
    ([77, 114, 3087], 307.0, false),
    ([211, 582, 3089], 343.0, false),
    ([803, 902, 3088], 403.0, true),
    ([906, 908, 3090], 439.0, true),
    ([583, 592, 595], 511.0, false),
    ([596, 597, 605], 547.0, false),
    ([608, 609, 610], 583.0, false),
];
const LOGO_TOP: f32 = 68.0;
/// The dashes under the logo: 17 of them, 24 px apart.
const DASH: [u32; 3] = [1097, 1098, 1099];
const DASH_LEFT: f32 = 223.0;
const DASH_TOP: f32 = 184.0;

/// Draws the plate; false without engine art.
pub fn background(ui: &mut Ui, g: &mut Gfx, ticks: u64, wobble: u32) -> bool {
    let Some(a) = g.art.get(&mut g.atlas, PLATE[(wobble % 3) as usize]) else {
        return false;
    };
    let s = ui.scale;
    let off = (ticks / SCROLL_FRAMES) as f32;
    let ox = off % PLATE_W;
    let oy = off % PLATE_H;
    let c = [PLATE_TINT, PLATE_TINT, PLATE_TINT, 255];
    let mut y = -oy;
    while y < ui.height / s {
        let mut x = -ox;
        while x < ui.width / s {
            ui.quads.push(Quad {
                x: (x * s).round(),
                y: (y * s).round(),
                w: a.w as f32 * s,
                h: a.h as f32 * s,
                region: a.region,
                color: c,
            });
            x += PLATE_W;
        }
        y += PLATE_H;
    }
    true
}

/// Draws the logo and its dashes; false without engine art.
pub fn logo(ui: &mut Ui, g: &mut Gfx, wobble: u32) -> bool {
    let pink = g.pal(4, 1);
    let white = g.pal(0, 3);
    for (i, (frames, left, w)) in LOGO.iter().enumerate() {
        let f = frames[((wobble as usize) + i * 2) % 3];
        let Some(a) = g.art.get(&mut g.atlas, f) else {
            return false;
        };
        ui.sprite(
            a.region,
            *left,
            LOGO_TOP,
            a.w as f32,
            a.h as f32,
            if *w { white } else { pink },
        );
    }
    for i in 0..17 {
        let f = DASH[((wobble as usize) + i * 7 / 3) % 3];
        let Some(a) = g.art.get(&mut g.atlas, f) else {
            return false;
        };
        ui.sprite(
            a.region,
            DASH_LEFT + 24.0 * i as f32,
            DASH_TOP,
            a.w as f32,
            a.h as f32,
            white,
        );
    }
    true
}
