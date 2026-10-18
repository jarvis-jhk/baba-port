//! Integration tests over every level of every world in the player's
//! install. Skipped (pass with a note) when `$BABA_GAME_DIR` is missing.
//! `BABA_SIM_TURNS` sets the random turns per level (default 40).

use std::sync::Arc;

use baba_formats::GameInstall;
use baba_sim::rng::Rng;
use baba_sim::{Context, Input, Level, Source};

fn install() -> Option<GameInstall> {
    match GameInstall::from_env() {
        Some(Ok(g)) => Some(g),
        _ => {
            eprintln!("skipping: $BABA_GAME_DIR not set or missing");
            None
        }
    }
}

fn all_levels(g: &GameInstall) -> Vec<(String, Arc<Source>)> {
    let objects = g.load_objects().expect("objects");
    let mut out = Vec::new();
    for w in g.world_ids().expect("worlds") {
        let world = g.world(&w).expect("world");
        for stem in &world.levels {
            let data = world
                .load_level(stem)
                .unwrap_or_else(|e| panic!("{w}/{stem}: {e}"));
            out.push((
                format!("{w}/{stem}"),
                Arc::new(Source::from_level(&objects, &data, &w)),
            ));
        }
    }
    out
}

fn inputs(seed: u64, n: usize, undo: bool) -> Vec<Input> {
    let mut r = Rng::new(seed);
    (0..n)
        .map(|_| match r.range(0, if undo { 13 } else { 11 }) {
            0..=2 => Input::dir(0),
            3..=4 => Input::dir(1),
            5..=7 => Input::dir(2),
            8..=9 => Input::dir(3),
            10 | 11 => Input::wait(),
            _ => Input::Undo,
        })
        .collect()
}

fn turns() -> usize {
    std::env::var("BABA_SIM_TURNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40)
}

type Snap = Vec<(i32, String, i32, i32, u8)>;

fn snap(l: &Level) -> Snap {
    let mut v: Snap = l
        .units()
        .into_iter()
        .map(|u| (u.id, u.name, u.x, u.y, u.dir))
        .collect();
    v.sort();
    v
}

#[test]
fn every_level_random_turns_deterministic() {
    let Some(g) = install() else { return };
    let levels = all_levels(&g);
    assert!(levels.len() > 500, "only {} levels", levels.len());
    let n = turns();
    let mut total_turns = 0usize;
    for (i, (name, src)) in levels.iter().enumerate() {
        let ins = inputs(i as u64 + 1, n, true);
        let mut a = Level::start(src.clone(), Context::default());
        let mut b = a.clone();
        assert_eq!(a.state_hash(), b.state_hash(), "{name}: clone");
        for (t, &inp) in ins.iter().enumerate() {
            let oa = a.step(inp);
            let ob = b.step(inp);
            assert_eq!(oa, ob, "{name}: outcome differs at turn {t}");
            assert_eq!(
                a.state_hash(),
                b.state_hash(),
                "{name}: hash differs at turn {t}"
            );
            total_turns += 1;
        }
        // A fresh start replays to the same hash.
        let mut c = Level::start(src.clone(), Context::default());
        for &inp in &ins {
            c.step(inp);
        }
        assert_eq!(c.state_hash(), a.state_hash(), "{name}: replay");
    }
    eprintln!("{} levels, {total_turns} turns", levels.len());
}

#[test]
fn every_level_undo_round_trip() {
    let Some(g) = install() else { return };
    let n = turns();
    let mut checked = 0usize;
    for (i, (name, src)) in all_levels(&g).iter().enumerate() {
        let mut l = Level::start(src.clone(), Context::default());
        let mut stack = vec![snap(&l)];
        for &inp in &inputs(i as u64 + 1000, n, false) {
            let o = l.step(inp);
            if o.undo_step {
                stack.push(snap(&l));
            }
        }
        assert_eq!(l.undo_depth() + 1, stack.len(), "{name}: undo depth");
        while stack.len() > 1 {
            stack.pop();
            l.step(Input::Undo);
            assert_eq!(
                &snap(&l),
                stack.last().unwrap(),
                "{name}: undo to step {}",
                stack.len() - 1
            );
            checked += 1;
        }
        // Restart gives back the initial state.
        l.step(Input::Restart);
        assert_eq!(
            snap(&l),
            snap(&Level::start(src.clone(), Context::default())),
            "{name}: restart"
        );
    }
    eprintln!("{checked} undo steps verified");
}

#[test]
fn rules_parse_at_start() {
    let Some(g) = install() else { return };
    let mut with_rules = 0;
    let levels = all_levels(&g);
    for (_, src) in &levels {
        let l = Level::start(src.clone(), Context::default());
        if l.rules().features.len() > 3 {
            with_rules += 1;
        }
    }
    // Nearly every level shows rules at the start (maps included).
    assert!(
        with_rules * 10 > levels.len() * 8,
        "{with_rules}/{}",
        levels.len()
    );
}
