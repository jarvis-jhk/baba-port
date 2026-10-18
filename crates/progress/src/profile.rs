//! The port's own data directory and the files in it: `ba.ba` (global),
//! `0ba.ba`–`2ba.ba` (slots) and `SettingsC.txt`, with the original's
//! names and key layout (`docs/spec/saves.md` §2) so an import can copy
//! them. Never the original's directory: that one is read only.

use std::path::{Path, PathBuf};

use crate::settings::Settings;
use crate::slot::Slot;
use crate::store::Store;

/// Number of save slots.
pub const SLOTS: usize = 3;

/// Environment variable that overrides the data directory (development
/// builds keep it inside the project, see `env.sh`).
pub const DATA_DIR_ENV: &str = "BABA_PORT_DATA_DIR";

/// The port's data directory: `$BABA_PORT_DATA_DIR`, else the platform's
/// per-user data location + `baba-port`. Never the original's
/// `Baba_Is_You` directory.
pub fn default_data_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os(DATA_DIR_ENV).filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(d));
    }
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if cfg!(windows) {
        return env("APPDATA").map(|p| p.join("baba-port"));
    }
    if cfg!(target_os = "macos") {
        return env("HOME").map(|p| p.join("Library/Application Support/baba-port"));
    }
    env("XDG_DATA_HOME")
        .or_else(|| env("HOME").map(|h| h.join(".local/share")))
        .map(|p| p.join("baba-port"))
}

/// Everything persisted: settings, the global file and three slots.
#[derive(Clone, Debug, Default)]
pub struct Profile {
    /// `None` = in memory only (tests).
    pub dir: Option<PathBuf>,
    pub settings: Settings,
    pub global: Store,
    pub slots: [Slot; SLOTS],
}

fn read(path: &Path) -> String {
    match std::fs::read(path) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!("{}: {e}", path.display());
            }
            String::new()
        }
    }
}

/// Writes through a temporary file and a rename, so a crash never leaves
/// a half-written save.
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

impl Profile {
    /// In-memory profile with defaults.
    pub fn in_memory() -> Self {
        Self {
            dir: None,
            settings: Settings::defaults(),
            ..Default::default()
        }
    }

    /// Loads (missing files = fresh) from `dir`.
    pub fn load(dir: &Path) -> Self {
        let slots = std::array::from_fn(|i| Slot::parse(&read(&dir.join(format!("{i}ba.ba")))));
        Self {
            settings: Settings::parse(&read(&dir.join("SettingsC.txt"))),
            global: Store::parse(&read(&dir.join("ba.ba"))),
            slots,
            dir: Some(dir.to_path_buf()),
        }
    }

    fn write(&self, name: &str, text: &str) {
        if let Some(d) = &self.dir
            && let Err(e) = write_atomic(&d.join(name), text)
        {
            log::error!("saving {name}: {e}");
        }
    }

    pub fn save_settings(&self) {
        self.write("SettingsC.txt", &self.settings.to_text());
    }

    pub fn save_global(&self) {
        self.write("ba.ba", &self.global.to_text());
    }

    pub fn save_slot(&self, slot: usize) {
        if slot < SLOTS {
            self.write(&format!("{slot}ba.ba"), &self.slots[slot].to_text());
        }
    }

    pub fn save_all(&self) {
        self.save_settings();
        self.save_global();
        for i in 0..SLOTS {
            self.save_slot(i);
        }
    }

    /// `[baba] firsttime` of the global file (`saves.md` §3.1).
    pub fn first_time(&self) -> bool {
        self.global.int("baba", "firsttime") == 0
    }

    pub fn set_first_time_done(&mut self) {
        self.global.set("baba", "firsttime", 1);
    }

    pub fn last_slot(&self) -> usize {
        self.global.int("baba", "lastslot").clamp(0, 2) as usize
    }

    pub fn set_last_slot(&mut self, slot: usize) {
        self.global.set("baba", "lastslot", slot);
    }

    /// "Slot N" or the player's name for it.
    pub fn slot_label(&self, slot: usize, default: &str) -> String {
        match self.settings.slot_name(slot) {
            Some(n) => n.to_string(),
            None => format!("{default} {}", slot + 1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_reload() {
        let dir = std::env::var_os("TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("target"))
            .join(format!("baba-progress-test-{}", std::process::id()));
        let mut p = Profile::load(&dir);
        assert!(p.first_time());
        p.set_first_time_done();
        p.set_last_slot(2);
        p.slots[2].set_level_state("baba", "0level", 3);
        p.settings.set_music(33);
        p.save_all();
        let q = Profile::load(&dir);
        assert!(!q.first_time());
        assert_eq!(q.last_slot(), 2);
        assert_eq!(q.slots[2].level_state("baba", "0level"), 3);
        assert_eq!(q.settings.music(), 33);
        assert!(dir.join("SettingsC.txt").is_file());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
