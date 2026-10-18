# Object definitions

The game describes its objects (sprites, colours, word kinds, draw order) in
Lua source files as literal tables. The port reads them from the player's
install with a data-only Lua reader (`baba_formats::lua`): it tokenises the
file and evaluates only top-level `name = <literal>` assignments (nil,
booleans, numbers, strings, nested table constructors). Functions and other
expressions are skipped, never run. This document lists the fields, not
their values.

Reader: `baba_formats::objects` (`ObjectTable`, `TileChanges`,
`EditorObjectList`).

## Object slots: `tileslist` in `Data/values.lua`

`tileslist` is a table keyed by slot id: `object000` ... `object155` (156
slots) plus `edge`. A slot is a fixed identity; levels refer to slots, and
each level can re-dress any slot as a different object (see
[Overrides](#overrides)).

| Field | Type | Meaning |
|---|---|---|
| `name` | string | name used by rules and the editor; text objects are named `text_<word>` |
| `sprite` | string | sprite base name (files `<sprite>_<variant>_<frame>.png`) |
| `sprite_in_root` | bool | `true`: sprite in `Data/Sprites`; `false`: in the world's `Sprites/` |
| `unittype` | string | `object` or `text` |
| `tiling` | int | sprite variant scheme, see below |
| `type` | int | word class of a text (noun, verb, property, prefix, ..., letter); 0 for objects |
| `colour` | `{x, y}` | palette coordinate; for text the inactive colour |
| `active` | `{x, y}` | text only: colour while part of an active rule |
| `tile` | `{x, y}` | the slot's tile coordinate; its level-file id is `x \| y << 8` |
| `grid` | `{x, y}` | position in the editor's object grid |
| `layer` | int | draw order (higher is drawn on top); objects use roughly 2..19, text 20 |
| `argtype` | int list | for some words: which kinds of words may follow |
| `argextra` | string list | for some words: extra accepted argument words |
| `does_not_exist` | bool | placeholder slot |

`edge` has only `colour` and `tile = {0, 0}`: the level border, id 0 in
level files.

Defaults used by the reader when a field is missing: `sprite` = `name`,
`sprite_in_root` = true, `unittype` = `object`, `tiling` = -1, `type` = 0,
`layer` = 10, `colour` = `{0, 3}`.

### Tiling (sprite variant scheme)

| Value | Kind | Variant number |
|---|---|---|
| -1 | static | `0` |
| 0 | directional | `dir * 8` (right 0, up 8, left 16, down 24) |
| 1 | tiled | 4-bit mask of joined neighbours: right 1, up 2, left 4, down 8 (0..15) |
| 2 | character | `dir * 8 + step` (walk steps 0..3); `dir * 8 + 7` while asleep |
| 3 | animated, directional | `dir * 8 + step` (0..3) |
| 4 | animated | `step` (0..3) |

A tiled object joins with neighbours of the same name, with the level
border cells and with level icons; the exact rules belong to the renderer's
specification. `baba_formats::sprite_variant` computes the number from
tiling, facing, mask and step.

## Editor catalogue: `Data/Editor/editor_objectlist.lua`

`editor_objlist` is an array (explicit `[n] = {...}` keys) of every object
the editor offers; `editor_objlist_order` lists their names in display
order. These are *catalogue* entries, not slots: when an author adds one to
a level, the editor assigns it a free slot and stores the differences from
that slot's default in the level's `[tiles]` section.

| Field | Meaning |
|---|---|
| `name` | object name (`text_` prefix for text) |
| `sprite` | sprite base name when it differs from `name` (rare) |
| `unittype` | `object` / `text` |
| `tags` | search tags (string list) |
| `tiling`, `type`, `layer` | as above |
| `colour` | colour |
| `colour_active` | active colour (text) |
| `advanced` | listed only with the editor's advanced option |
| `pairedwith` | text that the editor pairs with this one |
| `unpaired`, `special` | editor flags |
| `argtype`, `argextra`, `customobjects` | word arguments / custom object names |

## Overrides

A level's `[tiles]` section (and a theme's) changes slots:

```text
changed=object012,object031,      slots with overrides (also changed2, changed3, ...)
object012_name=wall               per-slot fields
object012_image=wall_map
object012_colour=1,1
object012_root=0
```

Fields: `name`, `image` (sprite), `colour`, `activecolour`, `tiling`,
`type`, `unittype`, `root` (`1`/`0` = `sprite_in_root`), `layer`,
`argtype`, `argextra`, `customobjects` (lists comma separated with a
trailing comma). An empty value means "unchanged". Only slots named in a
`changed` list are applied (in the shipped data every override key belongs
to a listed slot).

Resolution for a level: start from the `tileslist` definitions and apply
the level's overrides field by field. Every level starts again from the
base definitions; overrides never carry over between levels. A field that
is not overridden keeps the base slot's value, so a slot renamed to `keke`
without an `image` key still uses the base slot's sprite.

`ObjectTable::with_changes(&level.info.tiles)` performs this and returns
the effective table; `by_tile(id)` then maps every cell of the grid to its
definition. With the install's data all tile ids of all 600 shipped levels
resolve.

Themes (`Themes/<n>theme.txt`) use the same `[tiles]` syntax; the editor
applies a theme to a level, so the result ends up in the level's own
`[tiles]` section. Playing a level never consults themes.
