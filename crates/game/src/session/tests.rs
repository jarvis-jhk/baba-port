//! Session tests on the player's install (skip without `BABA_GAME_DIR`):
//! shared-save propagation between two instances (mode 2) and replaying
//! encoded turns into fresh instances (what an online replica does).

use std::path::PathBuf;

use baba_formats::ObjectTable;
use baba_progress::Slot;
use baba_render::Atlas;

use crate::lang::Texts;
use crate::play::{Env, Play};
use crate::session::{Fx, Turn};
use crate::world::WorldInfo;

struct Data {
    data: PathBuf,
    objects: ObjectTable,
    atlas: Atlas,
    texts: Texts,
}

fn data() -> Option<Data> {
    let Some(dir) = std::env::var_os("BABA_GAME_DIR") else {
        eprintln!("skipping: no BABA_GAME_DIR");
        return None;
    };
    let data = PathBuf::from(dir).join("Data");
    let objects = ObjectTable::load(&data.join("values.lua")).ok()?;
    let texts = Texts::load(&data, "en");
    Some(Data {
        data,
        objects,
        atlas: Atlas::new(2048),
        texts,
    })
}

fn moves(s: &str) -> Vec<Turn> {
    s.chars()
        .map(|c| match c {
            'R' => Turn::Move { dir: 0, you: 1 },
            'U' => Turn::Move { dir: 1, you: 1 },
            'L' => Turn::Move { dir: 2, you: 1 },
            'D' => Turn::Move { dir: 3, you: 1 },
            'Z' => Turn::Undo,
            _ => Turn::wait(),
        })
        .collect()
}

/// Applies turns; returns whether a win happened.
fn play(d: &mut Data, p: &mut Play, slot: &mut Slot, turns: &[Turn]) -> bool {
    let mut env = Env {
        data_dir: &d.data,
        objects: &d.objects,
        atlas: &mut d.atlas,
    };
    let mut won = false;
    for t in turns {
        let a = p.apply(t, &mut env, slot, &d.texts);
        won |= a.fx.iter().any(|f| matches!(f, Fx::Won));
    }
    won
}

fn start(d: &mut Data, slot: &mut Slot, own: Option<Slot>) -> Play {
    let w = WorldInfo::open(&d.data, "baba").expect("baba world");
    let mut env = Env {
        data_dir: &d.data,
        objects: &d.objects,
        atlas: &mut d.atlas,
    };
    Play::start_with(&mut env, w, 0, slot, own).expect("start")
}

#[test]
fn shared_save_propagates_to_a_map() {
    let Some(mut d) = data() else { return };
    let mut slot = Slot::default();
    // P1 starts a fresh save in 0level, solves it, returns to the map.
    let mut p1 = start(&mut d, &mut slot, None);
    assert_eq!(p1.cur.stem, "0level");
    assert!(play(&mut d, &mut p1, &mut slot, &moves("RRRRRRRR")));
    play(&mut d, &mut p1, &mut slot, &[Turn::Proceed]);
    assert!(p1.cur.is_map());
    p1.finish_anim();
    // P2 joins on the same save (own navigation state): on the map.
    let own = slot.clone();
    let mut p2 = start(&mut d, &mut slot, Some(own));
    assert!(p2.cur.is_map());
    let map = p2.cur.stem.clone();
    assert_eq!(
        p2.cur.layout.as_ref().unwrap().icon_state("1level"),
        Some(2)
    );
    // P2 moves its own cursor somewhere.
    play(&mut d, &mut p2, &mut slot, &moves("R"));
    let c2 = p2.cur.cursors()[0].x;
    // P1 enters 1level through the icon under its cursor and solves it.
    play(&mut d, &mut p1, &mut slot, &moves("RUU"));
    let (target, unit) = p1.cur.candidates(&p1.world)[0].clone();
    assert_eq!(target, "1level");
    play(&mut d, &mut p1, &mut slot, &[Turn::Enter { target, unit }]);
    assert_eq!(p1.cur.stem, "1level");
    assert!(play(
        &mut d,
        &mut p1,
        &mut slot,
        &moves("ULLLLLLLUUURRDDRRRR")
    ));
    assert_eq!(slot.level_state("baba", "1level"), 3);
    // P2's map is stale until it re-reads the save; the cursor stays.
    assert_eq!(
        p2.cur.layout.as_ref().unwrap().icon_state("1level"),
        Some(2)
    );
    assert_eq!(p2.refresh_map(&mut slot), Some(true));
    assert_eq!(p2.cur.stem, map);
    assert_eq!(
        p2.cur.layout.as_ref().unwrap().icon_state("1level"),
        Some(3)
    );
    assert_eq!(p2.cur.cursors()[0].x, c2);
    assert_eq!(p2.refresh_map(&mut slot), Some(false));
    // A pending win blocks P1's own refresh until it proceeds.
    assert_eq!(p1.refresh_map(&mut slot), None);
    // Navigation: P1's Previous is in the slot, P2's in its own copy.
    assert_eq!(slot.previous("baba"), Some("1level"));
    assert_eq!(
        p2.own_nav.as_ref().unwrap().previous("baba"),
        Some("0level")
    );
    // P2 enters 0level (already won) and wins again: no second spore.
    let prize = slot.prize_total("baba");
    play(&mut d, &mut p2, &mut slot, &moves("L"));
    let (target, unit) = p2.cur.candidates(&p2.world)[0].clone();
    assert_eq!(target, "0level");
    play(&mut d, &mut p2, &mut slot, &[Turn::Enter { target, unit }]);
    assert!(play(&mut d, &mut p2, &mut slot, &moves("RRRRRRRR")));
    assert_eq!(slot.prize_total("baba"), prize);
}

#[test]
fn encoded_turns_replay_to_the_same_state() {
    let Some(mut d) = data() else { return };
    let script = [
        moves("RRRZZRRRRRRRR"),
        vec![Turn::Proceed],
        moves("RUU"),
        moves("UZW"),
    ]
    .concat();
    // The "local" instance records the bytes of every turn it applies.
    let mut slot_a = Slot::default();
    let mut a = start(&mut d, &mut slot_a, None);
    let mut log: Vec<Vec<u8>> = Vec::new();
    let mut hashes = Vec::new();
    for t in &script {
        play(&mut d, &mut a, &mut slot_a, std::slice::from_ref(t));
        log.push(t.encode());
        hashes.push(a.state_hash());
    }
    // A "replica" decodes and applies the same bytes on its own save.
    let mut slot_b = Slot::default();
    let mut b = start(&mut d, &mut slot_b, None);
    for (bytes, h) in log.iter().zip(&hashes) {
        let t = Turn::decode(bytes).expect("decodes");
        play(&mut d, &mut b, &mut slot_b, &[t]);
        assert_eq!(b.state_hash(), *h);
    }
    assert_eq!(slot_a.to_text(), slot_b.to_text());
    assert!(b.cur.is_map());
}

/// A level played on its own goes through the same turn path: the win is
/// recorded as a single-level result, then `Proceed` ends the play.
#[test]
fn single_level_win_through_apply() {
    let Some(mut d) = data() else { return };
    let mut slot = Slot::default();
    let w = WorldInfo::open(&d.data, "baba").expect("baba world");
    let level = crate::play::SingleLevel {
        code: Some("TEST-CODE".into()),
        temp: false,
    };
    let mut p = {
        let mut env = Env {
            data_dir: &d.data,
            objects: &d.objects,
            atlas: &mut d.atlas,
        };
        Play::single(&mut env, w, 0, "0level", level).expect("single")
    };
    assert!(play(&mut d, &mut p, &mut slot, &moves("RRRRRRRR")));
    assert!(matches!(
        p.pending,
        Some(crate::session::Pending::LeaveSingle { won: true })
    ));
    assert_eq!(slot.level_state(crate::packs::LEVELS, "0level"), 3);
    assert!(crate::levelcodes::store::won(&slot, "TEST-CODE").won);
    let mut env = Env {
        data_dir: &d.data,
        objects: &d.objects,
        atlas: &mut d.atlas,
    };
    let a = p.apply(&Turn::Proceed, &mut env, &mut slot, &d.texts);
    assert!(
        a.fx.iter()
            .any(|f| matches!(f, Fx::LeaveSingle { won: true }))
    );
    assert_eq!(p.cur.stem, "0level");
}

/// The map-clear showcase is pending state stepped by `Proceed` turns:
/// each shows a map (with its animation), the last returns to the map the
/// player stands on; the save is not changed by the showcase.
#[test]
fn showcase_steps_are_turns() {
    let Some(mut d) = data() else { return };
    let mut slot = Slot::default();
    let mut p = start(&mut d, &mut slot, None);
    assert!(play(&mut d, &mut p, &mut slot, &moves("RRRRRRRR")));
    play(&mut d, &mut p, &mut slot, &[Turn::Proceed]);
    p.finish_anim();
    let map = p.cur.stem.clone();
    let before = slot.level_state("baba", "1level");
    p.pending = Some(crate::session::Pending::Showcase(crate::play::Showcase {
        queue: vec![map.clone()],
        cleared: "1level".into(),
    }));
    play(&mut d, &mut p, &mut slot, &moves("R"));
    assert!(p.pending.is_some(), "moves are ignored during the showcase");
    play(&mut d, &mut p, &mut slot, &[Turn::Proceed]);
    assert!(p.anim.is_some(), "the cleared icon animates");
    assert!(matches!(
        p.pending,
        Some(crate::session::Pending::Showcase(_))
    ));
    play(&mut d, &mut p, &mut slot, &[Turn::Proceed]);
    assert!(p.pending.is_none());
    assert_eq!(p.cur.stem, map);
    assert_eq!(slot.level_state("baba", "1level"), before);
}
