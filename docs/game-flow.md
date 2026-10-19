# Game flow (M4): saves, progression, menus, audio rules

How baba-port turns the rules engine into a game: the save model, map
progression, entering and leaving levels, the title screen and menus,
settings and controls, the audio rules, credits and endings, and the
scripted headless flow test. Behaviour follows the private specs
(`progression.md`, `saves.md`, `ui.md`, `audio.md`); sections are cited,
not quoted.

## Crates

```
baba-progress (crates/progress)   save model + progression rules, no rendering
baba-port     (crates/game)       lib `baba_game`: platform-agnostic App; bin `baba-port` (SDL2)
```

- `baba_game::App` owns everything game-side (renderer, atlas, fonts,
  profile, audio rules, menus, play session) and is driven by any
  `baba_platform::Platform`: `App::new(gl, AppConfig)` then
  `app.frame(&mut platform)` once per display frame (the caller swaps).
  Logic runs at a fixed 60 ticks per second (the original's frame rate);
  `handle_event` / `tick` / `render` are public for scripted drivers.
- The SDL2 dependency of `crates/game` sits behind the default `sdl`
  feature; the library builds without it (`--no-default-features`), so the
  Android entry point can run the same `App` on its own `Platform`.
- `crates/render` got one addition: `Renderer::draw_overlay` (screen-space
  quads for menus, text, HUD and transitions).
- `crates/sim` is unchanged; game flow only uses its public API (icon data
  of placed units, `Context::path_states`, rule display list, turn
  outcomes).

## Save model (`baba-progress`)

- Files and keys follow the original (`saves.md` §2–§4) so the M9 importer
  can copy them group by group: `ba.ba` (global: `firsttime`, `lastslot`),
  `0ba.ba`–`2ba.ba` (per world: icon states by target stem, path states
  `<map>,<x>,<y>`, `<stem>_intro`, `Previous`, `leveltree`(+`_id`),
  `levelsurrounds`, `time`/`time_frames`, `end`/`done`, counter groups
  `<world>_prize|_clears|_bonus|_complete`, conversion groups), and
  `SettingsC.txt` (`[settings]`, `[keyboard]` SDL keycodes, `[gamepad]`,
  `[slotnames]`, `[savegame]`; unknown groups are preserved).
- `store::Store`: ordered ini store in the game's dialect (all line ends
  read; LF written, one empty line after each group).
- Directory: `$BABA_PORT_DATA_DIR`, else the platform's user data dir +
  `baba-port` (never the original's `Baba_Is_You`). Writes go through a
  temporary file and a rename.
- Our one extra key: `[port] nopad` (gamepad input disabled; the original
  keeps it in memory only, `saves.md` §4.1).

## Progression

- `map::MapLayout` holds the icon and path states of one map: `.ld`
  initial states plus icons made from objects by `level` specials
  (`progression.md` §3.6), saved states on top (§3.2 step 4), then
  re-propagation (§3.2 step 5).
- Winning (`MapLayout::complete`, §5.1): the original writes the save
  first and changes the map's units only during the return animation, so
  the neighbour opening and the hidden-chain flood fill both look at the
  states from *before* the win (the fill passes through tiles the same win
  just opened); results are merged keeping the larger state. On load the
  re-propagation changes states immediately and therefore does not reveal
  chains again; both behaviours are covered by unit tests.
- Gates (§4): counters (total spores, flowers, orbs, local spores) are
  compared on map load; newly met gates get the opening animation, gates
  opened earlier are open at once. Opening a gate runs the same unlock as a
  win from the gate's tile.
- Counters: a spore per first completion (per parent map and total), an
  orb per level won with its bonus, a flower when the parent map reaches
  `unlockcount` completed levels (the map's own state becomes 3),
  `<world>_complete` when every icon of the map is completed.
- Level tree (`tree::LevelTree`, §6): enter (cut back when re-entering an
  ancestor), leave, change (`customparent`, §7.4), the "Level X" label and
  level ids from icon style/number/`mapid` (§6.1–§6.2).
- Map loading (`play.rs`): hidden icons (state 0) are left out of the sim
  level; icon states and conversions (`<world>_<map>_convert`, §11.2) are
  applied to the source before `Level::start`; path states go in through
  `Context::path_states`; the cursor is moved onto the icon of `Previous`
  (§3.4).
- Entering (§7.1): tree push, `Previous`, `<stem>_intro`,
  `levelsurrounds` (also passed to the level for outer-level conditions),
  closing transition, title card (label + name + subtitle), level music.
  Several icons under the cursor open the "select the level" menu (§3.5).
- Return animation (§5.2): the map loads with the pre-win states and a
  timeline applies the changes (icon completed at 70, rings of the reveal
  every 10 frames from 110, the four neighbours from 130), with the roll,
  win/clear, pop and whoosh sounds; the map is rebuilt from its source at
  each step (input is blocked, so nothing is lost). "Map clear!" banner on
  a clear. The clear showcase on the `unlocklevels` maps (§5.3, M5) is
  pending state (`Pending::Showcase`): each map shown is one `Proceed`
  turn at full transition cover, the last one returns to the cleared map.
- First launch (`ui.md` §1.2): no menu; slot 0 starts the base world in
  `0level` with the intro cutscene (§10.4; `crate::cutscene::intro`, input
  released at its end, hold a key to skip); its flags are recorded. The
  "watch the intro?" prompt appears for base-world starts in slots that
  have not seen it; Yes plays the intro when the world starts in its
  first level.
- Worlds: the pack list offers the base world first, then every folder
  with a `world_data.txt` except `levels`, `baba_m`, `debug`, `new_adv_m`
  (`ui.md` §3.3). A world without a saved tree starts inside its
  `firstlevel` if that is not completed (§7.5); Continue restores the saved
  tree.
- Endings (§10): END in the base world sets `end=1` and plays the scripted
  sequence (blossoms to collect, the closing flower, the island, THE END,
  the logo credits) before the short credits; ALL IS DONE sets `done=1`,
  unlocks the editor's advanced words (`[editor] special=1`), plays the
  rising-island sequence and rolls credits; `customparent=<win>` and END
  or DONE in other worlds set the pack's flag, show the victory banner and
  roll its credits. Details and timings: `docs/presentation.md` (M5b).
- LEVEL IS AUTO: while it holds the presenter submits `Turn::Auto` (the
  sim's `Input::Tick`) every `rhythm` + 1 frames (measured: 11 at the
  default 10), in single player and per splitscreen view.
- Signs (`sign,…` / `sign_lang=…` specials): their three lines show while
  a YOU/YOU2 unit is on or beside the tile.

## Screens and menus

- Boot card (our own attribution, hold to skip), title screen (the logo in
  large letters over a dimmed level, the version), main menu as the PC
  variant (`ui.md` §2; Level editor: see "Level editor (M8)"; Play levels:
  see "M9" below), slot select with stats (time, spores, flowers, orbs, ended
  marker), erase with confirmation (No preselected), pack select with
  paging, switch slot, slot rename (text entry), settings, controls,
  keyboard and gamepad setup (rebinding captures the next key/button and
  stores it like the original), default bindings, disable gamepad,
  language (every `Data/Languages` file, two columns), credits, exit.
- Pause (`ui.md` §8.1): level label and name, Resume, Return to map
  (enabled per §7.3), Restart (confirmation unless "Do not prompt on
  restart"), Settings, Return to menu, and the active visible rules as
  readable lines with duplicates merged.
- Settings (`ui.md` §5): music/sound volume, input repeat delay,
  fullscreen, grid, wobble, particles, screenshake, contrast, blinking,
  restart prompt, zoom (None = integer scale, Default = integer-ish fit,
  Stretch = fit); every change is applied and saved at once. Grid,
  particles, shake, contrast and blinking are stored but only take effect
  with M5 presentation.
- Menu mechanics (§4): a stack of menus rebuilt from the game state every
  frame (language switches show immediately), rows and columns, disabled
  items skipped, sliders on left/right, escape items.
- Text: the game's TrueType fonts (`merged.ttf`; `mergedsc.ttf` for
  Simplified Chinese) rasterised on demand with `ab_glyph` (pure Rust,
  lazy outlines) into the shared atlas, so CJK costs only the glyphs used;
  full-width punctuation missing from the font falls back to ASCII. Markup
  `$x,y` colours and `#key` insertions are honoured. Pixel-font languages
  are drawn upper-case with the TTF: the original's pixel font is unnamed
  engine art in `Assets.dat` (open item for M5).
- Map HUD: hovered level name top left, spores/flowers (and orbs) top
  right. Tutorial `controls` specials draw key caps with the *current*
  binding and the action name.
- Transitions: a simplified blob iris in colour (1,0) (`presentation.md`
  §11), action at full cover; restart uses the fast variant.

## Input

- Keyboard bindings are SDL keycodes (US-layout conversion from our
  physical keys); Escape always pauses / goes back. Gamepad bindings use
  the original's value format (`a0`, `dpad`, button indices) with our own
  positional numbering (0 A, 1 B, 2 X, 3 Y, 4/5 shoulders, 6 Back,
  9 Start), which makes the original's defaults sensible.
- Repeat (`ui.md` §6.1): a held direction (the most recent one) and a held
  undo repeat every `delay` frames.
- On maps confirm enters the level under the cursor; wait is a turn only
  where confirm has nothing to enter (they share Space/Enter by default).
- Pad Select/Back (handhelds, Android pads) acts as Escape unless it is
  bound to an action.
- Touch and mouse: a touch down (`Event::Touch`) or a left click activates
  the item under it in the topmost menu (`Menu::hit`, same layout as
  drawing; sliders step down/up by the half tapped); on the title a tap is
  confirm, in credits back, on the boot card skip. In play, touches go to
  the platform's on-screen controls, which the entry point shows only
  while `App::wants_touch_controls()` (playing, no menu).
- Text prompts outside the editor (slot name, level code, online room
  code; `crate::osk`, `app/prompt.rs`) show an on-screen keyboard of menu
  items (digits, letters, `-` for codes; Shift and Space for names; Del,
  OK, Cancel), so pads and touch can fill them in. Pad: Confirm presses
  the selected key, Undo deletes (cancels on an empty text), Pause
  submits, Restart toggles shift, Back/Select cancels. While a prompt is
  open, `Input::text_entry` makes keyboard keys type instead of running
  their bindings (Space and WASD would otherwise confirm and move); Enter
  submits, Escape cancels, arrows move over the keys.
- List menus on screens narrower than 16:9 are drawn bigger, into the
  height the 854x480 canvas leaves free (`Menu::ui_for`; 854x480 and
  16:9 screens unchanged).
- Android's lost GL context: `App::frame` re-creates its renderer on
  `Resume { gl_lost: true }` and re-uploads the atlas.

## Audio rules (`audio.md`)

- Music: level/map track (`localmusic` from the world's `Music/`; empty =
  `baba`), kept playing when the next screen uses the same track; `menu` on
  the title, `ending` in end credits. Volumes are linear bus gains.
- Music state (§3): no YOU-like unit fades the track out, a player brings
  it back; maps with a cursor always count as having a player.
- Turn sounds (§4.1) from the sim's events: removal slot (sink → plop,
  boom/weak/defeat/eat → pop, melt → burn, bonus → silent), turn slot
  (unlock → lock, done → done, any committed turn → move), `_short`
  variants when the same family played the turn before; `bonus`,
  `rule1..5`, `infinity`, `doneall_c`; undo, restart, confirm and menu
  clicks; the unlock/gate sequences' roll, win, pop, whoosh and gate sounds.
- Sound effects are Ogg files in `Assets.dat` stored by index; their names
  are read at run time from the player's executable (Linux
  `bin64/Chowdren`, Windows `Baba Is You.exe`, macOS
  `Contents/MacOS/Chowdren`: a string hash table plus a parallel index
  array, located by searching for a known name; `baba_formats::exe`).
  Copies without the executable stay silent for effects.

## Scripted flow test

`scripts/dev/flow-test.sh [--release]` runs `crates/game/tests/flow/*.drive`
in order on a fresh `scratch/m4/flow-data` and writes screenshots and
comparison diffs to `scratch/m4/flow/`. The drive language (documented in
`crates/game/src/drive.rs`) sends key presses at 60 ticks per second,
waits until input is accepted, solves the current level from the corpus
(`ref/tas/corpus/corpus.tsv`, also alternative endings), walks the map
cursor to an icon, jumps to a level (`goto`), presses pad buttons
(`pad`), taps the screen (`tap`), and checks the state, the event and audio
logs and the files on disk. `--window` runs a script on the real SDL
window instead of headless EGL (`--size`, `--render-every N`): used under
Wine (`docs/platform-windows.md`, all three scripts pass) and under qemu
for PortMaster (`docs/platform-portmaster.md`).

1. `1-fresh.drive`: first launch straight into `0level` with the intro
   (two shots of it, then skipped by holding a key), solve it, return
   with the unlock animation, check `0level=3`, spores, `Previous`, the
   tree, `firsttime`, `[savegame]`, pause menu on the map.
2. `2-menus.drive`: title, Continue/Start, slot and pack menus, the map,
   walk to `1level` and back, enter `0level` (title card), solve, enter
   `1level`, pause, return to map, settings (volume saved), back to title.
3. `3-worlds-settings.drive`: New Adventures (first level, then its map),
   the Museum map, Japanese and Simplified Chinese, key rebinding and
   reset, credits, END (`203level`, FLAG IS END: the END sequence, the
   island, the logo credits) and ALL IS DONE (`198level`, its sequence),
   erasing a slot.

Comparisons with captures of the original at 854×480 (mean absolute
difference / share of pixels differing by more than 8):

| screen | result | main differences |
|---|---|---|
| main map after `0level` | 3.0 / 4.6 % | icon art (our squares and digits), TTF HUD text, cursor wobble |
| New Adventures map | 2.6 / 4.6 % | reference had `42level` completed |
| Museum map | 2.1 / 2.7 % | reference had `101level` completed |
| level title card | 3.5 / 1.8 % | TTF instead of the pixel/handwritten fonts |
| pause in `1level` | 8.3 / 10.4 % | reference is a different level/tree; fonts |
| title screen | 27.4 / 37.7 % | logo art and the title's background level differ |

## Multiplayer (M6, local)

Up to 12 local players in the three modes of `docs/ARCHITECTURE.md`.
Code: `src/session/` (turns, apply, saves, the running session),
`src/join.rs` (devices, seats, join screen), `src/split.rs` (layout),
`src/vpads.rs` (virtual controllers for tests).

### One path for every change: turns

- `session::Turn` is everything that changes an instance: `Move {dir,
  you}` (0–3, 4 = wait; `you` 1 = YOU, 2 = YOU2 falling back to YOU),
  `Undo`, `Restart`, `Enter {target, unit}`, `Leave`, `Proceed`.
  `encode`/`decode` give ≤ 64 bytes (M7 stream entries, `docs/net.md`).
- `Play::apply(turn, env, slot, texts)` is the only way an instance
  changes (single player included: `App::apply_turn`); it returns
  presentation effects (`Fx`: turn sounds, undo sound, banners, `Won`,
  `ProceedAfterTransition`, `Credits`, `Loaded`) and whether the slot or
  shared progress changed. What a win, a level conversion or an ending
  leaves to do is state (`Play::pending`), carried out by `Proceed`.
  A level played on its own (`Play::single`, M9) takes the same path:
  `Play::single` marks the instance, `apply` records its results and
  leaves `Pending::LeaveSingle`, and `Proceed` yields `Fx::LeaveSingle`
  (the presenter returns to the menus).
  Presentation (transitions, title cards, the victory delay, menus) only
  decides *when* the local machine submits a turn: e.g. the victory banner
  runs, the closing transition reaches full cover, then `Proceed`.
  A running map animation is finished at once when a turn arrives
  (`Play::finish_anim`; its end state never depends on timing).
- `Play::state_hash` covers the tree, the current level's sim state and
  what is pending (desync checks). A test replays the encoded turns of a
  session into a fresh instance and gets identical hashes and save text.
- Not turns: the debug `goto`, and settings.

### Players and devices (`join.rs`)

- A seat (P1–P12) has a colour and a name (12 each, distinct), a save
  slot and world (mode 3) and an input source: a controller (SDL instance
  id; hot-plug) or a keyboard half. In multiplayer the keyboard is always
  two players with fixed bindings: **left** W A S D, E / Space wait and
  confirm, Q / Z undo, R restart, Tab pause, Esc back; **right** arrows,
  Enter wait and confirm, Backspace undo, 0 restart, P pause. Controllers
  use the gamepad bindings of the settings (even if "disable gamepad" is
  on). Each seat has its own key repeat.
- **Arrival order**: every platform event becomes commands at once, in
  one queue of `(seat, command)`; the session handles them in that order
  (= SDL event order; held-key repeats are added per tick in seat order).
  SDL reports same-frame controller presses newest controller first; the
  test `chord` presses one button on many pads in one frame and checks the
  turns follow SDL's delivery order.
- Joining: an unassigned device presses A / Start, E / Space or Enter.
  Unplugging a controller marks the seat "unplugged" (its view stays);
  the next join from any free device takes that seat over (rejoin).
  "Leave game" in the pause menu removes the seat (its game is saved) and
  remembers it: the same device joining again gets the seat and its game
  back. A new device joins mid-session as a new seat (mode 1: into the
  co-op; mode 2: a new instance on the shared save; mode 3: the lowest
  free slot). Controller Back/Select zooms the player's own view; F2
  cycles the zoom.

### Join screen

Main menu → Multiplayer (last item). Twelve cards (4 × 3); a joined
player's card has rows: Mode (host = the first seat), Save and World
(host in modes 1/2; everyone in mode 3, slots kept distinct), Colour,
Name, Ready. Up/down picks a row, left/right changes it, A toggles
ready, B un-readies or leaves; Esc / B on a free device with nobody
seated goes back to the title. When everyone is ready a short countdown
starts the session.

### The modes (`session/multi.rs`)

1. **Co-op**: one instance, one full-screen view, every seat's commands
   are turns in arrival order; seats alternate YOU / YOU2 (YOU2 falls back
   to YOU, so normal levels are driven by everybody). Undo is shared and
   immediate (it undoes the last turn, whoever made it). Restart: the
   presser confirms (unless "do not prompt"), then it restarts for all.
   Anyone moves the map cursor and enters levels. Pause is global: the
   opener drives the menu, the others' inputs are ignored meanwhile.
   Input during a transition / title card / animation / victory delay is
   dropped, as in single player.
2. **Shared save**: one instance per seat on the session's slot. Wins,
   gates and conversions are written to the slot by the instance that
   made them; every other instance standing on a map of that world
   re-reads it at once (`Play::refresh_map`: icons and paths open
   instantly with a whoosh, newly met gates with their animation, the
   cursor stays) or as soon as its own animation ends. Map undo history
   is lost on such a refresh. Level tree, `Previous` and
   `levelsurrounds` are per player (`Play::own_nav`, a private copy of
   the slot used only for those keys); the first seat's go to the slot,
   so single-player Continue resumes where P1 was. Clearing a level twice
   (two players) counts once. Play time counts once (first view).
3. **Own games**: one instance per seat on its own slot. Slots: the three
   normal slots plus nine extra slots `3ba.ba`–`11ba.ba` in the data
   directory (same format, port only; the original and the save import
   use 0–2), so twelve players can each have a save; a slot can be used
   by one player at a time.

Endings (END, ALL IS DONE, a pack's end) in a session show a banner and
the view stays in the level (single player rolls the credits). The pause
menu has Resume, Return to map, Restart, Leave game, and End session
(host only; saves everything and returns to the title); settings are
changed outside sessions.

### Splitscreen (`split.rs`) and drawing

- Views tile to the grid that gives every view the largest scale for a
  33 × 18 level (`baba_render::tile`): 1×2 / 2×2 / 2×3 / 3×3 / 4×3 (or
  3×4) on 16:9; the same counts on 4:3 and 640×480 with taller cells
  (12 views at 640×480: 3 × 4 cells of 210 × 117). Each view fits its own
  level (integer scale where it fills the cell well, fractional below 1×).
- Zoom: one view takes the top three quarters (or the left three
  quarters on very wide screens), the others tile the strip; everyone
  stays visible.
- Per view: a frame in the player's colour, player tags, the map HUD,
  banners, title cards, menus (text on the view's level area, scaled
  down for small views), its own transition. Co-op's single view looks
  like single player plus the tags.

### Audio with many instances

One music track: the zoomed view's, else the first view's (with its
no-player fade). Turn sounds come from every instance; per tick each
sound family (name without variant digits / `_short`) plays once and at
most six sounds play (`AudioDirector::set_mixing`; dropped ones are
logged as `sound-skip:`). `_short` variants follow each instance's own
previous turn (the sim's sound slots). Each view has its own effects
(`LevelFx` in its `Play`: tweens, particles, shake, scroll, rotation),
fed by the turns it applies.

### Tests

- Unit: `join` (arrival order across devices and seats, per-seat repeat,
  unplug and takeover, 12 seats, the join screen), `split` (1–12 views
  on 1920×1080, 1280×720, 854×480, 1024×768, 640×480, with and without
  zoom: inside the screen, no overlap, equal tiles), `session::turn`
  (encoding), `session::saves` (extra slots), `session::tests` (on the
  install: shared-save propagation to a player on the map with the cursor
  kept, per-player navigation, no double spore; replaying encoded turns
  gives identical hashes and saves).
- Drive scripts `crates/game/tests/mp/{coop,shared,own}-{2,4,12}.drive`
  via `scripts/dev/mp-test.sh [--release]` (fresh data directory each;
  output `scratch/m6/mp/<script>/`): SDL virtual controllers through the
  drive commands `pads`, `unplug`, `pad`, `chord`, and per-seat `pN moves
  / solve / walk / press / idle / expect level / expect icon`, `expect
  same / differ pA pB` (instance hashes). Keyboard halves take part in
  the 4-player scripts. Screenshots include the join screen, 12 views
  at 1920×1080 and 640×480, zoom, per-player pause, unplugged seats.

## Online (M7)

Up to 12 players over Warpzone in the three modes, any mix of machines and
local controllers; protocol and replica details in `docs/net.md` ("The
game on baba-net"). Code: `app/online.rs` (screens, the frame loop),
`session/online.rs` (replica), `session/multi.rs` (the same session
presenter as local play, with a `NetSide`).

### Getting into a room

- Main menu → **Online** (below Multiplayer), or on the multiplayer join
  screen **O / F4** (keyboard) or **Y** on a controller without a seat: the
  online menu, with the join screen's players coming along. It connects to
  Warpzone (`WARPZONE_SERVER` or the public instance; a generated account
  name) and offers: Host a private room (code + invite link), Host a
  public room (listed), Join with a code (text entry), Browse rooms
  (public rooms, newest first, host / players / mode / world, other
  versions marked), Return.
- Launcher: `--warpzone-join <room id | warpzone://join/... invite link>`
  joins at once, `--warpzone-host` opens the online menu; the install may
  come from `WARPZONE_DATA_BABA` (the launcher's `[[data]]`).
- Refusals are spelled out: another baba-port build ("This room runs
  another version…"), other Baba Is You data ("Your Baba Is You data
  differs from the host's…"), a full or closed room, a stale room.

### The room

- The join screen of this machine's players (controllers and keyboard
  halves join, colour, name, ready; up to 12 per machine) drawn smaller on
  the left; the room code and invite link (host), mode and world, and the
  machines (name, host, players, ready, round-trip time) on the right.
- The host's first player has the Mode, Save and World rows (modes 1/2:
  the host's save is what everybody plays on; mode 3: every player picks
  an own slot and world of this machine). Guests see the host's choice.
- Start: when every machine has players and all are ready (at least two
  machines), a short countdown; **F5** (host) starts at once, machines
  without players then watch. Esc with no players seated leaves the room.

### Playing

- The views are those of local multiplayer: mode 1 one full-screen view
  with every player's tag; modes 2/3 every player's instance, this
  machine's players large and the others in a strip (**F3**: equal tiles;
  F2 / Back zoom as before); 12 views at most.
- Turns go to the stream; the instance changes when they come back (the
  host's own players and own streams at once, guests after a round trip;
  mode-1 prediction for far guests). Transitions wait at full cover for
  their turn; another machine's level change shows the opening half.
- Every instance shown (local or remote) gets its effects (`LevelFx`,
  M5) from the turns the replica applies through `Play::apply`; the
  predicted level is drawn with the confirmed one's effects; a quiet
  catch-up (late join, resync) starts the effects afresh. Pending steps
  (Proceed after a win, the map-clear showcase) are sent by the machine
  responsible for the view (`net_tick`).
- Pause (per player): Resume, Return to map, Restart, Leave online
  session, End session (host). Esc on a watching machine leaves.
- Status line at the top: "The host left: regrouping…", "m2 hosts the
  session now", "m1 left", "… out of sync", "Out of sync: replaying the
  session…" / "Back in sync", "Catching up…" (late join).
- A controller plugged in mid-session joins as a new player (admitted by
  the host; mode 3 starts on the lowest free slot of this machine).
- The session ends (host) → everybody back in the room, with "Session
  over: every machine agrees on every game." (or the disagreement).
  Saves: mode 1/2 on the host's slot file, mode 3 on each player's own
  machine; nothing of other machines is written.

### Tests

- Unit: `session::online` (settings, mode-3 save chunks, progress events),
  `session::online_tests` (two machines over `MemNet` with the real
  replica: co-op turns from both, mode-2 progress reaching the other map
  through the host and a Refresh, own games side by side; identical hashes
  of every stream), `split::big_and_small`, `turn` (Refresh); baba-net
  `players_added_after_joining_and_own_events`.
- `scripts/net/online-test.sh`: the matrix in `docs/net.md`; screenshots
  in `scratch/m7/online/<case>/m<i>/`.

## Not done yet

Intro cutscene and the scripted ending sequences; LEVEL IS AUTO timer;
signs; map icons of ancestors (`map_level`); level editor (M8).
Presentation (particles, transitions, bitmap fonts, engine art, undo hint,
map-clear showcase) moved to M5: `docs/presentation.md`.
Intro cutscene and the scripted ending sequences; map-clear showcase on
`unlocklevels` maps; LEVEL IS AUTO timer; undo/restart hint; signs; map
icons of ancestors (`map_level`); particles, blob iris fidelity, menu
slides; the pixel font.

## M9: save import, Play levels, level codes, packs

Code: `import.rs` (finding and importing the original's save),
`levelcodes/` (protocol, transports, storage, `upload.rs` behind a
feature), `packs.rs` (pack discovery, the user `levels` world),
`app/extras.rs` (the screens, a child module of `app` so `app.rs` only
needs a few hooks), `crates/http` (`baba-http`: blocking GET/POST with a
timeout over the workspace's ureq/rustls; ring on desktop, rustls-rustcrypto
on Android/aarch64 like `baba_net::tls`).

### Importing the original's save (`saves.md` §5)

- Where: Linux `$XDG_DATA_HOME` or `~/.local/share` + `Baba_Is_You`, the
  Flatpak Steam sandbox, and Proton prefixes
  (`steamapps/compatdata/736260/pfx/.../AppData/Roaming/Baba_Is_You`);
  Windows `%APPDATA%\Baba_Is_You`; macOS `~/Library/Application
  Support/Baba_Is_You`. Steam Cloud has no separate copy: its Auto-Cloud
  entries are `Baba_Is_You/<file>` under the "AppData Roaming" root, which
  Steam maps to exactly these folders (checked on this machine: the cloud
  cache hashes equal the local files). `BABA_IMPORT_FROM=<dir>` overrides
  the search (tests; `flow-test.sh` points it at nothing so a real save on
  the developer machine does not change the first launch).
- When: on the very first launch an original save with progress is offered
  ("Yes" imports, "No" continues into the normal first launch); any time
  from Settings → "Import Baba Is You progress" (disabled while playing).
  The screen lists each slot's completed levels, spores and flowers and
  which of our slots get replaced.
- What: every slot file present replaces ours, all groups copied (custom
  packs keep their progress under the folder name), `m_lastpack` dropped,
  `_prize` totals recomputed as the sum and `_clears`/`_bonus` totals as
  the count (mismatches are logged and shown). Settings: volumes, delay,
  toggles, zoom, language (if installed), keyboard (SDL keycodes, used
  directly), gamepad values we understand (`a<N>`, `dpad`, indices < 128;
  others keep ours, noted), `[gamepad_editor]`, `[gamepad_currobjlist]`,
  `[editor]`, `[slotnames]`, `[savegame]`, `[get_d]`, `[get_u]` verbatim;
  `[log]` skipped; our `[port]` kept. `ba.ba`: `lastslot`; `firsttime`
  set. The original's own single levels (its install's `Worlds/levels`) are
  copied into ours. The source is only read.
- Cross-check (2026-10-18): a copy of the owner's save imported by
  `1-import.drive`; the original run headless with the same copy
  (`tools/baba-home`, restored afterwards). Continue → the Island map
  (`207level`) and Return to map → Solitary Island (`106level`) show the
  same icons, paths, cursor and 30 spores / 2 flowers in both
  (`scratch/ref/m9-orig-0{3,5}*.png` vs `scratch/m9/shots/0{3,4}*.png`);
  `examples/map_states.rs` prints the per-icon states. Differences seen are
  rendering (M5): icon art; the `16level` icon (style -1, custom picture)
  is not drawn by us; the original's map HUD shows the local spore count.

### Play levels (`ui.md` §8.4)

Main menu → Play levels: "Current save slot", Play custom levels, Play
levelpacks (the pack list), Get new levels, Level history, Upload level
(disabled unless built with `upload`), Change slot name, Return.

- Custom levels: `<data>/worlds/levels/*.ld` (ours, downloads) then the
  install's `Worlds/levels` (read only), 8 per page, name + code/author, ✽
  when won, preview `.png` and subtitle/author of the selected one. A level
  plays on its own (`Play::single`: tree = the level): the pause menu has
  "Return to level list" and shows the code; a win records
  `[levels] <stem>=3` and, for coded levels, `[levels_won] <code>=won,
  bonus,end,done,converted`; END / DONE / level conversions are stored in
  `levels_end_single` / `levels_done_single` / `levels_converts_single`;
  then the victory banner and back to the menus it was started from
  (recorded in `Play::apply`, like every other turn result).
- Packs (`worldpacks.md`): folders of `<data>/worlds/` and the install's
  `Data/Worlds/` (ours win on a name clash), base world first, hidden ones
  left out, unnamed → "Unnamed levelpack", packs without a start level
  disabled. Resource lookups for packs outside the install pass the
  absolute folder (`WorldInfo::res_key`) so sprites, palettes, images and
  music come from the pack. Packs with Lua (`Lua/*.lua` or `mods=1`) show
  "(Lua)"; starting one shows a one-time notice (`[port_luanotice]` in the
  settings) and plays the levels with the standard rules.

### Level codes (`levelcodes.md`)

- Get new levels: "Use level code" (text entry, upper-cased, hyphen added
  after 8 characters), "Featured levels", "Level history" (two columns,
  click = download). Downloading shows "Downloading...", then "Level
  found!" (name, author, subtitle, code) with Save the level (next free
  `<N>level` in `<data>/worlds/levels`, counters in its `world_data.txt`)
  → "Level saved!" (Play / View custom levels / Return), Play now (the
  temp level `<data>/temp/temp.*`), Nevermind. The code goes to `[get_d]`
  (15 newest, deduplicated).
- Featured: 6 levels per page, page arrows ±1/±5, thumbnail + code +
  subtitle + tagline of the selected entry; thumbnails are kept in
  `<data>/temp/thumbs/` (fetched once).
- Errors are the original's strings (`invalid`, `timeout`, `usagelimit`,
  `outdatedversion`, `spriteerror`, `networkerror`, or the server's text on
  HTTP 500 — an unknown code answers 500 `error`).
- Learned from the real server (one list page, one level, one bad code):
  `difficulty` is the subtitle, `desc` the tagline; files and thumbnails
  live on S3 (`babalevels3`); `.l`/`.ld` are stored zlib-wrapped and are
  inflated; the `.ld` has no `levelcode` (we add it); page 0 is the first,
  38 featured pages.
- Requests run on a worker thread; the UI polls. `BABA_LEVELSERVER`
  overrides the server, `BABA_LEVELCODES_CACHE=<dir>` records 200 responses
  by URL and replays them, `BABA_LEVELCODES_OFFLINE=1` never goes online
  (the M9 drive test runs this way from `scratch/m9/server-cache`).
  `examples/levelcodes.rs` fetches into the cache from the command line.
- Upload / report (`--features upload`, off by default): multipart body
  with the original's boundary and a deflate zip of `level.ld` + `level.l`,
  the code written back into the `.ld` and `[get_u]`; the level must be
  won in a test play first. Only exercised against the local mock
  (`levelcodes/mock.rs`); never against Hempuli's server without the owner's
  approval.

### Test

`scripts/dev/m9-test.sh` (needs a copy of an original save in
`scratch/m9/orig-save/` and the recorded responses in
`scratch/m9/server-cache/`): `1-import.drive` (offer, import, every group
checked on disk, Continue, Return to map) and `2-levels.drive` (enter a
code, found/save/play, pause in a single level, featured list, network and
invalid errors, a custom level won from the list, the Lua notice and a Lua
pack from the data dir). 17 screenshots in `scratch/m9/shots/`.

## Level editor (M8)

Code: `src/editor/` (the editing model, independent of the app:
`model.rs` level/palette/quickbar/files, `tools.rs` geometry and the
floating selection, `undo.rs`, `view.rs` drawing and button layouts,
`pad.rs` gamepad groups, `tutorial.rs`, `files.rs` folders and ids) and
`app/editor_ui.rs` (screens, menus, prompts, test play; a child module of
`app` like `extras`, so `app.rs` only has hooks: `Mode::Editor`,
`MenuId::Editor` / `Act::Editor`, the event queue, `tick_editor`,
`render_editor` / `draw_editor`, the pause item and the end of a test).
Behaviour follows `editor.md` (cited, not quoted).

### Files and compatibility

- Levels live in the port's data dir: single levels in
  `<data>/worlds/levels/`, packs in `<data>/worlds/<N>world/` (`N` = first
  free number ≥ the folder count, §3.3/§10.2). The install is never
  written: install packs open as a copy in the data dir.
- Saving writes `<stem>.l` (always three layers, the border as `edge` in
  layer 1), `<stem>.ld` and a 216×112 preview `<stem>.png` (the interior
  scaled to fit). `.ld` comes from `LevelInfo::to_ini` in `baba-formats`:
  typed `[general]` fields over the loaded keys (unknown keys and
  sections kept), `[images] [levels] [icons] [currobjlist] [hotbar]
  [specials] [paths] [tiles]` rebuilt (stale records dropped, counts
  written, CRLF). A palette entry's slot gets the overrides that turn the
  slot's base definition into the catalogue object, field by field like
  the original (`ObjectChange::between`); the slot is the base slot of
  that name if free, else the first free `objectNNN`. Grid positions
  follow the original's growing shells (row k, then column k+1, at most
  10 rows; checked: the same arrangement as the original's palette).
- New level ids: first unused number ≥ max(level count, `[levels]
  levelid`), counters advanced (`packs::next_level_stem`).
- Checks: every level of every installed world loads into the editor and
  re-saves to the same grid, metadata, palette and overrides
  (`editor::model::tests::install_levels_round_trip`, 600 levels). A level
  and a pack made by `1-create.drive` / `3-pack.drive` were copied into a
  private copy of the original (`tools/baba-m8`, home
  `tools/baba-home-m8`, display `:71`, `scripts/dev/ref-run.sh` with
  `REF_RUN/REF_HOME/REF_OUT/REF_LEVELS=1`): the original's editor lists
  and opens the level (same palette, quickbar, rules lit), test-plays it
  to "Congratulations!"; the pack shows in its pack list, its map has the
  icon (numbered, opened), the path and the cursor, the level is entered
  and won and the save records `0level=3` (captures
  `scratch/m8/ref/o24*`, `o27*`, `o42*`–`o45*`).

### Screens

- Start (`§2.1`): Edit custom levels, Edit levelpacks (extended), Get new
  levels / Level history (the M9 screens), Help & general settings,
  Return. Help (`§2.2`): pad line, Default gamepad controls, Editor
  control settings (rebind `[gamepad_editor]`), Tutorials and shortcuts,
  toggles (button hints, visual effects, custom editor music, advanced
  words, extended features), default author.
- Level list (`§3.1`): return, pack name/author, first level / world map
  modes, sort by name / type, create, levels 8 per page with the selected
  level's preview, search / remove search, Delete key → confirm. Pack
  list (`§3.3`): create, ours first by author, install packs marked
  "(copy)".
- Main view in the original's arrangement (captured): title
  "NAME (STEM)", quickbar top right, layers and the seven tools down the
  right edge, Test / Swap / Undo / Save · Objects / Settings · Menu along
  the bottom, the level full screen underneath (`§4`). Menus veil the
  level in its outside colour and hide the bars, like the original.
- Palette (`§5.4`): fixed 15×10 grid (full view by default, pairs view
  toggle), tools, search / clear / tags, add / remove / edit, close, the
  direction cross, our brush column (objects, levels, paths, specials,
  set cursor, path setup). Object list (`§5.5`): 15×4 per page,
  search, tags (all must match), paging; click adds and returns,
  Ctrl+click / Ctrl+Enter keeps browsing. Adding always brings the
  noun/text partner and companion words; the 150 cap holds.
- Edit object (`§5.6`): sprite picker (root and pack sprites, search,
  the catalogue's animation style adopted), name (rules of §5.6, the
  partner renamed too), type, base / active colour (picker on the level's
  palette), animation style, text type + manual stepper, z-layer, reset.
- Level settings (`§7`), size (`§7.8`: steppers ±1/±5, presets, Apply;
  1–33 × 1–18 interior; objects keep their cell from the top left, the
  rest is dropped — our choice), music / effect / palette lists (pack
  files marked), AUTO delay stored as `[general] rhythm` (our reading of
  the engine key), map setup (`§13.4`: level type, clear limit, return
  to parent / victory / a chosen level).
- Level menu (`§8.1`): close, test, upload (enabled only in `upload`
  builds and once saved), themes, return to list / editor menu (unsaved
  confirm), delete, copy (name + 1). Quick menu for pads (`§8.2`).
- Themes (`§5.8`): load (install and pack themes; clears the level),
  save (overwrite confirm, `<N>theme.txt`, `themecount`), delete (pack
  themes).
- Map editing (`§13`): level icons (target from the pack, icon / clear
  colour, hidden / normal / opened, symbol none / number / letter / dot /
  custom picture from `[icons]` with a picker of `icon_*` and sprite
  frames, number), paths (object = the palette's `line`, added when the
  Paths brush is chosen; hidden / visible, gate type and requirement),
  specials (free text), the map cursor (`selectorX/Y`).
- Text prompts (`§16`): typing with Shift for capitals and punctuation,
  plus an on-screen keyboard (pads, touch); limits and sanitising of
  §16, empty names become `EMPTY`.

### Editing

- Tools (`§6`): pen and eraser strokes (cells between mouse samples are
  filled), line / rectangle / filled rectangle with a live preview, flood
  fill with a preview, select (lift, ←/→ rotate, ↑/↓ mirror, paste at the
  bottom-right cell, Shift / pad undo = ignore empty, Escape or another
  tool drops it), Ctrl+drag one object, right click copy (with facing),
  middle click cut, Ctrl+middle clear tile, WASD move all (wrapping),
  F directions, I/U/Y/Q quick picks, digits pick quickbar slots, wheel /
  Ctrl+wheel (swap) / Shift+wheel (tool), F4 words → one-row selection,
  R object list, E over an icon edits its level, Backspace back to the
  list, Ctrl+S / Ctrl+Z / Ctrl+Delete.
- Undo (`§11`): one group per action (stroke, span, fill, paste, drag,
  move all, map thing); a group stores the grid and map records from
  before the action, so undo reverts exactly what the original's records
  revert. Cleared on opening a level and on palette changes; no redo.
- Test (`§12`): the current state is written next to the level as
  `_EDITORTEST_.l/.ld` (so pack sprites, palettes and music resolve),
  loaded with `Play::single` (the normal `Turn` path; temp, nothing
  recorded) and deleted at once; no title card. Win / END / DONE / level
  conversion end the test after the victory banner and return to the
  editor; the pause menu has "Return to editor". Like the original, a
  test saves nothing (the original writes `Data/Temp/temp.l`, captured).
  An upload build runs the upload after a win of the upload test.
- Gamepad (`§15`, `editor/pad.rs`): our defaults — d-pad / left stick
  move the cursor, right stick (or alt + d-pad) turns the facing, A
  place, X copy, B undo, Y palette, LB / RB quickbar, LT / RT tool,
  L3 drag (hold), R3 next layer, Start quick menu, Back = alt (alt + A
  swap, + X cut, + B clear tile, + L3 directions, + RT lock, + LT empty
  slot, + RB pick empty, + LB words, + Start save, + Y test). Imported
  settings use the original's values verbatim; `h0.N` (d-pad) bindings
  then win over cursor movement, as in the original. Palette / object
  list / pickers have their own group.
- Touch: taps and drags paint and press buttons; prompts have the
  on-screen keyboard; menus are tapped as elsewhere.
- Tutorial (`§14`): lessons 1–10 (lesson 1 is the first-time intro, run
  once, `[editor] firsttime`); slide texts from the language file (`_gpad`
  variants with a pad), our demo edits on a scratch level (BABA IS YOU /
  FLAG IS WIN in the intro, the tools demos of lesson 3, …), the menu a
  slide talks about shown behind it; Continue / Skip; the end returns to
  the tutorials list (captured).

### Test

`scripts/dev/m8-test.sh` (fresh data dir under `scratch/m8/`, first
launch done, no import): `1-create.drive` (new level, settings, objects
by quickbar / Swap / mouse, undo, test play to a win, save / leave /
reopen / save byte-identical, object list adds keke, theme saved),
`2-tutorial-pad.drive` (help, lesson 3, shortcuts, gamepad cursor /
place / copy / undo / alt+A swap / quickbar / triggers / palette / quick
menu, touch, select + rotate + paste, flood fill, directions),
`3-pack.drive` (pack, level, map with icon / path / cursor, world map
set, then played from Play levels: the icon is completed),
`4-compare.drive` (needs the captures in `scratch/m8/ref/`). Drive
commands for the editor are in `drive/ed.rs` (`key ctrl+s`, `type`,
`click`, `cell`, `paint`, `touchcell`, `trigger`, `pad select+a`, `snap`,
`expect snap / file / nofile`).

Screens against captures of the original at 854×480 (mean |d| / share of
pixels differing by more than 8; the same level loaded in both):

| screen | result | main differences |
|---|---|---|
| main view with the level | 4.9 / 8.0 % | button art (ours: framed text and drawn tool icons), TTF font, cursor |
| palette | 13.0 / 31.1 % | icon buttons vs our labels, cell frames, font |
| level menu | 11.9 / 20.3 % | button colours, font |
| level list | 14.1 / 18.7 % | thumbnails as square tiles in the original, ours a list |
| start screen | 14.8 / 24.6 % | button sizes, font |
| level settings | 24.2 / 41.9 % | the original's label / value text vs our two button columns |
