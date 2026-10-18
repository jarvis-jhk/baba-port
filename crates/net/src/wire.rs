//! Everything baba-net puts on a Warpzone peer channel.
//!
//! One channel, [`CH_SESSION`], reliable and ordered per peer: every
//! [`Msg`] is bincode (varint integers, 256 KiB limit), split into chunks of
//! at most [`MAX_DATAGRAM`] bytes (`[more:u8] bytes`). Field order is the
//! format: append new fields only at the end and bump [`PROTOCOL`] when
//! changing anything.

use bincode::Options;
use serde::{Deserialize, Serialize};

pub use warpzone_core::protocol::MAX_DATAGRAM;

/// Wire protocol version; machines with another one are rejected in `Hello`.
pub const PROTOCOL: u16 = 2;
/// The reliable, ordered session channel.
pub const CH_SESSION: u8 = 3;
/// Most players in one session (any mix of machines).
pub const MAX_PLAYERS: usize = 12;
/// Most local players one machine may announce.
pub const MAX_LOCAL_PLAYERS: usize = 12;
/// Size limit of one turn input ([`EntryKind::Input`]).
pub const MAX_INPUT: usize = 64;
/// Size limit of one shared event ([`EntryKind::Event`]).
pub const MAX_EVENT: usize = 4096;
/// Size limit of the host's settings blob (baba-port puts the shared save
/// in it, deflated).
pub const MAX_SETTINGS: usize = 32 * 1024;
/// Size limit of one decoded message.
pub const MAX_MESSAGE: u64 = 256 * 1024;
/// Entries per `Append` message when streaming a long log (late join).
pub const APPEND_BATCH: usize = 256;

/// A machine (one game process, one Warpzone room member). The first host is
/// machine 0; the others get 1, 2, ... in join order. Ids are never reused
/// within a session and survive host migration.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
pub struct MachineId(pub u8);

impl std::fmt::Display for MachineId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "m{}", self.0)
    }
}

/// A player of a session: 0, 1, 2, ... in the order the host admitted them,
/// never reused within the session (a player who leaves and comes back is a
/// new player). At most [`MAX_PLAYERS`] are active at once.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
pub struct PlayerId(pub u8);

impl std::fmt::Display for PlayerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "p{}", self.0)
    }
}

/// The three multiplayer modes (docs/ARCHITECTURE.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Mode {
    /// 1: one level instance; every player's input is one turn, in the order
    /// the host sequences them (all in the shared stream).
    #[default]
    Coop,
    /// 2: one shared save, one instance per player (own stream each); save
    /// updates are host-sequenced events in the shared stream.
    SharedSave,
    /// 3: everyone in their own game (own stream each, own save).
    OwnGames,
}

impl Mode {
    /// Does every player own a stream of their own turns?
    pub fn per_player(self) -> bool {
        !matches!(self, Mode::Coop)
    }

    pub fn label(self) -> &'static str {
        match self {
            Mode::Coop => "co-op",
            Mode::SharedSave => "shared save",
            Mode::OwnGames => "own games",
        }
    }
}

/// A turn stream. Every machine keeps a full copy of every stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum StreamId {
    /// Sequenced by the host: roster changes (all modes), every turn (mode
    /// 1), shared-save events (mode 2).
    Shared,
    /// Appended by the machine that owns the player (modes 2 and 3).
    Player(PlayerId),
}

impl std::fmt::Display for StreamId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StreamId::Shared => write!(f, "shared"),
            StreamId::Player(p) => write!(f, "{p}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EntryKind {
    /// One turn of `player` (opaque input bytes, ≤ [`MAX_INPUT`]).
    Input,
    /// A game event of `player`, opaque, ≤ [`MAX_EVENT`]: in the shared
    /// stream sequenced by the host (mode 2: "level cleared" → save update),
    /// in a player stream appended by its owner (game data that is not a
    /// turn, e.g. the player's starting save in mode 3).
    Event,
    /// `player` joins from here on; `data` = [`PlayerInfo`] (shared stream).
    Joined,
    /// `player` left; `data` = the final length of their own stream as
    /// little-endian u32 (modes 2/3), else empty (shared stream).
    Left,
}

/// One entry of a stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub kind: EntryKind,
    pub player: PlayerId,
    /// The player's own counter of submissions to the shared stream (used by
    /// the host to drop duplicates after host migration); 0 for entries the
    /// host generates and for own-stream entries.
    pub seq: u32,
    /// Own-stream entries (modes 2/3): how many shared-stream entries the
    /// owner had when it made this turn. Replaying machines give the
    /// player's instance the shared state of exactly that point
    /// (docs/net.md "Mode 2"). 0 elsewhere.
    pub sync: u32,
    pub data: Vec<u8>,
}

impl Entry {
    /// The joining player's description of a [`EntryKind::Joined`] entry.
    pub fn player_info(&self) -> Option<PlayerInfo> {
        (self.kind == EntryKind::Joined)
            .then(|| decode(&self.data))
            .flatten()
    }

    /// The final own-stream length of a [`EntryKind::Left`] entry.
    pub fn final_len(&self) -> Option<u32> {
        (self.kind == EntryKind::Left && self.data.len() == 4)
            .then(|| u32::from_le_bytes(self.data[..4].try_into().unwrap()))
    }
}

/// A player as the shared stream announces them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerInfo {
    pub player: PlayerId,
    pub machine: MachineId,
    /// Index into that machine's local player list.
    pub local_index: u8,
    pub name: String,
}

/// A machine as the lobby shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineView {
    pub machine: MachineId,
    /// Warpzone account id (0 = unknown / in-memory).
    pub account: i64,
    /// Warpzone user name.
    pub name: String,
    /// The machine's local players (display names), in local order.
    pub players: Vec<String>,
    /// Session players of this machine, by local index (`None` = not
    /// admitted: the session is full).
    pub admitted: Vec<Option<PlayerId>>,
    /// No local players, watches (always true for a machine without players).
    pub spectator: bool,
    pub ready: bool,
    /// Has the session state (Start + streams) and replays it.
    pub synced: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Phase {
    #[default]
    Lobby,
    /// A session runs.
    Session(u32),
    /// The host is gone; the machines regroup in a new room.
    Migrating,
}

/// The host's lobby, sent to every guest whenever it changes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LobbyState {
    pub phase: Phase,
    pub mode: Mode,
    /// The host's settings blob (opaque: world, level pack, options).
    pub settings: Vec<u8>,
    pub machines: Vec<MachineView>,
    /// Which machine hosts (0 until a migration).
    pub host: MachineId,
    /// Warpzone room id, join code (private rooms) and invite link.
    pub room: String,
    pub code: Option<String>,
    pub invite: String,
}

impl LobbyState {
    pub fn machine(&self, m: MachineId) -> Option<&MachineView> {
        self.machines.iter().find(|v| v.machine == m)
    }

    /// Players announced over all machines.
    pub fn player_count(&self) -> usize {
        self.machines.iter().map(|m| m.players.len()).sum()
    }
}

/// What every machine needs to build a session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionStart {
    pub session: u32,
    pub mode: Mode,
    /// For the sim's seeded PRNG (rules randomness).
    pub seed: u64,
    pub settings: Vec<u8>,
    /// Report a state hash every `hash_interval` entries of a stream.
    pub hash_interval: u32,
}

/// Machines disagreeing on a state hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesyncReport {
    pub session: u32,
    pub stream: StreamId,
    /// The hash is of the state after entries `0..index`.
    pub index: u32,
    /// The value most machines agree on (ties: the host's).
    pub majority: u64,
    /// Every machine's hash.
    pub hashes: Vec<(MachineId, u64)>,
}

impl DesyncReport {
    /// Machines whose hash differs from the majority.
    pub fn desynced(&self) -> Vec<MachineId> {
        self.hashes
            .iter()
            .filter(|(_, h)| *h != self.majority)
            .map(|(m, _)| *m)
            .collect()
    }
}

/// The end of a session: final lengths and every machine's final hashes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndReport {
    pub session: u32,
    pub lengths: Vec<(StreamId, u32)>,
    /// Final hash of every stream, per machine.
    pub hashes: Vec<(MachineId, Vec<(StreamId, u64)>)>,
    /// Every machine that reported has the same final hashes.
    pub agreed: bool,
    pub desyncs: Vec<DesyncReport>,
}

/// Rejoining after a host migration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resume {
    /// The room we were in.
    pub old_room: String,
    pub machine: MachineId,
    pub session: Option<u32>,
    /// Our length of every stream.
    pub lengths: Vec<(StreamId, u32)>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Msg {
    // ---------------------------------------------------------- guest -> host
    Hello {
        protocol: u16,
        /// Game build (`NetConfig::build`): must equal the host's.
        build: String,
        /// Imported game content (`NetConfig::content`): must equal the
        /// host's.
        content: String,
        players: Vec<String>,
        spectate: bool,
        ready: bool,
        resume: Option<Resume>,
    },
    SetPlayers(Vec<String>),
    SetReady(bool),
    SetSpectate(bool),
    /// A turn (mode 1) or event for the shared stream.
    Submit {
        session: u32,
        player: PlayerId,
        seq: u32,
        kind: EntryKind,
        data: Vec<u8>,
    },
    /// The state hash after entries `0..index` of `stream`.
    Hash {
        session: u32,
        stream: StreamId,
        index: u32,
        hash: u64,
    },
    /// Answer to `Stop`: all our own-stream entries were sent before this.
    Stopped {
        session: u32,
    },
    /// Final hashes after `End`.
    Final {
        session: u32,
        hashes: Vec<(StreamId, u64)>,
    },
    // ---------------------------------------------------------- host -> guest
    Welcome {
        machine: MachineId,
        /// The host's machine id.
        host: MachineId,
    },
    Reject {
        reason: String,
    },
    Lobby(LobbyState),
    Start(SessionStart),
    /// Send us the entries of `stream` from `from` on (migration).
    Want {
        stream: StreamId,
        from: u32,
    },
    /// Stop making turns, answer `Stopped` (the session is ending).
    Stop {
        session: u32,
    },
    /// Final lengths: replay up to them and answer `Final`.
    End {
        session: u32,
        lengths: Vec<(StreamId, u32)>,
    },
    Ended(EndReport),
    /// The session was aborted (host action, desync policy).
    Abort {
        session: u32,
        reason: String,
    },
    Desync(DesyncReport),
    // ---------------------------------------------------------- both ways
    /// Entries `first..` of `stream`. Host → guest: any stream. Guest → host:
    /// the guest's own players' streams (modes 2/3) and migration tails.
    Append {
        session: u32,
        stream: StreamId,
        first: u32,
        entries: Vec<Entry>,
    },
    /// Game data outside the session streams (chat, cursors), forwarded by
    /// the host; `to = None` = everybody.
    App {
        from: MachineId,
        to: Option<MachineId>,
        data: Vec<u8>,
    },
}

fn opts() -> impl Options {
    bincode::DefaultOptions::new().with_limit(MAX_MESSAGE)
}

pub fn encode<T: Serialize>(v: &T) -> Vec<u8> {
    opts().serialize(v).expect("serialisable")
}

pub fn decode<'a, T: Deserialize<'a>>(b: &'a [u8]) -> Option<T> {
    opts().deserialize(b).ok()
}

/// Split an encoded message into chunks: `[more:u8] bytes`.
pub fn chunk(msg: &[u8]) -> Vec<Vec<u8>> {
    let n = MAX_DATAGRAM - 1;
    let parts: Vec<&[u8]> = if msg.is_empty() {
        vec![&[][..]]
    } else {
        msg.chunks(n).collect()
    };
    let last = parts.len() - 1;
    parts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let mut v = Vec::with_capacity(p.len() + 1);
            v.push(u8::from(i != last));
            v.extend_from_slice(p);
            v
        })
        .collect()
}

/// Reassembles chunks (they arrive in order per peer).
#[derive(Default)]
pub struct Dechunker {
    buf: Vec<u8>,
}

impl Dechunker {
    pub fn push(&mut self, chunk: &[u8]) -> Option<Vec<u8>> {
        let (&more, rest) = chunk.split_first()?;
        self.buf.extend_from_slice(rest);
        if self.buf.len() as u64 > MAX_MESSAGE {
            self.buf.clear();
            return None;
        }
        (more == 0).then(|| std::mem::take(&mut self.buf))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_roundtrip() {
        for len in [0usize, 1, MAX_DATAGRAM - 1, MAX_DATAGRAM, 5000] {
            let msg: Vec<u8> = (0..len).map(|i| i as u8).collect();
            let mut d = Dechunker::default();
            let mut out = None;
            for c in chunk(&msg) {
                assert!(c.len() <= MAX_DATAGRAM);
                assert!(out.is_none());
                out = d.push(&c);
            }
            assert_eq!(out.unwrap(), msg);
        }
    }

    #[test]
    fn msg_roundtrip_and_size() {
        let e = Entry {
            kind: EntryKind::Input,
            player: PlayerId(3),
            seq: 17,
            sync: 0,
            data: vec![2],
        };
        let m = Msg::Append {
            session: 1,
            stream: StreamId::Shared,
            first: 1000,
            entries: vec![e.clone(); 12],
        };
        let b = encode(&m);
        assert!(b.len() < 120, "12 turns: {} bytes", b.len());
        assert_eq!(decode::<Msg>(&b), Some(m));
        assert_eq!(decode::<Msg>(&[0xff, 0xff, 0xff]), None);
        let info = PlayerInfo {
            player: PlayerId(1),
            machine: MachineId(2),
            local_index: 0,
            name: "Baba".into(),
        };
        let j = Entry {
            kind: EntryKind::Joined,
            player: PlayerId(1),
            seq: 0,
            sync: 0,
            data: encode(&info),
        };
        assert_eq!(j.player_info(), Some(info));
        assert_eq!(e.player_info(), None);
        let l = Entry {
            kind: EntryKind::Left,
            player: PlayerId(1),
            seq: 0,
            sync: 0,
            data: 77u32.to_le_bytes().to_vec(),
        };
        assert_eq!(l.final_len(), Some(77));
    }
}
