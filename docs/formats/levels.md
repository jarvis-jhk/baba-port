# Levels: `.l` grids and `.ld` metadata

Every level is a pair of files in a world directory (`Data/Worlds/<world>/`):

| File | What |
|---|---|
| `<stem>.l` | binary: the tile grid (which object slot sits in each cell, and its facing) |
| `<stem>.ld` | INI text: name, palette, music, map links, paths, specials, object slot overrides, editor state |
| `<stem>.png` | preview thumbnail (RGB, 72x72 or 216x112), not needed to play |

`<stem>` is usually `<number>level` (`0level`, `106level`); other names occur
(`boop`). Reader: `baba_formats::level` and `baba_formats::leveldata`.

## `.l`: binary grid

All integers little endian. The file is a signature, a version and a list of
chunks:

```text
offset size  field
0      8     signature "ACHTUNG!"
8      2     u16 version            0x0105 in every shipped file
10     ...   chunks until end of file:
             4  tag (ASCII)
             4  u32 payload size
             n  payload
```

Chunks seen: `MAP ` then `LAYR`. Unknown chunks can be skipped by size (the
reader keeps them for round trips).

### `MAP ` chunk

Two bytes, always `00 00`.

### `LAYR` chunk

```text
u16 layer count                 1..3 (all three counts occur)
per layer:
  u32 width                     cells, including the one-cell border
  u32 height                    cells, including the border
  32  layer parameters          see below
  u8  block count               number of blocks that follow: 2 (MAIN, DATA) or 1 (MAIN only)
  "MAIN"
  u32 size                      of the zlib stream that follows
  ... zlib(width*height x u16)  tile ids, row major, top-left first
  "DATA"                        only if block count >= 2
  u32 n                         number of data arrays (1 in every file)
  n x:
    u8  kind                    0 = facing directions
    u32 size
    ... zlib(width*height x u8)
```

All layers of a level have the same size. Sizes vary per level (26x20,
30x18, 35x20, 26x16, 20x20, 17x10 are the most common; most maps are 35x20,
i.e. 33x18 playable cells plus the border). Exactly one
shipped layer has a block count of 1 and no DATA block (the last layer of
the main map); readers must accept both.

**Tile ids** (u16): `0xFFFF` is an empty cell. Any other value names an
object *slot* by its tile coordinate: `id = x | y << 8`, where `(x, y)` is
the slot's `tile` field in the object definitions (see
[objects.md](objects.md)). Id `0` (tile `(0, 0)`) is the special `edge`
slot: the level border. With the definitions from the install every id in
every shipped level resolves (0 unknown).

**Directions** (u8, kind 0): `0` right, `1` up, `2` left, `3` down. Empty
cells carry arbitrary values. The facing selects the sprite variant and is
the object's initial direction.

**Layer parameters** (32 bytes): identical in every shipped layer:

```text
0c 00 0c 00 00 ff 00 00 00 00 00 00 00 00 00 00
80 3f 00 00 80 3f 00 00 01 00 00 80 3f ff ff ff
```

Their meaning is not needed to load levels. The pattern looks like a packed
layer record: two 16-bit values 12, three unaligned 32-bit floats equal to
1.0 (at bytes 14, 18, 25), a 1 flag (byte 24) and an RGB white (bytes
29..31). This reading is unverified; writers should emit these bytes
unchanged (`DEFAULT_LAYER_PARAMS`).

The reader validates sizes (decompressed lengths must equal `width*height`
times the element size) and reports the byte offset of any inconsistency.
Our writer re-compresses with zlib, so written files differ byte-wise from
the originals but parse to the same data.

## INI dialect (`.ld`, `world_data.txt`, themes, languages, saves)

- `[section]` lines; `key=value` lines, split at the first `=`.
- Keys are trimmed. Values are taken verbatim up to the line end (some
  values end in spaces that matter, e.g. `difficulty`).
- Line ends are `\r\n`, `\n`, or (one language file) a lone `\r`. An UTF-8
  BOM is skipped. Text is UTF-8.
- No comment syntax. A line such as `--particles=clouds` defines a key
  literally named `--particles` (an author's way of disabling a key).
- Lines without `=` are ignored.
- A section name can occur more than once (saves do this). Readers treat
  repeated blocks as one section; for a repeated key the last value wins.
- Lists inside values are comma separated and usually end with a trailing
  comma (`object001,object002,`). Colours are `x,y` palette coordinates.

## `.ld`: level metadata

### `[general]`

| Key | Meaning |
|---|---|
| `name` | display name |
| `subtitle` | line shown under the name |
| `author` | author |
| `palette` | palette file (`default.png`) |
| `paletteroot` | `1`: palette in `Data/Palettes`; `0`: in the world's `Palettes/` |
| `music` | music track name (file stem in a `Music/` directory), empty = default |
| `localmusic` | music location switch (`1` presumably: the world's `Music/`) |
| `particles` | ambient particle effect name, empty = none |
| `leveltype` | `0` level, `1` map |
| `mapid` | identifier of a map |
| `levels`, `paths`, `specials` | record counts of the sections below |
| `currobjlist_total` | record count of `[currobjlist]` |
| `selectorX`, `selectorY` | initial map cursor cell, `-1` = none |
| `unlockcount` | clears needed to open the level |
| `unlocklevels` | comma separated levels opened by clearing this one |
| `customparent` | parent level stem for custom levels |
| `levelz` | engine-side value (20 in every file); not needed to load |
| `rhythm` | engine-side timing value; not needed to load |
| `difficulty`, `difficultynum` | editor difficulty display / value |
| `customruleword` | custom rule word setting |
| `disableparticles`, `disableshake`, `disableruleeffect` | effect switches |
| `notcompleteable`, `endgame`, `showhud` | special-level flags |
| `levelid`, `levelcode` | ids for the online level system |
| `contrastpalette` | palette used in high-contrast mode |

Only some keys exist in a given file; absent means default (empty/0, with
`paletteroot` defaulting to 1). `LevelInfo::ini` keeps every key.

### Indexed sections

`[levels]`, `[paths]`, `[specials]` and `[currobjlist]` hold records as
`<index><field>=value` (e.g. `12file=207level`). **The count in `[general]`
is authoritative**: files often contain stale records with higher indexes
(left over from editing) that the game ignores. `[levels]`, `[paths]`,
`[specials]` are 0-based; `[currobjlist]` is 1-based.

`[levels]`: level icons on a map.

| Field | Meaning |
|---|---|
| `name` | name override (usually empty) |
| `file` | target level stem |
| `colour`, `clearcolour` | icon colour, colour once cleared |
| `X`, `Y`, `Z` | cell and draw layer |
| `state` | initial visibility/state number |
| `number` | number or letter index shown on the icon |
| `style` | icon style number |
| `dir` | facing |

`[paths]`: path tiles on a map: `object` (slot id, e.g. `object117`), `X`,
`Y`, `Z`, `dir`, `style`, `gate` (0 = plain path, otherwise a gate kind),
`requirement` (amount needed to pass a gate), `unlock` (condition text).

`[specials]`: special objects such as control hints: `X`, `Y`, `Z`, `data`
(comma separated payload, e.g. `controls,up`).

`[images]`: `total=<n>` and `0=<name>` ... `<n-1>=<name>`: background
images drawn under the grid, from the world's `Images/<name>_<frame>.png`
(frames 1..3), sized to the playable area (33x18 cells of 24 px).

`[icons]`: custom icons for level numbers: `<i>file=<sprite file stem>`,
`<i>root=` `1` (`Data/Sprites`) or `0` (world `Sprites/`). Not counted;
empty `file` means unused.

`[currobjlist]`: the level's editor palette: `id` (editor object index),
`name`, `object` (slot id it occupies in this level), `gox`/`goy` and
`gfx`/`gfy` (positions in the two editor list views), `pair` (index of the
paired text/object record, `-1` none), `tile` (`x,y` of the slot).

`[hotbar]`: editor hotbar slots `<i>X`, `<i>Y`, `<i>fixed`, `<i>target`
(slot id), plus `saved`.

### `[tiles]`: object slot overrides

How a level dresses its object slots; see [objects.md](objects.md#overrides).
Keys: `changed` (comma separated slot ids; long lists continue in `changed2`,
`changed3`, ... with `changed_count` giving the number of lists),
`changed_short` (the same ids by their last three digits, an editor
helper), and `<slot>_<field>=value` overrides.

## Writing `.ld` (the editor)

`LevelInfo::to_ini` / `to_text` serialise a level the way the editor
stores it: the typed `[general]` fields are written over the keys read
from the file (keys the port does not interpret stay as they were), and
the indexed sections `[images]`, `[levels]`, `[icons]`, `[currobjlist]`,
`[hotbar]`, `[specials]`, `[paths]` and `[tiles]` are rebuilt from the
typed records with their counts (`levels`, `paths`, `specials`,
`currobjlist_total`, `[images] total`), dropping stale records. Line ends
are `\r\n`. Unknown sections are kept.

- `[icons]` is written for indexes 0..19 (and any higher one in use),
  unused ones as empty `file` / `root`.
- `[currobjlist]` records are 1-based: `id` (catalogue index), `name`,
  `object` (slot), `gox`/`goy` (paired view: an object and its text share
  a cell), `gfx`/`gfy` (full view), `pair` (record index or `-1`), `tile`
  (`x,y`). Older files lack `tile`. New entries take the first free cell
  in growing shells: row `k` from the left (k ≤ 9), then column `k + 1`
  from the top, so the grid is at most 10 rows high.
- `[hotbar]`: `saved=1`, then per slot 0..9 `target` (slot id or empty),
  `X`/`Y` (the target's paired grid cell, `-1` if empty), `fixed` (locked).
- `[tiles]`: per changed slot the set fields (`name`, `image`, `colour`,
  `tiling`, `type`, `unittype`, `activecolour`, `root`, `layer`, `argtype`,
  `argextra`, `customobjects`), then `changed_count`, the `changed`,
  `changed2`, … lists (a list is closed once it reaches 600 characters)
  and `changed_short` (the last three characters of each slot id). A slot
  binding a catalogue object stores only the fields that differ from the
  slot's base definition; `activecolour` is kept when the base has none.

`.l` files written by the port always carry three layers with direction
data; the level border is `edge` in the first layer.
