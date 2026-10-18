//! AUTO turns, 3D mode and the presentation events (`turn.md` §3,
//! `movement.md` §19.2, `presentation.md` §7–§9, `audio.md` §4).

use baba_sim::testkit::level;
use baba_sim::{CosmeticRng, Event, Input, MoveKind, Particle, TextState, TurnKind};

// ------------------------------------------------------------------ AUTO

#[test]
fn auto_keys_only_store_until_the_tick() {
    let mut l = level(&["baba> . . . .", "BABA IS YOU", "LEVEL IS AUTO"], &[]);
    assert!(l.auto_active());
    let o = l.play("R");
    assert_eq!(o[0].kind, TurnKind::Stored);
    assert_eq!(l.find_units("baba")[0].0, 1, "a key alone does not move");
    let o = l.play("A");
    assert_eq!(o[0].kind, TurnKind::Turn);
    assert_eq!(
        l.find_units("baba")[0].0,
        2,
        "the tick applies the stored key"
    );
    l.play("A");
    assert_eq!(l.find_units("baba")[0].0, 2, "the store is cleared");
    l.play("Z");
    assert_eq!(l.find_units("baba")[0].0, 1);
}

#[test]
fn auto_runs_one_move_phase_per_rule() {
    let mut l = level(
        &[
            "keke> . . . . .",
            "LEVEL IS AUTO . .",
            "LEVEL IS AUTO . .",
            "KEKE IS MOVE . .",
        ],
        &[],
    );
    l.play("A");
    assert_eq!(l.find_units("keke")[0].0, 3);
}

#[test]
fn tick_without_auto_does_nothing() {
    let mut l = level(&["baba> . .", "BABA IS YOU"], &[]);
    let o = l.play("A");
    assert_eq!(o[0].kind, TurnKind::Stored);
    assert_eq!(l.undo_depth(), 0);
    l.play("R");
    assert_eq!(l.find_units("baba")[0].0, 2);
}

// ------------------------------------------------------------------ 3D

#[test]
fn camera_turns_moves_and_cycles() {
    let mut l = level(
        &[
            ". . . . . .",
            ". keke^ . . keke^ .",
            ". . . . . .",
            "KEKE IS 3D . .",
        ],
        &[],
    );
    let cam = l.camera().expect("3D on at level start");
    let first = cam.target.expect("a target");
    // Right turns the camera unit clockwise (up → right).
    l.play("R");
    let c = l.camera().unwrap();
    assert_eq!((c.dir, c.target), (0, Some(first)));
    // Up moves it forward.
    l.play("U");
    assert_eq!(l.camera().unwrap().x, cam.x + 1);
    // Down switches to the other keke and is an undo step by itself.
    let o = l.play("D");
    assert!(o[0].undo_step);
    let second = l.camera().unwrap().target.unwrap();
    assert_ne!(second, first);
    assert!(
        o[0].events
            .iter()
            .any(|e| matches!(e, Event::Camera { .. }))
    );
    l.play("Z");
    assert_eq!(l.camera().unwrap().target, Some(first));
}

#[test]
fn camera_left_where_its_unit_died() {
    let mut l = level(
        &[
            ". keke^ .",
            ". skull .",
            ". . .",
            "KEKE IS 3D",
            "SKULL IS DEFEAT",
        ],
        &[],
    );
    let start = l.camera().unwrap();
    // Down: the keke walks into the skull; the camera stays, fixed.
    l.play("L");
    l.play("L");
    l.play("U");
    let c = l.camera().expect("3D stays on");
    assert_eq!(c.target, None);
    assert_eq!((c.x, c.y), (start.x, start.y + 1));
    assert!(l.find_units("keke").is_empty());
    l.play("Z");
    assert_eq!(l.camera().unwrap().target, start.target);
}

// ------------------------------------------------------------------ events

#[test]
fn move_events_and_turn_sound() {
    let mut l = level(&["baba> rock . .", "BABA IS YOU", "ROCK IS PUSH"], &[]);
    let o = l.play("R").remove(0);
    let moved: Vec<_> = o
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Moved { from, to, kind, .. } => Some((*from, *to, *kind)),
            _ => None,
        })
        .collect();
    assert!(moved.contains(&((1, 1), (2, 1), MoveKind::Step)));
    assert!(moved.contains(&((2, 1), (3, 1), MoveKind::Step)));
    assert_eq!(o.turn_sound.unwrap().family, "move_hi");
    let o = l.play("R").remove(0);
    assert_eq!(o.turn_sound.unwrap().family, "move_hi");
    let o = l.play("Z").remove(0);
    assert_eq!(o.kind, TurnKind::Undo);
    assert!(o.events.iter().any(|e| matches!(
        e,
        Event::Moved {
            kind: MoveKind::Undo,
            ..
        }
    )));
}

#[test]
fn removal_sound_short_variant_and_particles() {
    let mut l = level(
        &[
            "baba> water . water .",
            "rock> . water . .",
            "BABA IS YOU . .",
            "WATER IS SINK . .",
            "ROCK IS YOU . .",
        ],
        &[],
    );
    let o = l.play("R").remove(0);
    let s = o.removal_sound.expect("sink sound");
    assert_eq!((s.family, s.short), ("plop", false));
    assert!(o.events.iter().any(|e| matches!(
        e,
        Event::Particles(b) if b.kind == Particle::Destroy && b.count == 15 && b.scale == 100
    )));
    // The same effect again the next turn: short sound, halved burst.
    let o = l.play("R").remove(0);
    let s = o.removal_sound.expect("sink sound");
    assert!(s.short, "{s:?}");
    assert!(o.events.iter().any(|e| matches!(
        e,
        Event::Particles(b) if b.kind == Particle::Destroy && b.scale == 50
    )));
}

#[test]
fn rule_formed_and_text_states() {
    let mut l = level(
        &[
            ". . . . .",
            "BABA IS YOU . .",
            ". . . . .",
            "ROCK IS . PUSH baba<",
            ". . . . .",
            "rock . . . .",
        ],
        &[],
    );
    l.play("W");
    let push = l
        .units()
        .into_iter()
        .find(|u| u.name == "text_push")
        .unwrap();
    assert_eq!(push.text, TextState::Inactive);
    // Baba pushes PUSH left into ROCK IS PUSH (not on the level's first
    // committed turn, which never shows new-rule effects).
    l.play("UD");
    let o = l.play("L").remove(0);
    let formed = o.events.iter().find_map(|e| match e {
        Event::RuleFormed { texts, sound } => Some((texts.clone(), *sound)),
        _ => None,
    });
    let (texts, sound) = formed.expect("rule formed");
    assert_eq!(texts.len(), 3);
    assert_eq!(sound.unwrap().family, "rule");
    let views = l.units();
    assert!(
        views
            .iter()
            .filter(|u| u.name.starts_with("text_"))
            .all(|u| u.text == TextState::Active)
    );
}

#[test]
fn disabled_rules_get_crosses() {
    let mut l = level(
        &["baba> . .", "BABA IS YOU", "BABA IS WIN", "BABA IS NOT WIN"],
        &[],
    );
    l.play("W");
    let states: Vec<(String, TextState)> = l
        .units()
        .into_iter()
        .filter(|u| u.y == 3)
        .map(|u| (u.name, u.text))
        .collect();
    assert!(
        states.iter().all(|(_, t)| *t == TextState::Disabled),
        "{states:?}"
    );
}

#[test]
fn colour_properties() {
    let mut l = level(
        &["baba> keke", "BABA IS RED", "BABA IS BLUE", "LEVEL IS PINK"],
        &["text_level"],
    );
    l.play("W");
    let baba = l.units().into_iter().find(|u| u.name == "baba").unwrap();
    assert_eq!(baba.colours, vec![(2, 2), (3, 2)]);
    assert_eq!(baba.colour, (2, 2));
    assert_eq!(l.level_colours(), vec![(4, 1)]);
}

#[test]
fn sound_names_use_the_cosmetic_generator() {
    let mut rng = CosmeticRng::new(7);
    let s = baba_sim::events::sounds::POP;
    for _ in 0..50 {
        let n = s.name(&mut rng);
        assert!(
            ["pop1", "pop2", "pop3", "pop4", "pop5"].contains(&n.as_str()),
            "{n}"
        );
    }
    assert_eq!(baba_sim::events::note_freq("c"), 28540);
    assert_eq!(baba_sim::events::note_freq("c6"), 57081);
    assert_eq!(
        baba_sim::events::note_freq("cflat"),
        baba_sim::events::note_freq("b")
    );
    assert_eq!(baba_sim::events::note_freq("x"), 24000);
    assert_eq!(baba_sim::events::play_sample("keke"), "tune_blop");
    // Presentation queries never touch the simulation state.
    let mut l = level(&["baba> .", "BABA IS YOU", "BABA IS RED"], &[]);
    let _ = l.step(Input::dir(0));
    let before = l.state_hash();
    let _ = (l.units(), l.camera(), l.level_colours(), l.no_player());
    assert_eq!(before, l.state_hash());
}
