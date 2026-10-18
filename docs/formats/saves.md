# Saves and settings of the original game

The port can import progress from the original game (milestone M9). The
original's files are **read only** for us: never write, rename or lock them.
Reader: `baba_formats::save`.

## Location

| Platform | Directory |
|---|---|
| Linux | `$XDG_DATA_HOME/Baba_Is_You/`, default `~/.local/share/Baba_Is_You/` (`save::default_save_dir()`) |

(Other platforms keep the same files in their per-user application data
directory. `steam_autocloud.vdf` belongs to Steam Cloud, not to the game.)

| File | Content |
|---|---|
| `0ba.ba`, `1ba.ba`, `2ba.ba` | the three save slots |
| `ba.ba` | slot bookkeeping |
| `SettingsC.txt` | options and input bindings |

All use the INI dialect described in [levels.md](levels.md#ini-dialect-ld-world_datatxt-themes-languages-saves).
Save slots can contain the same section more than once; readers must merge repeated sections (last value wins).

## Save slot (`<n>ba.ba`)

One group of sections per world id (`baba`, `new_adv`, `museum`, custom
packs):

`[<world>]`: progress in that world.

| Key | Meaning |
|---|---|
| `<level stem>=<n>` | state of a level (`5level=3`); some stems carry an `x` prefix (`x20level`; meaning not established here) |
| `<level stem>_intro=1` | the level's title card was shown |
| `<map stem>,<x>,<y>=<n>` | state of a map cell |
| `Previous` | last level played |
| `leveltree`, `leveltree_id` | stack of maps leading to the current one (stems, display ids) |
| `levelsurrounds` | the cursor's surroundings on the map, for resuming |
| `time`, `time_frames` | play time counters |
| `intro`, `firsttime`, `m_lastpack`, ... | flags |

State numbers observed: `1`, `2`, `3`; their exact meaning (opened /
visited / cleared ...) belongs to the progression specification, not to
this format description.

`[<world>_prize]`: `total=<n>` and per-level prize counts; `[<world>_clears]`:
`total` and cleared counts; other `[<world>_<kind>]` sections (e.g.
`_complete`) follow the same pattern.

`SaveSlot` exposes the raw INI plus helpers: `worlds()`, `levels(world)`
(level-state keys), `map_cells(world)`, `total(world, kind)`.

## `ba.ba`

```text
[baba]        firsttime=1, lastslot=<n>
[savegame]    slot=<n>, world=<world id>
```

## `SettingsC.txt`

| Section | Content |
|---|---|
| `[settings]` | `lang` (language code, `lang_<code>.txt`), `music`, `sound` (0..100), `fullscreen`, `zoom`, `grid`, `delay` (input repeat), `restartask`, init flags |
| `[keyboard]` | action → SDL keycode (`up`, `left`, `undo`, `restart`, `confirm`, `pause`, `idle`, and `...2` alternates) |
| `[gamepad]` | action → controller button/axis (`a0`, `b5`, `h0.4`, `dpad`, numbers) |
| `[gamepad_editor]`, `[gamepad_currobjlist]` | editor controller bindings |
| `[slotnames]` | `<n>=<name>` for the slot menu |
| `[editor]`, `[log]` | editor first-run flag, logging options |

Controller binding values: `b<n>` button, `a<n>` axis (`+`/`-` suffix for a
direction), `h<hat>.<mask>` hat direction.
