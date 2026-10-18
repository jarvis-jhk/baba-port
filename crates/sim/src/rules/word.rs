//! WORD units (`docs/spec/rules.md` §17).

use std::collections::BTreeSet;

use super::{Cond, RuleSet};
use crate::defs::{NameId, is_group};
use crate::level::Level;
use crate::unit::Handle;

pub(crate) struct WordInfo {
    /// (unit, conds snapshot, conds id)
    pub units: Vec<(Handle, Vec<Cond>, u32)>,
    pub ident: String,
    pub related: Vec<Handle>,
    pub kinds: BTreeSet<NameId>,
}

/// Collects WORD units from `rs` and applies the stability check in place.
pub(crate) fn collect(lvl: &Level, rs: &mut RuleSet) -> WordInfo {
    let mut out = WordInfo {
        units: Vec::new(),
        ident: String::new(),
        related: Vec::new(),
        kinds: BTreeSet::new(),
    };
    let list: Vec<usize> = rs.list("word").to_vec();
    if list.is_empty() {
        return out;
    }
    let mut done: BTreeSet<String> = BTreeSet::new();
    let mut fullid: Vec<String> = Vec::new();
    let mut checkrec: Vec<(String, usize)> = Vec::new();
    let any_broken = rs.index.contains_key("broken");
    for (pos, &i) in list.iter().enumerate() {
        let r = &rs.features[i];
        if r.verb != "is" {
            continue;
        }
        let name = r.subject.clone();
        let mut subid = String::new();
        if lvl.known.contains(&name) && name != "text" && done.insert(name.clone()) {
            if let Some(n) = lvl.defs.name_id(&name) {
                out.kinds.insert(n);
            }
            for &h in lvl.units_named(&name) {
                let broken = any_broken
                    && lvl.has(
                        lvl.uname(h),
                        "is",
                        "broken",
                        crate::conditions::Subj::Unit(h),
                    );
                if !broken {
                    out.units.push((h, r.conds.clone(), r.cid));
                    subid.push_str(&name);
                }
            }
        }
        if !subid.is_empty() {
            for c in &r.conds {
                subid.push_str(&c.name);
                for p in &c.params {
                    subid.push_str(p);
                    let names: Vec<String> = if is_group(p) {
                        rs.members
                            .iter()
                            .filter(|m| m.group == *p)
                            .map(|m| m.name.clone())
                            .collect()
                    } else {
                        vec![p.clone()]
                    };
                    for n in names {
                        out.related.extend(lvl.units_named(&n).iter().copied());
                    }
                }
            }
        }
        fullid.push(subid);
        if let Some(first) = r.ids.first()
            && first.len() == 1
            && lvl.alive(first[0])
        {
            let notname = name.strip_prefix("not ").unwrap_or(&name);
            let un = lvl.uname(first[0]);
            if un != format!("text_{name}") && un != format!("text_{notname}") {
                checkrec.push((name.clone(), pos));
            }
        }
    }
    fullid.sort();
    out.ident = fullid.concat();
    for (checkname, pos) in checkrec {
        let bname = checkname
            .strip_prefix("not ")
            .unwrap_or(&checkname)
            .to_string();
        let mut found = false;
        for &i in &list {
            let r = &rs.features[i];
            let s = r.subject.as_str();
            if s == bname || s == "all" || (s != bname && s.starts_with("not")) {
                for g in &r.ids {
                    for &d in g {
                        let tn = if lvl.exists(d) {
                            lvl.uname(d).to_string()
                        } else {
                            String::new()
                        };
                        if tn == format!("text_{s}")
                            || s == "all"
                            || s.starts_with("group")
                            || (s != checkname && s.starts_with("not"))
                        {
                            found = true;
                        }
                    }
                }
                if r.tags.iter().any(|t| t == "mimic") {
                    found = true;
                }
            }
        }
        if !found {
            out.ident = "null".to_string();
            for &i in &list {
                let s = rs.features[i].subject.clone();
                if s == bname || s == format!("not {bname}") {
                    rs.features[i].conds = vec![Cond::never()];
                }
            }
            if checkname.starts_with("not ")
                && let Some(v) = rs.not_index.get(&("word".to_string(), pos + 1)).cloned()
            {
                for i in v {
                    rs.features[i].conds = vec![Cond::never()];
                }
            }
        }
    }
    out
}
