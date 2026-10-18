//! CPU reference rasterizer for the draw list: nearest sampling, straight
//! alpha "over" blending, pixel centres at +0.5 (the same rules the GL path
//! follows). Tests render a scene both ways and compare with
//! [`crate::image::diff`].

use crate::atlas::Atlas;
use crate::image::RgbaImage;
use crate::scene::{Quad, TileScene};

/// Draws `quads` onto `target` (clipped).
pub fn rasterize(target: &mut RgbaImage, atlas: &Atlas, quads: &[Quad]) {
    for q in quads {
        let r = atlas.region(q.region);
        let page = atlas.page(r.page as usize);
        let x0 = (q.x - 0.5).ceil().max(0.0) as i64;
        let y0 = (q.y - 0.5).ceil().max(0.0) as i64;
        let x1 = ((q.x + q.w - 0.5).ceil() as i64).min(target.width as i64);
        let y1 = ((q.y + q.h - 0.5).ceil() as i64).min(target.height as i64);
        let c = q.color.map(|v| v as f32 / 255.0);
        for py in y0..y1 {
            let v = ((py as f32 + 0.5 - q.y) / q.h * r.h as f32).floor() as i64;
            let ty = (r.y as i64 + v.clamp(0, r.h as i64 - 1)) as u32;
            for px in x0..x1 {
                let u = ((px as f32 + 0.5 - q.x) / q.w * r.w as f32).floor() as i64;
                let tx = (r.x as i64 + u.clamp(0, r.w as i64 - 1)) as u32;
                let t = page.get(tx, ty);
                let s = [
                    t[0] as f32 / 255.0 * c[0],
                    t[1] as f32 / 255.0 * c[1],
                    t[2] as f32 / 255.0 * c[2],
                    t[3] as f32 / 255.0 * c[3],
                ];
                let d = target.get(px as u32, py as u32).map(|v| v as f32 / 255.0);
                let a = s[3];
                let out = [
                    s[0] * a + d[0] * (1.0 - a),
                    s[1] * a + d[1] * (1.0 - a),
                    s[2] * a + d[2] * (1.0 - a),
                    a + d[3] * (1.0 - a),
                ];
                target.put(
                    px as u32,
                    py as u32,
                    out.map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8),
                );
            }
        }
    }
}

/// Renders `scene` at integer `scale` into a new image (level area only).
pub fn render_scene(scene: &TileScene, atlas: &Atlas, wobble: u32, scale: u32) -> RgbaImage {
    let (w, h) = scene.pixel_size();
    let mut img = RgbaImage::filled(w * scale, h * scale, scene.outside_bg);
    let (ix, iy, iw, ih) = scene.interior(scale);
    for y in iy..iy + ih {
        for x in ix..ix + iw {
            img.put(x, y, scene.level_bg);
        }
    }
    let mut quads = Vec::new();
    scene.quads(atlas, wobble, scale, &mut quads);
    rasterize(&mut img, atlas, &quads);
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::SceneSprite;

    #[test]
    fn tint_blend_and_centering() {
        let mut atlas = Atlas::new(64);
        let mut img = RgbaImage::filled(2, 2, [255, 255, 255, 255]);
        img.put(1, 1, [0, 0, 0, 0]);
        let id = atlas.insert(&img);
        let mut scene = TileScene::new(2, 1, 4);
        scene.level_bg = [10, 20, 30, 255];
        scene.sprites.push(SceneSprite {
            x: 1.0,
            y: 0.0,
            frames: [id; 3],
            color: [255, 0, 0, 255],
            wobble: 0,
            layer: 0,
        });
        let out = render_scene(&scene, &atlas, 0, 2);
        assert_eq!((out.width, out.height), (16, 8));
        // Sprite 2x2 centred in a 4px cell at x=4..8, y=0..4 -> offset 1.
        assert_eq!(out.get(1, 1), [10, 20, 30, 255]);
        assert_eq!(out.get(10, 2), [255, 0, 0, 255]);
        assert_eq!(out.get(12, 4), [10, 20, 30, 255], "transparent texel");
    }
}
