//! # baba-net
//!
//! Online play for baba-port over [Warpzone](https://github.com/jarvis-jhk/warpzone)
//! peer channels (reliable, ordered). Baba Is You is turn-based, so there is
//! no rollback: machines exchange **turn streams** and every machine replays
//! every stream with the same deterministic sim. Game-agnostic: inputs and
//! events are opaque bytes, the sim is a [`Replica`]. See `docs/net.md`.
//!
//! | Module | What |
//! |---|---|
//! | [`session`] | [`NetSession`]: rooms, lobby, sessions, streams, late join, host migration, desync detection |
//! | [`wire`] | ids, modes, stream entries, messages, framing |
//! | [`link`] | the [`Link`] backend trait (Warpzone client), link conditioner |
//! | [`mem`] | [`MemNet`]: an in-process Warpzone stand-in for tests |
//! | [`toy`] | a deterministic toy puzzle game and bots (tests, `netsim`) |
//! | [`harness`] | a bot-driven machine following a plan (tests, `netsim`) |
//! | [`tls`] | rustls crypto provider (pure Rust on Android / PortMaster) |

pub mod harness;
pub mod link;
pub mod mem;
pub mod session;
pub mod tls;
pub mod toy;
pub mod wire;

pub use link::{Link, LinkSim, TrafficStats};
pub use mem::{MemClient, MemNet};
pub use session::{
    DesyncPolicy, GAME_ID, LaunchAction, MAX_MACHINES, NetConfig, NetError, NetEvent, NetSession,
    NetStats, Pending, PlayerState, REJECT_CONTENT, REJECT_VERSION, Replica,
};
pub use warpzone_core;
pub use wire::{
    DesyncReport, EndReport, Entry, EntryKind, LobbyState, MAX_INPUT, MAX_PLAYERS, MachineId,
    MachineView, Mode, Phase, PlayerId, PlayerInfo, SessionStart, StreamId,
};
