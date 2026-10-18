//! Level metadata: the INI `NNNlevel.ld` files next to each `.l`.
//!
//! Indexed sections (`[levels]`, `[paths]`, `[specials]`, `[images]`,
//! `[currobjlist]`) store records as `<index><field>=value`. The record
//! count lives in `[general]` (or `total=` in `[images]`); entries with an
//! index at or above the count are stale leftovers the game ignores, and so
//! do we. See `docs/formats/levels.md`.

use std::path::Path;

use crate::error::Result;
use crate::ini::{Ini, parse_int};
use crate::objects::TileChanges;
use crate::palette::PalColour;

/// `[general]` fields. Anything not listed here is still in [`LevelInfo::ini`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct General {
    /// Display name.
    pub name: String,
    /// Subtitle shown under the name.
    pub subtitle: String,
    /// Author.
    pub author: String,
    /// Palette file name (`default.png`).
    pub palette: String,
    /// Palette lives in `Data/Palettes` (true) or the world's `Palettes/` (false).
    pub palette_root: bool,
    /// Music track name (empty = default).
    pub music: String,
    /// Background particle effect name (empty = none).
    pub particles: String,
    /// 0 = level, 1 = map.
    pub level_type: i32,
    /// Map identifier for maps (`mapid`).
    pub map_id: Option<String>,
    /// Start position of the map cursor (`selectorX/Y`, `-1` = none).
    pub selector: Option<(i32, i32)>,
    /// Number of cleared levels needed to open (`unlockcount`).
    pub unlock_count: i32,
    /// Parent level for custom levels (`customparent`).
    pub custom_parent: String,
    /// Levels unlocked by clearing this one (`unlocklevels`, comma separated).
    pub unlock_levels: Vec<String>,
    /// Record counts declared for the indexed sections.
    pub level_count: usize,
    /// Number of `[paths]` records.
    pub path_count: usize,
    /// Number of `[specials]` records.
    pub special_count: usize,
    /// Number of `[currobjlist]` records.
    pub currobjlist_total: usize,
}

/// A level/map icon placed on a map (`[levels]` record).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LevelLink {
    /// Name override (often empty: the target's own name is used).
    pub name: String,
    /// Target level file stem (`304level`).
    pub file: String,
    /// Icon colour.
    pub colour: Option<PalColour>,
    /// Icon colour once cleared.
    pub clear_colour: Option<PalColour>,
    /// Cell x.
    pub x: i32,
    /// Cell y.
    pub y: i32,
    /// Layer (`Z`).
    pub z: i32,
    /// Initial state number.
    pub state: i32,
    /// Number/letter shown on the icon.
    pub number: i32,
    /// Icon style number.
    pub style: i32,
    /// Facing direction.
    pub dir: i32,
}

/// A path tile on a map (`[paths]` record).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MapPath {
    /// Slot id of the object drawn (`object117`).
    pub object: String,
    /// Cell x.
    pub x: i32,
    /// Cell y.
    pub y: i32,
    /// Layer (`Z`).
    pub z: i32,
    /// Facing direction.
    pub dir: i32,
    /// Style number.
    pub style: i32,
    /// Gate kind (0 = none).
    pub gate: i32,
    /// Requirement amount for gates.
    pub requirement: i32,
    /// Unlock condition text (`unlock`).
    pub unlock: String,
}

/// A special object (`[specials]` record), e.g. control hints.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Special {
    /// Cell x.
    pub x: i32,
    /// Cell y.
    pub y: i32,
    /// Layer (`Z`).
    pub z: i32,
    /// Comma separated payload (`controls,up`).
    pub data: String,
}

/// A custom level-number icon (`[icons]` record).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Icon {
    /// Icon index.
    pub index: u32,
    /// Sprite file stem.
    pub file: String,
    /// Sprite in `Data/Sprites` (true) or world sprites (false).
    pub root: bool,
}

/// An entry of the level's editor palette (`[currobjlist]` record, 1-based).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CurrObj {
    /// Record index (1-based).
    pub index: u32,
    /// Editor object index (`id`).
    pub id: i32,
    /// Object name.
    pub name: String,
    /// Slot id the object occupies in this level (`object000`).
    pub object: String,
    /// Grid position in the editor list (`gox`, `goy`).
    pub go: (i32, i32),
    /// Grid position in the editor's full view (`gfx`, `gfy`).
    pub gf: (i32, i32),
    /// Index of the paired object/text record (-1 = none).
    pub pair: i32,
    /// Tile coordinate of the slot (`tile=x,y`), if stored.
    pub tile: Option<(i32, i32)>,
}

/// A slot of the editor hotbar (`[hotbar]` record).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HotbarSlot {
    /// Slot index.
    pub index: u32,
    /// Position.
    pub x: i32,
    /// Position.
    pub y: i32,
    /// Pinned.
    pub fixed: bool,
    /// Slot id of the object (`object008`).
    pub target: String,
}

/// A parsed `.ld` file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LevelInfo {
    /// The full INI, for fields without a typed accessor.
    pub ini: Ini,
    /// `[general]`.
    pub general: General,
    /// `[images]`: background image names (`Images/<name>_<frame>.png`).
    pub images: Vec<String>,
    /// `[levels]`: level icons on a map.
    pub levels: Vec<LevelLink>,
    /// `[paths]`: path tiles on a map.
    pub paths: Vec<MapPath>,
    /// `[specials]`.
    pub specials: Vec<Special>,
    /// `[icons]`.
    pub icons: Vec<Icon>,
    /// `[tiles]`: object slot overrides.
    pub tiles: TileChanges,
    /// `[currobjlist]`: the editor palette of this level.
    pub currobjlist: Vec<CurrObj>,
    /// `[hotbar]`.
    pub hotbar: Vec<HotbarSlot>,
}

fn int(ini: &Ini, s: &str, k: &str) -> Option<i32> {
    ini.get(s, k).and_then(parse_int).map(|v| v as i32)
}

fn indices(ini: &Ini, section: &str) -> Vec<u32> {
    let mut v: Vec<u32> = ini
        .entries(section)
        .iter()
        .filter_map(|(k, _)| {
            let digits: String = k.chars().take_while(char::is_ascii_digit).collect();
            (!digits.is_empty() && digits.len() < k.len())
                .then(|| digits.parse().ok())
                .flatten()
        })
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

impl LevelInfo {
    /// Reads a `.ld` file.
    pub fn load(path: &Path) -> Result<LevelInfo> {
        Ok(LevelInfo::from_ini(Ini::load(path)?))
    }

    /// Parses `.ld` text.
    pub fn parse(text: &str) -> LevelInfo {
        LevelInfo::from_ini(Ini::parse(text))
    }

    /// Interprets a parsed INI.
    pub fn from_ini(ini: Ini) -> LevelInfo {
        let g = "general";
        let s = |k: &str| ini.get(g, k).unwrap_or("").to_string();
        let count = |k: &str| int(&ini, g, k).unwrap_or(0).max(0) as usize;
        let selector = match (int(&ini, g, "selectorX"), int(&ini, g, "selectorY")) {
            (Some(x), Some(y)) if x >= 0 && y >= 0 => Some((x, y)),
            _ => None,
        };
        let general = General {
            name: s("name"),
            subtitle: s("subtitle"),
            author: s("author"),
            palette: s("palette"),
            palette_root: int(&ini, g, "paletteroot").unwrap_or(1) != 0,
            music: s("music"),
            particles: s("particles"),
            level_type: int(&ini, g, "leveltype").unwrap_or(0),
            map_id: ini
                .get(g, "mapid")
                .filter(|v| !v.is_empty())
                .map(str::to_string),
            selector,
            unlock_count: int(&ini, g, "unlockcount").unwrap_or(0),
            custom_parent: s("customparent"),
            unlock_levels: s("unlocklevels")
                .split(',')
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
                .collect(),
            level_count: count("levels"),
            path_count: count("paths"),
            special_count: count("specials"),
            currobjlist_total: count("currobjlist_total"),
        };

        let maps: std::collections::HashMap<&str, std::collections::HashMap<&str, &str>> = [
            "levels",
            "paths",
            "specials",
            "icons",
            "currobjlist",
            "hotbar",
        ]
        .into_iter()
        .map(|s| (s, ini.section_map(s)))
        .collect();
        let rec = |sec: &str, i: u32, f: &str| maps[sec].get(format!("{i}{f}").as_str()).copied();
        let rint =
            |sec: &str, i: u32, f: &str| rec(sec, i, f).and_then(parse_int).unwrap_or(0) as i32;
        let rstr = |sec: &str, i: u32, f: &str| rec(sec, i, f).unwrap_or("").to_string();

        let image_total = int(&ini, "images", "total").unwrap_or(0).max(0) as u32;
        let images = (0..image_total)
            .filter_map(|i| ini.get("images", &i.to_string()).map(str::to_string))
            .collect();

        let levels = (0..general.level_count as u32)
            .map(|i| LevelLink {
                name: rstr("levels", i, "name"),
                file: rstr("levels", i, "file"),
                colour: rec("levels", i, "colour").and_then(PalColour::parse),
                clear_colour: rec("levels", i, "clearcolour").and_then(PalColour::parse),
                x: rint("levels", i, "X"),
                y: rint("levels", i, "Y"),
                z: rint("levels", i, "Z"),
                state: rint("levels", i, "state"),
                number: rint("levels", i, "number"),
                style: rint("levels", i, "style"),
                dir: rint("levels", i, "dir"),
            })
            .collect();

        let paths = (0..general.path_count as u32)
            .map(|i| MapPath {
                object: rstr("paths", i, "object"),
                x: rint("paths", i, "X"),
                y: rint("paths", i, "Y"),
                z: rint("paths", i, "Z"),
                dir: rint("paths", i, "dir"),
                style: rint("paths", i, "style"),
                gate: rint("paths", i, "gate"),
                requirement: rint("paths", i, "requirement"),
                unlock: rstr("paths", i, "unlock"),
            })
            .collect();

        let specials = (0..general.special_count as u32)
            .map(|i| Special {
                x: rint("specials", i, "X"),
                y: rint("specials", i, "Y"),
                z: rint("specials", i, "Z"),
                data: rstr("specials", i, "data"),
            })
            .collect();

        let icons = indices(&ini, "icons")
            .into_iter()
            .filter_map(|i| {
                let file = rstr("icons", i, "file");
                (!file.is_empty()).then(|| Icon {
                    index: i,
                    file,
                    root: rint("icons", i, "root") != 0,
                })
            })
            .collect();

        let currobjlist = (1..=general.currobjlist_total as u32)
            .filter(|&i| rec("currobjlist", i, "object").is_some())
            .map(|i| CurrObj {
                index: i,
                id: rint("currobjlist", i, "id"),
                name: rstr("currobjlist", i, "name"),
                object: rstr("currobjlist", i, "object"),
                go: (rint("currobjlist", i, "gox"), rint("currobjlist", i, "goy")),
                gf: (rint("currobjlist", i, "gfx"), rint("currobjlist", i, "gfy")),
                pair: rint("currobjlist", i, "pair"),
                tile: rec("currobjlist", i, "tile").and_then(|t| {
                    let (a, b) = t.split_once(',')?;
                    Some((parse_int(a)? as i32, parse_int(b)? as i32))
                }),
            })
            .collect();

        let hotbar = indices(&ini, "hotbar")
            .into_iter()
            .map(|i| HotbarSlot {
                index: i,
                x: rint("hotbar", i, "X"),
                y: rint("hotbar", i, "Y"),
                fixed: rint("hotbar", i, "fixed") != 0,
                target: rstr("hotbar", i, "target"),
            })
            .collect();

        let tiles = TileChanges::from_ini(&ini);
        LevelInfo {
            general,
            images,
            levels,
            paths,
            specials,
            icons,
            tiles,
            currobjlist,
            hotbar,
            ini,
        }
    }

    /// True for maps (`leveltype=1`).
    pub fn is_map(&self) -> bool {
        self.general.level_type == 1
    }

    /// Serialises to an INI the way the editor writes `.ld` files: the
    /// typed [`General`] fields over the `[general]` keys of
    /// [`LevelInfo::ini`] (other keys are kept), and the indexed sections
    /// `[images]`, `[levels]`, `[icons]`, `[currobjlist]`, `[hotbar]`,
    /// `[specials]`, `[paths]`, `[tiles]` rebuilt from the typed fields
    /// (stale records dropped, counts written). Unknown sections are kept.
    pub fn to_ini(&self) -> Ini {
        let mut ini = self.ini.clone();
        let g = &self.general;
        let gs = "general";
        if !ini.has_section(gs) {
            ini.sections.insert(
                0,
                crate::ini::Section {
                    name: gs.into(),
                    entries: Vec::new(),
                },
            );
        }
        let col = |c: &Option<PalColour>| c.map(|c| format!("{},{}", c.x, c.y)).unwrap_or_default();
        ini.set(gs, "name", &g.name);
        ini.set(gs, "subtitle", &g.subtitle);
        ini.set(gs, "author", &g.author);
        ini.set(gs, "palette", &g.palette);
        ini.set(gs, "paletteroot", if g.palette_root { 1 } else { 0 });
        ini.set(gs, "music", &g.music);
        ini.set(gs, "particles", &g.particles);
        ini.set(gs, "customparent", &g.custom_parent);
        ini.set(gs, "leveltype", g.level_type);
        match &g.map_id {
            Some(m) => ini.set(gs, "mapid", m),
            None => ini.remove(gs, "mapid"),
        }
        if !g.unlock_levels.is_empty() || ini.get(gs, "unlocklevels").is_some() {
            let s: String = g.unlock_levels.iter().map(|l| format!("{l},")).collect();
            ini.set(gs, "unlocklevels", s);
        }
        ini.set(gs, "unlockcount", g.unlock_count);
        let (sx, sy) = g.selector.unwrap_or((-1, -1));
        ini.set(gs, "selectorX", sx);
        ini.set(gs, "selectorY", sy);
        ini.set(gs, "levels", self.levels.len());
        ini.set(gs, "paths", self.paths.len());
        ini.set(gs, "specials", self.specials.len());
        ini.set(gs, "currobjlist_total", self.currobjlist.len());

        let mut images = vec![("total".to_string(), self.images.len().to_string())];
        for (i, n) in self.images.iter().enumerate() {
            images.push((i.to_string(), n.clone()));
        }
        ini.replace_section("images", images);

        let mut levels: Vec<(String, String)> = ini
            .entries("levels")
            .iter()
            .filter(|(k, _)| !k.starts_with(|c: char| c.is_ascii_digit()))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        for (i, l) in self.levels.iter().enumerate() {
            let mut put = |f: &str, v: String| levels.push((format!("{i}{f}"), v));
            put("name", l.name.clone());
            put("file", l.file.clone());
            put("colour", col(&l.colour));
            put("clearcolour", col(&l.clear_colour));
            put("X", l.x.to_string());
            put("Y", l.y.to_string());
            put("Z", l.z.to_string());
            put("state", l.state.to_string());
            put("number", l.number.to_string());
            put("style", l.style.to_string());
            put("dir", l.dir.to_string());
        }
        ini.replace_section("levels", levels);

        let mut icons = Vec::new();
        let n = self
            .icons
            .iter()
            .map(|i| i.index as usize + 1)
            .max()
            .unwrap_or(0)
            .max(ICON_COUNT);
        for i in 0..n {
            match self.icons.iter().find(|ic| ic.index as usize == i) {
                Some(ic) => {
                    icons.push((format!("{i}file"), ic.file.clone()));
                    icons.push((format!("{i}root"), if ic.root { "1" } else { "0" }.into()));
                }
                None => {
                    icons.push((format!("{i}file"), String::new()));
                    icons.push((format!("{i}root"), String::new()));
                }
            }
        }
        ini.replace_section("icons", icons);

        let mut cl = Vec::new();
        for (i, c) in self.currobjlist.iter().enumerate() {
            let k = i + 1;
            let mut put = |f: &str, v: String| cl.push((format!("{k}{f}"), v));
            put("id", c.id.to_string());
            put("name", c.name.clone());
            put("object", c.object.clone());
            put("gox", c.go.0.to_string());
            put("goy", c.go.1.to_string());
            put("gfx", c.gf.0.to_string());
            put("gfy", c.gf.1.to_string());
            put("pair", c.pair.to_string());
            put(
                "tile",
                c.tile.map(|(x, y)| format!("{x},{y}")).unwrap_or_default(),
            );
        }
        ini.replace_section("currobjlist", cl);

        if !self.hotbar.is_empty() {
            let mut hb = vec![("saved".to_string(), "1".to_string())];
            for h in &self.hotbar {
                let i = h.index;
                hb.push((format!("{i}target"), h.target.clone()));
                hb.push((format!("{i}X"), h.x.to_string()));
                hb.push((format!("{i}Y"), h.y.to_string()));
                hb.push((format!("{i}fixed"), if h.fixed { "1" } else { "0" }.into()));
            }
            ini.replace_section("hotbar", hb);
        }

        let mut sp = Vec::new();
        for (i, s) in self.specials.iter().enumerate() {
            sp.push((format!("{i}X"), s.x.to_string()));
            sp.push((format!("{i}Y"), s.y.to_string()));
            sp.push((format!("{i}Z"), s.z.to_string()));
            sp.push((format!("{i}data"), s.data.clone()));
        }
        ini.replace_section("specials", sp);

        let mut ps = Vec::new();
        for (i, p) in self.paths.iter().enumerate() {
            let mut put = |f: &str, v: String| ps.push((format!("{i}{f}"), v));
            put("object", p.object.clone());
            put("unlock", p.unlock.clone());
            put("X", p.x.to_string());
            put("Y", p.y.to_string());
            put("Z", p.z.to_string());
            put("dir", p.dir.to_string());
            put("style", p.style.to_string());
            put("gate", p.gate.to_string());
            put("requirement", p.requirement.to_string());
        }
        ini.replace_section("paths", ps);

        ini.replace_section("tiles", self.tiles.to_entries());
        ini
    }

    /// [`LevelInfo::to_ini`] as `.ld` text (`\r\n` line ends).
    pub fn to_text(&self) -> String {
        self.to_ini().to_text()
    }
}

/// Number of `[icons]` slots the editor writes (empty ones included).
pub const ICON_COUNT: usize = 20;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_records() {
        let ld = LevelInfo::parse(
            "[general]\nname=map\npalette=default.png\nlevels=1\npaths=1\nspecials=0\nleveltype=1\nselectorX=3\nselectorY=4\npaletteroot=0\n\
             [images]\ntotal=1\n0=island\n\
             [levels]\n0name=\n0file=5level\n0colour=1,3\n0clearcolour=1,4\n0X=2\n0Y=6\n0Z=0\n0state=1\n0number=10\n0style=2\n\
             1file=stale\n\
             [paths]\n0object=object117\n0X=20\n0Y=13\n0dir=0\n0style=0\n0gate=2\n0requirement=5\n\
             [specials]\n0X=1\n0data=controls,up\n\
             [icons]\n1file=icon_a\n1root=0\n0file=icon_b\n0root=1\n\
             [tiles]\nchanged=object001,\nobject001_name=keke\n",
        );
        assert!(ld.is_map());
        assert_eq!(ld.general.selector, Some((3, 4)));
        assert!(!ld.general.palette_root);
        assert_eq!(ld.images, vec!["island"]);
        assert_eq!(ld.levels.len(), 1);
        assert_eq!(ld.levels[0].file, "5level");
        assert_eq!(ld.levels[0].clear_colour, Some(PalColour::new(1, 4)));
        assert_eq!(ld.paths[0].requirement, 5);
        assert!(ld.specials.is_empty());
        assert_eq!(ld.icons.len(), 2);
        assert_eq!(ld.icons[0].file, "icon_b");
        assert_eq!(
            ld.tiles.get("object001").unwrap().name.as_deref(),
            Some("keke")
        );
    }

    #[test]
    fn write_round_trip() {
        let src = "[general]\nname=map\nmystery=7\npalette=default.png\nlevels=1\npaths=1\nspecials=1\nleveltype=1\nselectorX=3\nselectorY=4\ncurrobjlist_total=2\n\
             [levels]\nlevelid=12\n0file=5level\n0colour=1,3\n0X=2\n0Y=6\n0state=1\n0number=10\n0style=2\n1file=stale\n\
             [paths]\n0object=object117\n0X=20\n0Y=13\n0gate=2\n0requirement=5\n\
             [specials]\n0X=1\n0data=controls,up\n\
             [currobjlist]\n1id=3\n1name=baba\n1object=object000\n1gox=0\n1goy=0\n1gfx=0\n1gfy=0\n1pair=2\n1tile=1,0\n\
             2id=4\n2name=text_baba\n2object=object001\n2gox=0\n2goy=0\n2gfx=0\n2gfy=1\n2pair=1\n2tile=2,0\n\
             [hotbar]\nsaved=1\n0target=object000\n0X=0\n0Y=0\n0fixed=1\n\
             [tiles]\nchanged=object001,\nobject001_name=keke\nobject001_colour=2,2\n[extra]\nk=v\n";
        let ld = LevelInfo::parse(src);
        let text = ld.to_text();
        assert!(text.contains("\r\n"));
        let back = LevelInfo::parse(&text);
        assert_eq!(back.general, ld.general);
        assert_eq!(back.levels, ld.levels);
        assert_eq!(back.paths, ld.paths);
        assert_eq!(back.specials, ld.specials);
        assert_eq!(back.currobjlist, ld.currobjlist);
        assert_eq!(back.hotbar, ld.hotbar);
        assert_eq!(back.tiles, ld.tiles);
        assert_eq!(back.ini.get("general", "mystery"), Some("7"));
        assert_eq!(back.ini.get("levels", "levelid"), Some("12"));
        assert_eq!(back.ini.get("levels", "1file"), None);
        assert_eq!(back.ini.get("extra", "k"), Some("v"));
        assert_eq!(back.ini.get("icons", "19file"), Some(""));
        assert_eq!(back.ini.get("tiles", "changed_short"), Some("001,"));
    }
}
