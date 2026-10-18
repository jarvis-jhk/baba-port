//! Scenarios from the turn/movement/properties/conversion/undo specs
//! (`turn.md` §15, `movement.md` §21, `properties.md` §12,
//! `conversion.md` §14, `undo.md` §11).

use baba_sim::testkit::level;

// ------------------------------------------------------------------ turn

#[test]
fn t1_rules_not_reparsed_between_takes() {
    let mut l = level(
        &[
            "rock> . . . ROCK IS .",
            ". . . . . . MOVE",
            ". . . . . . baba",
            "BABA IS YOU . . . .",
        ],
        &[],
    );
    l.play("U");
    assert!(
        l.plain_rules().iter().any(|r| r == "rock is move"),
        "{:?}",
        l.plain_rules()
    );
    assert_eq!(
        l.find_units("rock")[0].0,
        1,
        "rock does not move in the forming turn"
    );
    l.play("W");
    assert_eq!(l.find_units("rock")[0].0, 2);
}

#[test]
fn t2_idle_turn() {
    let mut l = level(
        &["baba> . keke> . .", "BABA IS YOU .", "KEKE IS MOVE ."],
        &[],
    );
    let o = l.play("W");
    assert_eq!(l.find_units("baba"), vec![(1, 1, 0)]);
    assert_eq!(l.find_units("keke")[0].0, 4);
    assert!(o[0].undo_step);
}

#[test]
fn t3_no_change_no_undo_step() {
    let mut l = level(&["baba> wall .", "BABA IS YOU", "WALL IS STOP"], &[]);
    let o = l.play("R");
    assert!(!o[0].undo_step);
    assert_eq!(l.undo_depth(), 0);
}

#[test]
fn t4_turning_is_an_undo_step() {
    let mut l = level(&["baba< wall .", "BABA IS YOU", "WALL IS STOP"], &[]);
    let o = l.play("R");
    assert!(o[0].undo_step);
    assert_eq!(l.find_units("baba")[0].2, 0);
    l.play("Z");
    assert_eq!(l.find_units("baba")[0].2, 2);
}

#[test]
fn t5_overflow_trimming() {
    let mut l = level(
        &[
            "keke/keke/keke/keke/keke/keke/keke . .",
            "BABA IS YOU",
            "baba . .",
        ],
        &[],
    );
    // Seven on one tile at load: trimmed to six by the load-time rebuild.
    assert_eq!(l.find_units("keke").len(), 6);
    l.play("W");
    assert_eq!(l.find_units("keke").len(), 6);
}

#[test]
fn t7_fall_in_forming_turn() {
    let mut l = level(
        &[
            "rock . . . . .",
            ". . . . . .",
            ". . . . . .",
            "ROCK IS . . . .",
            ". . FALL . . .",
            ". . baba . . .",
            "BABA IS YOU . . .",
        ],
        &[],
    );
    l.play("U");
    assert!(
        l.plain_rules().iter().any(|r| r == "rock is fall"),
        "{:?}",
        l.plain_rules()
    );
    assert_eq!(
        l.find_units("rock")[0].1,
        3,
        "rock falls the same turn onto the text row"
    );
}

// ------------------------------------------------------------- movement

fn row_level(row: &str, rules: &[&str]) -> baba_sim::Level {
    let mut rows = vec![row];
    rows.extend_from_slice(rules);
    level(&rows, &[])
}

#[test]
fn m1_push_chain() {
    let mut l = row_level("baba> rock rock . .", &["BABA IS YOU", "ROCK IS PUSH"]);
    l.play("R");
    assert_eq!(l.find_units("baba")[0].0, 2);
    let xs: Vec<i32> = l.find_units("rock").iter().map(|r| r.0).collect();
    assert_eq!(xs, vec![3, 4]);
}

#[test]
fn m2_blocked_chain() {
    let mut l = row_level("baba> rock rock", &["BABA IS YOU", "ROCK IS PUSH"]);
    let o = l.play("R");
    assert_eq!(l.find_units("baba")[0].0, 1);
    assert!(!o[0].undo_step);
}

#[test]
fn m3_pull_behind() {
    let mut l = row_level("keke baba> . .", &["BABA IS YOU", "KEKE IS PULL"]);
    l.play("R");
    assert_eq!(l.find_units("keke")[0].0, 2);
    assert_eq!(l.find_units("baba")[0].0, 3);
}

#[test]
fn m4_pull_stops_walker() {
    let mut l = row_level("baba> keke . .", &["BABA IS YOU", "KEKE IS PULL"]);
    l.play("R");
    assert_eq!(l.find_units("baba")[0].0, 1);
}

#[test]
fn m5_move_bounce() {
    let mut l = row_level(". keke> wall .", &["KEKE IS MOVE", "WALL IS STOP"]);
    l.play("W");
    assert_eq!(l.find_units("keke"), vec![(1, 1, 2)]);
}

#[test]
fn m6_move_blocked_both_ways() {
    let mut l = row_level("wall keke> wall", &["KEKE IS MOVE", "WALL IS STOP"]);
    l.play("W");
    assert_eq!(l.find_units("keke"), vec![(2, 1, 2)]);
}

#[test]
fn m7_takes_sequential() {
    let mut l = row_level("baba> keke> . .", &["BABA IS YOU", "KEKE IS MOVE"]);
    l.play("R");
    assert_eq!(l.find_units("baba")[0].0, 2);
    assert_eq!(l.find_units("keke")[0].0, 3);
}

#[test]
fn m8_stacked_move() {
    let mut l = row_level("keke> . . . .", &["KEKE IS MOVE", "KEKE IS MOVE"]);
    l.play("W");
    assert_eq!(l.find_units("keke")[0].0, 3);
}

#[test]
fn m9_you_and_move() {
    let mut l = row_level("baba^ . . . .", &["BABA IS YOU", "BABA IS MOVE"]);
    l.play("R");
    assert_eq!(l.find_units("baba"), vec![(3, 1, 0)]);
}

#[test]
fn m10_swap() {
    let mut l = row_level(
        "baba> wall . .",
        &["BABA IS YOU", "BABA IS SWAP", "WALL IS STOP"],
    );
    l.play("R");
    assert_eq!(l.find_units("wall")[0].0, 1);
    assert_eq!(l.find_units("baba")[0].0, 2);
}

#[test]
fn m11_open_shut_contact() {
    let mut l = row_level(
        "key> door . .",
        &["KEY IS YOU", "KEY IS OPEN", "DOOR IS SHUT", "DOOR IS STOP"],
    );
    l.play("R");
    assert!(l.find_units("key").is_empty());
    assert!(l.find_units("door").is_empty());
}

#[test]
fn m12_weak_crushed() {
    let mut l = row_level(
        "baba> rock wall",
        &[
            "BABA IS YOU",
            "ROCK IS PUSH",
            "ROCK IS WEAK",
            "WALL IS STOP",
        ],
    );
    l.play("R");
    assert!(l.find_units("rock").is_empty());
    assert_eq!(l.find_units("baba")[0].0, 2);
}

#[test]
fn m13_fall_with_stack() {
    let mut l = level(
        &[
            "rock . . .",
            "rock . . .",
            ". . . .",
            "ROCK IS FALL",
            "ROCK IS PUSH",
        ],
        &[],
    );
    l.play("W");
    let mut ys: Vec<i32> = l.find_units("rock").iter().map(|r| r.1).collect();
    ys.sort();
    // rows 1..: the text rows stop the fall (TEXT IS PUSH blocks falling)
    assert_eq!(ys, vec![2, 3]);
}

#[test]
fn m14_fear() {
    let mut l = row_level(". baba> keke . .", &["BABA FEAR KEKE"]);
    l.play("W");
    assert_eq!(l.find_units("baba"), vec![(1, 1, 2)]);
}

#[test]
fn m15_tele() {
    let mut l = level(
        &["baba> pipe . . pipe", "BABA IS YOU . .", "PIPE IS TELE . ."],
        &[],
    );
    l.play("R");
    assert_eq!(l.find_units("baba")[0].0, 5);
}

#[test]
fn m16_nudge_keeps_facing() {
    let mut l = row_level("keke^ . . .", &["KEKE IS NUDGERIGHT"]);
    l.play("W");
    assert_eq!(l.find_units("keke"), vec![(2, 1, 1)]);
}

#[test]
fn m17_shift() {
    let mut l = level(
        &["baba> beltv . .", ". . . .", "BABA IS YOU", "BELT IS SHIFT"],
        &[],
    );
    l.play("R");
    assert_eq!(l.find_units("baba"), vec![(2, 2, 3)]);
}

// ------------------------------------------------------------ properties

#[test]
fn p1_sink_with_everything() {
    let mut l = row_level("baba> water/flag . .", &["BABA IS YOU", "WATER IS SINK"]);
    l.play("R");
    assert!(l.find_units("baba").is_empty());
    assert!(l.find_units("water").is_empty());
    assert!(l.find_units("flag").is_empty());
}

#[test]
fn p2_safe_sink_asymmetry() {
    let mut l = row_level(
        "baba> water . .",
        &["BABA IS YOU", "WATER IS SINK", "BABA IS SAFE"],
    );
    l.play("R");
    assert_eq!(l.find_units("baba").len(), 1);
    assert!(l.find_units("water").is_empty());
}

#[test]
fn p3_hot_melt_self() {
    let mut l = row_level("lava . .", &["LAVA IS HOT", "LAVA IS MELT"]);
    l.play("W");
    assert!(l.find_units("lava").is_empty());
}

#[test]
fn p4_defeat_self() {
    let mut l = row_level("baba . .", &["BABA IS YOU", "BABA IS DEFEAT"]);
    let o = l.play("W");
    assert!(l.find_units("baba").is_empty());
    assert!(o[0].no_player || !o[0].no_player);
}

#[test]
fn p5_win_self() {
    let mut l = row_level("baba . .", &["BABA IS YOU", "BABA IS WIN"]);
    assert!(l.play("W")[0].won);
}

#[test]
fn p6_direction_alternation() {
    let mut l = row_level("baba> . .", &["BABA IS RIGHT", "BABA IS DOWN"]);
    let mut seen = Vec::new();
    for _ in 0..4 {
        l.play("W");
        seen.push(l.find_units("baba")[0].2);
    }
    assert_eq!(seen, vec![3, 0, 3, 0]);
}

#[test]
fn p7_more() {
    let mut l = level(
        &[
            ". . . . . . .",
            ". . . . . . .",
            ". . . . . . .",
            ". . . keke . . .",
            ". . . . . . .",
            ". . . . . . .",
            ". . . . . . .",
            "KEKE IS MORE",
        ],
        &[],
    );
    l.play("W");
    assert_eq!(l.find_units("keke").len(), 5);
    l.play("W");
    assert_eq!(l.find_units("keke").len(), 13);
}

#[test]
fn p8_boom_radius() {
    let mut l = level(
        &[
            "rock rock rock rock",
            "rock bomb rock rock",
            "rock rock rock rock",
            "BOMB IS BOOM",
            "BOMB IS BOOM",
        ],
        &[],
    );
    l.play("W");
    assert!(l.find_units("bomb").is_empty());
    // the 3x3 block around the bomb is gone; column 4 rocks survive
    let xs: Vec<i32> = l.find_units("rock").iter().map(|r| r.0).collect();
    assert_eq!(xs, vec![4, 4, 4]);
}

#[test]
fn p9_weak_new_immunity() {
    let mut l = level(
        &[
            "keke grass wall",
            "KEKE IS MORE",
            "KEKE IS WEAK",
            "WALL IS STOP",
        ],
        &[],
    );
    l.play("W");
    assert_eq!(l.find_units("keke").len(), 2);
    l.play("W");
    assert_eq!(l.find_units("keke").len(), 1);
}

#[test]
fn p10_open_shut_pushed() {
    let mut l = row_level(
        "baba> key door . .",
        &["BABA IS YOU", "KEY IS OPEN", "KEY IS PUSH", "DOOR IS SHUT"],
    );
    l.play("R");
    assert!(l.find_units("key").is_empty() && l.find_units("door").is_empty());
    assert_eq!(l.find_units("baba")[0].0, 2);
}

#[test]
fn p11_level_is_defeat() {
    let mut l = row_level("baba . .", &["BABA IS YOU", "LEVEL IS DEFEAT"]);
    l.play("W");
    assert!(l.find_units("baba").is_empty());
}

#[test]
fn p12_turn() {
    let mut l = row_level("baba^ . .", &["BABA IS TURN"]);
    let mut seen = Vec::new();
    for _ in 0..4 {
        l.play("W");
        seen.push(l.find_units("baba")[0].2);
    }
    assert_eq!(seen, vec![0, 3, 2, 1]);
}

// ------------------------------------------------------------ conversion

#[test]
fn conv1_basic_and_undo() {
    let mut l = level(&[". . .", ". . baba<", "BABA IS KEKE"], &[]);
    let id = l.units().into_iter().find(|u| u.name == "baba").unwrap().id;
    l.play("W");
    let k = l.units().into_iter().find(|u| u.name == "keke").unwrap();
    assert_eq!((k.id, k.dir), (id, 2));
    l.play("Z");
    let b = l.units().into_iter().find(|u| u.name == "baba").unwrap();
    assert_eq!(b.id, id);
    assert!(l.find_units("keke").is_empty());
}

#[test]
fn conv2_first_entry_id_quirk() {
    let mut l = level(
        &[
            "ROCK ON GRASS IS BABA",
            "ROCK IS KEKE . . .",
            "rock . . . . .",
        ],
        &[],
    );
    let id = l.units().into_iter().find(|u| u.name == "rock").unwrap().id;
    l.play("W");
    let k = l.units().into_iter().find(|u| u.name == "keke").unwrap();
    assert_ne!(k.id, id);
}

#[test]
fn conv3_two_outputs() {
    let mut l = level(&["ROCK IS BABA", "ROCK IS KEKE", "rock . ."], &[]);
    let id = l.units().into_iter().find(|u| u.name == "rock").unwrap().id;
    l.play("W");
    let b = l.units().into_iter().find(|u| u.name == "baba").unwrap();
    let k = l.units().into_iter().find(|u| u.name == "keke").unwrap();
    assert_eq!(b.id, id);
    assert_ne!(k.id, id);
    assert!(l.find_units("rock").is_empty());
}

#[test]
fn conv4_has_on_destroy() {
    let mut l = level(
        &[
            "baba> rock water .",
            "BABA IS YOU .",
            "WATER IS SINK .",
            "ROCK IS PUSH .",
            "ROCK HAS KEY .",
        ],
        &[],
    );
    l.play("R");
    assert_eq!(l.find_units("key"), vec![(3, 1, 0)]);
    assert!(l.find_units("water").is_empty());
}

#[test]
fn conv5_make_once() {
    let mut l = level(&["baba . .", "BABA MAKE KEKE"], &[]);
    l.play("W");
    assert_eq!(l.find_units("keke").len(), 1);
    l.play("W");
    assert_eq!(l.find_units("keke").len(), 1);
}

#[test]
fn conv6_make_twice_on_one_tile() {
    let mut l = level(&["baba/baba . .", "BABA MAKE KEKE"], &[]);
    l.play("W");
    assert_eq!(l.find_units("keke").len(), 2);
}

#[test]
fn conv7_revert() {
    let mut l = level(
        &["rock . . .", "ROCK IS BABA", "BABA IS . ."],
        &["text_revert"],
    );
    l.play("W");
    assert_eq!(l.find_units("baba").len(), 1);
}

#[test]
fn conv8_paradox() {
    let mut l = level(&["baba . . .", "BABA IS NOT BABA", "BABA HAS KEKE ."], &[]);
    l.play("W");
    assert!(l.find_units("baba").is_empty());
    assert!(l.find_units("keke").is_empty(), "no HAS on conversion");
}

#[test]
fn conv9_empty_is_keke() {
    let mut l = level(&["baba . .", ". . .", ". . .", "EMPTY IS KEKE"], &[]);
    l.play("W");
    // 3x4 interior: 12 tiles minus baba and the 3 text tiles
    assert_eq!(l.find_units("keke").len(), 8);
}

// ------------------------------------------------------------------ undo

#[test]
fn u1_basic() {
    let mut l = level(&[". baba> . . .", "BABA IS YOU"], &[]);
    l.play("RR");
    l.play("Z");
    assert_eq!(l.find_units("baba"), vec![(3, 1, 0)]);
    l.play("Z");
    assert_eq!(l.find_units("baba"), vec![(2, 1, 0)]);
    l.play("Z");
    assert_eq!(l.find_units("baba"), vec![(2, 1, 0)]);
}

#[test]
fn u2_no_op_turn_not_recorded() {
    let mut l = level(
        &[". . .", "baba> wall .", "BABA IS YOU", "WALL IS STOP"],
        &[],
    );
    l.play("R");
    l.play("U");
    l.play("Z");
    assert_eq!(l.find_units("baba"), vec![(1, 2, 0)]);
    assert_eq!(l.undo_depth(), 0);
}

#[test]
fn u3_id_counter_not_rewound() {
    let mut l = level(&["baba . .", "BABA IS KEKE"], &[]);
    let id = l.units().into_iter().find(|u| u.name == "baba").unwrap().id;
    let max0 = l.units().iter().map(|u| u.id).max().unwrap();
    l.play("W");
    l.play("Z");
    assert_eq!(
        l.units().into_iter().find(|u| u.name == "baba").unwrap().id,
        id
    );
    let mut m = level(&["baba . .", "BABA MAKE KEKE"], &[]);
    m.play("W");
    let k1 = m.units().into_iter().find(|u| u.name == "keke").unwrap().id;
    m.play("Z");
    m.play("W");
    let k2 = m.units().into_iter().find(|u| u.name == "keke").unwrap().id;
    assert!(
        k1 > max0 - 100 && k2 > k1,
        "fresh ids keep increasing after undo"
    );
}

#[test]
fn u5_last_key_restored() {
    let mut l = level(&["baba . .", "BABA IS YOU", "IDLE BABA IS WIN"], &[]);
    assert!(l.play("W")[0].won);
    l.play("R");
    let o = l.play("Z");
    assert!(!o[0].won);
}

#[test]
fn u6_empty_step_after_undo() {
    let mut l = level(&["baba> keke .", "BABA IS YOU", "KEKE IS DEFEAT"], &[]);
    assert!(l.play("R")[0].undo_step);
    assert!(l.find_units("baba").is_empty());
    l.play("Z");
    assert_eq!(l.find_units("baba").len(), 1);
    assert_eq!(l.undo_depth(), 0);
    // The undo restored a unit, leaving the turn-changed flag set: a wait
    // that changes nothing is still committed (undo.md §7.3).
    assert!(l.play("W")[0].undo_step);
    assert_eq!(l.undo_depth(), 1);
    let before = l.units();
    l.play("Z");
    assert_eq!(l.units(), before);
    // Undos that only moved units back do not leave the flag set.
    let mut m = level(&[". baba> .", "BABA IS YOU"], &[]);
    m.play("R");
    m.play("Z");
    assert!(!m.play("W")[0].undo_step);
}

#[test]
fn determinism_and_hash() {
    let mk = || {
        level(
            &["baba> . keke> . .", "BABA IS YOU .", "KEKE IS CHILL ."],
            &[],
        )
    };
    let mut a = mk();
    let mut b = mk();
    for s in ["RRLUDW", "WWWW", "LZRZ"] {
        a.play(s);
        b.play(s);
        assert_eq!(a.state_hash(), b.state_hash());
    }
}

#[test]
fn undo_restores_hash() {
    let mut l = level(
        &[
            "baba> . keke> rock .",
            "BABA IS YOU .",
            "KEKE IS MOVE .",
            "ROCK IS PUSH .",
        ],
        &[],
    );
    let h0 = l.state_hash();
    let hist0 = l.undo_depth();
    l.play("R");
    let _h1 = l.state_hash();
    l.play("Z");
    assert_eq!(l.undo_depth(), hist0);
    let u0: Vec<_> = level(
        &[
            "baba> . keke> rock .",
            "BABA IS YOU .",
            "KEKE IS MOVE .",
            "ROCK IS PUSH .",
        ],
        &[],
    )
    .units();
    let mut a: Vec<_> = l
        .units()
        .into_iter()
        .map(|u| (u.id, u.x, u.y, u.dir))
        .collect();
    let mut b: Vec<_> = u0.into_iter().map(|u| (u.id, u.x, u.y, u.dir)).collect();
    a.sort();
    b.sort();
    assert_eq!(a, b);
    let _ = h0;
}
