//! What the renderer draws: a level as a grid of coloured, wobbling sprites,
//! flattened into a draw list of [`Quad`]s (shared by the GL path and the
//! CPU reference rasterizer).

use crate::atlas::{Atlas, RegionId};

/// Number of wobble frames every object cycles through.
pub const WOBBLE_FRAMES: u32 = 3;

/// One textured, tinted, axis-aligned rectangle in target pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub region: RegionId,
    /// Multiplied with the texel (straight alpha).
    pub color: [u8; 4],
}

/// A sprite placed on the grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneSprite {
    /// Tile coordinates of the cell (fractional while sliding).
    pub x: f32,
    pub y: f32,
    /// The three wobble frames.
    pub frames: [RegionId; 3],
    pub color: [u8; 4],
    /// Added to the scene's wobble counter (objects need not wobble in
    /// lockstep).
    pub wobble: u8,
    /// Draw order: lower first; equal layers keep insertion order.
    pub layer: i16,
}

/// A level ready to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct TileScene {
    pub cols: u32,
    pub rows: u32,
    /// Source tile size in pixels (24 for the game's sprites).
    pub tile: u32,
    /// Colour inside the level rectangle (inside the border ring).
    pub level_bg: [u8; 4],
    /// Colour of the border ring and of the viewport around the level
    /// (letterbox).
    pub outside_bg: [u8; 4],
    /// Width of the border ring in cells that shows `outside_bg` (the
    /// game's levels have a one-cell edge ring; 0 = none).
    pub border: u32,
    pub sprites: Vec<SceneSprite>,
    /// Sprites drawn scaled about their centre (particles); merged with
    /// `sprites` by layer, after them on equal layers.
    pub scaled: Vec<(SceneSprite, f32)>,
}

impl TileScene {
    pub fn new(cols: u32, rows: u32, tile: u32) -> Self {
        Self {
            cols,
            rows,
            tile,
            level_bg: [0, 0, 0, 255],
            outside_bg: [0, 0, 0, 255],
            border: 0,
            sprites: Vec::new(),
            scaled: Vec::new(),
        }
    }

    /// The level's size in source pixels.
    pub fn pixel_size(&self) -> (u32, u32) {
        (self.cols * self.tile, self.rows * self.tile)
    }

    /// The area filled with `level_bg` at integer `scale`, in target pixels
    /// `(x, y, w, h)`.
    pub fn interior(&self, scale: u32) -> (u32, u32, u32, u32) {
        let b = self.border.min(self.cols / 2).min(self.rows / 2);
        let t = self.tile * scale;
        (
            b * t,
            b * t,
            (self.cols - 2 * b) * t,
            (self.rows - 2 * b) * t,
        )
    }

    /// Appends the scene's quads at integer `scale`, in draw order, for
    /// wobble counter `wobble` (frame = (wobble + sprite.wobble) mod 3).
    /// Sprites larger or smaller than a tile are centred on their cell
    /// (rounded down to whole source pixels).
    pub fn quads(&self, atlas: &Atlas, wobble: u32, scale: u32, out: &mut Vec<Quad>) {
        let n = self.sprites.len();
        let mut order: Vec<usize> = (0..n + self.scaled.len()).collect();
        let layer = |i: usize| {
            if i < n {
                self.sprites[i].layer
            } else {
                self.scaled[i - n].0.layer
            }
        };
        order.sort_by_key(|&i| layer(i));
        let t = self.tile as f32;
        let s = scale as f32;
        for i in order {
            let (sp, k) = if i < n {
                (&self.sprites[i], 1.0)
            } else {
                let (sp, k) = &self.scaled[i - n];
                (sp, *k)
            };
            let frame = (wobble + sp.wobble as u32) % WOBBLE_FRAMES;
            let region = sp.frames[frame as usize];
            let r = atlas.region(region);
            let dx = (self.tile as i32 - r.w as i32).div_euclid(2) as f32;
            let dy = (self.tile as i32 - r.h as i32).div_euclid(2) as f32;
            let (w, h) = (r.w as f32 * k, r.h as f32 * k);
            // Scaled about the unscaled sprite's centre.
            let (cx, cy) = (
                sp.x * t + dx + r.w as f32 / 2.0,
                sp.y * t + dy + r.h as f32 / 2.0,
            );
            out.push(Quad {
                x: (cx - w / 2.0) * s,
                y: (cy - h / 2.0) * s,
                w: w * s,
                h: h * s,
                region,
                color: sp.color,
            });
        }
    }
}
