//! Whole sessions in one process over `MemNet` (no server): every mode,
//! machine mixes, link conditions, leaving, crashing, late join, host
//! migration, spectators, desync detection, version checks, prediction.

use baba_net::harness::{RunSpec, run_memory};
use baba_net::toy::{ToyBot, ToyReplica};
use baba_net::*;

fn pass(spec: &RunSpec) {
    let verbose = std::env::var("BABA_NET_VERBOSE").is_ok();
    let res = run_memory(spec, verbose);
    match spec.check(&res) {
        Ok(s) => eprintln!("PASS {s}"),
        Err(e) => panic!("FAIL {e}\n{res:#?}"),
    }
}

fn lossy() -> LinkSim {
    LinkSim {
        delay_ms: 30,
        jitter_ms: 10,
        loss: 0.05,
    }
}

#[test]
fn coop_4x3() {
    pass(&RunSpec::new(&[3, 3, 3, 3], Mode::Coop));
}

#[test]
fn coop_12x1_latency_loss() {
    let mut s = RunSpec::new(&[1; 12], Mode::Coop);
    s.link = lossy();
    s.secs = 4.0;
    pass(&s);
}

#[test]
fn shared_save_2_4_6_with_spectator() {
    let mut s = RunSpec::new(&[2, 4, 6, 0], Mode::SharedSave);
    s.secs = 4.0;
    pass(&s);
}

#[test]
fn own_games_1x12_and_12x1() {
    pass(&RunSpec::new(&[12], Mode::OwnGames));
    let mut s = RunSpec::new(&[1; 12], Mode::OwnGames);
    s.link = lossy();
    pass(&s);
}

#[test]
fn leave_and_crash_mid_session() {
    for mode in [Mode::Coop, Mode::SharedSave, Mode::OwnGames] {
        let mut s = RunSpec::new(&[2, 2, 2, 2, 1], mode);
        s.link = lossy();
        s.secs = 4.0;
        s.leave = vec![(2, 1.0)];
        s.crash = vec![(3, 1.8)];
        pass(&s);
    }
}

#[test]
fn late_join_replays_from_start() {
    for mode in [Mode::Coop, Mode::SharedSave, Mode::OwnGames] {
        let mut s = RunSpec::new(&[2, 1, 3, 2, 0], mode);
        s.secs = 4.0;
        s.late = vec![(3, 2.0), (4, 2.5)];
        pass(&s);
    }
}

#[test]
fn host_leaves_and_the_rest_carry_on() {
    for mode in [Mode::Coop, Mode::SharedSave, Mode::OwnGames] {
        let mut s = RunSpec::new(&[2, 1, 2, 0], mode);
        s.secs = 5.0;
        s.link = lossy();
        s.leave = vec![(0, 1.5)];
        pass(&s);
    }
}

#[test]
fn host_crashes_and_the_rest_carry_on() {
    let mut s = RunSpec::new(&[1, 3, 2, 2], Mode::SharedSave);
    s.secs = 5.0;
    s.link = lossy();
    s.crash = vec![(0, 1.5)];
    pass(&s);
}

#[test]
fn desync_is_detected() {
    let mut s = RunSpec::new(&[1, 1, 1], Mode::Coop);
    s.desync = vec![(1, 1.0)];
    pass(&s);
    let mut s = RunSpec::new(&[2, 2, 1], Mode::OwnGames);
    s.link = lossy();
    s.desync = vec![(2, 1.0)];
    pass(&s);
}

#[test]
fn desync_abort_policy() {
    let mut s = RunSpec::new(&[1, 2, 1], Mode::SharedSave);
    s.desync = vec![(1, 1.0)];
    s.abort_on_desync = true;
    pass(&s);
}

fn pump_all(nets: &mut [&mut NetSession]) {
    for _ in 0..50 {
        for n in nets.iter_mut() {
            n.poll();
        }
    }
}

fn events(n: &mut NetSession) -> Vec<NetEvent> {
    std::iter::from_fn(|| n.next_event()).collect()
}

#[test]
fn lobby_version_and_content_checks() {
    let net = MemNet::new(GAME_ID);
    let mut cfg = NetConfig::new("1.0");
    cfg.content = "abc".into();
    cfg.link_sim = LinkSim::default();
    let mut host = NetSession::memory(net.client("host"), cfg.clone());
    host.set_local_players(vec!["Baba".into(), "Keke".into()]);
    host.host_private();
    pump_all(&mut [&mut host]);
    let code = host
        .lobby()
        .code
        .clone()
        .expect("private rooms have a code");
    assert!(host.lobby().invite.starts_with("warpzone://join/"));

    let mut old = NetSession::memory(
        net.client("old"),
        NetConfig {
            build: "0.9".into(),
            ..cfg.clone()
        },
    );
    old.join_code(&code);
    let mut other = NetSession::memory(
        net.client("other"),
        NetConfig {
            content: "xyz".into(),
            ..cfg.clone()
        },
    );
    other.join_code(&code);
    let mut good = NetSession::memory(net.client("good"), cfg.clone());
    good.set_local_players(vec!["Me".into(), "".into(), "Fofo".into()]);
    good.join_code(&code);
    let mut watcher = NetSession::memory(net.client("watcher"), cfg.clone());
    watcher.set_spectate(true);
    let args = vec![
        "baba-port".to_string(),
        "--warpzone-join".into(),
        host.lobby().invite.clone(),
    ];
    assert!(matches!(
        watcher.handle_launch(&args),
        LaunchAction::Joining { .. }
    ));
    pump_all(&mut [&mut host, &mut old, &mut other, &mut good, &mut watcher]);

    let rej = |n: &mut NetSession| {
        events(n).into_iter().find_map(|e| match e {
            NetEvent::Rejected { reason } => Some(reason),
            _ => None,
        })
    };
    assert!(rej(&mut old).unwrap().starts_with(REJECT_VERSION));
    assert!(rej(&mut other).unwrap().starts_with(REJECT_CONTENT));
    assert!(!old.in_room() && !other.in_room());

    // Mode selection and the roster reach everybody.
    host.set_mode(Mode::SharedSave).unwrap();
    host.set_settings(vec![1, 2, 3]).unwrap();
    pump_all(&mut [&mut host, &mut good, &mut watcher]);
    for n in [&host, &good, &watcher] {
        let l = n.lobby();
        assert_eq!(l.mode, Mode::SharedSave);
        assert_eq!(l.settings, vec![1, 2, 3]);
        assert_eq!(l.machines.len(), 3);
        assert_eq!(l.player_count(), 5);
        let g = l.machine(good.me().unwrap()).unwrap();
        assert_eq!(g.players, vec!["Me", "Player 2", "Fofo"]);
        assert!(l.machine(watcher.me().unwrap()).unwrap().spectator);
    }
    assert!(good.set_mode(Mode::Coop).is_err());

    // Start: 5 players in machine order; the watcher replays everything too.
    let start = host.start_session(Some(9)).unwrap();
    pump_all(&mut [&mut host, &mut good, &mut watcher]);
    assert_eq!(good.session(), Some(&start));
    assert_eq!(watcher.session(), Some(&start));
    let ps = watcher.players();
    assert_eq!(ps.len(), 5);
    assert_eq!(ps[0].info.name, "Baba");
    assert_eq!(ps[2].info.machine, good.me().unwrap());
    assert_eq!(good.local_player(2), Some(PlayerId(4)));
    assert!(watcher.local_player(0).is_none());
    assert!(watcher.input(0, &[1]).is_err());
    // A new local player mid-session joins at once; removing one leaves.
    good.set_local_players(vec!["Me".into()]);
    pump_all(&mut [&mut host, &mut good, &mut watcher]);
    assert_eq!(watcher.players().iter().filter(|p| p.active).count(), 3);
    good.set_local_players(vec!["Me".into(), "Jiji".into()]);
    pump_all(&mut [&mut host, &mut good, &mut watcher]);
    let ps = watcher.players();
    assert_eq!(ps.iter().filter(|p| p.active).count(), 4);
    assert_eq!(ps.last().unwrap().info.name, "Jiji");
    assert_eq!(ps.last().unwrap().info.player, PlayerId(5));
}

#[test]
fn twelve_player_cap_and_waiting_players() {
    let net = MemNet::new(GAME_ID);
    let cfg = NetConfig {
        link_sim: LinkSim::default(),
        ..NetConfig::new("1")
    };
    let mut host = NetSession::memory(net.client("h"), cfg.clone());
    host.set_local_players((0..10).map(|i| format!("H{i}")).collect());
    host.host_private();
    pump_all(&mut [&mut host]);
    let code = host.lobby().code.clone().unwrap();
    let mut g = NetSession::memory(net.client("g"), cfg.clone());
    g.set_local_players(vec!["a".into(), "b".into(), "c".into(), "d".into()]);
    g.join_code(&code);
    pump_all(&mut [&mut host, &mut g]);
    host.start_session(None).unwrap();
    pump_all(&mut [&mut host, &mut g]);
    assert_eq!(g.players().len(), 12);
    let me = g.me().unwrap();
    let view = g.lobby().machine(me).unwrap().clone();
    assert_eq!(view.admitted.iter().filter(|a| a.is_some()).count(), 2);
    assert!(g.local_player(2).is_none());
    // Two host players put their controllers down: the waiting two get in.
    host.set_local_players((0..8).map(|i| format!("H{i}")).collect());
    pump_all(&mut [&mut host, &mut g]);
    assert!(g.local_player(2).is_some() && g.local_player(3).is_some());
    assert_eq!(g.players().iter().filter(|p| p.active).count(), 12);
}

/// Mode 1 prediction: the guest shows confirmed state + its pending turns;
/// it may differ for a moment, never in the end.
#[test]
fn coop_prediction_converges() {
    let net = MemNet::new(GAME_ID);
    let cfg = NetConfig {
        link_sim: LinkSim {
            delay_ms: 25,
            jitter_ms: 5,
            loss: 0.05,
        },
        ..NetConfig::new("1")
    };
    let mut host = NetSession::memory(net.client("h"), cfg.clone());
    host.set_local_players(vec!["A".into()]);
    host.host_private();
    for _ in 0..5 {
        host.poll();
    }
    let code = host.lobby().code.clone().unwrap();
    let mut g = NetSession::memory(net.client("g"), cfg.clone());
    g.set_local_players(vec!["B".into()]);
    g.join_code(&code);
    let t = std::time::Instant::now();
    while g.me().is_none() && t.elapsed().as_secs() < 5 {
        host.poll();
        g.poll();
    }
    let start = host.start_session(Some(1)).unwrap();
    let mut rh = ToyReplica::new(&start);
    let mut rg: Option<ToyReplica> = None;
    let (mut bh, mut bg) = (ToyBot::new(1), ToyBot::new(2));
    let mut mismatches = 0;
    let t = std::time::Instant::now();
    let mut turns = 0;
    while t.elapsed().as_millis() < 3000 {
        host.poll();
        g.poll();
        while let Some(e) = g.next_event() {
            if let NetEvent::SessionStarted(s) = e {
                rg = Some(ToyReplica::new(&s));
            }
        }
        host.pump(&mut rh);
        let Some(r) = rg.as_mut() else { continue };
        g.pump(r);
        if turns < 200 && host.local_player(0).is_some() && g.local_player(0).is_some() {
            host.input(0, &[bh.input()]).unwrap();
            g.input(0, &[bg.input()]).unwrap();
            turns += 1;
        }
        // The predicted view: a throwaway copy, never fed back.
        let mut predicted = r.clone();
        for p in g.pending() {
            predicted.apply(
                StreamId::Shared,
                u32::MAX,
                &Entry {
                    kind: p.kind,
                    player: p.player,
                    seq: p.seq,
                    sync: 0,
                    data: p.data.clone(),
                },
            );
        }
        if !g.pending().is_empty() {
            mismatches += 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(g.pending().is_empty());
    assert!(mismatches > 0, "the guest had turns in flight");
    let rgv = rg.as_mut().unwrap();
    assert_eq!(
        g.stream(StreamId::Shared).len(),
        host.stream(StreamId::Shared).len()
    );
    assert_eq!(
        rh.state_hash(StreamId::Shared),
        rgv.state_hash(StreamId::Shared)
    );
}

/// A guest that joined without players and adds some later plays (it is
/// not stuck as a spectator), and own-stream events (mode 3: a player's
/// starting save) reach every machine.
#[test]
fn players_added_after_joining_and_own_events() {
    let net = MemNet::new(GAME_ID);
    let mut cfg = NetConfig::new("1.0");
    cfg.link_sim = LinkSim::default();
    let mut host = NetSession::memory(net.client("host"), cfg.clone());
    host.set_local_players(vec!["Baba".into()]);
    host.host_private();
    pump_all(&mut [&mut host]);
    let code = host.lobby().code.clone().unwrap();
    let mut guest = NetSession::memory(net.client("guest"), cfg.clone());
    guest.join_code(&code);
    pump_all(&mut [&mut host, &mut guest]);
    assert!(host.lobby().machines[1].spectator);
    guest.set_local_players(vec!["Keke".into()]);
    pump_all(&mut [&mut host, &mut guest]);
    assert!(!host.lobby().machines[1].spectator);
    host.set_mode(Mode::OwnGames).unwrap();
    host.start_session(Some(1)).unwrap();
    pump_all(&mut [&mut host, &mut guest]);
    let p = guest
        .local_player(0)
        .expect("the guest's player is admitted");
    let big = vec![7u8; 3000];
    guest.own_event(0, &big).unwrap();
    guest.input(0, &[1, 2]).unwrap();
    pump_all(&mut [&mut host, &mut guest]);
    let s = host.stream(StreamId::Player(p));
    assert_eq!(s.len(), 2);
    assert_eq!(s[0].kind, EntryKind::Event);
    assert_eq!(s[0].data, big);
    assert_eq!(s[1].data, vec![1, 2]);
    assert!(
        guest
            .own_event(0, &vec![0; baba_net::wire::MAX_EVENT + 1])
            .is_err()
    );
}
