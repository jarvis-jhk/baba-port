//! Locating things inside the player's install.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::objects::{EditorObjectList, ObjectTable};
use crate::palette::Palette;
use crate::sprites::SpriteDir;
use crate::world::{World, list_files};

/// The player's copy of the game (the directory with `Data/` and
/// `Assets.dat`): the install folder on Linux and Windows,
/// `Baba Is You.app/Contents/Resources` on macOS.
///
/// Everything is read on demand; nothing is cached here.
#[derive(Clone, Debug)]
pub struct GameInstall {
    /// Install root.
    pub root: PathBuf,
}

impl GameInstall {
    /// Environment variable naming the install root.
    pub const ENV: &'static str = "BABA_GAME_DIR";

    /// Opens an install root, its `Data/` directory, `Data/values.lua`,
    /// a macOS `.app` bundle or a folder holding one (see
    /// [`GameInstall::locate`]).
    pub fn open(path: &Path) -> Result<GameInstall> {
        match Self::locate(path) {
            Some(root) => Ok(GameInstall { root }),
            None => Err(Error::missing(format!(
                "Baba Is You install (no Data/values.lua under {})",
                path.display()
            ))),
        }
    }

    /// The folder holding `Data/values.lua` for a path a player may point
    /// at: that folder itself (Linux, Windows), its `Data/` or
    /// `Data/values.lua`, a macOS app bundle (`Contents/Resources`), its
    /// `Contents/`, or a folder containing the bundle
    /// (`Baba Is You.app` first, then any `*.app`) or the game's folder
    /// (one level down, e.g. `gamedata/Baba Is You/Data`).
    pub fn locate(path: &Path) -> Option<PathBuf> {
        let has = |d: &Path| d.join("Data").join("values.lua").is_file();
        if path.is_file() {
            let d = path.parent()?.parent()?;
            return (path.file_name()? == "values.lua" && has(d)).then(|| d.to_path_buf());
        }
        if has(path) {
            return Some(path.to_path_buf());
        }
        if path.file_name().is_some_and(|n| n == "Data")
            && let Some(parent) = path.parent()
            && has(parent)
        {
            return Some(parent.to_path_buf());
        }
        let mut bundles = vec![
            path.join("Contents/Resources"),
            path.join("Resources"),
            path.join("Baba Is You.app/Contents/Resources"),
        ];
        let mut children: Vec<PathBuf> = std::fs::read_dir(path)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.is_dir())
                    .collect()
            })
            .unwrap_or_default();
        children.sort();
        let is_app = |p: &PathBuf| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("app"));
        bundles.extend(
            children
                .iter()
                .filter(|p| is_app(p))
                .map(|a| a.join("Contents/Resources")),
        );
        bundles.extend(children.iter().filter(|p| !is_app(p)).cloned());
        bundles.into_iter().find(|d| has(d))
    }

    /// Opens the install named by `$BABA_GAME_DIR`; `None` if the variable is
    /// unset or the directory does not exist.
    pub fn from_env() -> Option<Result<GameInstall>> {
        let p = std::env::var_os(Self::ENV).filter(|v| !v.is_empty())?;
        let p = PathBuf::from(p);
        p.is_dir().then(|| GameInstall::open(&p))
    }

    /// `Data/`.
    pub fn data_dir(&self) -> PathBuf {
        self.root.join("Data")
    }

    /// `Data/Worlds/`.
    pub fn worlds_dir(&self) -> PathBuf {
        self.data_dir().join("Worlds")
    }

    /// `Assets.dat` (every build: next to `Data/`).
    pub fn assets_dat(&self) -> PathBuf {
        self.root.join("Assets.dat")
    }

    /// The game's executables that exist, most likely first: Linux
    /// `bin64/Chowdren`, Windows `Baba Is You.exe` (then other `*.exe` next
    /// to `Data/`), macOS `../MacOS/Chowdren` (from `Contents/Resources`),
    /// and the places an imported or hand-copied install keeps them
    /// (`Chowdren`, `MacOS/Chowdren`). Only read, for the sound effects'
    /// names ([`crate::exe::sound_names`]).
    pub fn executables(&self) -> Vec<PathBuf> {
        executables_in(&self.root)
    }

    /// `Data/values.lua` (object slots, colours, particles, ...).
    pub fn values_lua(&self) -> PathBuf {
        self.data_dir().join("values.lua")
    }

    /// `Data/Editor/editor_objectlist.lua`.
    pub fn editor_objectlist_lua(&self) -> PathBuf {
        self.data_dir().join("Editor").join("editor_objectlist.lua")
    }

    /// World ids (directory names under `Data/Worlds`), sorted.
    pub fn world_ids(&self) -> Result<Vec<String>> {
        let dir = self.worlds_dir();
        let rd = std::fs::read_dir(&dir).map_err(|e| Error::io(&dir, e))?;
        let mut v: Vec<String> = rd
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .collect();
        v.sort();
        Ok(v)
    }

    /// Opens a world.
    pub fn world(&self, id: &str) -> Result<World> {
        World::open(&self.worlds_dir().join(id))
    }

    /// The base object slots from `values.lua`.
    pub fn load_objects(&self) -> Result<ObjectTable> {
        ObjectTable::load(&self.values_lua())
    }

    /// The editor catalogue from `editor_objectlist.lua`.
    pub fn load_editor_objects(&self) -> Result<EditorObjectList> {
        EditorObjectList::load(&self.editor_objectlist_lua())
    }

    /// `Data/Sprites/`.
    pub fn sprites_dir(&self) -> PathBuf {
        self.data_dir().join("Sprites")
    }

    /// Global palettes (`Data/Palettes/*.png`).
    pub fn palettes(&self) -> Vec<PathBuf> {
        list_files(&self.data_dir().join("Palettes"), "png")
    }

    /// Resolves a palette file: `root = true` looks in `Data/Palettes`,
    /// otherwise in the world's `Palettes/`; falls back to the other place.
    pub fn palette_path(&self, world: Option<&World>, name: &str, root: bool) -> Option<PathBuf> {
        let global = self.data_dir().join("Palettes").join(name);
        let local = world.map(|w| w.palettes_dir().join(name));
        let order = if root {
            [Some(global), local]
        } else {
            [local, Some(global)]
        };
        order.into_iter().flatten().find(|p| p.is_file())
    }

    /// Loads a palette via [`GameInstall::palette_path`] (`default.png` if
    /// the name is empty).
    pub fn load_palette(&self, world: Option<&World>, name: &str, root: bool) -> Result<Palette> {
        let name = if name.is_empty() { "default.png" } else { name };
        let p = self
            .palette_path(world, name, root)
            .ok_or_else(|| Error::missing(format!("palette {name}")))?;
        Palette::load(&p)
    }

    /// Global music (`Data/Music/*.ogg`).
    pub fn music(&self) -> Vec<PathBuf> {
        list_files(&self.data_dir().join("Music"), "ogg")
    }

    /// Global loose sounds (`Data/Sounds/*.ogg`; the game's own sound effects
    /// are inside `Assets.dat`).
    pub fn sounds(&self) -> Vec<PathBuf> {
        list_files(&self.data_dir().join("Sounds"), "ogg")
    }

    /// Language files (`Data/Languages/lang_*.txt`).
    pub fn languages(&self) -> Vec<PathBuf> {
        list_files(&self.data_dir().join("Languages"), "txt")
    }

    /// Global themes (`Data/Themes/<n>theme.txt`).
    pub fn themes(&self) -> Vec<PathBuf> {
        list_files(&self.data_dir().join("Themes"), "txt")
    }
}

/// [`GameInstall::executables`] for an install root.
pub fn executables_in(root: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = [
        "bin64/Chowdren",
        "Baba Is You.exe",
        "../MacOS/Chowdren",
        "MacOS/Chowdren",
        "Chowdren",
    ]
    .iter()
    .map(|r| root.join(r))
    .collect();
    if let Ok(rd) = std::fs::read_dir(root) {
        let mut exes: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("exe")))
            .filter(|p| {
                !p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.to_ascii_lowercase().starts_with("unins"))
            })
            .collect();
        exes.sort();
        v.extend(exes);
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for p in v {
        if p.is_file() && !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

/// The two sprite directories an object can draw from: `Data/Sprites`
/// ("root") and the world's `Sprites/`.
#[derive(Clone, Debug, Default)]
pub struct SpriteSet {
    /// `Data/Sprites`.
    pub root: SpriteDir,
    /// The world's `Sprites/` (empty without a world).
    pub world: SpriteDir,
}

impl SpriteSet {
    /// Scans both directories.
    pub fn load(install: &GameInstall, world: Option<&World>) -> Result<SpriteSet> {
        Ok(SpriteSet {
            root: SpriteDir::scan(&install.sprites_dir())?,
            world: match world {
                Some(w) => SpriteDir::scan(&w.sprites_dir())?,
                None => SpriteDir::default(),
            },
        })
    }

    /// Finds a sprite frame. Looks in the preferred directory first
    /// (`in_root`), then the other; if the variant is missing everywhere,
    /// falls back to variant 0.
    pub fn find(&self, sprite: &str, in_root: bool, variant: u8, frame: u8) -> Option<&Path> {
        let dirs = if in_root {
            [&self.root, &self.world]
        } else {
            [&self.world, &self.root]
        };
        dirs.iter()
            .find_map(|d| d.get(sprite, variant, frame))
            .or_else(|| dirs.iter().find_map(|d| d.get(sprite, 0, frame)))
    }

    /// True if the sprite exists in either directory.
    pub fn contains(&self, sprite: &str) -> bool {
        self.root.contains(sprite) || self.world.contains(sprite)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let base = std::env::var_os("TMPDIR")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let d = base.join(format!("baba-install-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn touch(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "").unwrap();
    }

    #[test]
    fn locates_the_three_layouts() {
        let d = tmp("layouts");
        // Linux
        let linux = d.join("linux");
        touch(&linux.join("Data/values.lua"));
        touch(&linux.join("bin64/Chowdren"));
        assert_eq!(GameInstall::locate(&linux), Some(linux.clone()));
        assert_eq!(
            GameInstall::locate(&linux.join("Data")),
            Some(linux.clone())
        );
        assert_eq!(
            GameInstall::locate(&linux.join("Data/values.lua")),
            Some(linux.clone())
        );
        let g = GameInstall::open(&linux).unwrap();
        assert_eq!(g.executables(), vec![linux.join("bin64/Chowdren")]);
        // Windows (with a GOG uninstaller that is not the game)
        let win = d.join("win");
        touch(&win.join("Data/values.lua"));
        touch(&win.join("unins000.exe"));
        touch(&win.join("Baba Is You.exe"));
        let g = GameInstall::open(&win).unwrap();
        assert_eq!(g.executables(), vec![win.join("Baba Is You.exe")]);
        // macOS: the bundle, its Contents, or the folder holding it
        let mac = d.join("mac");
        let app = mac.join("Baba Is You.app");
        let res = app.join("Contents/Resources");
        touch(&res.join("Data/values.lua"));
        touch(&app.join("Contents/MacOS/Chowdren"));
        for p in [&mac, &app, &app.join("Contents"), &res] {
            assert_eq!(GameInstall::locate(p), Some(res.clone()), "{}", p.display());
        }
        let g = GameInstall::open(&mac).unwrap();
        assert_eq!(g.executables(), vec![res.join("../MacOS/Chowdren")]);
        // any bundle name
        let other = d.join("other");
        touch(&other.join("Baba.app/Contents/Resources/Data/values.lua"));
        assert_eq!(
            GameInstall::locate(&other),
            Some(other.join("Baba.app/Contents/Resources"))
        );
        // a game folder copied one level too deep (PortMaster gamedata/)
        let gamedata = d.join("gamedata");
        touch(&gamedata.join("Baba Is You/Data/values.lua"));
        assert_eq!(
            GameInstall::locate(&gamedata),
            Some(gamedata.join("Baba Is You"))
        );
        assert!(GameInstall::open(&d.join("nothing")).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
