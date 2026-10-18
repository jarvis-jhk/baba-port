//! Importing the original game's progress (`docs/spec/saves.md` §5).
//!
//! The original keeps `ba.ba`, `0ba.ba`–`2ba.ba` and `SettingsC.txt` in a
//! `Baba_Is_You` folder under the platform's per-user data root; Steam
//! Cloud syncs exactly those files into the same folder (its Auto-Cloud
//! entries name them `Baba_Is_You/<file>` under the "AppData Roaming"
//! root, which Steam maps to `%APPDATA%` on Windows, `~/Library/Application
//! Support` on macOS and `$XDG_DATA_HOME` / `~/.local/share` on Linux), so
//! there is no separate cloud copy to look for. Proton and Flatpak installs
//! put the folder inside their own prefixes.
//!
//! baba-port's save uses the original's file names and key layout, so the
//! slots are copied group by group (with the counters re-validated) and the
//! settings are merged key by key. The source folder is only ever read.

use std::path::{Path, PathBuf};

use baba_progress::{Profile, Settings, Slot, Store};

/// Folder name of the original's data.
pub const SAVE_DIR: &str = "Baba_Is_You";
/// Overrides the search (development and tests): a folder holding a copy
/// of an original save.
pub const ENV_IMPORT_FROM: &str = "BABA_IMPORT_FROM";
/// Steam app id of Baba Is You (Proton prefixes).
const STEAM_APP: &str = "736260";

fn env_path(k: &str) -> Option<PathBuf> {
    std::env::var_os(k)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Every place the original may keep its save on this platform, most
/// likely first (existence not checked).
pub fn candidate_dirs() -> Vec<PathBuf> {
    if let Some(d) = env_path(ENV_IMPORT_FROM) {
        return vec![d];
    }
    let mut v = Vec::new();
    let home = env_path("HOME").or_else(|| env_path("USERPROFILE"));
    if cfg!(windows) {
        if let Some(a) = env_path("APPDATA") {
            v.push(a.join(SAVE_DIR));
        }
        if let Some(h) = &home {
            v.push(h.join("AppData/Roaming").join(SAVE_DIR));
        }
    } else if cfg!(target_os = "macos") {
        if let Some(h) = &home {
            v.push(h.join("Library/Application Support").join(SAVE_DIR));
        }
    } else {
        if let Some(x) = env_path("XDG_DATA_HOME") {
            v.push(x.join(SAVE_DIR));
        }
        if let Some(h) = &home {
            v.push(h.join(".local/share").join(SAVE_DIR));
            // Flatpak Steam keeps the game's XDG dirs in its sandbox.
            v.push(
                h.join(".var/app/com.valvesoftware.Steam/.local/share")
                    .join(SAVE_DIR),
            );
            v.push(
                h.join(".var/app/com.valvesoftware.Steam/data")
                    .join(SAVE_DIR),
            );
            // The Windows build under Proton.
            for steam in [
                ".steam/steam",
                ".local/share/Steam",
                ".var/app/com.valvesoftware.Steam/.local/share/Steam",
            ] {
                v.push(
                    h.join(steam)
                        .join("steamapps/compatdata")
                        .join(STEAM_APP)
                        .join("pfx/drive_c/users/steamuser/AppData/Roaming")
                        .join(SAVE_DIR),
                );
            }
        }
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for d in v {
        if !out.contains(&d) {
            out.push(d);
        }
    }
    out
}

/// Summary of one source slot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlotInfo {
    /// Worlds with progress (group names that are world folders).
    pub worlds: Vec<String>,
    /// Spores / flowers / seconds in the base world.
    pub stats: baba_progress::Stats,
    /// Completed levels and maps over all worlds.
    pub completed: usize,
}

/// A found original save.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub dir: PathBuf,
    pub slots: [Option<SlotInfo>; 3],
    pub settings: bool,
}

impl Found {
    pub fn any_progress(&self) -> bool {
        self.slots.iter().flatten().any(|s| !s.worlds.is_empty())
    }
}

fn read(path: &Path) -> Option<String> {
    std::fs::read(path)
        .ok()
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// Group names of a slot that are worlds (not counters or conversions).
fn world_groups(s: &Store) -> Vec<String> {
    let names: Vec<&str> = s.group_names().collect();
    names
        .iter()
        .filter(|g| {
            // A world group has a `firsttime`, `time` or level states.
            s.get(g, "firsttime").is_some()
                || s.get(g, "time").is_some()
                || s.items(g).iter().any(|(k, _)| k.ends_with("_intro"))
        })
        .map(|g| g.to_string())
        .collect()
}

fn slot_info(s: &Store) -> SlotInfo {
    let worlds = world_groups(s);
    let completed = worlds
        .iter()
        .map(|w| {
            s.items(w)
                .iter()
                .filter(|(k, v)| !k.contains(',') && v.trim() == "3")
                .count()
        })
        .sum();
    let slot = Slot { store: s.clone() };
    SlotInfo {
        stats: slot.stats("baba"),
        worlds,
        completed,
    }
}

/// Reads what is in `dir` (None if it holds none of the save files).
pub fn scan(dir: &Path) -> Option<Found> {
    let mut f = Found {
        dir: dir.to_path_buf(),
        ..Default::default()
    };
    let mut any = false;
    for i in 0..3 {
        if let Some(t) = read(&dir.join(format!("{i}ba.ba"))) {
            any = true;
            let s = Store::parse(&t);
            if !s.is_empty() {
                f.slots[i] = Some(slot_info(&s));
            }
        }
    }
    if dir.join("SettingsC.txt").is_file() {
        any = true;
        f.settings = true;
    }
    any |= dir.join("ba.ba").is_file();
    any.then_some(f)
}

/// The first candidate folder that holds a save.
pub fn find() -> Option<Found> {
    candidate_dirs().iter().find_map(|d| scan(d))
}

/// What an import did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub slots: Vec<usize>,
    pub settings: bool,
    pub levels_copied: usize,
    /// Corrections and skipped values (also logged).
    pub notes: Vec<String>,
}

/// Keys the original writes that mean nothing to us (`saves.md` §3.2).
const DROP_KEYS: [&str; 1] = ["m_lastpack"];

/// Converts one slot file: every group copied, mobile leftovers dropped,
/// counter totals re-computed (§5.4).
pub fn convert_slot(text: &str, notes: &mut Vec<String>, slot: usize) -> Slot {
    let src = Store::parse(text);
    let mut out = Store::new();
    for g in src.group_names() {
        for (k, v) in src.items(g) {
            if DROP_KEYS.contains(&k.as_str()) {
                continue;
            }
            out.set(g, k, v);
        }
    }
    let names: Vec<String> = out.group_names().map(str::to_string).collect();
    for g in &names {
        let counted = |f: &dyn Fn(&str) -> i64| -> i64 {
            out.items(g)
                .iter()
                .filter(|(k, _)| k != "total")
                .map(|(_, v)| f(v))
                .sum()
        };
        let want = if g.ends_with("_prize") {
            counted(&|v| baba_formats::ini::parse_int(v).unwrap_or(0).max(0))
        } else if g.ends_with("_clears") || g.ends_with("_bonus") {
            counted(&|v| (baba_formats::ini::parse_int(v).unwrap_or(0) != 0) as i64)
        } else {
            continue;
        };
        let have = out.get_int(g, "total");
        if have != Some(want) {
            let msg = format!(
                "slot {slot}: [{g}] total {} -> {want} (recomputed)",
                have.map(|h| h.to_string())
                    .unwrap_or_else(|| "missing".into())
            );
            log::info!("{msg}");
            notes.push(msg);
            out.set(g, "total", want);
        }
    }
    Slot { store: out }
}

/// `[settings]` keys copied as they are (range-checked by the accessors).
const SETTING_KEYS: [&str; 12] = [
    "music",
    "sound",
    "delay",
    "fullscreen",
    "grid",
    "wobble",
    "particles",
    "shake",
    "contrast",
    "blinking",
    "restartask",
    "zoom",
];

/// Groups copied verbatim (editor bindings, slot names, last session,
/// code history, editor flags).
const VERBATIM: [&str; 7] = [
    "gamepad_editor",
    "gamepad_currobjlist",
    "editor",
    "slotnames",
    "savegame",
    "get_d",
    "get_u",
];

/// Groups never copied: gameplay logging, our own keys, the `settings`
/// spelled differently by one debug call, and the ones handled above.
const SKIP: [&str; 6] = ["log", "port", "settings", "settingS", "keyboard", "gamepad"];

/// A gamepad value we can use: `a<N>`, `dpad` or a button index below 128
/// (larger values are an unexplained encoding, `saves.md` §4.4).
fn pad_value_ok(v: &str) -> bool {
    let v = v.trim();
    v == "dpad"
        || v.strip_prefix('a')
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        || v.parse::<u32>().is_ok_and(|n| n < 128)
}

/// Merges the original's `SettingsC.txt` into ours. `lang_ok` says whether
/// a language code exists in this install.
pub fn merge_settings(
    text: &str,
    ours: &mut Settings,
    lang_ok: &dyn Fn(&str) -> bool,
    notes: &mut Vec<String>,
) {
    let src = Store::parse(text);
    let st = &mut ours.store;
    for k in SETTING_KEYS {
        if let Some(v) = src.get_int("settings", k) {
            st.set("settings", k, v);
        }
    }
    if let Some(l) = src.get("settings", "lang").map(str::trim)
        && !l.is_empty()
    {
        if lang_ok(l) {
            st.set("settings", "lang", l);
        } else {
            notes.push(format!(
                "language {l} not available; kept {}",
                ours.language()
            ));
        }
    }
    let st = &mut ours.store;
    for (k, v) in src.items("keyboard") {
        if baba_formats::ini::parse_int(v).is_some() {
            st.set("keyboard", k, v.trim());
        }
    }
    for (k, v) in src.items("gamepad") {
        if pad_value_ok(v) {
            st.set("gamepad", k, v.trim());
        } else {
            notes.push(format!(
                "gamepad {k}={v}: unknown button encoding, kept {}",
                st.get("gamepad", k).unwrap_or("default")
            ));
        }
    }
    for g in src.group_names() {
        if SKIP.contains(&g) {
            continue;
        }
        if VERBATIM.contains(&g) || ours.store.group(g).is_none() {
            for (k, v) in src.items(g) {
                ours.store.set(g, k, v);
            }
        }
    }
}

/// Copies the original's own single levels (`Data/Worlds/levels` of its
/// install) into ours; stems we already have are left alone.
pub fn copy_levels(from: &Path, to: &Path) -> usize {
    let mut n = 0;
    for e in crate::packs::levels_in(from, false) {
        if to.join(format!("{}.ld", e.stem)).exists() {
            continue;
        }
        let mut ok = true;
        for ext in ["l", "ld", "png"] {
            let src = from.join(format!("{}.{ext}", e.stem));
            if src.is_file() {
                ok &= std::fs::copy(&src, to.join(format!("{}.{ext}", e.stem))).is_ok();
            }
        }
        n += ok as usize;
    }
    n
}

/// Imports everything found in `dir` into `profile` (in memory; the caller
/// saves). Slots present in the source replace ours; settings are merged.
pub fn import(dir: &Path, profile: &mut Profile, lang_ok: &dyn Fn(&str) -> bool) -> Report {
    let mut r = Report::default();
    for i in 0..3 {
        if let Some(t) = read(&dir.join(format!("{i}ba.ba"))) {
            let s = convert_slot(&t, &mut r.notes, i);
            if !s.is_empty() {
                profile.slots[i] = s;
                r.slots.push(i);
            }
        }
    }
    if let Some(t) = read(&dir.join("SettingsC.txt")) {
        merge_settings(&t, &mut profile.settings, lang_ok, &mut r.notes);
        r.settings = true;
    }
    let g = read(&dir.join("ba.ba"))
        .map(|t| Store::parse(&t))
        .unwrap_or_default();
    let last = g
        .get_int("baba", "lastslot")
        .filter(|s| (0..3).contains(s))
        .map(|s| s as usize)
        .or_else(|| r.slots.first().copied());
    if let Some(l) = last {
        profile.set_last_slot(l);
    }
    // The intro has been played on that install: no first-launch start.
    profile.set_first_time_done();
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLOT: &str = "[baba_prize]\r\n177level=15\r\ntotal=40\r\n106level=8\r\n\r\n\
        [baba]\r\n0level=3\r\nm_lastpack=0\r\n106level,16,11=3\r\nfirsttime=1\r\ntime=13365\r\n\
        leveltree=106level,206level\r\nleveltree_id=Map,Ruins\r\n0level_intro=1\r\n\r\n\
        [baba_clears]\r\ntotal=1\r\n177level=1\r\n\r\n[mypack]\r\n1level=3\r\n1level_intro=1\r\n";

    #[test]
    fn slot_groups_copied_and_counters_fixed() {
        let mut notes = Vec::new();
        let s = convert_slot(SLOT, &mut notes, 0);
        assert_eq!(s.prize_total("baba"), 23);
        assert_eq!(s.clears_total("baba"), 1);
        assert_eq!(s.level_state("baba", "0level"), 3);
        assert_eq!(s.path_state("baba", "106level", 16, 11), 3);
        assert_eq!(s.store.get("baba", "m_lastpack"), None);
        assert_eq!(
            s.level_tree("baba").unwrap().0,
            vec!["106level".to_string(), "206level".to_string()]
        );
        // Unknown packs keep their progress.
        assert_eq!(s.level_state("mypack", "1level"), 3);
        assert_eq!(notes.len(), 1, "{notes:?}");
        let info = slot_info(&s.store);
        assert_eq!(info.worlds, ["baba", "mypack"]);
        assert_eq!(info.completed, 2);
    }

    #[test]
    fn settings_merged() {
        let mut ours = Settings::defaults();
        ours.store.set("port", "nopad", 1);
        let src = "[settings]\nmusic=71\nzoom=2\nfullscreen=1\nrestartask=1\nlang=xx\n\
            [keyboard]\nundo=122\nup2=119\n[gamepad]\nrestart=136\npause=8\nmove=a0\n\
            [slotnames]\n0=a\n[log]\nlog=1\n[gamepad_editor]\nsave=b8\n[get_d]\ntotal=0\n";
        let mut notes = Vec::new();
        merge_settings(src, &mut ours, &|l| l == "de", &mut notes);
        assert_eq!(ours.music(), 71);
        assert_eq!(ours.zoom(), baba_progress::Zoom::Stretch);
        assert!(ours.toggle(baba_progress::Toggle::RestartNoAsk));
        assert_eq!(ours.language(), "en");
        assert_eq!(ours.pad(baba_progress::PadAction::Pause, false), "8");
        assert_eq!(ours.pad(baba_progress::PadAction::Restart, false), "5");
        assert_eq!(ours.slot_name(0), Some("a"));
        assert!(ours.pad_disabled());
        assert!(ours.store.group("log").is_none());
        assert_eq!(ours.store.get("gamepad_editor", "save"), Some("b8"));
        assert_eq!(notes.len(), 2, "{notes:?}");
    }

    #[test]
    fn import_reads_only_and_fills_profile() {
        let dir = std::env::var_os("TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("target"))
            .join(format!("baba-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("2ba.ba"), SLOT).unwrap();
        std::fs::write(dir.join("ba.ba"), "[baba]\nfirsttime=1\nlastslot=2\n").unwrap();
        std::fs::write(dir.join("SettingsC.txt"), "[settings]\nsound=40\n").unwrap();
        let before: Vec<_> = ["2ba.ba", "ba.ba", "SettingsC.txt"]
            .iter()
            .map(|f| std::fs::read(dir.join(f)).unwrap())
            .collect();
        let f = scan(&dir).unwrap();
        assert!(f.slots[2].is_some() && f.slots[0].is_none() && f.settings);
        let mut p = Profile::in_memory();
        let r = import(&dir, &mut p, &|_| true);
        assert_eq!(r.slots, [2]);
        assert_eq!(p.last_slot(), 2);
        assert!(!p.first_time());
        assert_eq!(p.settings.sound(), 40);
        assert_eq!(p.slots[2].level_state("baba", "0level"), 3);
        let after: Vec<_> = ["2ba.ba", "ba.ba", "SettingsC.txt"]
            .iter()
            .map(|f| std::fs::read(dir.join(f)).unwrap())
            .collect();
        assert_eq!(before, after);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
