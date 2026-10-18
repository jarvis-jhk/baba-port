//! Per-level object definitions: the effective slot table of one level, the
//! name lookups the rules engine needs and the interned unit names.
//!
//! Built once per level session (`docs/spec/words.md` §1) and shared by
//! every clone of the level state through an `Arc`.

use std::collections::{BTreeMap, BTreeSet};

use baba_formats::{LevelInfo, ObjectTable, TileId};

/// Index of a [`Kind`] in [`Defs::kinds`].
pub type KindId = u16;
/// Interned unit / rule name (index into [`Defs::names`]).
pub type NameId = u16;

/// Word kinds (`docs/spec/words.md` §3).
pub mod wk {
    /// noun
    pub const NOUN: i32 = 0;
    /// verb
    pub const VERB: i32 = 1;
    /// property
    pub const PROP: i32 = 2;
    /// prefix condition
    pub const PREFIX: i32 = 3;
    /// NOT
    pub const NOT: i32 = 4;
    /// letter
    pub const LETTER: i32 = 5;
    /// AND
    pub const AND: i32 = 6;
    /// infix condition
    pub const INFIX: i32 = 7;
    /// custom object (notes)
    pub const CUSTOM: i32 = 8;
}

/// One object type of the level (an effective slot, or the built-in level
/// icon type).
#[derive(Clone, Debug)]
pub struct Kind {
    /// Slot id (`object012`), or `level` for the built-in icon type.
    pub slot: String,
    /// Unit name (`baba`, `text_baba`).
    pub name: String,
    /// Word string: the name without a leading `text_`.
    pub word: String,
    /// Text (word tile) rather than an object.
    pub is_text: bool,
    /// Word kind (0 for objects).
    pub word_type: i32,
    /// Draw layer.
    pub layer: i32,
    /// Tiling mode (presentation).
    pub tiling: i32,
    /// Effective argument kinds (default `[0]`).
    pub argtype: Vec<i32>,
    /// Extra argument names.
    pub argextra: Vec<String>,
    /// Interned name.
    pub name_id: NameId,
    /// Colour (inactive colour for text; presentation).
    pub colour: (u8, u8),
    /// Colour of text in a valid rule (presentation).
    pub active_colour: (u8, u8),
}

/// The definitions of one level.
#[derive(Clone, Debug)]
pub struct Defs {
    /// All kinds; index = [`KindId`].
    pub kinds: Vec<Kind>,
    by_tile: BTreeMap<u16, KindId>,
    /// Reference map: name → kind (`words.md` §1.5).
    reference: BTreeMap<String, KindId>,
    /// Level palette names (`changed_short` slots plus `level`).
    palette: BTreeSet<String>,
    /// Custom-object names (`words.md` §3.9).
    pub customs: BTreeSet<String>,
    /// Dictionary words for letters, sorted (reference names minus `text_`).
    pub dictionary: Vec<String>,
    names: Vec<String>,
    name_ids: BTreeMap<String, NameId>,
    /// World id (`baba`, `new_adv`, ...).
    pub world: String,
    /// Level file stem (`1level`).
    pub stem: String,
    /// The world is the game's base world (`progression.md` §9.1).
    pub base_world: bool,
    /// Same-name units allowed per tile (`turn.md` §11).
    pub stack_limit: usize,
    /// Kind of level icons.
    pub level_kind: KindId,
    /// Interned `text`.
    pub text_name: NameId,
    /// Interned `empty`.
    pub empty_name: NameId,
    /// Interned `level`.
    pub level_name: NameId,
}

/// A minimal definition used to build synthetic levels (tests, tools).
#[derive(Clone, Debug)]
pub struct SimpleDef {
    /// Unit name.
    pub name: String,
    /// Word kind.
    pub word_type: i32,
    /// Argument kinds (empty = default).
    pub argtype: Vec<i32>,
    /// Extra argument names.
    pub argextra: Vec<String>,
    /// Custom objects.
    pub customobjects: Vec<String>,
}

impl Defs {
    /// Effective definitions of a level: the base slots with the level's
    /// `[tiles]` overrides applied.
    pub fn from_level(base: &ObjectTable, info: &LevelInfo, world: &str, stem: &str) -> Defs {
        let eff = base.with_changes(&info.tiles);
        let mut b = Builder::default();
        // Overridden (renamed) slots claim their names first.
        let renamed: BTreeSet<&str> = info
            .tiles
            .changes
            .iter()
            .filter(|(_, c)| c.name.is_some())
            .map(|(k, _)| k.as_str())
            .collect();
        let mut tile_of: Vec<(u16, KindId)> = Vec::new();
        let mut order: Vec<(bool, KindId)> = Vec::new();
        for o in eff.iter() {
            if o.id == "edge" {
                continue;
            }
            let k = b.add_kind(
                &o.id,
                &o.name,
                o.is_text(),
                o.word_type,
                o.layer,
                o.tiling.to_i32(),
                o.argtype.clone(),
                o.argextra.clone(),
            );
            let c = (o.colour.x, o.colour.y);
            b.kinds[k as usize].colour = c;
            b.kinds[k as usize].active_colour = o.active_colour.map_or(c, |a| (a.x, a.y));
            for c in &o.customobjects {
                b.customs.insert(c.clone());
            }
            if let Some(t) = o.tile {
                tile_of.push((t.0, k));
            }
            order.push((renamed.contains(o.id.as_str()), k));
        }
        for pass in [true, false] {
            for &(r, k) in &order {
                if r == pass {
                    let name = b.kinds[k as usize].name.clone();
                    b.reference.entry(name).or_insert(k);
                }
            }
        }
        let short = info.ini.get("tiles", "changed_short").unwrap_or("");
        for s in short.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let id = format!("object{s}");
            if let Some(o) = eff.get(&id) {
                b.palette.insert(o.name.clone());
            }
        }
        let stack_limit = if world == "baba" && (stem == "89level" || stem == "33level") {
            3
        } else if world == "baba_m" && (stem == "89level" || stem == "33level") {
            2
        } else {
            6
        };
        b.finish(world, stem, world == "baba", stack_limit, tile_of)
    }

    /// Synthetic definitions from a list of names (unit tests, tools). Every
    /// def is its own slot; all names are in the reference map and palette.
    pub fn synthetic(defs: &[SimpleDef], world: &str, stem: &str, base_world: bool) -> Defs {
        let mut b = Builder::default();
        for (i, d) in defs.iter().enumerate() {
            let is_text = d.name.starts_with("text_");
            let k = b.add_kind(
                &format!("object{i:03}"),
                &d.name,
                is_text,
                d.word_type,
                if is_text { 20 } else { 10 },
                -1,
                d.argtype.clone(),
                d.argextra.clone(),
            );
            for c in &d.customobjects {
                b.customs.insert(c.clone());
            }
            b.reference.entry(d.name.clone()).or_insert(k);
            b.palette.insert(d.name.clone());
        }
        b.finish(world, stem, base_world, 6, Vec::new())
    }

    /// Kind by its effective name (reference map).
    pub fn kind_by_name(&self, name: &str) -> Option<KindId> {
        if name == "level" {
            return Some(self.level_kind);
        }
        self.reference.get(name).copied()
    }

    /// Kind for a tile id of the level grid.
    pub fn kind_by_tile(&self, tile: TileId) -> Option<KindId> {
        self.by_tile.get(&tile.0).copied()
    }

    /// The kind record.
    pub fn kind(&self, k: KindId) -> &Kind {
        &self.kinds[k as usize]
    }

    /// Is `name` defined (reference map)?
    pub fn exists(&self, name: &str) -> bool {
        self.kind_by_name(name).is_some()
    }

    /// Lookup used by letters: reference map in the base world, the level
    /// palette elsewhere (`words.md` §1.5).
    pub fn active_lookup(&self, name: &str) -> Option<KindId> {
        if self.base_world || self.palette.contains(name) {
            self.reference.get(name).copied()
        } else {
            None
        }
    }

    /// Interned id of a name, if any unit or special noun uses it.
    pub fn name_id(&self, name: &str) -> Option<NameId> {
        self.name_ids.get(name).copied()
    }

    /// The string of an interned name.
    pub fn name(&self, id: NameId) -> &str {
        &self.names[id as usize]
    }

    /// Number of interned names.
    pub fn name_count(&self) -> usize {
        self.names.len()
    }

    /// Effective argtype / argextra of word `w` via its text slot; `None`
    /// if no `text_w` definition exists (`rules.md` §11).
    pub fn word_args(&self, w: &str) -> Option<(&[i32], &[String])> {
        let k = self.reference.get(&format!("text_{w}"))?;
        let k = &self.kinds[*k as usize];
        Some((&k.argtype, &k.argextra))
    }

    /// Word kind of the text slot for word `w` (`text_w`), if defined.
    pub fn word_type_of(&self, w: &str) -> Option<i32> {
        self.reference
            .get(&format!("text_{w}"))
            .map(|k| self.kinds[*k as usize].word_type)
    }
}

#[derive(Default)]
struct Builder {
    kinds: Vec<Kind>,
    reference: BTreeMap<String, KindId>,
    palette: BTreeSet<String>,
    customs: BTreeSet<String>,
    names: Vec<String>,
    name_ids: BTreeMap<String, NameId>,
}

impl Builder {
    fn intern(&mut self, s: &str) -> NameId {
        if let Some(&i) = self.name_ids.get(s) {
            return i;
        }
        let i = self.names.len() as NameId;
        self.names.push(s.to_string());
        self.name_ids.insert(s.to_string(), i);
        i
    }

    #[allow(clippy::too_many_arguments)]
    fn add_kind(
        &mut self,
        slot: &str,
        name: &str,
        is_text: bool,
        word_type: i32,
        layer: i32,
        tiling: i32,
        argtype: Vec<i32>,
        argextra: Vec<String>,
    ) -> KindId {
        let name_id = self.intern(name);
        let word = name.strip_prefix("text_").unwrap_or(name).to_string();
        let k = self.kinds.len() as KindId;
        self.kinds.push(Kind {
            slot: slot.to_string(),
            name: name.to_string(),
            word,
            is_text,
            word_type: if is_text { word_type } else { 0 },
            layer,
            tiling,
            argtype: if argtype.is_empty() { vec![0] } else { argtype },
            argextra,
            name_id,
            colour: (0, 3),
            active_colour: (0, 3),
        });
        k
    }

    fn finish(
        mut self,
        world: &str,
        stem: &str,
        base_world: bool,
        stack_limit: usize,
        tile_of: Vec<(u16, KindId)>,
    ) -> Defs {
        let level_kind = match self.reference.get("level") {
            Some(&k) => k,
            None => self.add_kind("level", "level", false, 0, 20, -1, vec![], vec![]),
        };
        self.palette.insert("level".to_string());
        for s in [
            "text", "empty", "level", "all", "cursor", "group", "group2", "group3",
        ] {
            self.intern(s);
        }
        let mut by_tile = BTreeMap::new();
        for (t, k) in tile_of {
            by_tile.entry(t).or_insert(k);
        }
        let mut dict: BTreeSet<String> = BTreeSet::new();
        for n in self.reference.keys() {
            dict.insert(n.strip_prefix("text_").unwrap_or(n).to_string());
        }
        let text_name = self.intern("text");
        let empty_name = self.intern("empty");
        let level_name = self.intern("level");
        Defs {
            kinds: self.kinds,
            by_tile,
            reference: self.reference,
            palette: self.palette,
            customs: self.customs,
            dictionary: dict.into_iter().collect(),
            names: self.names,
            name_ids: self.name_ids,
            world: world.to_string(),
            stem: stem.to_string(),
            base_world,
            stack_limit,
            level_kind,
            text_name,
            empty_name,
            level_name,
        }
    }
}

/// Name classes of `words.md` §2.3.
pub fn is_group(n: &str) -> bool {
    n.starts_with("group")
}

/// FULL: group*, all, text, empty, level.
pub fn in_full(n: &str) -> bool {
    is_group(n) || matches!(n, "all" | "text" | "empty" | "level")
}

/// SHORT: group*, all, text, empty.
pub fn in_short(n: &str) -> bool {
    is_group(n) || matches!(n, "all" | "text" | "empty")
}

/// BRIEF: group*, all.
pub fn in_brief(n: &str) -> bool {
    is_group(n) || n == "all"
}
