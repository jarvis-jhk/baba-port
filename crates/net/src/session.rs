//! [`NetSession`]: rooms, the host-authoritative lobby, session start and
//! end, turn streams (host-sequenced shared stream, owner-appended player
//! streams), late join by replay, host migration and desync detection.
//! See `docs/net.md`.

use crate::link::{Link, LinkSim, Outbox, TrafficStats};
use crate::mem::MemClient;
use crate::wire::{
    self, APPEND_BATCH, CH_SESSION, Dechunker, DesyncReport, EndReport, Entry, EntryKind,
    LobbyState, MAX_EVENT, MAX_INPUT, MAX_LOCAL_PLAYERS, MAX_PLAYERS, MAX_SETTINGS, MachineId,
    MachineView, Mode, Msg, PROTOCOL, Phase, PlayerId, PlayerInfo, Resume, SessionStart, StreamId,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, Instant};
use warpzone_core::client::{Client, Config as WzConfig, Event, HostOptions};
use warpzone_core::net::PeerId;
use warpzone_core::protocol::api::{Entry as RoomEntry, Visibility};

/// Warpzone game id (`packaging/warpzone/baba-port.toml`).
pub const GAME_ID: &str = "baba-port";
/// Most machines in a room (12 playing machines plus spectators).
pub const MAX_MACHINES: usize = 16;
/// Prefix of the reject reason for another build or protocol.
pub const REJECT_VERSION: &str = "version mismatch";
/// Prefix of the reject reason for other imported game content.
pub const REJECT_CONTENT: &str = "content mismatch";
/// Room metadata key that marks a room made by host migration.
pub const META_MIGRATE: &str = "migrate";
/// Room metadata key: the machine id hosting a migration room.
pub const META_MHOST: &str = "mhost";

const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const END_TIMEOUT: Duration = Duration::from_secs(30);
const HASH_TIMEOUT: Duration = Duration::from_secs(10);
const RECONCILE_TIMEOUT: Duration = Duration::from_secs(12);
const SUCCESSOR_TIMEOUT: Duration = Duration::from_secs(10);
const LIST_EVERY: Duration = Duration::from_millis(700);
const MAX_DESYNC_REPORTS: usize = 64;

/// The game's state as the session replays it: one call per stream entry,
/// in order, and a hash of what a stream drives. See [`NetSession::pump`].
pub trait Replica {
    /// Apply entry `index` of `stream`. Mode 2: a player-stream entry comes
    /// only after shared entries `0..entry.sync` were applied.
    fn apply(&mut self, stream: StreamId, index: u32, entry: &Entry);
    /// Hash of the state `stream` drives (the shared stream: the co-op
    /// level or the shared save; a player stream: that player's instance).
    fn state_hash(&mut self, stream: StreamId) -> u64;
}

/// What the host does when machines disagree on a state hash.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DesyncPolicy {
    /// Report it (`NetEvent::Desync`, `EndReport::desyncs`) and play on.
    #[default]
    Report,
    /// End the session for everybody (`NetEvent::SessionAborted`).
    Abort,
}

#[derive(Clone, Debug)]
pub struct NetConfig {
    /// The game build (version + anything that changes the sim); rooms only
    /// accept machines with the same one.
    pub build: String,
    /// Identity of the imported game content the sim reads (a hash of the
    /// player's Baba Is You data as baba-port uses it); must equal the
    /// host's. Empty = not checked.
    pub content: String,
    /// Warpzone server; `None` = `WARPZONE_SERVER` or the public instance.
    pub server: Option<String>,
    /// Register this username if the identity has none yet.
    pub auto_register: Option<String>,
    /// Route every peer connection through the server relay (testing;
    /// `BABA_NET_FORCE_RELAY=1`).
    pub force_relay: bool,
    /// Artificial latency/jitter/loss on everything we send (testing;
    /// `BABA_NET_DELAY_MS`, `BABA_NET_JITTER_MS`, `BABA_NET_LOSS`).
    pub link_sim: LinkSim,
    /// Host: a state hash every n entries of a stream (0 = never).
    pub hash_interval: u32,
    /// Host: what a desync does.
    pub desync_policy: DesyncPolicy,
    /// When the host leaves, regroup around a new host and continue
    /// (`false`: the session ends for everybody).
    pub host_migration: bool,
    /// Give up regrouping after this long.
    pub migration_timeout: Duration,
}

impl NetConfig {
    pub fn new(build: &str) -> NetConfig {
        NetConfig {
            build: build.into(),
            content: String::new(),
            server: None,
            auto_register: None,
            force_relay: std::env::var("BABA_NET_FORCE_RELAY").is_ok_and(|v| v == "1"),
            link_sim: LinkSim::from_env(),
            hash_interval: 4,
            desync_policy: DesyncPolicy::Report,
            host_migration: true,
            migration_timeout: Duration::from_secs(40),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetError {
    Warpzone(String),
    NotInRoom,
    NotHost,
    NoSession,
    InSession,
    /// The session is ending (or regrouping); no new turns.
    Stopped,
    /// No admitted player at this local index.
    NotLocal(usize),
    TooLarge,
    /// Nobody to play.
    NoPlayers,
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for NetError {}

/// What `--warpzone-join` / `--warpzone-host` / an invite link asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchAction {
    None,
    /// A join is under way: show "connecting…".
    Joining {
        room: String,
    },
    /// A join by code (`--warpzone-code`) is under way.
    JoiningCode {
        code: String,
    },
    /// Open the host screen (then [`NetSession::host`]).
    HostScreen,
}

#[derive(Debug, Clone)]
pub enum NetEvent {
    /// Warpzone events the game may show: `LoggedIn`, `Registered`,
    /// `RoomHosted` (code, invite link), `RoomJoined`, `RoomList`,
    /// `Knock` (approve rooms), `Knocking`, `KnockDenied`, `Error`, ...
    Warpzone(Box<Event>),
    /// We are machine `machine` of this room.
    Welcome {
        machine: MachineId,
    },
    /// The host refused us (`REJECT_VERSION`, `REJECT_CONTENT`, full).
    Rejected {
        reason: String,
    },
    /// `lobby()` changed (machines, players, mode, settings, phase).
    LobbyChanged,
    MachineJoined {
        machine: MachineId,
        name: String,
    },
    MachineLeft {
        machine: MachineId,
    },
    /// A session starts (or we got a fresh copy of a running one): build an
    /// empty replica and [`NetSession::pump`] from entry 0.
    SessionStarted(SessionStart),
    Desync(DesyncReport),
    /// The session ended; everybody is back in the lobby.
    SessionEnded(EndReport),
    SessionAborted {
        reason: String,
    },
    /// The host is gone; the machines regroup around `successor`. Turns made
    /// meanwhile are kept and sequenced later (mode 1) or stay valid (own
    /// streams, modes 2/3).
    HostLost {
        successor: MachineId,
    },
    /// Regrouped: `host` hosts room `room` now.
    HostMigrated {
        host: MachineId,
        room: String,
    },
    /// We are out of the room.
    RoomLeft {
        reason: String,
    },
    /// Game data outside the streams ([`NetSession::send_app`]).
    App {
        from: MachineId,
        data: Vec<u8>,
    },
}

/// A turn or event we submitted to the host that is not in the shared
/// stream yet (mode 1 prediction, see docs/net.md).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    pub player: PlayerId,
    pub seq: u32,
    pub kind: EntryKind,
    pub data: Vec<u8>,
}

/// A player of the running session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerState {
    pub info: PlayerInfo,
    pub active: bool,
    /// Index of the `Joined` entry in the shared stream.
    pub joined_at: u32,
    pub left_at: Option<u32>,
}

#[derive(Clone, Debug, Default)]
pub struct PeerStats {
    pub machine: MachineId,
    pub rtt_ms: Option<u32>,
    pub relayed: Option<bool>,
}

#[derive(Clone, Debug, Default)]
pub struct NetStats {
    pub traffic: TrafficStats,
    /// Our submissions not sequenced yet.
    pub pending: usize,
    /// Chunks held back by the link conditioner.
    pub backlog: usize,
    pub peers: Vec<PeerStats>,
    pub migrations: u32,
    pub desyncs: u32,
    /// Entries of all streams / applied by `pump`.
    pub entries: u64,
    pub applied: u64,
}

struct Session {
    start: SessionStart,
    logs: BTreeMap<StreamId, Vec<Entry>>,
    applied: BTreeMap<StreamId, u32>,
    players: BTreeMap<PlayerId, PlayerState>,
    /// Our next submission number per local player.
    seq: BTreeMap<PlayerId, u32>,
    pending: Vec<Pending>,
    stopped: bool,
    end_lengths: Option<Vec<(StreamId, u32)>>,
    final_sent: bool,
    /// Our own hashes (stream, index) → hash.
    own_hashes: BTreeMap<(StreamId, u32), u64>,
    /// [`NetSession::restart_replay`]: hashes already reported are not
    /// sent again.
    replaying: bool,
}

impl Session {
    fn new(start: SessionStart) -> Session {
        let mut logs = BTreeMap::new();
        logs.insert(StreamId::Shared, Vec::new());
        Session {
            start,
            logs,
            applied: BTreeMap::new(),
            players: BTreeMap::new(),
            seq: BTreeMap::new(),
            pending: Vec::new(),
            stopped: false,
            end_lengths: None,
            final_sent: false,
            own_hashes: BTreeMap::new(),
            replaying: false,
        }
    }

    fn len(&self, s: StreamId) -> u32 {
        self.logs.get(&s).map_or(0, |l| l.len() as u32)
    }

    fn lengths(&self) -> Vec<(StreamId, u32)> {
        self.logs
            .iter()
            .map(|(s, l)| (*s, l.len() as u32))
            .collect()
    }

    /// Append one entry and update the roster / our pending list.
    fn push(&mut self, s: StreamId, e: Entry, me: Option<MachineId>) {
        let log = self.logs.entry(s).or_default();
        let index = log.len() as u32;
        if s == StreamId::Shared {
            match e.kind {
                EntryKind::Joined => {
                    if let Some(info) = e.player_info() {
                        self.logs.entry(StreamId::Player(info.player)).or_default();
                        self.players.insert(
                            info.player,
                            PlayerState {
                                info,
                                active: true,
                                joined_at: index,
                                left_at: None,
                            },
                        );
                    }
                }
                EntryKind::Left => {
                    if let Some(p) = self.players.get_mut(&e.player) {
                        p.active = false;
                        p.left_at = Some(index);
                    }
                    if let Some(n) = e.final_len()
                        && let Some(l) = self.logs.get_mut(&StreamId::Player(e.player))
                        && l.len() > n as usize
                    {
                        // Turns the host never got: nobody else has them.
                        l.truncate(n as usize);
                    }
                }
                EntryKind::Input | EntryKind::Event => {
                    let mine = self
                        .players
                        .get(&e.player)
                        .is_some_and(|p| Some(p.info.machine) == me);
                    if mine {
                        self.pending
                            .retain(|p| !(p.player == e.player && p.seq <= e.seq));
                    }
                }
            }
        }
        self.logs.entry(s).or_default().push(e);
    }

    /// Merge entries `first..` (overlap skipped). Returns how many were new,
    /// or `None` for a gap.
    fn merge(
        &mut self,
        s: StreamId,
        first: u32,
        entries: Vec<Entry>,
        me: Option<MachineId>,
    ) -> Option<usize> {
        let len = self.len(s);
        if first > len {
            return None;
        }
        let skip = (len - first) as usize;
        let mut n = 0;
        for e in entries.into_iter().skip(skip) {
            self.push(s, e, me);
            n += 1;
        }
        Some(n)
    }

    fn local_player(&self, me: MachineId, local: usize) -> Option<PlayerId> {
        self.players
            .values()
            .find(|p| p.active && p.info.machine == me && p.info.local_index as usize == local)
            .map(|p| p.info.player)
    }

    fn active_players(&self) -> usize {
        self.players.values().filter(|p| p.active).count()
    }
}

struct HostMachine {
    peer: Option<PeerId>,
    view: MachineView,
    /// The machine asked to watch only (`set_spectate`); without players
    /// it watches anyway.
    spectate: bool,
    /// How much of each stream this machine has (what we sent it, or what it
    /// told us it has).
    sent: BTreeMap<StreamId, u32>,
}

struct EndCollect {
    lengths: Vec<(StreamId, u32)>,
    finals: BTreeMap<MachineId, Vec<(StreamId, u64)>>,
    waiting: BTreeSet<MachineId>,
    since: Instant,
}

struct Reconcile {
    expected: BTreeSet<MachineId>,
    since: Instant,
    wants: BTreeMap<(MachineId, StreamId), u32>,
    queued: Vec<(MachineId, Pending)>,
}

#[derive(Default)]
struct HostState {
    machines: BTreeMap<MachineId, HostMachine>,
    peers: BTreeMap<PeerId, MachineId>,
    /// Joined, no `Hello` yet: account and name.
    unknown: BTreeMap<PeerId, (i64, String)>,
    next_machine: u8,
    next_session: u32,
    next_player: u8,
    seq_seen: BTreeMap<PlayerId, u32>,
    pending_hash: BTreeMap<(StreamId, u32), (BTreeMap<MachineId, u64>, Instant)>,
    desync_told: BTreeSet<(StreamId, MachineId)>,
    desyncs: Vec<DesyncReport>,
    stop: Option<(BTreeSet<MachineId>, Instant)>,
    end: Option<EndCollect>,
    reconcile: Option<Reconcile>,
    /// Host migration: accounts of the old room and their machine ids.
    old_accounts: BTreeMap<i64, MachineId>,
    old_room: Option<String>,
    kicks: Vec<(i64, Instant)>,
    metadata: BTreeMap<String, String>,
}

struct Migration {
    old_room: String,
    candidates: Vec<MachineId>,
    roster: Vec<MachineView>,
    since: Instant,
    cand_since: Instant,
    last_list: Option<Instant>,
    hosting: bool,
    joining: Option<String>,
}

enum Role {
    None,
    Host(Box<HostState>),
    Guest {
        host_peer: Option<PeerId>,
        welcomed: bool,
    },
    Migrating(Box<Migration>),
}

/// One machine's view of online play. Single-threaded: call everything from
/// the game thread, [`NetSession::poll`] every frame.
pub struct NetSession {
    cfg: NetConfig,
    link: Box<dyn Link>,
    out: Outbox,
    traffic: TrafficStats,
    events: VecDeque<NetEvent>,
    role: Role,
    me: Option<MachineId>,
    room: String,
    dechunk: BTreeMap<PeerId, Dechunker>,
    lobby: LobbyState,
    my_players: Vec<String>,
    ready: bool,
    spectate: bool,
    session: Option<Session>,
    /// Guest: rejoining after a migration from this room.
    resume_from: Option<String>,
    /// Guest: the stream lengths our last `Hello` reported.
    hello_lengths: BTreeMap<StreamId, u32>,
    migrations: u32,
    desyncs_seen: u32,
    applied_total: u64,
    rejected: bool,
    /// Extra room-browser labels ([`NetSession::set_room_label`]).
    room_labels: BTreeMap<String, String>,
}

fn sanitize_players(v: Vec<String>) -> Vec<String> {
    v.into_iter()
        .take(MAX_LOCAL_PLAYERS)
        .enumerate()
        .map(|(i, n)| {
            let n: String = n.trim().chars().take(16).collect();
            if n.is_empty() {
                format!("Player {}", i + 1)
            } else {
                n
            }
        })
        .collect()
}

fn random_seed() -> u64 {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut x = t ^ ((std::process::id() as u64) << 32) ^ 0x9E37_79B9_7F4A_7C15;
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    x
}

impl NetSession {
    /// Connect to Warpzone (identity from `WARPZONE_HOME` or the shared one).
    pub fn new(cfg: NetConfig) -> Result<NetSession, NetError> {
        crate::tls::install_crypto_provider();
        let mut wc = WzConfig::new(GAME_ID, &cfg.build);
        if let Some(s) = &cfg.server {
            wc.server = s.clone();
        }
        let client = Client::new(wc).map_err(|e| NetError::Warpzone(e.to_string()))?;
        Ok(Self::with_link(Box::new(client), cfg))
    }

    /// A machine on an in-process [`crate::mem::MemNet`].
    pub fn memory(client: MemClient, cfg: NetConfig) -> NetSession {
        Self::with_link(Box::new(client), cfg)
    }

    /// Any [`Link`].
    pub fn with_link(link: Box<dyn Link>, cfg: NetConfig) -> NetSession {
        NetSession {
            out: Outbox::new(cfg.link_sim),
            cfg,
            link,
            traffic: TrafficStats::default(),
            events: VecDeque::new(),
            role: Role::None,
            me: None,
            room: String::new(),
            dechunk: BTreeMap::new(),
            lobby: LobbyState::default(),
            my_players: Vec::new(),
            ready: false,
            spectate: false,
            session: None,
            resume_from: None,
            hello_lengths: BTreeMap::new(),
            migrations: 0,
            desyncs_seen: 0,
            applied_total: 0,
            rejected: false,
            room_labels: BTreeMap::new(),
        }
    }

    /// Seed the link conditioner (reproducible tests).
    pub fn seed_link_sim(&mut self, seed: u64) {
        self.out.reseed(seed);
    }

    pub fn config(&self) -> &NetConfig {
        &self.cfg
    }

    // ------------------------------------------------------------ rooms

    /// The Warpzone client, for anything not wrapped here.
    pub fn warpzone(&mut self) -> Option<&mut Client> {
        self.link.warpzone()
    }

    /// Act on `--warpzone-join <room>`, invite links, `WARPZONE_JOIN`,
    /// `--warpzone-code <CODE>`, `--warpzone-host` / `WARPZONE_HOST=1`.
    pub fn handle_launch<A: AsRef<str>>(&mut self, args: &[A]) -> LaunchAction {
        let intent = warpzone_core::launch::from_args(args);
        if let Some(room) = intent.join {
            self.join(&room, intent.code.as_deref(), intent.secret.as_deref());
            return LaunchAction::Joining { room };
        }
        if let Some(code) = intent.code {
            self.join_code(&code);
            return LaunchAction::JoiningCode { code };
        }
        if intent.host {
            LaunchAction::HostScreen
        } else {
            LaunchAction::None
        }
    }

    pub fn register(&mut self, username: &str) {
        self.link.register(username);
    }

    /// Host a room. `max_players` counts machines (default 16: 12 playing
    /// machines plus spectators).
    pub fn host(&mut self, mut opts: HostOptions) {
        self.apply_relay();
        if opts.max_players == 0 {
            opts.max_players = MAX_MACHINES as u32;
        }
        self.link.host(opts);
    }

    /// A public room, listed in the room browser.
    pub fn host_public(&mut self) {
        self.host(HostOptions {
            visibility: Visibility::Public,
            preset: Some("coop".into()),
            ..Default::default()
        });
    }

    /// A private room: joined by its code or invite link only.
    pub fn host_private(&mut self) {
        self.host(HostOptions {
            visibility: Visibility::Unlisted,
            preset: Some("coop".into()),
            ..Default::default()
        });
    }

    pub fn join(&mut self, room: &str, code: Option<&str>, secret: Option<&str>) {
        self.apply_relay();
        self.link.join(room, code, secret);
    }

    pub fn join_code(&mut self, code: &str) {
        self.apply_relay();
        self.link.join_code(code);
    }

    /// The room browser: `NetEvent::Warpzone(Event::RoomList(..))`.
    pub fn list_rooms(&mut self, has_space: bool) {
        self.link.list_rooms(has_space);
    }

    pub fn answer_knock(&mut self, knock: &str, accept: bool) {
        self.link.answer_knock(knock, accept);
    }

    /// Host: remove a machine.
    pub fn kick(&mut self, machine: MachineId, ban: bool) {
        let acc = match &self.role {
            Role::Host(h) => h.machines.get(&machine).map(|m| m.view.account),
            _ => None,
        };
        if let Some(a) = acc {
            self.link.kick(a, ban);
        }
    }

    /// Host: hide the room from new joiners (or open it again).
    pub fn set_room_closed(&mut self, closed: bool) {
        self.link.set_closed(closed);
    }

    /// Leave the room. As host: the others regroup around a new host (or
    /// the session ends, see `NetConfig::host_migration`).
    pub fn leave(&mut self) {
        self.out.flush(self.link.as_mut(), &mut self.traffic);
        self.link.leave();
        self.left_room("left");
    }

    /// Testing: vanish like a crashed process (no goodbye to anyone).
    pub fn simulate_crash(&mut self) {
        self.link.crash();
        self.out.clear();
        self.session = None;
        self.role = Role::None;
        self.me = None;
    }

    fn apply_relay(&mut self) {
        if self.cfg.force_relay {
            self.link.force_relay(true);
        }
    }

    // ------------------------------------------------------------ lobby

    pub fn is_host(&self) -> bool {
        matches!(self.role, Role::Host(_))
    }

    pub fn in_room(&self) -> bool {
        !matches!(self.role, Role::None)
    }

    pub fn is_migrating(&self) -> bool {
        matches!(self.role, Role::Migrating(_))
            || matches!(&self.role, Role::Host(h) if h.reconcile.is_some())
    }

    /// Our machine id once welcomed.
    pub fn me(&self) -> Option<MachineId> {
        self.me
    }

    /// Room id (empty when not in a room).
    pub fn room(&self) -> &str {
        &self.room
    }

    pub fn lobby(&self) -> &LobbyState {
        &self.lobby
    }

    /// This machine's local players (names), in local order; 0 = spectator.
    pub fn set_local_players(&mut self, players: Vec<String>) {
        self.my_players = sanitize_players(players);
        let players = self.my_players.clone();
        match &mut self.role {
            Role::Host(h) => {
                if let Some(me) = self.me
                    && let Some(m) = h.machines.get_mut(&me)
                {
                    m.view.players = players;
                    m.view.spectator = self.spectate || m.view.players.is_empty();
                }
                self.host_admit();
                self.host_flush();
                self.host_broadcast_lobby();
            }
            Role::Guest { welcomed: true, .. } => self.send_host(&Msg::SetPlayers(players)),
            _ => {}
        }
    }

    pub fn set_ready(&mut self, ready: bool) {
        self.ready = ready;
        match &mut self.role {
            Role::Host(h) => {
                if let Some(me) = self.me
                    && let Some(m) = h.machines.get_mut(&me)
                {
                    m.view.ready = ready;
                }
                self.host_broadcast_lobby();
            }
            Role::Guest { welcomed: true, .. } => self.send_host(&Msg::SetReady(ready)),
            _ => {}
        }
    }

    /// Watch only (our local players are not admitted).
    pub fn set_spectate(&mut self, spectate: bool) {
        self.spectate = spectate;
        match &mut self.role {
            Role::Host(h) => {
                if let Some(me) = self.me
                    && let Some(m) = h.machines.get_mut(&me)
                {
                    m.spectate = spectate;
                    m.view.spectator = spectate || m.view.players.is_empty();
                }
                self.host_admit();
                self.host_flush();
                self.host_broadcast_lobby();
            }
            Role::Guest { welcomed: true, .. } => self.send_host(&Msg::SetSpectate(spectate)),
            _ => {}
        }
    }

    /// Host: the mode of the next session.
    pub fn set_mode(&mut self, mode: Mode) -> Result<(), NetError> {
        self.require_host()?;
        if self.session.is_some() {
            return Err(NetError::InSession);
        }
        self.lobby.mode = mode;
        self.host_broadcast_lobby();
        Ok(())
    }

    /// Host: the settings blob of the next session (opaque to baba-net).
    pub fn set_settings(&mut self, settings: Vec<u8>) -> Result<(), NetError> {
        self.require_host()?;
        if settings.len() > MAX_SETTINGS {
            return Err(NetError::TooLarge);
        }
        if self.session.is_some() {
            return Err(NetError::InSession);
        }
        self.lobby.settings = settings;
        self.host_broadcast_lobby();
        Ok(())
    }

    /// Every machine with players is ready.
    pub fn all_ready(&self) -> bool {
        self.lobby
            .machines
            .iter()
            .all(|m| m.ready || m.spectator || m.players.is_empty())
    }

    fn require_host(&self) -> Result<(), NetError> {
        match self.role {
            Role::Host(_) => Ok(()),
            Role::None => Err(NetError::NotInRoom),
            _ => Err(NetError::NotHost),
        }
    }

    /// Host: start a session with every machine in the room; their players
    /// join in machine order (at most 12).
    pub fn start_session(&mut self, seed: Option<u64>) -> Result<SessionStart, NetError> {
        self.require_host()?;
        if self.session.is_some() {
            return Err(NetError::InSession);
        }
        let Role::Host(h) = &mut self.role else {
            unreachable!()
        };
        if h.machines
            .values()
            .all(|m| m.view.spectator || m.view.players.is_empty())
        {
            return Err(NetError::NoPlayers);
        }
        h.next_session += 1;
        let start = SessionStart {
            session: h.next_session,
            mode: self.lobby.mode,
            seed: seed.unwrap_or_else(random_seed),
            settings: self.lobby.settings.clone(),
            hash_interval: self.cfg.hash_interval,
        };
        h.next_player = 0;
        h.seq_seen.clear();
        h.pending_hash.clear();
        h.desync_told.clear();
        h.desyncs.clear();
        let mut to = Vec::new();
        for m in h.machines.values_mut() {
            m.sent.clear();
            m.view.synced = true;
            if let Some(p) = m.peer {
                to.push(p);
            }
        }
        self.session = Some(Session::new(start.clone()));
        self.lobby.phase = Phase::Session(start.session);
        for p in to {
            self.send_peer(p, &Msg::Start(start.clone()));
        }
        self.emit(NetEvent::SessionStarted(start.clone()));
        self.host_admit();
        self.host_flush();
        self.host_broadcast_lobby();
        Ok(start)
    }

    /// Host: end the session: everybody stops making turns, replays to the
    /// final lengths and compares final hashes (`NetEvent::SessionEnded`).
    pub fn end_session(&mut self) -> Result<(), NetError> {
        self.require_host()?;
        let Some(s) = self.session.as_mut() else {
            return Err(NetError::NoSession);
        };
        if s.stopped {
            return Err(NetError::Stopped);
        }
        s.stopped = true;
        let session = s.start.session;
        let Role::Host(h) = &mut self.role else {
            unreachable!()
        };
        let waiting: BTreeSet<MachineId> = h
            .machines
            .iter()
            .filter(|(_, m)| m.peer.is_some() && m.view.synced)
            .map(|(id, _)| *id)
            .collect();
        let peers: Vec<PeerId> = waiting.iter().filter_map(|m| h.machines[m].peer).collect();
        h.stop = Some((waiting, Instant::now()));
        for p in peers {
            self.send_peer(p, &Msg::Stop { session });
        }
        self.host_check_stop();
        Ok(())
    }

    /// Host: abort the session for everybody.
    pub fn abort_session(&mut self, reason: &str) -> Result<(), NetError> {
        self.require_host()?;
        let Some(s) = &self.session else {
            return Err(NetError::NoSession);
        };
        let session = s.start.session;
        self.host_broadcast(
            &Msg::Abort {
                session,
                reason: reason.into(),
            },
            None,
        );
        self.finish_session_locally();
        self.emit(NetEvent::SessionAborted {
            reason: reason.into(),
        });
        self.host_broadcast_lobby();
        Ok(())
    }

    /// Game data outside the streams (chat, menu cursors), through the host;
    /// `to = None` = everybody else.
    pub fn send_app(&mut self, to: Option<MachineId>, data: Vec<u8>) {
        let Some(me) = self.me else { return };
        let msg = Msg::App { from: me, to, data };
        match &self.role {
            Role::Host(_) => self.host_route_app(msg),
            Role::Guest { welcomed: true, .. } => self.send_host(&msg),
            _ => {}
        }
    }

    // ------------------------------------------------------------ session

    pub fn session(&self) -> Option<&SessionStart> {
        self.session.as_ref().map(|s| &s.start)
    }

    /// Every stream of the session (shared first).
    pub fn streams(&self) -> Vec<StreamId> {
        self.session
            .as_ref()
            .map(|s| s.logs.keys().copied().collect())
            .unwrap_or_default()
    }

    /// The full log of a stream (everything since entry 0).
    pub fn stream(&self, s: StreamId) -> &[Entry] {
        self.session
            .as_ref()
            .and_then(|x| x.logs.get(&s))
            .map_or(&[], |l| l.as_slice())
    }

    /// How many entries of `s` [`pump`](Self::pump) applied.
    pub fn applied(&self, s: StreamId) -> u32 {
        self.session
            .as_ref()
            .and_then(|x| x.applied.get(&s).copied())
            .unwrap_or(0)
    }

    /// Have we applied everything we have?
    pub fn caught_up(&self) -> bool {
        self.session.as_ref().is_none_or(|s| {
            s.logs
                .iter()
                .all(|(id, l)| s.applied.get(id).copied().unwrap_or(0) as usize >= l.len())
        })
    }

    /// Every player the session has had, with whether they are still in.
    pub fn players(&self) -> Vec<PlayerState> {
        self.session
            .as_ref()
            .map(|s| s.players.values().cloned().collect())
            .unwrap_or_default()
    }

    /// The session player of our local player `local`, once admitted.
    pub fn local_player(&self, local: usize) -> Option<PlayerId> {
        let me = self.me?;
        if local >= self.my_players.len() {
            return None;
        }
        self.session.as_ref()?.local_player(me, local)
    }

    /// Our submissions the host has not sequenced yet, oldest first (mode 1
    /// prediction: confirmed state + these = what the player expects).
    pub fn pending(&self) -> &[Pending] {
        self.session.as_ref().map_or(&[], |s| s.pending.as_slice())
    }

    /// Can we make turns right now?
    pub fn accepting_input(&self) -> bool {
        self.session.as_ref().is_some_and(|s| !s.stopped) && self.me.is_some()
    }

    /// One turn of local player `local`: mode 1 → to the host, which
    /// sequences it into the shared stream; modes 2/3 → appended to the
    /// player's own stream at once (we own it) and broadcast.
    pub fn input(&mut self, local: usize, data: &[u8]) -> Result<PlayerId, NetError> {
        if data.len() > MAX_INPUT {
            return Err(NetError::TooLarge);
        }
        let mode = self.session.as_ref().ok_or(NetError::NoSession)?.start.mode;
        if mode.per_player() {
            self.own_append(local, EntryKind::Input, data)
        } else {
            self.submit(local, EntryKind::Input, data)
        }
    }

    /// A game event for the shared stream (mode 2: "level cleared" → the
    /// shared save), sequenced by the host.
    pub fn event(&mut self, local: usize, data: &[u8]) -> Result<PlayerId, NetError> {
        if data.len() > MAX_EVENT {
            return Err(NetError::TooLarge);
        }
        self.submit(local, EntryKind::Event, data)
    }

    /// Modes 2/3: game data of local player `local` that is not a turn (≤
    /// [`MAX_EVENT`] bytes), appended to the player's own stream like a turn
    /// (e.g. the save a player starts from in mode 3, so every machine can
    /// build that player's instance).
    pub fn own_event(&mut self, local: usize, data: &[u8]) -> Result<PlayerId, NetError> {
        if data.len() > MAX_EVENT {
            return Err(NetError::TooLarge);
        }
        let mode = self.session.as_ref().ok_or(NetError::NoSession)?.start.mode;
        if !mode.per_player() {
            return Err(NetError::NoSession);
        }
        self.own_append(local, EntryKind::Event, data)
    }

    /// Replay every stream into a fresh replica from entry 0 (the game
    /// rebuilt its replica, e.g. to recover from a desync). Hashes of
    /// indices reported before are not sent again; [`own_hash`](Self::own_hash)
    /// gives the recomputed values.
    pub fn restart_replay(&mut self) {
        if let Some(s) = self.session.as_mut() {
            s.applied.clear();
            s.replaying = true;
        }
    }

    /// Our own state hash of `stream` after entries `0..index` (as last
    /// computed by [`pump`](Self::pump)).
    pub fn own_hash(&self, stream: StreamId, index: u32) -> Option<u64> {
        self.session
            .as_ref()?
            .own_hashes
            .get(&(stream, index))
            .copied()
    }

    /// Host: an extra room-browser label (the descriptor's `display` keys,
    /// e.g. `world`), kept next to baba-net's own `players` and `mode`.
    pub fn set_room_label(&mut self, key: &str, value: &str) {
        if self.room_labels.get(key).map(String::as_str) == Some(value) {
            return;
        }
        self.room_labels.insert(key.to_string(), value.to_string());
        if self.is_host() {
            self.host_broadcast_lobby();
        }
    }

    fn check_input(&self, local: usize) -> Result<PlayerId, NetError> {
        let s = self.session.as_ref().ok_or(NetError::NoSession)?;
        if s.stopped {
            return Err(NetError::Stopped);
        }
        self.local_player(local).ok_or(NetError::NotLocal(local))
    }

    fn submit(&mut self, local: usize, kind: EntryKind, data: &[u8]) -> Result<PlayerId, NetError> {
        let player = self.check_input(local)?;
        let me = self.me;
        let s = self.session.as_mut().unwrap();
        let seq = {
            let c = s.seq.entry(player).or_insert(0);
            *c += 1;
            *c
        };
        let p = Pending {
            player,
            seq,
            kind,
            data: data.to_vec(),
        };
        let session = s.start.session;
        match &mut self.role {
            Role::Host(h) => {
                if let Some(rc) = h.reconcile.as_mut() {
                    rc.queued.push((me.unwrap(), p));
                } else {
                    self.host_sequence(p);
                    self.host_flush();
                }
            }
            Role::Guest { welcomed: true, .. } => {
                s.pending.push(p.clone());
                self.send_host(&Msg::Submit {
                    session,
                    player,
                    seq,
                    kind,
                    data: p.data,
                });
            }
            _ => s.pending.push(p),
        }
        Ok(player)
    }

    fn own_append(
        &mut self,
        local: usize,
        kind: EntryKind,
        data: &[u8],
    ) -> Result<PlayerId, NetError> {
        let player = self.check_input(local)?;
        let me = self.me;
        let s = self.session.as_mut().unwrap();
        let stream = StreamId::Player(player);
        let e = Entry {
            kind,
            player,
            seq: 0,
            sync: s.len(StreamId::Shared),
            data: data.to_vec(),
        };
        let first = s.len(stream);
        let session = s.start.session;
        s.push(stream, e.clone(), me);
        match &self.role {
            Role::Host(_) => self.host_flush(),
            Role::Guest { welcomed: true, .. } => self.send_host(&Msg::Append {
                session,
                stream,
                first,
                entries: vec![e],
            }),
            // Regrouping: kept, sent when the new host asks (`Want`).
            _ => {}
        }
        Ok(player)
    }

    /// Apply new entries to `replica` (all streams, in order), report state
    /// hashes every `hash_interval` entries, and the final hashes when the
    /// session ends. Call every frame. Returns how many entries it applied.
    pub fn pump(&mut self, replica: &mut dyn Replica) -> usize {
        self.pump_limited(replica, usize::MAX)
    }

    /// [`pump`](Self::pump) at most `max` entries (catching up after a late
    /// join without stalling a frame).
    pub fn pump_limited(&mut self, replica: &mut dyn Replica, max: usize) -> usize {
        let Some(s) = self.session.as_mut() else {
            return 0;
        };
        let interval = s.start.hash_interval;
        let mut n = 0;
        let mut reports = Vec::new();
        let ids: Vec<StreamId> = s.logs.keys().copied().collect();
        for id in ids {
            let shared_applied = s.applied.get(&StreamId::Shared).copied().unwrap_or(0);
            let log = &s.logs[&id];
            let mut a = s.applied.get(&id).copied().unwrap_or(0);
            while (a as usize) < log.len() && n < max {
                let e = &log[a as usize];
                if id != StreamId::Shared && e.sync > shared_applied {
                    break;
                }
                replica.apply(id, a, e);
                a += 1;
                n += 1;
                if interval > 0 && a % interval == 0 {
                    reports.push((id, a, replica.state_hash(id)));
                }
            }
            s.applied.insert(id, a);
        }
        self.applied_total += n as u64;
        for (id, index, hash) in reports {
            self.report_hash(id, index, hash);
        }
        // Final hashes once everything up to the final lengths is applied.
        let s = self.session.as_mut().unwrap();
        if let Some(lengths) = &s.end_lengths
            && !s.final_sent
            && lengths
                .iter()
                .all(|(id, len)| s.applied.get(id).copied().unwrap_or(0) >= *len)
        {
            let hashes: Vec<(StreamId, u64)> = lengths
                .iter()
                .map(|(id, _)| (*id, replica.state_hash(*id)))
                .collect();
            self.report_final(hashes);
        }
        n
    }

    /// The state hash after entries `0..index` of `stream` (what `pump`
    /// does by itself; for games that apply entries on their own).
    pub fn report_hash(&mut self, stream: StreamId, index: u32, hash: u64) {
        let Some(s) = self.session.as_mut() else {
            return;
        };
        let known = s.own_hashes.insert((stream, index), hash).is_some();
        if known && s.replaying {
            return;
        }
        let session = s.start.session;
        let Some(me) = self.me else { return };
        match &self.role {
            Role::Host(_) => self.host_hash(me, stream, index, hash),
            Role::Guest { welcomed: true, .. } => self.send_host(&Msg::Hash {
                session,
                stream,
                index,
                hash,
            }),
            _ => {}
        }
    }

    /// Final hashes at the end lengths (what `pump` does by itself).
    pub fn report_final(&mut self, hashes: Vec<(StreamId, u64)>) {
        let Some(s) = self.session.as_mut() else {
            return;
        };
        if s.final_sent {
            return;
        }
        s.final_sent = true;
        let session = s.start.session;
        let Some(me) = self.me else { return };
        match &self.role {
            Role::Host(_) => self.host_final(me, hashes),
            _ => self.send_host(&Msg::Final { session, hashes }),
        }
    }

    pub fn stats(&self) -> NetStats {
        let mut peers = Vec::new();
        match &self.role {
            Role::Host(h) => {
                for (id, m) in &h.machines {
                    if let Some(p) = m.peer {
                        peers.push(PeerStats {
                            machine: *id,
                            rtt_ms: self.link.peer_rtt(p).map(|d| d.as_millis() as u32),
                            relayed: self.link.peer_relayed(p),
                        });
                    }
                }
            }
            Role::Guest {
                host_peer: Some(p), ..
            } => peers.push(PeerStats {
                machine: self.lobby.host,
                rtt_ms: self.link.peer_rtt(*p).map(|d| d.as_millis() as u32),
                relayed: self.link.peer_relayed(*p),
            }),
            _ => {}
        }
        NetStats {
            traffic: self.traffic,
            pending: self.pending().len(),
            backlog: self.out.backlog(),
            peers,
            migrations: self.migrations,
            desyncs: self.desyncs_seen,
            entries: self
                .session
                .as_ref()
                .map_or(0, |s| s.logs.values().map(|l| l.len() as u64).sum()),
            applied: self.applied_total,
        }
    }

    pub fn next_event(&mut self) -> Option<NetEvent> {
        self.events.pop_front()
    }

    // ------------------------------------------------------------ poll

    /// Network work. Every frame.
    pub fn poll(&mut self) {
        self.link.poll();
        while let Some(e) = self.link.next_event() {
            self.on_event(e);
        }
        self.host_tick();
        self.migration_tick();
        self.host_flush();
        self.out.flush(self.link.as_mut(), &mut self.traffic);
        if std::mem::take(&mut self.rejected) {
            self.leave();
        }
    }

    fn emit(&mut self, e: NetEvent) {
        if let NetEvent::Desync(_) = e {
            self.desyncs_seen += 1;
        }
        self.events.push_back(e);
    }

    fn on_event(&mut self, e: Event) {
        match e {
            Event::LoggedIn { ref account, .. } => {
                if account.is_none()
                    && let Some(n) = self.cfg.auto_register.clone()
                {
                    self.link.register(&n);
                }
                self.apply_relay();
                self.emit(NetEvent::Warpzone(Box::new(e)));
            }
            Event::RoomHosted {
                ref room,
                ref code,
                ref invite,
            } => {
                self.room = room.id.clone();
                self.lobby.room = room.id.clone();
                self.lobby.code = code.clone();
                self.lobby.invite = invite.clone();
                self.emit(NetEvent::Warpzone(Box::new(e)));
            }
            Event::RoomJoined { ref room, host } => {
                self.room = room.id.clone();
                self.enter_room(host);
                self.emit(NetEvent::Warpzone(Box::new(e)));
            }
            Event::PeerJoined {
                peer,
                account,
                ref username,
            } => {
                let name = username.clone();
                self.on_peer_joined(peer, account, name);
                self.emit(NetEvent::Warpzone(Box::new(e)));
            }
            Event::PeerLeft { peer, .. } => self.on_peer_left(peer),
            Event::PeerData {
                peer,
                channel: CH_SESSION,
                data,
                ..
            } => {
                self.traffic.recv_bytes += data.len() as u64;
                let whole = self.dechunk.entry(peer).or_default().push(&data);
                if let Some(b) = whole {
                    match wire::decode::<Msg>(&b) {
                        Some(msg) => {
                            self.traffic.recv_msgs += 1;
                            self.on_msg(peer, msg);
                        }
                        None => log::warn!("baba-net: undecodable message from peer {peer}"),
                    }
                }
            }
            Event::PeerData { .. } => {}
            Event::RoomClosed { ref room } => {
                if *room == self.room && matches!(self.role, Role::Guest { .. }) {
                    self.host_lost();
                }
            }
            Event::Kicked {
                ref room, banned, ..
            } => {
                if *room == self.room {
                    self.left_room(if banned { "banned" } else { "kicked" })
                }
            }
            Event::Knock {
                ref knock, account, ..
            } => {
                let known = match &self.role {
                    Role::Host(h) => h.old_accounts.contains_key(&account),
                    _ => false,
                };
                if known {
                    let k = knock.clone();
                    self.link.answer_knock(&k, true);
                } else {
                    self.emit(NetEvent::Warpzone(Box::new(e)));
                }
            }
            Event::KnockDenied { .. } | Event::Error { .. }
                if matches!(self.role, Role::Migrating(_)) =>
            {
                if let Role::Migrating(m) = &mut self.role {
                    if let Event::Error { op, .. } = &e
                        && op == "host"
                    {
                        m.hosting = false;
                        m.candidates.retain(|c| Some(*c) != self.me);
                    }
                    m.joining = None;
                }
                self.emit(NetEvent::Warpzone(Box::new(e)));
            }
            Event::RoomList(ref rooms) if matches!(self.role, Role::Migrating(_)) => {
                let Role::Migrating(m) = &mut self.role else {
                    unreachable!()
                };
                if m.hosting || m.joining.is_some() {
                    return;
                }
                let best = rooms
                    .iter()
                    .filter(|r| r.metadata.get(META_MIGRATE) == Some(&m.old_room))
                    .filter_map(|r| {
                        let h: u8 = r.metadata.get(META_MHOST)?.parse().ok()?;
                        Some((h, r.id.clone()))
                    })
                    .min();
                if let Some((_, id)) = best {
                    m.joining = Some(id.clone());
                    self.link.join(&id, None, None);
                }
            }
            e => self.emit(NetEvent::Warpzone(Box::new(e))),
        }
    }

    fn enter_room(&mut self, host: bool) {
        self.dechunk.clear();
        self.out.clear();
        let migration = match std::mem::replace(&mut self.role, Role::None) {
            Role::Migrating(m) => Some(m),
            _ => None,
        };
        if host {
            self.become_host(migration);
        } else {
            self.resume_from = migration.map(|m| m.old_room);
            if self.resume_from.is_none() {
                self.me = None;
                self.session = None;
                self.lobby = LobbyState::default();
            }
            self.lobby.room = self.room.clone();
            self.role = Role::Guest {
                host_peer: None,
                welcomed: false,
            };
        }
    }

    fn my_view(&self, machine: MachineId) -> MachineView {
        MachineView {
            machine,
            account: self.link.account_id(),
            name: self.link.username().unwrap_or_else(|| "host".into()),
            players: self.my_players.clone(),
            admitted: Vec::new(),
            spectator: self.spectate || self.my_players.is_empty(),
            ready: self.ready,
            synced: self.session.is_some(),
        }
    }

    fn become_host(&mut self, migration: Option<Box<Migration>>) {
        let mut h = HostState::default();
        let me = match &migration {
            Some(_) => self.me.unwrap_or_default(),
            None => {
                self.session = None;
                self.lobby = LobbyState {
                    room: self.lobby.room.clone(),
                    code: self.lobby.code.clone(),
                    invite: self.lobby.invite.clone(),
                    ..Default::default()
                };
                MachineId(0)
            }
        };
        self.me = Some(me);
        h.machines.insert(
            me,
            HostMachine {
                peer: None,
                view: self.my_view(me),
                spectate: self.spectate,
                sent: BTreeMap::new(),
            },
        );
        h.next_machine = me.0 + 1;
        if let Some(m) = &migration {
            self.migrations += 1;
            let mut top = me.0;
            for v in &m.roster {
                top = top.max(v.machine.0);
                if v.machine != me && v.account != 0 {
                    h.old_accounts.insert(v.account, v.machine);
                }
            }
            if let Some(s) = &self.session {
                h.next_session = s.start.session;
                for p in s.players.values() {
                    top = top.max(p.info.machine.0);
                }
                h.reconcile = Some(Reconcile {
                    expected: m
                        .roster
                        .iter()
                        .map(|v| v.machine)
                        .filter(|&id| id != me && id != self.lobby.host)
                        .collect(),
                    since: Instant::now(),
                    wants: BTreeMap::new(),
                    queued: Vec::new(),
                });
            }
            h.next_machine = top.saturating_add(1);
            h.old_room = Some(m.old_room.clone());
            self.lobby.host = me;
            self.lobby.phase = match &self.session {
                Some(s) => Phase::Session(s.start.session),
                None => Phase::Lobby,
            };
        }
        self.role = Role::Host(Box::new(h));
        if migration.is_some() {
            // Our own unsequenced turns wait for the reconcile like everyone's.
            if let Some(s) = self.session.as_mut() {
                let pending = std::mem::take(&mut s.pending);
                if let Role::Host(h) = &mut self.role
                    && let Some(rc) = h.reconcile.as_mut()
                {
                    rc.queued.extend(pending.into_iter().map(|p| (me, p)));
                }
            }
            self.emit(NetEvent::HostMigrated {
                host: me,
                room: self.room.clone(),
            });
            self.host_check_reconcile();
        } else {
            self.emit(NetEvent::Welcome { machine: me });
        }
        self.host_broadcast_lobby();
    }

    fn left_room(&mut self, reason: &str) {
        if matches!(self.role, Role::None) {
            return;
        }
        if self.session.is_some() {
            self.emit(NetEvent::SessionAborted {
                reason: reason.into(),
            });
        }
        self.session = None;
        self.role = Role::None;
        self.me = None;
        self.room.clear();
        self.lobby = LobbyState::default();
        self.dechunk.clear();
        self.out.clear();
        self.resume_from = None;
        self.emit(NetEvent::RoomLeft {
            reason: reason.into(),
        });
    }

    fn on_peer_joined(&mut self, peer: PeerId, account: i64, name: String) {
        match &mut self.role {
            Role::Host(h) => {
                h.unknown.insert(peer, (account, name));
            }
            Role::Guest { host_peer, .. } => {
                *host_peer = Some(peer);
                self.hello_lengths = self
                    .session
                    .as_ref()
                    .map(|s| s.lengths().into_iter().collect())
                    .unwrap_or_default();
                let resume = self.resume_from.clone().map(|old_room| Resume {
                    old_room,
                    machine: self.me.unwrap_or_default(),
                    session: self.session.as_ref().map(|s| s.start.session),
                    lengths: self
                        .session
                        .as_ref()
                        .map(|s| s.lengths())
                        .unwrap_or_default(),
                });
                let hello = Msg::Hello {
                    protocol: PROTOCOL,
                    build: self.cfg.build.clone(),
                    content: self.cfg.content.clone(),
                    players: self.my_players.clone(),
                    spectate: self.spectate,
                    ready: self.ready,
                    resume,
                };
                self.send_peer(peer, &hello);
            }
            _ => {}
        }
    }

    fn on_peer_left(&mut self, peer: PeerId) {
        self.dechunk.remove(&peer);
        self.out.drop_peer(peer);
        match &mut self.role {
            Role::Host(h) => {
                h.unknown.remove(&peer);
                let Some(id) = h.peers.remove(&peer) else {
                    return;
                };
                h.machines.remove(&id);
                if let Some((w, _)) = h.stop.as_mut() {
                    w.remove(&id);
                }
                if let Some(e) = h.end.as_mut() {
                    e.waiting.remove(&id);
                }
                if let Some(rc) = h.reconcile.as_mut() {
                    rc.wants.retain(|(m, _), _| *m != id);
                    rc.expected.remove(&id);
                }
                self.emit(NetEvent::MachineLeft { machine: id });
                self.host_admit();
                self.host_check_stop();
                self.host_check_end();
                self.host_check_reconcile();
                self.host_flush();
                self.host_broadcast_lobby();
            }
            Role::Guest { host_peer, .. } if *host_peer == Some(peer) => self.host_lost(),
            _ => {}
        }
    }

    // ------------------------------------------------------------ messages

    fn on_msg(&mut self, peer: PeerId, msg: Msg) {
        match &self.role {
            Role::Host(_) => self.host_on_msg(peer, msg),
            Role::Guest { host_peer, .. } if *host_peer == Some(peer) => self.guest_on_msg(msg),
            _ => {}
        }
    }

    fn guest_on_msg(&mut self, msg: Msg) {
        let me = self.me;
        match msg {
            Msg::Welcome { machine, host } => {
                self.lobby.host = host;
                let resumed = self.resume_from.take().is_some() && me == Some(machine);
                if !resumed {
                    // A fresh seat: anything we had belongs to the old one.
                    self.session = None;
                }
                self.me = Some(machine);
                if let Role::Guest { welcomed, .. } = &mut self.role {
                    *welcomed = true;
                }
                if resumed {
                    self.migrations += 1;
                    let session = self.session.as_ref().map(|s| s.start.session);
                    let pending = self
                        .session
                        .as_ref()
                        .map(|s| s.pending.clone())
                        .unwrap_or_default();
                    if let Some(session) = session {
                        // Own turns made since the Hello (not sent: we had no
                        // seat yet) follow at once; the host asks (`Want`) for
                        // anything older it lacks.
                        let reported = self.hello_lengths.clone();
                        let backlog: Vec<Msg> = self
                            .session
                            .as_ref()
                            .unwrap()
                            .logs
                            .iter()
                            .filter(|(id, _)| match id {
                                StreamId::Player(p) => self
                                    .session
                                    .as_ref()
                                    .unwrap()
                                    .players
                                    .get(p)
                                    .is_some_and(|ps| Some(ps.info.machine) == me),
                                StreamId::Shared => false,
                            })
                            .filter_map(|(id, log)| {
                                let from = reported.get(id).copied().unwrap_or(0);
                                (log.len() > from as usize).then(|| Msg::Append {
                                    session,
                                    stream: *id,
                                    first: from,
                                    entries: log[from as usize..].to_vec(),
                                })
                            })
                            .collect();
                        for m in backlog {
                            self.send_host(&m);
                        }
                        for p in pending {
                            self.send_host(&Msg::Submit {
                                session,
                                player: p.player,
                                seq: p.seq,
                                kind: p.kind,
                                data: p.data,
                            });
                        }
                    }
                    let room = self.room.clone();
                    self.emit(NetEvent::HostMigrated {
                        host: self.lobby.host,
                        room,
                    });
                } else {
                    self.emit(NetEvent::Welcome { machine });
                }
            }
            Msg::Reject { reason } => {
                self.emit(NetEvent::Rejected { reason });
                self.rejected = true;
            }
            Msg::Lobby(state) => {
                self.lobby = state;
                self.emit(NetEvent::LobbyChanged);
            }
            Msg::Start(start) => {
                self.session = Some(Session::new(start.clone()));
                self.emit(NetEvent::SessionStarted(start));
            }
            Msg::Append {
                session,
                stream,
                first,
                entries,
            } => {
                if let Some(s) = self.session.as_mut()
                    && s.start.session == session
                    && s.merge(stream, first, entries, me).is_none()
                {
                    log::warn!(
                        "baba-net: gap in {stream}: have {}, got {first}",
                        s.len(stream)
                    );
                }
            }
            Msg::Want { stream, from } => {
                let Some(s) = self.session.as_ref() else {
                    return;
                };
                let session = s.start.session;
                let log = s.logs.get(&stream).map(|l| l.as_slice()).unwrap_or(&[]);
                let msgs: Vec<Msg> = log
                    .get(from as usize..)
                    .unwrap_or(&[])
                    .chunks(APPEND_BATCH)
                    .enumerate()
                    .map(|(i, c)| Msg::Append {
                        session,
                        stream,
                        first: from + (i * APPEND_BATCH) as u32,
                        entries: c.to_vec(),
                    })
                    .collect();
                if msgs.is_empty() {
                    self.send_host(&Msg::Append {
                        session,
                        stream,
                        first: from,
                        entries: Vec::new(),
                    });
                }
                for m in msgs {
                    self.send_host(&m);
                }
            }
            Msg::Stop { session } => {
                if let Some(s) = self.session.as_mut()
                    && s.start.session == session
                {
                    s.stopped = true;
                    self.send_host(&Msg::Stopped { session });
                }
            }
            Msg::End { session, lengths } => {
                if let Some(s) = self.session.as_mut()
                    && s.start.session == session
                {
                    s.stopped = true;
                    s.end_lengths = Some(lengths);
                }
            }
            Msg::Ended(report) => {
                if self
                    .session
                    .as_ref()
                    .is_some_and(|s| s.start.session == report.session)
                {
                    self.session = None;
                }
                self.emit(NetEvent::SessionEnded(report));
            }
            Msg::Abort { session, reason } => {
                if self
                    .session
                    .as_ref()
                    .is_some_and(|s| s.start.session == session)
                {
                    self.session = None;
                    self.emit(NetEvent::SessionAborted { reason });
                }
            }
            Msg::Desync(report) => self.emit(NetEvent::Desync(report)),
            Msg::App { from, data, .. } => self.emit(NetEvent::App { from, data }),
            _ => {}
        }
    }

    fn host_on_msg(&mut self, peer: PeerId, msg: Msg) {
        let Role::Host(h) = &mut self.role else {
            return;
        };
        let Some(&from) = h.peers.get(&peer) else {
            if let Msg::Hello {
                protocol,
                build,
                content,
                players,
                spectate,
                ready,
                resume,
            } = msg
            {
                self.host_hello(
                    peer, protocol, build, content, players, spectate, ready, resume,
                );
            }
            return;
        };
        let session = self.session.as_ref().map(|s| s.start.session);
        match msg {
            Msg::SetPlayers(v) => {
                if let Some(m) = h.machines.get_mut(&from) {
                    m.view.players = sanitize_players(v);
                    // A machine that had no players and now has some
                    // plays (unless it asked to watch).
                    m.view.spectator = m.spectate || m.view.players.is_empty();
                }
                self.host_admit();
                self.host_flush();
                self.host_broadcast_lobby();
            }
            Msg::SetReady(r) => {
                if let Some(m) = h.machines.get_mut(&from) {
                    m.view.ready = r;
                }
                self.host_broadcast_lobby();
            }
            Msg::SetSpectate(sp) => {
                if let Some(m) = h.machines.get_mut(&from) {
                    m.spectate = sp;
                    m.view.spectator = sp || m.view.players.is_empty();
                }
                self.host_admit();
                self.host_flush();
                self.host_broadcast_lobby();
            }
            Msg::Submit {
                session: sid,
                player,
                seq,
                kind,
                data,
            } => {
                if Some(sid) != session
                    || !matches!(kind, EntryKind::Input | EntryKind::Event)
                    || data.len() > MAX_EVENT
                    || (kind == EntryKind::Input && data.len() > MAX_INPUT)
                {
                    return;
                }
                let s = self.session.as_ref().unwrap();
                let owned = s
                    .players
                    .get(&player)
                    .is_some_and(|p| p.info.machine == from);
                if !owned {
                    return;
                }
                let p = Pending {
                    player,
                    seq,
                    kind,
                    data,
                };
                if let Some(rc) = h.reconcile.as_mut() {
                    rc.queued.push((from, p));
                    return;
                }
                if h.end.is_some() {
                    return;
                }
                self.host_sequence(p);
            }
            Msg::Append {
                session: sid,
                stream,
                first,
                entries,
            } => {
                if Some(sid) != session {
                    return;
                }
                let s = self.session.as_mut().unwrap();
                let want = h
                    .reconcile
                    .as_ref()
                    .is_some_and(|rc| rc.wants.contains_key(&(from, stream)));
                let own = match stream {
                    StreamId::Player(p) => s
                        .players
                        .get(&p)
                        .is_some_and(|ps| ps.active && ps.info.machine == from),
                    StreamId::Shared => false,
                };
                if !(own || want) || h.end.is_some() {
                    self.host_check_reconcile();
                    return;
                }
                if !want
                    && entries.iter().any(|e| {
                        !matches!(e.kind, EntryKind::Input | EntryKind::Event)
                            || e.data.len() > MAX_EVENT
                            || (e.kind == EntryKind::Input && e.data.len() > MAX_INPUT)
                    })
                {
                    return;
                }
                let n = entries.len() as u32;
                let me = self.me;
                if s.merge(stream, first, entries, me).is_none() {
                    // We missed some of its turns (it made them while it had
                    // no seat): ask for everything from where we are.
                    let have = s.len(stream);
                    log::info!("baba-net: gap from {from} in {stream}: have {have}, got {first}");
                    if !want && let Some(peer) = h.machines.get(&from).and_then(|m| m.peer) {
                        self.send_peer(peer, &Msg::Want { stream, from: have });
                    }
                    return;
                }
                if let Some(m) = h.machines.get_mut(&from) {
                    let c = m.sent.entry(stream).or_insert(0);
                    *c = (*c).max(first + n);
                }
                if want {
                    let len = s.len(stream);
                    if let Some(rc) = h.reconcile.as_mut()
                        && rc.wants.get(&(from, stream)).is_some_and(|t| len >= *t)
                    {
                        rc.wants.remove(&(from, stream));
                    }
                    self.host_recount();
                    self.host_check_reconcile();
                }
            }
            Msg::Hash {
                session: sid,
                stream,
                index,
                hash,
            } => {
                if Some(sid) == session {
                    self.host_hash(from, stream, index, hash);
                }
            }
            Msg::Stopped { session: sid } => {
                if Some(sid) == session
                    && let Some((w, _)) = h.stop.as_mut()
                {
                    w.remove(&from);
                    self.host_check_stop();
                }
            }
            Msg::Final {
                session: sid,
                hashes,
            } => {
                if Some(sid) == session {
                    self.host_final(from, hashes);
                }
            }
            Msg::App { to, data, .. } => self.host_route_app(Msg::App { from, to, data }),
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn host_hello(
        &mut self,
        peer: PeerId,
        protocol: u16,
        build: String,
        content: String,
        players: Vec<String>,
        spectate: bool,
        ready: bool,
        resume: Option<Resume>,
    ) {
        let Role::Host(h) = &mut self.role else {
            return;
        };
        let (account, name) = h.unknown.remove(&peer).unwrap_or((0, "?".into()));
        let reject = if protocol != PROTOCOL || build != self.cfg.build {
            Some(format!(
                "{REJECT_VERSION}: host {} (protocol {PROTOCOL}), you {build} (protocol {protocol})",
                self.cfg.build
            ))
        } else if content != self.cfg.content {
            Some(format!(
                "{REJECT_CONTENT}: host {}, you {content}",
                self.cfg.content
            ))
        } else if h.machines.len() >= MAX_MACHINES {
            Some("the room is full".into())
        } else {
            None
        };
        if let Some(reason) = reject {
            if account != 0 {
                h.kicks
                    .push((account, Instant::now() + Duration::from_millis(500)));
            }
            self.send_peer(peer, &Msg::Reject { reason });
            return;
        }
        let session = self.session.as_ref().map(|s| s.start.session);
        let resumed = match &resume {
            Some(r) => {
                h.old_room.as_deref() == Some(r.old_room.as_str())
                    && h.old_accounts.get(&account) == Some(&r.machine)
                    && !h.machines.contains_key(&r.machine)
                    && (session.is_none() || (h.reconcile.is_some() && r.session == session))
            }
            None => false,
        };
        let id = if resumed {
            resume.as_ref().unwrap().machine
        } else {
            let id = MachineId(h.next_machine);
            h.next_machine = h.next_machine.saturating_add(1);
            id
        };
        let players = sanitize_players(players);
        let in_session = session.is_some() && h.stop.is_none() && h.end.is_none();
        let view = MachineView {
            machine: id,
            account,
            name: name.clone(),
            spectator: spectate || players.is_empty(),
            players,
            admitted: Vec::new(),
            ready,
            synced: in_session,
        };
        let mut sent = BTreeMap::new();
        let mut wants = Vec::new();
        if resumed && in_session {
            let s = self.session.as_ref().unwrap();
            for (stream, len) in &resume.as_ref().unwrap().lengths {
                sent.insert(*stream, *len);
                if *len > s.len(*stream) {
                    wants.push((*stream, s.len(*stream)));
                }
            }
        }
        h.machines.insert(
            id,
            HostMachine {
                peer: Some(peer),
                view,
                spectate,
                sent,
            },
        );
        h.peers.insert(peer, id);
        if let Some(rc) = h.reconcile.as_mut() {
            rc.expected.remove(&id);
            for (stream, _) in &wants {
                let target = resume
                    .as_ref()
                    .and_then(|r| r.lengths.iter().find(|(s, _)| s == stream))
                    .map_or(0, |(_, l)| *l);
                rc.wants.insert((id, *stream), target);
            }
        }
        let host = self.me.unwrap_or_default();
        self.send_peer(peer, &Msg::Welcome { machine: id, host });
        self.host_broadcast_lobby();
        if in_session {
            let s = self.session.as_ref().unwrap();
            if resumed {
                for (stream, from) in wants {
                    self.send_peer(peer, &Msg::Want { stream, from });
                }
            } else {
                let start = s.start.clone();
                self.send_peer(peer, &Msg::Start(start));
            }
        }
        self.emit(NetEvent::MachineJoined { machine: id, name });
        self.host_check_reconcile();
        self.host_admit();
        self.host_flush();
        self.host_broadcast_lobby();
    }

    /// Append a submission to the shared stream (duplicates dropped).
    fn host_sequence(&mut self, p: Pending) {
        let Role::Host(h) = &mut self.role else {
            return;
        };
        let Some(s) = self.session.as_mut() else {
            return;
        };
        if !s.players.get(&p.player).is_some_and(|x| x.active) {
            return;
        }
        let next = h.seq_seen.entry(p.player).or_insert(0);
        if p.seq < *next {
            return;
        }
        *next = p.seq + 1;
        let me = self.me;
        s.push(
            StreamId::Shared,
            Entry {
                kind: p.kind,
                player: p.player,
                seq: p.seq,
                sync: 0,
                data: p.data,
            },
            me,
        );
    }

    /// After merging logs from others: duplicate filter and player ids.
    fn host_recount(&mut self) {
        let Role::Host(h) = &mut self.role else {
            return;
        };
        let Some(s) = self.session.as_ref() else {
            return;
        };
        for e in &s.logs[&StreamId::Shared] {
            match e.kind {
                EntryKind::Input | EntryKind::Event => {
                    let c = h.seq_seen.entry(e.player).or_insert(0);
                    *c = (*c).max(e.seq + 1);
                }
                EntryKind::Joined => {
                    h.next_player = h.next_player.max(e.player.0.saturating_add(1));
                }
                EntryKind::Left => {}
            }
        }
    }

    /// Admit and remove players to match the machines' announcements.
    fn host_admit(&mut self) {
        let Role::Host(h) = &mut self.role else {
            return;
        };
        let Some(s) = self.session.as_mut() else {
            return;
        };
        if s.stopped || h.reconcile.is_some() || h.end.is_some() {
            return;
        }
        let me = self.me;
        let mode = s.start.mode;
        // Removals.
        let gone: Vec<PlayerId> = s
            .players
            .values()
            .filter(|p| p.active)
            .filter(|p| match h.machines.get(&p.info.machine) {
                None => true,
                Some(m) => {
                    !m.view.synced
                        || m.view.spectator
                        || p.info.local_index as usize >= m.view.players.len()
                }
            })
            .map(|p| p.info.player)
            .collect();
        for p in gone {
            let data = if mode.per_player() {
                s.len(StreamId::Player(p)).to_le_bytes().to_vec()
            } else {
                Vec::new()
            };
            s.push(
                StreamId::Shared,
                Entry {
                    kind: EntryKind::Left,
                    player: p,
                    seq: 0,
                    sync: 0,
                    data,
                },
                me,
            );
        }
        // Admissions, machine by machine.
        let mut joins = Vec::new();
        let mut active = s.active_players();
        for (id, m) in &h.machines {
            if !m.view.synced || m.view.spectator {
                continue;
            }
            for (i, name) in m.view.players.iter().enumerate() {
                if active >= MAX_PLAYERS {
                    break;
                }
                if s.local_player(*id, i).is_none() {
                    joins.push(PlayerInfo {
                        player: PlayerId(0),
                        machine: *id,
                        local_index: i as u8,
                        name: name.clone(),
                    });
                    active += 1;
                }
            }
        }
        for mut info in joins {
            if h.next_player == u8::MAX {
                break;
            }
            info.player = PlayerId(h.next_player);
            h.next_player += 1;
            s.push(
                StreamId::Shared,
                Entry {
                    kind: EntryKind::Joined,
                    player: info.player,
                    seq: 0,
                    sync: 0,
                    data: wire::encode(&info),
                },
                me,
            );
        }
        // The lobby shows who got in.
        for (id, m) in h.machines.iter_mut() {
            m.view.admitted = (0..m.view.players.len())
                .map(|i| s.local_player(*id, i))
                .collect();
        }
    }

    /// Send every machine the entries it does not have yet.
    fn host_flush(&mut self) {
        let Role::Host(h) = &mut self.role else {
            return;
        };
        let Some(s) = self.session.as_ref() else {
            return;
        };
        let session = s.start.session;
        let mut out: Vec<(PeerId, Msg)> = Vec::new();
        for m in h.machines.values_mut() {
            let Some(peer) = m.peer else { continue };
            if !m.view.synced {
                continue;
            }
            for (id, log) in &s.logs {
                let sent = m.sent.entry(*id).or_insert(0);
                while (*sent as usize) < log.len() {
                    let a = *sent as usize;
                    let b = (a + APPEND_BATCH).min(log.len());
                    out.push((
                        peer,
                        Msg::Append {
                            session,
                            stream: *id,
                            first: a as u32,
                            entries: log[a..b].to_vec(),
                        },
                    ));
                    *sent = b as u32;
                }
            }
        }
        for (p, m) in out {
            self.send_peer(p, &m);
        }
    }

    fn host_hash(&mut self, from: MachineId, stream: StreamId, index: u32, hash: u64) {
        let Role::Host(h) = &mut self.role else {
            return;
        };
        let Some(s) = self.session.as_ref() else {
            return;
        };
        let key = (stream, index);
        let me = self.me.unwrap_or_default();
        if !h.pending_hash.contains_key(&key)
            && from != me
            && let Some(&own) = s.own_hashes.get(&key)
        {
            // Evaluated already (or a late joiner replaying): compare with ours.
            if own != hash {
                let r = DesyncReport {
                    session: s.start.session,
                    stream,
                    index,
                    majority: own,
                    hashes: vec![(me, own), (from, hash)],
                };
                self.host_desync(r);
            }
            return;
        }
        let entry = h
            .pending_hash
            .entry(key)
            .or_insert_with(|| (BTreeMap::new(), Instant::now()));
        entry.0.insert(from, hash);
        let complete = h
            .machines
            .iter()
            .filter(|(_, m)| m.view.synced)
            .all(|(id, _)| entry.0.contains_key(id));
        if complete {
            let (reports, _) = h.pending_hash.remove(&key).unwrap();
            self.host_evaluate(stream, index, reports);
        }
    }

    fn host_evaluate(&mut self, stream: StreamId, index: u32, reports: BTreeMap<MachineId, u64>) {
        if reports.len() < 2 {
            return;
        }
        let me = self.me.unwrap_or_default();
        let mut counts: BTreeMap<u64, usize> = BTreeMap::new();
        for h in reports.values() {
            *counts.entry(*h).or_default() += 1;
        }
        if counts.len() == 1 {
            return;
        }
        let top = *counts.values().max().unwrap();
        let tied: Vec<u64> = counts
            .iter()
            .filter(|(_, c)| **c == top)
            .map(|(h, _)| *h)
            .collect();
        let majority = match reports.get(&me) {
            Some(own) if tied.contains(own) => *own,
            _ => tied[0],
        };
        let session = self.session.as_ref().map_or(0, |s| s.start.session);
        self.host_desync(DesyncReport {
            session,
            stream,
            index,
            majority,
            hashes: reports.into_iter().collect(),
        });
    }

    fn host_desync(&mut self, r: DesyncReport) {
        let Role::Host(h) = &mut self.role else {
            return;
        };
        let mut new = false;
        for m in r.desynced() {
            new |= h.desync_told.insert((r.stream, m));
        }
        if !new {
            return;
        }
        if h.desyncs.len() < MAX_DESYNC_REPORTS {
            h.desyncs.push(r.clone());
        }
        self.host_broadcast(&Msg::Desync(r.clone()), None);
        let stream = r.stream;
        let index = r.index;
        self.emit(NetEvent::Desync(r));
        if self.cfg.desync_policy == DesyncPolicy::Abort {
            let _ = self.abort_session(&format!("desync in {stream} at {index}"));
        }
    }

    fn host_check_stop(&mut self) {
        let Role::Host(h) = &mut self.role else {
            return;
        };
        let Some((waiting, since)) = &h.stop else {
            return;
        };
        if !waiting.is_empty() && since.elapsed() < STOP_TIMEOUT {
            return;
        }
        h.stop = None;
        self.host_flush();
        let Role::Host(h) = &mut self.role else {
            return;
        };
        let Some(s) = self.session.as_mut() else {
            return;
        };
        let lengths = s.lengths();
        s.end_lengths = Some(lengths.clone());
        let session = s.start.session;
        let mut waiting = BTreeSet::new();
        let mut peers = Vec::new();
        for (id, m) in &h.machines {
            if m.view.synced {
                waiting.insert(*id);
                if let Some(p) = m.peer {
                    peers.push(p);
                }
            }
        }
        h.end = Some(EndCollect {
            lengths: lengths.clone(),
            finals: BTreeMap::new(),
            waiting,
            since: Instant::now(),
        });
        for p in peers {
            self.send_peer(
                p,
                &Msg::End {
                    session,
                    lengths: lengths.clone(),
                },
            );
        }
    }

    fn host_final(&mut self, from: MachineId, hashes: Vec<(StreamId, u64)>) {
        let Role::Host(h) = &mut self.role else {
            return;
        };
        let Some(e) = h.end.as_mut() else {
            return;
        };
        e.waiting.remove(&from);
        e.finals.insert(from, hashes);
        self.host_check_end();
    }

    fn host_check_end(&mut self) {
        let Role::Host(h) = &mut self.role else {
            return;
        };
        let Some(e) = &h.end else {
            return;
        };
        if !e.waiting.is_empty() && e.since.elapsed() < END_TIMEOUT {
            return;
        }
        let e = h.end.take().unwrap();
        // Hash checks still open: evaluate what we have.
        let pending: Vec<_> = std::mem::take(&mut h.pending_hash).into_iter().collect();
        for ((stream, index), (reports, _)) in pending {
            self.host_evaluate(stream, index, reports);
        }
        let Role::Host(h) = &mut self.role else {
            return;
        };
        let first = e.finals.values().next();
        let agreed = !e.finals.is_empty() && e.finals.values().all(|f| Some(f) == first);
        let report = EndReport {
            session: self.session.as_ref().map_or(0, |s| s.start.session),
            lengths: e.lengths,
            hashes: e.finals.into_iter().collect(),
            agreed,
            desyncs: h.desyncs.clone(),
        };
        self.host_broadcast(&Msg::Ended(report.clone()), None);
        self.finish_session_locally();
        self.emit(NetEvent::SessionEnded(report));
        self.host_broadcast_lobby();
    }

    fn finish_session_locally(&mut self) {
        self.session = None;
        self.lobby.phase = Phase::Lobby;
        if let Role::Host(h) = &mut self.role {
            h.stop = None;
            h.end = None;
            h.reconcile = None;
            h.pending_hash.clear();
            for m in h.machines.values_mut() {
                m.view.synced = false;
                m.view.admitted.clear();
                m.sent.clear();
            }
        }
    }

    fn host_check_reconcile(&mut self) {
        let Role::Host(h) = &mut self.role else {
            return;
        };
        let Some(rc) = &h.reconcile else {
            return;
        };
        let waited = rc.since.elapsed() >= RECONCILE_TIMEOUT;
        if !(rc.expected.is_empty() || waited) || !(rc.wants.is_empty() || waited) {
            return;
        }
        let rc = h.reconcile.take().unwrap();
        self.host_recount();
        // Players of machines that did not come back leave now (admit), then
        // the turns that waited are sequenced (duplicates dropped).
        self.host_admit();
        let mut queued = rc.queued;
        queued.sort_by_key(|(_, p)| (p.player, p.seq));
        for (_, p) in queued {
            self.host_sequence(p);
        }
        self.host_flush();
        self.host_broadcast_lobby();
    }

    fn host_tick(&mut self) {
        let Role::Host(h) = &mut self.role else {
            return;
        };
        let now = Instant::now();
        let due: Vec<i64> = h
            .kicks
            .iter()
            .filter(|(_, at)| *at <= now)
            .map(|(a, _)| *a)
            .collect();
        h.kicks.retain(|(_, at)| *at > now);
        let stale: Vec<(StreamId, u32)> = h
            .pending_hash
            .iter()
            .filter(|(_, (_, born))| born.elapsed() > HASH_TIMEOUT)
            .map(|(k, _)| *k)
            .collect();
        for acc in due {
            self.link.kick(acc, false);
        }
        for k in stale {
            let Role::Host(h) = &mut self.role else {
                return;
            };
            if let Some((reports, _)) = h.pending_hash.remove(&k) {
                self.host_evaluate(k.0, k.1, reports);
            }
        }
        self.host_check_stop();
        self.host_check_end();
        self.host_check_reconcile();
    }

    fn host_route_app(&mut self, msg: Msg) {
        let Msg::App { from, to, data } = msg else {
            return;
        };
        let me = self.me;
        if (to.is_none() || to == me) && Some(from) != me {
            self.emit(NetEvent::App {
                from,
                data: data.clone(),
            });
        }
        if to == me {
            return;
        }
        let Role::Host(h) = &self.role else { return };
        let peers: Vec<PeerId> = h
            .machines
            .iter()
            .filter(|(id, _)| **id != from && (to.is_none() || to == Some(**id)))
            .filter_map(|(_, m)| m.peer)
            .collect();
        let msg = Msg::App { from, to, data };
        for p in peers {
            self.send_peer(p, &msg);
        }
    }

    fn host_broadcast(&mut self, msg: &Msg, except: Option<MachineId>) {
        let Role::Host(h) = &self.role else { return };
        let peers: Vec<PeerId> = h
            .machines
            .iter()
            .filter(|(id, _)| Some(**id) != except)
            .filter_map(|(_, m)| m.peer)
            .collect();
        for p in peers {
            self.send_peer(p, msg);
        }
    }

    fn host_broadcast_lobby(&mut self) {
        let Role::Host(h) = &mut self.role else {
            return;
        };
        self.lobby.machines = h.machines.values().map(|m| m.view.clone()).collect();
        self.lobby.host = self.me.unwrap_or_default();
        self.lobby.room = self.room.clone();
        if self.lobby.phase == Phase::Migrating {
            self.lobby.phase = Phase::Lobby;
        }
        if let Some(s) = &self.session {
            self.lobby.phase = Phase::Session(s.start.session);
        }
        // Room browser labels (the descriptor's `display` keys).
        let mut meta = BTreeMap::new();
        let players: usize = match &self.session {
            Some(s) => s.active_players(),
            None => self.lobby.player_count().min(MAX_PLAYERS),
        };
        meta.insert("players".to_string(), format!("{players}/{MAX_PLAYERS}"));
        meta.insert("mode".to_string(), self.lobby.mode.label().to_string());
        for (k, v) in &self.room_labels {
            meta.entry(k.clone()).or_insert_with(|| v.clone());
        }
        if let Some(old) = &h.old_room {
            meta.insert(META_MIGRATE.to_string(), old.clone());
            meta.insert(
                META_MHOST.to_string(),
                self.me.unwrap_or_default().0.to_string(),
            );
        }
        let meta_changed = meta != h.metadata;
        if meta_changed {
            h.metadata = meta.clone();
        }
        let msg = Msg::Lobby(self.lobby.clone());
        self.host_broadcast(&msg, None);
        if meta_changed {
            self.link.set_metadata(meta);
        }
        self.emit(NetEvent::LobbyChanged);
    }

    // ------------------------------------------------------------ migration

    fn host_lost(&mut self) {
        let welcomed = matches!(self.role, Role::Guest { welcomed: true, .. });
        if !self.cfg.host_migration || !welcomed || self.me.is_none() {
            self.left_room("host left");
            return;
        }
        let me = self.me.unwrap();
        let old_host = self.lobby.host;
        let mut candidates: Vec<MachineId> = self
            .lobby
            .machines
            .iter()
            .map(|m| m.machine)
            .filter(|&m| m != old_host)
            .collect();
        if !candidates.contains(&me) {
            candidates.push(me);
        }
        candidates.sort();
        let successor = candidates[0];
        let now = Instant::now();
        self.role = Role::Migrating(Box::new(Migration {
            old_room: self.room.clone(),
            candidates,
            roster: self.lobby.machines.clone(),
            since: now,
            cand_since: now,
            last_list: None,
            hosting: false,
            joining: None,
        }));
        self.dechunk.clear();
        self.out.clear();
        self.lobby.phase = Phase::Migrating;
        self.emit(NetEvent::HostLost { successor });
        self.emit(NetEvent::LobbyChanged);
        self.migration_tick();
    }

    fn migration_tick(&mut self) {
        let me = self.me;
        let Role::Migrating(m) = &mut self.role else {
            return;
        };
        if m.since.elapsed() > self.cfg.migration_timeout {
            self.left_room("host left (regrouping failed)");
            return;
        }
        if m.cand_since.elapsed() > SUCCESSOR_TIMEOUT
            && !m.hosting
            && m.joining.is_none()
            && m.candidates.len() > 1
        {
            m.candidates.remove(0);
            m.cand_since = Instant::now();
        }
        if let Some(me) = me
            && m.candidates.first() == Some(&me)
        {
            if !m.hosting {
                m.hosting = true;
                let mut metadata = BTreeMap::new();
                metadata.insert(META_MIGRATE.to_string(), m.old_room.clone());
                metadata.insert(META_MHOST.to_string(), me.0.to_string());
                let opts = HostOptions {
                    max_players: MAX_MACHINES as u32,
                    visibility: Visibility::Public,
                    entry: RoomEntry::Approve,
                    preset: Some("coop".into()),
                    metadata,
                    ..Default::default()
                };
                self.apply_relay();
                self.link.host(opts);
            }
            return;
        }
        if m.hosting || m.joining.is_some() {
            return;
        }
        if m.last_list.is_none_or(|t| t.elapsed() >= LIST_EVERY) {
            m.last_list = Some(Instant::now());
            self.link.list_rooms(false);
        }
    }

    // ------------------------------------------------------------ sending

    fn send_peer(&mut self, peer: PeerId, msg: &Msg) {
        let b = wire::encode(msg);
        self.traffic.sent_msgs += 1;
        for c in wire::chunk(&b) {
            self.out
                .submit(self.link.as_mut(), &mut self.traffic, peer, CH_SESSION, c);
        }
    }

    fn send_host(&mut self, msg: &Msg) {
        if let Role::Guest {
            host_peer: Some(p), ..
        } = self.role
        {
            self.send_peer(p, msg);
        }
    }
}
