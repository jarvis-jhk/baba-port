//! Save slots for multiplayer: the profile's three slots (`0ba.ba` –
//! `2ba.ba`, the original's) plus nine extra slots `3ba.ba` – `11ba.ba`
//! in the same data directory and format, so twelve players can each have
//! their own game in mode 3. Only the port reads the extra slots (the
//! original and the save import use 0–2).

use std::path::{Path, PathBuf};

use baba_progress::{Profile, SLOTS, Slot};

/// Slots selectable in multiplayer.
pub const ALL_SLOTS: usize = 12;

/// Slots 3..12.
#[derive(Default)]
pub struct ExtraSlots {
    dir: Option<PathBuf>,
    slots: Vec<Slot>,
}

impl ExtraSlots {
    /// Loads the extra slots from `dir` (`None` = in memory).
    pub fn load(dir: Option<&Path>) -> ExtraSlots {
        let slots = (SLOTS..ALL_SLOTS)
            .map(|i| {
                dir.and_then(|d| std::fs::read(d.join(format!("{i}ba.ba"))).ok())
                    .map(|b| Slot::parse(&String::from_utf8_lossy(&b)))
                    .unwrap_or_default()
            })
            .collect();
        ExtraSlots {
            dir: dir.map(Path::to_path_buf),
            slots,
        }
    }

    fn write(&self, i: usize) {
        let (Some(d), Some(s)) = (&self.dir, self.slots.get(i - SLOTS)) else {
            return;
        };
        let path = d.join(format!("{i}ba.ba"));
        let tmp = path.with_extension("tmp");
        let r = std::fs::create_dir_all(d)
            .and_then(|_| std::fs::write(&tmp, s.to_text()))
            .and_then(|_| std::fs::rename(&tmp, &path));
        if let Err(e) = r {
            log::error!("saving {}: {e}", path.display());
        }
    }
}

/// Slot `i` (0–11).
pub fn slot<'a>(profile: &'a Profile, extra: &'a ExtraSlots, i: usize) -> &'a Slot {
    if i < SLOTS {
        &profile.slots[i]
    } else {
        &extra.slots[(i - SLOTS).min(extra.slots.len() - 1)]
    }
}

/// Slot `i` (0–11), mutable.
pub fn slot_mut<'a>(profile: &'a mut Profile, extra: &'a mut ExtraSlots, i: usize) -> &'a mut Slot {
    if i < SLOTS {
        &mut profile.slots[i]
    } else {
        let n = extra.slots.len() - 1;
        &mut extra.slots[(i - SLOTS).min(n)]
    }
}

/// Writes slot `i`.
pub fn save(profile: &Profile, extra: &ExtraSlots, i: usize) {
    if i < SLOTS {
        profile.save_slot(i);
    } else {
        extra.write(i);
    }
}

/// "Slot N" (or its name) for slot `i`.
pub fn label(profile: &Profile, i: usize, word: &str) -> String {
    profile.slot_label(i, word)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extra_slots_round_trip() {
        let dir = std::env::var_os("TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("target"))
            .join(format!("baba-extra-slots-{}", std::process::id()));
        let mut p = Profile::in_memory();
        let mut x = ExtraSlots::load(Some(&dir));
        slot_mut(&mut p, &mut x, 7).set_level_state("baba", "0level", 3);
        slot_mut(&mut p, &mut x, 1).set_level_state("baba", "1level", 3);
        save(&p, &x, 7);
        let y = ExtraSlots::load(Some(&dir));
        assert_eq!(slot(&p, &y, 7).level_state("baba", "0level"), 3);
        assert_eq!(slot(&p, &y, 8).level_state("baba", "0level"), 0);
        assert_eq!(slot(&p, &y, 1).level_state("baba", "1level"), 3);
        assert!(dir.join("7ba.ba").is_file());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
