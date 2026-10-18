# Reference capture: running the original, solution corpora

How we run the original game for reference and where the test corpora come from. Everything
here stays on the developer's machine: screenshots go to `scratch/` and downloaded or derived
data to `ref/` (both git-ignored). Nothing from the game is committed.

## Running the original headless

`scripts/dev/ref-run.sh` runs the Linux (Chowdren) build from our copy in
`tools/baba-original` on the private Xvfb (`scripts/dev/xvfb.sh`, display `:68`):

```sh
scripts/dev/ref-run.sh start          # Xvfb + game, PID in scratch/ref/game.pid, stdout in scratch/ref/game.log
scripts/dev/ref-run.sh shot NAME      # window screenshot -> scratch/ref/NAME.png (ImageMagick import)
scripts/dev/ref-run.sh key Return     # key presses (xdotool keysyms), "+N" = pause N/10 s
scripts/dev/ref-run.sh keys RRUZW     # turn string: U/D/L/R arrows, W space, Z undo (z), r restart, E Return, X Escape
scripts/dev/ref-run.sh stop           # stops the game by its recorded PID
scripts/dev/ref-run.sh reset-save     # wipes the private save (game must be stopped)
```

- **Offline, no Steam.** The game runs from `tools/baba-run/`: a copy of `bin64/Chowdren` plus
  symlinks to the data, *without* `libsteam_api.so`. Chowdren loads the Steam library lazily;
  when it is missing it logs "Could not load Steam DLL" and runs normally (same for
  `libsentry.so`, the crash reporter). The Steam client is never contacted and nothing is
  created outside the project.
- **Private home.** `HOME`, `XDG_DATA_HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`,
  `XDG_RUNTIME_DIR` and `TMPDIR` point into `tools/baba-home/`; the save
  (`tools/baba-home/.local/share/Baba_Is_You/`) and Mesa's shader cache stay there. The
  environment is cleared (`env -i`); audio uses SDL's `dummy` driver.
- **Session bus.** `DBUS_SESSION_BUS_ADDRESS` points at a non-existent socket on purpose: with
  it unset, libdbus "autolaunches" a private bus on `:68`, which then starts
  xdg-desktop-portal, at-spi, gvfsd and FUSE mounts.
- **Windowed.** Fullscreen on a window-manager-less Xvfb renders cropped, so the script sets
  `fullscreen=0` in the private `SettingsC.txt`, and `restartask=1` (no restart prompt).
- **Input.** The game polls key state once per frame and frames are slow under llvmpipe, so
  keys must be held across a frame; held keys auto-repeat (undo especially). Reliable scripted
  play holds a key until the game has taken the turn (see the oracle tooling below). The level
  title card swallows input for ~3 s after entering a level, as does a restart transition.
- Version shown on the title screen: 483.

Captured (`scratch/ref/capture/`, contact sheet `contact.png`): splash, title menu, the first
level before/after an undo and its win, the map of each world (main game, New Adventures,
Museum), two levels of each world, the pause menu, a level title card.

### Traced build (private)

`REF_TRACE=1 scripts/dev/ref-run.sh start` runs a second tree, `tools/baba-trace/`, whose `Data/`
loads an extra private tracer (in `docs/spec/oracle/`, not part of this repository). It logs
the engine/Lua call sequences and the unit state after every turn to the game's stdout; that
is how the reference oracle was built and how it is checked turn by turn against the real game.
`REF_QUEUE=a,b,c` (with `REF_WORLD=…`) makes the map's "enter level" action open those level
files in turn, so scripted checks can reach any level without an unlocked save.

## Solution corpus

Normalised format (`ref/tas/corpus/corpus.tsv`, tab separated, one solution per line):

| column | meaning |
|---|---|
| `world` | level pack folder (`baba`, `new_adv`, `museum`) |
| `level` | level file without extension (`177level`) |
| `name` | level name as in the source |
| `variant` | `-`, or an alternative ending such as `LEVEL IS BABA` |
| `source` | `<source>:<level code>` |
| `inputs` | one character per input from the level start: `U D L R` move, `W` wait, `Z` undo, `r` restart |

`ref/tas/corpus/verified.tsv` adds the result of replaying each entry in the private reference
oracle: `status` (`win`; `win-seed` = wins only under another random outcome, e.g. TELE;
`event` = an alternative ending such as a level conversion; `nowin`), `win_turn`, `length`,
`events`.

### Sources

| Source | What | Where | Used |
|---|---|---|---|
| [baba-is-optimized](https://github.com/SzieberthAdam/baba-is-optimized) (SzieberthAdam, archived 2023) | shortest known solutions of the main game ("Discord Collective" min-steps routing), 246 move strings incl. alternative endings | `ref/tas/baba-is-optimized/` (GitHub tarball) | yes |
| [TASVideos #4281M](https://tasvideos.org/4281M) (lexikiq, libTAS 1.4, v277) and [userfile 637787978063843404](https://tasvideos.org/UserFiles/Info/637787978063843404) (keylie, resynced on v451C) | any% TAS movies (`.ltm`: per-frame X11 keysyms; uses two key bindings per frame for double moves) | `ref/tas/tasvideos/` | not converted: any% only (a few main-game levels already covered above), map navigation and frame timing would have to be simulated |
| [Steam guide 2681613839](https://steamcommunity.com/sharedfiles/filedetails/?id=2681613839) ("New Adventures - Step by Step Solutions Guide", v449-451) | prose hints for 158 New Adventures levels (no Museum) | `ref/tas/steam-2681613839/` (`guide.html`, `guide.txt`) | as hints for hand solving (stage goals, below) |
| [baba-is-harbor](https://github.com/stared/baba-is-harbor) | agent benchmark, oracle move strings for 55 early main-game levels | – | no (all already covered) |
| speedrun.com, TASVideos, other walkthrough sites, fan wiki | videos / prose only | – | no |

No machine-readable solution sets for *New Adventures* or the *Museum* were found (searched again
2026-10-18); their corpus comes from the solver and hand solving below.

### Solver and hand-made solutions (`solved.tsv`)

`crates/solver` (`baba-solver`, dev tool, not shipped) searches with `baba-sim`; every solution it
or a person finds is only a candidate until `scripts/dev/corpus_solved.py` has replayed it in the
private oracle. Accepted solutions (cut at the winning turn, shortest kept per level) go to
`ref/tas/corpus/solved.tsv`: the corpus columns plus `status` and `win_turn`; `source` is
`solver:<strategy>` or `manual`.

- Search: BFS, greedy best-first and best-first width search (novelty over unit positions and
  active rules, per heuristic value), state-key dedup (unit multiset; facing only where a word can
  make it matter), levels kept only for open nodes within a memory budget, checkpoints and replay
  otherwise. Macro moves where the walking model applies (one YOU unit, no conditional or per-turn
  rules): the walker's region is flood filled without the sim and only moves that touch something
  are simulated (an exhausted macro search is retried with single inputs). Heuristic: over object
  kinds A/B, cost of making A YOU and B WIN (push distance of the texts, Sokoban-style frozen
  texts excluded) plus the path distance between them avoiding STOP/DEFEAT/SINK.
- `baba-solver batch` runs the portfolio over every unsolved level (threads, time per level);
  levels containing `3D` or `AUTO` text are skipped (3D was not in the sim, AUTO not in the oracle).
- Hand solving: `baba-solver dump` renders a level as text (ruler every 5 columns, unit positions,
  rules); `baba-solver solve --plan "stage;stage..."` chains sub-goals (`rule:`, `norule:`,
  `at:`, `notat:`, `gone:`, `exists:`, `count:`, `pair:`, `near:`, `face:`, `win`; `&`-prefixed
  stages inherit the previous goals, `-goal` drops one; stages only count while a player
  exists). Plans were written from the Steam guide's steps or from reading the level; pinning the
  positions of the rule texts that must stay is usually what makes a stage findable.

```sh
source env.sh
cargo run --release -p baba-solver -- batch --world new_adv --world museum --time 120 --threads 6 \
    --out scratch/solver/batch.tsv
cargo run --release -p baba-solver -- solve new_adv 114level --strategy bfws \
    --plan "at:text_wall:11,2&at:text_is:12,2;at:text_push:14,2;at:text_push:13,2;at:text_wall:9,2&at:text_is:10,2;win"
python -I scripts/dev/corpus_solved.py scratch/solver/batch.tsv scratch/solver/manual.tsv  # -> solved.tsv
cargo run --release -p baba-sim --bin sim-corpus -- ref/tas/corpus/solved.tsv /dev/null
```

### Regenerating

```sh
source env.sh
mkdir -p ref/tas/baba-is-optimized && cd ref/tas/baba-is-optimized
curl -sSfLo repo.tar.gz https://codeload.github.com/SzieberthAdam/baba-is-optimized/tar.gz/refs/heads/master
python -I -c "import tarfile; tarfile.open('repo.tar.gz').extractall('src', filter='data')"
cd ../../..
python -I scripts/dev/corpus_build.py     # -> ref/tas/corpus/corpus.tsv
python -I scripts/dev/corpus_verify.py    # -> ref/tas/corpus/verified.tsv + coverage (needs the private oracle)
python -I scripts/dev/ltm_dump.py ref/tas/tasvideos/4281M.ltm 40   # inspect a libTAS movie
```

Downloads are treated as untrusted data: they are only parsed (`python -I`, paths as
arguments), never executed.

The source's move notation differs from ours: its `Z` is *reset* (our `r`), `X` (exit to map)
entries are skipped, and repeat counts are expanded. Level names are matched to level files by
the `name=` of the `.ld` files; where a name occurs twice the level linked from a map wins.

### Coverage (2026-10-18)

Levels shown on a map (within the map's `levels=` count; `corpus_verify.py` also counts
left-over `[levels]` records beyond it: 18 main-game, 4 New Adventures, 5 Museum levels that
no map shows):

| World | Before | After | Missing |
|---|---|---|---|
| main game | 217/218 | 217/218 | `277level` (the box) |
| New Adventures | 0/162 | 44/162 (15 solver, 29 hand) | 118, incl. 13 with 3D/AUTO |
| Museum | 0/88 | 17/88 (all solver) | 71 |

All 61 `solved.tsv` entries win in the oracle and in `baba-sim` (`sim-corpus`). Solver runs:
120 s/level over all unsolved (26 found), 360 s (4 more, Museum part only), 240 s with the
fixed macro search (2 more, stopped early): about 1 in 7 levels falls to plain search; the rest
need hand plans. Candidate files, hand plans and notes on attempted but unfinished levels (with
reached prefixes) are in `ref/tas/corpus/solver-work/` (`deferred.txt`, `manual.tsv`, `plans/`).

Earlier main-game corpus (baba-is-optimized):

Main game: 225 of 246 entries win in the oracle exactly on their last input, 16 alternative
endings reach their level conversion, 2 win only under another random outcome (TELE), 3 do not
win (solutions marked uncertain or stale in the source). That verifies **218 of the 236**
playable main-game levels linked from a map; the 18 without a verified solution are mostly
unused/old levels. *New Adventures* (166) and the *Museum* (93) have no corpus; the oracle is
checked there against the real game with random walks instead. 14 corpus solutions were also
replayed in the real game (traced build) and won there with every turn identical to the oracle.
