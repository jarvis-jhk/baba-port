//! Custom world packs and the user `levels` world
//! (`docs/spec/worldpacks.md` §1, §4–§6).
//!
//! Packs are read from two places: the port's own data directory
//! (`<data>/worlds/<folder>/`, where the player drops packs and where
//! downloaded and imported levels go) and the install's `Data/Worlds/`
//! (read only). A folder in the data directory takes precedence over an
//! install folder of the same name. Lua mods are not supported: packs that
//! ship them are listed and playable with the standard rules, and starting
//! one shows a notice (once per pack).

use std::path::{Path, PathBuf};

use baba_formats::WorldData;

use crate::world::WorldInfo;

/// Folder of the user worlds inside the port's data dir.
pub const USER_WORLDS: &str = "worlds";
/// The single-level collection (`worldpacks.md` §6).
pub const LEVELS: &str = "levels";

/// Folders never offered as packs (`worldpacks.md` §1). `debug` is the
/// original's test pack (debug builds only).
const HIDDEN: [&str; 5] = ["levels", "baba", "baba_m", "new_adv_m", "debug"];

/// `<data>/worlds`.
pub fn user_worlds_dir(user: &Path) -> PathBuf {
    user.join(USER_WORLDS)
}

/// `<data>/worlds/levels` (created on demand with a fresh
/// `world_data.txt`).
pub fn user_levels_dir(user: &Path) -> PathBuf {
    user_worlds_dir(user).join(LEVELS)
}

/// Where a world folder lives: the data dir first, then the install.
pub fn find_dir(install_data: &Path, user: Option<&Path>, id: &str) -> Option<(PathBuf, bool)> {
    if id.is_empty() || id.contains(['/', '\\']) || id == "." || id == ".." {
        return None;
    }
    if let Some(u) = user {
        let d = user_worlds_dir(u).join(id);
        if d.is_dir() {
            return Some((d, true));
        }
    }
    let d = install_data.join("Worlds").join(id);
    d.is_dir().then_some((d, false))
}

/// Opens a world by folder name from either place.
pub fn open_world(install_data: &Path, user: Option<&Path>, id: &str) -> Option<WorldInfo> {
    match find_dir(install_data, user, id)? {
        (d, true) => WorldInfo::open_dir(&d, id),
        (_, false) => WorldInfo::open(install_data, id),
    }
}

/// True if the pack ships Lua mods (a `Lua/` folder with `.lua` files, or
/// `mods=1`; `worldpacks.md` §4).
pub fn has_lua(dir: &Path, data: &WorldData) -> bool {
    data.mods
        || std::fs::read_dir(dir.join("Lua"))
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| {
                e.path()
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("lua"))
            })
}

/// One listed pack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pack {
    /// Folder name (save group name).
    pub id: String,
    pub dir: PathBuf,
    /// From the port's data directory.
    pub user: bool,
    /// `world_data.txt` name ("" if none).
    pub name: String,
    pub author: String,
    /// Ships Lua mods (not supported).
    pub lua: bool,
    /// Has a start level we can open.
    pub playable: bool,
}

fn folders(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut v: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| Some((e.file_name().to_str()?.to_string(), e.path())))
        .collect();
    v.sort();
    v
}

fn pack(id: &str, dir: PathBuf, user: bool) -> Pack {
    let data = std::fs::read(dir.join("world_data.txt"))
        .map(|b| WorldData::parse(&String::from_utf8_lossy(&b)))
        .unwrap_or_default();
    let start = if data.start.is_empty() {
        &data.first_level
    } else {
        &data.start
    };
    let playable = !start.is_empty()
        && dir.join(format!("{start}.l")).is_file()
        && dir.join(format!("{start}.ld")).is_file();
    Pack {
        id: id.to_string(),
        lua: has_lua(&dir, &data),
        name: data.name.clone(),
        author: data.author.clone(),
        dir,
        user,
        playable,
    }
}

/// Packs for the start list / "Play levelpacks" (`ui.md` §3.3, §8.4): the
/// base world first, then every other folder of both places by name (the
/// data dir's copy wins), without `levels` and the hidden folders. Only
/// folders with a `world_data.txt` are listed (they need one to start).
pub fn list(install_data: &Path, user: Option<&Path>) -> Vec<Pack> {
    let mut out: Vec<Pack> = Vec::new();
    let mut seen = std::collections::BTreeMap::new();
    if let Some(u) = user {
        for (id, d) in folders(&user_worlds_dir(u)) {
            seen.insert(id.clone(), (d, true));
        }
    }
    for (id, d) in folders(&install_data.join("Worlds")) {
        seen.entry(id).or_insert((d, false));
    }
    if let Some((d, user)) = seen.get("baba")
        && d.join("world_data.txt").is_file()
    {
        out.push(pack("baba", d.clone(), *user));
    }
    for (id, (d, user)) in seen {
        if HIDDEN.contains(&id.as_str()) || !d.join("world_data.txt").is_file() {
            continue;
        }
        out.push(pack(&id, d, user));
    }
    out
}

/// Ids only (the existing pack menu).
pub fn list_ids(install_data: &Path, user: Option<&Path>) -> Vec<String> {
    list(install_data, user).into_iter().map(|p| p.id).collect()
}

/// A single level of the `levels` collection (ours or the install's).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LevelEntry {
    pub stem: String,
    pub dir: PathBuf,
    /// In the port's data dir (writable); else the install's (read only).
    pub user: bool,
    pub name: String,
    pub author: String,
    pub subtitle: String,
    /// `[general] levelcode` (downloaded or uploaded levels).
    pub code: String,
}

/// Levels of a `levels` folder (`.ld` + `.l` pairs), by natural order.
pub fn levels_in(dir: &Path, user: bool) -> Vec<LevelEntry> {
    let mut stems: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension()? == "ld")
                .then(|| p.file_stem()?.to_str().map(str::to_string))
                .flatten()
        })
        .filter(|s| dir.join(format!("{s}.l")).is_file())
        .collect();
    stems.sort_by_key(|s| baba_formats::world::natural_key(s));
    stems
        .into_iter()
        .map(|stem| {
            let ini = baba_formats::Ini::load(&dir.join(format!("{stem}.ld"))).unwrap_or_default();
            let g = |k: &str| ini.get("general", k).unwrap_or("").trim().to_string();
            LevelEntry {
                name: g("name"),
                author: g("author"),
                subtitle: g("subtitle"),
                code: g("levelcode"),
                stem,
                dir: dir.to_path_buf(),
                user,
            }
        })
        .collect()
}

/// "Play custom levels": ours first, then the install's `levels` folder
/// (stems we already have are skipped).
pub fn custom_levels(install_data: &Path, user: Option<&Path>) -> Vec<LevelEntry> {
    let mut v = match user {
        Some(u) => levels_in(&user_levels_dir(u), true),
        None => Vec::new(),
    };
    for e in levels_in(&install_data.join("Worlds").join(LEVELS), false) {
        if !v.iter().any(|x| x.stem == e.stem) {
            v.push(e);
        }
    }
    v
}

/// Creates `<data>/worlds/levels` with a `world_data.txt` if missing.
pub fn ensure_user_levels(user: &Path) -> std::io::Result<PathBuf> {
    let d = user_levels_dir(user);
    std::fs::create_dir_all(&d)?;
    let wd = d.join("world_data.txt");
    if !wd.is_file() {
        std::fs::write(
            &wd,
            "[levels]\nlevelid=0\nrolling_levelid=0\n\n[general]\nthemecount=0\n\n",
        )?;
    }
    Ok(d)
}

/// Next free `<N>level` stem in a `levels` folder (`editor.md` §10.2): the
/// first unused number ≥ max(number of levels, `[levels] levelid`); the
/// counters in `world_data.txt` are advanced.
pub fn next_level_stem(dir: &Path) -> std::io::Result<String> {
    let wd = dir.join("world_data.txt");
    let mut st = baba_progress::Store::parse(
        &std::fs::read(&wd)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default(),
    );
    let count = levels_in(dir, true).len() as i64;
    let mut n = st.int("levels", "levelid").max(count).max(0);
    while dir.join(format!("{n}level.ld")).exists() || dir.join(format!("{n}level.l")).exists() {
        n += 1;
    }
    st.set("levels", "levelid", n + 1);
    let rolling = st.int("levels", "rolling_levelid").max(n) + 1;
    st.set("levels", "rolling_levelid", rolling);
    std::fs::write(&wd, st.to_text())?;
    Ok(format!("{n}level"))
}

/// One-time Lua notice bookkeeping (`[port_luanotice] <pack>=1` in the
/// settings).
pub fn lua_notice_due(settings: &baba_progress::Settings, id: &str) -> bool {
    settings.store.int("port_luanotice", id) == 0
}

pub fn lua_notice_seen(settings: &mut baba_progress::Settings, id: &str) {
    settings.store.set("port_luanotice", id, 1);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::var_os("TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("target"))
            .join(format!("baba-packs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn world(dir: &Path, name: &str, extra: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("world_data.txt"),
            format!("[general]\nname={name}\nstart=1level\n{extra}"),
        )
        .unwrap();
        std::fs::write(dir.join("1level.l"), b"x").unwrap();
        std::fs::write(dir.join("1level.ld"), b"[general]\nname=One\n").unwrap();
    }

    #[test]
    fn lists_both_places_user_first() {
        let root = tmp("list");
        let install = root.join("Data");
        let user = root.join("user");
        world(&install.join("Worlds/baba"), "Baba", "");
        world(&install.join("Worlds/museum"), "Museum", "");
        world(&install.join("Worlds/debug"), "Test", "mods=1\n");
        world(&install.join("Worlds/levels"), "", "");
        world(&user.join("worlds/museum"), "My museum", "");
        world(&user.join("worlds/zpack"), "Z", "");
        std::fs::create_dir_all(user.join("worlds/zpack/Lua")).unwrap();
        std::fs::write(user.join("worlds/zpack/Lua/mod.lua"), b"--").unwrap();
        std::fs::create_dir_all(user.join("worlds/nodata")).unwrap();
        let l = list(&install, Some(&user));
        let ids: Vec<&str> = l.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["baba", "museum", "zpack"]);
        assert_eq!(l[1].name, "My museum");
        assert!(l[1].user && !l[0].user);
        assert!(l[2].lua && !l[1].lua && l[2].playable);
        let w = open_world(&install, Some(&user), "zpack").unwrap();
        assert!(w.external && w.res_key().ends_with("zpack"));
        let b = open_world(&install, Some(&user), "baba").unwrap();
        assert_eq!(b.res_key(), "baba");
        assert!(open_world(&install, Some(&user), "../x").is_none());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn level_slots_advance() {
        let root = tmp("slots");
        let d = ensure_user_levels(&root).unwrap();
        assert_eq!(next_level_stem(&d).unwrap(), "0level");
        std::fs::write(
            d.join("1level.ld"),
            b"[general]\nname=A\nlevelcode=ABCD-EFGH\n",
        )
        .unwrap();
        std::fs::write(d.join("1level.l"), b"x").unwrap();
        // levelid is 1 now, 1level exists → 2level.
        assert_eq!(next_level_stem(&d).unwrap(), "2level");
        let l = levels_in(&d, true);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].code, "ABCD-EFGH");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
