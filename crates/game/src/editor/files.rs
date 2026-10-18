//! Folders and files the editor manages (`docs/spec/editor.md` §3, §5.8,
//! §8.4–§8.5, §10): the user `levels` world and user packs in the port's
//! data directory, new level / pack / theme ids, deleting and copying
//! levels, themes, music, palettes, particles and sprite lists.
//!
//! The editor writes only into the port's data directory
//! (`<data>/worlds/...`); the install is read.

use std::path::{Path, PathBuf};

use baba_formats::world::{list_files, natural_key};
use baba_progress::Store;

use crate::packs;

/// A level in a list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LevelItem {
    pub stem: String,
    pub name: String,
    pub is_map: bool,
}

/// Stem prefix of files the editor writes for test plays (never listed).
pub const TEST_STEM: &str = "_EDITORTEST_";

/// Levels of a folder (`.l` + `.ld`), natural order; test files skipped.
pub fn levels(dir: &Path) -> Vec<LevelItem> {
    packs::levels_in(dir, true)
        .into_iter()
        .filter(|e| !e.stem.starts_with('_'))
        .map(|e| {
            let ini =
                baba_formats::Ini::load(&dir.join(format!("{}.ld", e.stem))).unwrap_or_default();
            LevelItem {
                is_map: ini.get_int("general", "leveltype") == Some(1),
                name: e.name,
                stem: e.stem,
            }
        })
        .collect()
}

/// Sorted / filtered level list (`§3.1`).
pub fn sorted(mut v: Vec<LevelItem>, by_name: bool, by_type: bool, search: &str) -> Vec<LevelItem> {
    let s = search.to_lowercase();
    if !s.is_empty() {
        v.retain(|l| l.name.to_lowercase().contains(&s));
    }
    if by_name {
        v.sort_by_key(|a| a.name.to_lowercase());
    }
    if by_type {
        v.sort_by_key(|l| !l.is_map);
    }
    v
}

/// A pack the editor can edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackItem {
    pub id: String,
    pub dir: PathBuf,
    pub name: String,
    pub author: String,
    /// In the install (read only): editing works on a copy in the data
    /// directory.
    pub install: bool,
}

/// Folders never edited (the shipped worlds, `§3.3`).
const SHIPPED: [&str; 7] = [
    "levels",
    "baba",
    "baba_m",
    "new_adv",
    "new_adv_m",
    "museum",
    "debug",
];

fn pack_dirs(root: &Path, install: bool) -> Vec<PackItem> {
    std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let id = e.file_name().to_str()?.to_string();
            if SHIPPED.contains(&id.as_str()) {
                return None;
            }
            let st = world_store(&e.path());
            Some(PackItem {
                name: st.get("general", "name").unwrap_or("").to_string(),
                author: st.get("general", "author").unwrap_or("").to_string(),
                dir: e.path(),
                id,
                install,
            })
        })
        .collect()
}

/// Packs the editor can edit (`§3.3`): the folders of `<data>/worlds/`
/// and of the install's `Data/Worlds/` (not ours already) except
/// `levels` and the shipped worlds; those by `author` first.
pub fn packs(user: &Path, install_data: &Path, author: &str) -> Vec<PackItem> {
    let mut v = pack_dirs(&packs::user_worlds_dir(user), false);
    for p in pack_dirs(&install_data.join("Worlds"), true) {
        if !v.iter().any(|q| q.id == p.id) {
            v.push(p);
        }
    }
    v.sort_by_key(|p| natural_key(&p.id));
    if !author.is_empty() {
        v.sort_by_key(|p| p.author != author);
    }
    v
}

/// `world_data.txt` of a folder as a store.
pub fn world_store(dir: &Path) -> Store {
    Store::parse(
        &std::fs::read(dir.join("world_data.txt"))
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default(),
    )
}

/// Copies a folder tree (an install pack into the data directory).
pub fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)?.flatten() {
        let p = e.path();
        let dest = to.join(e.file_name());
        if p.is_dir() {
            copy_dir(&p, &dest)?;
        } else {
            std::fs::copy(&p, &dest)?;
        }
    }
    Ok(())
}

/// Changes `world_data.txt` keys.
pub fn set_world(dir: &Path, f: impl FnOnce(&mut Store)) -> std::io::Result<()> {
    let mut st = world_store(dir);
    f(&mut st);
    std::fs::write(dir.join("world_data.txt"), st.to_text())
}

/// Creates a pack folder `<N>world` (`§3.3`, `§10.2`): N is the first free
/// number ≥ the number of folders.
pub fn create_pack(user: &Path, name: &str, author: &str) -> std::io::Result<PackItem> {
    let root = packs::user_worlds_dir(user);
    std::fs::create_dir_all(&root)?;
    let count = std::fs::read_dir(&root)?
        .flatten()
        .filter(|e| e.path().is_dir())
        .count();
    let mut n = count;
    while root.join(format!("{n}world")).exists() {
        n += 1;
    }
    let id = format!("{n}world");
    let dir = root.join(&id);
    std::fs::create_dir_all(&dir)?;
    let mut st = Store::new();
    st.set("general", "name", name);
    st.set("general", "author", author);
    st.set("general", "start", "");
    st.set("general", "firstlevel", "");
    st.set("general", "themecount", 0);
    st.set("levels", "levelid", 0);
    st.set("levels", "rolling_levelid", 0);
    std::fs::write(dir.join("world_data.txt"), st.to_text())?;
    Ok(PackItem {
        id,
        dir,
        name: name.to_string(),
        author: author.to_string(),
        install: false,
    })
}

/// A new level stem in `dir` (`§10.2`).
pub fn new_stem(dir: &Path) -> std::io::Result<String> {
    std::fs::create_dir_all(dir)?;
    if !dir.join("world_data.txt").is_file() {
        std::fs::write(
            dir.join("world_data.txt"),
            "[levels]\nlevelid=0\nrolling_levelid=0\n\n[general]\nthemecount=0\n\n",
        )?;
    }
    packs::next_level_stem(dir)
}

/// Deletes `.l`, `.ld` and `.png` (`§8.4`).
pub fn delete_level(dir: &Path, stem: &str) {
    for ext in ["l", "ld", "png"] {
        let _ = std::fs::remove_file(dir.join(format!("{stem}.{ext}")));
    }
}

/// The name of a copy (`§8.5`): a trailing number goes up by one, else
/// " 2" is appended.
pub fn copy_name(name: &str) -> String {
    let digits: String = name
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if !digits.is_empty()
        && let Ok(n) = digits.parse::<u64>()
    {
        return format!("{}{}", &name[..name.len() - digits.len()], n + 1);
    }
    format!("{name} 2")
}

/// Copies a saved level to a new stem with a new name (`§8.5`).
pub fn copy_level(dir: &Path, stem: &str, new_name: &str) -> std::io::Result<String> {
    let new = new_stem(dir)?;
    let l = std::fs::read(dir.join(format!("{stem}.l")))?;
    let ld = std::fs::read(dir.join(format!("{stem}.ld")))?;
    let mut info = baba_formats::LevelInfo::parse(&String::from_utf8_lossy(&ld));
    info.general.name = new_name.to_string();
    info.ini.remove("general", "levelcode");
    std::fs::write(dir.join(format!("{new}.l")), l)?;
    std::fs::write(dir.join(format!("{new}.ld")), info.to_text())?;
    if let Ok(png) = std::fs::read(dir.join(format!("{stem}.png"))) {
        let _ = std::fs::write(dir.join(format!("{new}.png")), png);
    }
    Ok(new)
}

/// A theme file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeItem {
    pub path: PathBuf,
    pub name: String,
    /// From the pack / levels folder (deletable), not the install's.
    pub own: bool,
}

/// Themes (`§5.8`): the install's `Data/Themes/` and the collection's
/// `Themes/`.
pub fn themes(install_data: &Path, coll: Option<&Path>) -> Vec<ThemeItem> {
    let mut v = Vec::new();
    let mut add = |dir: &Path, own: bool| {
        for p in list_files(dir, "txt") {
            let ini = baba_formats::Ini::load(&p).unwrap_or_default();
            v.push(ThemeItem {
                name: ini.get("general", "name").unwrap_or("").to_string(),
                path: p,
                own,
            });
        }
    };
    add(&install_data.join("Themes"), false);
    if let Some(d) = coll {
        add(&d.join("Themes"), true);
    }
    v
}

/// Saves a theme into `<coll>/Themes/` (`§5.8`): a theme of the same name
/// is overwritten, else the first free `<N>theme.txt`; `themecount` is
/// advanced.
pub fn save_theme(coll: &Path, name: &str, text: &str) -> std::io::Result<PathBuf> {
    let dir = coll.join("Themes");
    std::fs::create_dir_all(&dir)?;
    let existing = themes(Path::new("/nonexistent"), Some(coll))
        .into_iter()
        .find(|t| t.own && t.name == name);
    let path = match existing {
        Some(t) => t.path,
        None => {
            let mut n = world_store(coll).int("general", "themecount").max(0);
            while dir.join(format!("{n}theme.txt")).exists() {
                n += 1;
            }
            let _ = set_world(coll, |st| st.set("general", "themecount", n + 1));
            dir.join(format!("{n}theme.txt"))
        }
    };
    std::fs::write(&path, text)?;
    Ok(path)
}

/// Music tracks (`§7` item 4): `(stem, from the pack)`.
pub fn music(install_data: &Path, coll: Option<&Path>) -> Vec<(String, bool)> {
    let mut v: Vec<(String, bool)> = list_files(&install_data.join("Music"), "ogg")
        .iter()
        .filter_map(|p| p.file_stem()?.to_str().map(|s| (s.to_string(), false)))
        .filter(|(s, _)| !s.starts_with("ambience"))
        .collect();
    if let Some(d) = coll {
        for p in list_files(&d.join("Music"), "ogg") {
            if let Some(s) = p.file_stem().and_then(|s| s.to_str()) {
                v.push((s.to_string(), true));
            }
        }
    }
    v
}

/// Palettes (`§7` item 6): `(file, in root)`.
pub fn palettes(install_data: &Path, coll: Option<&Path>) -> Vec<(String, bool)> {
    let mut v: Vec<(String, bool)> = list_files(&install_data.join("Palettes"), "png")
        .iter()
        .filter_map(|p| p.file_name()?.to_str().map(|s| (s.to_string(), true)))
        .collect();
    if let Some(d) = coll {
        for p in list_files(&d.join("Palettes"), "png") {
            if let Some(s) = p.file_name().and_then(|s| s.to_str()) {
                v.push((s.to_string(), false));
            }
        }
    }
    v
}

/// Background particle styles (`presentation.md` §10, without `world`).
pub const PARTICLES: [&str; 12] = [
    "bubbles", "soot", "sparks", "dust", "snow", "clouds", "smoke", "pollen", "stars", "glitter",
    "leaves", "rain",
];

/// Sprite names for the sprite picker (`§5.6`): root sprites with a
/// `_0_1` frame, then the collection's; `(name, in root)`.
pub fn sprites(install_data: &Path, coll: Option<&Path>) -> Vec<(String, bool)> {
    let names = |dir: &Path| -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_str()?.to_string();
                n.strip_suffix("_0_1.png").map(str::to_string)
            })
            .collect();
        v.sort();
        v
    };
    let mut out: Vec<(String, bool)> = names(&install_data.join("Sprites"))
        .into_iter()
        .map(|n| (n, true))
        .collect();
    if let Some(d) = coll {
        out.extend(names(&d.join("Sprites")).into_iter().map(|n| (n, false)));
    }
    out
}

/// Pictures for custom map icons (`§13.1`, `[icons]`): file stems of
/// `icon_*` images and of first sprite frames (`*_0_1`); `(stem, in root)`.
pub fn icon_sprites(install_data: &Path, coll: Option<&Path>) -> Vec<(String, bool)> {
    let stems = |dir: &Path| -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_str()?.to_string();
                let s = n.strip_suffix(".png")?;
                (s.starts_with("icon_") || s.ends_with("_0_1")).then(|| s.to_string())
            })
            .collect();
        v.sort_by_key(|s| !s.starts_with("icon_"));
        v
    };
    let mut out: Vec<(String, bool)> = Vec::new();
    if let Some(d) = coll {
        out.extend(stems(&d.join("Sprites")).into_iter().map(|n| (n, false)));
    }
    out.extend(
        stems(&install_data.join("Sprites"))
            .into_iter()
            .map(|n| (n, true)),
    );
    out
}

/// Removes leftover test-play files of a folder.
pub fn clean_test_files(dir: &Path) {
    for ext in ["l", "ld", "png"] {
        let _ = std::fs::remove_file(dir.join(format!("{TEST_STEM}.{ext}")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_names() {
        assert_eq!(copy_name("Pond 3"), "Pond 4");
        assert_eq!(copy_name("Pond"), "Pond 2");
        assert_eq!(copy_name("x9"), "x10");
    }

    #[test]
    fn packs_and_themes() {
        let root = std::env::var_os("TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("target"))
            .join(format!("baba-edfiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let p = create_pack(&root, "My pack", "me").unwrap();
        assert_eq!(p.id, "0world");
        let q = create_pack(&root, "Other", "you").unwrap();
        assert_eq!(q.id, "1world");
        let list = packs(&root, Path::new("/nonexistent"), "you");
        assert_eq!(list[0].id, "1world");
        assert_eq!(list[1].name, "My pack");
        let s = new_stem(&p.dir).unwrap();
        assert_eq!(s, "0level");
        let t = save_theme(&p.dir, "Mine", "[general]\nname=Mine\n").unwrap();
        assert!(t.ends_with("Themes/0theme.txt"));
        let t2 = save_theme(&p.dir, "Mine", "[general]\nname=Mine\n").unwrap();
        assert_eq!(t, t2);
        assert_eq!(world_store(&p.dir).int("general", "themecount"), 1);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
