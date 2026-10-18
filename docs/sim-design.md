# baba-sim design

`crates/sim` (`baba-sim`) is the deterministic turn engine. It has no I/O
besides reading `baba-formats` structures, no rendering, no clock, no
threads and no floats. A `Level` is a plain value: `step(Input)` mutates it
and returns a `TurnOutcome`; cloning it gives an independent copy (online
views, look-ahead, tests).

## Determinism rules

- All iteration is over ordered containers: `Vec` in unit-list / rule /
  tile order, `BTreeMap`/`BTreeSet` keyed by handle, name or tile index.
  No `HashMap` anywhere in the crate.
- Wherever the original game iterates a hash table, the sim uses ascending
  byte-wise name order (names) or ascending tile index (tiles).
- Randomness comes only from `rng::Rng` (SplitMix64) stored in the level
  state, seeded at level start (FNV-1a of world name, level file and the
  unit count before the map cursor, `turn.md` §12.1), saved in every undo
  record and restored by undo. Draws happen only at the documented draw
  points, in the original's order; OFTEN/SELDOM draw only on a cache miss.
  The private oracle can run with this exact generator (`--port-rng`), so
  the draw sequence itself is checked.
- Everything cosmetic (sound variants, particle chances and jitter, wobble
  frames) is left to the presentation, which rolls it with its own
  `rng::CosmeticRng`; the sim never draws for presentation. Presentation
  queries (`units()`, `camera()`, `level_colours()`, events) never touch
  the simulation state.
- Unit handles come from a per-level counter (never reused within a level
  session); persistent ids come from the id counter (never rewound).
- `Level::state_hash()` is FNV-1a 64 over explicit little-endian fields
  (unit list, level-wide state, PRNG state, known names, undo depth), so
  it is identical on every platform.

## State

| Part | Type | Notes |
|---|---|---|
| definitions | `Arc<Defs>` | effective object slots of the level, name lookups, interned unit names; immutable |
| units | `BTreeMap<Handle, Unit>` + `order: Vec<Handle>` | `order` is the unit list; removed units stay in the map (flag `dead`) until the end of the turn, because movement still reads them |
| per-name lists | `Vec<Vec<Handle>>` by interned name | text units are also in the `text` list |
| tile map | `Vec<Vec<Handle>>` | stack order; rebuilt from the unit list (with overflow trimming) at the defined points |
| rule store | `Arc<RuleSet>` | rebuilt from scratch on every recompute, never mutated afterwards |
| known names | `Arc<BTreeSet<String>>` | only grows; copy-on-write |
| undo history | `Vec<Arc<Record>>` + in-progress `Record` | committed records are shared between clones |
| caches | `RefCell<Caches>` | OFTEN/SELDOM rolls, POWERED results; part of the state (cloned) |
| PRNG | `Cell<Rng>` | so condition evaluation can draw from `&Level` |
| 3D | `vision::Vision` | on flag, camera target (unit id / fixed camera), target cycle; hashed; target ids also stored per undo record |
| AUTO | `auto_count`, `auto_dirs` | LEVEL IS AUTO count from the status passes; directions stored per player until the next tick |
| no player | `noplayer` | result of the end-of-turn player check (music state) |
| presentation | `fx::Fx`, `Unit::{active, disabled, paint, visible}` | effect history ("pm", `_short`), sound slots, move kind, level colours; not hashed, never read by the rules |

Cloning copies the unit map, lists and tile map (a few hundred small
entries) and bumps reference counts for everything else.

## Module map and the hook interface

Every gameplay family lives in its own file and is entered through plain
functions `fn(&mut Level)` called from a fixed place in the pipeline. The
order of those calls is the turn order of the original, written down once
in `turn.rs` (move phase, end-of-turn update, post-undo update, level start)
and in `effects/block.rs::run` (block steps). Adding or refining a family
means editing that family's file; the call sites only change when the order
itself changes.

| Family | File | Entry points |
|---|---|---|
| definitions, load order | `defs.rs`, `load.rs` | `Defs::from_level`, `load_order` (single function, correctable from the oracle), `Level::start` |
| rule parsing | `rules/parse.rs`, `rules/letters.rs` | `parse::run`, `letters::build_map` |
| ALL, MIMIC, NOT, X IS X | `rules/post.rs` | `all_and_mimic`, `postprocess`, `invert` |
| groups | `rules/groups.rs` | `groups::run` |
| WORD units | `rules/word.rs` | `word::collect` |
| rule queries | `rules/query.rs` | `prop`, `level_prop`, `empty_prop`, `count`, `units_with`, `verb_groups`, `any_is` |
| conditions | `conditions.rs` | `holds`, `has`, `empty_dir`, `group_members` |
| movement takes and resolution | `movement/mod.rs` | `takes`, `obstacles`, `run_take` |
| FALL, FOLLOW, BACK, TELE, SHIFT facing, HOLD, SELECT, level as mover | `movement/extra.rs` | `fall_pass`, `moveblock`, `hold`, `select`, `level_you` |
| status pass, rotation | `effects/status.rs` | `status_pass`, `status_pass_unit`, `level_rotation` |
| block effects (DONE … CRASH) | `effects/block.rs` | `run` (one function per step) |
| level block | `effects/level.rs` | `run` |
| conversions, HAS, MAKE, level conversion | `convert.rs` | `collect`, `execute`, `run_has`, `make`, `level_collect`, `level_apply` |
| undo | `undo.rs` | `line`, `commit_record`, `undo_record` |
| 3D (vision) | `vision.rs` | `check`, `change_target`, `refresh_targets`, `after_recompute`, `find_player`, `on_destroy`, `on_done`, `undo_line`, `Level::camera` |
| presentation output | `events.rs`, `fx.rs` | event types; `Level::burst`, `turn_sound`, `removal_sound`, `fx_removed`, `unit_colour`, `level_colours` |
| pipeline | `turn.rs` | `Level::step`, `flush`, `level_start`, `group_check` |

Primitives every family uses (all on `Level`, `level.rs`):
`create` (generic creation with id, undo line, registration and the
single-unit status pass), `destroy` (HAS, undo line, unregister),
`remove_recorded` (no HAS), `unregister`, `update_unit` / `set_dir`
(recorded moves and turns), `place_unit` (unrecorded), the pending queue
`scratch.pending` + `turn::flush`, `request_destroy`, `rand`. Presentation
events are emitted from the same primitives (moves, creations, removals with
their cause) plus the effect sites (`fx.rs` maps removal causes to bursts
and sound slots).

## Rules

`rules::recompute` follows the original pipeline: WORD units from the old
rules, base rules, letter words, sentence starts, sentence variants, the
grammar state machine with its restart rules, extraction, insertion (with
NOT-subject and ALL-parameter expansion), ALL and MIMIC, groups, active
text, NOT application, X IS X protection, then the WORD stability re-run.
Condition lists are kept as strings exactly as the original builds them
(OR-group parentheses included); every inserted rule gets a fresh conds id
for caches and recursion guards. Conditions are evaluated lazily on every
query against the current state.

## Undo

Each turn appends typed lines to the in-progress record. The record is
committed only if the turn-changed flag is set; undo replays the last
record backwards, restores the last key and the PRNG, and then runs the
post-undo update. Restored units are appended to the unit list. Undo's own
creations/removals leave the flag set, so the next turn commits even if it
changes nothing (the original's "empty step"). Records also keep the ids of
the WORD-watched units (undoing a line of one of them forces a rule
recompute, since a restored unit gets a new handle) and of the 3D target
cycle.

## Level start

`turn::level_start` runs the two start passes of `turn.md` §4.2a: per unit
every rule indexed under its rule name is tested (FLOAT; the tests matter
for OFTEN/SELDOM draws), FOLLOW facing, visual refresh; between the passes
the rule store, caches and `mapdir` are reset and the rules parsed again;
then 3D mode is switched on or off silently and the first record opened.

## 3D

`vision.rs` keeps what of 3D mode is simulation (`movement.md` §19.2): the
camera unit is a YOU-like mover in take 1 (up = forward, right/left =
turn clockwise/counter-clockwise), "down" cycles the target (and is then
not a camera move), the target is refreshed after rule recomputations that
mention 3D, handed on when its unit is destroyed (fixed camera if none
left), and 3D mode starts/stops at the end of the move phase and in the
player check. All of it is undoable. `Level::camera()` gives the
presentation the camera cell, facing and target; the first-person
rendering is presentation (`presentation.md` §8).

## Presentation events

`TurnOutcome` carries, besides win/end/no-player/transition flags:
- `events` in execution order: `Moved` (from/to, old/new facing, kind:
  step, fall, teleport, back, undo — the tween and move-puff source),
  `Created`/`Removed` with cause, `Converted` (ids and names), `RuleFormed`
  (text ids for the bling burst, rule sound), `Particles` (`Burst`: kind,
  cell, base count, effect-history scale, colour, 1-in-N chance, shake,
  attached unit), `Sound` (played directly: bonus, undo, infinity, ALL IS
  DONE), `Play` (PLAY verb groups; `events::play_sample`, `note_freq`),
  `BonusCollected`, `LevelDestroyed`, `LevelMoved`, `Camera`;
- the two sound slots `turn_sound` / `removal_sound` (`audio.md` §4.1,
  family + variant count + `_short`), and `kind` (turn, stored AUTO key,
  undo, restart).

Per-unit display state comes from `Level::units()`: text state (inactive /
active / disabled with a cross), current colour and the blink list of
colour properties, visibility, layer; `Level::level_colours()` gives the
background colours. Effect-history multipliers follow `presentation.md`
§9.2 (history decays at end-of-turn step 3, reset by undo).

## Inputs

`Input::Key { dir, player }` (0–3 directions, 4 wait; player 1 drives YOU,
player 2 YOU2 with fallback), `Input::Tick` (the LEVEL IS AUTO timer fired),
`Input::Undo`, `Input::Restart`. While `Level::auto_active()` keys only
store a direction per player (outcome kind `Stored`); the game layer owns
the timer and sends `Tick`s, each running the stored directions as one
auto turn (one move phase per AUTO rule, both players at once, then one
end-of-turn update). `Input::from_char` reads the corpus alphabet
`R U L D W Z r` plus `A` = tick.

## Tooling

- `cargo run --release -p baba-sim --bin sim-corpus` replays every solution
  of `ref/tas/corpus/corpus.tsv` and checks the win turn against
  `verified.tsv`.
- `cargo run --release -p baba-sim --bin sim-diff -- WORLD LEVEL INPUTS
  [--nodir] [--rules] [--all] [--data DIR]` runs the private reference
  oracle on the same inputs (with `--port-rng --sorted-pairs`) and prints
  the first turn whose units, rules, win flag, no-player state or PRNG
  state differ (`--no-rng`, `--native-rng`, `--hash-pairs` relax it).
- `cargo run --release -p baba-sim --bin sim-scenario -- OUT_WORLD_DIR
  FILE...` turns the ASCII scenarios of `crates/sim/scenarios/` (3D, AUTO,
  group corner cases) into real `.l`/`.ld` files for a scratch world, so
  they run through both the sim and the oracle (`sim-diff --data`).
- `cargo run --release -p baba-sim --example probe -- WORLD LEVEL INPUTS
  [names]` prints unit records after the inputs.
- `baba_sim::testkit` builds synthetic levels from an ASCII grid for tests.

Both binaries need `$BABA_GAME_DIR`; `sim-diff` also needs the private
`docs/spec/oracle` checkout and `tools/bin/lua5.3`.

## Verification status

Turn-by-turn identical to the oracle (units, rules, win, no-player state,
PRNG state): all 246 corpus solution traces, random input runs with undo,
restart and AUTO ticks over every level (several seeds, up to 250 inputs),
and the synthetic scenarios (3D, AUTO, groups: GGR condition accumulation,
NGR, recursive memberships incl. the end-of-turn group check destroying the
level, GROUP IS GROUP, NOT GROUP, verbs to/from groups, crash shapes) with
random inputs.

## Not implemented yet

- Map progression around the cursor (unlock propagation, saved path and
  icon states, level conversions applied on the parent map): the sim takes
  path states through `Context::path_states` and reports
  `Transition::LevelConversion`; game flow does the rest.
- Presentation events cover the per-turn effects of `presentation.md` §9.3
  for units; the empty-tile "1 in 4" bursts of the level block and the
  map-only effects (gates, unlock sequences) are game flow / presentation.
