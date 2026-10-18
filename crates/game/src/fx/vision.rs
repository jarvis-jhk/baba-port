//! The 3D ("vision") view (`docs/spec/presentation.md` §8): a pseudo-3D
//! first-person view of the grid ahead of the camera unit, drawn as screen
//! quads. Units within 8 cells in front are projected with the scale
//! `s(z) = 4 + n(n+1)/4`, `n = 8 − z`; objects and text are billboards,
//! walls are faces at their distance, floors are squashed below the eye
//! line; distance darkens colours. The original's trapezoid/skew shader
//! is approximated with rectangles (the exact look is still to capture,
//! §8.8).

use baba_render::{Quad, RegionId, SceneSprite};
use baba_sim::Camera;

/// Unit scale at forward distance `z` (cells).
pub fn scale(z: f32) -> f32 {
    let n = 8.0 - z;
    4.0 + n * (n + 1.0) / 4.0
}

/// A drawable unit for the 3D view.
pub struct VisionUnit {
    pub x: i32,
    pub y: i32,
    pub layer: i16,
    pub sprite: SceneSprite,
    pub target: bool,
    pub edge: bool,
}

/// Builds the 3D view's quads for a `w`×`h` viewport at `(ox, oy)`;
/// `tile` is the UI tile in pixels, `wobble` picks the sprite frame,
/// `edge_colour` is palette (1,1) and `line` (1,2), `white` a solid region.
#[allow(clippy::too_many_arguments)]
pub fn quads(
    cam: Camera,
    units: &[VisionUnit],
    rect: (f32, f32, f32, f32),
    tile: f32,
    wobble: u32,
    line: [u8; 4],
    white: RegionId,
    region_size: &dyn Fn(RegionId) -> (u32, u32),
) -> Vec<Quad> {
    let (ox, oy, w, h) = rect;
    let (cx, cy) = (ox + w / 2.0, oy + h / 2.0);
    // Forward and right vectors in grid cells (y down).
    let (fx, fy) = match cam.dir {
        0 => (1, 0),
        1 => (0, -1),
        2 => (-1, 0),
        _ => (0, 1),
    };
    let (rx, ry) = (-fy, fx);
    let mut out = Vec::new();
    // Backdrop: horizontal lines at each depth (§8.7).
    for z in 0..=8 {
        let s = scale(z as f32);
        let y = cy + s * tile / 2.0;
        if y < oy + h {
            out.push(Quad {
                x: ox,
                y: y.round(),
                w,
                h: 1.0,
                region: white,
                color: line,
            });
        }
    }
    let mut items: Vec<(f32, Quad)> = Vec::new();
    for u in units {
        if u.target || (u.layer >= 21 && !u.edge) {
            continue;
        }
        let (dx, dy) = (u.x - cam.x, u.y - cam.y);
        let z = (dx * fx + dy * fy) as f32;
        let side = (dx * rx + dy * ry) as f32;
        if !(0.0..9.0).contains(&z) || side.abs() > 8.0 || (z == 0.0 && side == 0.0) {
            continue;
        }
        let object = u.layer >= 16;
        let zz = if object { z + 0.1 } else { z };
        let s = scale(zz);
        let size = s * tile;
        let x = cx + side * s * tile;
        if x + size / 2.0 < ox || x - size / 2.0 > ox + w {
            continue;
        }
        let dark = ((z - 1.0) * 7.0).clamp(0.0, 255.0) as u8;
        let c = u.sprite.color;
        let color = [
            c[0].saturating_sub(dark),
            c[1].saturating_sub(dark),
            c[2].saturating_sub(dark),
            c[3],
        ];
        let region = u.sprite.frames[((wobble + u.sprite.wobble as u32) % 3) as usize];
        let (rw, rh) = region_size(region);
        let k = size / 24.0;
        let (qw, qh) = (rw as f32 * k, rh as f32 * k);
        let q = if u.layer <= 10 && !u.edge {
            // Floor: squashed below the eye line.
            let fh = qh * ((8.0 - z) / 4.0).max(0.0) / 8.0;
            Quad {
                x: x - qw / 2.0,
                y: cy + size / 2.0,
                w: qw,
                h: fh.max(1.0),
                region,
                color,
            }
        } else {
            Quad {
                x: x - qw / 2.0,
                y: cy - qh / 2.0,
                w: qw,
                h: qh,
                region,
                color,
            }
        };
        let depth = zz + if object { 0.25 } else { 0.0 };
        items.push((depth, q));
    }
    items.sort_by(|a, b| b.0.total_cmp(&a.0));
    out.extend(items.into_iter().map(|(_, q)| q));
    out
}

#[cfg(test)]
mod tests {
    use super::scale;

    #[test]
    fn scale_matches_the_spec() {
        assert_eq!(scale(0.0), 22.0);
        assert_eq!(scale(8.0), 4.0);
    }
}
