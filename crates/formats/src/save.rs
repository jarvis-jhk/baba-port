//! Saves and settings of the original game (read only).
//!
//! On Linux they live in `~/.local/share/Baba_Is_You/` (`$XDG_DATA_HOME`):
//! - `0ba.ba`, `1ba.ba`, `2ba.ba`: the three save slots ([`SaveSlot`]);
//! - `ba.ba`: which slot was used last ([`SaveIndex`]);
//! - `SettingsC.txt`: options and key/controller bindings ([`Settings`]).
//!
//! All are INI files ([`crate::ini`]). Saves may contain the same section
//! twice; reads merge them. Field meanings: `docs/formats/saves.md`.

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::ini::{Ini, parse_int};

/// The original game's data directory on this machine, if it can be derived
/// from the environment (`$XDG_DATA_HOME` or `$HOME/.local/share`, plus
/// `Baba_Is_You`). Existence is not checked.
pub fn default_save_dir() -> Option<PathBuf> {
    if let Some(x) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(x).join("Baba_Is_You"));
    }
    let home = std::env::var_os("HOME").filter(|v| !v.is_empty())?;
    Some(PathBuf::from(home).join(".local/share/Baba_Is_You"))
}

/// A save slot file (`<n>ba.ba`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SaveSlot {
    /// The raw INI.
    pub ini: Ini,
}

/// Progress recorded for one level in a world section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LevelProgress {
    /// Level file stem (`"5level"`).
    pub level: String,
    /// Stored state number (see `docs/formats/saves.md`).
    pub state: i64,
}

impl SaveSlot {
    /// Reads a slot file.
    pub fn load(path: &Path) -> Result<SaveSlot> {
        Ok(SaveSlot {
            ini: Ini::load(path)?,
        })
    }

    /// Parses slot text.
    pub fn parse(text: &str) -> SaveSlot {
        SaveSlot {
            ini: Ini::parse(text),
        }
    }

    /// World ids with progress (sections without a `_suffix`).
    pub fn worlds(&self) -> Vec<&str> {
        self.ini
            .section_names()
            .into_iter()
            .filter(|n| {
                !["_prize", "_clears", "_complete", "_bonus", "_converts"]
                    .iter()
                    .any(|s| n.ends_with(s))
            })
            .collect()
    }

    /// Per-level states of a world: keys of the form `<level>=<n>` (map cell
    /// keys `<map>,<x>,<y>` and `_intro` flags excluded).
    pub fn levels(&self, world: &str) -> Vec<LevelProgress> {
        self.ini
            .entries(world)
            .into_iter()
            .filter(|(k, _)| is_level_key(k))
            .filter_map(|(k, v)| {
                Some(LevelProgress {
                    level: k.to_string(),
                    state: parse_int(v)?,
                })
            })
            .collect()
    }

    /// Map cell states `(map, x, y, value)` from keys `<map>,<x>,<y>=<n>`.
    pub fn map_cells(&self, world: &str) -> Vec<(String, i32, i32, i64)> {
        self.ini
            .entries(world)
            .into_iter()
            .filter_map(|(k, v)| {
                let mut it = k.split(',');
                let map = it.next()?;
                let x = parse_int(it.next()?)? as i32;
                let y = parse_int(it.next()?)? as i32;
                if it.next().is_some() {
                    return None;
                }
                Some((map.to_string(), x, y, parse_int(v)?))
            })
            .collect()
    }

    /// `total` of `[<world>_<kind>]` (`kind` = `prize`, `clears`, `bonus`, ...).
    pub fn total(&self, world: &str, kind: &str) -> Option<i64> {
        self.ini.get_int(&format!("{world}_{kind}"), "total")
    }

    /// A key in a world section.
    pub fn get(&self, world: &str, key: &str) -> Option<&str> {
        self.ini.get(world, key)
    }
}

/// Level keys are level file stems: `5level`, `x20level` (no `,`, no `_intro`).
fn is_level_key(k: &str) -> bool {
    !k.contains(',')
        && !k.ends_with("_intro")
        && (k.ends_with("level") || k.starts_with(|c: char| c.is_ascii_digit()))
}

/// `ba.ba`: global save index.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SaveIndex {
    /// The raw INI.
    pub ini: Ini,
    /// Last used slot (`[baba] lastslot` / `[savegame] slot`).
    pub last_slot: Option<i64>,
    /// World of the last session (`[savegame] world`).
    pub world: Option<String>,
}

impl SaveIndex {
    /// Parses `ba.ba` text.
    pub fn parse(text: &str) -> SaveIndex {
        let ini = Ini::parse(text);
        SaveIndex {
            last_slot: ini
                .get_int("savegame", "slot")
                .or_else(|| ini.get_int("baba", "lastslot")),
            world: ini.get("savegame", "world").map(str::to_string),
            ini,
        }
    }

    /// Reads `ba.ba`.
    pub fn load(path: &Path) -> Result<SaveIndex> {
        Ok(SaveIndex::parse(&crate::error::read_text(path)?))
    }
}

/// `SettingsC.txt`: options and bindings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    /// The raw INI (`[settings]`, `[keyboard]`, `[gamepad]`, `[gamepad_editor]`,
    /// `[gamepad_currobjlist]`, `[slotnames]`, `[editor]`, `[log]`).
    pub ini: Ini,
}

impl Settings {
    /// Parses settings text.
    pub fn parse(text: &str) -> Settings {
        Settings {
            ini: Ini::parse(text),
        }
    }

    /// Reads the file.
    pub fn load(path: &Path) -> Result<Settings> {
        Ok(Settings {
            ini: Ini::load(path)?,
        })
    }

    /// `[settings] lang` (`en`, `de`, ...).
    pub fn language(&self) -> Option<&str> {
        self.ini.get("settings", "lang")
    }

    /// Music volume 0..=100.
    pub fn music_volume(&self) -> Option<i64> {
        self.ini.get_int("settings", "music")
    }

    /// Sound volume 0..=100.
    pub fn sound_volume(&self) -> Option<i64> {
        self.ini.get_int("settings", "sound")
    }

    /// Keyboard binding for an action (`up`, `undo2`, ...): an SDL keycode.
    pub fn key(&self, action: &str) -> Option<i64> {
        self.ini.get_int("keyboard", action)
    }

    /// Slot display name (`[slotnames] <n>`).
    pub fn slot_name(&self, slot: u32) -> Option<&str> {
        self.ini.get("slotnames", &slot.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot() {
        let s = SaveSlot::parse(
            "[w_prize]\ntotal=8\n5level=8\n\n[w]\n5level=3\n5level_intro=1\nmap,16,11=3\nx20level=2\nPrevious=5level\nleveltree=map,5level\n[w]\n6level=2\n[w_clears]\ntotal=1\n",
        );
        assert_eq!(s.worlds(), vec!["w"]);
        let lv = s.levels("w");
        let names: Vec<_> = lv.iter().map(|l| l.level.as_str()).collect();
        assert_eq!(names, vec!["5level", "x20level", "6level"]);
        assert_eq!(s.map_cells("w"), vec![("map".to_string(), 16, 11, 3)]);
        assert_eq!(s.total("w", "prize"), Some(8));
        assert_eq!(s.total("w", "clears"), Some(1));
    }

    #[test]
    fn index_and_settings() {
        let i =
            SaveIndex::parse("[baba]\nfirsttime=1\nlastslot=2\n[savegame]\nslot=2\nworld=baba\n");
        assert_eq!(i.last_slot, Some(2));
        assert_eq!(i.world.as_deref(), Some("baba"));
        let st = Settings::parse("[settings]\nlang=en\nmusic=71\n[keyboard]\nup=1073741906\n");
        assert_eq!(st.language(), Some("en"));
        assert_eq!(st.music_volume(), Some(71));
        assert_eq!(st.key("up"), Some(1073741906));
    }
}
