//! The status pass (`docs/spec/properties.md` §3): BROKEN cache, direction
//! properties, TURN/DETURN, BACK start/stop, FLOAT; plus level rotation
//! (§5.6).

use crate::conditions::Subj;
use crate::level::Level;
use crate::turn::Pending;
use crate::undo::Line;
use crate::unit::Handle;

/// Status pass mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Start of the move phase: no direction properties, no TURN.
    NoTurn,
    /// End-of-turn step 3 and creation.
    Full,
    /// After an undo: FLOAT only, silently.
    Undoing,
}

/// Status pass for every unit.
pub(crate) fn status_pass(lvl: &mut Level, mode: Mode) {
    lvl.auto_count = lvl.count("level", "is", "auto", Subj::Level) as u32;
    let units = lvl.order.clone();
    if mode != Mode::Undoing {
        for &h in &units {
            broken_update(lvl, h);
        }
    }
    for h in units {
        if lvl.alive(h) {
            unit_status(lvl, h, mode);
        }
    }
}

/// Status pass for one freshly registered unit (`turn.md` §2.2 step 3).
pub(crate) fn status_pass_unit(lvl: &mut Level, h: Handle, undoing: bool) {
    if lvl.rules.features.is_empty() {
        return;
    }
    if undoing {
        unit_status(lvl, h, Mode::Undoing);
    } else {
        broken_update(lvl, h);
        unit_status(lvl, h, Mode::Full);
    }
}

fn broken_update(lvl: &mut Level, h: Handle) {
    if !lvl.alive(h) {
        return;
    }
    let b = lvl.rules.index.contains_key("broken") && {
        let name = lvl.uname(h).to_string();
        crate::conditions::has_g(
            lvl,
            &name,
            "is",
            "broken",
            Subj::Unit(h),
            &Default::default(),
            crate::conditions::Opts {
                ignore_broken: true,
                ..Default::default()
            },
        )
    };
    let u = lvl.unit(h);
    if u.broken != b {
        let id = u.id;
        lvl.line(Line::Broken { id, new_state: b });
        lvl.unit_mut(h).broken = b;
    }
}

fn unit_status(lvl: &mut Level, h: Handle, mode: Mode) {
    let rn = lvl.rname(h).to_string();
    let subj = Subj::Unit(h);
    if mode != Mode::Undoing {
        if mode == Mode::Full {
            let r = lvl.count(&rn, "is", "right", subj);
            let up = lvl.count(&rn, "is", "up", subj);
            let l = lvl.count(&rn, "is", "left", subj);
            let d = lvl.count(&rn, "is", "down", subj);
            let f = lvl.unit(h).dir;
            if let Some(nd) = facing_from_counts(f, r, up, l, d) {
                lvl.set_dir(h, nd);
            }
            let t = lvl.count(&rn, "is", "turn", subj) as i32;
            let s = lvl.count(&rn, "is", "deturn", subj) as i32;
            let n = t - s;
            if n != 0 {
                let f = lvl.unit(h).dir as i32;
                let nd = (f - n).rem_euclid(4) as u8;
                lvl.set_dir(h, nd);
            }
        }
        // BACK start/stop.
        let any_back = lvl.any_is("back");
        let (id, bs) = {
            let u = lvl.unit(h);
            (u.id, u.back_start)
        };
        if any_back {
            let isback = lvl.prop(h, "back");
            if isback && bs == 0 {
                lvl.line(Line::BackStart { id, old: 0 });
                lvl.unit_mut(h).back_start = lvl.history.len() as u32 + 2;
            } else if !isback && bs != 0 {
                lvl.line(Line::BackStart { id, old: bs });
                lvl.unit_mut(h).back_start = 0;
            }
        } else if bs != 0 {
            lvl.line(Line::BackStart { id, old: bs });
            lvl.unit_mut(h).back_start = 0;
        }
    }
    // FLOAT.
    let old = lvl.unit(h).float;
    if old == 2 {
        return;
    }
    let nf = lvl.prop(h, "float") as u8;
    if nf != old {
        lvl.unit_mut(h).float = nf;
        if mode != Mode::Undoing && !lvl.ending_running {
            lvl.scratch.pending.push(Pending::Float { h, old });
        }
    }
}

/// Facing from direction-property counts (§3 tie table).
pub(crate) fn facing_from_counts(f: u8, r: usize, up: usize, l: usize, d: usize) -> Option<u8> {
    let c = [r, up, l, d];
    for i in 0..4 {
        if (0..4).all(|j| j == i || c[i] > c[j]) {
            return Some(i as u8);
        }
    }
    if c.iter().all(|&x| x == 0) {
        return None;
    }
    match f {
        3 => {
            if l > 0 && l >= up && l >= r {
                Some(2)
            } else if up > 0 && up > l && up >= r {
                Some(1)
            } else if r > 0 && r > l && r > up {
                Some(0)
            } else {
                None
            }
        }
        2 => {
            if up > 0 && up >= r && up >= l {
                Some(1)
            } else if r > 0 && r > up && r >= d {
                Some(0)
            } else if d > 0 && d > r && d > up {
                Some(3)
            } else {
                None
            }
        }
        1 => {
            if r > 0 && r >= l && r >= d {
                Some(0)
            } else if d > 0 && d > r && d >= l {
                Some(3)
            } else if l > 0 && l > r && l > d {
                Some(2)
            } else {
                None
            }
        }
        _ => {
            if d > 0 && d >= l && d >= up {
                Some(3)
            } else if l > 0 && l > d && l >= up {
                Some(2)
            } else if up > 0 && up > d && up > l {
                Some(1)
            } else {
                None
            }
        }
    }
}

/// LEVEL IS RIGHT/UP/LEFT/DOWN rotation (§5.6). `record`: undo lines.
pub(crate) fn level_rotation(lvl: &mut Level, record: bool) {
    let rules: Vec<(String, Vec<crate::rules::Cond>, u32)> = lvl
        .rules
        .list("level")
        .iter()
        .map(|&ri| &lvl.rules.features[ri])
        .filter(|r| r.subject == "level" && r.verb == "is")
        .map(|r| (r.object.clone(), r.conds.clone(), r.cid))
        .collect();
    for (o, conds, cid) in rules {
        let (d, rot) = match o.as_str() {
            "right" => (0u8, 90),
            "up" => (1, 180),
            "left" => (2, 270),
            "down" => (3, 0),
            _ => continue,
        };
        if !lvl.holds(&conds, cid, Subj::Level) {
            continue;
        }
        if record {
            if lvl.mapdir != d {
                lvl.line(Line::MapDir { old: lvl.mapdir });
                lvl.line(Line::MapRotation { old: lvl.rotation });
                lvl.mapdir = d;
                lvl.rotation = rot;
                lvl.turn_changed = true;
            }
        } else {
            lvl.mapdir = d;
            lvl.rotation = rot;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::facing_from_counts;

    #[test]
    fn right_and_down_alternate() {
        // facing right with RIGHT and DOWN → down; facing down → right.
        assert_eq!(facing_from_counts(0, 1, 0, 0, 1), Some(3));
        assert_eq!(facing_from_counts(3, 1, 0, 0, 1), Some(0));
        assert_eq!(facing_from_counts(1, 0, 2, 0, 0), Some(1));
    }
}
