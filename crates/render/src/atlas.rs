//! Run-time sprite atlas: images are shelf-packed into fixed-size pages.
//! The CPU copy of every page is kept so pages can be re-uploaded after a
//! GL context loss (Android) and used by the CPU reference rasterizer.

use crate::image::RgbaImage;

/// Gap between packed images (transparent), so filtered sampling never
/// bleeds into a neighbour.
const PAD: u32 = 1;

/// Handle of a packed image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RegionId(pub u32);

/// Where a packed image lives: page index and pixel rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub page: u16,
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

struct Shelf {
    y: u32,
    h: u32,
    x: u32,
}

struct Page {
    image: RgbaImage,
    shelves: Vec<Shelf>,
    next_y: u32,
    /// Rows `[from, to)` changed since the last upload.
    dirty: Option<(u32, u32)>,
}

impl Page {
    fn new(size: u32) -> Self {
        Self {
            image: RgbaImage::new(size, size),
            shelves: Vec::new(),
            next_y: 0,
            dirty: None,
        }
    }

    fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let size = self.image.width;
        let (pw, ph) = (w + PAD, h + PAD);
        // Best-fitting existing shelf (least wasted height).
        let mut best: Option<usize> = None;
        for (i, s) in self.shelves.iter().enumerate() {
            if s.h >= ph
                && s.x + pw <= size
                && s.h <= ph + ph / 2
                && best.is_none_or(|b| self.shelves[b].h > s.h)
            {
                best = Some(i);
            }
        }
        let i = match best {
            Some(i) => i,
            None => {
                if self.next_y + ph > size || pw > size {
                    return None;
                }
                self.shelves.push(Shelf {
                    y: self.next_y,
                    h: ph,
                    x: 0,
                });
                self.next_y += ph;
                self.shelves.len() - 1
            }
        };
        let s = &mut self.shelves[i];
        let pos = (s.x, s.y);
        s.x += pw;
        Some(pos)
    }
}

/// A growing set of atlas pages.
pub struct Atlas {
    page_size: u32,
    pages: Vec<Page>,
    regions: Vec<Region>,
}

impl Atlas {
    /// `page_size`: width and height of each page (a GL texture), e.g.
    /// `Renderer::atlas_page_size()`.
    pub fn new(page_size: u32) -> Self {
        Self {
            page_size,
            pages: Vec::new(),
            regions: Vec::new(),
        }
    }

    pub fn page_size(&self) -> u32 {
        self.page_size
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn page(&self, i: usize) -> &RgbaImage {
        &self.pages[i].image
    }

    pub fn len(&self) -> usize {
        self.regions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    pub fn region(&self, id: RegionId) -> Region {
        self.regions[id.0 as usize]
    }

    /// Packs `img` and returns its handle. Panics if the image is larger
    /// than a page.
    pub fn insert(&mut self, img: &RgbaImage) -> RegionId {
        assert!(
            img.width + PAD <= self.page_size && img.height + PAD <= self.page_size,
            "image {}x{} larger than atlas page {}",
            img.width,
            img.height,
            self.page_size
        );
        let mut placed = None;
        for (pi, page) in self.pages.iter_mut().enumerate() {
            if let Some(pos) = page.alloc(img.width, img.height) {
                placed = Some((pi, pos));
                break;
            }
        }
        let (pi, (x, y)) = match placed {
            Some(p) => p,
            None => {
                let mut page = Page::new(self.page_size);
                let pos = page
                    .alloc(img.width, img.height)
                    .expect("fits an empty page");
                self.pages.push(page);
                (self.pages.len() - 1, pos)
            }
        };
        let page = &mut self.pages[pi];
        for row in 0..img.height {
            let s = (row * img.width * 4) as usize;
            let d = (((y + row) * self.page_size + x) * 4) as usize;
            page.image.data[d..d + img.width as usize * 4]
                .copy_from_slice(&img.data[s..s + img.width as usize * 4]);
        }
        page.dirty = Some(match page.dirty {
            Some((a, b)) => (a.min(y), b.max(y + img.height)),
            None => (y, y + img.height),
        });
        self.regions.push(Region {
            page: pi as u16,
            x: x as u16,
            y: y as u16,
            w: img.width as u16,
            h: img.height as u16,
        });
        RegionId(self.regions.len() as u32 - 1)
    }

    /// Returns and clears the changed row ranges per page.
    pub fn take_dirty(&mut self) -> Vec<(usize, u32, u32)> {
        self.pages
            .iter_mut()
            .enumerate()
            .filter_map(|(i, p)| p.dirty.take().map(|(a, b)| (i, a, b)))
            .collect()
    }

    /// Marks every page dirty (after a GL context loss).
    pub fn mark_all_dirty(&mut self) {
        let size = self.page_size;
        for p in &mut self.pages {
            p.dirty = Some((0, size));
        }
    }

    /// The pixels of a packed image (CPU copy).
    pub fn pixels(&self, id: RegionId) -> RgbaImage {
        let r = self.region(id);
        self.pages[r.page as usize]
            .image
            .crop(r.x as u32, r.y as u32, r.w as u32, r.h as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_without_overlap_and_spills_to_new_pages() {
        let mut atlas = Atlas::new(64);
        let mut ids = Vec::new();
        for i in 0..20u8 {
            let img = RgbaImage::filled(24, if i % 3 == 0 { 30 } else { 24 }, [i, 0, 0, 255]);
            ids.push(atlas.insert(&img));
        }
        assert!(atlas.page_count() > 1);
        for (i, id) in ids.iter().enumerate() {
            let px = atlas.pixels(*id);
            assert!(px.data.chunks(4).all(|p| p == [i as u8, 0, 0, 255]));
        }
        let rs: Vec<Region> = ids.iter().map(|&i| atlas.region(i)).collect();
        for (i, a) in rs.iter().enumerate() {
            for b in &rs[i + 1..] {
                let overlap = a.page == b.page
                    && a.x < b.x + b.w
                    && b.x < a.x + a.w
                    && a.y < b.y + b.h
                    && b.y < a.y + a.h;
                assert!(!overlap, "{a:?} {b:?}");
            }
        }
        assert_eq!(atlas.take_dirty().len(), atlas.page_count());
        assert!(atlas.take_dirty().is_empty());
    }
}
