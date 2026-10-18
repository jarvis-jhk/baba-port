//! Online play (M7): the game as a `baba_net::Replica` (`docs/net.md`).
//!
//! * **One path for every change.** A stream entry is a [`Turn`]
//!   (`Turn::encode`) applied through [`Multi::apply_now`] →
//!   `Play::apply`, on every machine, in stream order. Turns made on this
//!   machine are only *sent* (`Multi`'s outbox); they change an instance
//!   when they come back from the stream.
//! * **Mode 1 (co-op)**: one instance driven by the host-sequenced shared
//!   stream; it plays on a copy of the host's save (in the session
//!   settings, [`SessionSettings`]); the host writes it to its slot file.
//!   Seats are session players (`p0..`), YOU / YOU2 by player number.
//! * **Modes 2/3**: one instance per player stream, on every machine.
//!   Mode 3: the stream starts with the player's own save (chunks of
//!   [`init_chunks`], an own-stream event); only the owner writes it to its
//!   slot file. Mode 2: every instance plays on its own copy of the shared
//!   save; a player's progress (a win, an ending, a level conversion:
//!   [`Progress`]) goes into the shared stream as an event made by the
//!   player's machine, sequenced by the host, and is merged into another
//!   player's copy exactly when that player's stream reaches it (the
//!   entry's `sync`), so every machine computes the same. The player's
//!   own machine then sends [`Turn::Refresh`] so the map shows it at
//!   once. Merging is idempotent (a level won twice counts once).
//! * **Hashes**: the shared stream → the co-op instance (mode 1), the
//!   shared save (mode 2) or the roster (mode 3); a player stream → that
//!   player's instance and save copy.
//! * Players that leave keep their instance (hidden): stream entries may
//!   still arrive for it on some machines after the `Left`.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::Path;

use baba_net::{Entry, EntryKind, MachineId, Mode, PlayerId, PlayerInfo, Replica, StreamId};
use baba_progress::Slot;

use crate::join::{MAX_PLAYERS, Source};
use crate::play::{Env, Play, win_progress};
use crate::session::apply::{FNV0, fnv};
use crate::session::multi::{Cx, Multi, View};
use crate::session::{Fx, MpMode, Pending, Turn};
use crate::world::WorldInfo;

/// Version of what baba-port puts into streams and settings (turn
/// encoding, settings, events); part of the build id machines must share.
pub const GAME_NET: u32 = 1;

/// The build id rooms compare (`NetConfig::build`).
pub fn build_id() -> String {
    format!("baba-port {} g{GAME_NET}", env!("CARGO_PKG_VERSION"))
}

/// Our side of an online session, kept in [`Multi::net`].
#[derive(Default)]
pub struct NetSide {
    /// Turns made here, to send: `(seat, turn)`.
    pub outbox: Vec<(usize, Turn)>,
    /// This machine's seats: seat id → local player index.
    pub local: BTreeMap<usize, usize>,
    /// Devices of the local players, by local index.
    pub sources: Vec<Option<Source>>,
    /// Mode 3: world and slot file of each local player.
    pub local_games: Vec<(String, usize)>,
    /// Mode 1: this machine sends the turns nobody pressed (Proceed).
    pub leader: bool,
    pub host: bool,
    /// Catching up (late join, resync): states only, no presentation.
    pub quiet: bool,
    /// Mode 2 progress of our players for the shared stream: `(local
    /// index, event bytes)`.
    pub events: Vec<(usize, Vec<u8>)>,
    /// Devices that pressed join mid-session.
    pub joins: Vec<Source>,
    /// Pause menu: leave the room / end the session (host).
    pub leave: bool,
    pub end: bool,
}

pub fn mp_mode(m: Mode) -> MpMode {
    match m {
        Mode::Coop => MpMode::Coop,
        Mode::SharedSave => MpMode::SharedSave,
        Mode::OwnGames => MpMode::OwnGames,
    }
}

pub fn net_mode(m: MpMode) -> Mode {
    match m {
        MpMode::Coop => Mode::Coop,
        MpMode::SharedSave => Mode::SharedSave,
        MpMode::OwnGames => Mode::OwnGames,
    }
}

// ------------------------------------------------------------ encoding

fn put_str(v: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    let n = b.len().min(u16::MAX as usize);
    v.extend_from_slice(&(n as u16).to_le_bytes());
    v.extend_from_slice(&b[..n]);
}

struct Rd<'a>(&'a [u8]);

impl Rd<'_> {
    fn u8(&mut self) -> Option<u8> {
        let (&b, r) = self.0.split_first()?;
        self.0 = r;
        Some(b)
    }
    fn str(&mut self) -> Option<String> {
        if self.0.len() < 2 {
            return None;
        }
        let n = u16::from_le_bytes([self.0[0], self.0[1]]) as usize;
        let s = self.0.get(2..2 + n)?;
        self.0 = &self.0[2 + n..];
        String::from_utf8(s.to_vec()).ok()
    }
    fn rest(&mut self) -> &[u8] {
        std::mem::take(&mut self.0)
    }
}

fn deflate(text: &str) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    let _ = e.write_all(text.as_bytes());
    e.finish().unwrap_or_default()
}

fn inflate(b: &[u8]) -> Option<String> {
    let mut d = flate2::read::ZlibDecoder::new(b);
    let mut s = String::new();
    d.read_to_string(&mut s).ok()?;
    Some(s)
}

/// What the host's settings blob carries (modes 1/2: the world and the
/// save everybody starts from; mode 3: the default world only).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionSettings {
    pub world: String,
    /// The host's slot (it writes the result there).
    pub slot: usize,
    /// The save's text (modes 1/2).
    pub save: String,
}

const SETTINGS_TAG: &[u8; 3] = b"BS1";

impl SessionSettings {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = SETTINGS_TAG.to_vec();
        put_str(&mut v, &self.world);
        v.push(self.slot as u8);
        v.extend(deflate(&self.save));
        v
    }

    pub fn decode(b: &[u8]) -> Option<SessionSettings> {
        let rest = b.strip_prefix(SETTINGS_TAG)?;
        let mut r = Rd(rest);
        let world = r.str()?;
        let slot = r.u8()? as usize;
        let save = inflate(r.rest())?;
        Some(SessionSettings { world, slot, save })
    }
}

const INIT: u8 = b'I';
const PROGRESS: u8 = b'P';
/// Payload bytes per own-stream event chunk (≤ `MAX_EVENT`).
const CHUNK: usize = 3500;

/// Mode 3: a player's world and save as own-stream events
/// (`[I, index, count, bytes...]`).
pub fn init_chunks(world: &str, save: &str) -> Vec<Vec<u8>> {
    let mut payload = Vec::new();
    put_str(&mut payload, world);
    payload.extend(deflate(save));
    let parts: Vec<&[u8]> = payload.chunks(CHUNK).collect();
    let n = parts.len().clamp(1, 255);
    parts
        .iter()
        .take(n)
        .enumerate()
        .map(|(i, p)| {
            let mut v = vec![INIT, i as u8, n as u8];
            v.extend_from_slice(p);
            v
        })
        .collect()
}

fn init_decode(chunks: &[Vec<u8>]) -> Option<(String, String)> {
    let mut payload = Vec::new();
    for c in chunks {
        payload.extend_from_slice(c.get(3..)?);
    }
    let mut r = Rd(&payload);
    let world = r.str()?;
    let save = inflate(r.rest())?;
    Some((world, save))
}

/// Mode 2: progress one player made in the shared save.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Progress {
    /// `stem` won under the map `parent` (`None`: no parent map;
    /// `cp_win`: the level's custom parent is `<win>`).
    Win {
        world: String,
        stem: String,
        parent: Option<String>,
        bonus: bool,
        cp_win: bool,
    },
    /// A world flag (`end`, `done`).
    Flag { world: String, key: String },
    /// LEVEL IS X stored under the parent map (empty = reverted).
    Convert {
        world: String,
        parent: String,
        stem: String,
        objs: Vec<String>,
    },
}

impl Progress {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = vec![PROGRESS];
        match self {
            Progress::Win {
                world,
                stem,
                parent,
                bonus,
                cp_win,
            } => {
                v.push(1);
                put_str(&mut v, world);
                put_str(&mut v, stem);
                put_str(&mut v, parent.as_deref().unwrap_or(""));
                v.push(*bonus as u8 | (*cp_win as u8) << 1);
            }
            Progress::Flag { world, key } => {
                v.push(2);
                put_str(&mut v, world);
                put_str(&mut v, key);
            }
            Progress::Convert {
                world,
                parent,
                stem,
                objs,
            } => {
                v.push(3);
                put_str(&mut v, world);
                put_str(&mut v, parent);
                put_str(&mut v, stem);
                v.push(objs.len().min(255) as u8);
                for o in objs.iter().take(255) {
                    put_str(&mut v, o);
                }
            }
        }
        v
    }

    pub fn decode(b: &[u8]) -> Option<Progress> {
        let mut r = Rd(b);
        if r.u8()? != PROGRESS {
            return None;
        }
        Some(match r.u8()? {
            1 => {
                let world = r.str()?;
                let stem = r.str()?;
                let parent = r.str()?;
                let f = r.u8()?;
                Progress::Win {
                    world,
                    stem,
                    parent: (!parent.is_empty()).then_some(parent),
                    bonus: f & 1 != 0,
                    cp_win: f & 2 != 0,
                }
            }
            2 => Progress::Flag {
                world: r.str()?,
                key: r.str()?,
            },
            3 => {
                let world = r.str()?;
                let parent = r.str()?;
                let stem = r.str()?;
                let n = r.u8()?;
                let objs = (0..n).map(|_| r.str()).collect::<Option<Vec<_>>>()?;
                Progress::Convert {
                    world,
                    parent,
                    stem,
                    objs,
                }
            }
            _ => return None,
        })
    }

    /// Merges into a save (idempotent).
    pub fn merge(&self, world: &WorldInfo, slot: &mut Slot) {
        match self {
            Progress::Win {
                world: w,
                stem,
                parent,
                bonus,
                cp_win,
            } => {
                if *w != world.id {
                    return;
                }
                match parent {
                    Some(p) if !cp_win => {
                        if win_progress(world, slot, stem, p, *bonus).is_none() {
                            slot.set_level_state(w, stem, 3);
                        }
                    }
                    _ => slot.set_level_state(w, stem, 3),
                }
            }
            Progress::Flag { world: w, key } => slot.set_flag(w, key, true),
            Progress::Convert {
                world: w,
                parent,
                stem,
                objs,
            } => slot.set_conversion(w, parent, stem, objs),
        }
    }
}

/// Identity of the imported game data the sim reads (`NetConfig::content`):
/// the object definitions and every level of the install's worlds.
pub fn content_hash(data: &Path) -> String {
    let mut h = FNV0;
    if let Ok(b) = std::fs::read(data.join("values.lua")) {
        h = fnv(h, &b);
    }
    let mut worlds: Vec<_> = std::fs::read_dir(data.join("Worlds"))
        .map(|r| r.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    worlds.sort();
    for w in worlds.iter().filter(|w| w.is_dir()) {
        let mut files: Vec<_> = std::fs::read_dir(w)
            .map(|r| r.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        files.sort();
        for f in files {
            let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("");
            let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(ext, "l" | "ld") || name == "world_data.txt" {
                h = fnv(h, name.as_bytes());
                if let Ok(b) = std::fs::read(&f) {
                    h = fnv(h, &b);
                }
            }
        }
    }
    format!("{h:016x}")
}

// ------------------------------------------------------------- replica

struct PState {
    info: PlayerInfo,
    active: bool,
    /// Mode 3: chunks of the player's save so far.
    init: Vec<Vec<u8>>,
    /// Mode 2: shared-stream entries merged into the player's save copy.
    cursor: u32,
}

/// The replicated game besides the views: settings, the shared save
/// (mode 2), the roster.
pub struct NetGame {
    pub mode: MpMode,
    pub settings: SessionSettings,
    world: Option<WorldInfo>,
    /// Mode 2: the session's save + every sequenced progress event.
    shared: Slot,
    /// Mode 2: `(shared-stream index, event)`.
    events: Vec<(u32, Progress)>,
    players: BTreeMap<PlayerId, PState>,
    me: Option<MachineId>,
    /// Own-stream entries made live on this machine (mode 2 progress is
    /// sent for these only, never for replayed ones).
    pub made: BTreeSet<(StreamId, u32)>,
    /// The host writes the shared save (modes 1/2) to this slot file.
    host_disk: Option<usize>,
    pub log: Vec<String>,
}

impl NetGame {
    /// A fresh replica for a session; mode 1 creates the co-op instance.
    pub fn new(
        mode: MpMode,
        settings: SessionSettings,
        me: Option<MachineId>,
        host: bool,
        m: &mut Multi,
        cx: &mut Cx,
    ) -> NetGame {
        let world = WorldInfo::open(cx.data, &settings.world);
        let shared = Slot::parse(&settings.save);
        let host_disk = (host && mode != MpMode::OwnGames).then_some(settings.slot);
        let mut g = NetGame {
            mode,
            settings,
            world,
            shared,
            events: Vec::new(),
            players: BTreeMap::new(),
            me,
            made: BTreeSet::new(),
            host_disk,
            log: Vec::new(),
        };
        if mode == MpMode::Coop {
            match g.start_view(Vec::new(), g.shared.clone(), g.host_disk, cx) {
                Ok(v) => m.views.push(v),
                Err(e) => g.log.push(format!("error: {e}")),
            }
        }
        g
    }

    fn start_view(
        &self,
        seats: Vec<usize>,
        mut save: Slot,
        disk: Option<usize>,
        cx: &mut Cx,
    ) -> Result<View, String> {
        let w = WorldInfo::open(cx.data, &self.settings.world)
            .ok_or_else(|| format!("world {} is not installed here", self.settings.world))?;
        Self::view_in(w, seats, &mut save, disk, cx).map(|p| View::with_mem(p.1, p.0, save, disk))
    }

    fn view_in(
        w: WorldInfo,
        seats: Vec<usize>,
        save: &mut Slot,
        disk: Option<usize>,
        cx: &mut Cx,
    ) -> Result<(Play, Vec<usize>), String> {
        let mut env = Env {
            data_dir: cx.data,
            objects: cx.objects,
            atlas: &mut cx.gfx.atlas,
        };
        let p = Play::start_with(&mut env, w, disk.unwrap_or(0), save, None)?;
        Ok((p, seats))
    }

    fn view_of(m: &Multi, seat: usize) -> Option<usize> {
        m.views.iter().position(|v| v.seats.first() == Some(&seat))
    }

    fn insert_view(m: &mut Multi, v: View) {
        let id = v.seats.first().copied().unwrap_or(0);
        let at = m
            .views
            .iter()
            .position(|w| w.seats.first().is_some_and(|s| *s > id))
            .unwrap_or(m.views.len());
        m.views.insert(at, v);
    }

    /// Is `p` one of this machine's players?
    fn local(&self, p: &PlayerInfo) -> bool {
        Some(p.machine) == self.me
    }

    fn joined(&mut self, m: &mut Multi, cx: &mut Cx, index: u32, info: PlayerInfo) {
        let id = info.player.0 as usize;
        let local = self.local(&info);
        let (source, name) = {
            let n = m.net.as_mut();
            let src = n
                .as_ref()
                .and_then(|n| n.sources.get(info.local_index as usize).copied())
                .flatten();
            if local && let Some(n) = n {
                n.local.insert(id, info.local_index as usize);
            }
            (src.filter(|_| local), info.name.clone())
        };
        m.players
            .add_net(id, source, !local, &name, id % MAX_PLAYERS);
        self.log.push(format!(
            "joined p{} {} {}{}",
            id + 1,
            info.machine,
            name,
            if local { " (here)" } else { "" }
        ));
        match self.mode {
            MpMode::Coop => {
                if let Some(v) = m.views.first_mut() {
                    v.seats.push(id);
                    v.seats.sort();
                }
            }
            MpMode::SharedSave => {
                let save = self.shared.clone();
                match self.start_view(vec![id], save, None, cx) {
                    Ok(v) => Self::insert_view(m, v),
                    Err(e) => self.log.push(format!("error: {e}")),
                }
            }
            MpMode::OwnGames => {}
        }
        self.players.insert(
            info.player,
            PState {
                info,
                active: true,
                init: Vec::new(),
                cursor: index,
            },
        );
    }

    fn left(&mut self, m: &mut Multi, cx: &mut Cx, p: PlayerId) {
        let id = p.0 as usize;
        if let Some(ps) = self.players.get_mut(&p) {
            ps.active = false;
        }
        m.players.remove(id);
        if let Some(n) = &mut m.net {
            n.local.remove(&id);
        }
        if m.focus == Some(id) {
            m.focus = None;
        }
        match self.mode {
            MpMode::Coop => {
                if let Some(v) = m.views.first_mut() {
                    v.seats.retain(|s| *s != id);
                }
            }
            _ => {
                if let Some(vi) = Self::view_of(m, id) {
                    m.save_one(vi, cx);
                    m.views[vi].hidden = true;
                }
            }
        }
        self.log.push(format!("left p{}", id + 1));
    }

    /// Mode 3: a chunk of a player's starting save.
    fn init(&mut self, m: &mut Multi, cx: &mut Cx, p: PlayerId, data: &[u8]) {
        let Some(ps) = self.players.get_mut(&p) else {
            return;
        };
        if data.len() < 3 || data[0] != INIT || data[1] as usize != ps.init.len() {
            return;
        }
        let count = data[2] as usize;
        ps.init.push(data.to_vec());
        if ps.init.len() < count {
            return;
        }
        let local = Some(ps.info.machine) == self.me;
        let li = ps.info.local_index as usize;
        let id = p.0 as usize;
        let Some((world, save)) = init_decode(&ps.init) else {
            self.log.push(format!("error: bad save of p{}", id + 1));
            return;
        };
        let disk = if local {
            m.net
                .as_ref()
                .and_then(|n| n.local_games.get(li))
                .map(|g| g.1)
        } else {
            None
        };
        let Some(w) = WorldInfo::open(cx.data, &world) else {
            self.log.push(format!(
                "error: world {world} of p{} is not installed",
                id + 1
            ));
            return;
        };
        let mut slot = Slot::parse(&save);
        match Self::view_in(w, vec![id], &mut slot, disk, cx) {
            Ok((play, seats)) => {
                let v = View::with_mem(seats, play, slot, disk);
                Self::insert_view(m, v);
                if let Some(s) = m.players.by_id_mut(id) {
                    s.world = world.clone();
                }
                self.log.push(format!("p{} plays {world}", id + 1));
            }
            Err(e) => self.log.push(format!("error: {e}")),
        }
    }

    /// Mode 2: merges the shared events below `sync` into a player's save.
    fn catch_up(&mut self, m: &mut Multi, p: PlayerId, vi: usize, sync: u32) {
        let Some(ps) = self.players.get_mut(&p) else {
            return;
        };
        let (Some(w), Some(save)) = (&self.world, m.views[vi].mem.as_mut()) else {
            return;
        };
        for (_, ev) in self
            .events
            .iter()
            .filter(|(i, _)| *i >= ps.cursor && *i < sync)
        {
            ev.merge(w, save);
        }
        ps.cursor = ps.cursor.max(sync);
    }

    /// Mode 2: progress events in the shared save so far.
    pub fn events_len(&self) -> usize {
        self.events.len()
    }

    /// Mode 2: shared progress events not merged into the player's save.
    pub fn unmerged(&self, p: PlayerId) -> bool {
        let Some(ps) = self.players.get(&p) else {
            return false;
        };
        self.events.last().is_some_and(|(i, _)| *i >= ps.cursor)
    }

    fn turn(
        &mut self,
        m: &mut Multi,
        cx: &mut Cx,
        stream: StreamId,
        index: u32,
        e: &Entry,
        vi: usize,
    ) {
        let Some(turn) = Turn::decode(&e.data) else {
            self.log.push(format!("bad turn from {}", e.player));
            return;
        };
        let seat = e.player.0 as usize;
        // Mode 2: what the turn may record, for the shared save.
        let live = self.made.remove(&(stream, index)) && !m.net.as_ref().is_some_and(|n| n.quiet);
        let pre = (self.mode == MpMode::SharedSave && live).then(|| {
            let p = &m.views[vi].play;
            let pending = match &p.pending {
                Some(Pending::Convert(o)) => Some(Some(o.clone())),
                Some(Pending::PackEnd) => Some(None),
                _ => None,
            };
            (
                p.world.id.clone(),
                p.cur.stem.clone(),
                p.tree.parent().map(str::to_string),
                p.cur.meta.custom_parent.clone(),
                pending,
            )
        });
        let Some(a) = m.apply_now(vi, Some(seat), turn.clone(), cx) else {
            return;
        };
        let Some((world, stem, parent, cp, pending)) = pre else {
            return;
        };
        if a.ignored {
            return;
        }
        let mut out = Vec::new();
        for fx in &a.fx {
            match fx {
                Fx::Won => out.push(Progress::Win {
                    world: world.clone(),
                    stem: stem.clone(),
                    parent: parent.clone(),
                    bonus: m.views[vi].play.cur.level.bonus(),
                    cp_win: cp == "<win>",
                }),
                Fx::Turn(o) if o.ended || o.all_done => out.push(Progress::Flag {
                    world: world.clone(),
                    key: if o.ended { "end" } else { "done" }.into(),
                }),
                _ => {}
            }
        }
        if turn == Turn::Proceed {
            match pending {
                Some(None) => out.push(Progress::Flag {
                    world: world.clone(),
                    key: "end".into(),
                }),
                Some(Some(objs)) => {
                    if let Some(parent) = parent {
                        let objs = if objs.iter().any(|o| o == "revert") {
                            Vec::new()
                        } else {
                            objs
                        };
                        out.push(Progress::Convert {
                            world,
                            parent,
                            stem,
                            objs,
                        });
                    }
                }
                None => {}
            }
        }
        if let Some(n) = &mut m.net
            && let Some(&li) = n.local.get(&seat)
        {
            for p in out {
                self.log.push(format!("progress p{} {p:?}", seat + 1));
                n.events.push((li, p.encode()));
            }
        }
    }

    /// One stream entry (see [`Replica::apply`]).
    pub fn apply(&mut self, m: &mut Multi, cx: &mut Cx, stream: StreamId, index: u32, e: &Entry) {
        match stream {
            StreamId::Shared => match e.kind {
                EntryKind::Joined => {
                    if let Some(info) = e.player_info() {
                        self.joined(m, cx, index, info);
                    }
                }
                EntryKind::Left => self.left(m, cx, e.player),
                EntryKind::Input => {
                    if self.mode == MpMode::Coop && !m.views.is_empty() {
                        self.turn(m, cx, stream, index, e, 0);
                    }
                }
                EntryKind::Event => {
                    let Some(p) = Progress::decode(&e.data) else {
                        return;
                    };
                    if let Some(w) = &self.world {
                        p.merge(w, &mut self.shared);
                        if let Some(d) = self.host_disk {
                            let s = crate::session::saves::slot_mut(cx.profile, &mut m.extra, d);
                            p.merge(w, s);
                            crate::session::saves::save(cx.profile, &m.extra, d);
                        }
                    }
                    self.log.push(format!("shared {p:?}"));
                    self.events.push((index, p));
                }
            },
            StreamId::Player(p) => match e.kind {
                EntryKind::Event => {
                    if self.mode == MpMode::OwnGames {
                        self.init(m, cx, p, &e.data);
                    }
                }
                EntryKind::Input => {
                    let Some(vi) = Self::view_of(m, p.0 as usize) else {
                        return;
                    };
                    if self.mode == MpMode::SharedSave {
                        self.catch_up(m, p, vi, e.sync);
                    }
                    self.turn(m, cx, stream, index, e, vi);
                }
                _ => {}
            },
        }
    }

    /// See [`Replica::state_hash`].
    pub fn state_hash(&self, m: &Multi, stream: StreamId) -> u64 {
        let mut h = FNV0;
        for (p, s) in &self.players {
            h = fnv(h, &[p.0, s.active as u8]);
        }
        match stream {
            StreamId::Shared => match self.mode {
                MpMode::Coop => {
                    if let Some(v) = m.views.first() {
                        h = fnv(h, &v.play.state_hash().to_le_bytes());
                    }
                }
                MpMode::SharedSave => h = fnv(h, self.shared.to_text().as_bytes()),
                MpMode::OwnGames => {}
            },
            StreamId::Player(p) => {
                h = fnv(h, &[p.0]);
                match Self::view_of(m, p.0 as usize) {
                    Some(vi) => {
                        let v = &m.views[vi];
                        h = fnv(h, &v.play.state_hash().to_le_bytes());
                        if let Some(s) = &v.mem {
                            h = fnv(h, s.to_text().as_bytes());
                        }
                    }
                    None => {
                        let n = self.players.get(&p).map_or(0, |s| s.init.len());
                        h = fnv(h, &[n as u8]);
                    }
                }
            }
        }
        h
    }
}

/// The replica `NetSession::pump` drives: the game state of a frame.
pub struct Rep<'r, 'c> {
    pub g: &'r mut NetGame,
    pub m: &'r mut Multi,
    pub cx: &'r mut Cx<'c>,
}

impl Replica for Rep<'_, '_> {
    fn apply(&mut self, stream: StreamId, index: u32, entry: &Entry) {
        self.g.apply(self.m, self.cx, stream, index, entry);
    }

    fn state_hash(&mut self, stream: StreamId) -> u64 {
        self.g.state_hash(self.m, stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip() {
        let s = SessionSettings {
            world: "baba".into(),
            slot: 7,
            save: "[baba]\n0level=3\n".repeat(400),
        };
        let b = s.encode();
        assert!(b.len() < 400, "deflated: {}", b.len());
        assert_eq!(SessionSettings::decode(&b), Some(s));
        assert_eq!(SessionSettings::decode(b"xx"), None);
    }

    #[test]
    fn init_chunks_round_trip() {
        // Incompressible enough to need several chunks.
        let mut save = String::new();
        let mut x: u64 = 1;
        for i in 0..3000 {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            save.push_str(&format!("k{i}={:x}\n", x >> 20));
        }
        let c = init_chunks("new_adv", &save);
        assert!(c.len() > 1);
        assert!(c.iter().all(|c| c.len() <= baba_net::wire::MAX_EVENT));
        assert_eq!(init_decode(&c), Some(("new_adv".into(), save)));
    }

    #[test]
    fn progress_round_trip() {
        let all = [
            Progress::Win {
                world: "baba".into(),
                stem: "1level".into(),
                parent: Some("106level".into()),
                bonus: true,
                cp_win: false,
            },
            Progress::Win {
                world: "baba".into(),
                stem: "0level".into(),
                parent: None,
                bonus: false,
                cp_win: true,
            },
            Progress::Flag {
                world: "museum".into(),
                key: "done".into(),
            },
            Progress::Convert {
                world: "baba".into(),
                parent: "106level".into(),
                stem: "5level".into(),
                objs: vec!["rock".into(), "keke".into()],
            },
        ];
        for p in all {
            assert_eq!(Progress::decode(&p.encode()), Some(p));
        }
        assert_eq!(Progress::decode(&[PROGRESS, 9]), None);
    }

    #[test]
    fn modes_map() {
        for m in [MpMode::Coop, MpMode::SharedSave, MpMode::OwnGames] {
            assert_eq!(mp_mode(net_mode(m)), m);
        }
    }
}
