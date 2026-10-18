//! Online replicas in one process (skip without `BABA_GAME_DIR`): two
//! machines over `baba_net::MemNet`, each with the real game replica
//! (`Multi` + `NetGame`), driven by encoded turns like the app does. Every
//! machine must compute the same state for every stream, and in mode 2 a
//! win of one player reaches the other player's map through the host.

use std::path::PathBuf;

use baba_formats::ObjectTable;
use baba_net::{LinkSim, MemNet, NetConfig, NetEvent, NetSession, PlayerId, StreamId};
use baba_progress::Profile;
use baba_render::Atlas;

use crate::audio::AudioDirector;
use crate::join::Players;
use crate::lang::Texts;
use crate::session::multi::{Cx, Multi};
use crate::session::online::{NetGame, NetSide, Rep, SessionSettings, mp_mode, net_mode};
use crate::session::saves::ExtraSlots;
use crate::session::{MpMode, Turn};
use crate::sounds::SoundBank;
use crate::ui::Gfx;

struct Machine {
    net: NetSession,
    game: Option<(Multi, NetGame)>,
    data: PathBuf,
    objects: ObjectTable,
    gfx: Gfx,
    texts: Texts,
    profile: Profile,
    audio: AudioDirector,
    locals: usize,
}

impl Machine {
    fn new(net: &MemNet, name: &str, locals: usize) -> Option<Machine> {
        let game = PathBuf::from(std::env::var_os("BABA_GAME_DIR")?);
        let data = game.join("Data");
        let mut cfg = NetConfig::new("test");
        cfg.link_sim = LinkSim::default();
        let mut n = NetSession::memory(net.client(name), cfg);
        n.set_local_players((0..locals).map(|i| format!("{name}{i}")).collect());
        n.set_ready(true);
        Some(Machine {
            net: n,
            game: None,
            objects: ObjectTable::load(&data.join("values.lua")).ok()?,
            gfx: Gfx::new(Atlas::new(2048), &data),
            texts: Texts::load(&data, "en"),
            profile: Profile::in_memory(),
            audio: AudioDirector::new(&game, SoundBank::empty()),
            data,
            locals,
        })
    }

    /// Network, session start, stream entries, mode-2 progress events.
    fn step(&mut self) {
        self.net.poll();
        while let Some(e) = self.net.next_event() {
            if let NetEvent::SessionStarted(s) = e {
                let settings = SessionSettings::decode(&s.settings).expect("settings");
                let side = NetSide {
                    sources: vec![None; self.locals],
                    local_games: vec![("baba".into(), 0); self.locals],
                    host: self.net.is_host(),
                    quiet: true,
                    ..Default::default()
                };
                let mut cx = Cx {
                    data: &self.data,
                    objects: &self.objects,
                    gfx: &mut self.gfx,
                    texts: &self.texts,
                    profile: &mut self.profile,
                    audio: &mut self.audio,
                    cursor: None,
                };
                let players = Players::new(&cx.profile.settings, &settings.world);
                let mode = mp_mode(s.mode);
                let mut m = Multi::new_net(
                    mode,
                    players,
                    &settings.world,
                    ExtraSlots::default(),
                    side,
                    &mut cx,
                );
                let g = NetGame::new(
                    mode,
                    settings,
                    self.net.me(),
                    self.net.is_host(),
                    &mut m,
                    &mut cx,
                );
                self.game = Some((m, g));
            }
        }
        let Some((m, g)) = &mut self.game else {
            return;
        };
        let mut cx = Cx {
            data: &self.data,
            objects: &self.objects,
            gfx: &mut self.gfx,
            texts: &self.texts,
            profile: &mut self.profile,
            audio: &mut self.audio,
            cursor: None,
        };
        self.net.pump(&mut Rep { g, m, cx: &mut cx });
        if let Some(n) = &mut m.net {
            n.quiet = false;
            for (li, ev) in std::mem::take(&mut n.events) {
                self.net.event(li, &ev).expect("event");
            }
        }
    }

    /// A turn of local player `li`, as the app sends it.
    fn send(&mut self, li: usize, t: Turn) {
        let p = self.net.input(li, &t.encode()).expect("input");
        let (_, g) = self.game.as_mut().expect("session");
        if g.mode != MpMode::Coop {
            let len = self.net.stream(StreamId::Player(p)).len() as u32;
            g.made.insert((StreamId::Player(p), len - 1));
        }
    }

    fn multi(&self) -> &Multi {
        &self.game.as_ref().expect("session").0
    }

    fn view_of(&self, li: usize) -> usize {
        let p = self.net.local_player(li).expect("admitted").0 as usize;
        let m = self.multi();
        m.views
            .iter()
            .position(|v| v.seats.contains(&p))
            .expect("view")
    }

    fn hashes(&mut self) -> Vec<(StreamId, u64)> {
        let (m, g) = self.game.as_ref().expect("session");
        self.net
            .streams()
            .into_iter()
            .map(|s| (s, g.state_hash(m, s)))
            .collect()
    }
}

fn settle(ms: &mut [&mut Machine]) {
    for _ in 0..40 {
        for m in ms.iter_mut() {
            m.step();
        }
    }
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

/// Two machines, one player each, in a room with a fresh save.
fn pair(mode: MpMode) -> Option<(Machine, Machine)> {
    let net = MemNet::new(baba_net::GAME_ID);
    let mut a = Machine::new(&net, "a", 1)?;
    let mut b = Machine::new(&net, "b", 1)?;
    a.net.host_private();
    settle(&mut [&mut a]);
    let code = a.net.lobby().code.clone().expect("code");
    b.net.join_code(&code);
    settle(&mut [&mut a, &mut b]);
    a.net.set_mode(net_mode(mode)).unwrap();
    let s = SessionSettings {
        world: "baba".into(),
        slot: 0,
        save: String::new(),
    };
    a.net.set_settings(s.encode()).unwrap();
    a.net.start_session(Some(7)).unwrap();
    settle(&mut [&mut a, &mut b]);
    if mode == MpMode::OwnGames {
        // Each player's save at the start of its stream.
        for m in [&mut a, &mut b] {
            for c in crate::session::online::init_chunks("baba", "") {
                m.net.own_event(0, &c).unwrap();
            }
        }
        settle(&mut [&mut a, &mut b]);
    }
    Some((a, b))
}

fn same(a: &mut Machine, b: &mut Machine) {
    let (ha, hb) = (a.hashes(), b.hashes());
    assert!(!ha.is_empty());
    assert_eq!(ha, hb, "the machines disagree");
}

#[test]
fn coop_turns_from_both_machines() {
    let Some((mut a, mut b)) = pair(MpMode::Coop) else {
        return;
    };
    assert_eq!(a.multi().views.len(), 1);
    assert_eq!(a.multi().views[0].seats, vec![0, 1]);
    // 0level = RRRRRRRR; both players push, in host order.
    for (i, t) in moves("RRRRRRRR").into_iter().enumerate() {
        if i % 2 == 0 {
            a.send(0, t);
        } else {
            b.send(0, t);
        }
        settle(&mut [&mut a, &mut b]);
    }
    assert!(b.multi().views[0].play.pending.is_some(), "0level won");
    b.send(0, Turn::Proceed);
    settle(&mut [&mut a, &mut b]);
    assert!(a.multi().views[0].play.cur.is_map());
    same(&mut a, &mut b);
}

#[test]
fn shared_save_progress_reaches_the_other_map() {
    let Some((mut a, mut b)) = pair(MpMode::SharedSave) else {
        return;
    };
    assert_eq!(b.multi().views.len(), 2);
    // Both solve 0level in their own instances and go to the map.
    for m in [&mut a, &mut b] {
        for t in moves("RRRRRRRR") {
            m.send(0, t);
        }
        m.send(0, Turn::Proceed);
    }
    settle(&mut [&mut a, &mut b]);
    let vb = b.view_of(0);
    assert!(b.multi().views[vb].play.cur.is_map());
    // A enters 1level and wins it.
    for t in moves("RUU") {
        a.send(0, t);
    }
    settle(&mut [&mut a, &mut b]);
    let va = a.view_of(0);
    let p = &a.multi().views[va].play;
    let (target, unit) = p.cur.candidates(&p.world)[0].clone();
    assert_eq!(target, "1level");
    a.send(0, Turn::Enter { target, unit });
    for t in moves("ULLLLLLLUUURRDDRRRR") {
        a.send(0, t);
    }
    a.send(0, Turn::Proceed);
    settle(&mut [&mut a, &mut b]);
    // (A running unlock animation ends in its final layout.)
    let icon = |m: &Machine, vi: usize| {
        let p = &m.multi().views[vi].play;
        p.anim
            .as_ref()
            .map(|a| &a.layout)
            .or(p.cur.layout.as_ref())
            .and_then(|l| l.icon_state("1level"))
    };
    // B's map still shows the old state until B's machine refreshes it at a
    // point of B's stream.
    assert_eq!(icon(&b, vb), Some(2));
    let pb = b.net.local_player(0).unwrap();
    assert!(b.game.as_ref().unwrap().1.unmerged(PlayerId(pb.0)));
    b.send(0, Turn::Refresh);
    settle(&mut [&mut a, &mut b]);
    assert_eq!(icon(&b, vb), Some(3));
    // A's machine shows B's instance identically.
    let vb_on_a = a
        .multi()
        .views
        .iter()
        .position(|v| v.seats.contains(&(pb.0 as usize)))
        .unwrap();
    assert_eq!(icon(&a, vb_on_a), Some(3));
    same(&mut a, &mut b);
}

#[test]
fn own_games_side_by_side() {
    let Some((mut a, mut b)) = pair(MpMode::OwnGames) else {
        return;
    };
    assert_eq!(a.multi().views.len(), 2);
    for t in moves("RRRRRRRR") {
        a.send(0, t);
    }
    a.send(0, Turn::Proceed);
    for t in moves("RRZRL") {
        b.send(0, t);
    }
    settle(&mut [&mut a, &mut b]);
    let pa = a.net.local_player(0).unwrap().0 as usize;
    let on_b = b
        .multi()
        .views
        .iter()
        .position(|v| v.seats.contains(&pa))
        .unwrap();
    assert!(b.multi().views[on_b].play.cur.is_map());
    same(&mut a, &mut b);
}
