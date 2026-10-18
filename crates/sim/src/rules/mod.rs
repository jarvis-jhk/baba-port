//! The rule set: data model, insertion and the recomputation pipeline
//! (`docs/spec/rules.md`). Sub-modules: letters (`words.md` §5), parsing
//! (`rules.md` §5–§11), ALL/MIMIC/NOT/X IS X post-processing (§13, §15),
//! groups (§14), WORD units (§17), queries (§18).

mod groups;
mod letters;
mod parse;
mod post;
pub(crate) mod query;
mod word;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::defs::{in_full, in_short, is_group};
use crate::events::{DestroyStyle, Event};
use crate::level::Level;
use crate::unit::Handle;

/// One condition entry (`rules.md` §1.1): a name such as `on`, `not near`,
/// `(not on`, `facing)`, `never`, and its params.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Cond {
    /// Condition name with optional `not ` and OR-group parentheses.
    pub name: String,
    /// Parameters (each optionally `not `-prefixed).
    pub params: Vec<String>,
}

impl Cond {
    /// A condition.
    pub fn new(name: impl Into<String>, params: Vec<String>) -> Cond {
        Cond {
            name: name.into(),
            params,
        }
    }
    /// The `never` condition.
    pub fn never() -> Cond {
        Cond::new("never", vec![])
    }
}

/// One rule (a *feature*).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// Subject (`baba`, `not keke`, `text`, `level`, `empty`, ...).
    pub subject: String,
    /// Verb (`is`, `has`, ...).
    pub verb: String,
    /// Object (`you`, `not push`, `keke`, ...).
    pub object: String,
    /// Conditions.
    pub conds: Vec<Cond>,
    /// Identity of the condition list (caches, recursion guards).
    pub cid: u32,
    /// Units that formed the rule, one list per word.
    pub ids: Vec<Vec<Handle>>,
    /// Tags (`base`, `mimic`, `mimicparent_<n>`).
    pub tags: Vec<String>,
}

impl Rule {
    /// The rule is disabled outright (a `never` entry).
    pub fn disabled(&self) -> bool {
        self.conds.iter().any(|c| c.name == "never")
    }

    /// Display form: `baba is you [on(tile),lonely]`.
    pub fn display(&self) -> String {
        let mut s = format!("{} {} {}", self.subject, self.verb, self.object);
        if !self.conds.is_empty() {
            let cs: Vec<String> = self
                .conds
                .iter()
                .map(|c| {
                    if c.params.is_empty() {
                        c.name.clone()
                    } else {
                        format!("{}({})", c.name, c.params.join("|"))
                    }
                })
                .collect();
            s.push_str(&format!(" [{}]", cs.join(",")));
        }
        s
    }
}

/// A group member record (`rules.md` §14 step 3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    /// Member name.
    pub name: String,
    /// Conditions of the membership.
    pub conds: Vec<Cond>,
    /// Conds identity.
    pub cid: u32,
    /// Group (`group`, `group2`, `group3`).
    pub group: String,
    /// Conditions mention groups.
    pub recursive: bool,
}

/// The rule store, rebuilt from scratch at every recomputation.
#[derive(Clone, Debug, Default)]
pub struct RuleSet {
    /// All rules in order.
    pub features: Vec<Rule>,
    /// String → rule indexes (object, verb, subject).
    pub index: BTreeMap<String, Vec<usize>>,
    /// Condition base name → rule indexes.
    pub cond_index: BTreeMap<String, Vec<usize>>,
    /// Snapshot of visible rules (pause screen).
    pub visual: Vec<String>,
    /// NOT-subject expansions by (object, origin position).
    pub not_index: BTreeMap<(String, usize), Vec<usize>>,
    /// Rules mentioning a group.
    pub group_list: Vec<usize>,
    /// Group member records.
    pub members: Vec<Member>,
}

impl RuleSet {
    /// The base rules only.
    pub fn is_empty(&self) -> bool {
        self.features.is_empty()
    }

    /// Rule indexes listed under a key.
    pub fn list(&self, key: &str) -> &[usize] {
        self.index.get(key).map_or(&[], |v| v.as_slice())
    }

    /// Rules as display strings in feature order.
    pub fn display(&self) -> Vec<String> {
        self.features.iter().map(Rule::display).collect()
    }
}

/// Build context for one recomputation.
pub(crate) struct Build<'a> {
    pub lvl: &'a Level,
    pub rs: RuleSet,
    pub cid: u32,
    pub known: BTreeSet<String>,
    pub destroy: Option<DestroyStyle>,
    pub word_valid: BTreeMap<Handle, bool>,
    pub letter_map: BTreeMap<i32, Vec<letters::LetterWord>>,
}

impl<'a> Build<'a> {
    fn new(lvl: &'a Level) -> Build<'a> {
        Build {
            lvl,
            rs: RuleSet::default(),
            cid: lvl.conds_counter,
            known: (*lvl.known).clone(),
            destroy: None,
            word_valid: BTreeMap::new(),
            letter_map: BTreeMap::new(),
        }
    }

    pub fn fresh_cid(&mut self) -> u32 {
        self.cid += 1;
        self.cid
    }

    pub fn destroy(&mut self, s: DestroyStyle) {
        if self.destroy.is_none() {
            self.destroy = Some(s);
        }
    }

    /// Known names (sorted) not in FULL.
    pub fn known_plain(&self) -> Vec<String> {
        self.known.iter().filter(|n| !in_full(n)).cloned().collect()
    }

    /// Insertion (`rules.md` §12).
    #[allow(clippy::too_many_arguments)]
    pub fn add(
        &mut self,
        subject: &str,
        verb: &str,
        object: &str,
        mut conds: Vec<Cond>,
        ids: Vec<Vec<Handle>>,
        visible: bool,
        not_origin: Option<(String, usize)>,
        tags: Vec<String>,
    ) {
        let idx = self.rs.features.len();
        let mut group_flag = is_group(subject)
            || is_group(object)
            || subject.starts_with("not group")
            || object.starts_with("not group");
        // §12.3 condition params, in place.
        let mut added: BTreeSet<String> = BTreeSet::new();
        for c in conds.iter_mut() {
            let base = c.name.strip_prefix("not ").unwrap_or(&c.name).to_string();
            if added.insert(base.clone()) {
                self.rs.cond_index.entry(base).or_default().push(idx);
            }
            if !c.params.is_empty() {
                let mut np = Vec::new();
                for p in &c.params {
                    if p == "all" {
                        for n in self.known.iter() {
                            if !in_short(n) {
                                np.push(n.clone());
                            }
                        }
                    } else if p == "not all" {
                        np.push("empty".to_string());
                        np.push("text".to_string());
                    } else {
                        np.push(p.clone());
                    }
                    if is_group(p) || p.starts_with("not group") {
                        group_flag = true;
                    }
                }
                c.params = np;
            }
        }
        let cid = self.fresh_cid();
        let rule = Rule {
            subject: subject.to_string(),
            verb: verb.to_string(),
            object: object.to_string(),
            conds: conds.clone(),
            cid,
            ids: ids.clone(),
            tags: tags.clone(),
        };
        if visible {
            self.rs.visual.push(rule.display());
        }
        self.rs.features.push(rule);
        self.rs
            .index
            .entry(object.to_string())
            .or_default()
            .push(idx);
        self.rs.index.entry(verb.to_string()).or_default().push(idx);
        if subject != object {
            self.rs
                .index
                .entry(subject.to_string())
                .or_default()
                .push(idx);
        }
        if let Some(o) = not_origin {
            self.rs.not_index.entry(o).or_default().push(idx);
        }
        if group_flag {
            self.rs.group_list.push(idx);
        }
        // §12.2 NOT-subject expansion.
        if let Some(x) = subject.strip_prefix("not ") {
            let obj_group = is_group(object) || object.starts_with("not group");
            let expand =
                (self.known.contains(x) && !is_group(x) && !obj_group) || (obj_group && x == "all");
            if expand {
                let names: Vec<String> = if x != "all" {
                    self.known
                        .iter()
                        .filter(|n| n.as_str() != x && !in_full(n))
                        .cloned()
                        .collect()
                } else {
                    vec!["empty".to_string(), "text".to_string()]
                };
                for n in names {
                    let pos = self.rs.list(object).len();
                    self.add(
                        &n,
                        verb,
                        object,
                        conds.clone(),
                        ids.clone(),
                        false,
                        Some((object.to_string(), pos)),
                        tags.clone(),
                    );
                }
            }
        }
    }

    /// Base rules (`rules.md` §4).
    fn base_rules(&mut self) {
        for (s, v, o) in [
            ("text", "is", "push"),
            ("level", "is", "stop"),
            ("cursor", "is", "select"),
        ] {
            self.add(s, v, o, vec![], vec![], false, None, vec!["base".into()]);
        }
    }
}

/// "Refresh" (`rules.md` §2.2): reset the POWERED cache; recompute if
/// dirty.
pub(crate) fn refresh(lvl: &mut Level) {
    lvl.caches.borrow_mut().powered.clear();
    if lvl.dirty {
        recompute(lvl);
    }
}

/// Full recomputation with the WORD re-run loop (`rules.md` §3).
pub(crate) fn recompute(lvl: &mut Level) {
    let mut rerun = false;
    let mut formed: Option<Vec<Handle>> = None;
    loop {
        lvl.caches.borrow_mut().powered.clear();
        lvl.recompute_count += 1;
        if lvl.recompute_count >= 200 {
            lvl.request_destroy(DestroyStyle::Infinity);
            return;
        }
        // Step 2: WORD units from the previous rules.
        let mut prev = (*lvl.rules).clone();
        let w0 = word::collect(lvl, &mut prev);
        let mut word_valid = BTreeMap::new();
        let mut valid_heads = Vec::new();
        for (h, conds, cid) in &w0.units {
            let ok = lvl.alive(*h)
                && crate::conditions::holds_with(
                    lvl,
                    &prev,
                    conds,
                    *cid,
                    crate::conditions::Subj::Unit(*h),
                );
            word_valid.insert(*h, ok);
            if ok {
                valid_heads.push(*h);
            }
        }
        let (rs, cid, known, destroy, active, parsed) = {
            let mut b = Build::new(lvl);
            b.word_valid = word_valid;
            b.base_rules();
            letters::build_map(&mut b);
            let mut candidates = valid_heads;
            candidates.extend(lvl.text_units().iter().copied());
            let letters_exist = lvl.text_units().iter().any(|&h| lvl.kind(h).word_type == 5);
            let mut active = BTreeSet::new();
            let parsed = !(candidates.is_empty() && !letters_exist);
            if parsed {
                parse::run(&mut b, &candidates);
                if b.destroy.is_none() {
                    post::all_and_mimic(&mut b);
                }
                if b.destroy.is_none() {
                    groups::run(&mut b);
                }
                active = post::postprocess(&mut b);
            }
            (b.rs, b.cid, b.known, b.destroy, active, parsed)
        };
        lvl.conds_counter = cid;
        lvl.known = Arc::new(known);
        if let Some(d) = destroy {
            lvl.request_destroy(d);
            lvl.rules = Arc::new(RuleSet::default());
            lvl.dirty = true;
            return;
        }
        lvl.rules = Arc::new(rs);
        if !parsed {
            // Only base rules; the dirty flag stays set (§3 step 7).
            for u in lvl.order.clone() {
                let v = lvl.unit_mut(u);
                v.active = false;
                v.disabled = false;
            }
            lvl.word_watch = Arc::new(BTreeSet::new());
            lvl.word_kinds = Arc::new(BTreeSet::new());
            return;
        }
        lvl.dirty = false;
        // Active text and new-rule effects (§15.1); text of disabled rules
        // gets a cross (`presentation.md` §7.2).
        let mut disabled: BTreeSet<Handle> = BTreeSet::new();
        for r in lvl.rules.features.iter().filter(|r| r.disabled()) {
            disabled.extend(r.ids.iter().flatten().copied());
        }
        for u in lvl.order.clone() {
            let a = active.contains(&u) && lvl.is_text(u);
            let d = disabled.contains(&u) && lvl.is_text(u);
            let v = lvl.unit_mut(u);
            v.active = a;
            v.disabled = d;
        }
        if !rerun && !lvl.first_turn {
            let fresh: Vec<Handle> = active
                .iter()
                .copied()
                .filter(|u| !lvl.active_prev.contains(u) && lvl.alive(*u))
                .collect();
            if !fresh.is_empty() {
                formed = Some(fresh);
            }
        }
        lvl.active_prev = Arc::new(active);
        // Step 13: WORD stability.
        let mut rs = (*lvl.rules).clone();
        let w1 = word::collect(lvl, &mut rs);
        lvl.rules = Arc::new(rs);
        let mut watch: BTreeSet<Handle> = w1.units.iter().map(|w| w.0).collect();
        watch.extend(w1.related.iter().copied());
        lvl.word_watch = Arc::new(watch);
        lvl.word_kinds = Arc::new(w1.kinds.clone());
        if w1.ident != w0.ident {
            lvl.dirty = true;
            rerun = true;
            continue;
        }
        break;
    }
    if let Some(fresh) = formed {
        rule_formed(lvl, &fresh);
    }
    crate::vision::after_recompute(lvl);
}

/// New-rule effects (`presentation.md` §7.1): bling on each text that
/// joined a rule, and the rule sound unless only PLAY rules formed.
fn rule_formed(lvl: &mut Level, fresh: &[Handle]) {
    let set: BTreeSet<Handle> = fresh.iter().copied().collect();
    let sounding = lvl
        .rules
        .features
        .iter()
        .any(|r| r.verb != "play" && r.ids.iter().flatten().any(|h| set.contains(h)));
    let sound = if sounding {
        let (_, short) = lvl.fx.check("rule");
        Some(crate::events::Sound {
            short,
            ..crate::events::sounds::RULE
        })
    } else {
        None
    };
    let texts = fresh.iter().map(|&h| lvl.unit(h).id).collect();
    lvl.events.push(Event::RuleFormed { texts, sound });
}
