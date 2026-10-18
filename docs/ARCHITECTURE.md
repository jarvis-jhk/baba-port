# Architecture

## The original

Baba Is You is a Clickteam Fusion game. On Linux it ships as a Chowdren build
(`bin64/Chowdren`, the Fusion event logic translated to C++) plus
`Assets.dat` (engine-side images, fonts, sounds), and plain data under `Data/`:

- `Data/*.lua`, `Data/Editor/*.lua`: the game rules, map, menus and editor in
  Lua (~29k lines). The engine calls into them and they call back through
  `MF_*` functions (documented by the game in `Data/MF_documentation.txt`).
  Object definitions (sprite, colour, tiling, layer, word type) are Lua tables.
- `Data/Worlds/<world>/`: `NNNlevel.l` (binary, zlib-compressed tile layers),
  `NNNlevel.ld` (ini: name, palette, map links, paths, specials), previews,
  per-world sprites, palettes, themes, music. Worlds: `baba` (main game),
  `new_adv` (New Adventures), `museum`, `levels` (user levels), `debug`.
- `Data/Sprites`, `Data/Palettes`, `Data/Music`, `Data/Sounds`,
  `Data/Languages`, `Data/Themes`, fonts.
- Saves: `~/.local/share/Baba_Is_You/{0,1,2}ba.ba` and `ba.ba` (ini).

The port reads all of this from the player's own install at run time.

## The port

```
formats ──► sim ──► game (SDL2: menus, map, editor, sessions) ◄── net (Warpzone)
   │                  │
   └────────► render (GLES2-level, headless capable) ◄┘
```

- **`baba-formats`**: parsers for every file above, including a data-only
  reader for the Lua tables that hold object definitions (no Lua VM).
- **`baba-sim`**: a pure, deterministic turn engine. `Level` state = units on
  a grid + level flags; `step(input) -> events` applies one turn exactly as
  the original does (rule parsing, movement, property resolution,
  conversions, win/defeat, undo). Undo is a stack of per-turn deltas. A
  `World` (a save slot: map levels, completion, unlocks) drives progression.
  Many independent instances may live in one process (splitscreen, online
  views of other players). Cosmetic randomness (particles) is separate from
  rules randomness so presentation never changes outcomes.
- **`baba-render`**: OpenGL ES 2.0-level features only (runs on GL 3.3 core,
  GLES2/3, and PortMaster devices via SDL2's KMSDRM), via `glow`. Draws the
  tile grid with the 3-frame wobble, palettes, text, particles, transitions
  and level rotation; renders N viewports for splitscreen; offscreen for
  headless screenshots and tests.
- **`baba-net`**: Warpzone peer channels (reliable, ordered). Baba is
  turn-based, so there is no rollback: peers exchange turn inputs; the host
  sequences inputs where several players drive one level; all peers run the
  same sim and compare state hashes.
- **`baba-port`** (`crates/game`): SDL2 platform layer (windows, controllers,
  touch on Android, audio), title/menus, map navigation, editor, level codes,
  save import, session and mode management.

## Multiplayer modes

1. **Co-op, same character.** One level instance; every player's input drives
   the YOU objects. Each press is one turn, in the order the host receives it
   (locally: in the order SDL delivers them); anyone can undo or restart.
2. **Shared save, splitscreen.** One save; each player has their own map
   cursor and level instance. Clearing a level updates the shared save at once
   and everyone's map reflects it.
3. **Own games, splitscreen.** Each player has their own save and instances;
   the screen (and, online, every machine) shows everyone's game side by side.

Local: 1–12 players with controllers (plus keyboard), viewports tiled to fit.
Online: up to 12 players over Warpzone in any mix with local players; every
machine runs every visible instance from the shared input stream, local views
large, remote views smaller (switchable).

## Platforms

Linux and Windows (cross-built from Linux, checked under Wine), macOS (CI
runners), Android (APK, NativeActivity without Java: android-activity + EGL
+ AAudio backend, touch + controllers; tested on the tablet), PortMaster (aarch64 Linux, GLES2, KMSDRM, 640x480-class screens).
