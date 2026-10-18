//! Rule queries used by the turn engine (`docs/spec/rules.md` §18).

use std::collections::BTreeMap;

use crate::conditions::Subj;
use crate::defs::in_brief;
use crate::level::Level;
use crate::rules::Cond;
use crate::unit::Handle;

impl Level {
    /// Unit `h` has property `p` (`IS p` applies).
    pub(crate) fn prop(&self, h: Handle, p: &str) -> bool {
        self.exists(h) && self.has(self.uname(h), "is", p, Subj::Unit(h))
    }

    /// The level has property `p`.
    pub(crate) fn level_prop(&self, p: &str) -> bool {
        self.has("level", "is", p, Subj::Level)
    }

    /// The empty tile has property `p`.
    pub(crate) fn empty_prop(&self, p: &str, x: i32, y: i32) -> bool {
        self.has("empty", "is", p, Subj::Empty(x, y))
    }

    /// Generic "thing has property" for units, empties and the level.
    #[allow(dead_code)]
    pub(crate) fn subj_prop(&self, s: Subj, p: &str) -> bool {
        match s {
            Subj::Unit(h) => self.prop(h, p),
            Subj::Empty(x, y) => self.empty_prop(p, x, y),
            Subj::Level => self.level_prop(p),
        }
    }

    /// Number of matching rules whose conditions hold (stacking).
    pub(crate) fn count(&self, s: &str, v: &str, o: &str, subj: Subj) -> usize {
        let a = self.rules.list(s);
        let b = self.rules.list(o);
        if a.is_empty() || b.is_empty() {
            return 0;
        }
        let list = if a.len() < b.len() { a } else { b };
        list.iter()
            .filter(|&&ri| {
                let r = &self.rules.features[ri];
                r.subject == s && r.verb == v && r.object == o && self.holds(&r.conds, r.cid, subj)
            })
            .count()
    }

    /// Units with property `p`, in rule order then unit order, with
    /// duplicates for several matching rules (`getunitswitheffect`).
    pub(crate) fn units_with(&self, p: &str) -> Vec<Handle> {
        let mut out = Vec::new();
        for &ri in self.rules.list(p) {
            let r = &self.rules.features[ri];
            if r.verb != "is" || r.object != p || in_brief(&r.subject) || r.subject == "empty" {
                continue;
            }
            for &h in self.units_named(&r.subject) {
                if !self.unit(h).dead && self.holds(&r.conds, r.cid, Subj::Unit(h)) {
                    out.push(h);
                }
            }
        }
        out
    }

    /// Rules (s, v, o) without condition test: (subject, conds, cid).
    #[allow(dead_code)]
    pub(crate) fn find(
        &self,
        s: Option<&str>,
        v: &str,
        o: Option<&str>,
    ) -> Vec<(String, Vec<Cond>, u32)> {
        let key = o.or(s).unwrap_or(v);
        self.rules
            .list(key)
            .iter()
            .map(|&ri| &self.rules.features[ri])
            .filter(|r| {
                r.verb == v && s.is_none_or(|s| r.subject == s) && o.is_none_or(|o| r.object == o)
            })
            .map(|r| (r.subject.clone(), r.conds.clone(), r.cid))
            .collect()
    }

    /// Rules with verb `v` grouped by subject (ascending name), objects in
    /// rule order; negated objects and BRIEF/negated subjects skipped.
    pub(crate) fn verb_groups(&self, v: &str) -> BTreeMap<String, Vec<(String, Vec<Cond>, u32)>> {
        let mut out: BTreeMap<String, Vec<(String, Vec<Cond>, u32)>> = BTreeMap::new();
        for &ri in self.rules.list(v) {
            let r = &self.rules.features[ri];
            if r.verb != v
                || r.object.starts_with("not ")
                || r.subject.starts_with("not ")
                || in_brief(&r.subject)
            {
                continue;
            }
            out.entry(r.subject.clone()).or_default().push((
                r.object.clone(),
                r.conds.clone(),
                r.cid,
            ));
        }
        out
    }

    /// Any `X IS p` rule (any subject) exists.
    pub(crate) fn any_is(&self, p: &str) -> bool {
        self.rules
            .list(p)
            .iter()
            .any(|&ri| self.rules.features[ri].verb == "is" && self.rules.features[ri].object == p)
    }
}
