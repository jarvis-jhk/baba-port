//! Dev tool: writes every image of `Assets.dat` as `<out>/<index>.png` plus
//! `<out>/images.tsv` (index, image number, width, height, hotspot, action
//! point, flags), for identifying engine art (font glyphs, particles, logo).
//!
//! ```text
//! cargo run --release -p baba-formats --example assets_dump -- [--game DIR] [--out DIR]
//! ```
//!
//! The output is game art: keep it in `scratch/` (git-ignored).

use std::fmt::Write as _;
use std::path::PathBuf;

use baba_formats::{AssetKind, AssetsDat};

fn main() {
    let mut game = std::env::var_os("BABA_GAME_DIR").map(PathBuf::from);
    let mut out = PathBuf::from("scratch/m5/assets");
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--game" => game = it.next().map(PathBuf::from),
            "--out" => out = it.next().map(PathBuf::from).expect("--out DIR"),
            _ => panic!("unknown argument {a}"),
        }
    }
    let game = game.expect("--game DIR or BABA_GAME_DIR");
    let dat = AssetsDat::load(&game.join("Assets.dat")).expect("Assets.dat");
    std::fs::create_dir_all(&out).expect("out dir");
    let mut tsv = String::from("index\timage\tw\th\thx\thy\tax\tay\tflags\n");
    for (n, e) in dat.of_kind(AssetKind::Image).enumerate() {
        let img = dat.image(e).expect("image");
        let i = img.info;
        let _ = writeln!(
            tsv,
            "{}\t{n}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            e.index,
            i.width,
            i.height,
            i.hotspot.0,
            i.hotspot.1,
            i.action_point.0,
            i.action_point.1,
            i.flags
        );
        if img.image.width > 0 && img.image.height > 0 {
            img.image
                .save_png(&out.join(format!("{n:04}.png")))
                .expect("png");
        }
    }
    std::fs::write(out.join("images.tsv"), tsv).expect("tsv");
}
