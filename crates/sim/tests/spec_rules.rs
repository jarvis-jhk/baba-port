//! Scenarios from the words/rules/conditions specs (`words.md` §7,
//! `rules.md` §22, `conditions.md` §9).

use baba_sim::testkit::level;

fn has(rules: &[String], r: &str) -> bool {
    rules.iter().any(|x| x == r)
}

#[test]
fn w1_text_is_win_applies_to_text() {
    // BABA text is YOU-like only via TEXT rules; here TEXT IS WIN + TEXT IS YOU
    let mut l = level(
        &["BABA IS YOU . .", "TEXT IS WIN . .", "TEXT IS YOU . ."],
        &[],
    );
    let out = l.play("W");
    assert!(out[0].won, "a text unit that is YOU and WIN wins");
}

#[test]
fn w2_known_from_text_and_all() {
    let l = level(&["KEKE IS YOU", "ALL IS PUSH", ". . ."], &[]);
    assert!(
        has(&l.plain_rules(), "keke is push"),
        "{:?}",
        l.plain_rules()
    );
}

#[test]
fn w3_known_set_is_monotonic() {
    let mut l = level(
        &[
            "BABA IS YOU . .",
            "ALL IS YOU . .",
            "ROCK IS WEAK . .",
            "baba/rock . . . .",
        ],
        &[],
    );
    l.play("W");
    assert!(l.find_units("rock").is_empty());
    assert!(
        has(&l.plain_rules(), "rock is you"),
        "{:?}",
        l.plain_rules()
    );
}

#[test]
fn w4_letters_horizontal() {
    let l = level(&["l:b l:a l:b l:a IS YOU"], &["text_baba"]);
    assert!(
        has(&l.plain_rules(), "baba is you"),
        "{:?}",
        l.plain_rules()
    );
    let r = l
        .rules()
        .features
        .iter()
        .find(|r| r.subject == "baba")
        .unwrap();
    assert_eq!(r.ids[0].len(), 4, "subject ids are the four letters");
}

#[test]
fn w5_letters_vertical() {
    let l = level(&["BABA", "IS", "l:y", "l:o", "l:u"], &["text_you"]);
    assert!(
        has(&l.plain_rules(), "baba is you"),
        "{:?}",
        l.plain_rules()
    );
}

#[test]
fn w6_letters_spell_verb() {
    let l = level(&["BABA l:i l:s YOU"], &["text_is"]);
    assert!(
        has(&l.plain_rules(), "baba is you"),
        "{:?}",
        l.plain_rules()
    );
}

#[test]
fn w7_stacked_letters() {
    let l = level(&["l:b l:a l:b l:a/l:e IS YOU"], &["text_baba"]);
    assert!(has(&l.plain_rules(), "baba is you"));
    assert!(!l.plain_rules().iter().any(|r| r.starts_with("babe")));
}

#[test]
fn w10_letters_add_known_noun() {
    let l = level(
        &["l:f l:l l:a l:g IS YOU", "ALL IS PUSH . . . ."],
        &["flag", "text_flag"],
    );
    assert!(
        has(&l.plain_rules(), "flag is push"),
        "{:?}",
        l.plain_rules()
    );
}

#[test]
fn r1_basic_and_base_rules() {
    let l = level(&["BABA IS YOU"], &[]);
    let all = l.rule_strings();
    assert_eq!(
        &all[..3],
        &["text is push", "level is stop", "cursor is select"]
    );
    assert_eq!(l.plain_rules(), vec!["baba is you"]);
}

#[test]
fn r3_shared_word_horizontal_first() {
    let l = level(&["BABA IS YOU", "IS . .", "PUSH . ."], &[]);
    assert_eq!(l.plain_rules(), vec!["baba is you", "baba is push"]);
}

#[test]
fn r4_stacked_subjects() {
    let l = level(&["BABA/KEKE IS YOU"], &[]);
    assert_eq!(l.plain_rules(), vec!["baba is you", "keke is you"]);
}

#[test]
fn r5_and() {
    let l = level(&["BABA AND KEKE IS YOU AND PUSH"], &[]);
    assert_eq!(
        l.plain_rules(),
        vec!["baba is you", "baba is push", "keke is you", "keke is push"]
    );
}

#[test]
fn r6_break() {
    let l = level(&["BABA IS YOU KEKE IS PUSH"], &[]);
    assert_eq!(l.plain_rules(), vec!["baba is you", "keke is push"]);
}

#[test]
fn r7_verb_chain_restart() {
    let l = level(&["BABA IS KEKE IS YOU"], &[]);
    let r = l.plain_rules();
    assert!(has(&r, "baba is keke") && has(&r, "keke is you"), "{r:?}");
}

#[test]
fn r8_not_restart() {
    let l = level(&["BABA IS YOU NOT KEKE IS PUSH", "rock . . . . . ."], &[]);
    let r = l.plain_rules();
    assert!(has(&r, "baba is you"), "{r:?}");
    assert!(has(&r, "not keke is push"), "{r:?}");
    assert!(has(&r, "rock is push") && has(&r, "baba is push"), "{r:?}");
    assert!(!has(&r, "keke is push"));
}

#[test]
fn r9_leading_junk() {
    let l = level(&["IS BABA IS YOU"], &[]);
    assert_eq!(l.plain_rules(), vec!["baba is you"]);
}

#[test]
fn r10_too_short() {
    for row in ["BABA IS . .", "IS YOU . .", "BABA YOU . ."] {
        let l = level(&[row], &[]);
        assert!(l.plain_rules().is_empty(), "{row}: {:?}", l.plain_rules());
    }
}

#[test]
fn r11_argument_kinds() {
    let l = level(&["BABA HAS YOU"], &[]);
    assert!(l.plain_rules().is_empty(), "{:?}", l.plain_rules());
    let l = level(&["BABA HAS KEY AND YOU"], &[]);
    assert_eq!(l.plain_rules(), vec!["baba has key"]);
}

#[test]
fn r12_prefix_and_infix() {
    let l = level(&["LONELY BABA ON TILE IS YOU"], &[]);
    assert_eq!(l.plain_rules(), vec!["baba is you [lonely,on(tile)]"]);
}

#[test]
fn r13_double_not() {
    let l = level(&["NOT NOT BABA IS YOU"], &[]);
    assert_eq!(l.plain_rules(), vec!["baba is you"]);
}

#[test]
fn r14_not_disables() {
    let l = level(&["BABA IS YOU . .", "BABA IS NOT YOU"], &[]);
    let r = l
        .rules()
        .features
        .iter()
        .find(|r| r.object == "you")
        .unwrap();
    assert!(r.disabled());
}

#[test]
fn r15_conditional_not() {
    let l = level(&["BABA IS YOU . . . .", "BABA ON ROCK IS NOT YOU"], &[]);
    let r = l
        .rules()
        .features
        .iter()
        .find(|r| r.object == "you")
        .unwrap();
    assert_eq!(r.display(), "baba is you [(not on)(rock)]");
}

#[test]
fn r16_text_is_not_push() {
    let l = level(&["TEXT IS NOT PUSH"], &[]);
    assert!(l.rules().features[0].disabled());
}

#[test]
fn r17_protection() {
    let l = level(&["BABA IS BABA", "BABA IS KEKE"], &[]);
    let r = l
        .rules()
        .features
        .iter()
        .find(|r| r.object == "keke")
        .unwrap();
    assert!(r.disabled());
    let l = level(
        &["BABA IS BABA . .", "BABA IS KEKE . .", "BABA IS NOT BABA"],
        &[],
    );
    let r = l
        .rules()
        .features
        .iter()
        .find(|r| r.object == "keke")
        .unwrap();
    let mut l2 = l.clone();
    assert!(!r.disabled());
    // and the conversion happens
    let mut l3 = level(
        &[
            "BABA IS BABA . .",
            "BABA IS KEKE . .",
            "BABA IS NOT BABA",
            "baba . . .",
        ],
        &[],
    );
    l3.play("W");
    let _ = l2.play("W");
    assert!(l3.find_units("baba").is_empty());
}

#[test]
fn r18_all_expansion() {
    let l = level(&["ALL IS PUSH", "baba rock ."], &[]);
    let r = l.plain_rules();
    assert!(has(&r, "baba is push") && has(&r, "rock is push"), "{r:?}");
    assert!(!has(&r, "text is push") && !has(&r, "level is push"));
}

#[test]
fn r19_not_subject() {
    let l = level(&["NOT BABA IS STOP", "keke rock . ."], &[]);
    let r = l.plain_rules();
    assert!(has(&r, "keke is stop") && has(&r, "rock is stop"), "{r:?}");
    assert!(!has(&r, "baba is stop"));
}

#[test]
fn r20_mimic() {
    let l = level(&["KEKE MIMIC BABA", "BABA IS YOU .", "keke . ."], &[]);
    let r = l
        .rules()
        .features
        .iter()
        .find(|r| r.subject == "keke" && r.object == "you");
    assert!(
        r.is_some_and(|r| r.tags.iter().any(|t| t == "mimic")),
        "{:?}",
        l.rule_strings()
    );
    let l = level(
        &[
            "KEKE MIMIC BABA . .",
            "BABA IS YOU . .",
            "KEKE MIMIC NOT BABA",
        ],
        &[],
    );
    assert!(
        !l.rules()
            .features
            .iter()
            .any(|r| r.subject == "keke" && r.object == "you")
    );
}

#[test]
fn r21_groups() {
    let l = level(&["BABA IS GROUP", "GROUP IS PUSH"], &[]);
    assert!(
        has(&l.plain_rules(), "baba is push"),
        "{:?}",
        l.plain_rules()
    );
    let l = level(
        &[
            "BABA IS GROUP . .",
            "GROUP IS PUSH . .",
            "BABA IS NOT GROUP",
        ],
        &[],
    );
    assert!(
        !has(&l.plain_rules(), "baba is push"),
        "{:?}",
        l.plain_rules()
    );
}

#[test]
fn r22_group_is_not_group_crash() {
    let l = level(&["BABA IS GROUP . .", "GROUP IS NOT GROUP"], &[]);
    assert!(
        has(&l.plain_rules(), "baba is crash"),
        "{:?}",
        l.plain_rules()
    );
}

#[test]
fn r24_word_object() {
    let l = level(&["BABA IS WORD", "baba IS WIN"], &[]);
    let r = l.rules().features.iter().find(|r| r.object == "win");
    assert!(r.is_some(), "{:?}", l.rule_strings());
}

#[test]
fn r25_word_instability() {
    let l = level(&["baba IS WORD"], &[]);
    let active = l
        .rules()
        .features
        .iter()
        .filter(|r| r.object == "word" && !r.disabled())
        .count();
    assert_eq!(active, 0, "{:?}", l.rule_strings());
}

#[test]
fn r27_dirty_on_turning_text() {
    // A pushed text forms a rule only after the move phase refresh.
    let mut l = level(&["BABA IS . .", ". . YOU .", ". . baba^ ."], &[]);
    assert!(l.plain_rules().is_empty());
    l.play("U");
    assert_eq!(l.plain_rules(), Vec::<String>::new());
}

// ------------------------------------------------------------- conditions

#[test]
fn c1_on() {
    let mut l = level(
        &[
            "BABA IS YOU . . . .",
            "BABA ON TILE IS WIN",
            "baba> tile . . . .",
        ],
        &[],
    );
    assert!(!l.play("W")[0].won);
    assert!(l.play("R")[0].won);
}

#[test]
fn c2_on_distinct_units() {
    let mut l = level(
        &[
            "BABA IS YOU . . . . . .",
            "BABA ON KEKE AND KEKE IS WIN",
            "baba/keke . . . . . . .",
        ],
        &[],
    );
    assert!(!l.play("W")[0].won);
    let mut l = level(
        &[
            "BABA IS YOU . . . . . .",
            "BABA ON KEKE AND KEKE IS WIN",
            "baba/keke/keke . . . . . . .",
        ],
        &[],
    );
    assert!(l.play("W")[0].won);
}

#[test]
fn c7_facing() {
    let mut l = level(
        &[
            "BABA IS YOU . . . .",
            "BABA FACING KEKE IS WIN",
            "baba> keke . . . .",
        ],
        &[],
    );
    assert!(l.play("W")[0].won);
    let mut l = level(
        &[
            "BABA IS YOU . . . .",
            "BABA FACING RIGHT IS WIN",
            "baba> . . . . .",
        ],
        &[],
    );
    assert!(l.play("W")[0].won);
}

#[test]
fn c12_without() {
    let mut l = level(
        &[
            "BABA IS YOU . . . .",
            "BABA WITHOUT BABA IS WIN",
            "baba . . . . .",
        ],
        &[],
    );
    assert!(l.play("W")[0].won);
    let mut l = level(
        &[
            "BABA IS YOU . . . .",
            "BABA WITHOUT BABA IS WIN",
            "baba baba . . . .",
        ],
        &[],
    );
    assert!(!l.play("W")[0].won);
}

#[test]
fn c15_lonely() {
    let mut l = level(
        &["BABA IS YOU . .", "LONELY BABA IS WIN", "baba . . . ."],
        &[],
    );
    assert!(l.play("W")[0].won);
    let mut l = level(
        &["BABA IS YOU . .", "LONELY BABA IS WIN", "baba/tile . . . ."],
        &[],
    );
    assert!(!l.play("W")[0].won);
}

#[test]
fn c16_feeling_no_self_support() {
    let mut l = level(
        &[
            "BABA IS YOU . . .",
            "BABA FEELING YOU IS WIN",
            "baba . . . . .",
        ],
        &[],
    );
    assert!(l.play("W")[0].won);
    let l = level(&["BABA FEELING YOU IS YOU", "baba> . . . ."], &[]);
    let mut l2 = l.clone();
    l2.play("R");
    assert_eq!(l2.find_units("baba")[0].0, 1, "baba is not YOU");
}

#[test]
fn c18_powered() {
    let mut l = level(
        &["KEKE IS POWER", "POWERED BABA IS YOU", "baba> . . keke"],
        &[],
    );
    l.play("R");
    assert_eq!(l.find_units("baba")[0].0, 2);
}

#[test]
fn c19_idle() {
    let mut l = level(
        &["BABA IS YOU . .", "IDLE KEKE IS MOVE", "keke> . . . ."],
        &[],
    );
    l.play("R");
    assert_eq!(l.find_units("keke")[0].0, 1);
    l.play("W");
    assert_eq!(l.find_units("keke")[0].0, 2);
}

#[test]
fn c24_broken_gate() {
    let mut l = level(&["BABA IS YOU . .", "BABA IS BROKEN", "baba> . . . ."], &[]);
    l.play("R");
    assert_eq!(l.find_units("baba")[0].0, 1);
}

#[test]
fn c27_often_statistics() {
    let mut l = level(
        &["BABA IS YOU . .", "OFTEN BABA IS WIN", "baba . . . ."],
        &[],
    );
    let n = 2000;
    let wins = l.play(&"W".repeat(n)).iter().filter(|o| o.won).count();
    assert!((1300..1700).contains(&wins), "OFTEN won {wins}/{n}");
}
