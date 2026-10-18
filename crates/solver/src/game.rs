//! Loading levels from the player's install and reading the corpus files.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use baba_formats::GameInstall;
use baba_formats::objects::ObjectTable;
use baba_sim::{Context, Input, Level, Source};

/// The install plus its object table.
pub struct Game {
    pub install: GameInstall,
    pub objects: ObjectTable,
}

/// A playable level of a world.
#[derive(Clone, Debug)]
pub struct LevelRef {
    pub world: String,
    pub stem: String,
    pub name: String,
}

impl Game {
    pub fn open() -> Result<Game, String> {
        let install = match GameInstall::from_env() {
            Some(Ok(i)) => i,
            _ => return Err("$BABA_GAME_DIR not set or not an install".into()),
        };
        let objects = install.load_objects().map_err(|e| e.to_string())?;
        Ok(Game { install, objects })
    }

    /// Fresh level (turn 0).
    pub fn start(&self, world: &str, stem: &str) -> Result<(Level, String), String> {
        let w = self.install.world(world).map_err(|e| e.to_string())?;
        let data = w.load_level(stem).map_err(|e| e.to_string())?;
        let name = data.info.general.name.clone();
        let src = Arc::new(Source::from_level(&self.objects, &data, world));
        Ok((Level::start(src, Context::default()), name))
    }

    /// Levels of a world that some map shows (within the map's declared
    /// `levels=` count) and that are not maps themselves. With `stale`, also
    /// levels named only by `[levels]` records beyond that count (left-overs
    /// the game never loads; `corpus_verify.py` counts those too).
    pub fn playable(&self, world: &str, stale: bool) -> Result<Vec<LevelRef>, String> {
        let w = self.install.world(world).map_err(|e| e.to_string())?;
        let mut linked = BTreeSet::new();
        let mut infos = BTreeMap::new();
        for stem in &w.levels {
            let Ok(info) = baba_formats::leveldata::LevelInfo::load(&w.info_path(stem)) else {
                continue;
            };
            if info.is_map() || !info.levels.is_empty() {
                for l in &info.levels {
                    linked.insert(l.file.clone());
                }
                if stale {
                    for (k, v) in info.ini.entries("levels") {
                        let digits = k.trim_start_matches(|c: char| c.is_ascii_digit());
                        if digits == "file" && !v.is_empty() {
                            linked.insert(v.to_string());
                        }
                    }
                }
            }
            infos.insert(stem.clone(), info);
        }
        let mut out = Vec::new();
        for stem in &w.levels {
            let Some(info) = infos.get(stem) else {
                continue;
            };
            if !linked.contains(stem) || info.is_map() || !info.levels.is_empty() {
                continue;
            }
            out.push(LevelRef {
                world: world.to_string(),
                stem: stem.clone(),
                name: info.general.name.clone(),
            });
        }
        Ok(out)
    }
}

/// Project root (where `ref/` lives): `$BABA_ROOT` (set by env.sh) or cwd.
pub fn root() -> PathBuf {
    std::env::var_os("BABA_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Levels (world, stem) with an oracle-verified solution: `status` win or
/// win-seed in `verified.tsv`, plus every entry of `solved.tsv`.
pub fn solved_set(root: &Path) -> BTreeSet<(String, String)> {
    let mut s = BTreeSet::new();
    let dir = root.join("ref/tas/corpus");
    if let Ok(t) = std::fs::read_to_string(dir.join("verified.tsv")) {
        for l in t.lines().skip(1) {
            let c: Vec<&str> = l.split('\t').collect();
            if c.len() > 5 && (c[5] == "win" || c[5] == "win-seed") {
                s.insert((c[0].to_string(), c[1].to_string()));
            }
        }
    }
    if let Ok(t) = std::fs::read_to_string(dir.join("solved.tsv")) {
        for l in t.lines().skip(1) {
            let c: Vec<&str> = l.split('\t').collect();
            if c.len() > 5 {
                s.insert((c[0].to_string(), c[1].to_string()));
            }
        }
    }
    s
}

/// Parses an input string (corpus alphabet; other characters ignored).
pub fn parse_inputs(s: &str) -> Vec<Input> {
    s.chars().filter_map(Input::from_char).collect()
}

/// Corpus character of a search move (0..3 directions, 4 wait).
pub fn move_char(m: u8) -> char {
    ['R', 'U', 'L', 'D', 'W'][m as usize]
}
