//! Files and save data of online levels: the temporary download
//! (`<data>/temp/temp.{l,ld,png}`, the original's `Data/Temp`), saving it
//! into the user `levels` world, the code history (`[get_d]`, `[get_u]` in
//! the settings, §1.3) and the completion of online levels
//! (`[levels_won]` in the slot).

use std::path::{Path, PathBuf};

use baba_progress::{Settings, Slot};

use super::Record;

/// Stem of the temporary level (the original's `temp`; its level name is
/// `_TEMP_`).
pub const TEMP: &str = "temp";
/// At most this many codes per history group (§1.3).
pub const HISTORY_MAX: usize = 15;

/// A level fetched into the temp folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Downloaded {
    pub code: String,
    pub record: Record,
    /// Folder holding `temp.l`, `temp.ld` (and `temp.png`).
    pub dir: PathBuf,
    pub has_png: bool,
}

/// `<data>/temp`.
pub fn temp_dir(user: &Path) -> PathBuf {
    user.join("temp")
}

/// `<data>/temp/thumbs` (featured list thumbnails, kept between runs).
pub fn thumbs_dir(user: &Path) -> PathBuf {
    temp_dir(user).join("thumbs")
}

/// Sets `[general] levelcode=<code>` in `.ld` text (inserted after the
/// `[general]` line; replaced if present).
pub fn with_levelcode(ld: &[u8], code: &str) -> Vec<u8> {
    let text = String::from_utf8_lossy(ld);
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut out = String::new();
    let mut in_general = false;
    let mut done = false;
    let mut saw_general = false;
    for line in text.split_inclusive('\n') {
        let t = line.trim_end_matches(['\r', '\n']);
        if t.starts_with('[') {
            in_general = t == "[general]";
            if in_general {
                saw_general = true;
                out.push_str(line);
                if !line.ends_with('\n') {
                    out.push_str(nl);
                }
                if !done {
                    out.push_str(&format!("levelcode={code}{nl}"));
                    done = true;
                }
                continue;
            }
        }
        if in_general && t.starts_with("levelcode=") {
            continue;
        }
        out.push_str(line);
    }
    if !saw_general {
        out.push_str(&format!("{nl}[general]{nl}levelcode={code}{nl}"));
    }
    out.into_bytes()
}

/// Copies the temporary level into the user `levels` world under the next
/// free `<N>level` stem (§3.5 "Save the level"). Returns the stem.
pub fn save_downloaded(temp: &Path, user: &Path) -> std::io::Result<String> {
    let dir = crate::packs::ensure_user_levels(user)?;
    let stem = crate::packs::next_level_stem(&dir)?;
    for ext in ["l", "ld", "png"] {
        let src = temp.join(format!("{TEMP}.{ext}"));
        if src.is_file() {
            std::fs::copy(&src, dir.join(format!("{stem}.{ext}")))?;
        }
    }
    Ok(stem)
}

/// History of `group` (`get_d` downloaded, `get_u` uploaded): `(code,
/// name)`, newest first.
pub fn history(s: &Settings, group: &str) -> Vec<(String, String)> {
    let n = s.store.int(group, "total").max(0) as usize;
    (0..n)
        .filter_map(|i| {
            let c = s.store.get(group, &format!("{i}code"))?;
            let name = s.store.get(group, &format!("{i}name")).unwrap_or("");
            Some((c.to_string(), name.to_string()))
        })
        .collect()
}

/// Puts `code` on top of the history (duplicates removed, at most
/// [`HISTORY_MAX`]).
pub fn push_history(s: &mut Settings, group: &str, code: &str, name: &str) {
    let mut h = history(s, group);
    h.retain(|(c, _)| c != code);
    h.insert(0, (code.to_string(), name.to_string()));
    h.truncate(HISTORY_MAX);
    s.store.remove_group(group);
    s.store.set(group, "total", h.len());
    for (i, (c, n)) in h.iter().enumerate() {
        s.store.set(group, &format!("{i}code"), c);
        s.store.set(group, &format!("{i}name"), n);
    }
}

/// Completion of an online level (`[levels_won] <code>=won,bonus,end,
/// done,converted`; the exact key layout of the original is an open
/// question in the spec, §10.5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Won {
    pub won: bool,
    pub bonus: bool,
    pub end: bool,
    pub done: bool,
    pub converted: bool,
}

pub const WON_GROUP: &str = "levels_won";

pub fn won(slot: &Slot, code: &str) -> Won {
    let v = slot.store.get(WON_GROUP, code).unwrap_or("");
    let f: Vec<bool> = v.split(',').map(|x| x.trim() == "1").collect();
    let g = |i: usize| f.get(i).copied().unwrap_or(false);
    Won {
        won: g(0),
        bonus: g(1),
        end: g(2),
        done: g(3),
        converted: g(4),
    }
}

/// Merges `w` into the stored record (flags are only ever added).
pub fn record_won(slot: &mut Slot, code: &str, w: Won) {
    let old = won(slot, code);
    let b = |x: bool| if x { "1" } else { "0" };
    let v = format!(
        "{},{},{},{},{}",
        b(old.won || w.won),
        b(old.bonus || w.bonus),
        b(old.end || w.end),
        b(old.done || w.done),
        b(old.converted || w.converted)
    );
    slot.store.set(WON_GROUP, code, v);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levelcode_inserted_or_replaced() {
        let a = with_levelcode(b"[tiles]\r\nx=1\r\n[general]\r\nname=A\r\n", "ABCD-1234");
        assert_eq!(
            String::from_utf8(a).unwrap(),
            "[tiles]\r\nx=1\r\n[general]\r\nlevelcode=ABCD-1234\r\nname=A\r\n"
        );
        let b = with_levelcode(b"[general]\nlevelcode=OLD0-0000\nname=A\n", "ABCD-1234");
        assert_eq!(
            String::from_utf8(b).unwrap(),
            "[general]\nlevelcode=ABCD-1234\nname=A\n"
        );
        let c = with_levelcode(b"[tiles]\n", "ABCD-1234");
        assert!(
            String::from_utf8(c)
                .unwrap()
                .ends_with("[general]\nlevelcode=ABCD-1234\n")
        );
    }

    #[test]
    fn history_is_capped_and_deduplicated() {
        let mut s = Settings::defaults();
        for i in 0..20 {
            push_history(&mut s, "get_d", &format!("AAAA-{i:04}"), &format!("L{i}"));
        }
        push_history(&mut s, "get_d", "AAAA-0010", "again");
        let h = history(&s, "get_d");
        assert_eq!(h.len(), 15);
        assert_eq!(h[0], ("AAAA-0010".to_string(), "again".to_string()));
        assert_eq!(h[1].0, "AAAA-0019");
        assert_eq!(h.iter().filter(|(c, _)| c == "AAAA-0010").count(), 1);
    }

    #[test]
    fn won_flags_merge() {
        let mut s = Slot::default();
        record_won(
            &mut s,
            "ABCD-1234",
            Won {
                won: true,
                ..Default::default()
            },
        );
        record_won(
            &mut s,
            "ABCD-1234",
            Won {
                bonus: true,
                ..Default::default()
            },
        );
        assert_eq!(s.store.get(WON_GROUP, "ABCD-1234"), Some("1,1,0,0,0"));
        assert!(won(&s, "ABCD-1234").won);
    }

    #[test]
    fn save_into_levels_world() {
        let root = std::env::var_os("TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("target"))
            .join(format!("baba-lcstore-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let t = temp_dir(&root);
        std::fs::create_dir_all(&t).unwrap();
        std::fs::write(t.join("temp.l"), b"L").unwrap();
        std::fs::write(
            t.join("temp.ld"),
            b"[general]\nname=X\nlevelcode=ABCD-1234\n",
        )
        .unwrap();
        assert_eq!(save_downloaded(&t, &root).unwrap(), "0level");
        assert_eq!(save_downloaded(&t, &root).unwrap(), "1level");
        let l = crate::packs::levels_in(&crate::packs::user_levels_dir(&root), true);
        assert_eq!(l.len(), 2);
        assert_eq!(l[1].code, "ABCD-1234");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
