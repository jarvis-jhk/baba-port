//! The Android import keeps a working install from a zip of any build's
//! folder. The zips are simulated from read-only copies of the three
//! builds (`BABA_GAME_DIR_LINUX` / `_WINDOWS` / `_MAC`, default
//! `tools/baba-{original,windows,mac}`; skipped where missing): entry names
//! as a zip of the folder would hold them, mapped by `zip_paths`, and the
//! kept files linked (not copied) into a temporary folder.
#![cfg(unix)]

use std::path::{Path, PathBuf};

use baba_formats::{AssetKind, AssetsDat, GameInstall, exe};
use babaport::zip_paths;

fn source(var: &str, dir: &str) -> Option<PathBuf> {
    let p = std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../tools")
                .join(dir)
        });
    if p.is_dir() {
        Some(p)
    } else {
        eprintln!("skipping {dir}: no copy at {}", p.display());
        None
    }
}

/// Every file under `dir`, as zip entry names below `top/`.
fn entries(dir: &Path, top: &str, out: &mut Vec<(String, PathBuf)>) {
    let mut rd: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
    rd.sort_by_key(|e| e.file_name());
    for e in rd {
        let name = format!("{top}{}", e.file_name().to_string_lossy());
        if e.path().is_dir() {
            entries(&e.path(), &format!("{name}/"), out);
        } else {
            out.push((name, e.path()));
        }
    }
}

fn import(name: &str, folder: &Path, top: &str) -> Vec<String> {
    let mut list = Vec::new();
    entries(folder, top, &mut list);
    let prefix = zip_paths::data_prefix(list.iter().map(|(n, _)| n.as_str())).unwrap();
    let base = std::env::var_os("TMPDIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let game = base.join(format!("baba-import-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&game);
    let mut kept = Vec::new();
    for (entry, src) in &list {
        if let Some(rel) = zip_paths::target(entry, &prefix) {
            let dest = game.join(&rel);
            std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(src, &dest).unwrap();
            kept.push(rel);
        }
    }
    let g = GameInstall::open(&game).unwrap();
    assert_eq!(g.root, game);
    let a = AssetsDat::load(&g.assets_dat()).unwrap();
    let count = a.of_kind(AssetKind::Sound).count();
    let exes = g.executables();
    let names = exes
        .iter()
        .find_map(|p| exe::sound_names(&std::fs::read(p).unwrap(), count))
        .unwrap_or_else(|| panic!("{name}: no sound names in {exes:?}"));
    std::fs::remove_dir_all(&game).unwrap();
    kept.retain(|k| !k.starts_with("Data/"));
    kept.sort();
    eprintln!("{name}: kept {kept:?} + Data/");
    names
}

#[test]
fn zips_of_every_build_import() {
    let (Some(linux), Some(win), Some(mac)) = (
        source("BABA_GAME_DIR_LINUX", "baba-original"),
        source("BABA_GAME_DIR_WINDOWS", "baba-windows"),
        source("BABA_GAME_DIR_MAC", "baba-mac"),
    ) else {
        return;
    };
    let reference = import("linux", &linux, "Baba Is You/");
    // Windows: "Send to > Compressed folder" of the install folder, and a
    // zip of its contents (no top folder).
    assert_eq!(import("windows", &win, "Baba Is You/"), reference);
    assert_eq!(import("windows-flat", &win, ""), reference);
    // Mac: Finder's "Compress" of the .app (entries start with the bundle).
    assert_eq!(import("mac", &mac, ""), reference);
}
