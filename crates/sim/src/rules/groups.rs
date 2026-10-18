//! Groups (`docs/spec/rules.md` §14).

use std::collections::BTreeMap;

use super::post::invert;
use super::{Build, Cond, Member};
use crate::defs::is_group;
use crate::unit::Handle;

#[derive(Clone)]
struct GR {
    s: String,
    v: String,
    o: String,
    conds: Vec<Cond>,
    ids: Vec<Vec<Handle>>,
    tags: Vec<String>,
    rec: bool,
}

fn is_notgroup(s: &str) -> bool {
    s.starts_with("not group")
}

/// Runs group processing over the group list.
pub(crate) fn run(b: &mut Build<'_>) {
    if b.rs.group_list.is_empty() {
        return;
    }
    let mut isgroup: Vec<GR> = Vec::new();
    let mut isnotgroup: BTreeMap<String, Vec<GR>> = BTreeMap::new();
    let mut xgroup: Vec<GR> = Vec::new();
    let mut xnotgroup: BTreeMap<String, Vec<GR>> = BTreeMap::new();
    let mut groupx: Vec<GR> = Vec::new();
    let mut notgroupx: Vec<GR> = Vec::new();
    let mut ggroup: Vec<GR> = Vec::new();
    let mut gg_diff: Vec<GR> = Vec::new();
    let mut g_isnot_g: Vec<GR> = Vec::new();
    let mut ng_is_g: Vec<GR> = Vec::new();
    let mut ng_diff: Vec<GR> = Vec::new();
    let mut base_members: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for &i in &b.rs.group_list.clone() {
        let r = &b.rs.features[i];
        let rec = r
            .conds
            .iter()
            .any(|c| c.params.iter().any(|p| is_group(p) || is_notgroup(p)));
        let g = GR {
            s: r.subject.clone(),
            v: r.verb.clone(),
            o: r.object.clone(),
            conds: r.conds.clone(),
            ids: r.ids.clone(),
            tags: r.tags.clone(),
            rec,
        };
        let groupx_t = is_group(&g.s);
        let notgroupx_t = is_notgroup(&g.s);
        let gn1 = if notgroupx_t {
            g.s[4..].to_string()
        } else {
            g.s.clone()
        };
        let xgroup_t = is_group(&g.o);
        let xnotgroup_t = is_notgroup(&g.o);
        let gn2 = if xnotgroup_t {
            g.o[4..].to_string()
        } else {
            g.o.clone()
        };
        let is = g.v == "is";
        if xgroup_t && is {
            if !groupx_t && !notgroupx_t {
                base_members
                    .entry(g.o.clone())
                    .or_default()
                    .push(g.s.clone());
                isgroup.push(g);
            } else if !notgroupx_t {
                if gn1 == gn2 {
                    ggroup.push(g)
                } else {
                    gg_diff.push(g)
                }
            } else if gn1 == gn2 {
                ng_is_g.push(g)
            } else {
                ng_diff.push(g)
            }
        } else if xgroup_t {
            if !groupx_t && !notgroupx_t {
                xgroup.push(g)
            } else {
                ggroup.push(g)
            }
        } else if xnotgroup_t && is {
            if !groupx_t && !notgroupx_t {
                isnotgroup.entry(g.s.clone()).or_default().push(g.clone());
                xnotgroup.entry(g.s.clone()).or_default().push(g);
            } else if !notgroupx_t {
                if gn1 == gn2 {
                    g_isnot_g.push(g)
                } else {
                    gg_diff.push(g)
                }
            } else if gn1 == gn2 {
                ggroup.push(g)
            } else {
                ng_diff.push(g)
            }
        } else if xnotgroup_t {
            xnotgroup.entry(g.s.clone()).or_default().push(g);
        } else if groupx_t {
            groupx.push(g)
        } else if notgroupx_t {
            notgroupx.push(g)
        }
    }

    // Step 1: group-group renames.
    let mut used = vec![false; gg_diff.len()];
    loop {
        let mut changed = false;
        for (i, g) in gg_diff.iter().enumerate() {
            if used[i] {
                continue;
            }
            let notrule = g.o.starts_with("not ");
            let mut acc = g.conds.clone();
            let mut produced: Vec<GR> = Vec::new();
            let mut j = 0;
            while j < isgroup.len() {
                let m = isgroup[j].clone();
                j += 1;
                if m.o == g.s {
                    used[i] = true;
                    changed = true;
                    acc.extend(m.conds.iter().cloned());
                    produced.push(GR {
                        s: m.s.clone(),
                        v: "is".into(),
                        o: g.o.clone(),
                        conds: vec![],
                        ids: g.ids.clone(),
                        tags: g.tags.clone(),
                        rec: m.rec || g.rec,
                    });
                }
            }
            for mut p in produced {
                p.conds = acc.clone();
                if !notrule {
                    isgroup.push(p);
                } else {
                    isnotgroup.entry(p.s.clone()).or_default().push(p);
                }
            }
        }
        if !changed {
            break;
        }
    }

    // Step 2: NOT Gi IS Gj (i ≠ j) fixed point.
    if !ng_diff.is_empty() {
        let known = b.known_plain();
        let mut current: BTreeMap<String, Vec<String>> = base_members.clone();
        let mut prev_sig = String::new();
        let mut done = false;
        let mut found: BTreeMap<String, BTreeMap<String, Vec<GR>>> = BTreeMap::new();
        for _round in 0..10 {
            found.clear();
            for g in &ng_diff {
                let gn1 = &g.s[4..];
                let gn2 = g.o.strip_prefix("not ").unwrap_or(&g.o).to_string();
                let e = found.entry(gn2).or_default();
                for n in &known {
                    if !current.get(gn1).is_some_and(|m| m.contains(n)) {
                        e.entry(n.clone()).or_default().push(g.clone());
                    }
                }
            }
            let mut sig = String::new();
            for (k, v) in &found {
                sig.push_str(k);
                for n in v.keys() {
                    sig.push_str(n);
                }
            }
            if sig == prev_sig {
                done = true;
                break;
            }
            prev_sig = sig;
            current = base_members.clone();
            for (k, v) in &found {
                current.insert(k.clone(), v.keys().cloned().collect());
            }
        }
        if !done {
            b.destroy(crate::events::DestroyStyle::Infinity);
            return;
        }
        for v in found.values() {
            for (n, list) in v {
                for g in list {
                    let p = GR {
                        s: n.clone(),
                        v: "is".into(),
                        o: g.o.clone(),
                        conds: g.conds.clone(),
                        ids: g.ids.clone(),
                        tags: g.tags.clone(),
                        rec: g.rec,
                    };
                    if !g.o.starts_with("not ") {
                        isgroup.push(p);
                    } else {
                        isnotgroup.entry(n.clone()).or_default().push(p);
                    }
                }
            }
        }
    }

    // Step 3: memberships.
    let mut combined: Vec<GR> = Vec::new();
    let mut memberships: BTreeMap<String, Vec<(String, Vec<Cond>)>> = BTreeMap::new();
    let mut members: Vec<Member> = Vec::new();
    let known = b.known_plain();
    for m in &isgroup {
        let names: Vec<String> = if !m.s.starts_with("not ") {
            vec![m.s.clone()]
        } else if m.s != "not all" {
            known.iter().filter(|n| **n != m.s[4..]).cloned().collect()
        } else {
            vec![]
        };
        for name in names {
            let mut never = false;
            let mut prevents: Vec<Cond> = Vec::new();
            let mut rec = m.rec;
            if let Some(list) = isnotgroup.get(&name) {
                for x in list {
                    if x.o[4..] == m.o {
                        rec = rec || x.rec;
                        let (inv, _, nf) = invert(&x.conds, None);
                        if nf {
                            never = true;
                            break;
                        }
                        prevents.extend(inv);
                    }
                }
            }
            if never {
                continue;
            }
            let mut fconds = m.conds.clone();
            fconds.extend(prevents.iter().cloned());
            let cid = b.fresh_cid();
            members.push(Member {
                name: name.clone(),
                conds: fconds.clone(),
                cid,
                group: m.o.clone(),
                recursive: rec,
            });
            memberships
                .entry(m.o.clone())
                .or_default()
                .push((name.clone(), fconds));
            for gx in &groupx {
                if gx.s == m.o {
                    let mut c = m.conds.clone();
                    c.extend(gx.conds.iter().cloned());
                    c.extend(prevents.iter().cloned());
                    let mut ids = m.ids.clone();
                    ids.extend(gx.ids.iter().cloned());
                    let mut tags = m.tags.clone();
                    tags.extend(gx.tags.iter().cloned());
                    combined.push(GR {
                        s: name.clone(),
                        v: gx.v.clone(),
                        o: gx.o.clone(),
                        conds: c,
                        ids,
                        tags,
                        rec: false,
                    });
                }
            }
        }
    }

    // Step 4: group verb group.
    for g in &ggroup {
        let (notrule, gn1) = match g.s.strip_prefix("not ") {
            Some(x) => (true, x.to_string()),
            None => (false, g.s.clone()),
        };
        let mut never = false;
        let mut prevents = Vec::new();
        if let Some(list) = xnotgroup.get(&gn1) {
            for x in list {
                if x.s == g.s && x.v == g.v && x.o == format!("not {}", g.o) {
                    let (inv, _, nf) = invert(&x.conds, None);
                    if nf {
                        never = true;
                        break;
                    }
                    prevents.extend(inv);
                }
            }
        }
        if never {
            continue;
        }
        let mut team1: Vec<(String, Option<Vec<Cond>>)> = Vec::new();
        if !notrule {
            for (n, c) in memberships.get(&gn1).into_iter().flatten() {
                team1.push((n.clone(), Some(c.clone())));
            }
        } else {
            let mut ignore = Vec::new();
            for (n, c) in memberships.get(&gn1).into_iter().flatten() {
                ignore.push(n.clone());
                let (inv, _, nf) = invert(c, None);
                if !nf {
                    team1.push((n.clone(), Some(inv)));
                }
            }
            for n in &known {
                if !ignore.contains(n) {
                    team1.push((n.clone(), None));
                }
            }
        }
        let gn2 = g.o.strip_prefix("not ").unwrap_or(&g.o).to_string();
        let team2: Vec<(String, Vec<Cond>)> = memberships.get(&gn2).cloned().unwrap_or_default();
        for (a, ac) in &team1 {
            for (d, dc) in &team2 {
                let mut c = g.conds.clone();
                if let Some(ac) = ac {
                    c.extend(ac.iter().cloned());
                }
                c.extend(dc.iter().cloned());
                c.extend(prevents.iter().cloned());
                combined.push(GR {
                    s: a.clone(),
                    v: g.v.clone(),
                    o: d.clone(),
                    conds: c,
                    ids: g.ids.clone(),
                    tags: g.tags.clone(),
                    rec: false,
                });
            }
        }
    }

    // Step 5: NOT group subject.
    if !notgroupx.is_empty() {
        for name in &known {
            for g in &notgroupx {
                let gname = &g.s[4..];
                let mut c = g.conds.clone();
                let mut valid = true;
                for m in members
                    .iter()
                    .filter(|m| &m.name == name && m.group == gname)
                {
                    let (inv, _, nf) = invert(&m.conds, None);
                    c.extend(inv);
                    if nf {
                        valid = false;
                        break;
                    }
                }
                if valid {
                    combined.push(GR {
                        s: name.clone(),
                        v: g.v.clone(),
                        o: g.o.clone(),
                        conds: c,
                        ids: g.ids.clone(),
                        tags: g.tags.clone(),
                        rec: false,
                    });
                }
            }
        }
    }

    // Step 6: X verb group.
    for g in &xgroup {
        if g.v == "is" {
            continue;
        }
        for (n, mc) in memberships.get(&g.o).into_iter().flatten() {
            let mut c = g.conds.clone();
            c.extend(mc.iter().cloned());
            combined.push(GR {
                s: g.s.clone(),
                v: g.v.clone(),
                o: n.clone(),
                conds: c,
                ids: g.ids.clone(),
                tags: g.tags.clone(),
                rec: false,
            });
        }
    }

    // Step 7: X verb NOT group.
    for list in xnotgroup.values() {
        for g in list {
            if is_group(&g.s) || is_notgroup(&g.s) || g.v == "is" {
                continue;
            }
            let gn2 = &g.o[4..];
            for (n, mc) in memberships.get(gn2).into_iter().flatten() {
                let mut c = g.conds.clone();
                c.extend(mc.iter().cloned());
                combined.push(GR {
                    s: g.s.clone(),
                    v: g.v.clone(),
                    o: format!("not {n}"),
                    conds: c,
                    ids: g.ids.clone(),
                    tags: g.tags.clone(),
                    rec: false,
                });
            }
        }
    }

    // Step 8: G IS NOT G → members crash.
    for g in &g_isnot_g {
        for (n, mc) in memberships.get(&g.s).into_iter().flatten() {
            let mut c = g.conds.clone();
            c.extend(mc.iter().cloned());
            combined.push(GR {
                s: n.clone(),
                v: "is".into(),
                o: "crash".into(),
                conds: c,
                ids: g.ids.clone(),
                tags: g.tags.clone(),
                rec: false,
            });
        }
    }

    // Step 9: NOT G IS G → non-members crash.
    for g in &ng_is_g {
        let gn1 = &g.s[4..];
        let mut team1: Vec<(String, Vec<Cond>)> = Vec::new();
        let mut ignore = Vec::new();
        for (n, mc) in memberships.get(gn1).into_iter().flatten() {
            ignore.push(n.clone());
            let (inv, _, nf) = invert(mc, None);
            if !nf {
                team1.push((n.clone(), inv));
            }
        }
        for n in &known {
            if !ignore.contains(n) {
                team1.push((n.clone(), vec![]));
            }
        }
        for (n, mc) in team1 {
            let mut c = g.conds.clone();
            c.extend(mc);
            combined.push(GR {
                s: n,
                v: "is".into(),
                o: "crash".into(),
                conds: c,
                ids: g.ids.clone(),
                tags: g.tags.clone(),
                rec: false,
            });
        }
    }

    b.rs.members = members;
    for g in combined {
        b.add(&g.s, &g.v, &g.o, g.conds, g.ids, false, None, g.tags);
    }
}
