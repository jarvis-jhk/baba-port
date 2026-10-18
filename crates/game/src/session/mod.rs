//! Sessions: the turn path every instance change goes through, and local
//! multiplayer in the three modes (`docs/ARCHITECTURE.md`, "Multiplayer
//! modes"; `docs/game-flow.md`, "Multiplayer").
//!
//! | module | what |
//! |---|---|
//! | [`turn`] | [`Turn`]: one instance-changing action, byte encoding (≤ 64 bytes) |
//! | [`apply`] | `Play::apply(turn)` → presentation effects ([`Fx`]); `Play::state_hash` |
//! | [`saves`] | slots 0–11 (the profile's three + nine port-only extra slots) |
//! | [`multi`] | a running local multiplayer session: seats, instances, views, per-view presentation, audio mixing |
//! | [`online`] | online play (M7): the session as a `baba_net::Replica`, session settings, mode-2 shared progress |
//!
//! Single player is the same path with one instance (`App::apply_turn`).
//! For M7 (`docs/net.md`) the online replica maps streams onto this:
//! mode 1 = one instance fed by the host-sequenced shared stream, modes
//! 2/3 = one instance per player stream; `Turn::encode` / `decode` are the
//! entry bytes and `Play::state_hash` (plus the slot text in mode 2) the
//! per-stream hash. Presentation (transitions, title cards, the victory
//! delay, menus) only decides when the *local* machine submits a turn.

pub mod apply;
pub mod multi;
pub mod online;
pub mod saves;
pub mod turn;

pub use apply::{Applied, Fx, Pending};
pub use multi::Multi;
pub use turn::Turn;

/// The three multiplayer modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MpMode {
    /// One level instance; every player's input is a turn, in arrival
    /// order; undo, restart and map navigation are shared.
    Coop,
    /// One save; each player has an own map cursor and level instance;
    /// progress reaches everyone's map at once.
    SharedSave,
    /// Everyone plays an own save, side by side.
    OwnGames,
}

#[cfg(test)]
mod online_tests;
#[cfg(test)]
mod tests;
