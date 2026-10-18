//! Object definitions: the base object slots (`tileslist` in
//! `Data/values.lua`), the editor's object catalogue
//! (`editor_objlist` in `Data/Editor/editor_objectlist.lua`) and the
//! per-level / per-theme overrides stored in `[tiles]` sections.
//!
//! How they fit together (details in `docs/formats/objects.md`):
//! - A level grid stores [`TileId`]s. Each base slot (`object000` ...
//!   `object155`, plus `edge`) owns one tile id, so the grid always refers to
//!   slots, never to names.
//! - A level's `[tiles]` section ([`TileChanges`]) re-dresses slots: it can
//!   give a slot another name, sprite, colour, tiling, layer, ... (that is
//!   how a level holds e.g. `keke` in a slot whose default is `baba`).
//! - [`ObjectTable::with_changes`] applies such overrides and yields the
//!   effective definitions for one level.

use std::collections::BTreeMap;
use std::path::Path;

use crate::error::{Error, Result};
use crate::ini::{Ini, parse_int};
use crate::lua::{LuaChunk, Table, Value};
use crate::palette::PalColour;

/// A tile id as stored in level layers: `x | (y << 8)` where `(x, y)` is the
/// slot's `tile` coordinate. `0xFFFF` means "no object".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileId(pub u16);

impl TileId {
    /// The empty cell.
    pub const EMPTY: TileId = TileId(0xFFFF);
    /// The level border object (`edge`, tile (0, 0)).
    pub const EDGE: TileId = TileId(0);

    /// From the `tile = {x, y}` coordinate of a slot.
    pub const fn from_xy(x: u8, y: u8) -> TileId {
        TileId(x as u16 | (y as u16) << 8)
    }
    /// The `(x, y)` coordinate.
    pub const fn xy(self) -> (u8, u8) {
        ((self.0 & 0xFF) as u8, (self.0 >> 8) as u8)
    }
    /// True for [`TileId::EMPTY`].
    pub const fn is_empty(self) -> bool {
        self.0 == 0xFFFF
    }
}

/// How an object picks its sprite variant (the middle number of
/// `name_<variant>_<frame>.png`). See `docs/formats/palettes-sprites.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tiling {
    /// `-1`: a single variant `0`.
    None,
    /// `0`: one variant per facing direction: `dir * 8`.
    Directional,
    /// `1`: joins with equal neighbours: variant = 4-bit neighbour mask (0..15).
    Tiled,
    /// `2`: a character: `dir * 8 + walk step (0..3)`, `dir * 8 + 7` asleep.
    Character,
    /// `3`: directional with a 4-step animation: `dir * 8 + step`.
    AnimatedDirectional,
    /// `4`: a 4-step animation without direction: `step`.
    Animated,
    /// Any other value found in the data.
    Other(i32),
}

impl Tiling {
    /// From the numeric value in the data.
    pub fn from_i32(v: i32) -> Tiling {
        match v {
            -1 => Tiling::None,
            0 => Tiling::Directional,
            1 => Tiling::Tiled,
            2 => Tiling::Character,
            3 => Tiling::AnimatedDirectional,
            4 => Tiling::Animated,
            o => Tiling::Other(o),
        }
    }
    /// The numeric value used in the data.
    pub fn to_i32(self) -> i32 {
        match self {
            Tiling::None => -1,
            Tiling::Directional => 0,
            Tiling::Tiled => 1,
            Tiling::Character => 2,
            Tiling::AnimatedDirectional => 3,
            Tiling::Animated => 4,
            Tiling::Other(o) => o,
        }
    }
}

/// An object slot's definition (one entry of `tileslist`, possibly with a
/// level's overrides applied).
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectDef {
    /// Slot id (`"object000"` ... or `"edge"`). Never changes.
    pub id: String,
    /// Name used by rules and the editor (`"baba"`, `"text_baba"`).
    pub name: String,
    /// Sprite base name (files `<sprite>_<variant>_<frame>.png`).
    pub sprite: String,
    /// `true`: sprite in `Data/Sprites`; `false`: in the world's `Sprites/`.
    pub sprite_in_root: bool,
    /// `"object"` or `"text"` (kept as a string; other values may appear in mods).
    pub unittype: String,
    /// Sprite variant scheme.
    pub tiling: Tiling,
    /// Word class number for text (`type` in the data); 0 for objects.
    pub word_type: i32,
    /// Colour (inactive colour for text).
    pub colour: PalColour,
    /// Colour of text that is part of an active rule.
    pub active_colour: Option<PalColour>,
    /// Tile id this slot owns in level grids.
    pub tile: Option<TileId>,
    /// Position in the editor's object grid (`grid = {x, y}`).
    pub grid: Option<(i32, i32)>,
    /// Draw order: higher layers are drawn on top.
    pub layer: i32,
    /// Argument kinds accepted by a word (`argtype`), if given.
    pub argtype: Vec<i32>,
    /// Extra argument words (`argextra`), if given.
    pub argextra: Vec<String>,
    /// Custom object names (`customobjects`), if given.
    pub customobjects: Vec<String>,
    /// Marked as a placeholder slot (`does_not_exist = true`).
    pub does_not_exist: bool,
}

impl ObjectDef {
    /// True for text objects.
    pub fn is_text(&self) -> bool {
        self.unittype == "text"
    }

    fn from_lua(id: &str, t: &Table) -> Result<ObjectDef> {
        let int = |k: &str, d: i64| t.get(k).and_then(Value::as_i64).unwrap_or(d);
        let colour = |k: &str| {
            t.get(k)
                .and_then(Value::as_int_pair)
                .and_then(PalColour::from_pair)
        };
        let name = t
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(id)
            .to_string();
        Ok(ObjectDef {
            id: id.to_string(),
            sprite: t
                .get("sprite")
                .and_then(Value::as_str)
                .unwrap_or(&name)
                .to_string(),
            name,
            sprite_in_root: t
                .get("sprite_in_root")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            unittype: t
                .get("unittype")
                .and_then(Value::as_str)
                .unwrap_or("object")
                .to_string(),
            tiling: Tiling::from_i32(int("tiling", -1) as i32),
            word_type: int("type", 0) as i32,
            colour: colour("colour").unwrap_or(PalColour::new(0, 3)),
            active_colour: colour("active"),
            tile: t
                .get("tile")
                .and_then(Value::as_int_pair)
                .map(|(x, y)| TileId::from_xy(x as u8, y as u8)),
            grid: t
                .get("grid")
                .and_then(Value::as_int_pair)
                .map(|(x, y)| (x as i32, y as i32)),
            layer: int("layer", 10) as i32,
            argtype: int_list(t.get("argtype")),
            argextra: str_list(t.get("argextra")),
            customobjects: str_list(t.get("customobjects")),
            does_not_exist: t
                .get("does_not_exist")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }
}

fn int_list(v: Option<&Value>) -> Vec<i32> {
    v.and_then(Value::as_int_list)
        .map(|l| l.into_iter().map(|i| i as i32).collect())
        .unwrap_or_default()
}

fn str_list(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_str_list).unwrap_or_default()
}

/// The object slots, indexed by slot id and by tile id.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObjectTable {
    objects: Vec<ObjectDef>,
    by_id: BTreeMap<String, usize>,
    by_tile: BTreeMap<TileId, usize>,
}

impl ObjectTable {
    /// Builds a table from definitions (ids must be unique; later wins).
    pub fn new(objects: Vec<ObjectDef>) -> ObjectTable {
        let mut t = ObjectTable {
            objects,
            ..Default::default()
        };
        t.reindex();
        t
    }

    fn reindex(&mut self) {
        self.by_id.clear();
        self.by_tile.clear();
        for (i, o) in self.objects.iter().enumerate() {
            self.by_id.insert(o.id.clone(), i);
            if let Some(t) = o.tile {
                self.by_tile.insert(t, i);
            }
        }
    }

    /// Reads the `tileslist` table from the source of `Data/values.lua`.
    pub fn from_values_lua(src: &str) -> Result<ObjectTable> {
        let chunk = LuaChunk::parse(src)?;
        ObjectTable::from_chunk(&chunk)
    }

    /// Reads `tileslist` from an already parsed chunk.
    pub fn from_chunk(chunk: &LuaChunk) -> Result<ObjectTable> {
        let list = chunk
            .get("tileslist")
            .and_then(Value::as_table)
            .ok_or_else(|| Error::missing("table `tileslist`"))?;
        let mut objects = Vec::new();
        for (id, v) in list.named() {
            if let Some(t) = v.as_table() {
                objects.push(ObjectDef::from_lua(id, t)?);
            }
        }
        if objects.is_empty() {
            return Err(Error::missing("entries in `tileslist`"));
        }
        Ok(ObjectTable::new(objects))
    }

    /// Reads `Data/values.lua`.
    pub fn load(values_lua: &Path) -> Result<ObjectTable> {
        ObjectTable::from_chunk(&LuaChunk::load(values_lua)?).map_err(|e| e.at(values_lua))
    }

    /// All slots in source order.
    pub fn iter(&self) -> impl Iterator<Item = &ObjectDef> {
        self.objects.iter()
    }

    /// Number of slots.
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// True if there are no slots.
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// Slot by id (`"object012"`).
    pub fn get(&self, id: &str) -> Option<&ObjectDef> {
        self.by_id.get(id).map(|&i| &self.objects[i])
    }

    /// Slot owning a tile id.
    pub fn by_tile(&self, tile: TileId) -> Option<&ObjectDef> {
        self.by_tile.get(&tile).map(|&i| &self.objects[i])
    }

    /// First slot whose (effective) name is `name`.
    pub fn by_name(&self, name: &str) -> Option<&ObjectDef> {
        self.objects.iter().find(|o| o.name == name)
    }

    /// A copy with `changes` applied (unknown slot ids are ignored).
    pub fn with_changes(&self, changes: &TileChanges) -> ObjectTable {
        let mut t = self.clone();
        for (id, ch) in &changes.changes {
            if let Some(&i) = t.by_id.get(id) {
                ch.apply(&mut t.objects[i]);
            }
        }
        t
    }
}

/// Overrides for one slot, as stored in a `[tiles]` section
/// (`objectNNN_<field>=value`). `None` = unchanged.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObjectChange {
    /// `_name`
    pub name: Option<String>,
    /// `_image`: sprite base name.
    pub image: Option<String>,
    /// `_colour`
    pub colour: Option<PalColour>,
    /// `_activecolour`
    pub active_colour: Option<PalColour>,
    /// `_tiling`
    pub tiling: Option<i32>,
    /// `_type`
    pub word_type: Option<i32>,
    /// `_unittype`
    pub unittype: Option<String>,
    /// `_root` (`1` = `Data/Sprites`, `0` = world sprites)
    pub root: Option<bool>,
    /// `_layer`
    pub layer: Option<i32>,
    /// `_argtype` (`"0,2,"`)
    pub argtype: Option<Vec<i32>>,
    /// `_argextra`
    pub argextra: Option<Vec<String>>,
    /// `_customobjects`
    pub customobjects: Option<Vec<String>>,
}

impl ObjectChange {
    /// Applies the overrides to a definition.
    pub fn apply(&self, o: &mut ObjectDef) {
        if let Some(v) = &self.name {
            o.name.clone_from(v);
        }
        if let Some(v) = &self.image {
            o.sprite.clone_from(v);
        }
        if let Some(v) = self.colour {
            o.colour = v;
        }
        if let Some(v) = self.active_colour {
            o.active_colour = Some(v);
        }
        if let Some(v) = self.tiling {
            o.tiling = Tiling::from_i32(v);
        }
        if let Some(v) = self.word_type {
            o.word_type = v;
        }
        if let Some(v) = &self.unittype {
            o.unittype.clone_from(v);
        }
        if let Some(v) = self.root {
            o.sprite_in_root = v;
        }
        if let Some(v) = self.layer {
            o.layer = v;
        }
        if let Some(v) = &self.argtype {
            o.argtype.clone_from(v);
        }
        if let Some(v) = &self.argextra {
            o.argextra.clone_from(v);
        }
        if let Some(v) = &self.customobjects {
            o.customobjects.clone_from(v);
        }
    }

    /// True if nothing is overridden.
    pub fn is_empty(&self) -> bool {
        *self == ObjectChange::default()
    }
}

/// The slot overrides of a level or theme (`[tiles]` section).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TileChanges {
    /// Overrides by slot id, for the slots named in the `changed` lists.
    pub changes: BTreeMap<String, ObjectChange>,
}

impl TileChanges {
    /// Reads the `[tiles]` section of a parsed `.ld` / theme file.
    ///
    /// Only slots listed in `changed`, `changed2`, ... are taken (the lists
    /// are comma separated, with a trailing comma); empty values mean
    /// "unchanged".
    pub fn from_ini(ini: &Ini) -> TileChanges {
        let entries = ini.entries("tiles");
        let mut listed: Vec<&str> = Vec::new();
        for (k, v) in &entries {
            let is_list = k
                .strip_prefix("changed")
                .is_some_and(|rest| rest.is_empty() || rest.bytes().all(|b| b.is_ascii_digit()));
            if is_list {
                for id in v.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    if !listed.contains(&id) {
                        listed.push(id);
                    }
                }
            }
        }
        let map = ini.section_map("tiles");
        let mut changes = BTreeMap::new();
        for id in listed {
            let get = |f: &str| {
                map.get(format!("{id}_{f}").as_str())
                    .copied()
                    .filter(|v| !v.is_empty())
            };
            let int = |f: &str| get(f).and_then(parse_int).map(|v| v as i32);
            let list = |f: &str| {
                get(f).map(|v| {
                    v.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
            };
            let ch = ObjectChange {
                name: get("name").map(str::to_string),
                image: get("image").map(str::to_string),
                colour: get("colour").and_then(PalColour::parse),
                active_colour: get("activecolour").and_then(PalColour::parse),
                tiling: int("tiling"),
                word_type: int("type"),
                unittype: get("unittype").map(str::to_string),
                root: get("root").and_then(|v| match v.trim() {
                    "1" | "true" => Some(true),
                    "0" | "false" => Some(false),
                    _ => None,
                }),
                layer: int("layer"),
                argtype: list("argtype").map(|l| {
                    l.iter()
                        .filter_map(|s| parse_int(s))
                        .map(|v| v as i32)
                        .collect()
                }),
                argextra: list("argextra"),
                customobjects: list("customobjects"),
            };
            changes.insert(id.to_string(), ch);
        }
        TileChanges { changes }
    }

    /// Overrides for a slot.
    pub fn get(&self, id: &str) -> Option<&ObjectChange> {
        self.changes.get(id)
    }

    /// The `[tiles]` entries as the editor writes them: per slot its set
    /// fields (`<slot>_<field>`), then `changed_count`, the `changed`,
    /// `changed2`, ... lists (a list is closed once it reaches 600
    /// characters) and `changed_short` (last three digits of each slot).
    pub fn to_entries(&self) -> Vec<(String, String)> {
        let list = |v: &[String]| v.iter().map(|s| format!("{s},")).collect::<String>();
        let mut out = Vec::new();
        let mut lists: Vec<String> = Vec::new();
        let mut cur = String::new();
        let mut short = String::new();
        for (id, ch) in &self.changes {
            let mut put = |f: &str, v: String| out.push((format!("{id}_{f}"), v));
            let col = |c: PalColour| format!("{},{}", c.x, c.y);
            if let Some(v) = &ch.name {
                put("name", v.clone());
            }
            if let Some(v) = &ch.image {
                put("image", v.clone());
            }
            if let Some(v) = ch.colour {
                put("colour", col(v));
            }
            if let Some(v) = ch.tiling {
                put("tiling", v.to_string());
            }
            if let Some(v) = ch.word_type {
                put("type", v.to_string());
            }
            if let Some(v) = &ch.unittype {
                put("unittype", v.clone());
            }
            if let Some(v) = ch.active_colour {
                put("activecolour", col(v));
            }
            if let Some(v) = ch.root {
                put("root", if v { "1" } else { "0" }.to_string());
            }
            if let Some(v) = ch.layer {
                put("layer", v.to_string());
            }
            if let Some(v) = &ch.argtype {
                let s: Vec<String> = v.iter().map(i32::to_string).collect();
                put("argtype", list(&s));
            }
            if let Some(v) = &ch.argextra {
                put("argextra", list(v));
            }
            if let Some(v) = &ch.customobjects {
                put("customobjects", list(v));
            }
            cur.push_str(id);
            cur.push(',');
            let tail: String = id
                .chars()
                .rev()
                .take(3)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            short.push_str(&tail);
            short.push(',');
            if cur.len() >= 600 {
                lists.push(std::mem::take(&mut cur));
            }
        }
        if !cur.is_empty() {
            lists.push(cur);
        }
        out.push(("changed_count".into(), lists.len().to_string()));
        if lists.is_empty() {
            out.push(("changed".into(), String::new()));
        }
        for (i, l) in lists.into_iter().enumerate() {
            let key = if i == 0 {
                "changed".to_string()
            } else {
                format!("changed{}", i + 1)
            };
            out.push((key, l));
        }
        out.push(("changed_short".into(), short));
        out
    }
}

impl ObjectChange {
    /// The overrides that turn `base` into `want`, field by field, as the
    /// editor stores them: a field equal to the base value is left out;
    /// the active colour is kept when the base has none.
    pub fn between(base: &ObjectDef, want: &ObjectDef) -> ObjectChange {
        let diff = |a: &str, b: &str| (a != b).then(|| b.to_string());
        ObjectChange {
            name: diff(&base.name, &want.name),
            image: diff(&base.sprite, &want.sprite),
            colour: (base.colour != want.colour).then_some(want.colour),
            active_colour: match (base.active_colour, want.active_colour) {
                (Some(a), Some(b)) if a == b => None,
                (_, b) => b,
            },
            tiling: (base.tiling != want.tiling).then(|| want.tiling.to_i32()),
            word_type: (base.word_type != want.word_type).then_some(want.word_type),
            unittype: diff(&base.unittype, &want.unittype),
            root: (base.sprite_in_root != want.sprite_in_root).then_some(want.sprite_in_root),
            layer: (base.layer != want.layer).then_some(want.layer),
            argtype: {
                let b = if base.argtype.is_empty() {
                    vec![0]
                } else {
                    base.argtype.clone()
                };
                let w = if want.argtype.is_empty() {
                    vec![0]
                } else {
                    want.argtype.clone()
                };
                (b != w).then_some(w)
            },
            argextra: (base.argextra != want.argextra).then(|| want.argextra.clone()),
            customobjects: (base.customobjects != want.customobjects)
                .then(|| want.customobjects.clone()),
        }
    }
}

/// One entry of the editor's object catalogue (`editor_objlist`).
#[derive(Clone, Debug, PartialEq)]
pub struct EditorObject {
    /// Index in `editor_objlist` (1-based, as in the file).
    pub index: i64,
    /// Object name.
    pub name: String,
    /// Sprite base name if it differs from the name.
    pub sprite: Option<String>,
    /// `"object"` or `"text"`.
    pub unittype: String,
    /// Editor search tags.
    pub tags: Vec<String>,
    /// Sprite variant scheme.
    pub tiling: Tiling,
    /// Word class number (`type`).
    pub word_type: i32,
    /// Draw layer.
    pub layer: i32,
    /// Colour.
    pub colour: PalColour,
    /// Active colour (text).
    pub active_colour: Option<PalColour>,
    /// Hidden behind the editor's "advanced" toggle.
    pub advanced: bool,
    /// Listed only in the editor's extended mode (`special`).
    pub special: bool,
    /// Never listed (`redacted`).
    pub redacted: bool,
    /// Not paired with its noun/text counterpart (`unpaired`).
    pub unpaired: bool,
    /// Paired text for the editor's pairing feature.
    pub paired_with: Option<String>,
    /// `argtype`, if given.
    pub argtype: Vec<i32>,
    /// `argextra`, if given.
    pub argextra: Vec<String>,
    /// `customobjects`, if given.
    pub customobjects: Vec<String>,
}

/// The editor's object catalogue.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditorObjectList {
    /// All entries ordered by index.
    pub objects: Vec<EditorObject>,
    /// Display order (`editor_objlist_order`), names.
    pub order: Vec<String>,
}

impl EditorObjectList {
    /// Reads the source of `Data/Editor/editor_objectlist.lua`.
    pub fn from_lua(src: &str) -> Result<EditorObjectList> {
        EditorObjectList::from_chunk(&LuaChunk::parse(src)?)
    }

    /// Reads the file.
    pub fn load(path: &Path) -> Result<EditorObjectList> {
        EditorObjectList::from_chunk(&LuaChunk::load(path)?).map_err(|e| e.at(path))
    }

    /// From a parsed chunk.
    pub fn from_chunk(chunk: &LuaChunk) -> Result<EditorObjectList> {
        let list = chunk
            .get("editor_objlist")
            .and_then(Value::as_table)
            .ok_or_else(|| Error::missing("table `editor_objlist`"))?;
        let mut objects = Vec::new();
        for (k, v) in &list.entries {
            let (crate::lua::Key::Index(index), Some(t)) = (k, v.as_table()) else {
                continue;
            };
            let int = |k: &str, d: i64| t.get(k).and_then(Value::as_i64).unwrap_or(d);
            let colour = |k: &str| {
                t.get(k)
                    .and_then(Value::as_int_pair)
                    .and_then(PalColour::from_pair)
            };
            let Some(name) = t.get("name").and_then(Value::as_str) else {
                continue;
            };
            objects.push(EditorObject {
                index: *index,
                name: name.to_string(),
                sprite: t.get("sprite").and_then(Value::as_str).map(str::to_string),
                unittype: t
                    .get("unittype")
                    .and_then(Value::as_str)
                    .unwrap_or("object")
                    .to_string(),
                tags: str_list(t.get("tags")),
                tiling: Tiling::from_i32(int("tiling", -1) as i32),
                word_type: int("type", 0) as i32,
                layer: int("layer", 10) as i32,
                colour: colour("colour").unwrap_or(PalColour::new(0, 3)),
                active_colour: colour("colour_active"),
                advanced: t.get("advanced").and_then(Value::as_bool).unwrap_or(false),
                special: t.get("special").and_then(Value::as_bool).unwrap_or(false),
                redacted: t.get("redacted").and_then(Value::as_bool).unwrap_or(false),
                unpaired: t.get("unpaired").and_then(Value::as_bool).unwrap_or(false),
                paired_with: t
                    .get("pairedwith")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                argtype: int_list(t.get("argtype")),
                argextra: str_list(t.get("argextra")),
                customobjects: str_list(t.get("customobjects")),
            });
        }
        objects.sort_by_key(|o| o.index);
        let order = chunk
            .get("editor_objlist_order")
            .and_then(Value::as_str_list)
            .unwrap_or_default();
        Ok(EditorObjectList { objects, order })
    }

    /// Entry by name.
    pub fn get(&self, name: &str) -> Option<&EditorObject> {
        self.objects.iter().find(|o| o.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALUES: &str = r#"
        colours = { default = {0, 3} }
        tileslist =
        {
            edge = { colour = {0, 2}, tile = {0, 0} },
            object000 = { name = "baba", sprite = "baba", sprite_in_root = true,
                unittype = "object", tiling = 2, type = 0, colour = {0, 3},
                tile = {1, 0}, grid = {0, 1}, layer = 18 },
            object001 = { name = "text_is", sprite = "text_is", sprite_in_root = true,
                unittype = "text", tiling = -1, type = 1, colour = {0, 1}, active = {0, 3},
                tile = {2, 1}, grid = {0, 2}, layer = 20, argtype = {0, 2} },
        }
        function f() local tileslist = 3 end
    "#;

    #[test]
    fn tileslist() {
        let t = ObjectTable::from_values_lua(VALUES).unwrap();
        assert_eq!(t.len(), 3);
        let baba = t.by_tile(TileId::from_xy(1, 0)).unwrap();
        assert_eq!(baba.name, "baba");
        assert_eq!(baba.tiling, Tiling::Character);
        assert_eq!(baba.layer, 18);
        let is = t.by_tile(TileId(0x0102)).unwrap();
        assert!(is.is_text());
        assert_eq!(is.active_colour, Some(PalColour::new(0, 3)));
        assert_eq!(is.argtype, vec![0, 2]);
        assert_eq!(t.by_tile(TileId::EDGE).unwrap().id, "edge");
        assert!(t.by_tile(TileId::EMPTY).is_none());
    }

    #[test]
    fn changes() {
        let t = ObjectTable::from_values_lua(VALUES).unwrap();
        let ini = Ini::parse(
            "[tiles]\nchanged=object000,\nobject000_name=keke\nobject000_image=keke\nobject000_colour=2,2\nobject000_root=0\nobject000_layer=\nobject001_name=ignored\n",
        );
        let ch = TileChanges::from_ini(&ini);
        assert_eq!(ch.changes.len(), 1);
        let lt = t.with_changes(&ch);
        let o = lt.get("object000").unwrap();
        assert_eq!((o.name.as_str(), o.sprite.as_str()), ("keke", "keke"));
        assert_eq!(o.colour, PalColour::new(2, 2));
        assert!(!o.sprite_in_root);
        assert_eq!(o.layer, 18);
        assert_eq!(lt.get("object001").unwrap().name, "text_is");
        assert_eq!(lt.by_tile(TileId(1)).unwrap().name, "keke");
    }

    #[test]
    fn editor_list() {
        let src = r#"
            editor_objlist_order = {"baba","text_baba"}
            editor_objlist = {
                [2] = { name = "text_baba", unittype = "text", tags = {"animal"}, tiling = -1,
                        type = 0, layer = 20, colour = {4, 0}, colour_active = {4, 1} },
                [1] = { name = "baba", unittype = "object", tags = {"animal","common"},
                        tiling = 2, type = 0, layer = 18, colour = {0, 3}, advanced = true },
            }
        "#;
        let l = EditorObjectList::from_lua(src).unwrap();
        assert_eq!(l.objects[0].name, "baba");
        assert!(l.objects[0].advanced);
        assert_eq!(
            l.get("text_baba").unwrap().active_colour,
            Some(PalColour::new(4, 1))
        );
        assert_eq!(l.order, vec!["baba", "text_baba"]);
    }
}
