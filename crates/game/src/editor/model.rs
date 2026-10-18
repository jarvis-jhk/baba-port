//! The level being edited (`docs/spec/editor.md` §4–§7, §10): the tile
//! grid (three layers per cell, a facing per object), the `.ld` metadata,
//! the object palette (up to 150 catalogue objects bound to object slots,
//! with noun/text pairing and per-level overrides) and the editor quickbar.
//! Pure data and file I/O; no drawing, no input.

use std::path::{Path, PathBuf};

use baba_formats::leveldata::{CurrObj, HotbarSlot};
use baba_formats::objects::{EditorObject, EditorObjectList, ObjectChange, Tiling};
use baba_formats::{Layer, LevelInfo, LevelMap, ObjectDef, ObjectTable, PalColour, TileId};

/// Palette cap of the original (levels with more may not load there).
pub const MAX_PALETTE: usize = 150;
/// Layers per cell.
pub const LAYERS: usize = 3;
/// Quickbar slots.
pub const QUICK: usize = 10;
/// Default interior size of a new level.
pub const NEW_W: u32 = 33;
pub const NEW_H: u32 = 18;
/// Interior size limits (`§7.8`; the presets top out at 33×18).
pub const MAX_W: u32 = 33;
pub const MAX_H: u32 = 18;

/// The editor's object catalogue (`Data/Editor/editor_objectlist.lua`).
pub struct Catalogue {
    pub list: EditorObjectList,
}

impl Catalogue {
    pub fn load(data_dir: &Path) -> Catalogue {
        let list = EditorObjectList::load(&data_dir.join("Editor/editor_objectlist.lua"))
            .unwrap_or_else(|e| {
                log::error!("editor object list: {e}");
                EditorObjectList::default()
            });
        Catalogue { list }
    }

    /// Entry by catalogue index (`id` in `[currobjlist]`).
    pub fn by_id(&self, id: i32) -> Option<&EditorObject> {
        self.list.objects.iter().find(|o| o.index == id as i64)
    }

    pub fn by_name(&self, name: &str) -> Option<&EditorObject> {
        self.list.get(name)
    }

    /// Display order (`§5.5`): the catalogue order list, then anything not
    /// in it.
    pub fn ordered(&self) -> Vec<&EditorObject> {
        let mut out: Vec<&EditorObject> = self
            .list
            .order
            .iter()
            .filter_map(|n| self.list.get(n))
            .collect();
        for o in &self.list.objects {
            if !self.list.order.contains(&o.name) {
                out.push(o);
            }
        }
        out
    }

    /// Listed in the object list: never redacted; `special` only in
    /// extended mode; `advanced` only with advanced words (`§1.3`).
    pub fn visible(o: &EditorObject, extended: bool, advanced: bool) -> bool {
        !o.redacted && (!o.special || extended) && (!o.advanced || advanced)
    }

    /// All tags in first-seen order.
    pub fn tags(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for o in self.ordered() {
            for t in &o.tags {
                if !v.contains(t) {
                    v.push(t.clone());
                }
            }
        }
        v
    }
}

/// One palette entry (`[currobjlist]` record).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PalEntry {
    /// Catalogue index.
    pub id: i32,
    /// Name in this level (a duplicate gets a trailing number).
    pub name: String,
    /// Object slot (`object012`).
    pub slot: String,
    /// Grid cell in the paired ("overlap") view.
    pub go: (i32, i32),
    /// Grid cell in the full view.
    pub gf: (i32, i32),
    /// Index of the paired entry.
    pub pair: Option<usize>,
    /// Tile coordinate of the slot.
    pub tile: (i32, i32),
}

/// A quickbar slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quick {
    pub slot: String,
    pub locked: bool,
}

/// Where a level lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    /// World folder name (save group, `levels` for single levels).
    pub world: String,
    pub dir: PathBuf,
}

impl Place {
    pub fn is_pack(&self) -> bool {
        self.world != crate::packs::LEVELS
    }
}

/// One object on the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Obj {
    pub tile: TileId,
    pub dir: u8,
}

/// Why an object could not be added.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddError {
    Full,
    Present(usize),
    Unknown,
}

/// The level being edited.
pub struct EdLevel {
    pub stem: String,
    pub place: Place,
    /// Always three layers, border included.
    pub map: LevelMap,
    pub info: LevelInfo,
    pub palette: Vec<PalEntry>,
    pub quick: [Option<Quick>; QUICK],
    /// Effective object definitions (base + `[tiles]`).
    pub table: ObjectTable,
    /// The files exist on disk (saved at least once).
    pub on_disk: bool,
}

/// The cells of the palette grid in the order new entries take them:
/// growing shells (row `k` left to right, then column `k + 1` top down),
/// at most 10 rows (`§5.4`).
pub fn grid_order() -> impl Iterator<Item = (i32, i32)> {
    (0..64).flat_map(|k: i32| {
        let row: Vec<(i32, i32)> = if k <= 9 {
            (0..=k).map(|x| (x, k)).collect()
        } else {
            Vec::new()
        };
        let col: Vec<(i32, i32)> = (0..=k.min(9)).map(|y| (k + 1, y)).collect();
        row.into_iter().chain(col)
    })
}

/// Swap cycles for property words (`§5.7`), without the `text_` prefix.
const CYCLES: &[&[&str]] = &[
    &["fall", "fallright", "fallup", "fallleft"],
    &["nudgedown", "nudgeright", "nudgeup", "nudgeleft"],
    &["lockeddown", "lockedright", "lockedup", "lockedleft"],
    &["turn", "deturn"],
    &["open", "shut"],
    &["hot", "melt"],
    &[
        "power", "powered", "power2", "powered2", "power3", "powered3",
    ],
    &[
        "red", "brown", "orange", "yellow", "lime", "green", "cyan", "blue", "purple", "pink",
        "rosy",
    ],
    &["black", "grey", "silver", "white"],
];

/// Name sanitising for prompts (`§16`): drops `# @ / \ [ ] { }`.
pub fn sanitize(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '#' | '@' | '/' | '\\' | '[' | ']' | '{' | '}'))
        .collect()
}

/// An object definition from a catalogue entry, for a slot.
pub fn def_from_catalogue(slot: &ObjectDef, o: &EditorObject, name: &str) -> ObjectDef {
    ObjectDef {
        id: slot.id.clone(),
        name: name.to_string(),
        sprite: o.sprite.clone().unwrap_or_else(|| o.name.clone()),
        sprite_in_root: true,
        unittype: o.unittype.clone(),
        tiling: o.tiling,
        word_type: o.word_type,
        colour: o.colour,
        active_colour: Some(o.active_colour.unwrap_or(PalColour::new(0, 3))),
        tile: slot.tile,
        grid: slot.grid,
        layer: o.layer,
        argtype: o.argtype.clone(),
        argextra: o.argextra.clone(),
        customobjects: o.customobjects.clone(),
        does_not_exist: false,
    }
}

fn tile_xy(t: TileId) -> (i32, i32) {
    let (x, y) = t.xy();
    (x as i32, y as i32)
}

impl EdLevel {
    /// A new, empty level (`§3.2`) with the default palette (`§5.2`,
    /// outside the base world) and quickbar (`§4.3`).
    pub fn new(
        stem: &str,
        place: Place,
        name: &str,
        author: &str,
        base: &ObjectTable,
        cat: &Catalogue,
    ) -> EdLevel {
        let (w, h) = (NEW_W + 2, NEW_H + 2);
        let mut map = LevelMap::new(w, h);
        map.layers.push(Layer::new(w, h));
        map.layers.push(Layer::new(w, h));
        let mut info = LevelInfo::default();
        info.general.name = name.to_string();
        info.general.author = author.to_string();
        info.general.palette = "default.png".into();
        info.general.palette_root = true;
        info.general.music = "baba".into();
        info.ini.set("general", "localmusic", 0);
        info.ini.set("general", "rhythm", 10);
        info.ini.set("general", "levelz", 20);
        info.ini.set("general", "customruleword", "");
        for k in ["disableparticles", "disableruleeffect", "disableshake"] {
            info.ini.set("general", k, 0);
        }
        let mut l = EdLevel {
            stem: stem.to_string(),
            place,
            map,
            info,
            palette: Vec::new(),
            quick: Default::default(),
            table: base.clone(),
            on_disk: false,
        };
        l.put_border();
        l.default_palette(base, cat);
        l
    }

    fn put_border(&mut self) {
        let (w, h) = (self.width(), self.height());
        let l = &mut self.map.layers[0];
        for y in 0..h {
            for x in 0..w {
                if x == 0 || y == 0 || x == w - 1 || y == h - 1 {
                    l.tiles[(y * w + x) as usize] = TileId::EDGE;
                }
            }
        }
    }

    /// The default palette and quickbar of a new level.
    pub fn default_palette(&mut self, base: &ObjectTable, cat: &Catalogue) {
        let names = [
            "text_is",
            "baba",
            "text_you",
            "flag",
            "text_win",
            "text_and",
            "text_stop",
            "text_push",
            "wall",
        ];
        if self.place.world != "baba" {
            for n in names {
                if let Some(o) = cat.by_name(n) {
                    let _ = self.add_object(base, cat, o.index as i32, true);
                }
            }
        }
        self.default_quick();
    }

    pub fn default_quick(&mut self) {
        let names = [
            "text_is",
            "baba",
            "text_you",
            "flag",
            "text_win",
            "text_and",
            "text_stop",
            "text_push",
            "wall",
        ];
        self.quick = Default::default();
        for (i, n) in names.iter().enumerate() {
            if let Some(e) = self.palette.iter().find(|e| e.name == *n) {
                self.quick[i] = Some(Quick {
                    slot: e.slot.clone(),
                    locked: i == 0,
                });
            }
        }
    }

    /// Loads `<dir>/<stem>.l/.ld`. Levels without a saved palette get one
    /// made from the objects they use (or the default one when empty).
    pub fn load(
        place: Place,
        stem: &str,
        base: &ObjectTable,
        cat: &Catalogue,
    ) -> Result<EdLevel, String> {
        let dir = place.dir.clone();
        let map = LevelMap::load(&dir.join(format!("{stem}.l"))).map_err(|e| e.to_string())?;
        let info = LevelInfo::load(&dir.join(format!("{stem}.ld"))).map_err(|e| e.to_string())?;
        Ok(Self::from_parts(place, stem, map, info, base, cat))
    }

    /// From an already parsed `.l` + `.ld`.
    pub fn from_parts(
        place: Place,
        stem: &str,
        mut map: LevelMap,
        info: LevelInfo,
        base: &ObjectTable,
        cat: &Catalogue,
    ) -> EdLevel {
        let (w, h) = (map.width().max(3), map.height().max(3));
        while map.layers.len() < LAYERS {
            map.layers.push(Layer::new(w, h));
        }
        map.layers.truncate(LAYERS);
        for l in &mut map.layers {
            if l.dirs.is_none() {
                l.dirs = Some(vec![0; (l.width * l.height) as usize]);
            }
        }
        let table = base.with_changes(&info.tiles);
        let palette: Vec<PalEntry> = info
            .currobjlist
            .iter()
            .map(|c| PalEntry {
                id: c.id,
                name: c.name.clone(),
                slot: c.object.clone(),
                go: c.go,
                gf: c.gf,
                pair: (c.pair >= 1).then(|| c.pair as usize - 1),
                tile: c.tile.unwrap_or_else(|| {
                    table
                        .get(&c.object)
                        .and_then(|d| d.tile)
                        .map(tile_xy)
                        .unwrap_or((0, 0))
                }),
            })
            .collect();
        let mut l = EdLevel {
            stem: stem.to_string(),
            place,
            map,
            info,
            palette,
            quick: Default::default(),
            table,
            on_disk: true,
        };
        // Pairs must point inside the list.
        let n = l.palette.len();
        for e in &mut l.palette {
            if e.pair.is_some_and(|p| p >= n) {
                e.pair = None;
            }
        }
        if l.palette.is_empty() {
            l.palette_from_grid(cat);
        }
        if l.palette.is_empty() {
            l.default_palette(base, cat);
        }
        l.load_quick();
        l
    }

    /// A palette for a level that has none: every slot it uses.
    fn palette_from_grid(&mut self, cat: &Catalogue) {
        let mut used: Vec<TileId> = Vec::new();
        for layer in &self.map.layers {
            for t in &layer.tiles {
                if !t.is_empty() && *t != TileId::EDGE && !used.contains(t) {
                    used.push(*t);
                }
            }
        }
        for t in used {
            if self.palette.len() >= MAX_PALETTE {
                break;
            }
            let Some(d) = self.table.by_tile(t).cloned() else {
                continue;
            };
            let id = cat.by_name(&d.name).map(|o| o.index as i32).unwrap_or(0);
            let go = self.free_cell(true, None);
            let gf = self.free_cell(false, None);
            self.palette.push(PalEntry {
                id,
                name: d.name.clone(),
                slot: d.id.clone(),
                go,
                gf,
                pair: None,
                tile: tile_xy(t),
            });
        }
        self.link_pairs();
    }

    /// Pairs noun objects with their texts already in the palette.
    fn link_pairs(&mut self) {
        let n = self.palette.len();
        for i in 0..n {
            if self.palette[i].pair.is_some() {
                continue;
            }
            let name = self.palette[i].name.clone();
            let want = match name.strip_prefix("text_") {
                Some(_) => continue,
                None => format!("text_{name}"),
            };
            if let Some(j) =
                (0..n).find(|&j| self.palette[j].name == want && self.palette[j].pair.is_none())
            {
                self.palette[i].pair = Some(j);
                self.palette[j].pair = Some(i);
                self.palette[j].go = self.palette[i].go;
            }
        }
    }

    fn load_quick(&mut self) {
        self.quick = Default::default();
        if self.info.hotbar.is_empty() {
            self.default_quick();
            return;
        }
        for h in &self.info.hotbar {
            let i = h.index as usize;
            if i < QUICK && !h.target.is_empty() && self.entry_by_slot(&h.target).is_some() {
                self.quick[i] = Some(Quick {
                    slot: h.target.clone(),
                    locked: h.fixed,
                });
            }
        }
    }

    pub fn width(&self) -> u32 {
        self.map.width()
    }

    pub fn height(&self) -> u32 {
        self.map.height()
    }

    /// Inside the playable area (not the border).
    pub fn inside(&self, x: i32, y: i32) -> bool {
        x >= 1 && y >= 1 && x < self.width() as i32 - 1 && y < self.height() as i32 - 1
    }

    pub fn get(&self, layer: usize, x: i32, y: i32) -> Option<Obj> {
        if !self.inside(x, y) {
            return None;
        }
        let l = &self.map.layers[layer];
        let i = (y as u32 * l.width + x as u32) as usize;
        let t = l.tiles[i];
        (!t.is_empty() && t != TileId::EDGE).then(|| Obj {
            tile: t,
            dir: l.dirs.as_ref().map_or(0, |d| d[i]) & 3,
        })
    }

    /// Sets or clears a cell; returns the previous content.
    pub fn set(&mut self, layer: usize, x: i32, y: i32, o: Option<Obj>) -> Option<Obj> {
        if !self.inside(x, y) {
            return None;
        }
        let old = self.get(layer, x, y);
        let l = &mut self.map.layers[layer];
        let i = (y as u32 * l.width + x as u32) as usize;
        match o {
            Some(o) => {
                l.tiles[i] = o.tile;
                if let Some(d) = &mut l.dirs {
                    d[i] = o.dir & 3;
                }
            }
            None => {
                l.tiles[i] = TileId::EMPTY;
            }
        }
        old
    }

    /// Every object (layer, x, y, obj).
    pub fn objects(&self) -> Vec<(usize, i32, i32, Obj)> {
        let mut v = Vec::new();
        for z in 0..LAYERS {
            for y in 1..self.height() as i32 - 1 {
                for x in 1..self.width() as i32 - 1 {
                    if let Some(o) = self.get(z, x, y) {
                        v.push((z, x, y, o));
                    }
                }
            }
        }
        v
    }

    /// Removes everything (objects, icons, paths, specials; `§6.11`).
    pub fn clear(&mut self) {
        for (z, x, y, _) in self.objects() {
            self.set(z, x, y, None);
        }
        self.info.levels.clear();
        self.info.paths.clear();
        self.info.specials.clear();
    }

    /// Shifts every object one cell, wrapping inside the interior (`§6.9`).
    pub fn move_all(&mut self, dir: u8) {
        let (iw, ih) = (self.width() as i32 - 2, self.height() as i32 - 2);
        if iw <= 0 || ih <= 0 {
            return;
        }
        let (dx, dy) = baba_sim::dir_delta(dir);
        let objs = self.objects();
        for (z, x, y, _) in &objs {
            self.set(*z, *x, *y, None);
        }
        let wrap = |v: i32, d: i32, n: i32| (v - 1 + d).rem_euclid(n) + 1;
        for (z, x, y, o) in objs {
            self.set(z, wrap(x, dx, iw), wrap(y, dy, ih), Some(o));
        }
        let mv = |x: &mut i32, y: &mut i32| {
            *x = wrap(*x, dx, iw);
            *y = wrap(*y, dy, ih);
        };
        for l in &mut self.info.levels {
            mv(&mut l.x, &mut l.y);
        }
        for p in &mut self.info.paths {
            mv(&mut p.x, &mut p.y);
        }
        for s in &mut self.info.specials {
            mv(&mut s.x, &mut s.y);
        }
    }

    /// New interior size (`§7.8`): objects keep their cell from the top
    /// left; what falls outside is dropped.
    pub fn resize(&mut self, iw: u32, ih: u32) {
        let (iw, ih) = (iw.clamp(1, MAX_W), ih.clamp(1, MAX_H));
        let (w, h) = (iw + 2, ih + 2);
        let objs = self.objects();
        let mut map = LevelMap::new(w, h);
        map.version = self.map.version;
        map.map = self.map.map.clone();
        map.other_chunks = self.map.other_chunks.clone();
        map.layers.push(Layer::new(w, h));
        map.layers.push(Layer::new(w, h));
        for (i, l) in self.map.layers.iter().enumerate() {
            map.layers[i].params = l.params;
        }
        self.map = map;
        self.put_border();
        for (z, x, y, o) in objs {
            self.set(z, x, y, Some(o));
        }
        let inside = |x: i32, y: i32| x >= 1 && y >= 1 && x <= iw as i32 && y <= ih as i32;
        self.info.levels.retain(|l| inside(l.x, l.y));
        self.info.paths.retain(|p| inside(p.x, p.y));
        self.info.specials.retain(|s| inside(s.x, s.y));
    }

    // ------------------------------------------------------------ palette

    pub fn entry_by_slot(&self, slot: &str) -> Option<usize> {
        self.palette.iter().position(|e| e.slot == slot)
    }

    pub fn entry_by_tile(&self, t: TileId) -> Option<usize> {
        let xy = tile_xy(t);
        self.palette.iter().position(|e| e.tile == xy)
    }

    pub fn entry_by_name(&self, name: &str) -> Option<usize> {
        self.palette.iter().position(|e| e.name == name)
    }

    /// Effective definition of an entry's slot.
    pub fn def(&self, i: usize) -> Option<&ObjectDef> {
        self.table.get(&self.palette.get(i)?.slot)
    }

    pub fn tile_of(&self, i: usize) -> Option<TileId> {
        let e = self.palette.get(i)?;
        Some(TileId::from_xy(e.tile.0 as u8, e.tile.1 as u8))
    }

    fn free_cell(&self, overlap: bool, pair_of: Option<usize>) -> (i32, i32) {
        if let (true, Some(p)) = (overlap, pair_of) {
            return self.palette[p].go;
        }
        grid_order()
            .find(|c| {
                !self
                    .palette
                    .iter()
                    .any(|e| if overlap { e.go == *c } else { e.gf == *c })
            })
            .unwrap_or((0, 0))
    }

    /// Slot for a new entry named `name`: the base slot of that name if it
    /// is free and not renamed, else the first free `objectNNN`.
    fn free_slot(&self, base: &ObjectTable, name: &str) -> Option<String> {
        let used = |s: &str| self.palette.iter().any(|e| e.slot == s);
        if let Some(d) = base.iter().find(|d| d.name == name && d.id != "edge") {
            let renamed = self
                .info
                .tiles
                .get(&d.id)
                .and_then(|c| c.name.as_deref())
                .is_some_and(|n| n != name);
            if !used(&d.id) && !renamed && d.tile.is_some() {
                return Some(d.id.clone());
            }
        }
        (0..1000)
            .map(|i| format!("object{i:03}"))
            .find(|s| !used(s) && base.get(s).is_some_and(|d| d.tile.is_some()))
    }

    /// Adds catalogue entry `id` (`§5.3`): with `pairs`, also its noun
    /// text / object and any companion word. Returns the new indices.
    pub fn add_object(
        &mut self,
        base: &ObjectTable,
        cat: &Catalogue,
        id: i32,
        pairs: bool,
    ) -> Result<Vec<usize>, AddError> {
        self.add_inner(base, cat, id, pairs, None, true)
    }

    fn add_inner(
        &mut self,
        base: &ObjectTable,
        cat: &Catalogue,
        id: i32,
        pairs: bool,
        pair_of: Option<usize>,
        companion: bool,
    ) -> Result<Vec<usize>, AddError> {
        let o = cat.by_id(id).ok_or(AddError::Unknown)?.clone();
        if let Some(i) = self.palette.iter().position(|e| e.id == id) {
            return Err(AddError::Present(i));
        }
        if self.palette.len() >= MAX_PALETTE {
            return Err(AddError::Full);
        }
        let mut name = o.name.clone();
        while self.palette.iter().any(|e| e.name == name) {
            name = match name.chars().last().and_then(|c| c.to_digit(10)) {
                Some(d) => format!("{}{}", &name[..name.len() - 1], d + 1),
                None => format!("{name}2"),
            };
        }
        let slot = self.free_slot(base, &name).ok_or(AddError::Full)?;
        let sdef = base.get(&slot).ok_or(AddError::Unknown)?.clone();
        let want = def_from_catalogue(&sdef, &o, &name);
        self.info
            .tiles
            .changes
            .insert(slot.clone(), ObjectChange::between(&sdef, &want));
        let go = self.free_cell(true, pair_of);
        let gf = self.free_cell(false, None);
        let tile = sdef.tile.map(tile_xy).unwrap_or((0, 0));
        self.palette.push(PalEntry {
            id,
            name: name.clone(),
            slot,
            go,
            gf,
            pair: pair_of,
            tile,
        });
        let me = self.palette.len() - 1;
        let mut added = vec![me];
        self.refresh_table(base);
        if pairs && pair_of.is_none() && o.word_type == 0 && !o.unpaired {
            let partner = if o.unittype == "object" {
                Some(format!("text_{}", o.name))
            } else {
                o.name.strip_prefix("text_").map(str::to_string)
            };
            if let Some(p) = partner
                && let Some(po) = cat.by_name(&p).cloned()
                && po.word_type == 0
                && po.unittype != o.unittype
                && let Ok(v) = self.add_inner(base, cat, po.index as i32, false, Some(me), false)
            {
                self.palette[me].pair = v.first().copied();
                added.extend(v);
            }
        }
        if companion
            && let Some(c) = o.paired_with.clone()
            && !self.palette.iter().any(|e| e.name == c)
            && let Some(co) = cat.by_name(&c).cloned()
            && co.word_type != 0
            && co.unittype == "text"
            && let Ok(v) = self.add_inner(base, cat, co.index as i32, false, None, false)
        {
            added.extend(v);
        }
        Ok(added)
    }

    /// Removes entry `i` (and with `pairs` its pair): its overrides, every
    /// placed instance and quickbar slot (`§5.3`). Returns the removed slots.
    pub fn remove_entry(&mut self, base: &ObjectTable, i: usize, pairs: bool) -> Vec<String> {
        let mut idx = vec![i];
        if pairs && let Some(p) = self.palette.get(i).and_then(|e| e.pair) {
            idx.push(p);
        }
        idx.sort_unstable();
        idx.dedup();
        let mut removed = Vec::new();
        for &k in idx.iter().rev() {
            if k >= self.palette.len() {
                continue;
            }
            let e = self.palette.remove(k);
            for o in &mut self.palette {
                o.pair = match o.pair {
                    Some(p) if p == k => None,
                    Some(p) if p > k => Some(p - 1),
                    p => p,
                };
            }
            self.info.tiles.changes.remove(&e.slot);
            let t = TileId::from_xy(e.tile.0 as u8, e.tile.1 as u8);
            for (z, x, y, o) in self.objects() {
                if o.tile == t {
                    self.set(z, x, y, None);
                }
            }
            self.info.paths.retain(|p| p.object != e.slot);
            for q in &mut self.quick {
                if q.as_ref().is_some_and(|q| q.slot == e.slot) {
                    *q = None;
                }
            }
            removed.push(e.slot);
        }
        self.refresh_table(base);
        removed
    }

    pub fn refresh_table(&mut self, base: &ObjectTable) {
        self.table = base.with_changes(&self.info.tiles);
    }

    /// Changes the definition of entry `i`'s slot (`§5.6`).
    pub fn set_def(&mut self, base: &ObjectTable, i: usize, want: ObjectDef) {
        let Some(e) = self.palette.get(i) else { return };
        let slot = e.slot.clone();
        let Some(sdef) = base.get(&slot) else { return };
        self.info
            .tiles
            .changes
            .insert(slot, ObjectChange::between(sdef, &want));
        if let Some(e) = self.palette.get_mut(i) {
            e.name = want.name.clone();
        }
        self.refresh_table(base);
    }

    /// Resets entry `i` to its catalogue definition.
    pub fn reset_def(&mut self, base: &ObjectTable, cat: &Catalogue, i: usize) {
        let Some(e) = self.palette.get(i).cloned() else {
            return;
        };
        let (Some(o), Some(sdef)) = (cat.by_id(e.id), base.get(&e.slot)) else {
            return;
        };
        let want = def_from_catalogue(sdef, o, &o.name.clone());
        self.set_def(base, i, want);
    }

    /// Renames entry `i` (`§5.6`); false if the name is refused.
    /// The noun/text partner follows.
    pub fn rename(&mut self, base: &ObjectTable, i: usize, raw: &str) -> bool {
        let mut name: String = raw
            .to_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        while name.starts_with("text_text_") {
            name = name["text_".len()..].to_string();
        }
        if name.is_empty()
            || name.len() > 24
            || matches!(
                name.as_str(),
                "level" | "crash" | "error" | "never" | "text_"
            )
        {
            return false;
        }
        let Some(e) = self.palette.get(i).cloned() else {
            return false;
        };
        let is_text = self.def(i).is_some_and(ObjectDef::is_text);
        let (noun, text) = match name.strip_prefix("text_") {
            Some(n) => (n.to_string(), name.clone()),
            None => (name.clone(), format!("text_{name}")),
        };
        let mine = if is_text { text.clone() } else { noun.clone() };
        let other = if is_text { noun } else { text };
        // The counterpart name must not be a separate entry.
        if let Some(j) = self.entry_by_name(&other)
            && Some(j) != e.pair
        {
            return false;
        }
        let mut mine = mine;
        while self
            .palette
            .iter()
            .enumerate()
            .any(|(k, p)| k != i && p.name == mine)
        {
            mine.push('2');
        }
        if let Some(mut d) = self.def(i).cloned() {
            d.name = mine;
            self.set_def(base, i, d);
        }
        if let Some(p) = e.pair
            && let Some(mut d) = self.def(p).cloned()
        {
            d.name = other;
            self.set_def(base, p, d);
        }
        true
    }

    /// The entry that Swap turns entry `i` into (`§5.7`), if any.
    pub fn swap_target(&self, i: usize) -> Option<usize> {
        let e = self.palette.get(i)?;
        if let Some(p) = e.pair {
            return Some(p);
        }
        let n = e.name.strip_prefix("text_")?;
        let cycle = CYCLES.iter().find(|c| c.contains(&n))?;
        let k = cycle.iter().position(|c| *c == n)?;
        (1..cycle.len()).find_map(|d| {
            let next = format!("text_{}", cycle[(k + d) % cycle.len()]);
            self.entry_by_name(&next)
        })
    }

    /// Moves an entry (and in the paired view its pair) to a grid cell.
    pub fn move_entry(&mut self, i: usize, cell: (i32, i32), overlap: bool) {
        let pair = self.palette.get(i).and_then(|e| e.pair);
        if let Some(e) = self.palette.get_mut(i) {
            if overlap {
                e.go = cell;
            } else {
                e.gf = cell;
            }
        }
        if overlap
            && let Some(p) = pair
            && let Some(e) = self.palette.get_mut(p)
        {
            e.go = cell;
        }
    }

    // --------------------------------------------------------------- files

    /// The palette and quickbar written back into the metadata.
    pub fn sync_info(&mut self) {
        self.info.currobjlist = self
            .palette
            .iter()
            .enumerate()
            .map(|(i, e)| CurrObj {
                index: i as u32 + 1,
                id: e.id,
                name: e.name.clone(),
                object: e.slot.clone(),
                go: e.go,
                gf: e.gf,
                pair: e.pair.map_or(-1, |p| p as i32 + 1),
                tile: Some(e.tile),
            })
            .collect();
        self.info.general.currobjlist_total = self.palette.len();
        self.info.hotbar = (0..QUICK)
            .map(|i| {
                let q = self.quick[i].as_ref();
                let pos = q
                    .and_then(|q| self.entry_by_slot(&q.slot))
                    .map(|k| self.palette[k].go)
                    .unwrap_or((-1, -1));
                HotbarSlot {
                    index: i as u32,
                    x: pos.0,
                    y: pos.1,
                    fixed: q.is_some_and(|q| q.locked),
                    target: q.map(|q| q.slot.clone()).unwrap_or_default(),
                }
            })
            .collect();
        self.info.general.level_count = self.info.levels.len();
        self.info.general.path_count = self.info.paths.len();
        self.info.general.special_count = self.info.specials.len();
    }

    /// The `.l` and `.ld` bytes.
    pub fn file_bytes(&mut self) -> (Vec<u8>, Vec<u8>) {
        self.sync_info();
        (self.map.to_bytes(), self.info.to_text().into_bytes())
    }

    /// Writes `.l` and `.ld` (not the preview) into the level's folder.
    pub fn save(&mut self) -> std::io::Result<()> {
        // A code written into the file by an upload meanwhile is kept.
        if self.info.ini.get("general", "levelcode").is_none()
            && let Ok(old) = LevelInfo::load(&self.place.dir.join(format!("{}.ld", self.stem)))
            && let Some(code) = old.ini.get("general", "levelcode")
        {
            self.info.ini.set("general", "levelcode", code);
        }
        let (l, ld) = self.file_bytes();
        std::fs::create_dir_all(&self.place.dir)?;
        write_atomic(&self.place.dir.join(format!("{}.l", self.stem)), &l)?;
        write_atomic(&self.place.dir.join(format!("{}.ld", self.stem)), &ld)?;
        self.on_disk = true;
        Ok(())
    }

    /// The level as `baba-formats` data (sim, previews).
    pub fn as_level(&mut self) -> baba_formats::Level {
        self.sync_info();
        baba_formats::Level {
            stem: self.stem.clone(),
            map: self.map.clone(),
            info: self.info.clone(),
        }
    }

    // ---------------------------------------------------------- settings

    pub fn gstr(&self, key: &str) -> String {
        self.info.ini.get("general", key).unwrap_or("").to_string()
    }

    pub fn gen_int(&self, key: &str) -> i64 {
        self.info.ini.get_int("general", key).unwrap_or(0)
    }

    pub fn set_gen(&mut self, key: &str, v: impl ToString) {
        self.info.ini.set("general", key, v);
    }

    /// Applies a theme (`§5.8`): palette, overrides, palette file, music,
    /// particles; the level is emptied.
    pub fn apply_theme(&mut self, base: &ObjectTable, cat: &Catalogue, theme: &LevelInfo) {
        self.clear();
        self.info.tiles = theme.tiles.clone();
        let g = &theme.general;
        if !g.palette.is_empty() {
            self.info.general.palette = g.palette.clone();
            self.info.general.palette_root = g.palette_root;
        }
        self.info.general.music = g.music.clone();
        self.info.general.particles = g.particles.clone();
        self.refresh_table(base);
        let tmp = EdLevel::from_parts(
            self.place.clone(),
            &self.stem,
            self.map.clone(),
            {
                let mut i = self.info.clone();
                i.currobjlist = theme.currobjlist.clone();
                i.general.currobjlist_total = theme.currobjlist.len();
                i.hotbar.clear();
                i
            },
            base,
            cat,
        );
        self.palette = tmp.palette;
        self.quick = tmp.quick;
        // Overrides of slots the theme's palette does not use are dropped.
        let slots: Vec<String> = self.palette.iter().map(|e| e.slot.clone()).collect();
        self.info.tiles.changes.retain(|k, _| slots.contains(k));
        self.refresh_table(base);
        self.default_quick();
    }

    /// A theme file from this level (`§5.8`).
    pub fn theme_text(&mut self, name: &str) -> String {
        self.sync_info();
        let mut t = LevelInfo::default();
        t.general.name = name.to_string();
        t.general.palette = self.info.general.palette.clone();
        t.general.palette_root = self.info.general.palette_root;
        t.general.music = self.info.general.music.clone();
        t.general.particles = self.info.general.particles.clone();
        t.tiles = self.info.tiles.clone();
        t.currobjlist = self.info.currobjlist.clone();
        t.general.currobjlist_total = t.currobjlist.len();
        let mut ini = t.to_ini();
        // A theme has only these sections.
        ini.sections.retain(|s| {
            matches!(
                s.name.as_str(),
                "general" | "tiles" | "icons" | "currobjlist"
            )
        });
        for k in [
            "subtitle",
            "author",
            "customparent",
            "leveltype",
            "unlockcount",
            "selectorX",
            "selectorY",
            "levels",
            "paths",
            "specials",
        ] {
            ini.remove("general", k);
        }
        ini.to_text()
    }
}

/// Writes through a temporary file and a rename.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// Animation style choices (`§5.6`) as tiling values.
pub const ANIM_STYLES: [(Tiling, &str); 6] = [
    (Tiling::None, "editor_object_animation_none"),
    (Tiling::Directional, "editor_object_animation_dirs"),
    (Tiling::Animated, "editor_object_animation_anim"),
    (
        Tiling::AnimatedDirectional,
        "editor_object_animation_animdirs",
    ),
    (Tiling::Character, "editor_object_animation_character"),
    (Tiling::Tiled, "editor_object_animation_tiled"),
];

/// Text types (`§5.6`): label word and type number.
pub const TEXT_TYPES: [(i32, &str); 6] = [
    (0, "baba"),
    (1, "is"),
    (2, "you"),
    (3, "lonely"),
    (7, "on"),
    (5, "ba"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_shells() {
        let v: Vec<(i32, i32)> = grid_order().take(8).collect();
        assert_eq!(
            v,
            vec![
                (0, 0),
                (1, 0),
                (0, 1),
                (1, 1),
                (2, 0),
                (2, 1),
                (0, 2),
                (1, 2)
            ]
        );
        // 150 distinct cells within 15 × 10.
        let mut all: Vec<(i32, i32)> = grid_order().take(150).collect();
        assert!(all.iter().all(|(x, y)| *x < 15 && *y < 10));
        all.sort();
        all.dedup();
        assert_eq!(all.len(), 150);
    }

    /// Every level of every world in the player's install loads into the
    /// editor and writes back to files that parse to the same grid and
    /// metadata (`§10`; the original's own levels and packs). Skips
    /// without `BABA_GAME_DIR`.
    #[test]
    fn install_levels_round_trip() {
        let Some(dir) = std::env::var_os("BABA_GAME_DIR")
            .filter(|d| Path::new(d).join("Data/values.lua").is_file())
        else {
            eprintln!("skipping: no install at BABA_GAME_DIR");
            return;
        };
        let data = Path::new(&dir).join("Data");
        let base = ObjectTable::load(&data.join("values.lua")).unwrap();
        let cat = Catalogue::load(&data);
        assert!(cat.list.objects.len() > 400);
        let mut n = 0;
        for w in std::fs::read_dir(data.join("Worlds")).unwrap().flatten() {
            let wdir = w.path();
            let world = w.file_name().to_string_lossy().into_owned();
            for f in std::fs::read_dir(&wdir).unwrap().flatten() {
                let p = f.path();
                if p.extension().is_none_or(|e| e != "l") || !p.with_extension("ld").is_file() {
                    continue;
                }
                let stem = p.file_stem().unwrap().to_string_lossy().into_owned();
                let place = Place {
                    world: world.clone(),
                    dir: wdir.clone(),
                };
                let orig_map = LevelMap::load(&p).unwrap();
                let orig = LevelInfo::load(&p.with_extension("ld")).unwrap();
                let mut l = EdLevel::load(place, &stem, &base, &cat).unwrap();
                let (lb, ldb) = l.file_bytes();
                let map = LevelMap::parse(&lb).unwrap_or_else(|e| panic!("{world}/{stem}: {e}"));
                let info = LevelInfo::parse(&String::from_utf8(ldb).unwrap());
                assert_eq!(
                    (map.width(), map.height()),
                    (orig_map.width(), orig_map.height())
                );
                assert_eq!(map.layers.len(), LAYERS);
                for (z, ol) in orig_map.layers.iter().enumerate() {
                    for (x, y, t, d) in ol.objects() {
                        let nl = &map.layers[z];
                        assert_eq!(nl.tile(x, y), Some(t), "{world}/{stem} layer {z} {x},{y}");
                        assert_eq!(nl.dir(x, y), d, "{world}/{stem} dir {z} {x},{y}");
                    }
                    assert_eq!(
                        ol.objects().count(),
                        map.layers[z].objects().count(),
                        "{world}/{stem} layer {z}"
                    );
                }
                let empty = orig_map
                    .layers
                    .iter()
                    .all(|l| l.objects().all(|(_, _, t, _)| t == TileId::EDGE));
                let mut g = info.general.clone();
                let mut og = orig.general.clone();
                g.currobjlist_total = 0;
                og.currobjlist_total = 0;
                assert_eq!(g, og, "{world}/{stem}");
                assert_eq!(info.levels, orig.levels, "{world}/{stem}");
                assert_eq!(info.paths, orig.paths, "{world}/{stem}");
                assert_eq!(info.specials, orig.specials, "{world}/{stem}");
                assert_eq!(info.images, orig.images, "{world}/{stem}");
                assert_eq!(info.icons, orig.icons, "{world}/{stem}");
                if !empty {
                    assert_eq!(info.tiles, orig.tiles, "{world}/{stem}");
                }
                if !orig.currobjlist.is_empty() {
                    // Older files have no `tile` field; we always write it.
                    let strip = |v: &[CurrObj], keep: &[CurrObj]| -> Vec<CurrObj> {
                        v.iter()
                            .zip(keep)
                            .map(|(c, k)| CurrObj {
                                tile: if k.tile.is_some() { c.tile } else { None },
                                ..c.clone()
                            })
                            .collect()
                    };
                    assert_eq!(
                        strip(&info.currobjlist, &orig.currobjlist),
                        orig.currobjlist,
                        "{world}/{stem}"
                    );
                }
                for (k, v) in orig.ini.entries("general") {
                    if k != "currobjlist_total" && k != "unlocklevels" {
                        assert_eq!(info.ini.get("general", k), Some(v), "{world}/{stem} {k}");
                    }
                }
                n += 1;
            }
        }
        eprintln!("{n} levels round-tripped");
        assert!(n > 500);
    }

    #[test]
    fn sanitizes() {
        assert_eq!(sanitize("a#b@c/d\\e[f]g{h}i, j!"), "abcdefghi, j!");
    }
}
