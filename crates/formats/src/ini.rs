//! The INI dialect used by `.ld` level files, `world_data.txt`, themes,
//! language files, saves and settings.
//!
//! Rules (see `docs/formats/levels.md`):
//! - `[section]` headers; `key=value` lines split at the first `=`.
//! - Keys are trimmed; values are kept verbatim (trailing spaces matter in a
//!   few places) except for the line terminator (`\r\n`, `\n` or `\r`).
//! - There is no comment syntax: a line such as `--particles=clouds` is just
//!   a key named `--particles`. Lines without `=` are ignored.
//! - A section name may appear more than once (seen in saves). Lookups treat
//!   all occurrences as one section and the last value of a key wins.
//! - Keys before the first header belong to a section with an empty name.

use std::collections::HashMap;
use std::path::Path;

use crate::error::{Result, read_text};

/// One `[section]` block as it appears in the file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Section {
    /// Name between the brackets.
    pub name: String,
    /// `key=value` pairs in file order.
    pub entries: Vec<(String, String)>,
}

impl Section {
    /// Last value of `key` in this block.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// A parsed INI document. Keeps every block in file order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ini {
    /// Blocks in file order (a name may repeat).
    pub sections: Vec<Section>,
}

impl Ini {
    /// Parses INI text. Never fails: unknown lines are skipped.
    pub fn parse(text: &str) -> Ini {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut sections: Vec<Section> = Vec::new();
        // Line ends may be `\r\n`, `\n` or a lone `\r` (one language file).
        for line in text.split(['\n', '\r']) {
            let trimmed = line.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') && !trimmed.contains('=') {
                sections.push(Section {
                    name: trimmed[1..trimmed.len() - 1].to_string(),
                    entries: Vec::new(),
                });
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                if sections.is_empty() {
                    sections.push(Section::default());
                }
                let sec = sections.last_mut().expect("pushed above");
                sec.entries.push((k.trim().to_string(), v.to_string()));
            }
        }
        Ini { sections }
    }

    /// Reads and parses a file.
    pub fn load(path: &Path) -> Result<Ini> {
        Ok(Ini::parse(&read_text(path)?))
    }

    /// True if at least one block has this name.
    pub fn has_section(&self, name: &str) -> bool {
        self.sections.iter().any(|s| s.name == name)
    }

    /// Distinct section names in order of first appearance.
    pub fn section_names(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for s in &self.sections {
            if !out.contains(&s.name.as_str()) {
                out.push(&s.name);
            }
        }
        out
    }

    /// Value of `key` in `section` (all blocks of that name merged, last wins).
    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.sections
            .iter()
            .rev()
            .filter(|s| s.name == section)
            .find_map(|s| s.get(key))
    }

    /// Parses the value with [`std::str::FromStr`]; `None` if absent or invalid.
    /// Surrounding whitespace is ignored.
    pub fn get_parsed<T: std::str::FromStr>(&self, section: &str, key: &str) -> Option<T> {
        self.get(section, key)?.trim().parse().ok()
    }

    /// Integer value; accepts a trailing fraction (`"3.0"`) like the game does.
    pub fn get_int(&self, section: &str, key: &str) -> Option<i64> {
        parse_int(self.get(section, key)?)
    }

    /// All entries of `section`, blocks merged: each key appears once, at the
    /// position of its first occurrence, with its last value.
    pub fn entries(&self, section: &str) -> Vec<(&str, &str)> {
        let mut out: Vec<(&str, &str)> = Vec::new();
        let mut pos: HashMap<&str, usize> = HashMap::new();
        for s in self.sections.iter().filter(|s| s.name == section) {
            for (k, v) in &s.entries {
                match pos.get(k.as_str()) {
                    Some(&i) => out[i].1 = v,
                    None => {
                        pos.insert(k, out.len());
                        out.push((k, v));
                    }
                }
            }
        }
        out
    }

    /// `section` as a key → value map (blocks merged, last value wins); for
    /// many lookups in one section.
    pub fn section_map(&self, section: &str) -> HashMap<&str, &str> {
        let mut m = HashMap::new();
        for s in self.sections.iter().filter(|s| s.name == section) {
            for (k, v) in &s.entries {
                m.insert(k.as_str(), v.as_str());
            }
        }
        m
    }

    /// Sets `key` in `section` (the last block of that name; the section is
    /// appended if missing). An existing key keeps its position.
    pub fn set(&mut self, section: &str, key: &str, value: impl ToString) {
        let value = value.to_string();
        if let Some(sec) = self.sections.iter_mut().rev().find(|s| s.name == section) {
            if let Some(e) = sec.entries.iter_mut().rev().find(|(k, _)| k == key) {
                e.1 = value;
            } else {
                sec.entries.push((key.to_string(), value));
            }
            return;
        }
        self.sections.push(Section {
            name: section.to_string(),
            entries: vec![(key.to_string(), value)],
        });
    }

    /// Removes `key` from every block of `section`.
    pub fn remove(&mut self, section: &str, key: &str) {
        for s in self.sections.iter_mut().filter(|s| s.name == section) {
            s.entries.retain(|(k, _)| k != key);
        }
    }

    /// Replaces all blocks of `section` by one block with `entries` (at the
    /// position of the first block, else appended).
    pub fn replace_section(&mut self, section: &str, entries: Vec<(String, String)>) {
        let pos = self.sections.iter().position(|s| s.name == section);
        self.sections.retain(|s| s.name != section);
        let sec = Section {
            name: section.to_string(),
            entries,
        };
        match pos {
            Some(p) => self.sections.insert(p.min(self.sections.len()), sec),
            None => self.sections.push(sec),
        }
    }

    /// Serialises back to text (`\r\n` line ends, as the game writes).
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        for sec in &self.sections {
            if !(sec.name.is_empty() && std::ptr::eq(sec, &self.sections[0])) {
                s.push_str(&format!("[{}]\r\n", sec.name));
            }
            for (k, v) in &sec.entries {
                s.push_str(&format!("{k}={v}\r\n"));
            }
        }
        s
    }
}

/// Parses an integer the lenient way (`"12"`, `" 12 "`, `"12.0"`, `"-1"`).
pub fn parse_int(s: &str) -> Option<i64> {
    let s = s.trim();
    if let Ok(v) = s.parse::<i64>() {
        return Some(v);
    }
    let f: f64 = s.parse().ok()?;
    f.is_finite().then_some(f as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_and_merge() {
        let ini = Ini::parse(
            "\u{feff}[general]\r\nname=a = b\r\ndifficulty=^ ^ \r\n--particles=x\r\njunk\r\n[s]\nk=1\n[general]\rname=c\r",
        );
        assert_eq!(ini.get("general", "name"), Some("c"));
        assert_eq!(ini.get("general", "difficulty"), Some("^ ^ "));
        assert_eq!(ini.get("general", "--particles"), Some("x"));
        assert_eq!(ini.get_int("s", "k"), Some(1));
        assert_eq!(ini.section_names(), vec!["general", "s"]);
        let e = ini.entries("general");
        assert_eq!(e[0], ("name", "c"));
        assert_eq!(e.len(), 3);
    }

    #[test]
    fn roundtrip() {
        let ini = Ini::parse("[a]\r\nx=1\r\n[b]\r\ny=\r\n");
        assert_eq!(Ini::parse(&ini.to_text()), ini);
    }

    #[test]
    fn lenient_int() {
        assert_eq!(parse_int(" 3.0"), Some(3));
        assert_eq!(parse_int("-1"), Some(-1));
        assert_eq!(parse_int("x"), None);
    }
}
