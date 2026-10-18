//! Finding the player's copy of Baba Is You for the desktop binary
//! (release builds start without `env.sh`). The install is only read.
//!
//! Order: `--game-dir` (handled by the caller), `$BABA_GAME_DIR`,
//! `$WARPZONE_DATA_BABA` (the Warpzone launcher hands over what it found,
//! `packaging/warpzone/baba-port.toml`), `gamedata/` next to the executable
//! (portable installs, PortMaster), `<data dir>/game` (a copy the player
//! made), then the usual Steam libraries (incl. `libraryfolders.vdf`) and
//! GOG folders of the platform. A candidate counts when it (or its macOS
//! `.app/Contents/Resources`) has `Data/values.lua`. Every build works
//! (Linux, Windows, macOS: `Assets.dat` next to `Data/`, the sound names in
//! the executable; `baba_formats::install`).

use std::path::{Path, PathBuf};

/// Folder name of the game under `steamapps/common` and in GOG installs.
const FOLDER: &str = "Baba Is You";

/// The folder that holds `Data/values.lua` for a candidate path: the
/// folder itself (Linux, Windows), its `Data/` or `Data/values.lua`, or the
/// `Contents/Resources` of a macOS app bundle (given directly, as its
/// `Contents/`, or as a folder holding the `.app`).
/// `baba_formats::GameInstall::locate` does the work.
pub fn resolve(p: &Path) -> Option<PathBuf> {
    baba_formats::GameInstall::locate(p)
}

fn env_path(k: &str) -> Option<PathBuf> {
    std::env::var_os(k)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Steam library roots listed in a `libraryfolders.vdf` (`"path" "..."`).
fn vdf_libraries(vdf: &Path) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(vdf) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| {
            let mut parts = l.split('"').filter(|s| !s.trim().is_empty());
            (parts.next()? == "path").then(|| parts.next())?
        })
        .map(|p| PathBuf::from(p.replace("\\\\", "\\")))
        .collect()
}

/// Every place to look, in order (existing or not).
pub fn candidates(data_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut v = Vec::new();
    for k in ["BABA_GAME_DIR", "WARPZONE_DATA_BABA"] {
        v.extend(env_path(k));
    }
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
    {
        v.push(exe_dir.join("gamedata"));
    }
    if let Some(d) = data_dir {
        v.push(d.join("game"));
    }
    let home = env_path("HOME").or_else(|| env_path("USERPROFILE"));
    let mut steam_roots = Vec::new();
    if cfg!(windows) {
        for k in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(p) = env_path(k) {
                steam_roots.push(p.join("Steam"));
                v.push(p.join("GOG Galaxy/Games").join(FOLDER));
            }
        }
        steam_roots.push(PathBuf::from("C:/Program Files (x86)/Steam"));
        v.push(PathBuf::from("C:/GOG Games").join(FOLDER));
    } else if cfg!(target_os = "macos") {
        if let Some(h) = &home {
            steam_roots.push(h.join("Library/Application Support/Steam"));
            v.push(h.join("Applications"));
        }
        v.push(PathBuf::from("/Applications"));
    } else if let Some(h) = &home {
        for s in [
            ".steam/steam",
            ".local/share/Steam",
            ".var/app/com.valvesoftware.Steam/.local/share/Steam",
            ".steam/debian-installation",
        ] {
            steam_roots.push(h.join(s));
        }
        v.push(h.join("GOG Games").join(FOLDER));
        v.push(h.join("Games").join(FOLDER));
    }
    let mut libs = Vec::new();
    for r in &steam_roots {
        libs.push(r.clone());
        libs.extend(vdf_libraries(&r.join("steamapps/libraryfolders.vdf")));
        libs.extend(vdf_libraries(&r.join("config/libraryfolders.vdf")));
    }
    for l in libs {
        v.push(l.join("steamapps/common").join(FOLDER));
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for p in v {
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

/// The first candidate with the game's data, if any.
pub fn find(data_dir: Option<&Path>) -> Option<PathBuf> {
    candidates(data_dir).iter().find_map(|p| resolve(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let base = env_path("TMPDIR").unwrap_or_else(std::env::temp_dir);
        let d = base.join(format!("baba-gamedir-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn resolves_folder_data_file_and_app_bundle() {
        let d = tmp("resolve");
        let game = d.join("Baba Is You");
        std::fs::create_dir_all(game.join("Data")).unwrap();
        std::fs::write(game.join("Data/values.lua"), "").unwrap();
        assert_eq!(resolve(&game), Some(game.clone()));
        assert_eq!(resolve(&game.join("Data")), Some(game.clone()));
        assert_eq!(resolve(&game.join("Data/values.lua")), Some(game.clone()));
        assert_eq!(resolve(&d.join("nothing")), None);
        let res = d.join("mac/Baba Is You.app/Contents/Resources");
        std::fs::create_dir_all(res.join("Data")).unwrap();
        std::fs::write(res.join("Data/values.lua"), "").unwrap();
        assert_eq!(resolve(&d.join("mac")), Some(res.clone()));
        assert_eq!(resolve(&d.join("mac/Baba Is You.app")), Some(res));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn reads_steam_library_folders() {
        let d = tmp("vdf");
        let vdf = d.join("libraryfolders.vdf");
        std::fs::write(
            &vdf,
            "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"/mnt/games/Steam\"\n\t\t\"label\"\t\t\"\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\SteamLibrary\"\n\t}\n}\n",
        )
        .unwrap();
        assert_eq!(
            vdf_libraries(&vdf),
            vec![
                PathBuf::from("/mnt/games/Steam"),
                PathBuf::from("D:\\SteamLibrary")
            ]
        );
        std::fs::remove_dir_all(&d).unwrap();
    }
}
