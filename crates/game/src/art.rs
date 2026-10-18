//! Engine art from the player's `Assets.dat` (`docs/formats/assets-dat.md`):
//! images addressed by their number among the image entries, uploaded to
//! the shared atlas on first use. Particle frames, map icon symbols, the
//! pixel font, transition blobs and other UI pieces of the original are
//! drawn from here; without an `Assets.dat` every lookup is `None` and
//! callers fall back to their own shapes.

use std::collections::HashMap;
use std::sync::Arc;

use baba_formats::{AssetKind, AssetsDat};
use baba_render::{Atlas, RegionId, RgbaImage};

/// One engine image in the atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArtImage {
    pub region: RegionId,
    pub w: u32,
    pub h: u32,
    /// Hot spot (drawing origin) in pixels from the top-left corner.
    pub hx: i32,
    pub hy: i32,
}

/// Lazily loaded engine images.
#[derive(Default)]
pub struct Art {
    dat: Option<Arc<AssetsDat>>,
    /// Entry index of every image, by image number.
    images: Vec<usize>,
    cache: HashMap<u32, Option<ArtImage>>,
}

impl Art {
    pub fn new(dat: Option<Arc<AssetsDat>>) -> Art {
        let images = dat
            .as_ref()
            .map(|d| {
                d.entries
                    .iter()
                    .filter(|e| e.kind == AssetKind::Image)
                    .map(|e| e.index)
                    .collect()
            })
            .unwrap_or_default();
        Art {
            dat,
            images,
            cache: HashMap::new(),
        }
    }

    /// Engine art is available.
    pub fn available(&self) -> bool {
        !self.images.is_empty()
    }

    /// Image `n`, uploading it on first use.
    pub fn get(&mut self, atlas: &mut Atlas, n: u32) -> Option<ArtImage> {
        if let Some(r) = self.cache.get(&n) {
            return *r;
        }
        let r = self.pixels(n).and_then(|(img, hx, hy)| {
            if img.width == 0
                || img.height == 0
                || img.width + 2 > atlas.page_size()
                || img.height + 2 > atlas.page_size()
            {
                return None;
            }
            Some(ArtImage {
                region: atlas.insert(&img),
                w: img.width,
                h: img.height,
                hx,
                hy,
            })
        });
        self.cache.insert(n, r);
        r
    }

    /// Image `n` as a white silhouette (its alpha only), so it can be drawn
    /// in any colour (the banners' dark backings). Cached under a separate
    /// key.
    pub fn silhouette(&mut self, atlas: &mut Atlas, n: u32) -> Option<ArtImage> {
        let key = n | 0x8000_0000;
        if let Some(r) = self.cache.get(&key) {
            return *r;
        }
        let r = self.pixels(n).and_then(|(mut img, hx, hy)| {
            if img.width == 0
                || img.width + 2 > atlas.page_size()
                || img.height + 2 > atlas.page_size()
            {
                return None;
            }
            for p in img.data.chunks_mut(4) {
                p[0] = 255;
                p[1] = 255;
                p[2] = 255;
            }
            Some(ArtImage {
                region: atlas.insert(&img),
                w: img.width,
                h: img.height,
                hx,
                hy,
            })
        });
        self.cache.insert(key, r);
        r
    }

    /// Decoded pixels and hot spot of image `n` (not cached).
    pub fn pixels(&self, n: u32) -> Option<(RgbaImage, i32, i32)> {
        let dat = self.dat.as_ref()?;
        let &idx = self.images.get(n as usize)?;
        let entry = dat.entries.get(idx)?;
        let a = dat.image(entry).ok()?;
        let img = RgbaImage {
            width: a.image.width,
            height: a.image.height,
            data: a.image.pixels,
        };
        Some((img, a.info.hotspot.0 as i32, a.info.hotspot.1 as i32))
    }
}
