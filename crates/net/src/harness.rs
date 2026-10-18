//! A test driver: one machine playing [`crate::toy`] with bots through a
//! [`NetSession`], following a [`PeerPlan`] (host or join, local players,
//! leave / crash / desync at a time). Used by the in-process tests and by
//! `examples/netsim.rs` (one OS process per machine against a real
//! warpzone-server).

use crate::session::{NetEvent, NetSession};
use crate::toy::{ToyBot, ToyReplica};
use crate::wire::{MachineId, Mode, Phase, StreamId};
use std::time::{Duration, Instant};
use warpzone_core::client::Event;

/// How a machine gets into the room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinHow {
    /// Host a room (private unless `PeerPlan::public`).
    Host,
    /// Type the join code.
    Code(String),
    /// Started by the launcher / an invite link: these arguments go through
    /// `handle_launch` (`--warpzone-join <room|invite link>`).
    Launch(Vec<String>),
}

#[derive(Clone, Debug)]
pub struct PeerPlan {
    pub index: usize,
    pub locals: usize,
    pub spectator: bool,
    pub mode: Mode,
    pub join: JoinHow,
    pub public: bool,
    /// Host: machines (including itself) to wait for before starting.
    pub expect: usize,
    /// Host (whoever hosts at that time): end the session after this long.
    pub secs: f64,
    /// Seconds after the session start.
    pub leave_at: Option<f64>,
    pub crash_at: Option<f64>,
    pub desync_at: Option<f64>,
    /// Bot seed.
    pub seed: u64,
    /// Fail if nothing finished after this long (seconds from creation).
    pub timeout: f64,
}

impl PeerPlan {
    pub fn new(index: usize, locals: usize, mode: Mode, join: JoinHow) -> PeerPlan {
        PeerPlan {
            index,
            locals,
            spectator: locals == 0,
            mode,
            join,
            public: false,
            expect: 1,
            secs: 3.0,
            leave_at: None,
            crash_at: None,
            desync_at: None,
            seed: 0x5eed ^ index as u64,
            timeout: 120.0,
        }
    }
}

/// How a machine's run ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerResult {
    pub machine: Option<MachineId>,
    pub agreed: bool,
    /// Desync reports this machine saw.
    pub desyncs: u32,
    /// Our final hashes folded into one value.
    pub final_hash: u64,
    /// Entries applied by our replica.
    pub applied: u64,
    pub migrations: u32,
    pub aborted: Option<String>,
    /// Turns our bots made.
    pub turns: u64,
    /// Players in the session at the end (active).
    pub players: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeerStatus {
    Running,
    Ended(PeerResult),
    Left,
    Crashed,
    Failed(String),
}

pub struct Peer {
    pub plan: PeerPlan,
    pub net: NetSession,
    pub replica: Option<ToyReplica>,
    bots: Vec<(ToyBot, Instant)>,
    born: Instant,
    t0: Option<Instant>,
    acted: bool,
    started: bool,
    ended_by_me: bool,
    desynced: bool,
    desyncs: u32,
    turns: u64,
    status: PeerStatus,
    room: Option<(String, Option<String>, String)>,
    trace_active: Vec<u8>,
    /// Log lines (prefixed with the machine index) for the caller to print.
    pub log: Vec<String>,
}

impl Peer {
    pub fn new(net: NetSession, plan: PeerPlan) -> Peer {
        Peer {
            net,
            replica: None,
            bots: Vec::new(),
            born: Instant::now(),
            t0: None,
            acted: false,
            started: false,
            ended_by_me: false,
            desynced: false,
            desyncs: 0,
            turns: 0,
            status: PeerStatus::Running,
            room: None,
            trace_active: Vec::new(),
            log: Vec::new(),
            plan,
        }
    }

    pub fn status(&self) -> &PeerStatus {
        &self.status
    }

    pub fn running(&self) -> bool {
        self.status == PeerStatus::Running
    }

    /// Room id, code and invite link once hosted.
    pub fn room(&self) -> Option<&(String, Option<String>, String)> {
        self.room.as_ref()
    }

    fn say(&mut self, s: String) {
        self.log.push(format!("[m{}] {s}", self.plan.index));
    }

    fn act(&mut self) {
        if self.acted {
            return;
        }
        self.acted = true;
        let names: Vec<String> = (0..self.plan.locals)
            .map(|i| format!("P{}.{}", self.plan.index, i + 1))
            .collect();
        self.net.set_local_players(names);
        self.net.set_spectate(self.plan.spectator);
        self.net.set_ready(true);
        match self.plan.join.clone() {
            JoinHow::Host => {
                if self.plan.public {
                    self.net.host_public()
                } else {
                    self.net.host_private()
                }
            }
            JoinHow::Code(c) => self.net.join_code(&c),
            JoinHow::Launch(args) => {
                let a = self.net.handle_launch(&args);
                self.say(format!("launch: {a:?}"));
            }
        }
    }

    fn fail(&mut self, why: String) {
        self.say(format!("FAIL {why}"));
        self.status = PeerStatus::Failed(why);
    }

    fn finish(&mut self, agreed: bool, aborted: Option<String>, final_hash: u64) {
        let st = self.net.stats();
        let players = self.net.players().iter().filter(|p| p.active).count();
        self.status = PeerStatus::Ended(PeerResult {
            machine: self.net.me(),
            agreed,
            desyncs: self.desyncs,
            final_hash,
            applied: st.applied,
            migrations: st.migrations,
            aborted,
            turns: self.turns,
            players,
        });
    }

    /// Network, events, bots, plan. Call often (every few ms).
    pub fn step(&mut self) {
        if !self.running() {
            // Keep the machine on the network (its last messages, e.g. the
            // host's `Ended`, may still sit in the link conditioner).
            if matches!(self.status, PeerStatus::Ended(_)) {
                self.net.poll();
                while self.net.next_event().is_some() {}
            }
            return;
        }
        if self.born.elapsed().as_secs_f64() > self.plan.timeout {
            self.fail("timeout".into());
            return;
        }
        self.net.poll();
        while let Some(e) = self.net.next_event() {
            self.on_event(e);
            if !self.running() {
                return;
            }
        }
        if self.net.is_host() && !self.started && self.net.session().is_none() {
            let lobby = self.net.lobby();
            let ready = lobby.machines.len() >= self.plan.expect;
            if ready {
                let _ = self.net.set_mode(self.plan.mode);
                match self.net.start_session(Some(0xBABA ^ self.plan.seed)) {
                    Ok(s) => {
                        self.started = true;
                        self.say(format!(
                            "started session {} ({:?}) with {} machines",
                            s.session,
                            s.mode,
                            lobby_len(&self.net)
                        ));
                    }
                    Err(e) => self.say(format!("start: {e}")),
                }
            }
        }
        self.play();
    }

    fn on_event(&mut self, e: NetEvent) {
        match e {
            NetEvent::Warpzone(ev) => match *ev {
                Event::LoggedIn {
                    account: Some(_), ..
                }
                | Event::Registered(_) => self.act(),
                Event::RoomHosted {
                    room, code, invite, ..
                } => {
                    self.say(format!("hosted {} code {code:?}", room.id));
                    if self.room.is_none() {
                        self.room = Some((room.id, code, invite));
                    }
                }
                Event::Error { op, message, .. } => {
                    self.say(format!("warpzone error {op}: {message}"))
                }
                _ => {}
            },
            NetEvent::Welcome { machine } => self.say(format!("welcome as {machine}")),
            NetEvent::Rejected { reason } => self.fail(format!("rejected: {reason}")),
            NetEvent::SessionStarted(s) => {
                self.say(format!(
                    "session {} {:?} seed {:x}",
                    s.session, s.mode, s.seed
                ));
                self.replica = Some(ToyReplica::new(&s));
                if self.t0.is_none() {
                    self.t0 = Some(Instant::now());
                }
                let now = Instant::now();
                self.bots = (0..self.plan.locals)
                    .map(|i| {
                        (
                            ToyBot::new(self.plan.seed.wrapping_mul(31).wrapping_add(i as u64)),
                            now,
                        )
                    })
                    .collect();
            }
            NetEvent::Desync(r) => {
                self.desyncs += 1;
                self.say(format!(
                    "DESYNC {} at {}: {:?} (majority {:x})",
                    r.stream,
                    r.index,
                    r.desynced(),
                    r.majority
                ));
            }
            NetEvent::SessionEnded(r) => {
                let me = self.net.me();
                let mine = r
                    .hashes
                    .iter()
                    .find(|(m, _)| Some(*m) == me)
                    .map(|(_, h)| fold(h))
                    .unwrap_or(0);
                self.say(format!(
                    "ended: agreed {} ({} machines reported, {} desync reports) final {mine:016x}",
                    r.agreed,
                    r.hashes.len(),
                    r.desyncs.len()
                ));
                if !r.agreed {
                    for (m, h) in &r.hashes {
                        let lens: Vec<String> =
                            r.lengths.iter().map(|(s, l)| format!("{s}:{l}")).collect();
                        let hs: Vec<String> = h
                            .iter()
                            .map(|(s, v)| format!("{s}={:04x}", v & 0xffff))
                            .collect();
                        self.say(format!("  {m}: {} | {}", hs.join(" "), lens.join(" ")));
                    }
                }
                self.finish(r.agreed, None, mine);
            }
            NetEvent::SessionAborted { reason } => {
                self.say(format!("aborted: {reason}"));
                self.finish(false, Some(reason), 0);
            }
            NetEvent::HostLost { successor } => {
                self.say(format!("host lost, successor {successor}"))
            }
            NetEvent::HostMigrated { host, room } => {
                self.say(format!("host migrated to {host}, room {room}"))
            }
            NetEvent::RoomLeft { reason } => {
                if self.running() {
                    self.fail(format!("left room: {reason}"));
                }
            }
            NetEvent::MachineLeft { machine } => self.say(format!("{machine} left")),
            _ => {}
        }
    }

    fn play(&mut self) {
        let Some(t0) = self.t0 else { return };
        let t = t0.elapsed().as_secs_f64();
        if self.plan.crash_at.is_some_and(|c| t >= c) {
            self.say("crash".into());
            self.net.simulate_crash();
            self.status = PeerStatus::Crashed;
            return;
        }
        if self.plan.leave_at.is_some_and(|c| t >= c) {
            self.say("leaving".into());
            self.net.leave();
            self.status = PeerStatus::Left;
            return;
        }
        if std::env::var("BABA_NET_TRACE").is_ok() {
            let act: Vec<u8> = self
                .net
                .players()
                .iter()
                .filter(|p| p.active)
                .map(|p| p.info.player.0)
                .collect();
            if act != self.trace_active {
                self.log.push(format!(
                    "[m{}] active {act:?} shared {}",
                    self.plan.index,
                    self.net.stream(StreamId::Shared).len()
                ));
                self.trace_active = act;
            }
        }
        if let Some(r) = self.replica.as_mut() {
            self.net.pump(r);
            if self.plan.desync_at.is_some_and(|d| t >= d) && !self.desynced {
                self.desynced = true;
                let stream = match r.mode {
                    Mode::Coop => StreamId::Shared,
                    _ => match self.net.local_player(0) {
                        Some(p) => StreamId::Player(p),
                        None => StreamId::Shared,
                    },
                };
                r.corrupt(stream);
                self.log
                    .push(format!("[m{}] corrupting {stream}", self.plan.index));
            }
            // Mode 2: our players' clears become shared save events.
            let clears = r.take_clears();
            if r.mode == Mode::SharedSave {
                for (p, level) in clears {
                    if let Some(local) =
                        (0..self.plan.locals).find(|&i| self.net.local_player(i) == Some(p))
                    {
                        let _ = self.net.event(local, &level.to_le_bytes());
                    }
                }
            }
        }
        if self.net.accepting_input() && !self.net.is_migrating() {
            let now = Instant::now();
            for i in 0..self.bots.len() {
                if self.bots[i].1 > now || self.net.local_player(i).is_none() {
                    continue;
                }
                let input = self.bots[i].0.input();
                let wait = self.bots[i].0.delay_ms();
                self.bots[i].1 = now + Duration::from_millis(wait);
                if self.net.input(i, &[input]).is_ok() {
                    self.turns += 1;
                }
            }
        }
        if self.net.is_host()
            && !self.ended_by_me
            && t >= self.plan.secs
            && self.net.session().is_some()
            && !self.net.is_migrating()
            && self.net.lobby().phase != Phase::Lobby
        {
            self.ended_by_me = true;
            self.say(format!("ending session at {t:.1}s"));
            if let Err(e) = self.net.end_session() {
                self.say(format!("end: {e}"));
            }
        }
    }
}

fn lobby_len(net: &NetSession) -> usize {
    net.lobby().machines.len()
}

/// One value for a machine's final hashes.
pub fn fold(h: &[(StreamId, u64)]) -> u64 {
    let mut x = 0xcbf2_9ce4_8422_2325u64;
    for (s, v) in h {
        let tag = match s {
            StreamId::Shared => 0xffu64,
            StreamId::Player(p) => p.0 as u64,
        };
        x = (x ^ tag).wrapping_mul(0x100_0000_01b3);
        x = (x ^ v).wrapping_mul(0x100_0000_01b3);
    }
    x
}

/// A whole run: machines, mode, link, events.
#[derive(Clone, Debug)]
pub struct RunSpec {
    /// Local players per machine (machine 0 hosts; 0 = spectator).
    pub locals: Vec<usize>,
    pub mode: Mode,
    /// Session length (the host ends it).
    pub secs: f64,
    pub link: crate::link::LinkSim,
    /// `(machine, seconds after the session start)`.
    pub leave: Vec<(usize, f64)>,
    pub crash: Vec<(usize, f64)>,
    pub desync: Vec<(usize, f64)>,
    /// Machines that join only this long after the session started.
    pub late: Vec<(usize, f64)>,
    pub abort_on_desync: bool,
    pub public: bool,
    /// Route every connection through the Warpzone relay.
    pub relay: bool,
}

impl RunSpec {
    pub fn new(locals: &[usize], mode: Mode) -> RunSpec {
        RunSpec {
            locals: locals.to_vec(),
            mode,
            secs: 3.0,
            link: crate::link::LinkSim::default(),
            leave: Vec::new(),
            crash: Vec::new(),
            desync: Vec::new(),
            late: Vec::new(),
            abort_on_desync: false,
            public: false,
            relay: false,
        }
    }

    fn at(v: &[(usize, f64)], m: usize) -> Option<f64> {
        v.iter().find(|(i, _)| *i == m).map(|(_, t)| *t)
    }

    pub fn late_at(&self, m: usize) -> Option<f64> {
        Self::at(&self.late, m)
    }

    /// The plan of machine `m` (join method filled in by the runner).
    pub fn plan(&self, m: usize, join: JoinHow) -> PeerPlan {
        let mut p = PeerPlan::new(m, self.locals[m], self.mode, join);
        p.public = self.public;
        p.expect = (0..self.locals.len())
            .filter(|&i| self.late_at(i).is_none())
            .count();
        p.secs = self.secs;
        p.leave_at = Self::at(&self.leave, m);
        p.crash_at = Self::at(&self.crash, m);
        // A late joiner's clock starts when it gets the session.
        p.desync_at = Self::at(&self.desync, m);
        p.timeout = self.secs + 90.0;
        p
    }

    /// How machine `m` joins: odd ones type the code, even ones come in
    /// through `--warpzone-join` with the invite link (the launcher path),
    /// or the room id for public rooms.
    pub fn join_for(&self, m: usize, room: &str, code: Option<&str>, invite: &str) -> JoinHow {
        if m == 0 {
            return JoinHow::Host;
        }
        match code {
            Some(c) if m % 2 == 1 => JoinHow::Code(c.to_string()),
            _ if self.public => JoinHow::Launch(vec!["--warpzone-join".into(), room.into()]),
            _ => JoinHow::Launch(vec!["--warpzone-join".into(), invite.into()]),
        }
    }

    pub fn net_config(&self) -> crate::session::NetConfig {
        let mut c = crate::session::NetConfig::new("netsim-1");
        c.content = "toy-content-1".into();
        c.link_sim = self.link;
        c.force_relay = self.relay;
        if self.abort_on_desync {
            c.desync_policy = crate::session::DesyncPolicy::Abort;
        }
        c
    }

    /// PASS/FAIL of a finished run.
    pub fn check(&self, results: &[PeerStatus]) -> Result<String, String> {
        let mut finals = BTreeSet::new();
        let mut ended = 0;
        let mut turns = 0;
        for (m, st) in results.iter().enumerate() {
            let leaves = Self::at(&self.leave, m).is_some();
            let crashes = Self::at(&self.crash, m).is_some();
            match st {
                PeerStatus::Left if leaves => continue,
                PeerStatus::Crashed if crashes => continue,
                PeerStatus::Ended(r) => {
                    ended += 1;
                    turns += r.turns;
                    if !self.desync.is_empty() {
                        if r.desyncs == 0 {
                            return Err(format!("m{m}: injected desync not reported"));
                        }
                        if self.abort_on_desync
                            && !r.aborted.as_deref().is_some_and(|a| a.contains("desync"))
                        {
                            return Err(format!("m{m}: not aborted: {:?}", r.aborted));
                        }
                        continue;
                    }
                    if let Some(a) = &r.aborted {
                        return Err(format!("m{m}: aborted: {a}"));
                    }
                    if !r.agreed || r.desyncs > 0 {
                        return Err(format!("m{m}: agreed {} desyncs {}", r.agreed, r.desyncs));
                    }
                    let host_gone =
                        Self::at(&self.leave, 0).is_some() || Self::at(&self.crash, 0).is_some();
                    if host_gone && r.migrations == 0 {
                        return Err(format!("m{m}: host left but no migration"));
                    }
                    finals.insert(r.final_hash);
                }
                other => return Err(format!("m{m}: {other:?}")),
            }
        }
        if ended == 0 {
            return Err("nobody finished".into());
        }
        if finals.len() > 1 {
            return Err(format!("final hashes differ: {finals:x?}"));
        }
        if turns == 0 {
            return Err("no turns were made".into());
        }
        if !self.desync.is_empty() {
            return Ok(format!(
                "desync reported to all {ended} machines, {turns} turns"
            ));
        }
        Ok(format!("{ended} machines agree, {turns} turns"))
    }
}

use std::collections::BTreeSet;

/// Run `spec` with every machine in this process on a [`crate::mem::MemNet`].
pub fn run_memory(spec: &RunSpec, verbose: bool) -> Vec<PeerStatus> {
    use crate::mem::MemNet;
    let net = MemNet::new(crate::session::GAME_ID);
    let n = spec.locals.len();
    let mut peers: Vec<Option<Peer>> = (0..n).map(|_| None).collect();
    let mk = |m: usize, join: JoinHow| {
        let mut s = NetSession::memory(net.client(&format!("m{m}")), spec.net_config());
        s.seed_link_sim(0x1234 + m as u64);
        Peer::new(s, spec.plan(m, join))
    };
    peers[0] = Some(mk(0, JoinHow::Host));
    let mut room: Option<(String, Option<String>, String)> = None;
    let mut t0: Option<Instant> = None;
    let born = Instant::now();
    let flush = |p: &mut Peer| {
        if verbose {
            for l in p.log.drain(..) {
                eprintln!("{l}");
            }
        } else {
            p.log.clear();
        }
    };
    loop {
        if room.is_none()
            && let Some(r) = peers[0].as_ref().and_then(|p| p.room().cloned())
        {
            room = Some(r);
        }
        if t0.is_none() && peers[0].as_ref().is_some_and(|p| p.t0.is_some()) {
            t0 = Some(Instant::now());
        }
        if let Some((id, code, invite)) = &room {
            for (m, slot) in peers.iter_mut().enumerate().skip(1) {
                if slot.is_some() {
                    continue;
                }
                let due = match spec.late_at(m) {
                    None => true,
                    Some(t) => t0.is_some_and(|t0| t0.elapsed().as_secs_f64() >= t),
                };
                if due {
                    *slot = Some(mk(m, spec.join_for(m, id, code.as_deref(), invite)));
                }
            }
        }
        let mut running = false;
        for p in peers.iter_mut().flatten() {
            p.step();
            flush(p);
            running |= p.running();
        }
        if !running && peers.iter().all(|p| p.is_some()) {
            break;
        }
        if !running && (room.is_none() || born.elapsed().as_secs_f64() > spec.secs + 120.0) {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    peers
        .into_iter()
        .map(|p| p.map_or(PeerStatus::Failed("never started".into()), |p| p.status))
        .collect()
}
