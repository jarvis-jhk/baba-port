//! Worlds (level packs): `Data/Worlds/<id>/`.
//!
//! A world directory holds `world_data.txt` ([`WorldData`]), level pairs
//! `<stem>.l` + `<stem>.ld` (+ `<stem>.png` preview), and optional
//! `Sprites/`, `Palettes/`, `Themes/`, `Music/`, `Sounds/`, `Images/`,
//! `Lua/` (mods), `icon.png`, `credits.txt`. See `docs/formats/worlds.md`.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::ini::{Ini, parse_int};
use crate::level::LevelMap;
use crate::leveldata::LevelInfo;

/// `world_data.txt`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorldData {
    /// The raw INI.
    pub ini: Ini,
    /// Display name.
    pub name: String,
    /// Author.
    pub author: String,
    /// Level the world opens on (usually the top map), a file stem.
    pub start: String,
    /// First playable level for a fresh save, a file stem.
    pub first_level: String,
    /// Totals used for completion displays (`prize_max`, `clear_max`, `bonus_max`).
    pub prize_max: Option<i64>,
    /// See `prize_max`.
    pub clear_max: Option<i64>,
    /// See `prize_max`.
    pub bonus_max: Option<i64>,
    /// Number of editor themes (`themecount`).
    pub theme_count: Option<i64>,
    /// The world ships Lua mods (`mods=1`).
    pub mods: bool,
}

impl WorldData {
    /// Parses `world_data.txt`.
    pub fn parse(text: &str) -> WorldData {
        let ini = Ini::parse(text);
        let s = |k: &str| ini.get("general", k).unwrap_or("").to_string();
        let i = |k: &str| ini.get("general", k).and_then(parse_int);
        WorldData {
            name: s("name"),
            author: s("author"),
            start: s("start"),
            first_level: s("firstlevel"),
            prize_max: i("prize_max"),
            clear_max: i("clear_max"),
            bonus_max: i("bonus_max"),
            theme_count: i("themecount"),
            mods: i("mods").unwrap_or(0) != 0,
            ini,
        }
    }
}

/// A level: grid plus metadata.
#[derive(Clone, Debug)]
pub struct Level {
    /// File stem (`"106level"`).
    pub stem: String,
    /// The `.l` grid.
    pub map: LevelMap,
    /// The `.ld` metadata.
    pub info: LevelInfo,
}

/// A world directory.
#[derive(Clone, Debug)]
pub struct World {
    /// Directory name (`"baba"`, `"new_adv"`, ...).
    pub id: String,
    /// The directory.
    pub path: PathBuf,
    /// `world_data.txt` (default if missing).
    pub data: WorldData,
    /// Stems of all `.l` files, in natural order (`0level`, `1level`, `10level`, ...).
    pub levels: Vec<String>,
}

/// Natural ordering key: numeric prefix first, then the rest.
pub fn natural_key(stem: &str) -> (u64, String) {
    let digits: String = stem.chars().take_while(char::is_ascii_digit).collect();
    (
        digits.parse().unwrap_or(u64::MAX),
        stem[digits.len()..].to_string(),
    )
}

impl World {
    /// Opens a world directory.
    pub fn open(path: &Path) -> Result<World> {
        let rd = std::fs::read_dir(path).map_err(|e| Error::io(path, e))?;
        let mut levels = Vec::new();
        for e in rd {
            let e = e.map_err(|e| Error::io(path, e))?;
            let p = e.path();
            if p.extension().is_some_and(|x| x == "l")
                && let Some(stem) = p.file_stem().and_then(|s| s.to_str())
            {
                levels.push(stem.to_string());
            }
        }
        levels.sort_by_key(|s| natural_key(s));
        let wd = path.join("world_data.txt");
        let data = match crate::error::read_text(&wd) {
            Ok(t) => WorldData::parse(&t),
            Err(e) if e.is_not_found() => WorldData::default(),
            Err(e) => return Err(e),
        };
        Ok(World {
            id: path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string(),
            path: path.to_path_buf(),
            data,
            levels,
        })
    }

    /// Path of `<stem>.l`.
    pub fn level_path(&self, stem: &str) -> PathBuf {
        self.path.join(format!("{stem}.l"))
    }

    /// Path of `<stem>.ld`.
    pub fn info_path(&self, stem: &str) -> PathBuf {
        self.path.join(format!("{stem}.ld"))
    }

    /// Path of the `<stem>.png` preview thumbnail.
    pub fn preview_path(&self, stem: &str) -> PathBuf {
        self.path.join(format!("{stem}.png"))
    }

    /// Loads a level's grid and metadata.
    pub fn load_level(&self, stem: &str) -> Result<Level> {
        Ok(Level {
            stem: stem.to_string(),
            map: LevelMap::load(&self.level_path(stem))?,
            info: LevelInfo::load(&self.info_path(stem))?,
        })
    }

    /// The world's sprite directory (`Sprites/`).
    pub fn sprites_dir(&self) -> PathBuf {
        self.path.join("Sprites")
    }

    /// The world's palette directory (`Palettes/`).
    pub fn palettes_dir(&self) -> PathBuf {
        self.path.join("Palettes")
    }

    /// The world's background image directory (`Images/`).
    pub fn images_dir(&self) -> PathBuf {
        self.path.join("Images")
    }

    /// Theme files (`Themes/<n>theme.txt`) in numeric order.
    pub fn themes(&self) -> Vec<PathBuf> {
        list_files(&self.path.join("Themes"), "txt")
    }

    /// Music files (`Music/*.ogg`).
    pub fn music(&self) -> Vec<PathBuf> {
        list_files(&self.path.join("Music"), "ogg")
    }

    /// Sound files (`Sounds/*.ogg`).
    pub fn sounds(&self) -> Vec<PathBuf> {
        list_files(&self.path.join("Sounds"), "ogg")
    }

    /// Lua mod files (`Lua/*.lua`).
    pub fn lua_mods(&self) -> Vec<PathBuf> {
        list_files(&self.path.join("Lua"), "lua")
    }
}

/// Files with extension `ext` in `dir`, natural order; empty if `dir` is missing.
pub fn list_files(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == ext))
        .collect();
    v.sort_by_key(|p| natural_key(p.file_name().and_then(|n| n.to_str()).unwrap_or("")));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_data() {
        let w = WorldData::parse(
            "[general]\nname=Test\nauthor=keke\nstart=121level\nfirstlevel=128level\nthemecount=30\nmods=1\n[levels]\nlevelid=129\n",
        );
        assert_eq!(w.name, "Test");
        assert_eq!(w.start, "121level");
        assert_eq!(w.first_level, "128level");
        assert_eq!(w.theme_count, Some(30));
        assert!(w.mods);
        assert_eq!(w.prize_max, None);
    }

    #[test]
    fn natural() {
        let mut v = vec!["10level", "boop", "2level", "1level"];
        v.sort_by_key(|s| natural_key(s));
        assert_eq!(v, vec!["1level", "2level", "10level", "boop"]);
    }
}
