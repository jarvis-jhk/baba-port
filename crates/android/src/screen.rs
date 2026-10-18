//! What the Android entry point draws itself: the screens before the
//! game's `App` can start (import progress, "no game data" help; built-in
//! font, no game assets) and the on-screen touch controls over the game's
//! frames.

use baba_platform::touch::{TouchControl, TouchItem};
use baba_platform::{Event, Platform};
use baba_render::Renderer;
use baba_render::ui::{UiIcon, UiQuad, UiSprite, text_quads, wrap};

/// Background disc + icon for each visible touch control.
pub fn touch_quads(items: &[TouchItem]) -> Vec<UiQuad> {
    let mut out = Vec::with_capacity(items.len() * 2);
    for it in items {
        let icon = match it.control {
            TouchControl::Up => UiIcon::ArrowUp,
            TouchControl::Down => UiIcon::ArrowDown,
            TouchControl::Left => UiIcon::ArrowLeft,
            TouchControl::Right => UiIcon::ArrowRight,
            TouchControl::Wait => UiIcon::Wait,
            TouchControl::Undo => UiIcon::Undo,
            TouchControl::Restart => UiIcon::Restart,
            TouchControl::Menu => UiIcon::Menu,
        };
        let bg = if it.pressed {
            [255, 255, 255, 110]
        } else {
            [20, 20, 30, 110]
        };
        let fg = if it.pressed {
            [255, 255, 255, 255]
        } else {
            [255, 255, 255, 190]
        };
        for (sprite, color) in [(UiIcon::Disc, bg), (icon, fg)] {
            out.push(UiQuad {
                x: it.x,
                y: it.y,
                w: it.w,
                h: it.h,
                sprite: UiSprite::Icon(sprite),
                color,
            });
        }
    }
    out
}

/// Text shown when no game data is found (built-in font, no game assets).
pub const NO_DATA: &str = "baba-port needs your own copy of Baba Is You.\n\n\
On your computer, zip the game's folder (the one with Data and Assets.dat), \
copy the zip to this device and open it with baba-port (Files app: tap the zip, \
or share it to baba-port).\n\n\
Or copy the folder to Android/data/org.babaport/files/game (USB or adb).\n\n\
Back closes baba-port.";

fn message_quads(text: &str, w: u32, h: u32) -> Vec<UiQuad> {
    let scale = ((w.min(h) as f32 / 260.0).floor()).max(1.0);
    let cols = ((w as f32 * 0.85) / (6.0 * scale)) as usize;
    let lines = wrap(text, cols.max(10));
    let line_h = 11.0 * scale;
    let top = (h as f32 - lines.len() as f32 * line_h) / 2.0;
    let left = w as f32 * 0.075;
    let mut q = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        q.extend(text_quads(
            l,
            left,
            top + i as f32 * line_h,
            scale,
            [255; 4],
        ));
    }
    q
}

/// Draws one frame with a centred message (built-in font) and presents it.
pub fn show_message(p: &mut dyn Platform, r: &mut Renderer, text: &str) {
    let (w, h) = p.drawable_size();
    r.begin_frame(w, h, [12, 12, 20, 255]);
    r.draw_ui(&message_quads(text, w, h));
    p.swap_buffers();
}

/// Shows `text` until the platform quits or Back is pressed. The renderer
/// is re-created after a lost GL context.
pub fn message_loop(p: &mut dyn Platform, r: &mut Renderer, text: &str) {
    let mut events = Vec::new();
    loop {
        events.clear();
        p.poll_events(&mut events);
        for e in &events {
            match e {
                Event::Quit
                | Event::Key {
                    key: baba_platform::Key::Back | baba_platform::Key::Escape,
                    down: true,
                    ..
                } => return,
                Event::Resume { gl_lost: true } => {
                    if let Some(gl) = p.take_restored_gl()
                        && let Ok(n) = Renderer::new(gl)
                    {
                        *r = n;
                    }
                }
                _ => {}
            }
        }
        show_message(p, r, text);
    }
}
