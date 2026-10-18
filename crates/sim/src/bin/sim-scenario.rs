//! Dev tool: turns a scenario description (ASCII grid + legend + inputs)
//! into a real level file pair (`.l` + `.ld`) in a scratch world, so the
//! same scenario can run through the sim and the private reference oracle
//! (`sim-diff --data`). Scenario files live in `crates/sim/scenarios/`.
//!
//! ```text
//! sim-scenario OUT_WORLD_DIR FILE...    writes <stem>.l/.ld per FILE and
//!                                       prints "<stem>\t<inputs>" lines
//! ```
//!
//! Scenario format (`#` comments, blank lines ignored outside grids):
//!
//! ```text
//! inputs: RRUWZ          corpus alphabet (R U L D W Z r A)
//! layer:                 a grid of one tile layer follows (several allowed)
//! B I Y . b              cells separated by spaces, `.` = empty
//! legend:
//! B text_baba            symbol, object name, optional facing (> ^ < v)
//! b baba v
//! ```
//! The grid is the level interior; the tool adds the one-cell border.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use baba_formats::level::Layer;
use baba_formats::{GameInstall, LevelMap};

struct Scenario {
    stem: String,
    inputs: String,
    layers: Vec<Vec<Vec<String>>>,
    legend: BTreeMap<String, (String, u8)>,
}

fn parse(path: &Path) -> Scenario {
    let text = std::fs::read_to_string(path).expect("scenario file");
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .expect("file name")
        .to_string();
    let mut sc = Scenario {
        stem,
        inputs: String::new(),
        layers: Vec::new(),
        legend: BTreeMap::new(),
    };
    #[derive(PartialEq)]
    enum Mode {
        Head,
        Grid,
        Legend,
    }
    let mut mode = Mode::Head;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if let Some(v) = line.strip_prefix("inputs:") {
            sc.inputs = v.trim().to_string();
            mode = Mode::Head;
        } else if line.trim() == "layer:" {
            sc.layers.push(Vec::new());
            mode = Mode::Grid;
        } else if line.trim() == "legend:" {
            mode = Mode::Legend;
        } else if mode == Mode::Grid {
            let row: Vec<String> = line.split_whitespace().map(str::to_string).collect();
            sc.layers.last_mut().expect("layer").push(row);
        } else if mode == Mode::Legend {
            let f: Vec<&str> = line.split_whitespace().collect();
            let dir = match f.get(2).copied() {
                Some("^") => 1,
                Some("<") => 2,
                Some("v") => 3,
                _ => 0,
            };
            sc.legend.insert(f[0].to_string(), (f[1].to_string(), dir));
        } else {
            panic!("{}: unexpected line {line:?}", sc.stem);
        }
    }
    sc
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: sim-scenario OUT_WORLD_DIR FILE...");
        std::process::exit(2);
    }
    let out = PathBuf::from(&args[0]);
    std::fs::create_dir_all(&out).expect("out dir");
    let Some(Ok(install)) = GameInstall::from_env() else {
        eprintln!("sim-scenario: $BABA_GAME_DIR not set");
        std::process::exit(2);
    };
    let base = install.load_objects().expect("objects");
    let catalogue = install.load_editor_objects().expect("editor objects");
    for f in &args[1..] {
        let sc = parse(Path::new(f));
        // Objects missing from the base table take over unused base slots
        // through `[tiles]` overrides, as levels made in the editor do.
        let used: Vec<&str> = sc.legend.values().map(|(n, _)| n.as_str()).collect();
        let mut free = base
            .iter()
            .filter(|o| o.id != "edge" && o.tile.is_some() && !used.contains(&o.name.as_str()))
            .map(|o| o.id.clone())
            .collect::<Vec<_>>()
            .into_iter()
            .rev();
        let mut changed = Vec::new();
        let mut tiles_ini = String::new();
        for name in &used {
            if base.by_name(name).is_some() || changed.iter().any(|(_, n)| n == name) {
                continue;
            }
            let e = catalogue
                .get(name)
                .unwrap_or_else(|| panic!("{}: unknown object {name}", sc.stem));
            let slot = free.next().expect("free slot");
            let mut kv = vec![
                ("name", e.name.clone()),
                ("image", e.sprite.clone().unwrap_or_else(|| e.name.clone())),
                ("unittype", e.unittype.clone()),
                ("type", e.word_type.to_string()),
                ("layer", e.layer.to_string()),
                ("tiling", e.tiling.to_i32().to_string()),
                ("colour", format!("{},{}", e.colour.x, e.colour.y)),
            ];
            if let Some(a) = e.active_colour {
                kv.push(("activecolour", format!("{},{}", a.x, a.y)));
            }
            if !e.argtype.is_empty() {
                let v: Vec<String> = e.argtype.iter().map(|a| a.to_string()).collect();
                kv.push(("argtype", v.join(",")));
            }
            if !e.argextra.is_empty() {
                kv.push(("argextra", e.argextra.join(",")));
            }
            if !e.customobjects.is_empty() {
                kv.push(("customobjects", e.customobjects.join(",")));
            }
            for (k, v) in kv {
                tiles_ini.push_str(&format!("{slot}_{k}={v}\n"));
            }
            changed.push((slot, name.to_string()));
        }
        let mut info_ini = format!(
            "[general]\nname={}\nlevels=0\npaths=0\nspecials=0\nunlockcount=0\n\n[tiles]\nchanged=",
            sc.stem
        );
        for (slot, _) in &changed {
            info_ini.push_str(slot);
            info_ini.push(',');
        }
        info_ini.push('\n');
        info_ini.push_str(&tiles_ini);
        let info = baba_formats::LevelInfo::parse(&info_ini);
        let objects = base.with_changes(&info.tiles);
        let rows = sc.layers.iter().map(|l| l.len()).max().unwrap_or(0);
        let cols = sc
            .layers
            .iter()
            .flat_map(|l| l.iter().map(|r| r.len()))
            .max()
            .unwrap_or(0);
        let (w, h) = (cols as u32 + 2, rows as u32 + 2);
        let mut map = LevelMap::new(w, h);
        map.layers.clear();
        for grid in &sc.layers {
            let mut layer = Layer::new(w, h);
            for (y, row) in grid.iter().enumerate() {
                for (x, cell) in row.iter().enumerate() {
                    if cell == "." {
                        continue;
                    }
                    let (name, dir) = sc
                        .legend
                        .get(cell)
                        .unwrap_or_else(|| panic!("{}: no legend for {cell}", sc.stem));
                    let def = objects
                        .by_name(name)
                        .unwrap_or_else(|| panic!("{}: unknown object {name}", sc.stem));
                    let tile = def
                        .tile
                        .unwrap_or_else(|| panic!("{}: {name} has no tile", sc.stem));
                    let i = ((y as u32 + 1) * w + x as u32 + 1) as usize;
                    layer.tiles[i] = tile;
                    if let Some(d) = layer.dirs.as_mut() {
                        d[i] = *dir;
                    }
                }
            }
            map.layers.push(layer);
        }
        std::fs::write(out.join(format!("{}.l", sc.stem)), map.to_bytes()).expect("write .l");
        std::fs::write(out.join(format!("{}.ld", sc.stem)), info_ini).expect("write .ld");
        println!("{}\t{}", sc.stem, sc.inputs);
    }
}
