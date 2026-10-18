//! Letters forming words (`docs/spec/words.md` §5).

use std::collections::{BTreeMap, BTreeSet};

use super::Build;
use crate::unit::Handle;

/// A word spelled by letter tiles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LetterWord {
    /// Effective name (`text_baba`), or the custom name (kind 8).
    pub name: String,
    pub kind: i32,
    pub x: i32,
    pub y: i32,
    /// 1 horizontal, 2 vertical.
    pub dir: u8,
    pub width: i32,
    pub units: Vec<Handle>,
}

/// Note names (keys of the note frequency table): the seven note letters
/// with optional sharp/flat and octave 4–6 suffixes, plus the special cases
/// `words.md` §5.4 lists by prefix.
fn is_note(w: &str) -> bool {
    if w.starts_with("bsharp")
        || w.starts_with("esharp")
        || w.starts_with("cflat")
        || w.starts_with("fflat")
    {
        return true;
    }
    let b = w.as_bytes();
    if b.is_empty() || !(b'a'..=b'g').contains(&b[0]) {
        return false;
    }
    let mut rest = &w[1..];
    if let Some(r) = rest.strip_prefix("sharp") {
        rest = r;
    } else if let Some(r) = rest.strip_prefix("flat") {
        rest = r;
    }
    matches!(rest, "" | "4" | "5" | "6")
}

struct Dict<'a> {
    words: &'a [String],
    customs: &'a BTreeSet<String>,
}

impl Dict<'_> {
    fn any_prefix(&self, w: &str) -> bool {
        let i = self.words.partition_point(|x| x.as_str() < w);
        self.words.get(i).is_some_and(|x| x.starts_with(w))
    }

    fn prefix_hit(&self, w: &str) -> bool {
        if w.len() <= 1 {
            return true;
        }
        self.any_prefix(w) || self.customs.iter().any(|c| c.starts_with(w))
    }

    fn branch_starts(&self, sub: &str) -> bool {
        self.any_prefix(sub) || self.customs.iter().any(|c| c.starts_with(sub))
    }
}

/// Builds the letter-word map into `b.letter_map`.
pub(crate) fn build_map(b: &mut Build<'_>) {
    let lvl = b.lvl;
    let defs = &*lvl.defs;
    let mut lettermap: BTreeMap<i32, Vec<(String, Handle)>> = BTreeMap::new();
    for &h in lvl.text_units() {
        let k = lvl.kind(h);
        if k.word_type != 5 || lvl.unit(h).dead {
            continue;
        }
        let u = lvl.unit(h);
        lettermap
            .entry(lvl.tile_index(u.x, u.y))
            .or_default()
            .push((k.word.clone(), h));
    }
    if lettermap.is_empty() {
        return;
    }
    let dict = Dict {
        words: &defs.dictionary,
        customs: &defs.customs,
    };
    let w = lvl.w;
    let mut db: Vec<LetterWord> = Vec::new();
    let tiles: Vec<i32> = lettermap.keys().copied().collect();
    for t in tiles {
        let (x, y) = (t % w, t / w);
        let has = |xx: i32, yy: i32| lvl.on_grid(xx, yy) && lettermap.contains_key(&(xx + yy * w));
        let custom_here = lettermap[&t].iter().any(|(s, _)| defs.customs.contains(s));
        let cont_r = has(x + 1, y) || custom_here;
        let cont_d = has(x, y + 1) || custom_here;
        if !has(x - 1, y) && cont_r {
            scan_line(b, &dict, &lettermap, x, y, 1, &mut db);
        }
        if !has(x, y - 1) && cont_d {
            scan_line(b, &dict, &lettermap, x, y, 2, &mut db);
        }
    }
    if defs.exists("text_play") {
        let mut longest: BTreeMap<(i32, i32, u8), usize> = BTreeMap::new();
        for e in &db {
            let wd = e.name.strip_prefix("text_").unwrap_or(&e.name);
            if is_note(wd) {
                let l = longest.entry((e.x, e.y, e.dir)).or_insert(0);
                *l = (*l).max(wd.len());
            }
        }
        db.retain(|e| {
            let wd = e.name.strip_prefix("text_").unwrap_or(&e.name);
            !is_note(wd) || wd.len() >= longest[&(e.x, e.y, e.dir)]
        });
    }
    for e in db {
        let start = lvl.tile_index(e.x, e.y);
        b.letter_map.entry(start).or_default().push(e.clone());
        if e.width > 1 {
            let (ox, oy) = if e.dir == 1 { (1, 0) } else { (0, 1) };
            let end = lvl.tile_index(e.x + ox * (e.width - 1), e.y + oy * (e.width - 1));
            b.letter_map.entry(end).or_default().push(e);
        }
    }
}

fn matches(b: &mut Build<'_>, dict: &Dict<'_>, w: &str) -> Vec<(String, i32)> {
    let defs = &*b.lvl.defs;
    let mut out = Vec::new();
    if w.len() >= 2 && dict.words.binary_search_by(|x| x.as_str().cmp(w)).is_ok() {
        let alt = format!("text_{w}");
        if let Some(k) = defs.active_lookup(&alt) {
            let kd = defs.kind(k);
            if (kd.name == w || kd.name == alt) && kd.word_type != 5 {
                if defs.active_lookup(w).is_some() || w == "all" || w == "empty" {
                    b.known.insert(w.to_string());
                }
                out.push((kd.name.clone(), kd.word_type));
            }
        }
    }
    if !w.is_empty() && defs.customs.contains(w) {
        out.push((w.to_string(), 8));
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn scan_line(
    b: &mut Build<'_>,
    dict: &Dict<'_>,
    lettermap: &BTreeMap<i32, Vec<(String, Handle)>>,
    x: i32,
    y: i32,
    dir: u8,
    db: &mut Vec<LetterWord>,
) {
    let (ox, oy) = if dir == 1 { (1, 0) } else { (0, 1) };
    let w = b.lvl.w;
    let mut stacks: Vec<&Vec<(String, Handle)>> = Vec::new();
    let (mut cx, mut cy) = (x, y);
    while b.lvl.on_grid(cx, cy) {
        match lettermap.get(&(cx + cy * w)) {
            Some(s) => stacks.push(s),
            None => break,
        }
        cx += ox;
        cy += oy;
    }
    let total: usize = stacks
        .iter()
        .map(|s| s.len())
        .try_fold(1usize, |a, n| a.checked_mul(n).filter(|&v| v <= 5000))
        .unwrap_or(5000);
    let mut combo = vec![0usize; stacks.len()];
    let mut seen: BTreeSet<(String, i32)> = BTreeSet::new();
    let mut record =
        |name: String, kind: i32, pos: i32, units: Vec<Handle>, db: &mut Vec<LetterWord>| {
            if seen.insert((name.clone(), pos)) {
                db.push(LetterWord {
                    name,
                    kind,
                    x: x + ox * pos,
                    y: y + oy * pos,
                    dir,
                    width: units.len() as i32,
                    units,
                });
            }
        };
    for _ in 0..total {
        let letters: Vec<(String, Handle)> = stacks
            .iter()
            .zip(&combo)
            .map(|(s, &c)| s[c].clone())
            .collect();
        // odometer, first position fastest
        for (i, s) in stacks.iter().enumerate() {
            combo[i] += 1;
            if combo[i] >= s.len() {
                combo[i] = 0;
            } else {
                break;
            }
        }
        let mut main = String::new();
        let mut main_units: Vec<Handle> = Vec::new();
        let mut offset = 0i32;
        let mut prev: Option<(String, Handle)> = None;
        // branches: (string, start, dead, units)
        let mut branches: Vec<(String, i32, bool, Vec<Handle>)> = Vec::new();
        for (p, (s, u)) in letters.iter().enumerate() {
            let p = p as i32;
            main.push_str(s);
            main_units.push(*u);
            if p >= 1
                && main.len() >= 2
                && let Some((ps, pu)) = &prev
            {
                let sub = format!("{ps}{s}");
                if dict.branch_starts(&sub)
                    && !branches
                        .iter()
                        .any(|br| br.1 == p - 1 && br.3.first() == Some(pu))
                {
                    branches.push((ps.clone(), p - 1, false, vec![*pu]));
                }
            }
            prev = Some((s.clone(), *u));
            for br in branches.iter_mut() {
                let (dead, start) = (br.2, br.1);
                br.0.push_str(s);
                br.3.push(*u);
                if dead {
                    continue;
                }
                let bs = br.0.clone();
                if !dict.prefix_hit(&bs) {
                    br.2 = true;
                    br.3.pop();
                } else {
                    for (name, kind) in matches(b, dict, &bs) {
                        record(name, kind, start, br.3.clone(), db);
                    }
                }
            }
            if !dict.prefix_hit(&main) {
                // Reset to the last *character* (matters for SHARP/FLAT;
                // the reference behaviour, see the spec note in the report).
                main = main.chars().last().map(String::from).unwrap_or_default();
                main_units = vec![*u];
                offset = p;
            } else {
                for (name, kind) in matches(b, dict, &main) {
                    record(name, kind, offset, main_units.clone(), db);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_note;

    #[test]
    fn notes() {
        assert!(is_note("c"));
        assert!(is_note("csharp5"));
        assert!(is_note("bsharp"));
        assert!(!is_note("baba"));
        assert!(!is_note("h"));
    }
}
