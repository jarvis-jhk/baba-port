# Presentation (M5): effects, engine art, fonts

How baba-port draws what the rules engine reports: animations, particles,
transitions, the original's bitmap fonts and UI art, and how these were
measured. Behaviour follows the private spec (`presentation.md`, its §15
holds the measured values); this file describes the port's code.

## Measuring the original

Everything the original's engine (not its Lua) decides was measured on the
original running headless on the private Xvfb (`docs/reference-capture.md`)
from a private run tree with a small probe script registered on the game's
own per-frame mod hook. The probe reads object fields (position, scale,
direction, visibility, values) every frame, logs engine calls with their
frame number and starts experiments on key presses (particle sweeps, shake
strengths, rotation, killing YOU, …). Window recordings (ffmpeg x11grab,
lossless) and screenshots are matched against the images of the player's
`Assets.dat` by exact shape (bitmap masks) to identify which engine image is
drawn where. The probe, the analysis scripts and all captures stay in
`scratch/` (git-ignored); nothing from the game is committed.

Key facts: engine time is frame based (60 Hz logic even when rendering is
slower), and every engine particle moves `velocity × 0.2` per frame.

## Code

| Module | What |
|---|---|
| `crates/game/src/fx/mod.rs` | `LevelFx`, one per level view: movement tweens, FLOAT bob, wobble and blink counters, background colour fade, shake, particles from the sim's events, continuous property particles, move puffs, animation counters (walk steps, turns, sleep), level rotation/scroll easing |
| `fx/kinds.rs` | particle kinds: engine images, speed range, angle deviation, decay, gravity, life; the continuous-effect table |
| `fx/background.rs` | level background particle styles |
| `fx/transition.rs` | the blob iris (close, covered, open) |
| `fx/vision.rs` | the 3D first-person view (projection per the spec; rectangles instead of the original's skew shader) |
| `art.rs` | engine images from `Assets.dat` by number, uploaded to the atlas on first use |
| `pixelfont.rs` | character → image tables of the small UI font and the large title font |
| `keycaps.rs` | keyboard key-cap images |
| `title.rs` | title screen plate and logo |
| `scene.rs`, `scene/icons.rs` | units with tween/bob offsets, colour properties and blinking, rule crosses, particles; map icons from engine art |
| `crates/sim/src/present.rs` | `Level::effect_sources(prop)`: units and empty cells that show a property's continuous particles |
| `crates/render` | `TileScene::scaled` (sprites scaled about their centre), quarter-turn rotations sampled nearest, `Renderer::draw_overlay_clipped`, `Renderer::draw_overlay_rotated` (quads turned about their centres) |
| `cutscene/mod.rs` | flowers (bud, opening petals, growth, scattering), our own petal/disc textures, `Cutscene` |
| `cutscene/intro.rs` | the intro on the first start of `0level` (scattered objects, word flowers, gliding into place, small flowers, the YOU flower, tiles) |
| `cutscene/ending.rs` | END in the level (trembling words, blossoms to collect, the growing flower), the island show (area flowers, spreading flowers, THE END, logo credits, scroll decorations), All Is Done (rising island, smoke, blob) |
| `overlays.rs` | shared by single player and every splitscreen view: title card (bitmap font), undo hint, HUD counters (engine art), sign texts, the blob iris, the 3D view; edge-anchored HUD on 4:3/1:1 |
| `scene.rs` | `Overrides` (cutscenes hide, move, recolour or take over units), the conversion poof, `HIGH_CONTRAST` palette swap |

`LevelFx` is driven by `on_turn` (the sim's `TurnOutcome` events), `tick`
(once per 60 Hz frame) and `sprites` (draw list). It owns a cosmetic RNG,
so particles and jitter never touch the simulation. Splitscreen views each
have their own `LevelFx` (it lives in the play session's current level).

Engine art is optional: without `Assets.dat` every lookup returns `None`
and the port falls back to its own shapes and the TrueType font.

## Comparison with captures (854×480)

Share of pixels differing by more than 8 (per channel), "wobble-matched" =
best of our three wobble frames per pixel (unit wobble start frames are
random in both games):

| screen | result |
|---|---|
| `0level` at start | 0.00 % wobble-matched (0.42 % plain) |
| main map after `0level` | 0.27 % wobble-matched (HUD counter icons, a hint label) |
| New Adventures / Museum maps | 2.30 % / 2.02 % (the captures have other completion states) |
| pause menu | 7.65 % (capture shows another level) |
| title screen | logo and buttons within 1–2 px; the background differs by its scroll phase |
| 3D view | about 25 % (walls drawn as front faces) |

## Second pass (M5b): cutscenes, rings, HUD, slides, contrast

Measured on the original in a second private run tree (values in the
private spec, `presentation.md` §15.20–§15.30): the probe now also logs
every object the engine creates for the scripts (flowers, credit tiles,
THE END, transition blobs), injects corpus inputs through the game's own
command function (to reach END / ALL IS DONE quickly) and gathers the END
blossoms; recordings are matched to probe frames by timestamp, shapes to
`Assets.dat` images.

- **Intro** (first start of the base world and "watch the intro?" → Yes):
  objects scattered, bobbing ±12 px with their own periods and swaying, in
  (1,1); word flowers at 160/530/900/1220 open at +70, scatter at +200
  (the word appears), the objects shake and glide in at +270 (30 % of the
  rest per frame); IS/STOP/PUSH/WIN small flowers, the pink YOU flower,
  floor tiles shaking then settling at phase-6 620, then input. Hold any
  key 60 frames to skip.
- **END**: smoke, trembling words, the YOU flower at the END tile, the
  words turn into blossoms (poof) at 220, the player walks to collect them
  (STOP/PUSH/PULL objects and the edge block), the flower grows and closes,
  iris → the island show on the credits screen (area flowers from
  `<world>_complete`, 85 spreading flowers from `endingflowers.txt`, THE
  END written on with its backing, `themetune`), iris → logo credits (the
  original's script with walking Baba), scroll with Keke/skull/flag/key/
  rocket tiles, Baba over "Thank you for playing!". Holding a non-arrow key
  skips to the island; Escape skips the show.
- **All Is Done**: 350 frames in the level, then the level drops away and
  `island_full` rises (3 % of the rest per frame), smoke at the mountain,
  the dark blob (5 px per frame), credits. Pack endings: victory banner,
  then the pack's credits.
- **Gate rings**: `requirement` symbols of the gate type spiral in to a
  24 px ring and turn until the gate opens; bonus particles in the gate
  type's colour.
- **HUD counters**: spore/flower/orb icons and the 8×16 digits from the
  engine art, on maps and `showhud` levels.
- **Conversion poof**: the removed unit's last image cut by white dashes
  for two frames.
- **Menu slides**: menus the original declares as sliding start a screen
  away and close 35 % of the rest per frame (slot/pack, play levels, most
  editor submenus; the editor's "disable visual effects" setting turns
  them off).
- **High contrast**: the setting swaps every level's palette for its
  `contrastpalette`, else `contrast.png` (exact on `0level`).
- **Editor art**: buttons with the cut corners in the level palette's
  editor colour (3,2), Return/Close in (3,1), information lines as text,
  the start screen's tall buttons; tutorial pointer arrows at the
  original's targets; the map's own icon pictures (`[icons]`) in Map setup.
- **Splitscreen**: every view has the blob iris laid out for its size, the
  bitmap-font title card, the undo hint, the 3D view, HUD art.
- **4:3 and 1:1** (PortMaster 640×480, 720×720): the title screen is laid
  out for its 600 px span (scale 1 at 640×480, 1.2 at 720×720), the HUD
  keeps to the corners at a readable size, the title card is widened.

Comparisons (854×480, share of pixels differing by more than 8):

| screen | result |
|---|---|
| `0level` with high contrast vs the original's | 0.43 % (wobble phases; 0.42 % without contrast) |
| main map after `0level` (HUD icons now engine art) | 2.53 % plain; HUD corner 9.25 % (one wobble frame, ±1 px) |
| editor start screen | 10.60 % (was 24.54 %) |
| editor level list / level settings / level menu | 17.90 % / 35.96 % / 23.59 % (were 18.70 / 41.53 / 23.60) |
| title card, New Adventures / Museum maps, pause | 2.27 % / 2.23 % / 1.94 % / 7.65 % (unchanged) |

Timelines checked against the probe logs frame by frame: intro events
(flowers, reveals, notes) at the same frames; END phase 2 at 30/250/430,
phase 3 `dididi` 30, THE END 700, backing 750, iris 1150, logo script
80–1100; All Is Done 350 / 120-frame rise / 200 / 220–420 / 400 / 1140;
AUTO turns every 11 frames at `rhythm=10`.

## Not done yet

Exact images and sway of the level background particles (the closest
engine art is used); the original's flower, blossom-absorb and island
sparkle images (our own petal/disc shapes in the right colours and
sizes); the sign text position (not captured: our band at the bottom);
"Congratulations!" / "Map clear!" banner images; 3D side faces.
