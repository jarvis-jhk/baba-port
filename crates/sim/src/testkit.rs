//! Synthetic levels for tests and tools: an ASCII grid notation and a
//! minimal word-kind table (`docs/spec/words.md` §3), so scenarios can be
//! written without the game's data.
//!
//! Notation: rows of space-separated cells; `.` empty; CAPS = text word
//! (`BABA` → `text_baba`); lowercase = object; `l:x` = letter tile `x`;
//! `A/B` = stack (A first); an object may end in `>` `^` `<` `v` for its
//! facing. The grid gets a one-tile border ring automatically.

use std::sync::Arc;

use crate::defs::{Defs, SimpleDef};
use crate::level::{Context, Level};
use crate::load::{Placed, Source};

const VERBS: &[&str] = &[
    "is", "has", "make", "eat", "fear", "follow", "mimic", "play", "write", "become",
];
const PREFIX: &[&str] = &[
    "lonely", "idle", "often", "seldom", "powered", "powered2", "powered3",
];
const INFIX: &[&str] = &[
    "on",
    "near",
    "nextto",
    "facing",
    "facedby",
    "seeing",
    "without",
    "above",
    "below",
    "besideleft",
    "besideright",
    "feeling",
];
const NOUNS: &[&str] = &[
    "baba", "keke", "rock", "wall", "flag", "water", "skull", "lava", "key", "door", "tile",
    "grass", "box", "belt", "pipe", "love", "star", "moon", "me", "jelly", "crab", "bomb", "cog",
    "fungus", "text", "all", "empty", "level", "group", "group2", "group3", "cursor", "image",
    "bog", "ice", "hedge", "tree", "bug", "bat", "statue", "pillar", "fence", "cliff", "brick",
    "algae", "robot",
];

fn word_type(w: &str) -> i32 {
    if VERBS.contains(&w) {
        1
    } else if PREFIX.contains(&w) {
        3
    } else if w == "not" {
        4
    } else if w == "and" {
        6
    } else if INFIX.contains(&w) {
        7
    } else if NOUNS.contains(&w) {
        0
    } else if w.len() == 1 || w == "sharp" || w == "flat" {
        5
    } else {
        2
    }
}

fn def_for(name: &str) -> SimpleDef {
    if let Some(w) = name.strip_prefix("text_") {
        let wt = word_type(w);
        let (argtype, argextra) = match w {
            "is" | "write" => (vec![0, 2], vec![]),
            "play" => (vec![8], vec![]),
            "feeling" => (vec![2], vec![]),
            "facing" => (
                vec![0],
                vec!["right".into(), "up".into(), "left".into(), "down".into()],
            ),
            _ => (vec![], vec![]),
        };
        let customobjects = if w == "play" {
            ["c", "d", "e", "f", "g", "a", "b", "csharp", "cflat"]
                .iter()
                .map(|s| s.to_string())
                .collect()
        } else {
            vec![]
        };
        SimpleDef {
            name: name.to_string(),
            word_type: wt,
            argtype,
            argextra,
            customobjects,
        }
    } else {
        SimpleDef {
            name: name.to_string(),
            word_type: 0,
            argtype: vec![],
            argextra: vec![],
            customobjects: vec![],
        }
    }
}

/// Builds a level from a grid. `extra` names are added to the
/// definitions even if not on the grid (e.g. conversion targets).
pub fn level(rows: &[&str], extra: &[&str]) -> Level {
    level_in(rows, extra, "test", true)
}

/// As [`level`], with a world name and base-world flag.
pub fn level_in(rows: &[&str], extra: &[&str], world: &str, base: bool) -> Level {
    let mut names: Vec<String> = Vec::new();
    let mut cells: Vec<Vec<Vec<(String, u8)>>> = Vec::new();
    for r in rows {
        let mut row = Vec::new();
        for c in r.split_whitespace() {
            let mut stack = Vec::new();
            if c != "." {
                for part in c.split('/') {
                    let (body, dir) = match part.chars().last() {
                        Some('>') => (&part[..part.len() - 1], 0u8),
                        Some('^') => (&part[..part.len() - 1], 1),
                        Some('<') => (&part[..part.len() - 1], 2),
                        Some('v')
                            if part.len() > 1
                                && part.chars().next().is_some_and(|c| c.is_lowercase())
                                && part != "v" =>
                        {
                            (&part[..part.len() - 1], 3)
                        }
                        _ => (part, 0),
                    };
                    let name = if let Some(l) = body.strip_prefix("l:") {
                        format!("text_{l}")
                    } else if body
                        .chars()
                        .find(|c| !c.is_ascii_digit())
                        .is_some_and(|c| c.is_uppercase())
                    {
                        format!("text_{}", body.to_lowercase())
                    } else {
                        body.to_string()
                    };
                    stack.push((name, dir));
                }
            }
            row.push(stack);
        }
        cells.push(row);
    }
    for row in &cells {
        for s in row {
            for (n, _) in s {
                if !names.contains(n) {
                    names.push(n.clone());
                }
            }
        }
    }
    for e in extra {
        if !names.iter().any(|n| n == e) {
            names.push(e.to_string());
        }
    }
    // Every noun word on the grid also defines its object.
    let mut more = Vec::new();
    for n in &names {
        if let Some(w) = n.strip_prefix("text_")
            && word_type(w) == 0
            && !matches!(
                w,
                "text" | "all" | "empty" | "level" | "group" | "group2" | "group3"
            )
            && !names.iter().any(|m| m == w)
            && !more.contains(&w.to_string())
        {
            more.push(w.to_string());
        }
    }
    names.extend(more);
    let defs: Vec<SimpleDef> = names.iter().map(|n| def_for(n)).collect();
    let d = Defs::synthetic(&defs, world, "testlevel", base);
    let h = cells.len() as i32 + 2;
    let w = cells.iter().map(|r| r.len()).max().unwrap_or(0) as i32 + 2;
    // Load order: x outer, y inner, stack order.
    let mut placed = Vec::new();
    for x in 0..w - 2 {
        for (y, row) in cells.iter().enumerate() {
            if let Some(stack) = row.get(x as usize) {
                for (n, dir) in stack {
                    placed.push(Placed {
                        kind: d.kind_by_name(n).expect("defined"),
                        x: x + 1,
                        y: y as i32 + 1,
                        dir: *dir,
                        icon: None,
                    });
                }
            }
        }
    }
    let src = Source::synthetic(d, w, h, placed);
    Level::start(Arc::new(src), Context::default())
}

impl Level {
    /// Units named `name` as (x, y, dir), in unit-list order.
    pub fn find_units(&self, name: &str) -> Vec<(i32, i32, u8)> {
        self.units()
            .into_iter()
            .filter(|u| u.name == name)
            .map(|u| (u.x, u.y, u.dir))
            .collect()
    }

    /// Plays a corpus input string (R U L D W Z r).
    pub fn play(&mut self, inputs: &str) -> Vec<crate::TurnOutcome> {
        inputs
            .chars()
            .filter_map(crate::Input::from_char)
            .map(|i| self.step(i))
            .collect()
    }

    /// Rule display strings without the base rules.
    pub fn plain_rules(&self) -> Vec<String> {
        self.rules
            .features
            .iter()
            .filter(|r| !r.tags.iter().any(|t| t == "base"))
            .map(|r| r.display())
            .collect()
    }
}
