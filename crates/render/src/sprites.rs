//! Loading the player's sprite and palette PNGs into the atlas.
//!
//! Sprite files are named `<sprite>_<frame>_<wobble>.png` with wobble 1..3
//! (see `docs/formats/`); the frame index encodes direction, animation step
//! or tiling. A world's own `Sprites/` folder overrides the shared one.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::atlas::{Atlas, RegionId};
use crate::image::RgbaImage;

/// A palette image: colours addressed as (x, y) as in the game's data.
#[derive(Clone, Debug)]
pub struct Palette {
    pub image: RgbaImage,
}

impl Palette {
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self, crate::image::ImageError> {
        Ok(Self {
            image: RgbaImage::load_png(path)?,
        })
    }

    /// The opaque colour at (x, y); out-of-range lookups give magenta so
    /// they stand out.
    pub fn get(&self, x: u32, y: u32) -> [u8; 4] {
        if x < self.image.width && y < self.image.height {
            let p = self.image.get(x, y);
            [p[0], p[1], p[2], 255]
        } else {
            [255, 0, 255, 255]
        }
    }
}

/// Loads sprite frames on demand from a search path and caches their atlas
/// regions.
pub struct SpriteBank {
    dirs: Vec<PathBuf>,
    cache: HashMap<(String, u32), Option<[RegionId; 3]>>,
    missing: Option<[RegionId; 3]>,
}

impl SpriteBank {
    /// `dirs`: sprite folders, highest priority first (e.g. the world's
    /// `Sprites/`, then `Data/Sprites/`).
    pub fn new(dirs: Vec<PathBuf>) -> Self {
        Self {
            dirs,
            cache: HashMap::new(),
            missing: None,
        }
    }

    fn find(&self, file: &str) -> Option<PathBuf> {
        self.dirs.iter().map(|d| d.join(file)).find(|p| p.is_file())
    }

    /// The three wobble frames of `name` at `frame`, loading them on first
    /// use. Missing wobble frames repeat the first; `None` when no file
    /// exists.
    pub fn get(&mut self, atlas: &mut Atlas, name: &str, frame: u32) -> Option<[RegionId; 3]> {
        let key = (name.to_string(), frame);
        if let Some(v) = self.cache.get(&key) {
            return *v;
        }
        let mut imgs: [Option<RgbaImage>; 3] = [None, None, None];
        for (w, slot) in imgs.iter_mut().enumerate() {
            if let Some(p) = self.find(&format!("{name}_{frame}_{}.png", w + 1)) {
                match RgbaImage::load_png(&p) {
                    Ok(img) => *slot = Some(img),
                    Err(e) => log::warn!("{}: {e}", p.display()),
                }
            }
        }
        let result = match imgs {
            [None, None, None] => None,
            _ => {
                let first = imgs.iter().flatten().next().cloned().unwrap();
                let ids = imgs.map(|i| atlas.insert(i.as_ref().unwrap_or(&first)));
                Some(ids)
            }
        };
        self.cache.insert(key, result);
        result
    }

    /// A placeholder (magenta-outlined box) for sprites that are missing.
    pub fn missing(&mut self, atlas: &mut Atlas) -> [RegionId; 3] {
        if let Some(m) = self.missing {
            return m;
        }
        let mut img = RgbaImage::new(24, 24);
        for i in 0..24 {
            for (x, y) in [(i, 0), (i, 23), (0, i), (23, i), (i, i), (23 - i, i)] {
                img.put(x, y, [255, 255, 255, 255]);
            }
        }
        let id = atlas.insert(&img);
        self.missing = Some([id; 3]);
        [id; 3]
    }
}
