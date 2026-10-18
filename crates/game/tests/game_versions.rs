//! The game's sound effects resolve to the same effects whichever build
//! of Baba Is You the player has (Linux, Windows, macOS). Runs only where
//! read-only copies of the three builds exist (`BABA_GAME_DIR_LINUX` /
//! `_WINDOWS` / `_MAC`, default `tools/baba-{original,windows,mac}` of the
//! project); skips otherwise.

use std::path::{Path, PathBuf};

use baba_formats::{AssetKind, AssetsDat, GameInstall};
use baba_game::sounds::SoundBank;

fn copy(var: &str, dir: &str) -> Option<PathBuf> {
    let p = std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../tools")
                .join(dir)
        });
    let root = GameInstall::locate(&p);
    if root.is_none() {
        eprintln!("skipping {dir}: no copy at {}", p.display());
    }
    root
}

fn names(root: &Path) -> Vec<String> {
    let a = AssetsDat::load(&root.join("Assets.dat")).unwrap();
    let count = a.of_kind(AssetKind::Sound).count();
    let exe = std::fs::read(root.join("bin64/Chowdren")).unwrap();
    baba_formats::exe::sound_names(&exe, count).unwrap()
}

#[test]
fn sound_effects_resolve_alike() {
    let (Some(linux), Some(win), Some(mac)) = (
        copy("BABA_GAME_DIR_LINUX", "baba-original"),
        copy("BABA_GAME_DIR_WINDOWS", "baba-windows"),
        copy("BABA_GAME_DIR_MAC", "baba-mac"),
    ) else {
        return;
    };
    let names = names(&linux);
    let mut banks: Vec<SoundBank> = [&linux, &win, &mac]
        .iter()
        .map(|r| SoundBank::load(r))
        .collect();
    for b in &banks {
        assert_eq!(b.len(), names.len());
    }
    for n in &names {
        let i = banks[0].index_of(n);
        assert!(i.is_some(), "{n}");
        assert_eq!(banks[1].index_of(n), i, "windows {n}");
        assert_eq!(banks[2].index_of(n), i, "mac {n}");
    }
    // Decoded samples are identical too (a few, decoding is slow in debug).
    for n in names.iter().step_by(61) {
        let d: Vec<_> = banks.iter_mut().map(|b| b.get(n).unwrap()).collect();
        assert_eq!(d[0].samples, d[1].samples, "{n}");
        assert_eq!(d[0].samples, d[2].samples, "{n}");
    }
}
