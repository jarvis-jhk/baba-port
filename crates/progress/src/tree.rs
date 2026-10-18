//! The level tree (root map → … → current level), level ids and the
//! "Level X" label (`docs/spec/progression.md` §6).

/// Placeholder id of the root and of maps without an id.
pub const DUMMY: &str = "dummy";
/// "No id" marker.
pub const EMPTY: &str = "<empty>";

/// One entry: level stem and its id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub stem: String,
    pub id: String,
}

/// The chain from the world root to the current level.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LevelTree {
    pub nodes: Vec<Node>,
}

impl LevelTree {
    pub fn root(stem: &str) -> Self {
        Self {
            nodes: vec![Node {
                stem: stem.to_string(),
                id: DUMMY.into(),
            }],
        }
    }

    pub fn from_parts(stems: &[String], ids: &[String]) -> Self {
        Self {
            nodes: stems
                .iter()
                .enumerate()
                .map(|(i, s)| Node {
                    stem: s.clone(),
                    id: ids.get(i).cloned().unwrap_or_else(|| DUMMY.into()),
                })
                .collect(),
        }
    }

    pub fn stems(&self) -> Vec<String> {
        self.nodes.iter().map(|n| n.stem.clone()).collect()
    }

    pub fn ids(&self) -> Vec<String> {
        self.nodes.iter().map(|n| n.id.clone()).collect()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn current(&self) -> Option<&str> {
        self.nodes.last().map(|n| n.stem.as_str())
    }

    pub fn parent(&self) -> Option<&str> {
        let n = self.nodes.len();
        (n >= 2).then(|| self.nodes[n - 2].stem.as_str())
    }

    pub fn contains(&self, stem: &str) -> bool {
        self.nodes.iter().any(|n| n.stem == stem)
    }

    /// §6.3 Enter: push, or cut back to an ancestor (keeping its old id).
    pub fn enter(&mut self, stem: &str, id: &str) {
        if let Some(i) = self.nodes.iter().position(|n| n.stem == stem) {
            self.nodes.truncate(i + 1);
        } else {
            self.nodes.push(Node {
                stem: stem.to_string(),
                id: id.to_string(),
            });
        }
    }

    /// §6.3 Leave: pop and return the level left.
    pub fn leave(&mut self) -> Option<Node> {
        if self.nodes.len() > 1 {
            self.nodes.pop()
        } else {
            None
        }
    }

    /// §6.3 Change: replace the top.
    pub fn change(&mut self, stem: &str, id: &str) {
        if let Some(top) = self.nodes.last_mut() {
            top.stem = stem.to_string();
            top.id = id.to_string();
        }
    }

    /// The text after the word "Level" (§6.2); `None` = blank label.
    pub fn label_suffix(&self) -> Option<String> {
        let n = self.nodes.len();
        let id = &self.nodes.last()?.id;
        if id == EMPTY {
            return None;
        }
        let special = |s: &str| s == "???" || s == "baba";
        let empty = |s: &str| s.is_empty() || s == EMPTY || s == DUMMY;
        let out = match n {
            0 => return None,
            1 => {
                if special(id) {
                    id.clone()
                } else {
                    return None;
                }
            }
            2 => {
                let p = &self.nodes[0].id;
                if special(p) {
                    format!("{p}-{id}")
                } else {
                    id.clone()
                }
            }
            _ => {
                let p = &self.nodes[n - 2].id;
                if empty(p) {
                    id.clone()
                } else {
                    format!("{p}-{id}")
                }
            }
        };
        (!out.is_empty() && out != DUMMY).then_some(out)
    }
}

/// The id an icon gives its level (§6.1): the target's `mapid` (or its
/// language override) wins; else by symbol style: -1 → the map id, 0 →
/// the number, 1 → a letter, 2 → "extra N+1" (`extra` = localised word).
pub fn level_id(mapid: Option<&str>, style: i32, number: i32, extra: &str) -> String {
    if let Some(m) = mapid.filter(|m| !m.is_empty()) {
        return m.to_string();
    }
    match style {
        0 => number.to_string(),
        1 => char::from(b'a' + number.clamp(0, 25) as u8).to_string(),
        2 => format!("{extra} {}", number + 1),
        _ => DUMMY.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        let mut t = LevelTree::root("106level");
        t.enter("0level", "0");
        assert_eq!(t.label_suffix().as_deref(), Some("0"));
        t.leave();
        t.enter("177level", "lake");
        t.enter("180level", "3");
        assert_eq!(t.label_suffix().as_deref(), Some("lake-3"));
        let mut q = LevelTree::root("200level");
        q.nodes[0].id = "???".into();
        q.enter("201level", "3");
        assert_eq!(q.label_suffix().as_deref(), Some("???-3"));
        let mut e = LevelTree::root("85level");
        e.enter("79level", EMPTY);
        assert_eq!(e.label_suffix(), None);
    }

    #[test]
    fn reenter_ancestor_cuts() {
        let mut t = LevelTree::root("106level");
        t.enter("283level", "meta");
        t.enter("106level", "x");
        assert_eq!(t.stems(), vec!["106level"]);
        assert_eq!(t.nodes[0].id, DUMMY);
    }

    #[test]
    fn ids() {
        assert_eq!(level_id(None, 0, 7, "extra"), "7");
        assert_eq!(level_id(None, 1, 2, "extra"), "c");
        assert_eq!(level_id(None, 2, 10, "extra"), "extra 11");
        assert_eq!(level_id(Some("lake"), 0, 7, "extra"), "lake");
    }
}
