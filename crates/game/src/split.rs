//! Splitscreen layout for 1–12 viewports (local multiplayer, modes 2 and
//! 3; online views later).
//!
//! * **Tiled**: the grid (columns × rows) that gives every view the
//!   largest scale for a typical level (33 × 18 tiles = 792 × 432 px),
//!   an incomplete last row centred (`baba_render::tile`). On 16:9 this is
//!   1×2, 2×2, 2×3 (5–6), 3×3 (7–9), 3×4 / 4×3 (10–12); 4:3 and 640×480
//!   handhelds get the same counts with taller cells.
//! * **Focus** (zoom toggle): one view large on top (three quarters of
//!   the height; on very wide screens the left three quarters), the
//!   others tiled in the remaining strip, so everyone stays visible.
//!
//! * **Big and small** (online): this machine's views tile the large
//!   part, the other machines' views the strip ([`layout_big`]).
//!
//! Each view then fits its own level into its rectangle (per-view scale:
//! integer where it fills the cell well, fractional below 1×,
//! `baba_render::fit`).

use baba_render::{Rect, tile};

/// Content size used to choose the grid (most levels and maps).
pub const CONTENT: (u32, u32) = (792, 432);
/// Pixels between views (a frame in the player's colour is drawn there).
pub const GAP: i32 = 4;

/// Viewport rectangles for `n` views on a `w` × `h` screen; `focus` is
/// the index of the enlarged view, if any.
pub fn layout(w: i32, h: i32, n: usize, focus: Option<usize>) -> Vec<Rect> {
    let gap = if w.min(h) < 400 { 2 } else { GAP };
    match (n, focus) {
        (0, _) => Vec::new(),
        (1, _) => vec![Rect::new(0, 0, w, h)],
        (_, Some(f)) if f < n => focused(w, h, n, f, gap),
        _ => tile(w, h, n, CONTENT.0, CONTENT.1, gap),
    }
}

/// Online splitscreen: the views in `big` (indices) tile the large part
/// of the screen, the others the strip (as with a focused view). All big
/// or none big: plain tiling.
pub fn layout_big(w: i32, h: i32, n: usize, big: &[usize]) -> Vec<Rect> {
    let k = (0..n).filter(|i| big.contains(i)).count();
    if k == 0 || k >= n {
        return layout(w, h, n, None);
    }
    let gap = if w.min(h) < 400 { 2 } else { GAP };
    let (main, strip) = regions(w, h, gap);
    let large = tile(main.w, main.h, k, CONTENT.0, CONTENT.1, gap);
    let small = tile(strip.w, strip.h, n - k, CONTENT.0, CONTENT.1, gap);
    let (mut li, mut si) = (large.into_iter(), small.into_iter());
    let v: Vec<Rect> = (0..n)
        .map(|i| {
            let (r, at) = if big.contains(&i) {
                (li.next(), main)
            } else {
                (si.next(), strip)
            };
            let r = r.unwrap_or(Rect::new(0, 0, 1, 1));
            Rect::new(r.x + at.x, r.y + at.y, r.w, r.h)
        })
        .collect();
    // Many big views and few small ones: the "small" ones would be the
    // larger; tile everything instead.
    let area = |r: &Rect| r.w * r.h;
    let min_big = (0..n)
        .filter(|i| big.contains(i))
        .map(|i| area(&v[i]))
        .min();
    let max_small = (0..n)
        .filter(|i| !big.contains(i))
        .map(|i| area(&v[i]))
        .max();
    if min_big < max_small {
        return layout(w, h, n, None);
    }
    v
}

/// The large part and the strip of a screen with one main area.
fn regions(w: i32, h: i32, gap: i32) -> (Rect, Rect) {
    let (cw, ch) = (CONTENT.0 as f32, CONTENT.1 as f32);
    let below_h = (h as f32 * 0.75) as i32;
    let side_w = (w as f32 * 0.75) as i32;
    let s_below = (w as f32 / cw).min(below_h as f32 / ch);
    let s_side = (side_w as f32 / cw).min(h as f32 / ch);
    if s_side > s_below * 1.05 {
        (
            Rect::new(0, 0, side_w, h),
            Rect::new(side_w + gap, 0, w - side_w - gap, h),
        )
    } else {
        (
            Rect::new(0, 0, w, below_h),
            Rect::new(0, below_h + gap, w, h - below_h - gap),
        )
    }
}

fn focused(w: i32, h: i32, n: usize, f: usize, gap: i32) -> Vec<Rect> {
    let (cw, ch) = (CONTENT.0 as f32, CONTENT.1 as f32);
    // Main view on top, strip below; or (very wide screens) main on the
    // left, strip on the right: whichever gives the main view more scale.
    let below_h = (h as f32 * 0.75) as i32;
    let side_w = (w as f32 * 0.75) as i32;
    let s_below = (w as f32 / cw).min(below_h as f32 / ch);
    let s_side = (side_w as f32 / cw).min(h as f32 / ch);
    let (main, strip) = if s_side > s_below * 1.05 {
        (
            Rect::new(0, 0, side_w, h),
            Rect::new(side_w + gap, 0, w - side_w - gap, h),
        )
    } else {
        (
            Rect::new(0, 0, w, below_h),
            Rect::new(0, below_h + gap, w, h - below_h - gap),
        )
    };
    let small = tile(strip.w, strip.h, n - 1, CONTENT.0, CONTENT.1, gap);
    let mut out = Vec::with_capacity(n);
    let mut it = small.into_iter();
    for i in 0..n {
        if i == f {
            out.push(main);
        } else if let Some(r) = it.next() {
            out.push(Rect::new(r.x + strip.x, r.y + strip.y, r.w, r.h));
        }
    }
    out
}

/// UI scale of a viewport (1 = the original's 854 × 480 window); unlike
/// the full-screen UI it may go well below 1 for small views.
pub fn ui_scale(r: Rect) -> f32 {
    let s = (r.w as f32 / 854.0).min(r.h as f32 / 480.0);
    if s >= 1.0 { s.floor() } else { s.max(0.05) }
}

/// Unrounded UI scale: the 854 × 480 box exactly fills the rectangle's
/// width or height (text anchored to a view's level area).
pub fn ui_scale_exact(r: Rect) -> f32 {
    (r.w as f32 / 854.0).min(r.h as f32 / 480.0).max(0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlaps(a: &Rect, b: &Rect) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    fn check(w: i32, h: i32, n: usize, focus: Option<usize>) -> Vec<Rect> {
        let v = layout(w, h, n, focus);
        assert_eq!(v.len(), n, "{w}x{h} n={n} focus={focus:?}");
        for (i, r) in v.iter().enumerate() {
            assert!(r.w > 0 && r.h > 0, "{w}x{h} n={n}: empty view {r:?}");
            assert!(
                r.x >= 0 && r.y >= 0 && r.x + r.w <= w && r.y + r.h <= h,
                "{w}x{h} n={n}: {r:?} off screen"
            );
            for s in &v[i + 1..] {
                assert!(!overlaps(r, s), "{w}x{h} n={n}: {r:?} overlaps {s:?}");
            }
        }
        v
    }

    #[test]
    fn every_count_fits_common_screens() {
        for (w, h) in [
            (1920, 1080),
            (1280, 720),
            (854, 480),
            (640, 480),
            (1024, 768),
        ] {
            for n in 1..=12 {
                let v = check(w, h, n, None);
                // Views are as large as each other (tiling).
                let a0 = v[0].w * v[0].h;
                assert!(v.iter().all(|r| r.w * r.h == a0));
                for f in [0, n - 1] {
                    let v = check(w, h, n, Some(f));
                    if n > 1 {
                        let big = v[f].w * v[f].h;
                        assert!(v.iter().all(|r| r.w * r.h <= big));
                    }
                }
            }
        }
    }

    #[test]
    fn grids() {
        // 16:9: four views as 2x2, twelve as 4x3 or 3x4.
        let v = layout(1920, 1080, 4, None);
        assert_eq!(v[0].w, v[1].w);
        assert_eq!(v[0].y, v[1].y);
        assert!(v[2].y > v[0].y);
        let v = layout(1920, 1080, 12, None);
        let cols = v.iter().filter(|r| r.y == v[0].y).count();
        assert!(cols == 3 || cols == 4, "{cols} columns");
        // 640x480: twelve views stay at least 140x80 pixels.
        let v = layout(640, 480, 12, None);
        assert!(v.iter().all(|r| r.w >= 140 && r.h >= 80), "{v:?}");
        // Focus: the focused view gets most of the screen.
        let v = layout(1920, 1080, 12, Some(5));
        assert!(v[5].w * v[5].h > 1920 * 1080 / 2);
    }

    #[test]
    fn big_and_small() {
        for (w, h) in [(1920, 1080), (854, 480), (640, 480)] {
            for n in 2..=12 {
                for k in 1..n {
                    // Every other view from the end, then the rest.
                    let order: Vec<usize> = (0..n)
                        .rev()
                        .step_by(2)
                        .chain((0..n).rev().skip(1).step_by(2))
                        .collect();
                    let big: Vec<usize> = order.into_iter().take(k).collect();
                    let v = layout_big(w, h, n, &big);
                    assert_eq!(v.len(), n);
                    for (i, r) in v.iter().enumerate() {
                        assert!(r.w > 0 && r.h > 0 && r.x + r.w <= w && r.y + r.h <= h);
                        for s in &v[i + 1..] {
                            assert!(!overlaps(r, s), "{w}x{h} n={n} k={k}");
                        }
                    }
                    let min_big = big.iter().map(|i| v[*i].w * v[*i].h).min().unwrap();
                    let max_small = (0..n)
                        .filter(|i| !big.contains(i))
                        .map(|i| v[i].w * v[i].h)
                        .max()
                        .unwrap();
                    assert!(min_big >= max_small, "{w}x{h} n={n} k={k}");
                }
            }
        }
    }

    #[test]
    fn small_views_scale_down() {
        assert_eq!(ui_scale(Rect::new(0, 0, 1920, 1080)), 2.0);
        assert!(ui_scale(Rect::new(0, 0, 160, 120)) < 0.25);
    }
}
