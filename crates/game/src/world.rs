//! Worlds (level packs) and the per-level metadata game flow needs
//! (`docs/spec/progression.md` §1, §2, §9).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use baba_formats::{LevelInfo, WorldData};

/// Progression-relevant `.ld [general]` keys of one level.
#[derive(Clone, Debug, Default)]
pub struct Meta {
    pub name: String,
    pub subtitle: String,
    pub is_map: bool,
    pub mapid: Option<String>,
    pub music: String,
    pub local_music: bool,
    pub unlock_count: i32,
    pub unlock_levels: Vec<String>,
    pub custom_parent: String,
    pub custom_return: String,
}

/// An opened world folder.
pub struct WorldInfo {
    pub id: String,
    pub dir: PathBuf,
    pub data: WorldData,
    /// Lives outside the install's `Data/Worlds` (the port's data dir:
    /// user packs, the `levels` world, downloads; see `packs`).
    pub external: bool,
    meta: RefCell<HashMap<String, Option<Meta>>>,
}

impl WorldInfo {
    pub fn open(data_dir: &Path, id: &str) -> Option<WorldInfo> {
        let mut w = Self::open_dir(&data_dir.join("Worlds").join(id), id)?;
        w.external = false;
        Some(w)
    }

    /// A world folder anywhere, saved under `id` (`packs::open_world`).
    pub fn open_dir(dir: &Path, id: &str) -> Option<WorldInfo> {
        let w = baba_formats::World::open(dir).ok()?;
        Some(WorldInfo {
            id: id.to_string(),
            dir: w.path,
            data: w.data,
            external: true,
            meta: RefCell::new(HashMap::new()),
        })
    }

    /// The world argument for resource lookups that join
    /// `Data/Worlds/<world>` (sprites, palettes, images, music): the id for
    /// install worlds, the absolute folder for external ones (joining an
    /// absolute path replaces the base).
    pub fn res_key(&self) -> String {
        if self.external {
            self.dir.to_string_lossy().into_owned()
        } else {
            self.id.clone()
        }
    }

    pub fn is_base(&self) -> bool {
        self.id == "baba"
    }

    pub fn exists(&self, stem: &str) -> bool {
        self.dir.join(format!("{stem}.ld")).is_file()
            && self.dir.join(format!("{stem}.l")).is_file()
    }

    /// The level's metadata (cached).
    pub fn meta(&self, stem: &str) -> Option<Meta> {
        if let Some(m) = self.meta.borrow().get(stem) {
            return m.clone();
        }
        let m = LevelInfo::load(&self.dir.join(format!("{stem}.ld")))
            .ok()
            .map(|info| {
                let g = &info.general;
                Meta {
                    name: g.name.clone(),
                    subtitle: g.subtitle.clone(),
                    is_map: g.level_type == 1,
                    mapid: g.map_id.clone(),
                    music: g.music.clone(),
                    local_music: info.ini.get("general", "localmusic").unwrap_or("0").trim() == "1",
                    unlock_count: g.unlock_count,
                    unlock_levels: g.unlock_levels.clone(),
                    custom_parent: g.custom_parent.clone(),
                    custom_return: info
                        .ini
                        .get("general", "customreturn")
                        .unwrap_or("")
                        .to_string(),
                }
            });
        self.meta.borrow_mut().insert(stem.to_string(), m.clone());
        m
    }

    pub fn load(&self, stem: &str) -> Result<baba_formats::Level, String> {
        let w = baba_formats::World {
            id: self.id.clone(),
            path: self.dir.clone(),
            data: self.data.clone(),
            levels: Vec::new(),
        };
        w.load_level(stem).map_err(|e| e.to_string())
    }

    /// `credits.txt` (or another credits file) of this world, if any.
    pub fn credits(&self, file: &str) -> Option<String> {
        std::fs::read(self.dir.join(file))
            .ok()
            .map(|b| String::from_utf8_lossy(&b).into_owned())
    }
}

/// Packs offered by "Start the game" (`ui.md` §3.3): the base world
/// first, then every other folder except the hidden ones and `levels`.
pub fn list_packs(data_dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(data_dir.join("Worlds"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir() && e.path().join("world_data.txt").is_file())
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| {
            !matches!(
                n.as_str(),
                "levels" | "baba" | "baba_m" | "debug" | "new_adv_m"
            )
        })
        .collect();
    v.sort();
    let mut out = Vec::new();
    if data_dir.join("Worlds/baba/world_data.txt").is_file() {
        out.push("baba".to_string());
    }
    out.extend(v);
    out
}
