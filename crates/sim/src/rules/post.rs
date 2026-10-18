//! ALL expansion, MIMIC (`rules.md` §13), and post-processing: active
//! text, NOT application and X IS X protection (§15).

use std::collections::{BTreeMap, BTreeSet};

use super::{Build, Cond};
use crate::unit::Handle;

/// `invert(conds)` (§15.2). `target` = the NOT rule's object (FEELING
/// special). Returns (inverted, crashy, never_found).
pub(crate) fn invert(conds: &[Cond], target: Option<&str>) -> (Vec<Cond>, bool, bool) {
    if conds.is_empty() {
        return (vec![Cond::never()], false, true);
    }
    let mut out = Vec::new();
    let mut crash = false;
    for c in conds {
        let (name, base) = if let Some(r) = c.name.strip_prefix("(not ") {
            (r.to_string(), r.to_string())
        } else if c.name.starts_with("(not") {
            let r = c.name.get(5..).unwrap_or("").to_string();
            (r.clone(), r)
        } else if let Some(r) = c.name.strip_prefix("not ") {
            (r.to_string(), r.to_string())
        } else {
            (format!("not {}", c.name), c.name.clone())
        };
        let mut params = Vec::new();
        for p in &c.params {
            if base != "feeling" || target.is_none_or(|t| t != format!("not {p}")) {
                params.push(p.clone());
            } else if c.name == "feeling" {
                crash = true;
            }
        }
        if !c.params.is_empty() && params.is_empty() {
            continue;
        }
        out.push(Cond { name, params });
    }
    if let Some(first) = out.first_mut() {
        first.name = format!("({}", first.name);
    }
    if let Some(last) = out.last_mut() {
        last.name.push(')');
    }
    (out, crash, false)
}

/// ALL and MIMIC (§13).
pub(crate) fn all_and_mimic(b: &mut Build<'_>) {
    // §13.1 ALL: iterate the growing `all` list.
    let mut k = 0;
    while k < b.rs.list("all").len() {
        let ri = b.rs.list("all")[k];
        k += 1;
        let r = b.rs.features[ri].clone();
        let notall: Vec<usize> = b.rs.list("not all").to_vec();
        if r.object == "all" && r.verb != "is" {
            let mut extra = Vec::new();
            for &pi in &notall {
                let p = &b.rs.features[pi];
                if p.subject == r.subject && p.verb == r.verb && p.object == "not all" {
                    extra.extend(invert(&p.conds, None).0);
                }
            }
            for n in b.known_plain() {
                let mut c = r.conds.clone();
                c.extend(extra.iter().cloned());
                b.add(
                    &r.subject,
                    &r.verb,
                    &n,
                    c,
                    r.ids.clone(),
                    false,
                    None,
                    r.tags.clone(),
                );
            }
        }
        if r.subject == "all" && !r.object.starts_with("not ") {
            let mut extra = Vec::new();
            let target = format!("not {}", r.object);
            for &pi in &notall {
                let p = &b.rs.features[pi];
                if p.subject == r.subject && p.verb == r.verb && p.object == target {
                    extra.extend(invert(&p.conds, None).0);
                }
            }
            for n in b.known_plain() {
                let mut c = r.conds.clone();
                c.extend(extra.iter().cloned());
                b.add(
                    &n,
                    &r.verb,
                    &r.object,
                    c,
                    r.ids.clone(),
                    false,
                    None,
                    r.tags.clone(),
                );
            }
        }
        if r.subject == "all" && r.object.starts_with("not ") {
            for n in b.known_plain() {
                b.add(
                    &n,
                    &r.verb,
                    &r.object,
                    r.conds.clone(),
                    r.ids.clone(),
                    false,
                    None,
                    r.tags.clone(),
                );
            }
        }
    }

    // §13.2 MIMIC.
    let mimic: Vec<usize> = b.rs.list("mimic").to_vec();
    let mut protects: BTreeMap<String, Vec<(String, Vec<Cond>)>> = BTreeMap::new();
    for &i in &mimic {
        let r = &b.rs.features[i];
        if r.verb == "mimic"
            && let Some(t) = r.object.strip_prefix("not ")
        {
            protects
                .entry(r.subject.clone())
                .or_default()
                .push((t.to_string(), r.conds.clone()));
        }
    }
    let mut limiter = 0;
    for (m_idx, &i) in mimic.iter().enumerate() {
        let r = b.rs.features[i].clone();
        if r.verb != "mimic" || r.object.starts_with("not ") {
            continue;
        }
        let (o, t) = (r.subject.clone(), r.object.clone());
        let mut valid = true;
        let mut extra = Vec::new();
        for (pt, pc) in protects.get(&o).into_iter().flatten() {
            if *pt == t {
                if pc.is_empty() {
                    valid = false;
                } else {
                    extra.extend(invert(pc, None).0);
                }
            }
        }
        if !valid || !b.known.contains(&o) || !b.known.contains(&t) {
            continue;
        }
        let mut j = 0;
        while j < b.rs.list(&t).len() {
            let bi = b.rs.list(&t)[j];
            j += 1;
            let br = b.rs.features[bi].clone();
            if br.subject != t || br.verb == "mimic" || br.tags.iter().any(|x| x == "mimic") {
                continue;
            }
            let mut c = br.conds.clone();
            c.extend(r.conds.iter().cloned());
            c.extend(extra.iter().cloned());
            let mut tags = br.tags.clone();
            tags.extend(r.tags.iter().cloned());
            tags.push(format!("mimicparent_{}", m_idx + 1));
            tags.push("mimic".into());
            limiter += 1;
            b.add(
                &o,
                &br.verb,
                &br.object,
                c,
                br.ids.clone(),
                true,
                None,
                tags,
            );
            if limiter > 250 {
                b.destroy(crate::events::DestroyStyle::TooComplex);
                return;
            }
        }
    }
}

/// §15: active text, NOT application, X IS X. Returns the units that are
/// part of rules (for the active colour and new-rule effects).
pub(crate) fn postprocess(b: &mut Build<'_>) -> BTreeSet<Handle> {
    let mut active = BTreeSet::new();
    let limit = b.rs.features.len();
    let mut protects = Vec::new();
    let mut i = 0;
    while i < limit {
        let r = b.rs.features[i].clone();
        if r.subject == r.object && r.verb == "is" {
            protects.push(i);
        }
        for l in &r.ids {
            for &u in l {
                active.insert(u);
            }
        }
        if let Some(neweffect) = r.object.strip_prefix("not ") {
            let (inv, crashy, _) = invert(&r.conds, Some(&r.object));
            let mut lists: Vec<(Vec<usize>, bool)> = vec![(b.rs.list(&r.subject).to_vec(), false)];
            if r.verb == "is" && neweffect == "text" && b.rs.index.contains_key("write") {
                lists.push((b.rs.list("write").to_vec(), true));
            }
            for (lst, is_write) in lists {
                for bi in lst {
                    let br = &b.rs.features[bi];
                    let same =
                        br.subject == r.subject && br.verb == r.verb && br.object == neweffect;
                    let wr = is_write && br.subject == r.subject && br.verb == "write";
                    if same || wr {
                        b.rs.features[bi].conds.extend(inv.iter().cloned());
                        if crashy {
                            let c = b.rs.features[bi].conds.clone();
                            b.add(
                                &r.subject,
                                "is",
                                "crash",
                                c,
                                r.ids.clone(),
                                false,
                                None,
                                r.tags.clone(),
                            );
                        }
                    }
                }
            }
        }
        i += 1;
    }
    for &p in &protects {
        let pr = b.rs.features[p].clone();
        let target = pr.subject.clone();
        let g: Vec<Cond> = if pr.conds.is_empty() {
            vec![Cond::never()]
        } else {
            let n = pr.conds.len();
            pr.conds
                .iter()
                .enumerate()
                .map(|(a, c)| {
                    let mut w = c.name.as_str();
                    w = w.strip_prefix('(').unwrap_or(w);
                    w = w.strip_suffix(')').unwrap_or(w);
                    let mut nw = if w.starts_with("not") {
                        w.get(4..).unwrap_or("").to_string()
                    } else {
                        format!("not {w}")
                    };
                    if a == 0 {
                        nw = format!("({nw}");
                    }
                    if a == n - 1 {
                        nw.push(')');
                    }
                    Cond::new(nw, c.params.clone())
                })
                .collect()
        };
        let list = b.rs.list(&target).to_vec();
        for bi in list {
            let br = &b.rs.features[bi];
            let o = br.object.as_str();
            let ok = br.subject == target
                && ((br.verb == "is" && target != o)
                    || (br.verb == "write" && !o.starts_with("not ")))
                && (b.known.contains(o)
                    || o == "revert"
                    || (br.verb == "write" && !o.starts_with("not ")))
                && !o.starts_with("group");
            if ok {
                b.rs.features[bi].conds.extend(g.iter().cloned());
            }
        }
    }
    active
}
