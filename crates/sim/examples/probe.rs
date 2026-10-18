//! Dev probe: play a level and print state (`probe WORLD LEVEL INPUTS`).
use std::sync::Arc;

use baba_formats::GameInstall;
use baba_sim::{Context, Input, Level, Source};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let install = GameInstall::from_env().unwrap().unwrap();
    let objects = install.load_objects().unwrap();
    let data = install.world(&a[0]).unwrap().load_level(&a[1]).unwrap();
    let src = Arc::new(Source::from_level(&objects, &data, &a[0]));
    let mut lvl = Level::start(src, Context::default());
    let inputs = a.get(2).cloned().unwrap_or_default();
    for (i, c) in inputs.chars().enumerate() {
        let Some(inp) = Input::from_char(c) else {
            continue;
        };
        let out = lvl.step(inp);
        if out.won || i + 1 == inputs.len() {
            println!(
                "turn {} won {} events {:?}",
                i + 1,
                out.won,
                out.events.len()
            );
        }
    }
    let filter = a.get(3).cloned().unwrap_or_default();
    for u in lvl.unit_records() {
        let name = &lvl.defs().kinds[u.kind as usize].name;
        if filter.is_empty() || filter.split(',').any(|f| f == name) {
            println!("{name} {:?}", u);
        }
    }
    println!("{:?}", lvl.rule_strings());
}
