//! Palettes: small PNGs whose pixels are the colours objects refer to.
//!
//! Colours everywhere in the data are given as palette coordinates
//! `(x, y)` ([`PalColour`]), never as RGB. A palette is normally 7x5 pixels.

use std::fmt;
use std::path::Path;

use crate::error::{Error, Result};
use crate::image::RgbaImage;

/// A colour reference: column `x` and row `y` in the current palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct PalColour {
    /// Column (0..7 in the shipped palettes).
    pub x: u8,
    /// Row (0..5 in the shipped palettes).
    pub y: u8,
}

impl PalColour {
    /// Builds a reference.
    pub const fn new(x: u8, y: u8) -> Self {
        PalColour { x, y }
    }

    /// Parses the `"x,y"` form used in `.ld` files (spaces allowed, a
    /// trailing comma tolerated).
    pub fn parse(s: &str) -> Option<PalColour> {
        let mut it = s.split(',').map(str::trim);
        let x = it.next()?.parse().ok()?;
        let y = it.next()?.parse().ok()?;
        if it.any(|r| !r.is_empty()) {
            return None;
        }
        Some(PalColour { x, y })
    }

    /// From a Lua pair `{x, y}`.
    pub fn from_pair(p: (i64, i64)) -> Option<PalColour> {
        Some(PalColour {
            x: u8::try_from(p.0).ok()?,
            y: u8::try_from(p.1).ok()?,
        })
    }
}

impl fmt::Display for PalColour {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{},{}", self.x, self.y)
    }
}

/// A decoded palette.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    /// Width in colours.
    pub width: u32,
    /// Height in colours.
    pub height: u32,
    /// Row-major RGB colours.
    pub colours: Vec<[u8; 3]>,
}

impl Palette {
    /// Converts a decoded palette image.
    pub fn from_image(img: &RgbaImage) -> Palette {
        let mut colours = Vec::with_capacity((img.width * img.height) as usize);
        for y in 0..img.height {
            for x in 0..img.width {
                let p = img.pixel(x, y);
                colours.push([p[0], p[1], p[2]]);
            }
        }
        Palette {
            width: img.width,
            height: img.height,
            colours,
        }
    }

    /// Decodes palette PNG bytes.
    pub fn decode_png(bytes: &[u8]) -> Result<Palette> {
        let img = RgbaImage::decode_png(bytes)?;
        if img.width == 0 || img.height == 0 {
            return Err(Error::malformed("palette", None, "empty image"));
        }
        Ok(Palette::from_image(&img))
    }

    /// Reads a palette PNG.
    pub fn load(path: &Path) -> Result<Palette> {
        Ok(Palette::from_image(&RgbaImage::load_png(path)?))
    }

    /// RGB of a colour reference; out-of-range coordinates are clamped to the
    /// palette edge (the data does not contain any, but mods might).
    pub fn rgb(&self, c: PalColour) -> [u8; 3] {
        let x = (c.x as u32).min(self.width - 1);
        let y = (c.y as u32).min(self.height - 1);
        self.colours[(y * self.width + x) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_colour() {
        assert_eq!(PalColour::parse("1,4"), Some(PalColour::new(1, 4)));
        assert_eq!(PalColour::parse(" 6 , 2,"), Some(PalColour::new(6, 2)));
        assert_eq!(PalColour::parse("1"), None);
        assert_eq!(PalColour::parse("1,2,3"), None);
        assert_eq!(PalColour::new(3, 2).to_string(), "3,2");
    }

    #[test]
    fn lookup() {
        let mut img = RgbaImage::new(7, 5);
        img.set_pixel(2, 3, [1, 2, 3, 255]);
        let p = Palette::from_image(&img);
        assert_eq!(p.rgb(PalColour::new(2, 3)), [1, 2, 3]);
        assert_eq!(p.rgb(PalColour::new(200, 200)), [0, 0, 0]);
    }
}
