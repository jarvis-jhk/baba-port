//! Integration tests against the player's install. They are skipped (pass
//! trivially, with a note on stderr) when `$BABA_GAME_DIR` is not set or does
//! not exist. They never write anything and print only counts.

use std::collections::BTreeSet;

use baba_formats::lua::LuaChunk;
use baba_formats::save::{SaveIndex, SaveSlot, Settings, default_save_dir};
use baba_formats::{
    AssetKind, AssetsDat, GameInstall, Language, LevelMap, Palette, SpriteSet, Theme, TileId,
};

fn install() -> Option<GameInstall> {
    match GameInstall::from_env() {
        Some(r) => Some(r.expect("BABA_GAME_DIR points at an install")),
        None => {
            eprintln!("skipping: $BABA_GAME_DIR not set or missing");
            None
        }
    }
}

#[test]
fn every_lua_file_tokenises() {
    let Some(g) = install() else { return };
    let mut n = 0;
    let mut stack = vec![g.data_dir()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "lua") {
                LuaChunk::load(&p).unwrap_or_else(|e| panic!("{e}"));
                n += 1;
            }
        }
    }
    assert!(n > 20, "only {n} lua files");
}

#[test]
fn object_tables() {
    let Some(g) = install() else { return };
    let objects = g.load_objects().unwrap();
    assert!(objects.len() > 100);
    assert_eq!(objects.by_tile(TileId::EDGE).unwrap().id, "edge");
    let tiles: BTreeSet<_> = objects.iter().filter_map(|o| o.tile).collect();
    assert_eq!(tiles.len(), objects.len(), "tile ids are unique");
    let editor = g.load_editor_objects().unwrap();
    assert!(editor.objects.len() > 300);
    let sprites = SpriteSet::load(&g, None).unwrap();
    for o in objects.iter().filter(|o| o.id != "edge") {
        assert!(
            sprites.contains(&o.sprite),
            "slot {} sprite {}",
            o.id,
            o.sprite
        );
    }
    for o in &editor.objects {
        let s = o.sprite.as_deref().unwrap_or(&o.name);
        assert!(sprites.contains(s), "editor object {} sprite {s}", o.name);
    }
}

#[test]
fn every_level_of_every_world() {
    let Some(g) = install() else { return };
    let base = g.load_objects().unwrap();
    let mut levels = 0;
    let mut dangling = Vec::new();
    for id in g.world_ids().unwrap() {
        let world = g.world(&id).unwrap();
        let sprites = SpriteSet::load(&g, Some(&world)).unwrap();
        for stem in &world.levels {
            let level = world.load_level(stem).unwrap();
            levels += 1;
            let m = &level.map;
            assert!(!m.layers.is_empty() && m.layers.len() <= 3, "{id}/{stem}");
            for l in &m.layers {
                assert_eq!((l.width, l.height), (m.width(), m.height()), "{id}/{stem}");
            }
            // Round trip through our writer.
            assert_eq!(&LevelMap::parse(&m.to_bytes()).unwrap(), m, "{id}/{stem}");
            // Every tile id resolves, every sprite exists.
            let objects = base.with_changes(&level.info.tiles);
            for l in &m.layers {
                for (x, y, t, _) in l.objects() {
                    let o = objects
                        .by_tile(t)
                        .unwrap_or_else(|| panic!("{id}/{stem} ({x},{y}) unknown tile {:#x}", t.0));
                    if t != TileId::EDGE {
                        assert!(
                            sprites.contains(&o.sprite),
                            "{id}/{stem}: sprite {} missing",
                            o.sprite
                        );
                    }
                }
            }
            let general = &level.info.general;
            let palette = if general.palette.is_empty() {
                "default.png"
            } else {
                &general.palette
            };
            assert!(
                g.palette_path(Some(&world), palette, general.palette_root)
                    .is_some(),
                "{id}/{stem}: palette {palette}"
            );
            for p in &level.info.paths {
                assert!(
                    objects.get(&p.object).is_some(),
                    "{id}/{stem}: path {}",
                    p.object
                );
            }
            for link in &level.info.levels {
                if !world.level_path(&link.file).is_file() {
                    dangling.push(format!("{id}/{stem} -> {}", link.file));
                }
            }
            for img in &level.info.images {
                assert!(
                    world.images_dir().join(format!("{img}_1.png")).is_file(),
                    "{id}/{stem}: image {img}"
                );
            }
        }
        if !world.data.start.is_empty() {
            assert!(world.level_path(&world.data.start).is_file(), "{id} start");
        }
    }
    assert!(levels > 500, "{levels} levels");
    // The debug world links to levels that are not shipped; the others are complete.
    dangling.retain(|d| !d.starts_with("debug/"));
    assert!(dangling.is_empty(), "dangling map links: {dangling:?}");
}

#[test]
fn palettes_themes_languages() {
    let Some(g) = install() else { return };
    let pals = g.palettes();
    assert!(!pals.is_empty());
    for p in &pals {
        let pal = Palette::load(p).unwrap();
        assert_eq!((pal.width, pal.height), (7, 5), "{}", p.display());
    }
    let mut themes = g.themes();
    for id in g.world_ids().unwrap() {
        themes.extend(g.world(&id).unwrap().themes());
    }
    for t in &themes {
        let theme = Theme::load(t).unwrap();
        assert!(!theme.name.is_empty(), "{}", t.display());
    }
    let langs = g.languages();
    assert!(langs.len() >= 10);
    for p in &langs {
        let l = Language::load(p).unwrap();
        assert!(
            l.texts().len() > 500,
            "{}: {} texts",
            p.display(),
            l.texts().len()
        );
        assert!(!l.name().is_empty());
    }
    assert!(!g.music().is_empty());
}

#[test]
fn assets_dat() {
    let Some(g) = install() else { return };
    let path = g.assets_dat();
    if !path.is_file() {
        eprintln!("skipping: no Assets.dat (not a Linux install)");
        return;
    }
    let a = AssetsDat::load(&path).unwrap();
    let mut images = 0;
    for e in a.of_kind(AssetKind::Image) {
        let img = a.image(e).unwrap();
        assert_eq!(
            img.image.pixels.len(),
            img.image.width as usize * img.image.height as usize * 4
        );
        images += 1;
    }
    assert!(images > 1000);
    for e in a.of_kind(AssetKind::Sound) {
        assert!(a.sound(e).unwrap().data.starts_with(b"OggS"));
    }
    for e in a.of_kind(AssetKind::Shader) {
        let s = a.shader(e).unwrap();
        match s.language {
            baba_formats::ShaderLanguage::Glsl => {
                let (v, f) = (s.vertex_text().unwrap(), s.fragment_text().unwrap());
                assert!(v.starts_with("#version") && f.starts_with("#version"));
                assert!(v.contains("void main") && f.contains("void main"));
            }
            baba_formats::ShaderLanguage::D3d9 => {
                assert_eq!(&s.vertex[2..4], &[0xfe, 0xff]);
                assert_eq!(&s.fragment[2..4], &[0xff, 0xff]);
            }
        }
    }
}

#[test]
fn original_saves_read_only() {
    let Some(_) = install() else { return };
    let Some(dir) = default_save_dir().filter(|d| d.is_dir()) else {
        eprintln!("skipping: no save directory");
        return;
    };
    for n in 0..3 {
        let p = dir.join(format!("{n}ba.ba"));
        if p.is_file() {
            let s = SaveSlot::load(&p).unwrap();
            for w in s.worlds() {
                let _ = s.levels(w);
                let _ = s.map_cells(w);
            }
        }
    }
    let p = dir.join("ba.ba");
    if p.is_file() {
        SaveIndex::load(&p).unwrap();
    }
    let p = dir.join("SettingsC.txt");
    if p.is_file() {
        Settings::load(&p).unwrap();
    }
}
