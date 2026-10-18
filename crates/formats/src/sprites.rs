//! Sprite files and their naming scheme.
//!
//! Object sprites are 24x24-ish PNGs named `<name>_<variant>_<frame>.png`:
//! `variant` selects direction / joined-neighbours / animation step (see
//! [`sprite_variant`]) and `frame` is the wobble frame 1..=3. Pixels are
//! white/grey on transparent and are tinted with the object's palette colour
//! (multiply). Map background images (`Images/`) are named
//! `<name>_<frame>.png`. Details: `docs/formats/palettes-sprites.md`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::level::Direction;
use crate::objects::Tiling;

/// Parsed parts of an object sprite file name.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SpriteFile {
    /// Base name (may itself contain `_`, e.g. `text_baba`).
    pub name: String,
    /// Variant number.
    pub variant: u8,
    /// Wobble frame (1..=3).
    pub frame: u8,
}

impl SpriteFile {
    /// Parses `<name>_<variant>_<frame>.png`; `None` for other names.
    pub fn parse(file_name: &str) -> Option<SpriteFile> {
        let stem = file_name.strip_suffix(".png")?;
        let (rest, frame) = stem.rsplit_once('_')?;
        let (name, variant) = rest.rsplit_once('_')?;
        if name.is_empty() {
            return None;
        }
        Some(SpriteFile {
            name: name.to_string(),
            variant: variant.parse().ok()?,
            frame: frame.parse().ok()?,
        })
    }

    /// The file name for these parts.
    pub fn file_name(name: &str, variant: u8, frame: u8) -> String {
        format!("{name}_{variant}_{frame}.png")
    }
}

/// The sprite variant number for an object.
///
/// - `dir`: the object's facing.
/// - `joined`: for [`Tiling::Tiled`], bit mask of neighbours it joins with:
///   right = 1, up = 2, left = 4, down = 8.
/// - `step`: animation / walk step 0..=3 (characters and animated objects).
///
/// Sleeping characters use `dir * 8 + 7`; that is up to the caller.
pub fn sprite_variant(tiling: Tiling, dir: Direction, joined: u8, step: u8) -> u8 {
    let d = dir as u8 * 8;
    let step = step & 3;
    match tiling {
        Tiling::None | Tiling::Other(_) => 0,
        Tiling::Directional => d,
        Tiling::Tiled => joined & 15,
        Tiling::Character | Tiling::AnimatedDirectional => d + step,
        Tiling::Animated => step,
    }
}

/// An index of one sprite directory.
#[derive(Clone, Debug, Default)]
pub struct SpriteDir {
    /// The directory.
    pub path: PathBuf,
    sprites: BTreeMap<String, BTreeMap<(u8, u8), PathBuf>>,
    /// Files that do not follow the `name_variant_frame.png` scheme.
    pub other_files: Vec<PathBuf>,
}

impl SpriteDir {
    /// Scans a directory. A missing directory yields an empty index.
    pub fn scan(dir: &Path) -> Result<SpriteDir> {
        let mut out = SpriteDir {
            path: dir.to_path_buf(),
            ..Default::default()
        };
        let rd = match std::fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(Error::io(dir, e)),
        };
        for entry in rd {
            let entry = entry.map_err(|e| Error::io(dir, e))?;
            let path = entry.path();
            let Some(fname) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            match SpriteFile::parse(fname) {
                Some(s) => {
                    out.sprites
                        .entry(s.name)
                        .or_default()
                        .insert((s.variant, s.frame), path);
                }
                None if fname.ends_with(".png") => out.other_files.push(path),
                None => {}
            }
        }
        out.other_files.sort();
        Ok(out)
    }

    /// Path of a sprite frame, if present.
    pub fn get(&self, name: &str, variant: u8, frame: u8) -> Option<&Path> {
        self.sprites
            .get(name)?
            .get(&(variant, frame))
            .map(PathBuf::as_path)
    }

    /// A file outside the naming scheme by stem (`icon_island_1` for
    /// `icon_island_1.png`; used by `[icons]` entries).
    pub fn plain(&self, stem: &str) -> Option<&Path> {
        self.other_files
            .iter()
            .find(|p| p.file_stem().is_some_and(|s| s == stem))
            .map(PathBuf::as_path)
    }

    /// True if any file exists for `name`.
    pub fn contains(&self, name: &str) -> bool {
        self.sprites.contains_key(name)
    }

    /// Variants present for `name` (sorted, deduplicated).
    pub fn variants(&self, name: &str) -> Vec<u8> {
        let mut v: Vec<u8> = self
            .sprites
            .get(name)
            .map(|m| m.keys().map(|k| k.0).collect())
            .unwrap_or_default();
        v.dedup();
        v
    }

    /// All sprite base names.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.sprites.keys().map(String::as_str)
    }

    /// Number of distinct sprite names.
    pub fn len(&self) -> usize {
        self.sprites.len()
    }

    /// True if no sprite was found.
    pub fn is_empty(&self) -> bool {
        self.sprites.is_empty()
    }

    /// Number of sprite files (all variants and frames).
    pub fn file_count(&self) -> usize {
        self.sprites.values().map(BTreeMap::len).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        let s = SpriteFile::parse("text_baba_0_1.png").unwrap();
        assert_eq!((s.name.as_str(), s.variant, s.frame), ("text_baba", 0, 1));
        assert_eq!(SpriteFile::parse("text_1_16_3.png").unwrap().name, "text_1");
        assert!(SpriteFile::parse("default_1.png").is_none());
        assert!(SpriteFile::parse("icon.png").is_none());
        assert_eq!(SpriteFile::file_name("wall", 5, 2), "wall_5_2.png");
    }

    #[test]
    fn variants() {
        use Direction::*;
        assert_eq!(sprite_variant(Tiling::None, Down, 15, 2), 0);
        assert_eq!(sprite_variant(Tiling::Directional, Up, 0, 0), 8);
        assert_eq!(sprite_variant(Tiling::Tiled, Left, 0b1010, 0), 10);
        assert_eq!(sprite_variant(Tiling::Character, Down, 0, 2), 26);
        assert_eq!(sprite_variant(Tiling::AnimatedDirectional, Left, 0, 3), 19);
        assert_eq!(sprite_variant(Tiling::Animated, Left, 0, 1), 1);
    }
}
