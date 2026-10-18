//! Parsing text on the grid into rules: grid lookup (`rules.md` §5),
//! sentence starts (§6), sentence enumeration (§7), grammar (§8), restarts
//! (§9), de-duplication (§10) and rule extraction (§11).

use std::collections::BTreeSet;
use std::rc::Rc;

use super::{Build, Cond};
use crate::defs::wk;
use crate::events::DestroyStyle;
use crate::unit::Handle;

#[derive(Clone, Debug)]
struct Entry {
    units: Vec<Handle>,
    width: i32,
    word: String,
    kind: i32,
}

#[derive(Clone, Debug)]
struct SWord {
    word: String,
    kind: i32,
    units: Vec<Handle>,
}

#[derive(Clone, Debug)]
struct FirstWord {
    units: Vec<Handle>,
    dir: u8,
    sent: Option<Rc<Vec<SWord>>>,
    /// 1-based start index into `sent`.
    index: usize,
    sent_id: String,
}

#[derive(Clone, Debug)]
struct FWord {
    word: String,
    kind: i32,
    units: Vec<Handle>,
    wordid: usize,
    subsent_id: String,
}

fn dvec(d: u8) -> (i32, i32) {
    if d == 1 { (1, 0) } else { (0, 1) }
}

/// Lua-style `string.sub(s, from)` with a 1-based start.
fn lsub(s: &str, from1: isize) -> String {
    let from = (from1 - 1).max(0) as usize;
    s.get(from.min(s.len())..).unwrap_or("").to_string()
}

/// Lua-style `string.sub(s, 1, n)`.
fn lpre(s: &str, n: usize) -> &str {
    &s[..n.min(s.len())]
}

/// `words_at` (§5): word entries at origin unit position + offset.
fn words_at(
    b: &Build<'_>,
    origin: Handle,
    ox: i32,
    oy: i32,
    d: u8,
    start_only: bool,
) -> (Vec<Entry>, bool) {
    let lvl = b.lvl;
    let u = lvl.unit(origin);
    let (ux, uy) = (u.x, u.y);
    let (x, y) = (ux + ox, uy + oy);
    let mut out = Vec::new();
    let mut letters_only = false;
    if lvl.on_grid(x, y) {
        for &h in lvl.at(x, y) {
            let v = lvl.unit(h);
            let k = lvl.kind(h);
            if k.word_type != wk::LETTER && !v.dead {
                if k.is_text {
                    out.push(Entry {
                        units: vec![h],
                        width: 1,
                        word: k.word.clone(),
                        kind: k.word_type,
                    });
                } else if b.word_valid.get(&h) == Some(&true) {
                    out.push(Entry {
                        units: vec![h],
                        width: 1,
                        word: k.name.clone(),
                        kind: k.word_type,
                    });
                }
            } else {
                letters_only = true;
            }
        }
        if let Some(list) = b.letter_map.get(&lvl.tile_index(x, y)) {
            for e in list {
                if e.dir != d {
                    continue;
                }
                if start_only && (x != e.x || y != e.y) && e.width > 1 {
                    continue;
                }
                if e.width > 1
                    && ((d == 1 && ux > e.x && ux < e.x + e.width)
                        || (d == 2 && uy > e.y && uy < e.y + e.width))
                {
                    continue;
                }
                let word = match e.name.strip_prefix("text_") {
                    Some(w) if e.name.len() > 5 => w.to_string(),
                    _ => e.name.clone(),
                };
                out.push(Entry {
                    units: e.units.clone(),
                    width: e.width,
                    word,
                    kind: e.kind,
                });
            }
        }
    }
    (out, letters_only)
}

/// Runs the parser over the candidates (§6–§11).
pub(crate) fn run(b: &mut Build<'_>, candidates: &[Handle]) {
    let lvl = b.lvl;
    let mut fws: Vec<FirstWord> = Vec::new();
    let mut used: std::collections::BTreeMap<i32, [bool; 3]> = std::collections::BTreeMap::new();
    for &h in candidates {
        let u = lvl.unit(h);
        if lvl.kind(h).word_type == wk::LETTER || u.dead {
            continue;
        }
        let t = lvl.tile_index(u.x, u.y);
        if used.contains_key(&t) {
            continue;
        }
        for d in 1..=2u8 {
            let (dx, dy) = dvec(d);
            let back = words_at(b, h, -dx, -dy, d, false).0;
            let fwd = words_at(b, h, dx, dy, d, false).0;
            if back.is_empty() && !fwd.is_empty() {
                fws.push(FirstWord {
                    units: vec![h],
                    dir: d,
                    sent: None,
                    index: 1,
                    sent_id: String::new(),
                });
                used.entry(t).or_default()[d as usize] = true;
            }
        }
    }
    let letter_entries: Vec<super::letters::LetterWord> = b
        .letter_map
        .values()
        .flat_map(|v| v.iter().cloned())
        .collect();
    for e in letter_entries {
        let t = lvl.tile_index(e.x, e.y);
        if used.get(&t).is_some_and(|u| u[e.dir as usize]) {
            continue;
        }
        let first = e.units[0];
        let (dx, dy) = dvec(e.dir);
        let back = words_at(b, first, -dx, -dy, e.dir, false).0;
        let fwd = words_at(b, first, dx * e.width, dy * e.width, e.dir, false).0;
        if back.is_empty() && !fwd.is_empty() {
            fws.push(FirstWord {
                units: e.units.clone(),
                dir: e.dir,
                sent: None,
                index: 1,
                sent_id: String::new(),
            });
            used.entry(t).or_default()[e.dir as usize] = true;
        }
    }
    docode(b, fws);
}

struct Calc {
    sentences: Vec<Rc<Vec<SWord>>>,
    ids: Vec<String>,
    maxpos: usize,
}

/// Sentence enumeration (§7). `None` = level destroyed.
fn calculate(b: &Build<'_>, origin: Handle, dir: u8, newfw: &mut Vec<FirstWord>) -> Option<Calc> {
    let (dx, dy) = dvec(dir);
    const LIMIT: usize = 5000;
    let mut steps: Vec<Option<Vec<Entry>>> = Vec::new(); // None = placeholder
    let mut verb = false;
    let mut noun = false;
    let mut starting = true;
    let mut total = 1usize;
    let mut prev_shared: i32 = -1;
    let mut prev_maxw = 1;
    let mut currw = 0;
    let mut rstep = 0;
    let mut step_count = 0usize; // Lua `step` (includes the terminating tile)
    loop {
        let (words, letters_only) = words_at(b, origin, dx * rstep, dy * rstep, dir, true);
        rstep += 1;
        step_count += 1;
        if !words.is_empty() {
            let mut shared: i32 = -1;
            let mut maxw = 1;
            for v in &words {
                if shared == -1 {
                    shared = v.kind;
                } else if v.kind != shared {
                    shared = -2;
                }
                if v.kind == wk::VERB {
                    verb = true;
                }
                if v.kind == wk::NOUN {
                    noun = true;
                }
                if starting && (v.kind == wk::NOUN || v.kind == wk::PREFIX || v.kind == wk::NOT) {
                    starting = false;
                }
                maxw = maxw.max(v.width);
                if v.width > 1 {
                    currw = currw.max(v.width + 1);
                }
            }
            let mut done = false;
            if shared >= 0 && prev_shared >= 0 && maxw == 1 && prev_maxw == 1 && currw == 0 {
                let pair = (shared, prev_shared);
                if pair == (0, 0) || pair == (1, 1) || pair == (2, 2) || pair == (0, 2) {
                    done = true;
                    newfw.push(FirstWord {
                        units: words[0].units.clone(),
                        dir,
                        sent: None,
                        index: 1,
                        sent_id: String::new(),
                    });
                }
            }
            currw = (currw - 1).max(0);
            prev_shared = shared;
            prev_maxw = maxw;
            if done {
                break;
            }
            if starting {
                step_count -= 1;
            } else {
                total = total.saturating_mul(words.len());
                if total >= LIMIT {
                    return None;
                }
                steps.push(Some(words));
            }
        } else if letters_only {
            if starting {
                step_count -= 1;
            } else {
                steps.push(None);
            }
        } else {
            break;
        }
    }
    if !verb || step_count < 3 || !noun {
        return Some(Calc {
            sentences: vec![],
            ids: vec![],
            maxpos: 0,
        });
    }
    let maxpos = step_count;
    let combospots: Vec<usize> = steps
        .iter()
        .enumerate()
        .filter(|(_, s)| s.as_ref().is_some_and(|w| w.len() > 1))
        .map(|(i, _)| i)
        .collect();
    let mut combo: Vec<usize> = vec![0; steps.len()];
    let mut sentences = Vec::with_capacity(total);
    let mut ids = Vec::with_capacity(total);
    for _ in 0..total {
        let mut sent = Vec::new();
        let mut id = String::new();
        let mut step = 0usize;
        while step + 1 < maxpos && step < steps.len() {
            match &steps[step] {
                Some(ws) => {
                    let c = combo[step];
                    let w = &ws[c];
                    sent.push(SWord {
                        word: w.word.clone(),
                        kind: w.kind,
                        units: w.units.clone(),
                    });
                    id.push_str(&c.to_string());
                    step += w.width.max(1) as usize;
                }
                None => break,
            }
        }
        sentences.push(Rc::new(sent));
        ids.push(id);
        if !combospots.is_empty() {
            let mut cs = 0;
            loop {
                let t = combospots[cs];
                combo[t] += 1;
                let n = steps[t].as_ref().map_or(1, |w| w.len());
                if combo[t] >= n {
                    combo[t] = 0;
                    cs = (cs + 1) % combospots.len();
                } else {
                    break;
                }
            }
        }
    }
    Some(Calc {
        sentences,
        ids,
        maxpos,
    })
}

fn docode(b: &mut Build<'_>, mut fws: Vec<FirstWord>) {
    let mut done_keys: BTreeSet<(i32, String, u8)> = BTreeSet::new();
    let mut existing_finals: BTreeSet<String> = BTreeSet::new();
    let mut limiter = 0;
    let mut k = 0;
    while k < fws.len() {
        let fw = fws[k].clone();
        k += 1;
        limiter += 1;
        if limiter > 5000 {
            b.destroy(DestroyStyle::TooComplex);
            return;
        }
        let lvl = b.lvl;
        let u0 = lvl.unit(fw.units[0]);
        let tile = lvl.tile_index(u0.x, u0.y);
        if !done_keys.insert((tile, fw.sent_id.clone(), fw.dir)) {
            continue;
        }
        let dir = fw.dir;
        let (sentences, ids, maxlen, start) = match &fw.sent {
            None => {
                let mut newfw = Vec::new();
                let Some(c) = calculate(b, fw.units[0], dir, &mut newfw) else {
                    b.destroy(DestroyStyle::TooComplex);
                    return;
                };
                fws.extend(newfw);
                (c.sentences, c.ids, c.maxpos, 1usize)
            }
            Some(s) => (vec![s.clone()], vec![fw.sent_id.clone()], 3, fw.index),
        };
        let mut finals: Vec<(Vec<FWord>, Rc<Vec<SWord>>)> = Vec::new();
        if maxlen > 2 {
            for (sent, sent_id) in sentences.iter().zip(&ids) {
                let cur = walk(
                    sent,
                    sent_id,
                    start,
                    tile,
                    dir,
                    &mut existing_finals,
                    &mut fws,
                );
                finals.push((cur, sent.clone()));
            }
        }
        extract(b, &finals, dir, &mut fws);
    }
}

/// Grammar walk of one variant (§8–§10). Returns the kept words.
#[allow(clippy::too_many_arguments)]
fn walk(
    sent: &Rc<Vec<SWord>>,
    sent_id: &str,
    start: usize,
    tile: i32,
    dir: u8,
    existing_finals: &mut BTreeSet<String>,
    fws: &mut Vec<FirstWord>,
) -> Vec<FWord> {
    let n = sent.len();
    let mut current: Vec<FWord> = Vec::new();
    let mut stage: i32 = 0;
    let mut prevstage: i32 = 0;
    let mut notids: Vec<Handle> = Vec::new();
    let mut notslot = 0usize;
    let mut stage3 = false;
    let mut stage2 = false;
    let mut doingcond = false;
    let mut nocondsafter = false;
    let mut firstreal = false;
    let mut curtype: i32 = 0;
    let mut prevsafe_id = 0usize;
    let mut prevsafe_type: i32 = 0;
    let mut stop = false;
    let mut cleared = false;
    let mut j = 0usize;
    let mut wordid = start;
    while wordid <= n {
        j += 1;
        let s = &sent[wordid - 1];
        let next = sent.get(wordid);
        let prevtype = curtype;
        let t = s.kind;
        curtype = t;
        if t != wk::LETTER {
            let fail = |stage: &mut i32, stop: &mut bool| {
                *stage = -1;
                *stop = true;
            };
            match stage {
                0 => {
                    if t == wk::NOUN {
                        prevstage = stage;
                        stage = 2;
                    } else if t == wk::PREFIX {
                        prevstage = stage;
                        stage = 1;
                    } else if t != wk::NOT {
                        prevstage = stage;
                        fail(&mut stage, &mut stop);
                    }
                }
                1 => {
                    if t == wk::NOUN {
                        prevstage = stage;
                        stage = 2;
                    } else if t == wk::AND {
                        prevstage = stage;
                        stage = 6;
                    } else if t != wk::NOT {
                        prevstage = stage;
                        fail(&mut stage, &mut stop);
                    }
                }
                2 => {
                    if wordid != n {
                        if t == wk::VERB
                            && prevtype != wk::NOT
                            && (prevstage != 4 || doingcond || !stage3)
                        {
                            stage2 = true;
                            doingcond = false;
                            prevstage = stage;
                            nocondsafter = true;
                            stage = 3;
                        } else if t == wk::INFIX
                            && !stage2
                            && !nocondsafter
                            && (!doingcond || prevstage != 4)
                        {
                            doingcond = true;
                            prevstage = stage;
                            stage = 3;
                        } else if t == wk::AND && prevtype != wk::NOT {
                            prevstage = stage;
                            stage = 4;
                        } else if t != wk::NOT {
                            prevstage = stage;
                            fail(&mut stage, &mut stop);
                        }
                    } else {
                        fail(&mut stage, &mut stop);
                    }
                }
                3 => {
                    stage3 = true;
                    if t == wk::NOUN || t == wk::PROP || t == wk::CUSTOM {
                        prevstage = stage;
                        stage = 5;
                    } else if t != wk::NOT {
                        fail(&mut stage, &mut stop);
                    }
                }
                4 => {
                    if t == wk::NOUN || ((t == wk::PROP || t == wk::CUSTOM) && stage3) {
                        prevstage = stage;
                        stage = 2;
                    } else if t == wk::VERB && stage3 && !doingcond && prevtype != wk::NOT {
                        stage2 = true;
                        nocondsafter = true;
                        prevstage = stage;
                        stage = 3;
                    } else if t == wk::INFIX && !nocondsafter && (prevtype != wk::AND || doingcond)
                    {
                        doingcond = true;
                        stage2 = true;
                        prevstage = stage;
                        stage = 3;
                    } else if t != wk::NOT {
                        prevstage = stage;
                        fail(&mut stage, &mut stop);
                    }
                }
                5 => {
                    if wordid != n {
                        if t == wk::VERB && doingcond && prevtype != wk::NOT {
                            stage2 = true;
                            doingcond = false;
                            prevstage = stage;
                            nocondsafter = true;
                            stage = 3;
                        } else if t == wk::AND && prevtype != wk::NOT {
                            prevstage = stage;
                            stage = 4;
                        } else if t != wk::NOT {
                            prevstage = stage;
                            fail(&mut stage, &mut stop);
                        }
                    } else {
                        fail(&mut stage, &mut stop);
                    }
                }
                6 => {
                    if t == wk::PREFIX {
                        prevstage = stage;
                        stage = 1;
                    } else if t != wk::NOT {
                        prevstage = stage;
                        fail(&mut stage, &mut stop);
                    }
                }
                _ => {}
            }
        }
        if stage > 0 {
            firstreal = true;
        }
        if t == wk::NOT {
            if notids.is_empty() || prevtype == wk::NOUN {
                notids = s.units.clone();
                notslot = wordid;
            }
        } else if !stop && t != wk::NOUN {
            notids.clear();
            notslot = 0;
        }
        if prevtype != wk::NOT && wordid > start {
            prevsafe_id = wordid - 1;
            prevsafe_type = prevtype;
        }
        if prevtype == wk::NOT && t == wk::AND {
            stop = true;
        }
        if !stop {
            let sub = lsub(sent_id, (wordid - start) as isize + 1);
            current.push(FWord {
                word: s.word.clone(),
                kind: t,
                units: s.units.clone(),
                wordid,
                subsent_id: sub,
            });
            if wordid == n && current.len() >= 3 && j > 1 {
                let key = format!("{tile}_{}_{dir}", lpre(sent_id, j));
                if !existing_finals.insert(key) {
                    cleared = true;
                }
            }
        } else {
            if t == wk::NOUN && prevtype == wk::NOUN && !notids.is_empty() {
                notids.clear();
            }
            if current.len() >= 3 && j > 1 {
                let key = format!("{tile}_{}_{dir}", lpre(sent_id, j - 1));
                if !existing_finals.insert(key) {
                    cleared = true;
                }
            }
            if wordid < n {
                let push =
                    |fws: &mut Vec<FirstWord>, units: Vec<Handle>, index: usize, sid: String| {
                        fws.push(FirstWord {
                            units,
                            dir,
                            sent: Some(sent.clone()),
                            index,
                            sent_id: sid,
                        });
                    };
                if wordid > start {
                    if !notids.is_empty()
                        && firstreal
                        && notslot > 1
                        && (t != wk::INFIX || prevtype == wk::NOUN)
                        && (t != wk::VERB || prevtype == wk::NOUN)
                    {
                        let sid = lsub(sent_id, (notslot - start) as isize + 1);
                        push(fws, notids.clone(), notslot, sid);
                        if let Some(nx) = next
                            && (nx.kind == wk::NOUN || nx.kind == wk::PREFIX || nx.kind == wk::NOT)
                            && t != wk::PREFIX
                        {
                            push(fws, s.units.clone(), wordid, lsub(sent_id, j as isize));
                        }
                    } else if prevtype == wk::NOUN && (t == wk::VERB || t == wk::INFIX) {
                        let sid = lsub(sent_id, (wordid - start) as isize);
                        push(fws, sent[wordid - 2].units.clone(), wordid - 1, sid);
                    } else if prevsafe_type == wk::NOUN
                        && prevsafe_id > 0
                        && prevtype == wk::NOT
                        && t != wk::VERB
                        && t != wk::PROP
                    {
                        let sid = lsub(sent_id, (prevsafe_id - start) as isize + 1);
                        push(fws, sent[prevsafe_id - 1].units.clone(), prevsafe_id, sid);
                    } else {
                        push(fws, s.units.clone(), wordid, lsub(sent_id, j as isize));
                    }
                    break;
                } else if wordid == start {
                    if let Some(nx) = next {
                        push(
                            fws,
                            nx.units.clone(),
                            wordid + 1,
                            lsub(sent_id, j as isize + 1),
                        );
                    }
                    break;
                }
            }
        }
        wordid += 1;
    }
    if cleared { Vec::new() } else { current }
}

/// A word (with `not ` prefix) and the units that formed it.
type WordIds = (String, Vec<Handle>);

enum Cur {
    Subj,
    Target(usize),
    Cond(usize),
}

/// Rule extraction (§11) from the finals of one firstword.
fn extract(
    b: &mut Build<'_>,
    finals: &[(Vec<FWord>, Rc<Vec<SWord>>)],
    dir: u8,
    fws: &mut Vec<FirstWord>,
) {
    for (i, (sentence, sent)) in finals.iter().enumerate() {
        if sentence.len() < 3 {
            continue;
        }
        let mut valid = true;
        if finals.len() > 1 {
            for (a, (other, _)) in finals.iter().enumerate() {
                if a > i && other.len() == sentence.len() {
                    let identical = other.iter().zip(sentence).all(|(o, s)| {
                        o.units
                            .iter()
                            .enumerate()
                            .all(|(e, f)| s.units.get(e) == Some(f))
                    });
                    if identical {
                        valid = false;
                    }
                }
            }
        }
        if !valid {
            continue;
        }
        let mut subjects: Vec<WordIds> = Vec::new();
        let mut targets: Vec<(WordIds, Vec<WordIds>)> = Vec::new();
        let mut conds: Vec<(WordIds, Vec<WordIds>)> = Vec::new();
        let mut cur = Cur::Subj;
        let mut prefix = "";
        let mut allowed: Vec<i32> = vec![0];
        let mut allowed_extra: Vec<String> = Vec::new();
        let mut extraids: Vec<Vec<Handle>> = Vec::new();
        let mut ifvalid: Vec<Vec<Handle>> = Vec::new();
        let mut extra_cur = String::new();
        let len = sentence.len();
        for (index0, w) in sentence.iter().enumerate() {
            let index = index0 + 1;
            let cat = if w.kind == wk::VERB || w.kind == wk::PREFIX || w.kind == wk::INFIX {
                1
            } else if w.kind != wk::NOT && w.kind != wk::AND {
                0
            } else {
                ifvalid.push(w.units.clone());
                extra_cur = w.word.clone();
                -1
            };
            if cat == 0 {
                let ok = allowed.contains(&w.kind) || allowed_extra.contains(&w.word);
                if ok {
                    let e = (format!("{prefix}{}", w.word), w.units.clone());
                    match cur {
                        Cur::Subj => subjects.push(e),
                        Cur::Target(t) => targets[t].1.push(e),
                        Cur::Cond(c) => conds[c].1.push(e),
                    }
                } else {
                    fws.push(FirstWord {
                        units: vec![w.units[0]],
                        dir,
                        sent: Some(sent.clone()),
                        index: w.wordid,
                        sent_id: w.subsent_id.clone(),
                    });
                    break;
                }
            } else if cat == 1 && index < len {
                allowed = vec![0];
                allowed_extra.clear();
                let Some((at, ae)) = b.lvl.defs.word_args(&w.word) else {
                    // No definition: the reference stops collecting here
                    // but still emits what it has.
                    break;
                };
                let (at, ae) = (at.to_vec(), ae.to_vec());
                let e = (format!("{prefix}{}", w.word), w.units.clone());
                if w.kind == wk::VERB {
                    allowed = at;
                    targets.push((e, Vec::new()));
                    cur = Cur::Target(targets.len() - 1);
                } else if w.kind == wk::PREFIX {
                    conds.push((e, Vec::new()));
                } else {
                    allowed = at;
                    allowed_extra = ae;
                    conds.push((e, Vec::new()));
                    cur = Cur::Cond(conds.len() - 1);
                }
            }
            if w.kind == wk::NOT {
                prefix = if prefix == "not " { "" } else { "not " };
            } else {
                prefix = "";
            }
            if w.word != extra_cur && !extra_cur.is_empty() && w.kind != wk::NOT {
                extraids.append(&mut ifvalid);
                extra_cur.clear();
            }
        }
        let mut cl: Vec<Cond> = Vec::new();
        let mut condids: Vec<Vec<Handle>> = Vec::new();
        for ((name, u), params) in &conds {
            condids.push(u.clone());
            let mut ps = Vec::new();
            for (p, pu) in params {
                condids.push(pu.clone());
                ps.push(p.clone());
            }
            cl.push(Cond::new(name.clone(), ps));
        }
        for (s, su) in &subjects {
            for ((v, vu), args) in &targets {
                for (o, ou) in args {
                    let mut ids = vec![su.clone(), vu.clone(), ou.clone()];
                    ids.extend(extraids.iter().cloned());
                    ids.extend(condids.iter().cloned());
                    b.add(s, v, o, cl.clone(), ids, true, None, vec![]);
                }
            }
        }
    }
}
