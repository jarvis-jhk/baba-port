# Online play (`baba-net`)

baba-port plays online over [Warpzone](https://github.com/jarvis-jhk/warpzone)
peer channels, and only over Warpzone (rooms, codes, invite links, room
browser, NAT traversal, relay). It is a **tier-2** Warpzone game with id
`baba-port` (`packaging/warpzone/baba-port.toml`). `warpzone-core` is used
directly from Rust, as a path dependency on the checkout in `ext/warpzone`
(copied from `~/Projects/warpzone`, newer than the commit bopl-port pins;
the root `Cargo.toml` excludes `ext/` so the checkout keeps its own
workspace). Once Warpzone publishes that state, switch to a pinned git
dependency like bopl-port.

Baba Is You is turn-based, so there is **no rollback**: machines exchange
*turn streams* on Warpzone's reliable, ordered channel and every machine
replays every stream with the same deterministic sim (`baba-sim`: integer
only, seeded PRNG, no clock). baba-net does not depend on the sim: inputs and
events are opaque bytes and the sim is a `Replica` (two methods).

The design follows bopl-port's `bopl-net` (rooms, lobby protocol, framing,
in-memory backend, link conditioner, multi-process harness) with the GGRS
parts replaced by streams, and adds late join, host migration and per-stream
desync detection.

## Concepts

* **Machine**: one game process, one Warpzone room member, 0..12 local
  players. Ids `m0, m1, ...` are given by the host (the first host is m0),
  never reused in a session, and survive host migration. Machines without
  players (or with `set_spectate(true)`) are **spectators**: they replay
  everything and take part in desync checks, but make no turns. A room holds
  16 machines (12 playing + spectators).
* **Player**: a session player `p0, p1, ...`, admitted by the host in machine
  order (each machine's local players in their order) up to **12 at once**.
  Ids are never reused: a player who leaves and comes back is a new player.
  Players beyond 12 wait and are admitted when someone leaves.
* **Stream**: an append-only log of `Entry { kind, player, seq, sync, data }`.
  Every machine keeps a full copy of every stream, from entry 0.
  * The **shared stream** is sequenced by the host. It carries the roster
    (`Joined` with `PlayerInfo {player, machine, local_index, name}`,
    `Left` with the final length of the player's own stream), every turn in
    mode 1 (`Input`), and host-sequenced game events (`Event`, mode 2: save
    updates).
  * A **player stream** (modes 2 and 3) holds one player's turns. The
    machine that owns the player appends to it directly (it is the
    authority) and sends it to the host, which relays it to everyone.

### The three modes (docs/ARCHITECTURE.md)

| Mode | Streams | How a turn travels |
|---|---|---|
| 1 `Coop`: one level, everybody drives the YOU objects | shared only | `input()` → `Submit` to the host → appended to the shared stream in the order the host receives it (the host's own players: at once) → `Append` to every machine. |
| 2 `SharedSave`: one save, own map cursor and level each | shared + one per player | `input()` → own stream at once → host → everyone. A cleared level is `event()` → host-sequenced into the shared stream → every machine applies it to the save. |
| 3 `OwnGames`: own save and instances, shown side by side | shared (roster) + one per player | as mode 2, no shared events needed. |

**Mode 2 and the `sync` cursor.** A player-stream entry carries `sync` = the
length of the shared stream on the owner when the turn was made. The
replica must give that player's instance the shared save **as of shared
entry `sync`** (not the newest one), because other machines may have applied
more (or fewer) save events when they replay the turn. `pump` guarantees
shared entries `0..sync` were applied before a player entry; the replica
keeps the save as an ordered list of events (or a per-instance view with its
own cursor) and evaluates "what does this player see" at `sync`, plus the
player's own clears. The toy game (`toy.rs`) does exactly this. Save events
are idempotent (clearing a level twice is one clear) and ordered by the
host, so every machine ends with the same save. The clearing player's own
machine submits the event; the round trip (one host hop) is far shorter
than the level-won transition, so the map looks updated "at once".

**Mode 1 prediction.** Without prediction a guest sees its own move after
one round trip to the host (host players: at once). Optional prediction
never touches the confirmed state: the confirmed replica is built *only*
from the shared stream; the screen may show a throwaway copy = confirmed
state + `pending()` (our submitted, not yet sequenced turns, oldest first),
rebuilt whenever either changes. When a turn is sequenced it leaves
`pending()` and arrives in the stream, possibly after other players' turns,
so the prediction can be wrong for a moment (another player moved first)
and then snaps to the confirmed state; it cannot diverge because nothing
predicted is ever fed back. Baba's levels are small, so cloning a level and
replaying a handful of pending turns per frame is cheap. Recommendation:
predict in co-op only for remote guests with high latency (> 80 ms), and
animate a correction like an undo. `coop_prediction_converges` tests it.

## Lobby and rooms

* `host_public()` (listed), `host_private()` (unlisted: join code and invite
  link only), or `host(HostOptions)`; `RoomHosted` carries the code and
  `warpzone://join/<room>#<secret>` invite link (also in `lobby().code`,
  `lobby().invite`). `join_code(code)`, `join(room, code, secret)`,
  `list_rooms(has_space)` → `RoomList`, `answer_knock` for approve rooms,
  `kick`, `set_room_closed`, `leave()`.
* `handle_launch(args)` acts on `--warpzone-join <room|invite link>`,
  `WARPZONE_JOIN`, `--warpzone-host` / `WARPZONE_HOST=1` (the launcher and
  the website start the game this way; descriptor `[launch]`).
* **Player list**: each machine announces its local players
  (`set_local_players(names)`; trimmed to 16 chars, "Player n" if empty)
  and `set_ready`, `set_spectate`. `lobby()` = `LobbyState { phase, mode,
  settings, machines: [MachineView { machine, account, name, players,
  admitted, spectator, ready, synced }], host, room, code, invite }`, sent by
  the host on every change (`NetEvent::LobbyChanged`). `admitted[i]` says
  which session player local player `i` is (`None` = waiting for a free
  seat).
* **Mode and settings**: the host's `set_mode(Mode)` and
  `set_settings(bytes)` (≤ 32 KiB since protocol 2, opaque: baba-port puts the world and the deflated starting save there). Both
  are in `SessionStart` and in the lobby. Room browser metadata (`display`
  keys): `players` ("5/12"), `mode`; the game may add `world` through
  `warpzone()`.
* **Version check**: the guest's `Hello` carries the wire protocol,
  `NetConfig::build` (game version + anything that changes the sim) and
  `NetConfig::content` (a hash of the imported Baba Is You data the sim
  reads: object definitions, levels; the game computes it). Any difference
  → `Reject` ("version mismatch: …" / "content mismatch: …",
  `REJECT_VERSION` / `REJECT_CONTENT`), `NetEvent::Rejected`, and the host
  kicks the machine. Warpzone also tags rooms with `game_version` = `build`.
* **Start**: `start_session(seed)` (host): `SessionStart { session, mode,
  seed, settings, hash_interval }` to every machine, then the host appends
  `Joined` entries for every player. `NetEvent::SessionStarted` → build an
  empty replica and `pump` it.
* **Leave**: `leave()`. A machine leaving (or timing out: Warpzone drops a
  silent peer after 15 s) → the host appends `Left` for its players; the
  session goes on. In modes 2/3 `Left` carries the final length of the
  player's stream (turns the host never got are dropped everywhere).
* **Players mid-session**: `set_local_players` during a session admits new
  local players at once (a controller plugged in) and removes dropped ones.

## Late join

A machine that joins a running session gets `Start` and then every stream
**from entry 0** (in `Append` batches of 256), and replays everything
(`pump_limited(replica, n)` spreads the catch-up over frames). Then its
players are admitted. Replay is cheap for Baba (thousands of turns per
second); there is no state snapshot transfer, which keeps the protocol
independent of the sim's state format. A long session (hours, 12 players)
is a few MB of log; a snapshot option can be added later without changing
the stream model (snapshot = replica state at some stream lengths).

## Host migration

Warpzone has no host migration (the host owns the room), so baba-net
regroups in a **new room**. Every machine already has every stream, so the
session continues where it was:

1. Guests notice the host is gone (`RoomClosed`, or `PeerLeft` of the host
   after a crash). With `NetConfig::host_migration` (default) they enter
   `Phase::Migrating` (`NetEvent::HostLost { successor }`); otherwise the
   session ends (`SessionAborted`, `RoomLeft`).
2. The **successor** is the lowest remaining machine id of the last lobby.
   It hosts a public room with entry "approve" and metadata `migrate=<old
   room id>`, `mhost=<its machine id>`; the others poll `list_rooms` and
   join it; the new host approves the knocks of accounts from the old room
   (strangers' knocks go to the game). A successor that does not show up
   within 10 s is skipped (next lowest id). After 40 s
   (`migration_timeout`) a machine gives up (`RoomLeft`).
3. Each returning machine sends `Hello { resume: { old_room, machine,
   session, lengths } }` and keeps its machine id. The new host
   **reconciles** before it sequences anything: for every stream where a
   machine has more than the host, it asks (`Want`) for the tail (streams
   are prefixes of one another, so the longest copy is the truth); owners
   also send the turns they made while there was no host. Then players of
   machines that did not come back (the old host's included) get `Left`,
   and the turns everybody submitted but the old host never sequenced are
   sequenced, duplicates dropped by `(player, seq)`. Reconciling ends when
   every expected machine is back or after 12 s.
4. `NetEvent::HostMigrated { host, room }` everywhere. The new room has a
   new code and invite link (`lobby()`).

While regrouping, modes 2/3 keep playing (own streams are appended locally
and handed over later); mode-1 turns wait in `pending()`. A machine that
comes back after the reconcile ended gets a fresh copy (new machine id,
`SessionStarted` again, its old players were removed).

## Ending a session

`end_session()` (host): `Stop` → every machine stops making turns and
answers `Stopped` (after its last own-stream turns) → `End { lengths }` →
every machine replays up to exactly those lengths and reports its final
hash of every stream (`Final`; `pump` does it) → `Ended(EndReport {
lengths, hashes per machine, agreed, desyncs })` →
`NetEvent::SessionEnded`, everybody is back in the lobby. `abort_session`
ends at once (`SessionAborted`).

## Desync detection

Every `hash_interval` entries (host's `NetConfig::hash_interval`, default 4)
of every stream, `pump` asks the replica for `state_hash(stream)` (a `u64`
of what that stream drives: the co-op level, the shared save, one player's
instance) and reports it to the host (`Hash`). The host compares the reports
of all machines for the same `(stream, index)` (or, for a late joiner
replaying old entries, with its own value), takes the majority (ties: the
host's value), and sends `Desync(DesyncReport { stream, index, majority,
hashes })` to everybody (`NetEvent::Desync`) the first time a machine
disagrees on a stream. `DesyncPolicy::Report` (default) plays on;
`DesyncPolicy::Abort` aborts the session. The final hashes at the end are
compared too (`EndReport::agreed`).

## What the game must provide

* A **replica**: `impl Replica { fn apply(&mut self, stream, index, entry);
  fn state_hash(&mut self, stream) -> u64 }` over the sim: mode 1 one
  `Level`, modes 2/3 one instance per player (created on `Joined`, retired
  on `Left`), mode 2 the shared save plus per-instance save views at
  `sync`. Deterministic bit for bit (CLAUDE.md), and `state_hash` covers
  everything the stream drives (units, rule state, undo stack depth, level
  progress).
* **Input encoding**: one turn = a few bytes (≤ 64): direction, wait, undo,
  restart, map moves, menu actions that change the game. Everything that
  changes the sim must be a turn; nothing else may change it.
* **Events** (mode 2): "level cleared" etc. as bytes, made by the owner of
  the player when its replica sees the clear (`event(local, bytes)`), and
  applied idempotently.
* `NetConfig::build` (version) and `NetConfig::content` (hash of the
  imported game data).
* The loop: `poll()` every frame, handle `next_event()`, `input(local,
  bytes)` for local players that are admitted (`local_player(i)`), `pump()`
  the replica, render. UI: lobby (machines, players, mode, ready, code,
  invite), connection status (`stats().peers`: RTT, relayed), "regrouping…"
  while `is_migrating()`, desync warning.

## API sketch

```rust
use baba_net::{Mode, NetConfig, NetEvent, NetSession, Replica, LaunchAction};

let mut cfg = NetConfig::new(env!("CARGO_PKG_VERSION"));
cfg.content = import.content_hash();          // hex string
cfg.auto_register = Some(os_user_name);       // or ask for a name
let mut net = NetSession::new(cfg)?;          // NetSession::memory(MemNet::client(..), cfg) in tests
match net.handle_launch(&std::env::args().collect::<Vec<_>>()) {
    LaunchAction::Joining { .. } => show_connecting(),
    LaunchAction::HostScreen => net.host_private(),
    LaunchAction::None => {}
}
net.set_local_players(vec!["Baba".into(), "Keke".into()]);
// host: net.set_mode(Mode::SharedSave)?; net.set_settings(blob)?; net.start_session(None)?;

// every frame
net.poll();
while let Some(e) = net.next_event() {
    match e {
        NetEvent::SessionStarted(s) => replica = MyReplica::new(&s),
        NetEvent::LobbyChanged => redraw_lobby(net.lobby()),
        NetEvent::Desync(r) => warn(r),
        NetEvent::SessionEnded(r) => back_to_lobby(r.agreed),
        _ => {}
    }
}
for (i, pad) in pads.iter().enumerate() {
    if let Some(turn) = pad.turn() { let _ = net.input(i, &turn.encode()); }
}
net.pump(&mut replica);
```

Other calls: `register`, `join`, `join_code`, `list_rooms`, `answer_knock`,
`kick`, `set_room_closed`, `set_ready`, `set_spectate`, `all_ready`,
`end_session`, `abort_session`, `send_app` (chat, cursors; `NetEvent::App`),
`session`, `streams`, `stream(id)` (full log), `applied`, `caught_up`,
`players`, `local_player`, `pending`, `accepting_input`, `event`,
`pump_limited`, `report_hash` / `report_final` (games that apply entries
themselves), `stats`, `me`, `is_host`, `is_migrating`, `lobby`, `room`,
`warpzone()` (the raw `warpzone_core::client::Client`). Single-threaded:
call everything from the game thread.

## Wire

One Warpzone channel, `CH_SESSION = 3`, **reliable and ordered**. Every
message is bincode 1 (varint, ≤ 256 KiB) split into chunks of `[more:u8]` +
≤ 1099 bytes. Rooms are a star (a guest talks to the host only), so the host
relays everything. A turn costs about 10 bytes per receiving machine.

| Message | Direction | Meaning |
|---|---|---|
| `Hello {protocol, build, content, players, spectate, ready, resume}` | guest → host | first message; mismatches get `Reject` |
| `SetPlayers`, `SetReady`, `SetSpectate` | guest → host | lobby changes |
| `Submit {session, player, seq, kind, data}` | guest → host | a turn (mode 1) or event for the shared stream |
| `Append {session, stream, first, entries}` | both | entries `first..` of a stream (host → guests: any stream, from the guest's cursor; guest → host: own player streams, migration tails) |
| `Hash {session, stream, index, hash}` | guest → host | state hash after entries `0..index` |
| `Stopped`, `Final {hashes}` | guest → host | session end |
| `Welcome {machine, host}`, `Reject {reason}` | host → guest | our seat |
| `Lobby(LobbyState)`, `Start(SessionStart)` | host → guests | lobby, session start / fresh copy |
| `Want {stream, from}` | host → guest | send me your entries from `from` (migration, gap repair) |
| `Stop`, `End {lengths}`, `Ended(EndReport)`, `Abort` | host → guests | session end |
| `Desync(DesyncReport)` | host → guests | machines disagree |
| `App {from, to, data}` | any → any via host | game data outside the streams |

The host keeps per-machine cursors (`sent`): each machine gets exactly the
entries it lacks, so a machine that was behind (late join, regrouping)
never sees a gap.

## Link simulation and relay

Everything a machine sends can be delayed, jittered and "lost":
`NetConfig::link_sim` or `BABA_NET_DELAY_MS` (one way), `BABA_NET_JITTER_MS`
(±), `BABA_NET_LOSS` (percent). The channel is reliable, so a lost
transmission arrives one retransmission timeout later (max(3 × delay,
60 ms)) and everything behind it waits (head-of-line blocking): what loss
does to a reliable stream. Every hop adds its own delay (guest → host →
guest = two). `BABA_NET_FORCE_RELAY=1` / `NetConfig::force_relay` routes
every Warpzone connection through the server relay.

## Testing

```sh
source env.sh
cargo test -p baba-net                     # unit tests + in-process sessions (MemNet)
scripts/net/dev-server.sh                  # local warpzone-server, http://127.0.0.1:3036, udp 3516
scripts/net/dev-server.sh --check          # warpzone-server check-games packaging/warpzone
export WARPZONE_SERVER=http://127.0.0.1:3036
cargo run -p baba-net --example netsim -- run --locals 4x3 --mode coop --secs 20
cargo run -p baba-net --example netsim -- run --locals 2,4,6 --mode shared --spectators 1 --delay 40 --jitter 10 --loss 5
cargo run -p baba-net --example netsim -- run --locals 4x2 --mode own --leave 0@5 --relay
cargo run -p baba-net --example netsim -- memory --locals 12x1 --mode coop   # one process, no server
perf-mode run -r "net matrix" -- scripts/net/netsim-matrix.sh [secs] [filter...]
```

`dev-server.sh` builds `warpzone-server` and `wz` from `ext/warpzone` into
`scratch/net/wz-target/` (ext/ is only read), writes its config and database
to `scratch/net/wz-dev/` (registration/login limits raised: a run creates a
dozen identities from one IP) and serves `packaging/warpzone/` as its games
directory (validated with `check-games` at every start). Ports 3036/3516
avoid Warpzone's defaults (2998/3478), towerfall-port's (3012/3490) and
bopl-port's (3024/3504).

**`MemNet`** (`mem.rs`) is an in-process stand-in for a Warpzone server:
public/unlisted rooms, open/code/approve entry, codes, invite secrets,
knocks, the room list, leave, kick, crash (no goodbye), star-shaped channels,
the same `Event`s. **`toy.rs`** is a deterministic puzzle game (10 × 7
grids, walls, pushable rocks, two YOU units, a flag; undo and restart are
turns; clearing goes to the next level; mode 2 keeps the shared save as
sequenced clear events and evaluates it at `sync`). **`harness.rs`** drives
one machine with bots (4–12 turns/s per player) through a plan: host or
join (odd machines type the code, even ones come in through
`--warpzone-join <invite link>`, public rooms by room id), leave, crash or
corrupt its replica at a time, and the host ends the session; `RunSpec::check`
is the PASS rule: every remaining machine ended with `agreed`, no desync,
identical final hashes, turns were made, and a migration happened if the
host left (with `--desync`: every remaining machine was told; with
`--abort-on-desync`: every machine aborted). `netsim run` runs one OS
process per machine, each with its own `WARPZONE_HOME` under
`scratch/net/homes/`, logs in `scratch/net/<tag>.log`.

### Results

`cargo test -p baba-net`: 4 unit tests and 13 in-process session tests
(~16 s): 4 × 3 co-op; 12 × 1 co-op with 30 ms ± 10 / 5 % loss; 2 + 4 + 6
shared save with a spectator; 1 × 12 and 12 × 1 own games (lossy); a leave
and a crash mid-session in all three modes; late joiners (players and a
spectator) in all three modes; the host leaving in all three modes
(migration); the host crashing (shared save, lossy); injected desyncs
(co-op, own games) detected; the abort policy; lobby (code, invite link,
version and content rejects, mode/settings/roster everywhere, players
added and removed mid-session); the 12-player cap with waiting players;
mode-1 prediction converging.

2026-10-18, `scripts/net/netsim-matrix.sh 20`: one OS process per machine
against the local dev server, one laptop, 20 s sessions (the crashed-host
case 45 s), bots making 4–12 turns/s per player (~1450 turns per case with
12 players). Link values are one way per machine and hop ("rtt80" = 40 ms
± 10 each way, so guest → host → guest is ~80 ms plus retransmissions).
PASS = every remaining machine ended with the same final hashes of every
stream, no desync, and (host gone) one migration.

| Case | Machines × players | Mode | Link | Result |
|---|---|---|---|---|
| 12x1-coop | 12 × 1 | co-op | clean | PASS, 1453 turns |
| 4x3-shared | 4 × 3 | shared save | clean | PASS, 1455 turns |
| 2-4-6-own | 2 + 4 + 6 | own games | clean | PASS, 1457 turns |
| 4x3-coop-rtt80-loss5 | 4 × 3 | co-op | 40 ± 10 ms, 5 % loss | PASS |
| 12x1-own-rtt150-loss3 | 12 × 1 | own games | 75 ± 25 ms, 3 % loss | PASS |
| 2-4-6-shared-rtt80-loss5 | 2 + 4 + 6 | shared save | 40 ± 10 ms, 5 % loss | PASS |
| 4x3-relay-spec | 4 × 3 + 1 spectator | shared save | forced relay | PASS (spectator agrees) |
| 2-4-6-coop-relay-loss | 2 + 4 + 6 | co-op | forced relay, 40 ± 10 ms, 5 % | PASS |
| public-2-1 | 2 + 1, public room joined by id | co-op | clean | PASS |
| 6x2-leave-crash | 6 × 2 | shared save | 40 ± 10 ms, 5 %; m2 leaves @4 s, m4 crashes @7 s | PASS (4 remain) |
| 4x1-late-join-spec | 4 × 1 + 1 spectator | co-op | clean; m3 joins @5 s, the spectator @6 s | PASS (replayed from 0) |
| 3x2-own-late-join | 3 × 2 | own games | 40 ± 10 ms, 5 %; m2 joins @6 s | PASS |
| host-leaves-coop | 4 × 2 | co-op | 40 ± 10 ms, 5 %; the host leaves @5 s | PASS, 1 migration, 3 remain |
| host-leaves-own-relay | 2 + 3 + 1 | own games | forced relay; the host leaves @5 s | PASS, 1 migration |
| host-crashes-shared | 3 × 2 | shared save | clean; the host crashes @5 s (noticed after Warpzone's 15 s time-out) | PASS, 1 migration |
| 3x1-desync | 3 × 1 | co-op | 40 ± 10 ms, 5 %; m1 corrupts its replica @4 s | PASS: detected, reported to all 3 |
| 3x2-desync-abort | 3 × 2 | own games | clean; m2 corrupts @4 s, abort policy | PASS: aborted on all 3 |

Odd machines join by typing the code, even ones through `--warpzone-join
<invite link>` (the launcher path), so every case exercises both.

## The game on baba-net (M7)

Code: `crates/game/src/session/online.rs` (the replica, encodings),
`crates/game/src/app/online.rs` (screens, the frame loop),
`session/multi.rs` (the session presenter shared with local play),
`session/online_tests.rs` (in-process tests), `drive.rs` (`online ...`
script commands), `scripts/net/online-test.sh` (multi-process matrix).

* **Ids.** `NetConfig::build` = `baba-port <version> g<GAME_NET>`
  (`GAME_NET` changes with the turn/settings/event encodings or the sim);
  `NetConfig::content` = FNV-1a over `Data/values.lua` and every `.l`,
  `.ld` and `world_data.txt` of the install's worlds. Online worlds are the
  install's (custom packs in the data directory are local-only for now).
  Warpzone account: generated `baba-xxxxx` (kept in
  `<data>/online-name.txt`; `BABA_NET_USER` overrides). Testing overrides:
  `BABA_NET_BUILD`, `BABA_NET_CONTENT`.
* **Settings blob** (`SessionSettings`, host): `BS1`, world, the host's
  slot, and the save everybody starts from (modes 1/2), zlib-deflated
  (`MAX_SETTINGS` is 32 KiB since protocol 2; a complete save deflates to a
  few KiB). Room-browser label `world` via `set_room_label`.
* **Turns** are `session::Turn::encode` (≤ 64 bytes) plus `Turn::Refresh`
  (mode 2, below) and `Turn::Auto` (byte 8: one LEVEL IS AUTO tick; like
  Proceed, a turn nobody pressed: the leader sends it in mode 1, the
  owner's machine in modes 2/3, each at the level's rhythm; mode-1
  prediction steps the pending ones too). Stream entries go through `Multi::apply_now` →
  `Play::apply` on every machine; local actions only fill the outbox
  (`Multi::apply`), sent after the frame's `Multi::tick`.
* **Mode 1**: one instance (seats = every session player; YOU/YOU2 by
  player number), on a copy of the host's save; only the host writes the
  result to its slot file. Turns nobody pressed (Proceed after a win, a
  conversion, an ending) are sent by the *leader*: the machine of the
  lowest active player (so a departed host or player never leaves a win
  stuck). A transition started here (enter, return to map, restart,
  proceed) waits at full cover until its turn comes back (≤ 10 s); a
  loading turn from another machine shows the opening half.
* **Mode 2**: one instance per player on every machine, each on its own
  copy of the save (created at the player's `Joined` from the shared save
  as of that entry). The owner's machine turns what a turn recorded into
  `Progress` events (`Win {stem, parent, bonus}`, `Flag end/done`,
  `Convert`) for the shared stream; `Progress::merge` re-runs the slot
  part of a win (`play::win_progress`, shared with single player) and is
  idempotent. Before a player-stream entry with `sync = s` is applied,
  every shared event below `s` not yet merged into that player's copy is
  merged; the owner sends `Turn::Refresh` when its player stands idle on a
  map with unmerged progress, so the map updates on every machine at the
  same point of that player's stream. The host merges events into its own
  slot file too. Events are sent only for turns made live here (not for
  replays).
* **Mode 3**: a player's stream starts with its world and save as
  own-stream events (`own_event`, new in protocol 2; chunks of 3.5 KB,
  `init_chunks`); only the owner writes its slot file.
* **Hashes**: shared stream = co-op instance / shared save text / roster;
  player stream = that instance (`Play::state_hash`) + its save copy.
  Players who leave keep a hidden instance (entries of theirs may still
  arrive after the `Left` on some machines).
* **Late join / resync**: a fresh replica replays from entry 0, quietly
  (no sounds, banners, transitions) while far behind. A `Desync` naming
  this machine rebuilds the replica and replays everything
  (`NetSession::restart_replay`, hashes already reported are not resent);
  if `own_hash` at the reported index then equals the majority it plays
  on ("Back in sync"), otherwise (or on a second desync of the same
  stream) it leaves the room with an explanation.
* **Prediction** (mode 1, guests with a round trip over 80 ms, or
  `BABA_NET_PREDICT=1`): the view draws a clone of the confirmed level
  stepped with our pending moves/undos (`View::predict`); never fed back.
* **Lobby rules**: the host starts (1.5 s countdown) when there are at
  least two machines and every machine has players and is ready; F5
  starts at once (machines without players watch). A guest's players and
  readiness are resent once it is welcomed.
* baba-net fixes found on the way: a machine that joined without players
  and added some stayed a spectator (now only an explicit `set_spectate`
  keeps it watching); `set_room_label`; `restart_replay` / `own_hash`.

### Online test matrix (game)

`scripts/net/online-test.sh [--release] [filter...]` runs one headless
`baba-port --drive` per machine (`crates/game/tests/online/*.drive`, the
same script on every machine; lines tagged `@m0`, `@host`, `@guest`, …)
against the local dev server, virtual controllers as local players, real
time while online. PASS = every remaining machine ended the session with
`agreed` (identical final hashes of every stream), plus each script's own
checks (levels, icons, views, players).

2026-10-18, `scripts/net/online-test.sh` (debug build, one laptop, local
dev server; "lossy" = 40 ± 10 ms one way per hop and 5 % loss): **19/19
PASS**, ~7200 bot presses in total besides the scripted play.

| Case | Machines × players | Mode | What else | Result |
|---|---|---|---|---|
| coop-2x1 / shared-2x1 / own-2x1 | 2 × 1 | each | code + scripted solves (0level, 1level), host's slot file checked | PASS ×3 |
| coop-3x4 / shared-3x4 / own-3x4 | 3 × 4 = 12 | each | code + invite link, mode-2 win reaching the others' maps, 8 s of all 12 pressing | PASS ×3 |
| coop-2-4-6 / shared-2-4-6 / own-2-4-6 | 2 + 4 + 6 | each | as above | PASS ×3 |
| coop-3x4-lossy-predict | 3 × 4 | co-op | lossy, prediction on | PASS |
| shared-2-4-6-lossy | 2 + 4 + 6 | shared | lossy | PASS |
| own-3x2-relay-lossy | 3 × 2 | own | forced relay + lossy | PASS |
| leave-shared-3x2 | 3 × 2 | shared | m2 leaves mid-session | PASS (2 remain) |
| late-join-own-3x2 / late-join-coop-3x1 | 3 × 2 / 3 × 1 | own / co-op | m2 joins the running session, replays, adds its players | PASS ×2 |
| host-leaves-coop-3x2 | 3 × 2 | co-op | the host leaves; regroup, new host ends | PASS (1 migration) |
| desync-coop-3x1 | 3 × 1 | co-op | m1 corrupts its level; reported to all, m1 replays, back in sync | PASS |
| mismatch-2x1 | 2 × 1 | – | m1 another build: refused with the message | PASS |
| menus-browser-2x1 | 2 × 1 | co-op | main menu → Online → host public; join screen → O → room browser → join | PASS |

The baba-net matrix (`netsim-matrix.sh 20`, toy game) still passes 17/17
with protocol 2.

## Open issues

* **No guest-to-guest links** (Warpzone rooms are a star): every turn goes
  through the host, so a guest sees another guest's turn after two hops.
  For a turn-based game this is fine; the host's upload carries every turn
  to every machine (~10 B × machines per turn).
* **Host migration needs the room list**: the regroup room is public (entry
  "approve") for a moment, because guests cannot talk to each other to pass
  on a private code. Strangers can see it and knock (the game decides); the
  new room has a new code. If two successors host at once (the first one
  slow beyond 10 s), the guests pick the lowest `mhost`, but the other
  successor is not told to give up (it ends up alone).
* **Crashed host**: guests notice only after Warpzone's 15 s peer time-out.
* **Late join replays from the start**; no snapshot transfer yet.
* **Game**: online worlds are the install's (custom packs from the data
  directory are not offered online yet); joining a *running* public
  session from the room browser by room id was seen to fail once ("no
  such room", codes and invite links work); no chat; mode-2 gates opened
  by one player reach the others when they load that map.
* `own_hashes` and the logs grow with the session (a few MB per hour with 12
  players).
* A local player removed mid-session (modes 2/3) while its last turn is in
  flight to the host: the host's `Left` cuts the stream at what it has; the
  owner truncates its copy, but its replica already applied the turn (the
  player is gone, so only that player's final hash can differ). The game
  should stop that player's input before calling `set_local_players`.
* **C compiler for TLS**: `warpzone-core` enables `ureq`'s `rustls`
  feature, which turns on `_ring` → `ring` (the only crate in the tree with
  a `cc` build step), so baba-net needs a C cross compiler for
  `aarch64-linux-android` / `aarch64-unknown-linux-gnu` (checked: build
  fails looking for `aarch64-linux-android-clang`). ureq already prefers a
  process-wide rustls `CryptoProvider` over ring at run time
  (`src/tls/rustls.rs`), so ring is only needed at compile time. Proven
  without a C compiler (`CC=/bin/false`, `scratch/tlsprobe/`): a
  `[patch.crates-io]` copy of ureq 3.4.2 whose `rustls` feature drops
  `_ring`, plus `CryptoProvider::install_default(...)` before
  `Client::new` with `rustls-graviola` (pure Rust; aarch64 needs the
  aes/sha2/pmull extensions, x86_64 AVX2/ADX era CPUs) or
  `rustls-rustcrypto` (portable, alpha): rlibs for both aarch64 targets
  build, HTTPS to the public Warpzone instance works. The clean fix is a
  `warpzone-core` feature (e.g. default `tls-ring`, off = `ureq`
  `rustls-no-provider` + `rustls-webpki-roots`), no code change there.
