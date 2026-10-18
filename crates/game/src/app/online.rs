//! Online play (M7) and its glue into [`App`]: the online menu (host a
//! public or private room, join by code, the room browser), launcher
//! arguments and invite links, the room lobby (this machine's players
//! from the join screen, the other machines, mode / save / world chosen
//! by the host, ready, start) and the running session (a
//! [`Multi`] with a [`NetSide`], replicated through
//! [`crate::session::online`]): connection status, regrouping after the
//! host left, desyncs (resync by replay, else leave with a message),
//! the end report. `docs/net.md`, `docs/game-flow.md` "Online".

use std::collections::{BTreeMap, BTreeSet};

use baba_net::warpzone_core::client::Event as WzEvent;
use baba_net::warpzone_core::protocol::api::RoomInfo;
use baba_net::{
    DesyncReport, NetConfig, NetEvent, NetSession, PlayerId, REJECT_CONTENT, REJECT_VERSION,
    SessionStart, StreamId,
};

use super::{App, Mode};
use crate::input::{Cmd, Input};
use crate::join::{Lobby, Players, Source};
use crate::menu::{Act, Item, Menu, MenuId, Style};
use crate::session::multi::{Cx, Multi};
use crate::session::online::{
    NetGame, NetSide, Rep, SessionSettings, build_id, content_hash, init_chunks, mp_mode, net_mode,
};
use crate::session::saves::{self, ExtraSlots};
use crate::session::{MpMode, Turn};
use crate::ui::{Align, Ui};
use crate::world::{WorldInfo, list_packs};

/// Online menus (inside [`MenuId::Online`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OnMenu {
    Main,
    Code {
        text: String,
    },
    Rooms,
    /// Connecting / joining (Back cancels).
    Wait(String),
    /// An error or a result.
    Message(String),
}

/// Online actions (inside [`Act::Online`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OnAct {
    /// Main menu → Online.
    Open,
    HostPublic,
    HostPrivate,
    EnterCode,
    SubmitCode,
    Browse,
    Refresh,
    JoinRoom(String),
    Cancel,
}

/// What to do once signed in to Warpzone.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Want {
    Host(bool),
    Code(String),
    Join(String, Option<String>),
    List,
}

/// A player of this machine during a session.
#[derive(Clone, Debug)]
struct LocalPlayer {
    name: String,
    source: Option<Source>,
    world: String,
    slot: usize,
}

/// How the last session ended.
#[derive(Clone, Debug, Default)]
pub struct EndResult {
    pub agreed: bool,
    pub desyncs: usize,
    pub aborted: Option<String>,
}

/// Online state kept by the app.
#[derive(Default)]
pub struct OnState {
    net: Option<NetSession>,
    signed_in: bool,
    want: Option<Want>,
    /// This machine's players (the join screen) in a room.
    pub(super) lobby: Option<Box<Lobby>>,
    game: Option<NetGame>,
    start: Option<SessionStart>,
    locals: Vec<LocalPlayer>,
    rooms: Vec<RoomInfo>,
    /// Room id, code, invite link (hosted or joined).
    pub room: Option<(String, Option<String>, String)>,
    public: bool,
    countdown: Option<u32>,
    sent_settings: Option<(MpMode, String, usize)>,
    sent_players: Option<(Vec<String>, bool, bool)>,
    init_sent: BTreeSet<PlayerId>,
    refresh_sent: BTreeMap<usize, usize>,
    resynced: BTreeSet<StreamId>,
    /// Status line in sessions (regrouping, desync), with ticks left.
    note: Option<(String, u32)>,
    pub result: Option<EndResult>,
    /// Desync reports seen this session / resyncs done.
    pub desyncs: u32,
    pub resyncs: u32,
    /// `NetConfig::content`, computed once.
    content: Option<String>,
}

fn on(m: OnMenu) -> MenuId {
    MenuId::Online(m)
}

/// A Warpzone username for this machine (3–16 of `A-Za-z0-9_-`), kept in
/// the data directory.
fn username(dir: Option<&std::path::Path>) -> String {
    if let Ok(n) = std::env::var("BABA_NET_USER")
        && !n.is_empty()
    {
        return n;
    }
    let file = dir.map(|d| d.join("online-name.txt"));
    if let Some(f) = &file
        && let Ok(n) = std::fs::read_to_string(f)
        && !n.trim().is_empty()
    {
        return n.trim().to_string();
    }
    let r = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0)
        ^ std::process::id();
    let n = format!("baba-{:05x}", r & 0xf_ffff);
    if let Some(f) = &file {
        let _ = std::fs::create_dir_all(f.parent().unwrap_or(f));
        let _ = std::fs::write(f, &n);
    }
    n
}

fn mode_label(m: MpMode) -> &'static str {
    match m {
        MpMode::Coop => "co-op",
        MpMode::SharedSave => "shared save",
        MpMode::OwnGames => "own games",
    }
}

/// A rejection as the player should read it.
fn reject_text(reason: &str) -> String {
    if reason.starts_with(REJECT_VERSION) {
        format!(
            "This room runs another version of baba-port.\nBoth machines need the same build to play together.\n\n({reason})"
        )
    } else if reason.starts_with(REJECT_CONTENT) {
        format!(
            "Your Baba Is You data differs from the host's\n(another game version or modified levels).\n\n({reason})"
        )
    } else {
        format!("The room did not let us in: {reason}")
    }
}

impl App {
    fn online_dir(&self) -> Option<&std::path::Path> {
        self.cfg.data_dir.as_deref()
    }

    /// Creates the Warpzone session on first use.
    fn online_net(&mut self) -> Result<&mut NetSession, String> {
        if self.online.net.is_none() {
            let mut cfg = NetConfig::new(&build_id());
            if let Ok(b) = std::env::var("BABA_NET_BUILD") {
                cfg.build = b;
            }
            let content = match &self.online.content {
                Some(c) => c.clone(),
                None => {
                    let c = content_hash(&self.data);
                    self.online.content = Some(c.clone());
                    c
                }
            };
            cfg.content = std::env::var("BABA_NET_CONTENT").unwrap_or(content);
            cfg.auto_register = Some(username(self.online_dir()));
            let net = NetSession::new(cfg).map_err(|e| e.to_string())?;
            self.log.push("online: connecting".into());
            self.online.net = Some(net);
        }
        Ok(self.online.net.as_mut().unwrap())
    }

    /// A lobby of this machine's players for online play.
    fn online_lobby(&self) -> Box<Lobby> {
        let packs = list_packs(&self.data)
            .into_iter()
            .map(|w| {
                let name = WorldInfo::open(&self.data, &w)
                    .map(|d| self.texts.pack_name(&w, &d.data.name))
                    .unwrap_or_else(|| w.clone());
                (w, name)
            })
            .collect();
        let extra = ExtraSlots::load(self.cfg.data_dir.as_deref());
        let mut l = Lobby::new(
            &self.profile.settings,
            packs,
            self.profile.last_slot(),
            extra,
        );
        l.online = true;
        Box::new(l)
    }

    /// Main menu → Online (or the join screen's online key): the online
    /// menu. `lobby`: the join screen's players come along.
    pub fn open_online(&mut self, lobby: Option<Box<Lobby>>) {
        if let Some(mut l) = lobby {
            l.online = true;
            l.to_online = false;
            l.start = false;
            self.online.lobby = Some(l);
        }
        if let Err(e) = self.online_net() {
            self.online_message(format!("Can't reach Warpzone: {e}"));
            return;
        }
        if !matches!(self.mode, Mode::Title) {
            self.audio.set_mixing(false);
            self.mode = Mode::Title;
            self.audio.music("menu");
        }
        self.input = Input::default();
        self.menus = vec![(MenuId::Main, (0, 0)), (on(OnMenu::Main), (0, 0))];
        self.log.push("online menu".into());
    }

    /// `--warpzone-host` / `--warpzone-join <room|invite>` (the launcher,
    /// an invite link).
    pub fn launch_online(&mut self, args: &[String]) {
        let intent = baba_net::warpzone_core::launch::from_args(args);
        if intent.join.is_none() && !intent.host {
            return;
        }
        self.open_online(None);
        if self.online.net.is_none() {
            return;
        }
        if let Some(room) = intent.join {
            self.online.want = Some(Want::Join(room, intent.secret));
            self.push_wait("Joining the room...");
        } else {
            self.log.push("online: host screen".into());
        }
        self.online_run_want();
    }

    fn online_message(&mut self, text: String) {
        self.log
            .push(format!("online: {}", text.replace('\n', " ")));
        if !matches!(self.mode, Mode::Title) {
            // In a room or session the note line shows it.
            self.online.note = Some((text.replace('\n', " "), 600));
            return;
        }
        if matches!(
            self.menus.last(),
            Some((MenuId::Online(OnMenu::Wait(_)), _))
        ) {
            self.menus.pop();
        }
        self.menus.push((on(OnMenu::Message(text)), (0, 0)));
    }

    fn push_wait(&mut self, what: &str) {
        if matches!(
            self.menus.last(),
            Some((MenuId::Online(OnMenu::Wait(_)), _))
        ) {
            self.menus.pop();
        }
        self.menus.push((on(OnMenu::Wait(what.into())), (0, 0)));
    }

    /// Carries out the pending request once signed in.
    fn online_run_want(&mut self) {
        if !self.online.signed_in {
            return;
        }
        let Some(w) = self.online.want.take() else {
            return;
        };
        let Some(net) = self.online.net.as_mut() else {
            return;
        };
        self.log.push(format!("online: {w:?}"));
        match w {
            Want::Host(public) => {
                self.online.public = public;
                if public {
                    net.host_public()
                } else {
                    net.host_private()
                }
            }
            Want::Code(c) => net.join_code(&c),
            Want::Join(room, secret) => net.join(&room, None, secret.as_deref()),
            Want::List => net.list_rooms(false),
        }
    }

    fn online_want(&mut self, w: Want, wait: &str) {
        if self.online_net().is_err() {
            return;
        }
        self.online.want = Some(w);
        if !wait.is_empty() {
            self.push_wait(wait);
        }
        self.online_run_want();
    }

    pub(super) fn online_activate(&mut self, a: OnAct) {
        match a {
            OnAct::Open => self.open_online(None),
            OnAct::HostPublic => self.online_want(Want::Host(true), "Opening a public room..."),
            OnAct::HostPrivate => self.online_want(Want::Host(false), "Opening a private room..."),
            OnAct::EnterCode => self.menus.push((
                on(OnMenu::Code {
                    text: String::new(),
                }),
                (0, 0),
            )),
            OnAct::SubmitCode => {
                let Some((MenuId::Online(OnMenu::Code { text }), _)) = self.menus.pop() else {
                    return;
                };
                if !text.trim().is_empty() {
                    let c = text.trim().to_string();
                    self.online_want(Want::Code(c), "Joining the room...");
                }
            }
            OnAct::Browse => {
                self.menus.push((on(OnMenu::Rooms), (0, 0)));
                self.online_want(Want::List, "");
            }
            OnAct::Refresh => self.online_want(Want::List, ""),
            OnAct::JoinRoom(id) => self.online_want(Want::Join(id, None), "Joining the room..."),
            OnAct::Cancel => {
                self.menus.pop();
                if let Some(net) = &mut self.online.net
                    && net.in_room()
                {
                    net.leave();
                }
                self.online.want = None;
            }
        }
    }

    /// Text entry of the join code; true if it took the frame.
    pub(super) fn online_text(&mut self, cmds: &[Cmd]) -> bool {
        let Some((MenuId::Online(OnMenu::Code { text }), _)) = self.menus.last_mut() else {
            return false;
        };
        for c in std::mem::take(&mut self.input.typed) {
            if c == '\u{8}' {
                text.pop();
            } else if text.chars().count() < 12 && (c.is_ascii_alphanumeric() || c == '-') {
                text.push(c.to_ascii_uppercase());
            }
        }
        for c in cmds {
            match c {
                Cmd::Back => {
                    self.menus.pop();
                    return true;
                }
                Cmd::Confirm => {
                    self.online_activate(OnAct::SubmitCode);
                    return true;
                }
                _ => {}
            }
        }
        true
    }

    pub(super) fn build_online(&self, id: &OnMenu) -> Menu {
        let mut m = Menu::new(Style::List);
        m.top = 70.0;
        m.min_width = 420.0;
        match id {
            OnMenu::Main => {
                m.header.push("Online".into());
                let status = match &self.online.net {
                    None => "not connected".to_string(),
                    Some(_) if !self.online.signed_in => "connecting to Warpzone...".to_string(),
                    Some(_) => "connected to Warpzone".to_string(),
                };
                m.header.push(status);
                let ok = self.online.signed_in;
                m.item(
                    Item::new("Host a private room", Act::Online(OnAct::HostPrivate)).enabled(ok),
                );
                m.item(Item::new("Host a public room", Act::Online(OnAct::HostPublic)).enabled(ok));
                m.item(Item::new("Join with a code", Act::Online(OnAct::EnterCode)).enabled(ok));
                m.item(Item::new("Browse rooms", Act::Online(OnAct::Browse)).enabled(ok));
                m.gap(10.0);
                m.item(Item::new(self.texts.get("return"), Act::Close));
                let n = self.online.lobby.as_ref().map_or(0, |l| l.cards.len());
                m.footer.push(if n > 0 {
                    format!("{n} player(s) of this machine come along")
                } else {
                    "Players of this machine join in the room".into()
                });
            }
            OnMenu::Code { text } => {
                m.top = 150.0;
                m.header.push("Enter the room code:".into());
                m.header.push(format!("{text}_"));
                m.item(Item::new("Join", Act::Online(OnAct::SubmitCode)));
                m.item(Item::new(self.texts.get("return"), Act::Close));
            }
            OnMenu::Rooms => {
                m.top = 40.0;
                m.min_width = 600.0;
                m.header.push("Public rooms".into());
                m.item(Item::new("Refresh", Act::Online(OnAct::Refresh)));
                m.item(Item::new(self.texts.get("return"), Act::Close));
                if self.online.rooms.is_empty() {
                    m.footer.push("No public rooms right now.".into());
                }
                for r in self.online.rooms.iter().take(10) {
                    let meta = |k: &str| r.metadata.get(k).cloned().unwrap_or_default();
                    let other = if r.game_version == build_id() {
                        ""
                    } else {
                        "  (other version)"
                    };
                    let label = format!(
                        "{}  {}  {}  {}{other}",
                        r.host,
                        meta("players"),
                        meta("mode"),
                        meta("world")
                    );
                    m.item(Item::new(label, Act::Online(OnAct::JoinRoom(r.id.clone()))));
                }
            }
            OnMenu::Wait(t) => {
                m.top = 180.0;
                m.header.push(t.clone());
                m.item(Item::new("Cancel", Act::Online(OnAct::Cancel)));
                m.escape = Act::Online(OnAct::Cancel);
            }
            OnMenu::Message(t) => {
                m.top = 140.0;
                for l in t.lines() {
                    m.header.push(l.to_string());
                }
                m.item(Item::new("OK", Act::Close));
            }
        }
        m
    }

    // ------------------------------------------------------------ events

    /// Every tick: network work and events (every mode).
    pub(super) fn online_poll(&mut self) {
        let Some(net) = self.online.net.as_mut() else {
            return;
        };
        net.poll();
        let mut events = Vec::new();
        while let Some(e) = net.next_event() {
            events.push(e);
        }
        for e in events {
            self.online_event(e);
        }
        if let Some((_, t)) = &mut self.online.note {
            *t = t.saturating_sub(1);
            if *t == 0 {
                self.online.note = None;
            }
        }
    }

    fn online_event(&mut self, e: NetEvent) {
        match e {
            NetEvent::Warpzone(ev) => match *ev {
                WzEvent::LoggedIn {
                    account: Some(a), ..
                } => {
                    self.online.signed_in = true;
                    self.log
                        .push(format!("online: signed in as {}", a.username));
                    self.online_run_want();
                }
                WzEvent::Registered(a) => {
                    self.online.signed_in = true;
                    self.log.push(format!("online: registered {}", a.username));
                    self.online_run_want();
                }
                WzEvent::RoomHosted { room, code, invite } => {
                    self.log.push(format!(
                        "online: hosted {} code {}",
                        room.id,
                        code.clone().unwrap_or_default()
                    ));
                    self.online.room = Some((room.id, code, invite));
                }
                WzEvent::RoomJoined { room, host } => {
                    self.log
                        .push(format!("online: in room {} host={host}", room.id));
                    if !host {
                        let keep = self.online.room.as_ref().is_some_and(|r| r.0 == room.id);
                        if !keep {
                            self.online.room = Some((room.id, None, String::new()));
                        }
                    }
                    self.enter_room();
                }
                WzEvent::RoomList(mut rooms) => {
                    // Newest first; rooms of the regrouping after a host
                    // left are not for strangers.
                    rooms.retain(|r| !r.metadata.contains_key(baba_net::session::META_MIGRATE));
                    rooms.sort_by_key(|r| std::cmp::Reverse(r.created));
                    self.online.rooms = rooms;
                }
                WzEvent::KnockDenied { .. } => {
                    self.online_message("The host did not let us in.".into())
                }
                WzEvent::Error { op, message, .. } => {
                    self.log
                        .push(format!("online: warpzone error {op}: {message}"));
                    if matches!(
                        self.menus.last(),
                        Some((MenuId::Online(OnMenu::Wait(_)), _))
                    ) {
                        self.online_message(format!("Warpzone: {message}"));
                    }
                }
                _ => {}
            },
            NetEvent::Welcome { machine } => {
                self.log.push(format!("online: welcome as {machine}"));
            }
            NetEvent::Rejected { reason } => {
                self.log.push(format!("online: rejected: {reason}"));
                self.leave_room_screen();
                self.online_message(reject_text(&reason));
            }
            NetEvent::LobbyChanged | NetEvent::MachineJoined { .. } => {}
            NetEvent::MachineLeft { machine } => {
                self.online_note(format!("{machine} left"));
            }
            NetEvent::SessionStarted(s) => {
                self.log.push(format!(
                    "online: session {} {:?}",
                    s.session,
                    mp_mode(s.mode)
                ));
                self.online.result = None;
                self.online.desyncs = 0;
                self.online.resyncs = 0;
                self.online.resynced.clear();
                self.online.init_sent.clear();
                self.online.refresh_sent.clear();
                self.online.start = Some(s.clone());
                self.online.locals = self.lobby_locals();
                if let Err(e) = self.online_build(&s) {
                    self.online_message(e);
                }
            }
            NetEvent::Desync(r) => self.online_desync(r),
            NetEvent::SessionEnded(r) => {
                let me = self.online.net.as_ref().and_then(|n| n.me());
                let mine = r
                    .hashes
                    .iter()
                    .find(|(m, _)| Some(*m) == me)
                    .map(|h| h.1.iter().fold(0u64, |a, (_, v)| a.rotate_left(7) ^ *v));
                self.log.push(format!(
                    "online: session ended agreed={} machines={} desyncs={} final={:016x}",
                    r.agreed,
                    r.hashes.len(),
                    r.desyncs.len(),
                    mine.unwrap_or(0)
                ));
                self.online.result = Some(EndResult {
                    agreed: r.agreed,
                    desyncs: r.desyncs.len(),
                    aborted: None,
                });
                self.back_to_room(if r.agreed {
                    "Session over: every machine agrees on every game.".into()
                } else {
                    "Session over, but the machines disagree on a game state.".into()
                });
            }
            NetEvent::SessionAborted { reason } => {
                self.log.push(format!("online: session aborted: {reason}"));
                self.online.result = Some(EndResult {
                    agreed: false,
                    desyncs: 0,
                    aborted: Some(reason.clone()),
                });
                self.back_to_room(format!("Session aborted: {reason}"));
            }
            NetEvent::HostLost { successor } => {
                self.log
                    .push(format!("online: host lost, successor {successor}"));
                self.online_note(format!("The host left: regrouping around {successor}..."));
            }
            NetEvent::HostMigrated { host, room } => {
                self.log
                    .push(format!("online: host migrated to {host} room {room}"));
                let code = self.online.net.as_ref().map(|n| {
                    let l = n.lobby();
                    (l.code.clone(), l.invite.clone())
                });
                if let Some((code, invite)) = code {
                    self.online.room = Some((room, code, invite));
                }
                self.online_note(format!("{host} hosts the session now"));
            }
            NetEvent::RoomLeft { reason } => {
                self.log.push(format!("online: left the room: {reason}"));
                let was = self.leave_room_screen();
                if was && reason != "left" {
                    self.online_message(format!("Disconnected from the room ({reason})."));
                }
            }
            NetEvent::App { .. } => {}
        }
    }

    fn online_note(&mut self, s: String) {
        self.log.push(format!("online: {s}"));
        self.online.note = Some((s, 300));
    }

    /// Hosted or joined: the room lobby with this machine's players.
    fn enter_room(&mut self) {
        if self.online.lobby.is_none() {
            self.online.lobby = Some(self.online_lobby());
        }
        if let Some(l) = &mut self.online.lobby {
            l.online = true;
            l.unready();
        }
        self.online.sent_settings = None;
        self.online.sent_players = None;
        self.online.countdown = None;
        self.menus.clear();
        self.input = Input::default();
        if !matches!(self.mode, Mode::Multi(_)) {
            self.mode = Mode::Online;
        }
    }

    /// Out of the room: back to the online menu. True if we were in a
    /// room screen or session.
    fn leave_room_screen(&mut self) -> bool {
        let was = matches!(self.mode, Mode::Online | Mode::Multi(_));
        if let Mode::Multi(m) = &mut self.mode {
            let mut cx = Cx {
                data: &self.data,
                objects: &self.objects,
                gfx: &mut self.gfx,
                texts: &self.texts,
                profile: &mut self.profile,
                audio: &mut self.audio,
                cursor: self.cursor_sprite,
            };
            m.save_all(&mut cx);
        }
        self.online.game = None;
        self.online.room = None;
        if was {
            self.audio.set_mixing(false);
            self.mode = Mode::Title;
            self.audio.music("menu");
            self.input = Input::default();
            self.menus = vec![(MenuId::Main, (0, 0)), (on(OnMenu::Main), (0, 0))];
        } else if matches!(
            self.menus.last(),
            Some((MenuId::Online(OnMenu::Wait(_)), _))
        ) {
            self.menus.pop();
        }
        was
    }

    /// The session ended: back in the room lobby.
    fn back_to_room(&mut self, msg: String) {
        if let Mode::Multi(m) = &mut self.mode {
            let mut cx = Cx {
                data: &self.data,
                objects: &self.objects,
                gfx: &mut self.gfx,
                texts: &self.texts,
                profile: &mut self.profile,
                audio: &mut self.audio,
                cursor: self.cursor_sprite,
            };
            m.save_all(&mut cx);
            self.log.append(&mut m.log);
        }
        self.online.game = None;
        self.audio.set_mixing(false);
        if let Some(l) = &mut self.online.lobby {
            l.unready();
        }
        self.online.sent_players = None;
        self.mode = Mode::Online;
        self.audio.music("menu");
        self.online_note(msg);
    }

    fn lobby_locals(&self) -> Vec<LocalPlayer> {
        let Some(l) = &self.online.lobby else {
            return Vec::new();
        };
        l.players
            .seats
            .iter()
            .map(|s| LocalPlayer {
                name: s.name().to_string(),
                source: s.source,
                world: if l.mode == MpMode::OwnGames {
                    s.world.clone()
                } else {
                    l.world.clone()
                },
                slot: s.slot,
            })
            .collect()
    }

    /// Builds the session's (empty) replica; the stream fills it.
    fn online_build(&mut self, s: &SessionStart) -> Result<(), String> {
        let Some(settings) = SessionSettings::decode(&s.settings) else {
            return Err("The host sent session settings we can't read.".into());
        };
        let net = self.online.net.as_ref().ok_or("no network")?;
        let me = net.me();
        let host = net.is_host();
        let mode = mp_mode(s.mode);
        let side = NetSide {
            local: BTreeMap::new(),
            sources: self.online.locals.iter().map(|l| l.source).collect(),
            local_games: self
                .online
                .locals
                .iter()
                .map(|l| (l.world.clone(), l.slot))
                .collect(),
            host,
            quiet: true,
            ..Default::default()
        };
        let players = Players::new(&self.profile.settings, &settings.world);
        let extra = ExtraSlots::load(self.cfg.data_dir.as_deref());
        let mut cx = Cx {
            data: &self.data,
            objects: &self.objects,
            gfx: &mut self.gfx,
            texts: &self.texts,
            profile: &mut self.profile,
            audio: &mut self.audio,
            cursor: self.cursor_sprite,
        };
        let mut m = Multi::new_net(mode, players, &settings.world, extra, side, &mut cx);
        let g = NetGame::new(mode, settings, me, host, &mut m, &mut cx);
        self.online.game = Some(g);
        self.menus.clear();
        self.input = Input::default();
        self.mode = Mode::Multi(Box::new(m));
        Ok(())
    }

    /// A machine disagrees: if it is us, rebuild the replica and replay
    /// every stream; still different → leave with a message.
    fn online_desync(&mut self, r: DesyncReport) {
        self.online.desyncs += 1;
        let me = self.online.net.as_ref().and_then(|n| n.me());
        let bad = r.desynced();
        self.log.push(format!(
            "online: desync {} at {}: {:?}",
            r.stream, r.index, bad
        ));
        if !me.is_some_and(|m| bad.contains(&m)) {
            let who: Vec<String> = bad.iter().map(|m| m.to_string()).collect();
            self.online_note(format!("{} out of sync ({})", who.join(", "), r.stream));
            return;
        }
        if !self.online.resynced.insert(r.stream) {
            self.leave_desynced();
            return;
        }
        let Some(start) = self.online.start.clone() else {
            return;
        };
        self.online_note("Out of sync: replaying the session...".into());
        // A fresh replica, every stream replayed from entry 0.
        let quiet_build = self.online_build(&start);
        if quiet_build.is_err() {
            self.leave_desynced();
            return;
        }
        if let Some(n) = &mut self.online.net {
            n.restart_replay();
        }
        self.online_pump(usize::MAX);
        let ok = self
            .online
            .net
            .as_ref()
            .and_then(|n| n.own_hash(r.stream, r.index))
            == Some(r.majority);
        if ok {
            self.online.resyncs += 1;
            self.online_note("Back in sync (replayed the session).".into());
        } else {
            self.leave_desynced();
        }
    }

    fn leave_desynced(&mut self) {
        self.log.push("online: can't resync, leaving".into());
        if let Some(n) = &mut self.online.net {
            n.leave();
        }
        self.leave_room_screen();
        self.online_message(
            "This machine got out of sync with the others and could not\nrecover. Check that everybody runs the same baba-port and\nthe same Baba Is You data, then join again."
                .into(),
        );
    }

    // ------------------------------------------------------------- lobby

    /// The room lobby: this machine's players, ready, the host's choices.
    pub(super) fn tick_online(&mut self) {
        let Some(l) = self.online.lobby.as_mut() else {
            self.online.lobby = Some(self.online_lobby());
            return;
        };
        let Some(net) = self.online.net.as_mut() else {
            return;
        };
        let host = net.is_host();
        l.guest = !host;
        if !host {
            l.mode = mp_mode(net.lobby().mode);
            if let Some(s) = SessionSettings::decode(&net.lobby().settings) {
                l.world = s.world;
            }
        }
        l.tick();
        self.log.append(&mut l.log);
        if l.exit {
            l.exit = false;
            net.leave();
            return;
        }
        // Our players and readiness.
        let names: Vec<String> = l
            .players
            .seats
            .iter()
            .map(|s| s.name().to_string())
            .collect();
        let ready = l.all_ready();
        // (Resent once welcomed: before that the host does not hear us.)
        let now = (names.clone(), ready, net.me().is_some());
        if self.online.sent_players.as_ref() != Some(&now) {
            net.set_local_players(names);
            net.set_ready(ready);
            self.online.sent_players = Some(now);
        }
        if !host {
            self.online.countdown = None;
            l.online_countdown = None;
            return;
        }
        // Host: mode, world and the save everybody starts from.
        let key = (l.mode, l.world.clone(), l.slot);
        if self.online.sent_settings.as_ref() != Some(&key) {
            let save = if l.mode == MpMode::OwnGames {
                String::new()
            } else {
                saves::slot(&self.profile, &l.extra, l.slot).to_text()
            };
            let s = SessionSettings {
                world: l.world.clone(),
                slot: l.slot,
                save,
            };
            let _ = net.set_mode(net_mode(l.mode));
            if let Err(e) = net.set_settings(s.encode()) {
                self.log.push(format!("online: settings: {e}"));
            }
            let world = l.world.clone();
            let label = l
                .packs
                .iter()
                .find(|p| p.0 == world)
                .map(|p| p.1.clone())
                .unwrap_or(world);
            net.set_room_label("world", &label);
            self.online.sent_settings = Some(key);
        }
        // Every machine has players and is ready (a machine without
        // players watches; the host starts with them by the start key).
        let machines = &net.lobby().machines;
        // (Alone in the room: only F5 starts.)
        let go = machines.len() > 1 && machines.iter().all(|m| !m.players.is_empty() && m.ready);
        let force =
            std::mem::take(&mut l.force_start) && machines.iter().any(|m| !m.players.is_empty());
        if force {
            self.online.countdown = Some(1);
        }
        if go || force || self.online.countdown == Some(1) {
            let t = self.online.countdown.unwrap_or(45);
            if t <= 1 {
                self.online.countdown = None;
                match net.start_session(None) {
                    Ok(s) => self
                        .log
                        .push(format!("online: started session {}", s.session)),
                    Err(e) => self.log.push(format!("online: start: {e}")),
                }
            } else {
                self.online.countdown = Some(t - 1);
            }
        } else {
            self.online.countdown = None;
        }
        l.online_countdown = self.online.countdown;
    }

    // ----------------------------------------------------------- session

    /// Applies new stream entries (at most `max`).
    fn online_pump(&mut self, max: usize) -> usize {
        let (Some(net), Some(g)) = (self.online.net.as_mut(), self.online.game.as_mut()) else {
            return 0;
        };
        let Mode::Multi(m) = &mut self.mode else {
            return 0;
        };
        let mut cx = Cx {
            data: &self.data,
            objects: &self.objects,
            gfx: &mut self.gfx,
            texts: &self.texts,
            profile: &mut self.profile,
            audio: &mut self.audio,
            cursor: self.cursor_sprite,
        };
        // Catching up (late join, many entries at once): states only.
        let st = net.stats();
        if let Some(n) = &mut m.net
            && st.entries.saturating_sub(st.applied) > 24
        {
            n.quiet = true;
        }
        let mut rep = Rep { g, m, cx: &mut cx };
        let k = net.pump_limited(&mut rep, max);
        if net.caught_up()
            && let Some(n) = &mut m.net
        {
            n.quiet = false;
        }
        self.log.append(&mut g.log);
        self.log.append(&mut m.log);
        k
    }

    /// One tick of an online session (`Mode::Multi` with a net side).
    pub(super) fn tick_online_session(&mut self) {
        if self.online.net.is_none() || self.online.game.is_none() {
            return;
        }
        // Catch up gradually after a late join (thousands of turns per
        // second, without stalling a frame).
        self.online_pump(4000);
        {
            let Mode::Multi(m) = &mut self.mode else {
                return;
            };
            let mut cx = Cx {
                data: &self.data,
                objects: &self.objects,
                gfx: &mut self.gfx,
                texts: &self.texts,
                profile: &mut self.profile,
                audio: &mut self.audio,
                cursor: self.cursor_sprite,
            };
            m.tick(&mut cx);
            self.log.append(&mut m.log);
        }
        self.online_send();
        self.online_pump(4000);
        self.online_predict();
    }

    /// Mode 1 on a guest far from the host (round trip > 80 ms, or
    /// `BABA_NET_PREDICT=1`): show the confirmed level plus our turns not
    /// sequenced yet; it snaps to the confirmed state when they arrive.
    fn online_predict(&mut self) {
        let Some(net) = &self.online.net else {
            return;
        };
        let Mode::Multi(m) = &mut self.mode else {
            return;
        };
        let Some(v) = m.views.first_mut() else {
            return;
        };
        let want = match std::env::var("BABA_NET_PREDICT").ok().as_deref() {
            Some("1") => true,
            Some("0") => false,
            _ => net
                .stats()
                .peers
                .iter()
                .any(|p| p.rtt_ms.is_some_and(|r| r > 80)),
        };
        if m.mode != MpMode::Coop || net.is_host() || !want || net.pending().is_empty() || v.busy()
        {
            v.predict = None;
            return;
        }
        let mut level = v.play.cur.level.clone();
        for p in net.pending() {
            match Turn::decode(&p.data) {
                Some(Turn::Move { dir, you }) => {
                    level.step(baba_sim::Input::Key { dir, player: you });
                }
                Some(Turn::Undo) => {
                    level.step(baba_sim::Input::Undo);
                }
                Some(Turn::Auto) if level.auto_active() => {
                    level.step(baba_sim::Input::Tick);
                }
                _ => break,
            }
        }
        v.predict = Some(level);
    }

    /// Sends what the session made this tick: turns, mode-3 saves, mode-2
    /// progress and refreshes, new local players; leader and host flags.
    fn online_send(&mut self) {
        let (Some(net), Some(g)) = (self.online.net.as_mut(), self.online.game.as_mut()) else {
            return;
        };
        let Mode::Multi(m) = &mut self.mode else {
            return;
        };
        let Some(n) = m.net.as_mut() else {
            return;
        };
        let per_player = g.mode != MpMode::Coop;
        let send = |net: &mut NetSession, g: &mut NetGame, li: usize, data: &[u8]| match net
            .input(li, data)
        {
            Ok(p) => {
                if per_player {
                    let len = net.stream(StreamId::Player(p)).len() as u32;
                    g.made.insert((StreamId::Player(p), len.saturating_sub(1)));
                }
            }
            Err(e) => log::debug!("online input: {e}"),
        };
        for (seat, turn) in std::mem::take(&mut n.outbox) {
            if let Some(&li) = n.local.get(&seat) {
                send(net, g, li, &turn.encode());
            }
        }
        // Mode 3: each local player's starting save, once admitted.
        if g.mode == MpMode::OwnGames {
            for li in 0..n.local_games.len() {
                let Some(p) = net.local_player(li) else {
                    continue;
                };
                if self.online.init_sent.contains(&p) {
                    continue;
                }
                let (world, slot) = n.local_games[li].clone();
                let text = saves::slot(&self.profile, &m.extra, slot).to_text();
                for c in init_chunks(&world, &text) {
                    if let Err(e) = net.own_event(li, &c) {
                        self.log.push(format!("online: save of l{li}: {e}"));
                    }
                }
                self.online.init_sent.insert(p);
            }
        }
        // Mode 2: progress for the shared save, then refresh our maps.
        for (li, ev) in std::mem::take(&mut n.events) {
            if let Err(e) = net.event(li, &ev) {
                self.log.push(format!("online: progress event: {e}"));
            }
        }
        if g.mode == MpMode::SharedSave {
            let total = g.events_len();
            let seats: Vec<(usize, usize)> = n.local.iter().map(|(s, l)| (*s, *l)).collect();
            for (seat, li) in seats {
                let Some(vi) = m.views.iter().position(|v| v.seats.first() == Some(&seat)) else {
                    continue;
                };
                let v = &m.views[vi];
                let ready =
                    v.play.cur.is_map() && !v.busy() && v.play.pending.is_none() && !v.waiting();
                if ready
                    && g.unmerged(PlayerId(seat as u8))
                    && self.online.refresh_sent.get(&seat) != Some(&total)
                {
                    self.online.refresh_sent.insert(seat, total);
                    send(net, g, li, &Turn::Refresh.encode());
                }
            }
        }
        let Some(n) = m.net.as_mut() else {
            return;
        };
        // New local players mid-session.
        if !n.joins.is_empty() {
            for src in std::mem::take(&mut n.joins) {
                let k = self.online.locals.len();
                if k >= crate::join::MAX_PLAYERS {
                    break;
                }
                let world = match self.online.locals.first() {
                    Some(l) => l.world.clone(),
                    None if !g.settings.world.is_empty() => g.settings.world.clone(),
                    None => "baba".to_string(),
                };
                let used: Vec<usize> = self.online.locals.iter().map(|l| l.slot).collect();
                let slot = (0..saves::ALL_SLOTS)
                    .find(|s| !used.contains(s))
                    .unwrap_or(0);
                self.online.locals.push(LocalPlayer {
                    name: crate::join::NAMES[k % crate::join::MAX_PLAYERS].to_string(),
                    source: Some(src),
                    world: world.clone(),
                    slot,
                });
                n.sources.push(Some(src));
                n.local_games.push((world, slot));
            }
            net.set_local_players(self.online.locals.iter().map(|l| l.name.clone()).collect());
        }
        // Mode 1: the machine of the lowest active player sends Proceed.
        let me = net.me();
        let lowest = net
            .players()
            .iter()
            .filter(|p| p.active)
            .min_by_key(|p| p.info.player)
            .map(|p| p.info.machine);
        n.leader = me.is_some() && lowest == me;
        n.host = net.is_host();
        if n.end {
            n.end = false;
            if net.is_host() {
                let _ = net.end_session();
                self.log.push("online: ending the session".into());
            }
        }
        if n.leave {
            n.leave = false;
            net.leave();
        }
    }

    // ------------------------------------------------------------ drawing

    /// The room lobby screen.
    pub(super) fn draw_online(&mut self, ui: &mut Ui) {
        let g = &mut self.gfx;
        let white = g.pal(0, 3);
        let grey = g.pal(0, 1);
        ui.fill(g, g.pal(1, 0));
        // This machine's players: the join screen, smaller, on the left.
        if let Some(l) = &self.online.lobby {
            let k = 0.70;
            let mut sub = Ui {
                quads: Vec::new(),
                scale: ui.scale * k,
                width: 854.0 * ui.scale * k,
                height: 480.0 * ui.scale * k,
                x0: None,
            };
            l.draw(&mut sub, g, &self.texts, &self.profile);
            let (ox, oy) = (ui.x(0.0), ui.y(70.0));
            for mut q in sub.quads {
                q.x += ox;
                q.y += oy;
                ui.quads.push(q);
            }
        }
        let net = self.online.net.as_ref();
        let lobby = net.map(|n| n.lobby());
        let host = net.is_some_and(|n| n.is_host());
        let title = match &self.online.room {
            Some((_, Some(code), _)) => format!("Online room {code}"),
            Some((id, None, _)) => format!("Online room {}", &id[..id.len().min(12)]),
            None => "Online room".into(),
        };
        ui.text(g, None, &title, 8.0, 4.0, 26.0, g.pal(4, 1), Align::Left);
        let vis = if self.online.public {
            "public"
        } else {
            "private"
        };
        let sub = match (&self.online.room, host) {
            (Some((_, _, inv)), true) if !inv.is_empty() => format!("{vis}, invite: {inv}"),
            (_, true) => vis.to_string(),
            _ => "waiting for the host to start".into(),
        };
        let sub: String = sub.chars().take(90).collect();
        ui.text(g, None, &sub, 8.0, 36.0, 11.0, grey, Align::Left);
        if let Some(lb) = lobby {
            let mode = mp_mode(lb.mode);
            let world = SessionSettings::decode(&lb.settings)
                .map(|s| s.world)
                .unwrap_or_default();
            ui.text(
                g,
                None,
                &format!("{}  {world}", mode_label(mode)),
                846.0,
                8.0,
                14.0,
                white,
                Align::Right,
            );
            // Machines.
            let x = 610.0;
            ui.text(g, None, "Machines", x, 70.0, 14.0, g.pal(4, 1), Align::Left);
            let rtts: BTreeMap<u8, Option<u32>> = net
                .map(|n| {
                    n.stats()
                        .peers
                        .iter()
                        .map(|p| (p.machine.0, p.rtt_ms))
                        .collect()
                })
                .unwrap_or_default();
            for (i, m) in lb.machines.iter().take(16).enumerate() {
                let y = 92.0 + i as f32 * 22.0;
                let tag = if m.machine == lb.host { " (host)" } else { "" };
                let me = net.and_then(|n| n.me()) == Some(m.machine);
                let rtt = rtts
                    .get(&m.machine.0)
                    .copied()
                    .flatten()
                    .map(|r| format!(" {r}ms"))
                    .unwrap_or_default();
                let name: String = m.name.chars().take(14).collect();
                let line = format!("{}{name}{tag}{}", if me { "> " } else { "" }, rtt);
                let col = if m.ready || m.players.is_empty() {
                    g.pal(5, 3)
                } else {
                    white
                };
                ui.text(g, None, &line, x, y, 12.0, col, Align::Left);
                let who = if m.players.is_empty() {
                    "watching".to_string()
                } else {
                    format!(
                        "{} player(s){}",
                        m.players.len(),
                        if m.ready { ", ready" } else { "" }
                    )
                };
                ui.text(g, None, &who, x + 12.0, y + 11.0, 9.0, grey, Align::Left);
            }
        } else {
            ui.text(
                g,
                None,
                "connecting to the host...",
                610.0,
                70.0,
                14.0,
                white,
                Align::Left,
            );
        }
        if let Some((n, _)) = &self.online.note {
            ui.text(g, None, n, 427.0, 466.0, 12.0, g.pal(2, 4), Align::Center);
        }
    }

    /// Session overlay: connection notes, regrouping.
    pub(super) fn draw_online_status(&mut self, ui: &mut Ui) {
        let g = &mut self.gfx;
        let net = self.online.net.as_ref();
        let mut lines = Vec::new();
        if net.is_some_and(|n| n.is_migrating()) {
            lines.push("Regrouping after the host left...".to_string());
        }
        if let Some((n, _)) = &self.online.note {
            lines.push(n.clone());
        }
        if self
            .online
            .game
            .as_ref()
            .is_some_and(|_| net.is_some_and(|n| !n.caught_up()))
        {
            lines.push("Catching up...".into());
        }
        for (i, l) in lines.iter().enumerate() {
            let y = 4.0 + i as f32 * 16.0;
            let w = ui.measure(g, None, l, 12.0);
            ui.rect(g, 427.0 - w / 2.0 - 6.0, y - 1.0, w + 12.0, 16.0, {
                let mut c = g.pal(1, 0);
                c[3] = 200;
                c
            });
            ui.text(g, None, l, 427.0, y, 12.0, g.pal(2, 4), Align::Center);
        }
    }

    // ------------------------------------------------- scripts and tests

    /// Online state for scripts: `online <phase> ...`.
    pub fn online_state(&self) -> String {
        let Some(net) = &self.online.net else {
            return "offline".into();
        };
        let l = net.lobby();
        let phase = match &self.mode {
            Mode::Online => "room",
            Mode::Multi(m) if m.net.is_some() => "session",
            _ if net.in_room() => "joining",
            _ => "menu",
        };
        let active = net.players().iter().filter(|p| p.active).count();
        format!(
            "online {phase} me={} host={} machines={} players={} mode={:?} signed={} caught_up={} pending={}",
            net.me().map(|m| m.to_string()).unwrap_or("-".into()),
            net.is_host(),
            l.machines.len(),
            active,
            l.mode,
            self.online.signed_in,
            net.caught_up(),
            net.pending().len()
        )
    }

    pub fn online_signed_in(&self) -> bool {
        self.online.signed_in
    }

    /// A Warpzone session exists (scripts pace ticks in real time).
    pub fn online_active(&self) -> bool {
        self.online.net.is_some()
    }

    /// Room id, code and invite link.
    pub fn online_room(&self) -> Option<(String, Option<String>, String)> {
        self.online.room.clone()
    }

    pub fn online_result(&self) -> Option<EndResult> {
        self.online.result.clone()
    }

    /// Desync reports seen and resyncs done this session.
    pub fn online_desyncs(&self) -> (u32, u32) {
        (self.online.desyncs, self.online.resyncs)
    }

    /// Host or join (scripts): like the menu items.
    pub fn online_host(&mut self, public: bool) {
        self.open_online(None);
        self.online_activate(if public {
            OnAct::HostPublic
        } else {
            OnAct::HostPrivate
        });
    }

    pub fn online_join_code(&mut self, code: &str) {
        self.open_online(None);
        self.online_want(Want::Code(code.to_string()), "Joining the room...");
    }

    pub fn online_join_room(&mut self, room: &str) {
        self.open_online(None);
        self.online_want(Want::Join(room.to_string(), None), "Joining the room...");
    }

    /// Host (scripts): the mode of the next session.
    pub fn online_set_mode(&mut self, m: MpMode) {
        if let Some(l) = &mut self.online.lobby {
            l.mode = m;
            if m == MpMode::OwnGames {
                // Distinct slots for this machine's players.
                let ids: Vec<usize> = l.players.seats.iter().map(|s| s.id).collect();
                for (k, id) in ids.into_iter().enumerate() {
                    if let Some(s) = l.players.by_id_mut(id) {
                        s.slot = k;
                    }
                }
            }
        }
    }

    pub fn online_end(&mut self) {
        if let Some(net) = &mut self.online.net
            && net.is_host()
        {
            let _ = net.end_session();
        }
    }

    pub fn online_leave(&mut self) {
        if let Some(net) = &mut self.online.net {
            net.leave();
        }
    }

    /// Testing: vanish without a goodbye.
    pub fn online_crash(&mut self) {
        if let Some(net) = &mut self.online.net {
            net.simulate_crash();
        }
        self.leave_room_screen();
    }

    pub fn online_is_host(&self) -> bool {
        self.online.net.as_ref().is_some_and(|n| n.is_host())
    }

    /// Everything we have is applied and nothing of ours is in flight.
    pub fn online_quiet(&self) -> bool {
        self.online
            .net
            .as_ref()
            .is_some_and(|n| n.caught_up() && n.pending().is_empty())
    }

    /// Total stream entries we have (settle detection).
    pub fn online_entries(&self) -> u64 {
        self.online.net.as_ref().map_or(0, |n| n.stats().entries)
    }

    /// Seat id of local player `i` in a session.
    pub fn local_seat(&self, i: usize) -> Option<usize> {
        let Mode::Multi(m) = &self.mode else {
            return None;
        };
        m.net
            .as_ref()?
            .local
            .iter()
            .find(|(_, l)| **l == i)
            .map(|(s, _)| *s)
    }

    /// Seat ids of this machine's players.
    pub fn local_seats(&self) -> Vec<usize> {
        let Mode::Multi(m) = &self.mode else {
            return Vec::new();
        };
        m.net
            .as_ref()
            .map(|n| n.local.keys().copied().collect())
            .unwrap_or_default()
    }

    /// Feeds a command to a seat as if its device sent it (bots).
    pub fn online_push(&mut self, seat: usize, c: Cmd) {
        if let Mode::Multi(m) = &mut self.mode {
            m.players.push(seat, c);
        }
    }

    /// Testing: change this machine's copy of a game behind the stream's
    /// back (a desync).
    pub fn online_corrupt(&mut self) -> bool {
        let Mode::Multi(m) = &mut self.mode else {
            return false;
        };
        let mut cx = Cx {
            data: &self.data,
            objects: &self.objects,
            gfx: &mut self.gfx,
            texts: &self.texts,
            profile: &mut self.profile,
            audio: &mut self.audio,
            cursor: self.cursor_sprite,
        };
        let net = m.net.take();
        let mut done = false;
        for vi in 0..m.views.len() {
            let before = m.views[vi].play.state_hash();
            for d in [0, 1, 2, 3, 4] {
                let _ = m.apply_now(vi, None, Turn::Move { dir: d, you: 1 }, &mut cx);
                if m.views[vi].play.state_hash() != before {
                    done = true;
                    break;
                }
            }
            if done {
                break;
            }
        }
        m.net = net;
        self.log.push(format!("online: corrupted={done}"));
        done
    }
}
