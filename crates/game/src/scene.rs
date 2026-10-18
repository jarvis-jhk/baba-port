//! A running `baba_sim::Level` turned into a [`TileScene`]: units with
//! their object sprites (tiling, facing, active text colour), map icons
//! (number / letter / dot / map picture symbols, colour before and after
//! completion), map paths and gates, and the `[images]` backgrounds
//! (`docs/spec/presentation.md` §1–§5, §12).

use std::collections::HashMap;
use std::path::Path;

use baba_formats::level::Direction;
use baba_formats::leveldata::LevelInfo;
use baba_formats::objects::{ObjectDef, ObjectTable, Tiling};
use baba_formats::palette::{PalColour, Palette};
use baba_formats::sprites::sprite_variant;
use baba_render::{Atlas, RegionId, RgbaImage, SceneSprite, SpriteBank, TileScene};
use baba_sim::{Level, TextState};

use crate::art::Art;
use crate::fx::LevelFx;

/// "Force high-contrast colours" (`presentation.md` §5.9, §15.28): every
/// level view draws with its high-contrast palette while set. Set by the
/// app from the settings.
pub static HIGH_CONTRAST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Engine image of the rule-disabled cross (`presentation.md` §7.2).
pub const CROSS_IMAGES: [u32; 3] = [593, 603, 606];

mod icons;

/// A path entry as drawn.
#[derive(Clone, Debug)]
pub struct PathGfx {
    pub x: i32,
    pub y: i32,
    pub dir: u8,
    pub object: String,
    pub gate: i32,
    pub unlock: String,
}

/// Presentation data of one level session.
pub struct LevelGfx {
    pub world: String,
    pub base_world: bool,
    table: ObjectTable,
    pub palette: Palette,
    /// Object definition per sim kind (by slot), `None` for the icon kind.
    kinds: Vec<Option<ObjectDef>>,
    world_bank: SpriteBank,
    root_bank: SpriteBank,
    /// Background images `[images]` (three wobble frames, size).
    images: Vec<([RegionId; 3], u32, u32)>,
    /// Map pictures for style -1 icons (`[icons]` by number).
    icon_pics: HashMap<i32, Option<RegionId>>,
    icon_files: HashMap<i32, (String, bool)>,
    pub paths: Vec<PathGfx>,
    square: Option<(RegionId, RegionId)>,
    dot: Option<RegionId>,
    glyphs: HashMap<char, Option<(RegionId, u32)>>,
    /// Units of the last built scene for the 3D view (§8).
    pub vision_units: Vec<crate::fx::vision::VisionUnit>,
    /// Cutscene changes to how units are drawn (`crate::cutscene`).
    pub overrides: Option<Overrides>,
    /// Unit sprites of the previous scene (the conversion poof draws the
    /// removed unit once more).
    last: HashMap<i32, SceneSprite>,
    /// The poof's two frames of white dashes.
    poof_tex: Option<[RegionId; 2]>,
    /// The other palette (high contrast when `palette` is the normal one,
    /// and the other way round) and whether the contrast one is active.
    other_palette: Option<Palette>,
    contrast_on: bool,
}

/// The conversion poof (`presentation.md` §15.24): white dashed lines
/// across the cell, three in the first frame, two shorter in the second.
fn poof_image(frame: u8) -> RgbaImage {
    let mut img = RgbaImage::new(24, 24);
    let rows: &[u32] = if frame == 0 { &[5, 11, 17] } else { &[8, 15] };
    for &r in rows {
        for y in r..r + 2 {
            for x in 1..23u32 {
                let on = if frame == 0 { x % 7 < 5 } else { x % 9 < 4 };
                if on {
                    let i = ((y * 24 + x) * 4) as usize;
                    img.data[i..i + 4].copy_from_slice(&[255; 4]);
                }
            }
        }
    }
    img
}

/// How a cutscene changes the drawing of a level's units: some are hidden
/// (drawn by the cutscene itself, or not yet revealed), some drawn off
/// their cell or in another colour.
#[derive(Clone, Debug, Default)]
pub struct Overrides {
    pub hide: std::collections::BTreeSet<i32>,
    /// Offset from the cell in source pixels.
    pub offset: std::collections::BTreeMap<i32, (f32, f32)>,
    pub colour: std::collections::BTreeMap<i32, (u8, u8)>,
    /// Units drawn by the cutscene: their sprites go to `detached`.
    pub detach: std::collections::BTreeSet<i32>,
    /// Sprites of the detached units from the last built scene.
    pub detached: Vec<(i32, SceneSprite)>,
}

fn load_frames(atlas: &mut Atlas, dir: &Path, name: &str) -> Option<([RegionId; 3], u32, u32)> {
    let mut imgs: Vec<RgbaImage> = Vec::new();
    for f in 1..=3 {
        if let Ok(i) = RgbaImage::load_png(dir.join(format!("{name}_{f}.png"))) {
            imgs.push(i);
        }
    }
    let first = imgs.first()?.clone();
    if first.width + 2 > atlas.page_size() || first.height + 2 > atlas.page_size() {
        return None;
    }
    let ids: Vec<RegionId> = (0..3)
        .map(|i| atlas.insert(imgs.get(i).unwrap_or(&first)))
        .collect();
    Some(([ids[0], ids[1], ids[2]], first.width, first.height))
}

/// Stable cosmetic wobble phase for a unit id.
pub(crate) fn phase(id: i32) -> u8 {
    let mut x = (id as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x5851_F42D_4C95_7F2D;
    x ^= x >> 29;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 32;
    (x % 3) as u8
}

impl LevelGfx {
    /// `data_dir` = the install's `Data/`.
    pub fn new(
        atlas: &mut Atlas,
        data_dir: &Path,
        world: &str,
        base: &ObjectTable,
        info: &LevelInfo,
        level: &Level,
    ) -> LevelGfx {
        let wdir = data_dir.join("Worlds").join(world);
        let table = base.with_changes(&info.tiles);
        let pal_name = if info.general.palette.is_empty() {
            "default.png"
        } else {
            &info.general.palette
        };
        let pal_dirs = if info.general.palette_root {
            [data_dir.join("Palettes"), wdir.join("Palettes")]
        } else {
            [wdir.join("Palettes"), data_dir.join("Palettes")]
        };
        let palette = pal_dirs
            .iter()
            .find_map(|d| Palette::load(&d.join(pal_name)).ok())
            .or_else(|| Palette::load(&data_dir.join("Palettes/default.png")).ok())
            .unwrap_or(Palette {
                width: 7,
                height: 5,
                colours: vec![[200, 200, 200]; 35],
            });
        // The high-contrast palette: the level's `contrastpalette`, else
        // `contrast.png` (measured: the default palette is swapped for it).
        let contrast_name = info
            .ini
            .get("general", "contrastpalette")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "contrast.png".to_string());
        let other_palette = pal_dirs
            .iter()
            .find_map(|d| Palette::load(&d.join(&contrast_name)).ok())
            .or_else(|| Palette::load(&data_dir.join("Palettes/contrast.png")).ok());
        let kinds = level
            .defs()
            .kinds
            .iter()
            .map(|k| table.get(&k.slot).cloned())
            .collect();
        let images = info
            .images
            .iter()
            .filter_map(|n| load_frames(atlas, &wdir.join("Images"), n))
            .collect();
        let icon_files = info
            .icons
            .iter()
            .map(|i| (i.index as i32, (i.file.clone(), i.root)))
            .collect();
        let paths = info
            .paths
            .iter()
            .map(|p| PathGfx {
                x: p.x,
                y: p.y,
                dir: p.dir.clamp(0, 3) as u8,
                object: p.object.clone(),
                gate: p.gate,
                unlock: p.unlock.clone(),
            })
            .collect();
        let root = data_dir.join("Sprites");
        LevelGfx {
            world: world.to_string(),
            base_world: world == "baba",
            table,
            palette,
            kinds,
            world_bank: SpriteBank::new(vec![wdir.join("Sprites"), root.clone()]),
            root_bank: SpriteBank::new(vec![root, wdir.join("Sprites")]),
            images,
            icon_pics: HashMap::new(),
            icon_files,
            paths,
            square: None,
            dot: None,
            glyphs: HashMap::new(),
            vision_units: Vec::new(),
            overrides: None,
            last: HashMap::new(),
            poof_tex: None,
            other_palette,
            contrast_on: false,
        }
    }

    /// Swaps in the high-contrast palette (or back) when the setting
    /// changed.
    fn sync_contrast(&mut self) {
        let want = HIGH_CONTRAST.load(std::sync::atomic::Ordering::Relaxed);
        if want != self.contrast_on
            && let Some(other) = self.other_palette.take()
        {
            self.other_palette = Some(std::mem::replace(&mut self.palette, other));
            self.contrast_on = want;
        }
    }

    fn colour(&self, c: PalColour) -> [u8; 4] {
        let [r, g, b] = self.palette.rgb(c);
        [r, g, b, 255]
    }

    pub fn pal(&self, x: u8, y: u8) -> [u8; 4] {
        self.colour(PalColour::new(x, y))
    }

    fn frames(&mut self, atlas: &mut Atlas, def: &ObjectDef, variant: u32) -> [RegionId; 3] {
        let bank = if def.sprite_in_root {
            &mut self.root_bank
        } else {
            &mut self.world_bank
        };
        bank.get(atlas, &def.sprite, variant)
            .or_else(|| bank.get(atlas, &def.sprite, 0))
            .unwrap_or_else(|| bank.missing(atlas))
    }

    fn icon_pic(&mut self, atlas: &mut Atlas, number: i32, data_dir: &Path) -> Option<RegionId> {
        if let Some(r) = self.icon_pics.get(&number) {
            return *r;
        }
        let r = self.icon_files.get(&number).and_then(|(file, root)| {
            let wdir = data_dir.join("Worlds").join(&self.world).join("Sprites");
            let rdir = data_dir.join("Sprites");
            let order = if *root { [rdir, wdir] } else { [wdir, rdir] };
            order
                .iter()
                .find_map(|d| RgbaImage::load_png(d.join(format!("{file}.png"))).ok())
                .map(|img| atlas.insert(&img))
        });
        self.icon_pics.insert(number, r);
        r
    }

    /// Builds the scene. `path_states`: state per `[paths]` index (0 =
    /// not drawn; closed gates < 3).
    #[allow(clippy::too_many_arguments)]
    pub fn scene(
        &mut self,
        atlas: &mut Atlas,
        data_dir: &Path,
        level: &Level,
        path_states: &[i32],
        fx: Option<&mut LevelFx>,
        art: &mut Art,
        wobble: u32,
    ) -> TileScene {
        self.sync_contrast();
        let (w, h) = (level.width() as u32, level.height() as u32);
        let mut scene = TileScene::new(w, h, 24);
        scene.level_bg = self.pal(0, 4);
        scene.outside_bg = self.pal(1, 0);
        scene.border = 1;
        for (frames, iw, ih) in &self.images {
            let dx = (24 - *iw as i32).div_euclid(2);
            let dy = (24 - *ih as i32).div_euclid(2);
            scene.sprites.push(SceneSprite {
                x: (24 - dx) as f32 / 24.0,
                y: (24 - dy) as f32 / 24.0,
                frames: *frames,
                color: [255; 4],
                wobble: 0,
                layer: -100,
            });
        }
        let defs = level.defs();
        let units = level.unit_records();
        // Drawn paths (the path object, or the open-path object for opened
        // gates, progression.md §4.3).
        let mut drawn_paths: Vec<(usize, ObjectDef)> = Vec::new();
        for (i, p) in self.paths.iter().enumerate() {
            let st = path_states.get(i).copied().unwrap_or(0);
            if st <= 0 {
                continue;
            }
            let slot = if p.gate != 0 && st >= 3 {
                if !p.unlock.is_empty() {
                    p.unlock.clone()
                } else {
                    self.table
                        .by_name("line")
                        .map(|d| d.id.clone())
                        .unwrap_or_default()
                }
            } else {
                p.object.clone()
            };
            if let Some(d) = self.table.get(&slot).cloned() {
                drawn_paths.push((i, d));
            }
        }
        // Names per cell for tiling joins (§4.3): units, paths, icons.
        let mut names: HashMap<(i32, i32), Vec<String>> = HashMap::new();
        let mut icon_cells: Vec<(i32, i32)> = Vec::new();
        for u in &units {
            if u.dead {
                continue;
            }
            if u.kind == defs.level_kind {
                let st = u.completion();
                if self.base_world || st >= 1 {
                    icon_cells.push((u.x, u.y));
                }
                continue;
            }
            names
                .entry((u.x, u.y))
                .or_default()
                .push(defs.kind(u.kind).name.clone());
        }
        for (i, d) in &drawn_paths {
            let p = &self.paths[*i];
            names.entry((p.x, p.y)).or_default().push(d.name.clone());
        }
        let joined = |name: &str, x: i32, y: i32| -> u8 {
            let mut m = 0u8;
            for (bit, dx, dy) in [(1u8, 1, 0), (2, 0, -1), (4, -1, 0), (8, 0, 1)] {
                let (nx, ny) = (x + dx, y + dy);
                let edge = nx <= 0 || ny <= 0 || nx >= w as i32 - 1 || ny >= h as i32 - 1;
                if edge
                    || names
                        .get(&(nx, ny))
                        .is_some_and(|v| v.iter().any(|n| n == name))
                    || icon_cells.contains(&(nx, ny))
                {
                    m |= bit;
                }
            }
            m
        };
        for (i, d) in &drawn_paths {
            let p = self.paths[*i].clone();
            let j = if d.tiling == Tiling::Tiled {
                joined(&d.name, p.x, p.y)
            } else {
                0
            };
            let variant = sprite_variant(d.tiling, Direction::from_u8(p.dir), j, 0) as u32;
            let frames = self.frames(atlas, d, variant);
            scene.sprites.push(SceneSprite {
                x: p.x as f32,
                y: p.y as f32,
                frames,
                color: self.colour(d.colour),
                wobble: phase(10_000 + *i as i32),
                layer: d.layer as i16,
            });
        }
        let views = level.units();
        self.vision_units.clear();
        let cam_target = level.camera().and_then(|c| c.target);
        let mut ov = self.overrides.take();
        if let Some(o) = &mut ov {
            o.detached.clear();
        }
        let mut drawn: HashMap<i32, SceneSprite> = HashMap::new();
        for (u, v) in units.into_iter().zip(views.iter()) {
            if u.dead || !u.visible || ov.as_ref().is_some_and(|o| o.hide.contains(&u.id)) {
                continue;
            }
            if u.kind == defs.level_kind {
                self.push_icon(atlas, data_dir, &mut scene, u, art);
                continue;
            }
            let Some(def) = self.kinds.get(u.kind as usize).cloned().flatten() else {
                continue;
            };
            let j = if def.tiling == Tiling::Tiled {
                joined(&def.name, u.x, u.y)
            } else {
                0
            };
            let (steps, turns, asleep) = fx.as_deref().map_or((0, 0, false), |f| f.anim(u.id));
            let dir = Direction::from_u8(u.dir);
            let variant = match def.tiling {
                Tiling::Character if asleep => (u.dir as u32 * 8 + 31) % 32,
                Tiling::Character => sprite_variant(def.tiling, dir, j, steps) as u32,
                _ => sprite_variant(def.tiling, dir, j, turns) as u32,
            };
            let frames = self.frames(atlas, &def, variant);
            // Colour properties blink through their list (§5.6, §5.8).
            let colour = match (&fx, v.colours.len()) {
                (Some(f), n) if n > 1 => v.colours[f.blink(n)],
                _ => v.colour,
            };
            let colour = ov
                .as_ref()
                .and_then(|o| o.colour.get(&u.id).copied())
                .unwrap_or(colour);
            let (mut ox, mut oy) = fx
                .as_deref()
                .map_or((0.0, 0.0), |f| f.unit_offset(u.id, u.float));
            if let Some((dx, dy)) = ov.as_ref().and_then(|o| o.offset.get(&u.id)) {
                ox += dx;
                oy += dy;
            }
            let (x, y) = (u.x as f32 + ox / 24.0, u.y as f32 + oy / 24.0);
            let sprite = SceneSprite {
                x,
                y,
                frames,
                color: self.colour(PalColour::new(colour.0, colour.1)),
                wobble: phase(u.id),
                layer: def.layer as i16,
            };
            if let Some(o) = ov.as_mut().filter(|o| o.detach.contains(&u.id)) {
                o.detached.push((u.id, sprite));
                continue;
            }
            drawn.insert(u.id, sprite);
            scene.sprites.push(sprite);
            if cam_target.is_some() || level.vision_on() {
                self.vision_units.push(crate::fx::vision::VisionUnit {
                    x: u.x,
                    y: u.y,
                    layer: def.layer as i16,
                    sprite,
                    target: cam_target == Some(u.id),
                    edge: false,
                });
            }
            // Rule-disabled cross over the text (§7.2).
            if v.text == TextState::Disabled
                && !level.vision_on()
                && let (Some(a), Some(b), Some(c)) = (
                    art.get(atlas, CROSS_IMAGES[0]),
                    art.get(atlas, CROSS_IMAGES[1]),
                    art.get(atlas, CROSS_IMAGES[2]),
                )
            {
                let dx = (24 - a.w as i32).div_euclid(2) as f32;
                let dy = (24 - a.h as i32).div_euclid(2) as f32;
                scene.sprites.push(SceneSprite {
                    x: x + (12.0 - a.hx as f32 - dx) / 24.0,
                    y: y + (12.0 - a.hy as f32 - dy) / 24.0,
                    frames: [a.region, b.region, c.region],
                    color: self.pal(2, 2),
                    wobble: phase(u.id),
                    layer: 21,
                });
            }
        }
        self.overrides = ov;
        let prev = std::mem::replace(&mut self.last, drawn);
        if let Some(f) = fx.as_deref()
            && f.poofs().next().is_some()
        {
            let tex = *self.poof_tex.get_or_insert_with(|| {
                [atlas.insert(&poof_image(0)), atlas.insert(&poof_image(1))]
            });
            let white = self.pal(0, 3);
            for (id, (x, y), age) in f.poofs() {
                if age == 0
                    && let Some(sp) = prev.get(&id)
                {
                    scene.sprites.push(SceneSprite { layer: 24, ..*sp });
                }
                scene.sprites.push(SceneSprite {
                    x: x as f32,
                    y: y as f32,
                    frames: [tex[age.min(1) as usize]; 3],
                    color: white,
                    wobble: 0,
                    layer: 25,
                });
            }
        }
        if let Some(f) = fx {
            let pal = &self.palette;
            let colour = |c: (u8, u8)| {
                let [r, g, b] = pal.rgb(PalColour::new(c.0, c.1));
                [r, g, b, 255]
            };
            f.sprites(
                atlas,
                art,
                wobble,
                &colour,
                &mut scene.sprites,
                &mut scene.scaled,
            );
            // Background: LEVEL IS <colour> (§5.7) with the fade.
            let cols = level.level_colours();
            let target = match cols.len() {
                0 => self.pal(0, 4),
                n => {
                    let c = cols[f.blink(n)];
                    let rgb = self.pal(c.0, c.1);
                    if c == (0, 4) {
                        rgb
                    } else {
                        [rgb[0] / 4, rgb[1] / 4, rgb[2] / 4, 255]
                    }
                }
            };
            scene.level_bg = f.background(target);
        }
        scene
    }
}
