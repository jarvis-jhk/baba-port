//! Text rendering of a level state, for reasoning about puzzles by hand.

use std::collections::BTreeMap;
use std::fmt::Write;

use baba_sim::Level;

/// Renders the grid (one 4-character cell per tile: 3-letter code plus `+`
/// when more units share the tile), the stacks, a legend and the rules.
/// Text is upper case, objects lower case; `#` is the level border.
pub fn render(lvl: &Level) -> String {
    let d = lvl.defs();
    let (w, h) = (lvl.width(), lvl.height());
    let units = lvl.unit_records();
    // Codes per kind.
    let mut codes: BTreeMap<u16, String> = BTreeMap::new();
    let mut used: BTreeMap<String, u16> = BTreeMap::new();
    for u in &units {
        if codes.contains_key(&u.kind) {
            continue;
        }
        let k = d.kind(u.kind);
        let base: String = if k.is_text {
            k.word.to_uppercase()
        } else {
            k.name.to_lowercase()
        };
        let base: String = base.chars().filter(|c| c.is_alphanumeric()).collect();
        let mut c: String = base.chars().take(3).collect();
        if c.is_empty() {
            c = "?".into();
        }
        let mut n = 1;
        while used.contains_key(&c) {
            let stem: String = base.chars().take(2).collect();
            c = format!("{stem}{n}");
            n += 1;
        }
        used.insert(c.clone(), u.kind);
        codes.insert(u.kind, c);
    }
    let mut cells: BTreeMap<(i32, i32), Vec<usize>> = BTreeMap::new();
    for (i, u) in units.iter().enumerate() {
        cells.entry((u.x, u.y)).or_default().push(i);
    }
    let mut s = String::new();
    let _ = writeln!(
        s,
        "size {w}x{h} (border included; x right, y down; '|' before every 5th column)"
    );
    let mut header = String::from("    ");
    for x in 0..w {
        if x > 0 && x % 5 == 0 {
            header.push('|');
        }
        let _ = write!(header, "{x:<4}");
    }
    let _ = writeln!(s, "{header}");
    for y in 0..h {
        let _ = write!(s, "{y:>3} ");
        for x in 0..w {
            if x > 0 && x % 5 == 0 {
                s.push('|');
            }
            match cells.get(&(x, y)) {
                Some(v) => {
                    // Show text first, then the highest layer.
                    let top = v
                        .iter()
                        .max_by_key(|&&i| {
                            let k = d.kind(units[i].kind);
                            (k.is_text, k.layer)
                        })
                        .copied()
                        .unwrap();
                    let c = &codes[&units[top].kind];
                    let _ = write!(s, "{c:<3}{}", if v.len() > 1 { '+' } else { ' ' });
                }
                None => {
                    let border = x == 0 || y == 0 || x == w - 1 || y == h - 1;
                    s.push_str(if border { "#   " } else { ".   " });
                }
            }
        }
        let _ = writeln!(s, " {y}");
    }
    let _ = writeln!(s, "{header}");
    let stacks: Vec<_> = cells.iter().filter(|(_, v)| v.len() > 1).collect();
    if !stacks.is_empty() {
        s.push_str("stacks:");
        for ((x, y), v) in stacks {
            let names: Vec<&str> = v.iter().map(|&i| codes[&units[i].kind].as_str()).collect();
            let _ = write!(s, " ({x},{y})={}", names.join("/"));
        }
        s.push('\n');
    }
    s.push_str("legend:");
    for (k, c) in &codes {
        let kd = d.kind(*k);
        let n = units.iter().filter(|u| u.kind == *k).count();
        let _ = write!(s, " {c}={}", kd.name);
        if n > 1 {
            let _ = write!(s, "x{n}");
        }
    }
    s.push('\n');
    // Positions of every kind with few units.
    for (k, c) in &codes {
        let ps: Vec<String> = units
            .iter()
            .filter(|u| u.kind == *k)
            .map(|u| format!("{},{}", u.x, u.y))
            .collect();
        if ps.len() <= 12 {
            let _ = writeln!(s, "  {c:<4} {}", ps.join(" "));
        }
    }
    s.push_str("rules: ");
    s.push_str(&lvl.rule_strings().join("; "));
    s.push('\n');
    s
}

/// Facing of units of the given kind names (for the dump's footer).
pub fn facings(lvl: &Level, names: &[String]) -> String {
    let d = lvl.defs();
    let mut s = String::new();
    for u in lvl.unit_records() {
        let k = d.kind(u.kind);
        if names.contains(&k.name) {
            let _ = write!(
                s,
                "{}@{},{} dir {} ",
                k.name,
                u.x,
                u.y,
                ["R", "U", "L", "D", "-"][u.dir.min(4) as usize]
            );
        }
    }
    s
}
