//! One save slot (`{n}ba.ba`): per-world progress in the original's key
//! layout (`docs/spec/saves.md` §3), so a later importer can copy the
//! original's files group by group.

use crate::store::Store;

/// Summary shown on slot and pack buttons (`ui.md` §3.1).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub seconds: i64,
    pub prize: i64,
    pub clears: i64,
    pub bonus: i64,
    pub ended: bool,
    pub done: bool,
}

/// A slot file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Slot {
    pub store: Store,
}

impl Slot {
    pub fn parse(text: &str) -> Self {
        Self {
            store: Store::parse(text),
        }
    }

    pub fn to_text(&self) -> String {
        self.store.to_text()
    }

    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
    }

    /// Erases everything (`saves.md` §3.6 "Erase a slot").
    pub fn erase(&mut self) {
        self.store.clear();
    }

    /// Deletes one pack's groups (`saves.md` §3.6 "Erase save").
    pub fn erase_world(&mut self, world: &str) {
        let prefix = format!("{world}_");
        self.store.retain_groups(|g| {
            g != world
                && !(g.starts_with(&prefix)
                    && matches!(
                        &g[prefix.len()..],
                        "prize"
                            | "clears"
                            | "bonus"
                            | "complete"
                            | "converts"
                            | "converts_single"
                            | "end_single"
                            | "done_single"
                    ))
                && !(g.starts_with(&prefix) && g.ends_with("_convert"))
        });
    }

    // ------------------------------------------------------------ states

    /// Saved state of every icon targeting `stem` (0 = use the `.ld`).
    pub fn level_state(&self, world: &str, stem: &str) -> i32 {
        self.store.int(world, stem) as i32
    }

    pub fn set_level_state(&mut self, world: &str, stem: &str, state: i32) {
        if state > 0 {
            self.store.set(world, stem, state);
        }
    }

    /// Saved path state of `<map>,<x>,<y>` (0 = none).
    pub fn path_state(&self, world: &str, map: &str, x: i32, y: i32) -> i32 {
        self.store.int(world, &format!("{map},{x},{y}")) as i32
    }

    pub fn set_path_state(&mut self, world: &str, map: &str, x: i32, y: i32, state: i32) {
        if state > 0 {
            self.store.set(world, &format!("{map},{x},{y}"), state);
        }
    }

    /// `<stem>_intro`: entered at least once.
    pub fn entered(&self, world: &str, stem: &str) -> bool {
        self.flag(world, &format!("{stem}_intro"))
    }

    pub fn set_entered(&mut self, world: &str, stem: &str) {
        self.set_flag(world, &format!("{stem}_intro"), true);
    }

    /// A `1` flag in the world group (`intro`, `firsttime`, `end`, `done`).
    pub fn flag(&self, world: &str, key: &str) -> bool {
        self.store.int(world, key) != 0
    }

    pub fn set_flag(&mut self, world: &str, key: &str, on: bool) {
        if on {
            self.store.set(world, key, 1);
        } else {
            self.store.remove(world, key);
        }
    }

    // ---------------------------------------------------------- counters

    /// Spores earned inside `map`.
    pub fn prize(&self, world: &str, map: &str) -> i64 {
        self.store.int(&format!("{world}_prize"), map)
    }

    /// Spores in total.
    pub fn prize_total(&self, world: &str) -> i64 {
        self.store.int(&format!("{world}_prize"), "total")
    }

    /// One more spore for `map` (first completion of a level in it).
    pub fn add_prize(&mut self, world: &str, map: &str) {
        let g = format!("{world}_prize");
        let n = self.store.int(&g, map) + 1;
        self.store.set(&g, map, n);
        let t = self.store.int(&g, "total") + 1;
        self.store.set(&g, "total", t);
    }

    pub fn clears_total(&self, world: &str) -> i64 {
        self.store.int(&format!("{world}_clears"), "total")
    }

    pub fn cleared(&self, world: &str, map: &str) -> bool {
        self.store.int(&format!("{world}_clears"), map) != 0
    }

    /// Records a map clear (flower); false if it was cleared before.
    pub fn add_clear(&mut self, world: &str, map: &str) -> bool {
        let g = format!("{world}_clears");
        if self.store.int(&g, map) != 0 {
            return false;
        }
        self.store.set(&g, map, 1);
        let t = self.store.int(&g, "total") + 1;
        self.store.set(&g, "total", t);
        true
    }

    pub fn bonus_total(&self, world: &str) -> i64 {
        self.store.int(&format!("{world}_bonus"), "total")
    }

    /// Records an orb for `key` (the level stem; `saves.md` open question
    /// 3); false if already recorded.
    pub fn add_bonus(&mut self, world: &str, key: &str) -> bool {
        let g = format!("{world}_bonus");
        if self.store.int(&g, key) != 0 {
            return false;
        }
        self.store.set(&g, key, 1);
        let t = self.store.int(&g, "total") + 1;
        self.store.set(&g, "total", t);
        true
    }

    pub fn complete(&self, world: &str, map: &str) -> bool {
        self.store.int(&format!("{world}_complete"), map) != 0
    }

    pub fn set_complete(&mut self, world: &str, map: &str) {
        self.store.set(&format!("{world}_complete"), map, 1);
    }

    // ------------------------------------------------------ position, tree

    pub fn previous(&self, world: &str) -> Option<&str> {
        self.store.get(world, "Previous").filter(|s| !s.is_empty())
    }

    pub fn set_previous(&mut self, world: &str, stem: &str) {
        self.store.set(world, "Previous", stem);
    }

    /// Saved level tree `(stems, ids)`, root first.
    pub fn level_tree(&self, world: &str) -> Option<(Vec<String>, Vec<String>)> {
        let t = self
            .store
            .get(world, "leveltree")
            .filter(|s| !s.is_empty())?;
        let stems: Vec<String> = t.split(',').map(str::to_string).collect();
        let ids: Vec<String> = self
            .store
            .get(world, "leveltree_id")
            .unwrap_or("")
            .split(',')
            .map(str::to_string)
            .collect();
        let ids = (0..stems.len())
            .map(|i| ids.get(i).cloned().unwrap_or_else(|| "dummy".into()))
            .collect();
        Some((stems, ids))
    }

    pub fn set_level_tree(&mut self, world: &str, stems: &[String], ids: &[String]) {
        self.store.set(world, "leveltree", stems.join(","));
        self.store.set(world, "leveltree_id", ids.join(","));
    }

    /// `levelsurrounds` (`saves.md` §3.2): direction, then nine tagged cells.
    pub fn set_surrounds(&mut self, world: &str, dir: u8, cells: &[Vec<String>; 9]) {
        const TAGS: [&str; 9] = ["r", "u", "l", "d", "dr", "ur", "ul", "dl", "o"];
        let mut s = format!("{dir},");
        for (tag, names) in TAGS.iter().zip(cells) {
            s.push_str(tag);
            s.push(',');
            if names.is_empty() {
                s.push_str("-,");
            } else {
                for n in names {
                    s.push_str(n);
                    s.push(',');
                }
            }
        }
        self.store.set(world, "levelsurrounds", s);
    }

    /// Parses `levelsurrounds` back into `(dir, cells)`.
    pub fn surrounds(&self, world: &str) -> Option<(u8, [Vec<String>; 9])> {
        const TAGS: [&str; 9] = ["r", "u", "l", "d", "dr", "ur", "ul", "dl", "o"];
        let v = self.store.get(world, "levelsurrounds")?;
        let mut parts = v.split(',').filter(|p| !p.is_empty());
        let dir: u8 = parts.next()?.parse().ok()?;
        let mut cells: [Vec<String>; 9] = Default::default();
        let mut cur: Option<usize> = None;
        for p in parts {
            if let Some(i) = TAGS.iter().position(|t| *t == p) {
                // A tag can also be a unit name only if it follows a tag;
                // the original never names units after direction tags.
                cur = Some(i);
            } else if p != "-"
                && let Some(i) = cur
            {
                cells[i].push(p.to_string());
            }
        }
        Some((dir, cells))
    }

    // ------------------------------------------------------- conversions

    /// Level conversions recorded under `map` (`saves.md` §3.4).
    pub fn conversions(&self, world: &str, map: &str) -> Vec<(String, Vec<String>)> {
        let g = format!("{world}_{map}_convert");
        let n = self.store.int(&g, "converts");
        (0..n)
            .filter_map(|i| {
                let v = self.store.get(&g, &i.to_string())?;
                let mut it = v.split(',').filter(|s| !s.is_empty());
                let stem = it.next()?.to_string();
                Some((stem, it.map(str::to_string).collect()))
            })
            .collect()
    }

    /// Stores (or replaces) the conversion of `stem` on `map`; an empty
    /// list removes it.
    pub fn set_conversion(&mut self, world: &str, map: &str, stem: &str, objects: &[String]) {
        let mut list = self.conversions(world, map);
        list.retain(|(s, _)| s != stem);
        if !objects.is_empty() {
            list.push((stem.to_string(), objects.to_vec()));
        }
        let g = format!("{world}_{map}_convert");
        let all = format!("{world}_converts");
        self.store.remove_group(&g);
        let mut groups: Vec<String> = (0..self.store.int(&all, "total"))
            .filter_map(|i| self.store.get(&all, &i.to_string()).map(str::to_string))
            .collect();
        groups.retain(|x| *x != g);
        if !list.is_empty() {
            self.store.set(&g, "converts", list.len());
            for (i, (s, objs)) in list.iter().enumerate() {
                let mut v = format!("{s},");
                for o in objs {
                    v.push_str(o);
                    v.push(',');
                }
                self.store.set(&g, &i.to_string(), v);
            }
            groups.push(g);
        }
        self.store.remove_group(&all);
        if !groups.is_empty() {
            self.store.set(&all, "total", groups.len());
            for (i, name) in groups.iter().enumerate() {
                self.store.set(&all, &i.to_string(), name);
            }
        }
    }

    // ------------------------------------------------------------- time

    /// Adds play time in frames (60 per second; `time` + `time_frames`).
    pub fn add_frames(&mut self, world: &str, frames: u64) {
        let total = self.store.int(world, "time_frames") as u64 + frames;
        let secs = self.store.int(world, "time") as u64 + total / 60;
        self.store.set(world, "time", secs);
        self.store.set(world, "time_frames", total % 60);
    }

    pub fn stats(&self, world: &str) -> Stats {
        Stats {
            seconds: self.store.int(world, "time"),
            prize: self.prize_total(world),
            clears: self.clears_total(world),
            bonus: self.bonus_total(world),
            ended: self.flag(world, "end"),
            done: self.flag(world, "done"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_and_layout() {
        let mut s = Slot::default();
        s.set_level_state("baba", "0level", 3);
        s.add_prize("baba", "106level");
        s.add_prize("baba", "106level");
        assert!(s.add_clear("baba", "177level"));
        assert!(!s.add_clear("baba", "177level"));
        s.set_path_state("baba", "106level", 10, 15, 3);
        s.set_entered("baba", "1level");
        assert_eq!(s.prize_total("baba"), 2);
        assert_eq!(s.prize("baba", "106level"), 2);
        assert_eq!(s.clears_total("baba"), 1);
        assert_eq!(s.path_state("baba", "106level", 10, 15), 3);
        let t = s.to_text();
        assert!(t.contains("[baba]\n0level=3\n106level,10,15=3\n1level_intro=1\n"));
        assert!(t.contains("[baba_prize]\n106level=2\ntotal=2\n"));
    }

    #[test]
    fn tree_and_surrounds() {
        let mut s = Slot::default();
        s.set_level_tree(
            "baba",
            &["106level".into(), "177level".into(), "180level".into()],
            &["dummy".into(), "lake".into(), "3".into()],
        );
        let (t, ids) = s.level_tree("baba").unwrap();
        assert_eq!(t[2], "180level");
        assert_eq!(ids[1], "lake");
        let mut cells: [Vec<String>; 9] = Default::default();
        cells[0].push("level".into());
        cells[3].push("line".into());
        cells[8].push("cursor".into());
        s.set_surrounds("baba", 0, &cells);
        assert_eq!(
            s.store.get("baba", "levelsurrounds"),
            Some("0,r,level,u,-,l,-,d,line,dr,-,ur,-,ul,-,dl,-,o,cursor,")
        );
        assert_eq!(s.surrounds("baba").unwrap(), (0, cells));
    }

    #[test]
    fn conversions_compact() {
        let mut s = Slot::default();
        s.set_conversion("baba", "106level", "5level", &["rock".into()]);
        s.set_conversion(
            "baba",
            "106level",
            "6level",
            &["keke".into(), "flag".into()],
        );
        s.set_conversion("baba", "106level", "5level", &[]);
        assert_eq!(
            s.conversions("baba", "106level"),
            vec![(
                "6level".to_string(),
                vec!["keke".to_string(), "flag".to_string()]
            )]
        );
        assert_eq!(
            s.store.get("baba_converts", "0"),
            Some("baba_106level_convert")
        );
        s.set_conversion("baba", "106level", "6level", &[]);
        assert!(s.store.group("baba_converts").is_none());
    }

    #[test]
    fn erase_world_keeps_others() {
        let mut s = Slot::default();
        s.set_level_state("baba", "0level", 3);
        s.add_prize("baba", "106level");
        s.set_level_state("new_adv", "42level", 3);
        s.add_prize("new_adv", "41level");
        s.erase_world("new_adv");
        assert_eq!(
            s.store.group_names().collect::<Vec<_>>(),
            vec!["baba", "baba_prize"]
        );
    }

    #[test]
    fn time() {
        let mut s = Slot::default();
        s.add_frames("baba", 125);
        s.add_frames("baba", 59);
        assert_eq!(s.stats("baba").seconds, 3);
        assert_eq!(s.store.int("baba", "time_frames"), 4);
    }
}
