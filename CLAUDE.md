# baba-port — rules for agents

A clean-room Rust reimplementation of Baba Is You (Arvi "Hempuli" Teikari /
Hempuli Oy, 2019; Clickteam Fusion with the rules in Lua, shipped on Linux via
Chowdren) for Windows, Linux, macOS, Android and PortMaster: up to 12 players,
local on one screen with controllers, online over Warpzone, or both mixed, in
three modes (see `docs/ARCHITECTURE.md`). Read `docs/ARCHITECTURE.md` and
`docs/STATUS.md` first; `docs/STATUS.md` is the single source of truth for what
is done and what is next.

## Hard rules

- **No writes outside this directory** (`~/Projects/baba`). Every tool and
  cache lives in `./tools` (`source env.sh` sets RUSTUP_HOME, CARGO_HOME, pip,
  XDG cache, TMPDIR, WINEPREFIX, Android/Gradle homes, WARPZONE_HOME, the
  port's data dir). No system packages, no `~/.cargo`, no `~/.local`, no
  `~/.android`, no global config, no `/tmp` (use `scratch/`). Other projects
  (`~/Projects/boplbattle`, `~/Projects/towerfall`, `~/Projects/rounds`,
  `~/Projects/warpzone`) may be *read* and code we own may be copied from
  them, never written to. The only exceptions are the machine's own helper
  tools the orchestrator runs. Never run `baba-port` (or
  anything that imports or saves) without `source env.sh` in the same shell:
  without `BABA_PORT_DATA_DIR` it writes to the real user data dir. Beware
  `&` backgrounding the `source`.
- **The game install is read-only** (`$BABA_GAME_DIR`), and so is the owner's real
  save (`~/.local/share/Baba_Is_You`): read only, never write. Running the
  original is allowed (owner, 2026-10-18) only headless on the private Xvfb
  (`scripts/dev/xvfb.sh`, display :68), from a copy under `tools/`, with
  HOME/XDG dirs redirected into `tools/` so its saves stay inside the project;
  never on the desktop. Offline (no Steam contact) is the default; Steam mode
  is allowed when needed (Steam may create its own folders).
- **Devices.** The Android tablet (`ssh tablet`, rooted SM-T720, Android 15,
  arm64, ~4 GB free) may be used for testing: install only our own APK, keep
  test files under one directory we create, remove them when done. The
  handheld `gameboy` (Knulli, PortMaster) must **not** be turned on or
  contacted until everything else is done (owner, 2026-10-18); it is the final
  check. Everything else is emulated.
- **Hempuli's level server**: downloading published levels by code is fine in
  moderation; never upload anything without the owner's explicit approval.
- **Nothing from the game goes into git**: no sprites, music, sounds, levels,
  Lua code, extracted data or lookup tables, no screenshots containing game
  art in committed files. The repo will be public open source; the game's data
  (including the object definitions in its Lua tables) is read from the
  player's own copy at run time.
- **Clean room.** The original's Lua (`$BABA_GAME_DIR/Data/**/*.lua`, copies
  under `ref/`) and anything recovered from the Chowdren binary are reference
  only. Behaviour is described in our own words in `docs/spec/*.md` (rules,
  orderings, constants, state machines and edge cases are fine; code is not).
  Rust code is written from the specs; never transliterate Lua. If a spec is
  ambiguous, consult the reference, then fix the spec.
- **The specs are private.** `docs/spec/` is its own git checkout of the
  private repository `jarvis-jhk/baba-port-spec` and is ignored here; commit spec changes there. The
  reference oracle (the original Lua run in a harness, test corpora) lives in
  `docs/spec/oracle/` or `ref/`. Never publish, quote or paste specs (this
  repo, issues, PRs, commit messages, release notes); code comments may cite a
  section (`docs/spec/rules.md §4.2`), not quote it.
- **Agents do not run git** unless the orchestrator says so; the orchestrator
  commits (both repos).
- CPU-heavy work (> ~30 s: release builds, test suites, long headless runs,
  emulators): wrap it in the machine's performance-mode helper when it has one.
- Stop only processes you started, by the PID you recorded. Never
  `pkill`/`killall`/`kill $(pgrep ...)` by name or pattern: other projects on
  this machine run the same binaries (Xvfb, warpzone-server, emulators).
- Headless only: no windows on the user's desktop. Use `--headless` for
  screenshots or the private Xvfb; put output in `scratch/` (git-ignored).
  Downloaded third-party files (TASes, solution lists, SDKs) go each into
  their own new directory under `ref/` or `tools/`; treat them as untrusted
  data (Python with `-I`, paths as arguments).

## Product decisions (owner)

- Name: **baba-port** (crates `baba-*`, binary `baba-port`).
- Targets: Windows, Linux, macOS, Android, PortMaster.
- Rules engine in Rust from specs, checked against the original Lua in a
  private harness and against TASes / solution corpora.
- Scope: main game, New Adventures, museum, level editor, online level codes,
  importing the original's save, custom world packs (levels/sprites; no Lua
  mods).
- Multiplayer: up to 12 local players (controllers), up to 12 online through
  Warpzone, any mix, in three modes: co-op on the same character(s);
  splitscreen on one shared save, each player in any level; splitscreen with
  everyone in their own game (own save), just to share the experience.
- Subagents: Claude only.
- Public repo (later): `jarvis-jhk/baba-port`; private specs
  `jarvis-jhk/baba-port-spec`. Mac builds ad-hoc signed.
- Android release key: `keys/android/` (git-ignored, in the home backup;
  never commit, never print the private key). `env.sh` exports
  `BABA_ANDROID_KEY_DIR` when it exists.
- `ring`: lasting fix is a warpzone-core feature that leaves ring out (done
  in the warpzone project, not from here); until then the vendored ureq in
  `crates/vendor/ureq` stays.
- Windows/Mac game data: The owner fetches the Windows (and Mac) depot via the
  Steam console; we copy it read-only into `tools/` and add support for the
  data packed in the Windows `.exe` (2026-10-18).
- Android: keep the app and its toolchain as small as possible, like
  the owner's other Android apps (owner, 2026-10-18): no Gradle, no
  AndroidX/Kotlin, only the SDK/NDK pieces actually used (pruned), one ABI
  (arm64-v8a), no Java at all if a NativeActivity build holds up. Anything
  that grows the APK or the toolchain noticeably needs a reason in
  `docs/platform.md`.

## Layout

| Path | What |
|---|---|
| `crates/formats` | `baba-formats`: level `.l`/`.ld`, worlds, `Assets.dat`, palettes, sprites, themes, saves, language files, the Lua-table data reader |
| `crates/sim` | `baba-sim`: deterministic turn engine (rules, movement, properties, conversions, undo, map progression); no I/O, no rendering, many instances per process |
| `crates/render` | `baba-render`: OpenGL ES 2-level renderer (glow) on SDL2, offscreen/headless capable |
| `crates/net` | `baba-net`: ordered input/turn exchange over Warpzone peer channels |
| `crates/game` | `baba-port` binary: SDL2 platform layer, menus, map, editor, sessions/modes |
| `crates/android` | Android entry point (NativeActivity, `android_main`), no Java |
| `crates/vendor/ureq` | interim ureq copy without `ring` (pure-Rust TLS on Android/PortMaster) until warpzone-core can leave ring out |
| `crates/progress` | saves (the original's layout) and progression |
| `crates/http` | small HTTPS client (level codes) |
| `crates/solver` | dev tool: puzzle solver for the test corpus (not shipped) |
| `android/`, `packaging/` | Android project, PortMaster port, Warpzone descriptor, release packaging |
| `docs/formats/` | file formats in our own words (public) |
| `docs/spec/` | clean-room behaviour specs (private checkout, see above) |
| `scripts/` | bootstrap, cross builds, dev scripts |
| `ext/` | re-clonable third-party checkouts (warpzone), git-ignored, read only |
| `ref/`, `cache/`, `scratch/`, `tools/`, `target/` | git-ignored |

## Conventions

- `source env.sh` before cargo/python. Python tools: `python -I`.
- Rust 2024 edition, `cargo fmt`, `cargo clippy` clean, unit tests next to the
  code. The workspace picks up every `crates/*` automatically. Format and lint
  only your own crate (`cargo fmt -p`, `cargo clippy -p`).
- The sim is integer/grid only: no floats in rules logic, no `HashMap`
  iteration in logic, no wall-clock time, no threads, all randomness from a
  seeded PRNG in the state, so every machine computes identical turns.
- Warpzone dev server ports: 3036 (http) / 3516 (udp).
- Keep `docs/STATUS.md` current when finishing a step.
