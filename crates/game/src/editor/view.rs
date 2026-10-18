//! Drawing the editor (`docs/spec/editor.md` §4–§6, §14): the level as a
//! tile scene straight from the edited grid (every object visible, active
//! text lit as the rules would light it), and the overlays: the top and
//! bottom bars with layers, tools and the quickbar, the cursor with the
//! picked object, tool previews, the floating selection, direction
//! arrows, the palette and object-list screens, the sprite and colour
//! pickers, the tutorial box. Layout is our own (§18.1) on the 854×480
//! logical canvas; every clickable element is a [`Btn`] so drawing, mouse,
//! touch and gamepad share one layout.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use baba_formats::level::Direction;
use baba_formats::objects::{ObjectDef, Tiling};
use baba_formats::sprites::sprite_variant;
use baba_formats::{ObjectTable, PalColour, Palette};
use baba_render::{Atlas, Fit, RegionId, RgbaImage, SceneSprite, SpriteBank, TileScene};

use super::model::{Catalogue, EdLevel, QUICK};
use super::tools::Tool;
use super::{Brush, ColourFor, Editor, Pick};
use crate::lang::Texts;
use crate::ui::{Align, Gfx, Ui};

/// Top bar height (logical units; the level is drawn full screen).
pub const TOP: f32 = 0.0;
/// Bottom bar height.
pub const BOTTOM: f32 = 0.0;

/// What a button does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BtnAct {
    // Main view.
    Layer(usize),
    Tool(Tool),
    Quick(usize),
    Menu,
    Settings,
    Objects,
    Save,
    Undo,
    Swap,
    Test,
    // Palette screen.
    Search,
    ClearSearch,
    Tags,
    Add,
    Remove,
    Edit,
    Pairs,
    SwapView,
    Return,
    Dir(u8),
    Brush(Brush),
    PathCog,
    Cell(i32, i32),
    // Object list, sprite picker.
    Item(usize),
    Page(i32),
    // Colour picker.
    Colour(u8, u8),
    // Tutorial.
    TutContinue,
    TutSkip,
}

/// A clickable element (logical rectangle).
#[derive(Clone, Debug)]
pub struct Btn {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub act: BtnAct,
    pub label: String,
    pub on: bool,
    pub enabled: bool,
}

impl Btn {
    fn new(x: f32, y: f32, w: f32, h: f32, act: BtnAct, label: impl Into<String>) -> Btn {
        Btn {
            x,
            y,
            w,
            h,
            act,
            label: label.into(),
            on: false,
            enabled: true,
        }
    }

    fn on(mut self, on: bool) -> Btn {
        self.on = on;
        self
    }

    fn enabled(mut self, e: bool) -> Btn {
        self.enabled = e;
        self
    }

    pub fn hit(&self, lx: f32, ly: f32) -> bool {
        lx >= self.x && lx < self.x + self.w && ly >= self.y && ly < self.y + self.h
    }
}

/// Presentation cache of the edited level: palette, sprite folders, the
/// active-text set (recomputed when the level changes).
#[derive(Default)]
pub struct EdGfx {
    key: Option<(PathBuf, String, bool)>,
    pub palette: Option<Palette>,
    world_bank: Option<SpriteBank>,
    root_bank: Option<SpriteBank>,
    square: Option<(RegionId, RegionId)>,
    hash: u64,
    /// Lit text: (x, y, slot).
    active: HashSet<(i32, i32, String)>,
    /// Background images `[images]`.
    images: Vec<([RegionId; 3], u32, u32)>,
    images_key: Vec<String>,
    /// Plain images (palette swatch previews etc.).
    pics: HashMap<PathBuf, Option<RegionId>>,
}

fn fnv(mut h: u64, b: &[u8]) -> u64 {
    for x in b {
        h ^= *x as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

impl EdGfx {
    /// Reloads the palette and sprite folders when the level's world or
    /// palette changed.
    pub fn sync(&mut self, l: &EdLevel, data_dir: &Path) {
        let g = &l.info.general;
        let pal = if g.palette.is_empty() {
            "default.png".to_string()
        } else {
            g.palette.clone()
        };
        let key = (l.place.dir.clone(), pal.clone(), g.palette_root);
        if self.key.as_ref() == Some(&key) {
            return;
        }
        let wdir = l.place.dir.clone();
        let dirs = if g.palette_root {
            [data_dir.join("Palettes"), wdir.join("Palettes")]
        } else {
            [wdir.join("Palettes"), data_dir.join("Palettes")]
        };
        self.palette = dirs
            .iter()
            .find_map(|d| Palette::load(&d.join(&pal)).ok())
            .or_else(|| Palette::load(&data_dir.join("Palettes/default.png")).ok());
        let root = data_dir.join("Sprites");
        let world_changed = self.key.as_ref().map(|k| &k.0) != Some(&key.0);
        if world_changed || self.world_bank.is_none() {
            self.world_bank = Some(SpriteBank::new(vec![wdir.join("Sprites"), root.clone()]));
            self.root_bank = Some(SpriteBank::new(vec![root, wdir.join("Sprites")]));
            self.images_key.clear();
        }
        self.key = Some(key);
    }

    pub fn colour(&self, c: PalColour) -> [u8; 4] {
        match &self.palette {
            Some(p) => {
                let [r, g, b] = p.rgb(c);
                [r, g, b, 255]
            }
            None => [200, 200, 200, 255],
        }
    }

    pub fn frames(&mut self, atlas: &mut Atlas, def: &ObjectDef, variant: u32) -> [RegionId; 3] {
        let bank = if def.sprite_in_root {
            self.root_bank.as_mut()
        } else {
            self.world_bank.as_mut()
        };
        let Some(bank) = bank else {
            return [RegionId(0); 3];
        };
        bank.get(atlas, &def.sprite, variant)
            .or_else(|| bank.get(atlas, &def.sprite, 0))
            .unwrap_or_else(|| bank.missing(atlas))
    }

    /// A sprite by name from root or the world.
    pub fn sprite(&mut self, atlas: &mut Atlas, name: &str, root: bool) -> Option<RegionId> {
        let bank = if root {
            self.root_bank.as_mut()
        } else {
            self.world_bank.as_mut()
        }?;
        bank.get(atlas, name, 0).map(|f| f[0])
    }

    /// Thumbnail of a definition facing `dir`.
    pub fn thumb(&mut self, atlas: &mut Atlas, def: &ObjectDef, dir: u8) -> RegionId {
        let v = sprite_variant(def.tiling, Direction::from_u8(dir), 15, 0) as u32;
        let v = if def.tiling == Tiling::Tiled { 0 } else { v };
        self.frames(atlas, def, v)[0]
    }

    /// The level's active text, recomputed when the level changed.
    pub fn update_actives(&mut self, l: &mut EdLevel, base: &ObjectTable) {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for layer in &l.map.layers {
            for t in &layer.tiles {
                h = fnv(h, &t.0.to_le_bytes());
            }
        }
        h = fnv(h, format!("{:?}", l.info.tiles).as_bytes());
        h = fnv(h, &(l.width() as u64).to_le_bytes());
        if h == self.hash {
            return;
        }
        self.hash = h;
        self.active.clear();
        let lvl = l.as_level();
        let src = baba_sim::Source::from_level(base, &lvl, &l.place.world);
        let sim = baba_sim::Level::start(Arc::new(src), baba_sim::Context::default());
        let defs = sim.defs();
        for u in sim.unit_records() {
            if u.active && !u.dead {
                self.active
                    .insert((u.x, u.y, defs.kind(u.kind).slot.clone()));
            }
        }
    }

    fn squares(&mut self, atlas: &mut Atlas) -> (RegionId, RegionId) {
        if let Some(s) = self.square {
            return s;
        }
        let n = 20u32;
        let mut fill = RgbaImage::new(n, n);
        let mut border = RgbaImage::new(n, n);
        for y in 0..n {
            for x in 0..n {
                let dx = x.min(n - 1 - x);
                let dy = y.min(n - 1 - y);
                if dx + dy < 3 {
                    continue;
                }
                if dx < 2 || dy < 2 || dx + dy < 5 {
                    border.put(x, y, [255; 4]);
                } else {
                    fill.put(x, y, [255; 4]);
                }
            }
        }
        let s = (atlas.insert(&fill), atlas.insert(&border));
        self.square = Some(s);
        s
    }

    fn sync_images(&mut self, atlas: &mut Atlas, l: &EdLevel) {
        if self.images_key == l.info.images {
            return;
        }
        self.images_key = l.info.images.clone();
        self.images.clear();
        let dir = l.place.dir.join("Images");
        for name in &l.info.images {
            let mut imgs = Vec::new();
            for f in 1..=3 {
                if let Ok(i) = RgbaImage::load_png(dir.join(format!("{name}_{f}.png"))) {
                    imgs.push(i);
                }
            }
            let Some(first) = imgs.first().cloned() else {
                continue;
            };
            if first.width + 2 > atlas.page_size() || first.height + 2 > atlas.page_size() {
                continue;
            }
            let ids: Vec<RegionId> = (0..3)
                .map(|i| atlas.insert(imgs.get(i).unwrap_or(&first)))
                .collect();
            self.images
                .push(([ids[0], ids[1], ids[2]], first.width, first.height));
        }
    }

    /// The level as a scene (every object visible, in its stored facing).
    pub fn scene(&mut self, atlas: &mut Atlas, l: &EdLevel) -> TileScene {
        let (w, h) = (l.width(), l.height());
        let mut scene = TileScene::new(w, h, 24);
        scene.level_bg = self.colour(PalColour::new(0, 4));
        scene.outside_bg = self.colour(PalColour::new(1, 0));
        scene.border = 1;
        self.sync_images(atlas, l);
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
        let objs = l.objects();
        let mut names: HashMap<(i32, i32), Vec<String>> = HashMap::new();
        for (_, x, y, o) in &objs {
            if let Some(d) = l.table.by_tile(o.tile) {
                names.entry((*x, *y)).or_default().push(d.name.clone());
            }
        }
        for p in &l.info.paths {
            if let Some(d) = l.table.get(&p.object) {
                names.entry((p.x, p.y)).or_default().push(d.name.clone());
            }
        }
        let icons: HashSet<(i32, i32)> = l.info.levels.iter().map(|i| (i.x, i.y)).collect();
        let joined = |name: &str, x: i32, y: i32| -> u8 {
            let mut m = 0u8;
            for (bit, dx, dy) in [(1u8, 1, 0), (2, 0, -1), (4, -1, 0), (8, 0, 1)] {
                let (nx, ny) = (x + dx, y + dy);
                let edge = nx <= 0 || ny <= 0 || nx >= w as i32 - 1 || ny >= h as i32 - 1;
                if edge
                    || names
                        .get(&(nx, ny))
                        .is_some_and(|v| v.iter().any(|n| n == name))
                    || icons.contains(&(nx, ny))
                {
                    m |= bit;
                }
            }
            m
        };
        let mut n = 0u64;
        for (_, x, y, o) in &objs {
            let Some(def) = l.table.by_tile(o.tile).cloned() else {
                continue;
            };
            let j = if def.tiling == Tiling::Tiled {
                joined(&def.name, *x, *y)
            } else {
                0
            };
            let variant = sprite_variant(def.tiling, Direction::from_u8(o.dir), j, 0) as u32;
            let frames = self.frames(atlas, &def, variant);
            let lit = self.active.contains(&(*x, *y, def.id.clone()));
            let colour = match (lit, def.active_colour) {
                (true, Some(a)) => a,
                _ => def.colour,
            };
            n += 1;
            scene.sprites.push(SceneSprite {
                x: *x as f32,
                y: *y as f32,
                frames,
                color: self.colour(colour),
                wobble: ((*x * 7 + *y * 13) % 3) as u8,
                layer: def.layer as i16,
            });
        }
        let _ = n;
        for p in &l.info.paths {
            let Some(def) = l.table.get(&p.object).cloned() else {
                continue;
            };
            let j = if def.tiling == Tiling::Tiled {
                joined(&def.name, p.x, p.y)
            } else {
                0
            };
            let variant = sprite_variant(
                def.tiling,
                Direction::from_u8(p.dir.clamp(0, 3) as u8),
                j,
                0,
            ) as u32;
            let frames = self.frames(atlas, &def, variant);
            let mut c = self.colour(def.colour);
            if p.style == 0 {
                c[3] = 140;
            }
            scene.sprites.push(SceneSprite {
                x: p.x as f32,
                y: p.y as f32,
                frames,
                color: c,
                wobble: 0,
                layer: def.layer as i16,
            });
        }
        let (fill, border) = self.squares(atlas);
        for icon in &l.info.levels {
            let c = self.colour(icon.colour.unwrap_or(PalColour::new(1, 2)));
            let shade = |k: f32| {
                [
                    (c[0] as f32 * k) as u8,
                    (c[1] as f32 * k) as u8,
                    (c[2] as f32 * k) as u8,
                    if icon.state == 0 { 150 } else { 255 },
                ]
            };
            let mut bc = c;
            if icon.state == 0 {
                bc[3] = 150;
            }
            for (r, col, layer) in [(fill, shade(0.3), 14), (border, bc, 15)] {
                scene.sprites.push(SceneSprite {
                    x: icon.x as f32,
                    y: icon.y as f32,
                    frames: [r; 3],
                    color: col,
                    wobble: 0,
                    layer,
                });
            }
        }
        scene
    }

    /// A plain image file as an atlas region (cached).
    pub fn pic(&mut self, atlas: &mut Atlas, path: &Path) -> Option<RegionId> {
        if let Some(r) = self.pics.get(path) {
            return *r;
        }
        let r = RgbaImage::load_png(path)
            .ok()
            .filter(|i| i.width + 2 < atlas.page_size() && i.height + 2 < atlas.page_size())
            .map(|i| atlas.insert(&i));
        self.pics.insert(path.to_path_buf(), r);
        r
    }
}

/// The preview thumbnail (`§10.3`): the level interior scaled to fit
/// 216×112, centred on the outside colour.
pub fn preview(scene: &TileScene, atlas: &Atlas) -> RgbaImage {
    let full = baba_render::cpu::render_scene(scene, atlas, 0, 1);
    let t = scene.tile;
    let (ix, iy) = (t, t);
    let (iw, ih) = (
        full.width.saturating_sub(2 * t).max(1),
        full.height.saturating_sub(2 * t).max(1),
    );
    let (tw, th) = (216u32, 112u32);
    let s = (tw as f32 / iw as f32).min(th as f32 / ih as f32);
    let (dw, dh) = (
        ((iw as f32 * s) as u32).max(1),
        ((ih as f32 * s) as u32).max(1),
    );
    let (ox, oy) = ((tw - dw.min(tw)) / 2, (th - dh.min(th)) / 2);
    let bg = scene.outside_bg;
    let mut out = RgbaImage::new(tw, th);
    for y in 0..th {
        for x in 0..tw {
            out.put(x, y, [bg[0], bg[1], bg[2], 255]);
        }
    }
    for y in 0..dh.min(th) {
        for x in 0..dw.min(tw) {
            // Box filter over the source pixels of this target pixel.
            let (sx0, sx1) = (
                ix + (x as f32 / s) as u32,
                ix + (((x + 1) as f32 / s) as u32).max((x as f32 / s) as u32 + 1),
            );
            let (sy0, sy1) = (
                iy + (y as f32 / s) as u32,
                iy + (((y + 1) as f32 / s) as u32).max((y as f32 / s) as u32 + 1),
            );
            let mut acc = [0u32; 3];
            let mut n = 0;
            for sy in sy0..sy1.min(iy + ih) {
                for sx in sx0..sx1.min(ix + iw) {
                    let p = full.get(sx, sy);
                    acc[0] += p[0] as u32;
                    acc[1] += p[1] as u32;
                    acc[2] += p[2] as u32;
                    n += 1;
                }
            }
            let n = n.max(1);
            out.put(
                ox + x,
                oy + y,
                [
                    (acc[0] / n) as u8,
                    (acc[1] / n) as u8,
                    (acc[2] / n) as u8,
                    255,
                ],
            );
        }
    }
    out
}

// ------------------------------------------------------------- layouts

/// Buttons of the main view.
pub fn main_buttons(ed: &Editor, t: &Texts, extended: bool) -> Vec<Btn> {
    // The original's arrangement (captured): quickbar top right, layers
    // and tools down the right edge, the action buttons along the bottom,
    // the level full screen underneath.
    let mut v = Vec::new();
    for i in 0..QUICK {
        v.push(
            Btn::new(
                551.0 + i as f32 * 30.0,
                3.0,
                28.0,
                28.0,
                BtnAct::Quick(i),
                "",
            )
            .on(ed.qsel == i),
        );
    }
    for i in 0..3 {
        v.push(
            Btn::new(
                820.0,
                38.0 + i as f32 * 23.0,
                30.0,
                21.0,
                BtnAct::Layer(i),
                format!("L{}", i + 1),
            )
            .on(ed.layer == i),
        );
    }
    for (i, tool) in Tool::ALL.into_iter().enumerate() {
        v.push(
            Btn::new(
                819.0,
                114.0 + i as f32 * 36.0,
                32.0,
                32.0,
                BtnAct::Tool(tool),
                "",
            )
            .on(ed.tool == tool),
        );
    }
    let can_undo = ed.undo.can_undo();
    let items: [(BtnAct, String, bool, f32, f32); 7] = [
        (BtnAct::Test, t.get("editor_test"), true, 276.0, 56.0),
        (BtnAct::Swap, t.get("editor_swap"), true, 336.0, 56.0),
        (BtnAct::Undo, t.get("editor_undo"), can_undo, 396.0, 56.0),
        (BtnAct::Save, t.get("editor_savelevel"), true, 456.0, 56.0),
        (
            BtnAct::Objects,
            t.get("editor_objectlist"),
            true,
            540.0,
            86.0,
        ),
        (
            BtnAct::Settings,
            t.get("editor_settingsmenu"),
            true,
            630.0,
            96.0,
        ),
        (BtnAct::Menu, t.get("editor_mainmenu"), true, 754.0, 94.0),
    ];
    for (act, label, en, bx, bw) in items {
        v.push(Btn::new(bx, 457.0, bw, 21.0, act, label).enabled(en));
    }
    let _ = extended;
    v
}

/// Draws a tool's icon (our own simple shapes) in a `s`-sized box.
pub fn draw_tool_icon(ui: &mut Ui, g: &Gfx, tool: Tool, x: f32, y: f32, s: f32, c: [u8; 4]) {
    let u = s / 8.0;
    let px = |ui: &mut Ui, i: f32, j: f32, w: f32, h: f32| {
        ui.rect(g, x + i * u, y + j * u, w * u, h * u, c)
    };
    match tool {
        Tool::Pen => {
            for k in 0..4 {
                px(ui, 2.0 + k as f32, 5.0 - k as f32, 1.0, 1.0);
            }
            px(ui, 1.5, 5.5, 1.0, 1.0);
        }
        Tool::Line => {
            for k in 0..6 {
                px(ui, 1.0 + k as f32, 6.0 - k as f32, 1.0, 1.0);
            }
        }
        Tool::Rect => {
            px(ui, 1.5, 1.5, 5.0, 0.7);
            px(ui, 1.5, 5.8, 5.0, 0.7);
            px(ui, 1.5, 1.5, 0.7, 5.0);
            px(ui, 5.8, 1.5, 0.7, 5.0);
        }
        Tool::FillRect => px(ui, 1.5, 1.5, 5.0, 5.0),
        Tool::Select => {
            for k in 0..3 {
                let o = 1.5 + k as f32 * 1.8;
                px(ui, o, 1.5, 0.9, 0.6);
                px(ui, o, 5.9, 0.9, 0.6);
                px(ui, 1.5, o, 0.6, 0.9);
                px(ui, 5.9, o, 0.6, 0.9);
            }
        }
        Tool::Fill => {
            px(ui, 2.0, 3.0, 3.5, 3.5);
            px(ui, 5.5, 4.0, 1.0, 2.5);
            px(ui, 3.0, 1.5, 1.0, 1.5);
        }
        Tool::Eraser => {
            for k in 0..3 {
                px(ui, 2.0 + k as f32, 4.0 - k as f32, 2.5, 2.5);
            }
            px(ui, 1.0, 6.5, 6.0, 0.6);
        }
    }
}

/// Cell size and origin of the palette grid.
pub fn palette_grid(ed: &Editor) -> (f32, f32, f32, i32, i32) {
    // A fixed 15 × 10 grid like the original's palette.
    let _ = ed;
    (36.0, 38.0, 100.0, 15, 10)
}

/// Buttons of the palette screen (the original's arrangement: tools,
/// search, add / remove / edit, the pairs toggle and close along the
/// top; the direction cross on the right; our brush column under it).
pub fn palette_buttons(ed: &Editor, t: &Texts, extended: bool) -> Vec<Btn> {
    let mut v = Vec::new();
    let (y, h) = (26.0, 44.0);
    let mut x = 14.0;
    for tool in Tool::ALL {
        v.push(Btn::new(x, y, 44.0, h, BtnAct::Tool(tool), "").on(ed.tool == tool));
        x += 48.0;
    }
    x += 12.0;
    v.push(Btn::new(x, y, 44.0, h, BtnAct::Search, "?"));
    x += 48.0;
    v.push(Btn::new(x, y, 44.0, h, BtnAct::ClearSearch, "x?").enabled(!ed.pal.search.is_empty()));
    x += 48.0;
    v.push(Btn::new(x, y, 44.0, h, BtnAct::Tags, "#"));
    x += 60.0;
    let full = ed
        .level
        .as_ref()
        .is_some_and(|l| l.palette.len() >= super::model::MAX_PALETTE);
    v.push(Btn::new(x, y, 44.0, h, BtnAct::Add, "+").enabled(!full));
    x += 48.0;
    v.push(Btn::new(x, y, 44.0, h, BtnAct::Remove, "-"));
    x += 48.0;
    v.push(Btn::new(x, y, 44.0, h, BtnAct::Edit, "abc"));
    x += 60.0;
    v.push(
        Btn::new(
            x,
            y,
            44.0,
            h,
            BtnAct::Pairs,
            if ed.pal.full { "1|1" } else { "1+1" },
        )
        .on(!ed.pal.full),
    );
    v.push(Btn::new(782.0, y, 44.0, h, BtnAct::Return, "X"));
    // The direction cross.
    for (d, bx, by) in [
        (1u8, 706.0, 94.0),
        (2, 646.0, 154.0),
        (0, 766.0, 154.0),
        (3, 706.0, 214.0),
    ] {
        v.push(Btn::new(bx, by, 56.0, 56.0, BtnAct::Dir(d), arrow(d)).on(ed.dir == d));
    }
    if extended {
        let brushes = [
            (Brush::Objects, t.get("editor_brush_normal")),
            (Brush::Levels, t.get("editor_brush_level")),
            (Brush::Paths, t.get("editor_brush_path")),
            (Brush::Special, t.get("editor_brush_special")),
            (Brush::Cursor, t.get("editor_setcursor")),
        ];
        for (i, (b, label)) in brushes.into_iter().enumerate() {
            v.push(
                Btn::new(
                    646.0,
                    286.0 + i as f32 * 26.0,
                    176.0,
                    22.0,
                    BtnAct::Brush(b),
                    label,
                )
                .on(ed.brush == b),
            );
        }
        v.push(Btn::new(
            646.0,
            286.0 + 5.0 * 26.0,
            176.0,
            22.0,
            BtnAct::PathCog,
            t.get("editor_setpath"),
        ));
    }
    let (cell, x0, y0, gw, gh) = palette_grid(ed);
    for gy in 0..gh {
        for gx in 0..gw {
            v.push(Btn::new(
                x0 + gx as f32 * cell,
                y0 + gy as f32 * cell,
                cell,
                cell,
                BtnAct::Cell(gx, gy),
                "",
            ));
        }
    }
    v
}

/// Object list entries (catalogue order, filtered, `§5.5`).
pub fn list_items<'a>(
    cat: &'a Catalogue,
    search: &str,
    tags: &[String],
    extended: bool,
    advanced: bool,
) -> Vec<&'a baba_formats::objects::EditorObject> {
    let s = search.to_lowercase();
    cat.ordered()
        .into_iter()
        .filter(|o| Catalogue::visible(o, extended, advanced))
        .filter(|o| s.is_empty() || o.name.strip_prefix("text_").unwrap_or(&o.name).contains(&s))
        .filter(|o| tags.iter().all(|t| o.tags.contains(t)))
        .collect()
}

pub const LIST_COLS: usize = 15;
pub const LIST_ROWS: usize = 4;
pub const LIST_PAGE: usize = LIST_COLS * LIST_ROWS;

/// Buttons of the object list (and, with `sprites`, the sprite picker).
pub fn list_buttons(count: usize, page: usize, t: &Texts, sprites: bool) -> Vec<Btn> {
    let mut v = Vec::new();
    let (cols, rows) = if sprites {
        (10, 8)
    } else {
        (LIST_COLS, LIST_ROWS)
    };
    let per = cols * rows;
    let cell = if sprites { 40.0 } else { 48.0 };
    let x0 = 427.0 - cell * cols as f32 / 2.0;
    let y0 = if sprites { 80.0 } else { 140.0 };
    let start = page * per;
    for i in 0..per {
        if start + i >= count {
            break;
        }
        let (cx, cy) = ((i % cols) as f32, (i / cols) as f32);
        v.push(Btn::new(
            x0 + cx * cell,
            y0 + cy * cell,
            cell,
            cell,
            BtnAct::Item(start + i),
            "",
        ));
    }
    let pages = count.div_ceil(per).max(1);
    let by = 444.0;
    v.push(Btn::new(250.0, by, 40.0, 24.0, BtnAct::Page(-5), "<<").enabled(page > 0));
    v.push(Btn::new(294.0, by, 40.0, 24.0, BtnAct::Page(-1), "<").enabled(page > 0));
    v.push(Btn::new(520.0, by, 40.0, 24.0, BtnAct::Page(1), ">").enabled(page + 1 < pages));
    v.push(Btn::new(564.0, by, 40.0, 24.0, BtnAct::Page(5), ">>").enabled(page + 1 < pages));
    v.push(Btn::new(
        20.0,
        10.0,
        120.0,
        24.0,
        BtnAct::Return,
        t.get("return"),
    ));
    v.push(Btn::new(
        150.0,
        10.0,
        120.0,
        24.0,
        BtnAct::Search,
        t.get("editor_levellist_search"),
    ));
    v.push(Btn::new(274.0, 10.0, 30.0, 24.0, BtnAct::ClearSearch, "x"));
    if !sprites {
        v.push(Btn::new(
            310.0,
            10.0,
            90.0,
            24.0,
            BtnAct::Tags,
            t.get("editor_objectlist_tags_select"),
        ));
    }
    v
}

/// Buttons of the colour picker.
pub fn colour_buttons(p: Option<&Palette>, t: &Texts) -> Vec<Btn> {
    let (pw, ph) = p.map(|p| (p.width, p.height)).unwrap_or((7, 5));
    let cell = 44.0;
    let x0 = 427.0 - cell * pw as f32 / 2.0;
    let y0 = 120.0;
    let mut v = Vec::new();
    for y in 0..ph {
        for x in 0..pw {
            v.push(Btn::new(
                x0 + x as f32 * cell,
                y0 + y as f32 * cell,
                cell - 2.0,
                cell - 2.0,
                BtnAct::Colour(x as u8, y as u8),
                "",
            ));
        }
    }
    v.push(Btn::new(
        367.0,
        400.0,
        120.0,
        24.0,
        BtnAct::Return,
        t.get("return"),
    ));
    v
}

/// Tutorial box buttons.
pub fn tutorial_buttons(t: &Texts, last: bool) -> Vec<Btn> {
    vec![
        Btn::new(
            300.0,
            426.0,
            150.0,
            24.0,
            BtnAct::TutContinue,
            t.get(if last {
                "tutorial_end"
            } else {
                "tutorial_continue"
            }),
        ),
        Btn::new(
            460.0,
            426.0,
            150.0,
            24.0,
            BtnAct::TutSkip,
            t.get("tutorial_skip"),
        ),
    ]
}

// ------------------------------------------------------------- drawing

/// Draws buttons (frames, labels).
pub fn draw_buttons(ui: &mut Ui, g: &mut Gfx, t: &Texts, btns: &[Btn], focus: Option<&BtnAct>) {
    for b in btns {
        if b.label.is_empty() {
            continue;
        }
        let border = if b.on { g.pal(4, 1) } else { g.pal(1, 3) };
        let border = if focus == Some(&b.act) {
            [
                border[0].saturating_add(60),
                border[1].saturating_add(60),
                border[2].saturating_add(60),
                255,
            ]
        } else {
            border
        };
        let fill = [
            (border[0] as f32 * 0.4) as u8,
            (border[1] as f32 * 0.4) as u8,
            (border[2] as f32 * 0.4) as u8,
            255,
        ];
        ui.frame(g, b.x, b.y, b.w, b.h, fill, border);
        let col = if b.enabled { g.pal(0, 3) } else { g.pal(0, 1) };
        let px = if b.h < 23.0 { 13.0 } else { 14.0 };
        let mut label = b.label.clone();
        while label.chars().count() > 3 && ui.measure(g, Some(t), &label, px) > b.w - 6.0 {
            label.pop();
        }
        ui.text(
            g,
            Some(t),
            &label,
            b.x + b.w / 2.0,
            b.y + (b.h - px * 1.2) / 2.0,
            px,
            col,
            Align::Center,
        );
    }
}

/// Screen pixel → logical units.
pub fn to_logical(ui: &Ui, px: f32, py: f32) -> (f32, f32) {
    ((px - ui.x(0.0)) / ui.scale, (py - ui.y(0.0)) / ui.scale)
}

/// Logical point → level cell through the level's fit.
pub fn cell_at(ui: &Ui, fit: &Fit, lx: f32, ly: f32) -> (i32, i32) {
    let (px, py) = (ui.x(lx), ui.y(ly));
    let c = 24.0 * fit.scale;
    (
        ((px - fit.rect.x as f32) / c).floor() as i32,
        ((py - fit.rect.y as f32) / c).floor() as i32,
    )
}

/// Logical rectangle of a level cell.
pub fn cell_rect(ui: &Ui, fit: &Fit, x: i32, y: i32) -> (f32, f32, f32) {
    let c = 24.0 * fit.scale;
    let px = fit.rect.x as f32 + x as f32 * c;
    let py = fit.rect.y as f32 + y as f32 * c;
    let (lx, ly) = to_logical(ui, px, py);
    (lx, ly, c / ui.scale)
}

fn outline(ui: &mut Ui, g: &Gfx, x: f32, y: f32, s: f32, t: f32, c: [u8; 4]) {
    ui.rect(g, x, y, s, t, c);
    ui.rect(g, x, y + s - t, s, t, c);
    ui.rect(g, x, y, t, s, c);
    ui.rect(g, x + s - t, y, t, s, c);
}

/// The arrow glyph of a direction.
pub fn arrow(d: u8) -> &'static str {
    ["\u{2192}", "\u{2191}", "\u{2190}", "\u{2193}"][(d & 3) as usize]
}

/// Overlays of the main view.
#[allow(clippy::too_many_arguments)]
pub fn draw_main(
    ui: &mut Ui,
    g: &mut Gfx,
    t: &Texts,
    ed: &Editor,
    eg: &mut EdGfx,
    fit: Option<Fit>,
    extended: bool,
    hints: bool,
) {
    let Some(l) = &ed.level else { return };
    let white = g.pal(0, 3);
    let grey = g.pal(0, 1);
    let bar = {
        let c = g.pal(1, 0);
        [c[0], c[1], c[2], 235]
    };
    if let Some(fit) = fit {
        // Map icons' symbols, specials, directions.
        for icon in &l.info.levels {
            let (x, y, s) = cell_rect(ui, &fit, icon.x, icon.y);
            let sym = match icon.style {
                0 => format!("{}", icon.number),
                1 => char::from(b'a' + icon.number.clamp(0, 25) as u8).to_string(),
                2 => "\u{2022}".to_string(),
                -1 => "*".to_string(),
                _ => String::new(),
            };
            if !sym.is_empty() {
                ui.text(
                    g,
                    None,
                    &sym,
                    x + s / 2.0,
                    y + s * 0.2,
                    s * 0.5,
                    white,
                    Align::Center,
                );
            }
            if icon.file.is_empty() {
                ui.text(
                    g,
                    None,
                    "?",
                    x + s * 0.85,
                    y,
                    s * 0.4,
                    g.pal(2, 2),
                    Align::Center,
                );
            }
        }
        if let Some((sx, sy)) = l.info.general.selector {
            let (x, y, s) = cell_rect(ui, &fit, sx, sy);
            outline(ui, g, x + 1.0, y + 1.0, s - 2.0, 2.0, g.pal(2, 4));
        }
        for sp in &l.info.specials {
            let (x, y, s) = cell_rect(ui, &fit, sp.x, sp.y);
            outline(ui, g, x + 2.0, y + 2.0, s - 4.0, 1.5, g.pal(3, 3));
            ui.text(
                g,
                None,
                "S",
                x + s / 2.0,
                y + s * 0.2,
                s * 0.5,
                g.pal(3, 3),
                Align::Center,
            );
        }
        if ed.show_dirs {
            for (z, x, y, o) in l.objects() {
                if z != ed.layer {
                    continue;
                }
                let (cx, cy, s) = cell_rect(ui, &fit, x, y);
                ui.rect(
                    g,
                    cx + s * 0.25,
                    cy + s * 0.25,
                    s * 0.5,
                    s * 0.5,
                    [0, 0, 0, 150],
                );
                ui.text(
                    g,
                    None,
                    arrow(o.dir),
                    cx + s / 2.0,
                    cy + s * 0.18,
                    s * 0.55,
                    white,
                    Align::Center,
                );
            }
        }
        // Tool previews.
        let hl = |ui: &mut Ui, g: &Gfx, cells: &[(i32, i32)], c: [u8; 4]| {
            for &(x, y) in cells {
                let (cx, cy, s) = cell_rect(ui, &fit, x, y);
                ui.rect(g, cx, cy, s, s, c);
            }
        };
        let mut pc = g.pal(4, 1);
        pc[3] = 90;
        hl(ui, g, &ed.span_preview(), pc);
        let mut fc = g.pal(3, 3);
        fc[3] = 60;
        hl(ui, g, &ed.fill_preview(), fc);
        // The floating selection at the cursor (bottom-right anchored).
        let (cx, cy) = ed.cursor;
        if let Some(sel) = &ed.selection {
            let (ox, oy) = (cx - sel.w + 1, cy - sel.h + 1);
            for y in 0..sel.h {
                for x in 0..sel.w {
                    let (px, py, s) = cell_rect(ui, &fit, ox + x, oy + y);
                    if let Some(o) = sel.get(x, y)
                        && let Some(def) = l.table.by_tile(o.tile).cloned()
                    {
                        let r = eg.thumb(&mut g.atlas, &def, o.dir);
                        let mut c = eg.colour(def.colour);
                        c[3] = 200;
                        ui.sprite(r, px, py, s, s, c);
                    } else {
                        ui.rect(g, px, py, s, s, [0, 0, 0, 60]);
                    }
                }
            }
            let (px, py, s) = cell_rect(ui, &fit, ox, oy);
            outline(ui, g, px, py, s * sel.w as f32, 2.0, g.pal(4, 1));
            let _ = s;
        } else if l.inside(cx, cy) && (ed.pointer.is_some() || ed.pad.active) {
            let (px, py, s) = cell_rect(ui, &fit, cx, cy);
            if let Pick::Slot(slot) = &ed.pick
                && ed.brush == Brush::Objects
                && ed.tool != Tool::Eraser
                && let Some(def) = l.table.get(slot).cloned()
            {
                let r = eg.thumb(&mut g.atlas, &def, ed.dir);
                let mut c = eg.colour(def.colour);
                c[3] = 150;
                ui.sprite(r, px, py, s, s, c);
            }
            let c = if ed.erasing() { g.pal(2, 2) } else { white };
            outline(ui, g, px, py, s, 2.0, c);
        }
    }
    // Title: "NAME (STEM)" top left, like the original.
    let mut title = format!(
        "{} ({})",
        if l.info.general.name.is_empty() {
            &l.stem
        } else {
            &l.info.general.name
        },
        l.stem
    );
    if ed.unsaved {
        title.push_str(" *");
    }
    ui.text(g, Some(t), &title, 12.0, 2.0, 16.0, white, Align::Left);
    let btns = main_buttons(ed, t, extended);
    let _ = bar;
    draw_buttons(ui, g, t, &btns, None);
    for b in &btns {
        match b.act {
            BtnAct::Quick(i) => {
                let border = if ed.qsel == i {
                    g.pal(4, 1)
                } else {
                    g.pal(1, 1)
                };
                ui.frame(g, b.x, b.y, b.w, b.h, [0, 0, 0, 230], border);
                if let Some(q) = &l.quick[i]
                    && let Some(def) = l.table.get(&q.slot).cloned()
                {
                    let r = eg.thumb(&mut g.atlas, &def, 3);
                    ui.sprite(r, b.x + 2.0, b.y + 2.0, 24.0, 24.0, eg.colour(def.colour));
                    if q.locked {
                        ui.rect(g, b.x + b.w - 6.0, b.y + 1.0, 5.0, 5.0, g.pal(2, 2));
                    }
                }
            }
            BtnAct::Tool(tool) => {
                let border = if ed.tool == tool {
                    g.pal(4, 1)
                } else {
                    g.pal(1, 3)
                };
                let fill = [
                    (border[0] as f32 * 0.4) as u8,
                    (border[1] as f32 * 0.4) as u8,
                    (border[2] as f32 * 0.4) as u8,
                    255,
                ];
                ui.frame(g, b.x, b.y, b.w, b.h, fill, border);
                draw_tool_icon(ui, g, tool, b.x + 4.0, b.y + 4.0, b.w - 8.0, white);
            }
            _ => {}
        }
    }
    // What is picked / the brush, left of the quickbar.
    let picked = match &ed.pick {
        Pick::Empty => t.get("editor_toolindicator_eraser"),
        Pick::Slot(s) => l.table.get(s).map(|d| d.name.clone()).unwrap_or_default(),
        Pick::None => String::new(),
    };
    let brush = match ed.brush {
        Brush::Objects => String::new(),
        Brush::Levels => format!("[{}] ", t.get("editor_brush_level")),
        Brush::Paths => format!("[{}] ", t.get("editor_brush_path")),
        Brush::Special => format!("[{}] ", t.get("editor_brush_special")),
        Brush::Cursor => format!("[{}] ", t.get("editor_setcursor")),
    };
    ui.text(
        g,
        Some(t),
        &format!("{brush}{picked} {}", arrow(ed.dir)),
        544.0,
        10.0,
        12.0,
        grey,
        Align::Right,
    );
    if hints && ed.pad.active {
        ui.text(
            g,
            None,
            "A place  X copy  B undo  Y palette  LB/RB quickbar  LT/RT tool  Select+: more  Start: menu",
            427.0,
            440.0,
            11.0,
            grey,
            Align::Center,
        );
    }
    if let Some((n, _)) = &ed.notice {
        ui.rect(g, 277.0, 200.0, 300.0, 40.0, [0, 0, 0, 200]);
        ui.text(
            g,
            Some(t),
            n,
            427.0,
            210.0,
            18.0,
            g.pal(2, 4),
            Align::Center,
        );
    }
}

/// The palette screen (`§5.4`).
pub fn draw_palette(
    ui: &mut Ui,
    g: &mut Gfx,
    t: &Texts,
    ed: &Editor,
    eg: &mut EdGfx,
    extended: bool,
) {
    let Some(l) = &ed.level else { return };
    let title = format!("{} ({})", l.info.general.name, l.stem);
    ui.text(
        g,
        Some(t),
        &title,
        12.0,
        2.0,
        16.0,
        g.pal(0, 3),
        Align::Left,
    );
    let btns = palette_buttons(ed, t, extended);
    draw_buttons(ui, g, t, &btns, None);
    for b in &btns {
        if let BtnAct::Tool(tool) = b.act {
            let border = if ed.tool == tool {
                g.pal(4, 1)
            } else {
                g.pal(1, 3)
            };
            let fill = [
                (border[0] as f32 * 0.4) as u8,
                (border[1] as f32 * 0.4) as u8,
                (border[2] as f32 * 0.4) as u8,
                255,
            ];
            ui.frame(g, b.x, b.y, b.w, b.h, fill, border);
            draw_tool_icon(ui, g, tool, b.x + 6.0, b.y + 6.0, b.w - 12.0, g.pal(0, 3));
        }
    }
    let (cell, x0, y0, gw, gh) = palette_grid(ed);
    let search = ed.pal.search.to_lowercase();
    let frame = g.pal(1, 1);
    for gy in 0..gh {
        for gx in 0..gw {
            outline(
                ui,
                g,
                x0 + gx as f32 * cell,
                y0 + gy as f32 * cell,
                cell,
                1.0,
                frame,
            );
        }
    }
    for (i, e) in l.palette.iter().enumerate() {
        let c = if ed.pal.full { e.gf } else { e.go };
        if !ed.pal.full {
            // Pairs share a cell: show the object (or the word in the
            // swapped view).
            if let Some(p) = e.pair {
                let is_text = l.def(i).is_some_and(ObjectDef::is_text);
                if is_text != ed.pal.words {
                    let _ = p;
                    continue;
                }
            }
        }
        if c.0 >= gw || c.1 >= gh {
            continue;
        }
        let Some(def) = l.def(i).cloned() else {
            continue;
        };
        let r = eg.thumb(&mut g.atlas, &def, ed.dir);
        let mut col = eg.colour(def.colour);
        let base = e.name.strip_prefix("text_").unwrap_or(&e.name);
        if !search.is_empty() && !base.contains(&search) {
            col[3] = 60;
        }
        let (px, py) = (x0 + c.0 as f32 * cell, y0 + c.1 as f32 * cell);
        ui.sprite(
            r,
            px + cell * 0.1,
            py + cell * 0.1,
            cell * 0.8,
            cell * 0.8,
            col,
        );
        if matches!(&ed.pick, Pick::Slot(s) if *s == e.slot) {
            outline(ui, g, px + 1.0, py + 1.0, cell - 2.0, 2.0, g.pal(4, 1));
        }
    }
    let (sx, sy) = ed.pal.sel;
    outline(
        ui,
        g,
        x0 + sx as f32 * cell,
        y0 + sy as f32 * cell,
        cell,
        2.0,
        g.pal(0, 3),
    );
    // Name of the entry under the selection.
    let name = palette_entry_at(ed, ed.pal.sel)
        .and_then(|i| l.palette.get(i))
        .map(|e| e.name.clone())
        .unwrap_or_default();
    let count = format!("{} / {}", l.palette.len(), super::model::MAX_PALETTE);
    ui.text(
        g,
        Some(t),
        &name,
        646.0,
        460.0,
        14.0,
        g.pal(0, 3),
        Align::Left,
    );
    ui.text(
        g,
        None,
        &count,
        836.0,
        460.0,
        12.0,
        g.pal(0, 1),
        Align::Right,
    );
    let search = if ed.pal.search.is_empty() {
        t.get("editor_objectlist_search_none")
    } else {
        format!("{} {}", t.get("editor_objectlist_result"), ed.pal.search)
    };
    ui.text(
        g,
        Some(t),
        &search,
        38.0,
        78.0,
        14.0,
        g.pal(0, 3),
        Align::Left,
    );
}

/// The palette entry shown at a grid cell (paired view: the object or
/// its word depending on the swap).
pub fn palette_entry_at(ed: &Editor, c: (i32, i32)) -> Option<usize> {
    let l = ed.level.as_ref()?;
    let mut found = None;
    for (i, e) in l.palette.iter().enumerate() {
        let p = if ed.pal.full { e.gf } else { e.go };
        if p != c {
            continue;
        }
        if ed.pal.full || e.pair.is_none() {
            return Some(i);
        }
        let is_text = l.def(i).is_some_and(ObjectDef::is_text);
        if is_text == ed.pal.words {
            return Some(i);
        }
        found = found.or(Some(i));
    }
    found
}

/// The object list (`§5.5`).
#[allow(clippy::too_many_arguments)]
pub fn draw_list(
    ui: &mut Ui,
    g: &mut Gfx,
    t: &Texts,
    ed: &Editor,
    eg: &mut EdGfx,
    base: &ObjectTable,
    extended: bool,
    advanced: bool,
) {
    ui.fill(g, g.pal(1, 0));
    let items = list_items(&ed.cat, &ed.list.search, &ed.list.tags, extended, advanced);
    let btns = list_buttons(items.len(), ed.list.page, t, false);
    draw_buttons(ui, g, t, &btns, None);
    let present: HashSet<i32> = ed
        .level
        .as_ref()
        .map(|l| l.palette.iter().map(|e| e.id).collect())
        .unwrap_or_default();
    let tmpl = base.get("object000").cloned();
    for b in &btns {
        let BtnAct::Item(i) = b.act else { continue };
        let Some(o) = items.get(i) else { continue };
        let Some(slot) = &tmpl else { continue };
        let def = super::model::def_from_catalogue(slot, o, &o.name);
        let r = eg.thumb(&mut g.atlas, &def, 3);
        let mut c = eg.colour(def.colour);
        if present.contains(&(o.index as i32)) {
            c[3] = 70;
        }
        ui.sprite(r, b.x + 8.0, b.y + 4.0, b.w - 16.0, b.w - 16.0, c);
        if i == ed.list.sel {
            outline(ui, g, b.x, b.y, b.w, 2.0, g.pal(0, 3));
        }
    }
    let name = items
        .get(ed.list.sel)
        .map(|o| o.name.clone())
        .unwrap_or_default();
    ui.text(
        g,
        Some(t),
        &name,
        427.0,
        50.0,
        16.0,
        g.pal(0, 3),
        Align::Center,
    );
    let pages = items.len().div_ceil(LIST_PAGE).max(1);
    ui.text(
        g,
        None,
        &format!("{}/{}", ed.list.page + 1, pages),
        427.0,
        448.0,
        14.0,
        g.pal(0, 3),
        Align::Center,
    );
    let mut info = String::new();
    if !ed.list.search.is_empty() {
        info.push_str(&format!(
            "{} {}  ",
            t.get("editor_objectlist_result"),
            ed.list.search
        ));
    }
    if !ed.list.tags.is_empty() {
        info.push_str(&format!(
            "{} {}",
            t.get("editor_objectlist_tags"),
            ed.list.tags.join(", ")
        ));
    }
    ui.text(
        g,
        Some(t),
        &info,
        420.0,
        14.0,
        13.0,
        g.pal(2, 4),
        Align::Left,
    );
}

/// The sprite picker.
pub fn draw_sprites(
    ui: &mut Ui,
    g: &mut Gfx,
    t: &Texts,
    ed: &Editor,
    eg: &mut EdGfx,
    data_dir: &Path,
) {
    let world_sprites = ed.level.as_ref().map(|l| l.place.dir.join("Sprites"));
    ui.fill(g, g.pal(1, 0));
    let list = sprite_items(ed);
    let btns = list_buttons(list.len(), ed.sprites.page, t, true);
    draw_buttons(ui, g, t, &btns, None);
    for b in &btns {
        let BtnAct::Item(i) = b.act else { continue };
        let Some((name, root)) = list.get(i) else {
            continue;
        };
        let region = if ed.sprites.icon {
            let dir = if *root {
                Some(data_dir.join("Sprites"))
            } else {
                world_sprites.clone()
            };
            dir.and_then(|d| eg.pic(&mut g.atlas, &d.join(format!("{name}.png"))))
        } else {
            eg.sprite(&mut g.atlas, name, *root)
        };
        if let Some(r) = region {
            ui.sprite(r, b.x + 4.0, b.y + 4.0, b.w - 8.0, b.w - 8.0, g.pal(0, 3));
        }
        if i == ed.sprites.sel {
            outline(ui, g, b.x, b.y, b.w, 2.0, g.pal(4, 1));
        }
    }
    let name = list
        .get(ed.sprites.sel)
        .map(|(n, r)| if *r { n.clone() } else { format!("{n} (pack)") })
        .unwrap_or_default();
    ui.text(
        g,
        Some(t),
        &t.get("editor_spritelist_title"),
        427.0,
        44.0,
        15.0,
        g.pal(0, 3),
        Align::Center,
    );
    ui.text(
        g,
        None,
        &name,
        427.0,
        62.0,
        13.0,
        g.pal(2, 4),
        Align::Center,
    );
}

/// The sprite list after the search.
pub fn sprite_items(ed: &Editor) -> Vec<(String, bool)> {
    let s = ed.sprites.search.to_lowercase();
    ed.sprites
        .list
        .iter()
        .filter(|(n, _)| s.is_empty() || n.contains(&s))
        .cloned()
        .collect()
}

/// The colour picker.
pub fn draw_colours(
    ui: &mut Ui,
    g: &mut Gfx,
    t: &Texts,
    eg: &EdGfx,
    which: ColourFor,
    current: Option<PalColour>,
) {
    ui.fill(g, g.pal(1, 0));
    let title = match which {
        ColourFor::Base | ColourFor::Icon(_) => t.get("editor_object_colour_select"),
        ColourFor::Active => t.get("editor_object_colour_active"),
        ColourFor::IconClear(_) => t.get("editor_level_setclearcolour"),
    };
    ui.text(
        g,
        Some(t),
        &title,
        427.0,
        80.0,
        16.0,
        g.pal(0, 3),
        Align::Center,
    );
    let btns = colour_buttons(eg.palette.as_ref(), t);
    for b in &btns {
        if let BtnAct::Colour(x, y) = b.act {
            let c = eg.colour(PalColour::new(x, y));
            ui.rect(g, b.x, b.y, b.w, b.h, c);
            if current == Some(PalColour::new(x, y)) {
                outline(ui, g, b.x - 2.0, b.y - 2.0, b.w + 4.0, 2.0, g.pal(0, 3));
            }
        }
    }
    draw_buttons(ui, g, t, &btns, None);
}

/// Tutorial pointer arrows (`editor.md` §14.6): each points from the
/// screen centre toward its target, the tip at the target, a 48 px shaft
/// and a two-stroke head, drawn as a pixel line.
pub fn draw_pointers(ui: &mut Ui, g: &Gfx, targets: &[(f32, f32)]) {
    let c = g.pal(0, 3);
    let line = |ui: &mut Ui, x0: f32, y0: f32, x1: f32, y1: f32| {
        let n = ((x1 - x0).hypot(y1 - y0)).ceil().max(1.0) as i32;
        for i in 0..=n {
            let k = i as f32 / n as f32;
            ui.rect(
                g,
                x0 + (x1 - x0) * k - 1.5,
                y0 + (y1 - y0) * k - 1.5,
                3.0,
                3.0,
                c,
            );
        }
    };
    for &(tx, ty) in targets {
        let (dx, dy) = (tx - 427.0, ty - 240.0);
        let d = dx.hypot(dy).max(1.0);
        let (ux, uy) = (dx / d, dy / d);
        line(ui, tx - ux * 48.0, ty - uy * 48.0, tx, ty);
        for s in [-1.0f32, 1.0] {
            let a = 35f32.to_radians() * s;
            let (rx, ry) = (ux * a.cos() - uy * a.sin(), ux * a.sin() + uy * a.cos());
            line(ui, tx - rx * 14.0, ty - ry * 14.0, tx, ty);
        }
    }
}

/// The tutorial text box (`§14`).
pub fn draw_tutorial(ui: &mut Ui, g: &mut Gfx, t: &Texts, run: &super::tutorial::Run, pad: bool) {
    let paras = super::tutorial::texts(t, run.lesson, run.slide, pad);
    let mut lines: Vec<(String, bool)> = Vec::new();
    for (i, p) in paras.iter().enumerate() {
        for l in super::tutorial::wrap(p, 78) {
            lines.push((l, i == 0 && run.slide == 1));
        }
        lines.push((String::new(), false));
    }
    let h = 22.0 + 17.0 * lines.len() as f32;
    let y0 = (420.0 - h).max(6.0);
    ui.rect(g, 40.0, y0, 774.0, h + 40.0, [0, 0, 0, 215]);
    let mut y = y0 + 10.0;
    for (l, head) in &lines {
        let c = if *head { g.pal(1, 3) } else { g.pal(0, 3) };
        ui.text(g, Some(t), l, 60.0, y, 14.0, c, Align::Left);
        y += 17.0;
    }
    let lesson = t.get(&format!("editor_tutorial{}", run.lesson));
    ui.text(
        g,
        Some(t),
        &format!("{lesson}  {}/{}", run.slide, run.count()),
        800.0,
        y0 + 4.0,
        11.0,
        g.pal(0, 1),
        Align::Right,
    );
    draw_buttons(ui, g, t, &tutorial_buttons(t, run.last()), None);
}
