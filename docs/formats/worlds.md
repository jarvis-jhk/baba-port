# Install layout, worlds and other data files

Reader entry points: `baba_formats::GameInstall` (finds the install,
lists worlds, loads object tables and palettes) and `baba_formats::World`.
The port locates the install from `$BABA_GAME_DIR` (dev) or its own
configuration; it never writes there.

## Install layout (Linux build)

```text
<install>/
  run.sh, bin64/Chowdren, bin64/libsteam_api.so   the game (Chowdren build)
  Assets.dat                                      engine-side images, sounds, shaders (assets-dat.md)
  gamecontrollerdb.txt                            SDL controller mappings
  Data/
    values.lua, *.lua                              rules, menus, object slots (objects.md)
    Editor/*.lua, Editor/Tutorials/*.lua           editor code, editor object catalogue
    Sprites/   Palettes/   Themes/   Music/   Sounds/   Languages/
    Worlds/<world>/                                level packs
    merged.ttf, mergedsc.ttf                       fonts
    levels.m, map0.m                               small INI files with level id counters
    endingflowers.txt, keyboard*.png, MF_documentation.txt, ...
```

## Worlds: `Data/Worlds/<id>/`

Shipped ids: `baba` (the main game), `new_adv` (New Adventures), `museum`,
`levels` (where the editor keeps custom levels; empty in a fresh install)
and `debug` (a test pack). Custom world packs use the same layout.

| Path | Content |
|---|---|
| `world_data.txt` | pack metadata (below) |
| `<stem>.l`, `<stem>.ld`, `<stem>.png` | levels ([levels.md](levels.md)) |
| `Sprites/` | pack-specific sprites ([palettes-sprites.md](palettes-sprites.md)) |
| `Palettes/` | pack-specific palettes |
| `Images/` | map background images |
| `Themes/` | editor themes for the pack |
| `Music/`, `Sounds/` | pack-specific Ogg files |
| `Lua/` | Lua mods of the pack (`mods=1`); the port does not run Lua mods |
| `icon.png` | 48x48 pack icon |
| `credits.txt` | credits text; lines may start with a `$x,y` formatting code (apparently a palette colour) |
| `Sprites_fi/` | (`baba`) Finnish control-hint sprites |

### `world_data.txt`

```text
[general]
name=...             display name
author=...
start=<stem>         level the pack opens on (the top map)
firstlevel=<stem>    first level of a fresh save
prize_max=<n>        totals used by completion displays
clear_max=<n>
bonus_max=<n>
themecount=<n>       number of editor themes
mods=1               the pack ships Lua mods
[levels]
levelid=<n>          id counters used by the editor when creating levels
rolling_levelid=<n>
[completion_icons]   optional: count, <i>_flag, <i>_file
```

## Themes: `Themes/<n>theme.txt`

Global (`Data/Themes/`) and per pack. INI with `[general]` (`name`,
`palette`, `paletteroot`, `music`, `particles`, `currobjlist_total`) and a
`[tiles]` section with slot overrides in the level syntax
([objects.md](objects.md#overrides)). Reader: `baba_formats::Theme`.

## Music and sounds

- `Data/Music/*.ogg`: music tracks (Ogg Vorbis), named by `.ld`
  `music=<stem>`; packs can add their own in `Music/`.
- `Data/Sounds/`: only an example file; the game's sound effects are inside
  `Assets.dat` ([assets-dat.md](assets-dat.md)).
- `GameInstall::music()`, `sounds()`, `World::music()`, `World::sounds()`
  list the files.

## Languages: `Data/Languages/lang_<code>.txt`

INI: `[general]` (`name` = the language's own name, `customfont` = needs a
special font) and `[texts]` with `<text id>=<text>` (about two thousand
entries). Encoding UTF-8; one file uses lone `\r` line ends. 18 languages
ship. Reader: `baba_formats::Language`.

## Other files

- `Data/levels.m`, `Data/map0.m`: tiny INI files (`[levels] levelid=...`,
  `rolling_levelid=...`): editor id counters.
- `Data/Editor/Tutorials/*.lua`: editor tutorial scripts (reference only).
- `Data/Temp/`, `Data/Lua/`: empty in a fresh install.
