//! Dev tool: reads every level of every world from the player's install and
//! renders contact sheets (CPU compositing), plus a summary of counts.
//!
//! ```text
//! cargo run --release -p baba-formats --example levels_sheet -- \
//!     [--game DIR] [--out DIR] [--tile PX] [--cols N] [--each] [--assets] [WORLD...]
//! ```
//!
//! Defaults: `--game $BABA_GAME_DIR`, `--out scratch/m1`, `--tile 8`.
//! Writes `<out>/levels-sheet-<world>.png`, `<out>/summary.txt`, with
//! `--each` also `<out>/levels/<world>/<stem>.png` at full size, with
//! `--assets` also `<out>/assets-images.png` (every image in `Assets.dat`).
//! The output contains game art: keep it out of git (scratch/ is ignored).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use baba_formats::level::Direction;
use baba_formats::{
    AssetKind, AssetsDat, GameInstall, Level, ObjectDef, ObjectTable, PalColour, Palette,
    RgbaImage, SpriteSet, TileId, Tiling, World, sprite_variant,
};

const CELL: u32 = 24;

type Cache<K, V> = Mutex<HashMap<K, Option<std::sync::Arc<V>>>>;

struct Args {
    game: Option<PathBuf>,
    out: PathBuf,
    tile: u32,
    cols: Option<u32>,
    each: bool,
    assets: bool,
    worlds: Vec<String>,
}

fn parse_args() -> Args {
    let mut a = Args {
        game: None,
        out: PathBuf::from("scratch/m1"),
        tile: 8,
        cols: None,
        each: false,
        assets: false,
        worlds: Vec::new(),
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--game" => a.game = it.next().map(PathBuf::from),
            "--out" => a.out = it.next().map(PathBuf::from).expect("--out DIR"),
            "--tile" => a.tile = it.next().and_then(|v| v.parse().ok()).expect("--tile PX"),
            "--cols" => a.cols = it.next().and_then(|v| v.parse().ok()),
            "--each" => a.each = true,
            "--assets" => a.assets = true,
            "-h" | "--help" => {
                eprintln!(
                    "levels_sheet [--game DIR] [--out DIR] [--tile PX] [--cols N] [--each] [--assets] [WORLD...]"
                );
                std::process::exit(0);
            }
            w => a.worlds.push(w.to_string()),
        }
    }
    assert!(
        a.tile > 0 && CELL.is_multiple_of(a.tile),
        "--tile must divide 24 (24, 12, 8, 6, 4, 3, 2, 1)"
    );
    a
}

/// Everything needed to draw one world's levels.
struct WorldCtx<'a> {
    install: &'a GameInstall,
    world: &'a World,
    base: &'a ObjectTable,
    sprites: SpriteSet,
    sprite_cache: Cache<(String, bool, u8), RgbaImage>,
    palette_cache: Mutex<HashMap<(String, bool), Option<Palette>>>,
    image_cache: Cache<String, RgbaImage>,
}

#[derive(Default)]
struct LevelStats {
    objects: usize,
    unknown: BTreeMap<u16, usize>,
    missing_sprites: BTreeSet<String>,
    missing_palette: Option<String>,
}

impl WorldCtx<'_> {
    fn sprite(&self, name: &str, root: bool, variant: u8) -> Option<std::sync::Arc<RgbaImage>> {
        let key = (name.to_string(), root, variant);
        if let Some(v) = self.sprite_cache.lock().unwrap().get(&key) {
            return v.clone();
        }
        let img = self
            .sprites
            .find(name, root, variant, 1)
            .and_then(|p| RgbaImage::load_png(p).ok())
            .map(std::sync::Arc::new);
        self.sprite_cache.lock().unwrap().insert(key, img.clone());
        img
    }

    fn palette(&self, name: &str, root: bool) -> Option<Palette> {
        let key = (name.to_string(), root);
        if let Some(v) = self.palette_cache.lock().unwrap().get(&key) {
            return v.clone();
        }
        let p = self.install.load_palette(Some(self.world), name, root).ok();
        self.palette_cache.lock().unwrap().insert(key, p.clone());
        p
    }

    fn image(&self, name: &str) -> Option<std::sync::Arc<RgbaImage>> {
        if let Some(v) = self.image_cache.lock().unwrap().get(name) {
            return v.clone();
        }
        let path = self.world.images_dir().join(format!("{name}_1.png"));
        let img = RgbaImage::load_png(&path).ok().map(std::sync::Arc::new);
        self.image_cache
            .lock()
            .unwrap()
            .insert(name.to_string(), img.clone());
        img
    }

    /// Renders a level at 24 px per cell.
    fn render(&self, level: &Level, stats: &mut LevelStats) -> RgbaImage {
        let objects = self.base.with_changes(&level.info.tiles);
        let g = &level.info.general;
        let palette = self.palette(&g.palette, g.palette_root).unwrap_or_else(|| {
            stats.missing_palette = Some(g.palette.clone());
            self.palette("default.png", true).expect("default palette")
        });
        let (w, h) = (level.map.width(), level.map.height());
        let mut img = RgbaImage::new(w * CELL, h * CELL);
        let bg = palette.rgb(PalColour::new(0, 4));
        let edge = palette.rgb(PalColour::new(1, 0));
        for y in 0..h {
            for x in 0..w {
                let border = x == 0 || y == 0 || x == w - 1 || y == h - 1;
                let c = if border { edge } else { bg };
                fill(
                    &mut img,
                    x * CELL,
                    y * CELL,
                    CELL,
                    CELL,
                    [c[0], c[1], c[2], 255],
                );
            }
        }
        for name in &level.info.images {
            if let Some(bgimg) = self.image(name) {
                blit(&mut img, &bgimg, CELL as i32, CELL as i32, None);
            }
        }

        // Collect (layer, order, x, y, obj, dir).
        let mut names: HashMap<(u32, u32), Vec<&str>> = HashMap::new();
        let mut draws: Vec<(i32, usize, u32, u32, &ObjectDef, Direction)> = Vec::new();
        let mut order = 0usize;
        for layer in &level.map.layers {
            for (x, y, tile, dir) in layer.objects() {
                if tile == TileId::EDGE {
                    continue;
                }
                match objects.by_tile(tile) {
                    Some(o) => {
                        stats.objects += 1;
                        names.entry((x, y)).or_default().push(&o.name);
                        draws.push((o.layer, order, x, y, o, dir));
                        order += 1;
                    }
                    None => *stats.unknown.entry(tile.0).or_default() += 1,
                }
            }
        }
        draws.sort_by_key(|d| (d.0, d.1));
        for (_, _, x, y, o, dir) in draws {
            let joined = if o.tiling == Tiling::Tiled {
                let mut m = 0u8;
                for (bit, (dx, dy)) in
                    [(1u8, (1i32, 0i32)), (2, (0, -1)), (4, (-1, 0)), (8, (0, 1))]
                {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    let border = nx <= 0 || ny <= 0 || nx >= w as i32 - 1 || ny >= h as i32 - 1;
                    let same = names
                        .get(&(nx as u32, ny as u32))
                        .is_some_and(|v| v.contains(&o.name.as_str()));
                    if border || same {
                        m |= bit;
                    }
                }
                m
            } else {
                0
            };
            let variant = sprite_variant(o.tiling, dir, joined, 0);
            let colour = palette.rgb(o.colour);
            self.draw_sprite(
                &mut img,
                &o.sprite,
                o.sprite_in_root,
                variant,
                x,
                y,
                colour,
                stats,
            );
        }

        // Map decorations: path tiles and level icons.
        for p in &level.info.paths {
            if let Some(o) = objects.get(&p.object) {
                let variant = sprite_variant(o.tiling, Direction::from_u8(p.dir as u8), 0, 0);
                let colour = palette.rgb(o.colour);
                self.draw_sprite(
                    &mut img,
                    &o.sprite,
                    o.sprite_in_root,
                    variant,
                    p.x as u32,
                    p.y as u32,
                    colour,
                    stats,
                );
            }
        }
        for l in &level.info.levels {
            let c = palette.rgb(l.colour.unwrap_or(PalColour::new(0, 3)));
            let (px, py) = (l.x as u32 * CELL, l.y as u32 * CELL);
            fill(
                &mut img,
                px + 3,
                py + 3,
                CELL - 6,
                CELL - 6,
                [c[0], c[1], c[2], 255],
            );
            let label = l.number.to_string();
            let lw = label.len() as u32 * 4;
            let fg = palette.rgb(PalColour::new(0, 4));
            draw_text(
                &mut img,
                px + (CELL - lw) / 2,
                py + 9,
                &label,
                1,
                [fg[0], fg[1], fg[2], 255],
            );
        }
        img
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_sprite(
        &self,
        img: &mut RgbaImage,
        sprite: &str,
        root: bool,
        variant: u8,
        x: u32,
        y: u32,
        colour: [u8; 3],
        stats: &mut LevelStats,
    ) {
        match self.sprite(sprite, root, variant) {
            Some(s) => {
                let ox = (x * CELL) as i32 + (CELL as i32 - s.width as i32) / 2;
                let oy = (y * CELL) as i32 + (CELL as i32 - s.height as i32) / 2;
                blit(img, &s, ox, oy, Some(colour));
            }
            None => {
                stats.missing_sprites.insert(sprite.to_string());
                fill(img, x * CELL + 6, y * CELL + 6, 12, 12, [255, 0, 255, 255]);
            }
        }
    }
}

fn fill(img: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32, c: [u8; 4]) {
    for yy in y..(y + h).min(img.height) {
        for xx in x..(x + w).min(img.width) {
            img.set_pixel(xx, yy, c);
        }
    }
}

/// Alpha-blends `src` at (x, y), optionally multiplying by a tint.
fn blit(dst: &mut RgbaImage, src: &RgbaImage, x: i32, y: i32, tint: Option<[u8; 3]>) {
    for sy in 0..src.height {
        let dy = y + sy as i32;
        if dy < 0 || dy >= dst.height as i32 {
            continue;
        }
        for sx in 0..src.width {
            let dx = x + sx as i32;
            if dx < 0 || dx >= dst.width as i32 {
                continue;
            }
            let mut p = src.pixel(sx, sy);
            if p[3] == 0 {
                continue;
            }
            if let Some(t) = tint {
                for c in 0..3 {
                    p[c] = (p[c] as u32 * t[c] as u32 / 255) as u8;
                }
            }
            let d = dst.pixel(dx as u32, dy as u32);
            let a = p[3] as u32;
            let out = [
                ((p[0] as u32 * a + d[0] as u32 * (255 - a)) / 255) as u8,
                ((p[1] as u32 * a + d[1] as u32 * (255 - a)) / 255) as u8,
                ((p[2] as u32 * a + d[2] as u32 * (255 - a)) / 255) as u8,
                255,
            ];
            dst.set_pixel(dx as u32, dy as u32, out);
        }
    }
}

/// Box-filter downscale by an integer factor.
fn downscale(src: &RgbaImage, f: u32) -> RgbaImage {
    if f == 1 {
        return src.clone();
    }
    let (w, h) = (src.width / f, src.height / f);
    let mut out = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0u32; 4];
            for yy in 0..f {
                for xx in 0..f {
                    let p = src.pixel(x * f + xx, y * f + yy);
                    for c in 0..4 {
                        acc[c] += p[c] as u32;
                    }
                }
            }
            let n = f * f;
            out.set_pixel(
                x,
                y,
                [
                    (acc[0] / n) as u8,
                    (acc[1] / n) as u8,
                    (acc[2] / n) as u8,
                    (acc[3] / n) as u8,
                ],
            );
        }
    }
    out
}

/// Nearest-neighbour fit into a `max`-sized box (for Assets.dat thumbnails).
fn fit(src: &RgbaImage, max: u32) -> RgbaImage {
    let s = (max as f32 / src.width.max(src.height).max(1) as f32).min(4.0);
    let (w, h) = (
        ((src.width as f32 * s) as u32).max(1),
        ((src.height as f32 * s) as u32).max(1),
    );
    let mut out = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let sx = ((x as f32 / s) as u32).min(src.width - 1);
            let sy = ((y as f32 / s) as u32).min(src.height - 1);
            out.set_pixel(x, y, src.pixel(sx, sy));
        }
    }
    out
}

// A 3x5 pixel font for labels (our own glyphs).
fn glyph(c: char) -> [u8; 5] {
    match c.to_ascii_lowercase() {
        '0' => [7, 5, 5, 5, 7],
        '1' => [2, 6, 2, 2, 7],
        '2' => [7, 1, 7, 4, 7],
        '3' => [7, 1, 7, 1, 7],
        '4' => [5, 5, 7, 1, 1],
        '5' => [7, 4, 7, 1, 7],
        '6' => [7, 4, 7, 5, 7],
        '7' => [7, 1, 1, 1, 1],
        '8' => [7, 5, 7, 5, 7],
        '9' => [7, 5, 7, 1, 7],
        'a' => [2, 5, 7, 5, 5],
        'b' => [6, 5, 6, 5, 6],
        'c' => [3, 4, 4, 4, 3],
        'd' => [6, 5, 5, 5, 6],
        'e' => [7, 4, 6, 4, 7],
        'f' => [7, 4, 6, 4, 4],
        'g' => [3, 4, 5, 5, 3],
        'h' => [5, 5, 7, 5, 5],
        'i' => [7, 2, 2, 2, 7],
        'j' => [1, 1, 1, 5, 2],
        'k' => [5, 5, 6, 5, 5],
        'l' => [4, 4, 4, 4, 7],
        'm' => [5, 7, 7, 5, 5],
        'n' => [6, 5, 5, 5, 5],
        'o' => [2, 5, 5, 5, 2],
        'p' => [6, 5, 6, 4, 4],
        'q' => [2, 5, 5, 6, 3],
        'r' => [6, 5, 6, 5, 5],
        's' => [3, 4, 2, 1, 6],
        't' => [7, 2, 2, 2, 2],
        'u' => [5, 5, 5, 5, 7],
        'v' => [5, 5, 5, 5, 2],
        'w' => [5, 5, 7, 7, 5],
        'x' => [5, 5, 2, 5, 5],
        'y' => [5, 5, 2, 2, 2],
        'z' => [7, 1, 2, 4, 7],
        '-' => [0, 0, 7, 0, 0],
        '_' => [0, 0, 0, 0, 7],
        '.' => [0, 0, 0, 0, 2],
        ',' => [0, 0, 0, 2, 4],
        ':' => [0, 2, 0, 2, 0],
        '!' => [2, 2, 2, 0, 2],
        '?' => [7, 1, 2, 0, 2],
        '\'' => [2, 2, 0, 0, 0],
        '(' => [1, 2, 2, 2, 1],
        ')' => [4, 2, 2, 2, 4],
        '/' => [1, 1, 2, 4, 4],
        '+' => [0, 2, 7, 2, 0],
        '&' => [2, 5, 2, 5, 3],
        ' ' => [0; 5],
        _ => [7, 7, 7, 7, 7],
    }
}

fn draw_text(img: &mut RgbaImage, x: u32, y: u32, text: &str, scale: u32, c: [u8; 4]) {
    for (i, ch) in text.chars().enumerate() {
        let g = glyph(ch);
        for (row, bits) in g.iter().enumerate() {
            for col in 0..3 {
                if bits & (4 >> col) != 0 {
                    fill(
                        img,
                        x + (i as u32 * 4 + col) * scale,
                        y + row as u32 * scale,
                        scale,
                        scale,
                        c,
                    );
                }
            }
        }
    }
}

fn parallel_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let n = std::thread::available_parallelism().map_or(4, |n| n.get());
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<R>>> = Mutex::new((0..items.len()).map(|_| None).collect());
    std::thread::scope(|s| {
        for _ in 0..n {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= items.len() {
                        break;
                    }
                    let r = f(&items[i]);
                    results.lock().unwrap()[i] = Some(r);
                }
            });
        }
    });
    results
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|r| r.expect("every item processed"))
        .collect()
}

fn main() {
    let args = parse_args();
    let t0 = Instant::now();
    let install = match &args.game {
        Some(p) => GameInstall::open(p),
        None => GameInstall::from_env().unwrap_or_else(|| {
            eprintln!("set $BABA_GAME_DIR or pass --game DIR");
            std::process::exit(2)
        }),
    }
    .unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2)
    });
    std::fs::create_dir_all(&args.out).expect("create output dir");
    let mut summary = String::new();
    macro_rules! say {
        ($($t:tt)*) => {{ let s = format!($($t)*); println!("{s}"); summary.push_str(&s); summary.push('\n'); }};
    }

    let base = install.load_objects().expect("values.lua");
    let editor = install
        .load_editor_objects()
        .expect("editor_objectlist.lua");
    say!(
        "object slots: {} (values.lua tileslist), editor objects: {} (order list {})",
        base.len(),
        editor.objects.len(),
        editor.order.len()
    );
    let root_sprites = baba_formats::SpriteDir::scan(&install.sprites_dir()).expect("sprites");
    say!(
        "Data/Sprites: {} sprite names, {} files ({} other pngs)",
        root_sprites.len(),
        root_sprites.file_count(),
        root_sprites.other_files.len()
    );
    let palettes = install.palettes();
    let pal_ok = palettes.iter().filter(|p| Palette::load(p).is_ok()).count();
    say!("palettes: {}/{} decoded", pal_ok, palettes.len());
    let themes = install.themes();
    let themes_ok = themes
        .iter()
        .filter(|p| baba_formats::Theme::load(p).is_ok())
        .count();
    say!("global themes: {}/{}", themes_ok, themes.len());
    let langs = install.languages();
    let lang_texts: usize = langs
        .iter()
        .filter_map(|p| baba_formats::Language::load(p).ok())
        .map(|l| l.texts().len())
        .sum();
    say!("languages: {} ({} texts)", langs.len(), lang_texts);
    say!(
        "music: {} files, loose sounds: {} files",
        install.music().len(),
        install.sounds().len()
    );

    let world_ids = if args.worlds.is_empty() {
        install.world_ids().expect("worlds")
    } else {
        args.worlds.clone()
    };
    // Pure parsing speed: every .l and .ld of the selected worlds.
    let tp = Instant::now();
    let mut parsed = 0;
    for id in &world_ids {
        let world = install.world(id).expect("world");
        for stem in &world.levels {
            if world.load_level(stem).is_ok() {
                parsed += 1;
            }
        }
    }
    say!(
        "parsed {parsed} levels (.l + .ld) in {:.0} ms",
        tp.elapsed().as_secs_f64() * 1000.0
    );
    let mut total_levels = 0;
    let mut total_unknown = 0;
    for id in &world_ids {
        let tw = Instant::now();
        let world = install.world(id).expect("world");
        let ctx = WorldCtx {
            install: &install,
            world: &world,
            base: &base,
            sprites: SpriteSet::load(&install, Some(&world)).expect("sprites"),
            sprite_cache: Mutex::default(),
            palette_cache: Mutex::default(),
            image_cache: Mutex::default(),
        };
        let wthemes = world.themes();
        let wthemes_ok = wthemes
            .iter()
            .filter(|p| baba_formats::Theme::load(p).is_ok())
            .count();
        let rendered = parallel_map(&world.levels, |stem| {
            let mut st = LevelStats::default();
            match world.load_level(stem) {
                Ok(level) => {
                    let img = ctx.render(&level, &mut st);
                    Ok((level, img, st))
                }
                Err(e) => Err(format!("{stem}: {e}")),
            }
        });
        let mut ok = Vec::new();
        for r in rendered {
            match r {
                Ok(v) => ok.push(v),
                Err(e) => say!("  ERROR {e}"),
            }
        }
        let maps = ok.iter().filter(|(l, _, _)| l.info.is_map()).count();
        let objects: usize = ok.iter().map(|(_, _, s)| s.objects).sum();
        let mut unknown: BTreeMap<u16, usize> = BTreeMap::new();
        let mut missing: BTreeSet<String> = BTreeSet::new();
        let mut missing_pal: BTreeSet<String> = BTreeSet::new();
        for (_, _, s) in &ok {
            for (k, v) in &s.unknown {
                *unknown.entry(*k).or_default() += v;
            }
            missing.extend(s.missing_sprites.iter().cloned());
            missing_pal.extend(s.missing_palette.iter().cloned());
        }
        total_levels += ok.len();
        total_unknown += unknown.values().sum::<usize>();
        say!(
            "world {id} ({:?}): {} levels ({} maps), {} objects placed, unknown tile ids: {} {:?}, missing sprites: {:?}, missing palettes: {:?}, themes {}/{}, world sprites {}, {:.2}s",
            world.data.name,
            ok.len(),
            maps,
            objects,
            unknown.values().sum::<usize>(),
            unknown
                .keys()
                .map(|k| format!("{k:#06x}"))
                .collect::<Vec<_>>(),
            missing,
            missing_pal,
            wthemes_ok,
            wthemes.len(),
            ctx.sprites.world.len(),
            tw.elapsed().as_secs_f32()
        );
        if args.each {
            let dir = args.out.join("levels").join(id);
            std::fs::create_dir_all(&dir).expect("create level dir");
            parallel_map(&ok, |(l, img, _)| {
                img.save_png(&dir.join(format!("{}.png", l.stem)))
                    .expect("write png")
            });
        }
        if ok.is_empty() {
            continue;
        }
        // Contact sheet.
        let f = CELL / args.tile;
        let thumbs = parallel_map(&ok, |(_, img, _)| downscale(img, f));
        let cw = thumbs.iter().map(|t| t.width).max().unwrap_or(1);
        let ch = thumbs.iter().map(|t| t.height).max().unwrap_or(1);
        let label_h = 14;
        let pad = 6;
        // Aim for a roughly 16:10 sheet.
        let (cell_w, cell_h) = ((cw + pad) as f32, (ch + label_h + pad) as f32);
        let cols = args
            .cols
            .unwrap_or_else(|| ((ok.len() as f32 * cell_h * 1.6 / cell_w).sqrt().ceil()) as u32)
            .max(1);
        let rows = (ok.len() as u32).div_ceil(cols);
        let mut sheet = RgbaImage::new(cols * (cw + pad) + pad, rows * (ch + label_h + pad) + pad);
        sheet
            .pixels
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .for_each(|p| p.copy_from_slice(&[30, 30, 34, 255]));
        for (i, ((l, _, _), t)) in ok.iter().zip(&thumbs).enumerate() {
            let (c, r) = (i as u32 % cols, i as u32 / cols);
            let x = pad + c * (cw + pad);
            let y = pad + r * (ch + label_h + pad);
            blit(&mut sheet, t, x as i32, y as i32, None);
            let max_chars = (cw / 8) as usize;
            let mut label = format!(
                "{} {}",
                l.stem.trim_end_matches("level"),
                l.info.general.name
            );
            label.truncate(
                label
                    .char_indices()
                    .nth(max_chars)
                    .map_or(label.len(), |(i, _)| i),
            );
            draw_text(&mut sheet, x, y + ch + 2, &label, 2, [230, 230, 230, 255]);
        }
        let path = args.out.join(format!("levels-sheet-{id}.png"));
        sheet.save_png(&path).expect("write sheet");
        say!(
            "  wrote {} ({}x{})",
            path.display(),
            sheet.width,
            sheet.height
        );
    }
    say!(
        "levels total: {total_levels}, unknown tile ids total: {total_unknown}, time {:.2}s",
        t0.elapsed().as_secs_f32()
    );

    if args.assets {
        let ta = Instant::now();
        match AssetsDat::load(&install.assets_dat()) {
            Ok(a) => {
                let imgs: Vec<_> = a.of_kind(AssetKind::Image).copied().collect();
                let decoded = parallel_map(&imgs, |e| a.image(e));
                let ok_n = decoded.iter().filter(|d| d.is_ok()).count();
                for d in decoded.iter().filter_map(|d| d.as_ref().err()).take(5) {
                    say!("  assets image error: {d}");
                }
                let sounds = a.of_kind(AssetKind::Sound).count();
                let ogg = a
                    .of_kind(AssetKind::Sound)
                    .filter(|e| a.sound(e).is_ok_and(|s| s.data.starts_with(b"OggS")))
                    .count();
                let shaders = a
                    .of_kind(AssetKind::Shader)
                    .filter(|e| a.shader(e).is_ok())
                    .count();
                say!(
                    "Assets.dat: {} entries, regions {:?}, images {ok_n}/{} decoded, sounds {sounds} ({ogg} Ogg), shaders {shaders}, {:.2}s",
                    a.entries.len(),
                    a.region_sizes,
                    imgs.len(),
                    ta.elapsed().as_secs_f32()
                );
                let box_ = 64u32;
                let cols = 48u32;
                let rows = (imgs.len() as u32).div_ceil(cols);
                let mut sheet = RgbaImage::new(cols * (box_ + 2), rows * (box_ + 2));
                sheet
                    .pixels
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .for_each(|p| p.copy_from_slice(&[60, 60, 90, 255]));
                for (i, d) in decoded.iter().enumerate() {
                    let (c, r) = (i as u32 % cols, i as u32 / cols);
                    if let Ok(img) = d
                        && img.image.width > 0
                        && img.image.height > 0
                    {
                        let t = fit(&img.image, box_);
                        blit(
                            &mut sheet,
                            &t,
                            (c * (box_ + 2)) as i32,
                            (r * (box_ + 2)) as i32,
                            None,
                        );
                    }
                }
                let p = args.out.join("assets-images.png");
                sheet.save_png(&p).expect("write assets sheet");
                say!("  wrote {}", p.display());
            }
            Err(e) => say!("Assets.dat: {e}"),
        }
    }
    std::fs::write(args.out.join("summary.txt"), summary).expect("write summary");
}
