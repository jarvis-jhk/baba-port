//! An ordered, editable store in the game's ini dialect
//! (`docs/spec/saves.md` §1): groups of `key=value` items. Reading goes
//! through `baba_formats::Ini` (all line terminators, duplicate groups
//! merged, last value wins); writing emits `[group]`, its items and one
//! empty line per group, with LF line ends like the original's Linux build.

use baba_formats::Ini;
use baba_formats::ini::parse_int;

/// One `[group]`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub items: Vec<(String, String)>,
}

/// Groups in insertion order; keys unique within a group.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Store {
    groups: Vec<Group>,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses text; never fails (unknown lines are skipped).
    pub fn parse(text: &str) -> Self {
        let ini = Ini::parse(text);
        let mut s = Store::new();
        for name in ini.section_names() {
            if name.is_empty() {
                continue;
            }
            for (k, v) in ini.entries(name) {
                s.set(name, k, v);
            }
        }
        s
    }

    /// Serialises (`[group]`, items, empty line; LF).
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for g in &self.groups {
            out.push('[');
            out.push_str(&g.name);
            out.push_str("]\n");
            for (k, v) in &g.items {
                out.push_str(k);
                out.push('=');
                out.push_str(v);
                out.push('\n');
            }
            out.push('\n');
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    pub fn clear(&mut self) {
        self.groups.clear();
    }

    pub fn group(&self, name: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.name == name)
    }

    pub fn group_names(&self) -> impl Iterator<Item = &str> {
        self.groups.iter().map(|g| g.name.as_str())
    }

    /// Items of a group (empty if missing).
    pub fn items(&self, name: &str) -> &[(String, String)] {
        self.group(name).map(|g| g.items.as_slice()).unwrap_or(&[])
    }

    pub fn get(&self, group: &str, key: &str) -> Option<&str> {
        self.group(group)?
            .items
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Integer value (lenient, like the game); `None` if missing or not a
    /// number.
    pub fn get_int(&self, group: &str, key: &str) -> Option<i64> {
        parse_int(self.get(group, key)?)
    }

    /// Integer value or 0.
    pub fn int(&self, group: &str, key: &str) -> i64 {
        self.get_int(group, key).unwrap_or(0)
    }

    pub fn set(&mut self, group: &str, key: &str, value: impl ToString) {
        let value = value.to_string();
        let g = match self.groups.iter().position(|g| g.name == group) {
            Some(i) => &mut self.groups[i],
            None => {
                self.groups.push(Group {
                    name: group.to_string(),
                    items: Vec::new(),
                });
                self.groups.last_mut().expect("pushed")
            }
        };
        match g.items.iter_mut().find(|(k, _)| k == key) {
            Some(item) => item.1 = value,
            None => g.items.push((key.to_string(), value)),
        }
    }

    pub fn remove(&mut self, group: &str, key: &str) {
        if let Some(g) = self.groups.iter_mut().find(|g| g.name == group) {
            g.items.retain(|(k, _)| k != key);
        }
    }

    pub fn remove_group(&mut self, group: &str) {
        self.groups.retain(|g| g.name != group);
    }

    /// Keeps only the groups for which `keep` returns true.
    pub fn retain_groups(&mut self, mut keep: impl FnMut(&str) -> bool) {
        self.groups.retain(|g| keep(&g.name));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_edit() {
        let mut s = Store::parse(
            "[baba]\r\nfirsttime=1\r\nleveltree=106level,0level\r\n[baba]\rlastslot=2\r",
        );
        assert_eq!(s.get("baba", "leveltree"), Some("106level,0level"));
        assert_eq!(s.int("baba", "lastslot"), 2);
        s.set("baba", "lastslot", 1);
        s.set("x_prize", "total", 3);
        s.remove("baba", "firsttime");
        let t = s.to_text();
        assert_eq!(
            t,
            "[baba]\nleveltree=106level,0level\nlastslot=1\n\n[x_prize]\ntotal=3\n\n"
        );
        assert_eq!(Store::parse(&t), s);
        s.remove_group("x_prize");
        assert!(s.group("x_prize").is_none());
    }

    #[test]
    fn values_keep_equals_signs() {
        let s = Store::parse("[w]\n0data=sign_lang=a=b=c\n");
        assert_eq!(s.get("w", "0data"), Some("sign_lang=a=b=c"));
    }
}
