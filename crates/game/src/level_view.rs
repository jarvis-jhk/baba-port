//! A level from the player's install turned into a [`TileScene`]: object
//! definitions + the level's overrides, palette, sprites (world folder
//! first), tiling masks, colours, draw order and random wobble phases
//! (`docs/spec/presentation.md` §1–§5).

use std::path::{Path, PathBuf};

use baba_formats::level::{Direction, LevelMap};
use baba_formats::leveldata::LevelInfo;
use baba_formats::objects::{ObjectDef, ObjectTable, TileId, Tiling};
use baba_formats::palette::{PalColour, Palette};
use baba_formats::sprites::sprite_variant;
use baba_render::{Atlas, SceneSprite, SpriteBank, TileScene};

/// Where the game's data lives.
pub struct GameData {
    pub root: PathBuf,
    pub objects: ObjectTable,
}

impl GameData {
    pub fn open(game_dir: &Path) -> Result<Self, String> {
        let data = game_dir.join("Data");
        let objects = ObjectTable::load(&data.join("values.lua")).map_err(|e| e.to_string())?;
        Ok(Self {
            root: data,
            objects,
        })
    }
}

/// One object on the grid (presentation only; the sim owns real units).
#[derive(Clone, Debug)]
struct Cell {
    x: u32,
    y: u32,
    def: usize,
    dir: Direction,
}

/// A loaded level and everything needed to draw it.
pub struct LoadedLevel {
    pub world: String,
    pub info: LevelInfo,
    pub map: LevelMap,
    pub objects: Vec<ObjectDef>,
    pub palette: Palette,
    cells: Vec<Cell>,
}

impl LoadedLevel {
    /// `spec` is `<world>/<file>` (e.g. `baba/0level`).
    pub fn load(data: &GameData, spec: &str) -> Result<Self, String> {
        let (world, file) = spec
            .split_once('/')
            .ok_or_else(|| format!("level must be <world>/<file>, got {spec:?}"))?;
        let file = file.trim_end_matches(".l").trim_end_matches(".ld");
        let wdir = data.root.join("Worlds").join(world);
        let info = LevelInfo::load(&wdir.join(format!("{file}.ld"))).map_err(|e| e.to_string())?;
        let map = LevelMap::load(&wdir.join(format!("{file}.l"))).map_err(|e| e.to_string())?;
        let table = data.objects.with_changes(&info.tiles);
        let pal_dir = if info.general.palette_root {
            data.root.join("Palettes")
        } else {
            wdir.join("Palettes")
        };
        let pal_name = if info.general.palette.is_empty() {
            "default.png"
        } else {
            &info.general.palette
        };
        let palette = Palette::load(&pal_dir.join(pal_name))
            .or_else(|_| Palette::load(&data.root.join("Palettes").join(pal_name)))
            .map_err(|e| e.to_string())?;
        let objects: Vec<ObjectDef> = table.iter().cloned().collect();
        let mut cells = Vec::new();
        for layer in &map.layers {
            for (x, y, tile, dir) in layer.objects() {
                if tile == TileId::EDGE {
                    continue; // the border ring is not drawn (§1.5)
                }
                match objects.iter().position(|o| o.tile == Some(tile)) {
                    Some(def) => cells.push(Cell { x, y, def, dir }),
                    None => log::warn!("{spec}: unknown tile {:?} at {x},{y}", tile.xy()),
                }
            }
        }
        Ok(Self {
            world: world.to_string(),
            info,
            map,
            objects,
            palette,
            cells,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        (self.map.width(), self.map.height())
    }

    fn colour(&self, c: PalColour) -> [u8; 4] {
        let [r, g, b] = self.palette.rgb(c);
        [r, g, b, 255]
    }

    /// Text that forms `NOUN VERB NOUN|PROPERTY` in a row or column.
    /// Placeholder until `baba-sim` reports rule state (rules.md); used
    /// only to pick the active text colour.
    fn active_text(&self) -> Vec<bool> {
        let (w, h) = self.size();
        let mut grid: Vec<Vec<usize>> = vec![Vec::new(); (w * h) as usize];
        for (i, c) in self.cells.iter().enumerate() {
            if self.objects[c.def].is_text() {
                grid[(c.y * w + c.x) as usize].push(i);
            }
        }
        let ty = |x: i64, y: i64, want: &[i32]| -> Vec<usize> {
            if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
                return Vec::new();
            }
            grid[(y as u32 * w + x as u32) as usize]
                .iter()
                .copied()
                .filter(|&i| want.contains(&self.objects[self.cells[i].def].word_type))
                .collect()
        };
        let mut active = vec![false; self.cells.len()];
        for y in 0..h as i64 {
            for x in 0..w as i64 {
                for (dx, dy) in [(1, 0), (0, 1)] {
                    let a = ty(x, y, &[0]);
                    let v = ty(x + dx, y + dy, &[1]);
                    let b = ty(x + 2 * dx, y + 2 * dy, &[0, 2]);
                    if !a.is_empty() && !v.is_empty() && !b.is_empty() {
                        for i in a.into_iter().chain(v).chain(b) {
                            active[i] = true;
                        }
                    }
                }
            }
        }
        active
    }

    /// Builds the scene. `sprites` must be for this level's world; `seed`
    /// drives the cosmetic random wobble phases; `uniform_wobble` puts every
    /// unit on phase 0 instead (comparisons).
    pub fn scene(
        &self,
        sprites: &mut SpriteSource,
        atlas: &mut Atlas,
        seed: u64,
        uniform_wobble: bool,
    ) -> TileScene {
        debug_assert_eq!(sprites.world, self.world);
        let (w, h) = self.size();
        let mut scene = TileScene::new(w, h, 24);
        scene.level_bg = self.colour(PalColour::new(0, 4));
        scene.outside_bg = self.colour(PalColour::new(1, 0));
        scene.border = 1;
        let SpriteSource {
            world_bank,
            root_bank,
            ..
        } = sprites;
        let active = self.active_text();
        let mut rng = Rng(seed ^ 0x9E37_79B9_7F4A_7C15);
        // Name per cell for tiling joins (same name joins, §4.3).
        let mut names: Vec<Vec<&str>> = vec![Vec::new(); (w * h) as usize];
        for c in &self.cells {
            names[(c.y * w + c.x) as usize].push(&self.objects[c.def].name);
        }
        for (i, c) in self.cells.iter().enumerate() {
            let def = &self.objects[c.def];
            let joined = if def.tiling == Tiling::Tiled {
                let mut m = 0u8;
                for (bit, dx, dy) in [(1u8, 1i64, 0i64), (2, 0, -1), (4, -1, 0), (8, 0, 1)] {
                    let (nx, ny) = (c.x as i64 + dx, c.y as i64 + dy);
                    let edge = nx <= 0 || ny <= 0 || nx >= w as i64 - 1 || ny >= h as i64 - 1;
                    if edge
                        || names[(ny as u32 * w + nx as u32) as usize].contains(&def.name.as_str())
                    {
                        m |= bit;
                    }
                }
                m
            } else {
                0
            };
            let variant = sprite_variant(def.tiling, c.dir, joined, 0) as u32;
            let bank = if def.sprite_in_root {
                &mut *root_bank
            } else {
                &mut *world_bank
            };
            let frames = bank
                .get(atlas, &def.sprite, variant)
                .or_else(|| bank.get(atlas, &def.sprite, 0))
                .or_else(|| root_bank.get(atlas, "default", 0))
                .unwrap_or_else(|| {
                    log::warn!("no sprite {}_{variant}", def.sprite);
                    world_bank.missing(atlas)
                });
            let colour = match (active[i], def.active_colour) {
                (true, Some(a)) => a,
                _ => def.colour,
            };
            let wobble = if uniform_wobble {
                0
            } else {
                (rng.next() % 3) as u8
            };
            scene.sprites.push(SceneSprite {
                x: c.x as f32,
                y: c.y as f32,
                frames,
                color: self.colour(colour),
                wobble,
                layer: def.layer as i16,
            });
        }
        scene
    }
}

/// Sprite folders of one world (cached atlas regions; reuse across that
/// world's levels).
pub struct SpriteSource {
    world: String,
    world_bank: SpriteBank,
    root_bank: SpriteBank,
}

impl SpriteSource {
    pub fn new(data: &GameData, world: &str) -> Self {
        let root = data.root.join("Sprites");
        let wdir = data.root.join("Worlds").join(world).join("Sprites");
        Self {
            world: world.to_string(),
            world_bank: SpriteBank::new(vec![wdir, root.clone()]),
            root_bank: SpriteBank::new(vec![root]),
        }
    }
}

/// Cosmetic RNG (xorshift64*), never used for rules.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0.max(1);
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every level of every world in the player's install builds a scene
    /// with a sprite for every unit. Skips without `BABA_GAME_DIR`.
    #[test]
    fn every_level_builds_a_scene() {
        let Some(dir) = std::env::var_os("BABA_GAME_DIR") else {
            eprintln!("skipping: no BABA_GAME_DIR");
            return;
        };
        let data = GameData::open(Path::new(&dir)).unwrap();
        let mut count = 0;
        let mut missing = Vec::new();
        for world in std::fs::read_dir(data.root.join("Worlds")).unwrap() {
            let world = world.unwrap().path();
            let wname = world.file_name().unwrap().to_string_lossy().into_owned();
            let mut sprites = SpriteSource::new(&data, &wname);
            let mut atlas = Atlas::new(2048);
            for f in std::fs::read_dir(&world).unwrap() {
                let p = f.unwrap().path();
                if p.extension().is_some_and(|e| e == "l") && p.with_extension("ld").is_file() {
                    let spec = format!("{wname}/{}", p.file_stem().unwrap().to_string_lossy());
                    let level =
                        LoadedLevel::load(&data, &spec).unwrap_or_else(|e| panic!("{spec}: {e}"));
                    let scene = level.scene(&mut sprites, &mut atlas, 1, false);
                    assert_eq!(scene.sprites.len(), level.cells.len());
                    let (w, h) = level.size();
                    assert!(w >= 3 && h >= 3, "{spec}: {w}x{h}");
                    if level.cells.iter().any(|c| {
                        let d = &level.objects[c.def];
                        !data
                            .root
                            .join("Sprites")
                            .join(format!("{}_0_1.png", d.sprite))
                            .is_file()
                            && !world
                                .join("Sprites")
                                .join(format!("{}_0_1.png", d.sprite))
                                .is_file()
                    }) {
                        missing.push(spec.clone());
                    }
                    count += 1;
                }
            }
        }
        eprintln!("{count} levels; levels with a sprite missing frame 0: {missing:?}");
        assert!(count > 100);
    }
}
