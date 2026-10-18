//! The Linux, Windows and macOS builds of the game read alike: same
//! images and sounds from `Assets.dat`, same sound names from each
//! executable. Runs only where read-only copies of the three builds exist
//! (`BABA_GAME_DIR_LINUX` / `_WINDOWS` / `_MAC`, default
//! `tools/baba-{original,windows,mac}` of the project); skips otherwise.

use std::path::PathBuf;

use baba_formats::{AssetKind, AssetsDat, GameInstall, ShaderLanguage, exe};

fn copy(var: &str, dir: &str) -> Option<GameInstall> {
    let p = std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../tools")
                .join(dir)
        });
    match GameInstall::open(&p) {
        Ok(g) => Some(g),
        Err(_) => {
            eprintln!("skipping {dir}: no copy at {}", p.display());
            None
        }
    }
}

fn copies() -> Option<[(&'static str, GameInstall); 3]> {
    Some([
        ("linux", copy("BABA_GAME_DIR_LINUX", "baba-original")?),
        ("windows", copy("BABA_GAME_DIR_WINDOWS", "baba-windows")?),
        ("mac", copy("BABA_GAME_DIR_MAC", "baba-mac")?),
    ])
}

fn sound_names(g: &GameInstall, count: usize) -> Vec<String> {
    let exes = g.executables();
    assert!(!exes.is_empty(), "no executable in {}", g.root.display());
    exes.iter()
        .find_map(|p| exe::sound_names(&std::fs::read(p).unwrap(), count))
        .unwrap_or_else(|| panic!("no sound names in {exes:?}"))
}

#[test]
fn layouts() {
    let Some([(_, linux), (_, win), (_, mac)]) = copies() else {
        return;
    };
    assert!(linux.executables()[0].ends_with("bin64/Chowdren"));
    assert!(win.executables()[0].ends_with("Baba Is You.exe"));
    assert!(mac.root.ends_with("Baba Is You.app/Contents/Resources"));
    assert!(mac.executables()[0].ends_with("MacOS/Chowdren"));
    for g in [&linux, &win, &mac] {
        assert!(g.assets_dat().is_file());
        assert!(g.root.join("gamecontrollerdb.txt").is_file());
    }
}

#[test]
fn assets_and_sound_names_match() {
    let Some(copies) = copies() else { return };
    let opened: Vec<(&str, AssetsDat)> = copies
        .iter()
        .map(|(n, g)| (*n, AssetsDat::load(&g.assets_dat()).unwrap()))
        .collect();
    let (_, reference) = &opened[0];
    let count = reference.of_kind(AssetKind::Sound).count();
    let names = sound_names(&copies[0].1, count);
    assert!(names.iter().all(|n| !n.is_empty()));
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), count, "sound names are distinct");
    for ((name, a), (_, g)) in opened.iter().zip(&copies) {
        let expect = if *name == "windows" {
            ShaderLanguage::D3d9
        } else {
            ShaderLanguage::Glsl
        };
        assert_eq!(a.shader_language(), Some(expect), "{name}");
        for kind in [AssetKind::Image, AssetKind::Sound] {
            let ours: Vec<_> = a.of_kind(kind).collect();
            let theirs: Vec<_> = reference.of_kind(kind).collect();
            assert_eq!(ours.len(), theirs.len(), "{name} {kind:?}");
            for (x, y) in ours.iter().zip(&theirs) {
                assert_eq!(a.raw(x), reference.raw(y), "{name} {kind:?} {}", x.index);
            }
        }
        for e in a.of_kind(AssetKind::Image) {
            a.image(e).unwrap();
        }
        for e in a.of_kind(AssetKind::Shader) {
            a.shader(e).unwrap();
        }
        assert_eq!(sound_names(g, count), names, "{name}: sound names");
    }
}
