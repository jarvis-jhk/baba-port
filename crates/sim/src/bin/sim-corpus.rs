//! Dev tool: replays every solution of a corpus TSV through the sim and
//! checks that it wins (on the turn the oracle verified, when known).
//!
//! ```text
//! sim-corpus [CORPUS.tsv] [VERIFIED.tsv] [--filter WORLD/LEVEL] [--verbose]
//! ```
//! Defaults: `ref/tas/corpus/corpus.tsv` and `verified.tsv` next to it.
//! Needs `$BABA_GAME_DIR`. Prints one line per failure and a summary.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use baba_formats::GameInstall;
use baba_sim::{Context, Input, Level, Source};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let verbose = args.iter().any(|a| a == "--verbose");
    let filter = args
        .iter()
        .position(|a| a == "--filter")
        .and_then(|i| args.get(i + 1).cloned());
    let pos: Vec<&String> = args
        .iter()
        .enumerate()
        .filter(|(i, a)| !a.starts_with("--") && (*i == 0 || args[i - 1] != "--filter"))
        .map(|(_, a)| a)
        .collect();
    let corpus = pos
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("ref/tas/corpus/corpus.tsv"));
    let verified = pos
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| corpus.with_file_name("verified.tsv"));
    let Some(Ok(install)) = GameInstall::from_env() else {
        eprintln!("sim-corpus: $BABA_GAME_DIR not set or not an install");
        std::process::exit(2);
    };
    let objects = install.load_objects().expect("objects");
    let text = std::fs::read_to_string(&corpus).expect("corpus");
    // (world, level, source, line) -> (status, expected event kind, turn);
    // the two files are line-aligned (same first five columns).
    type Ver = BTreeMap<(String, String, String, usize), (String, String, i64)>;
    let ver: Ver = std::fs::read_to_string(&verified)
        .map(|t| {
            t.lines()
                .skip(1)
                .enumerate()
                .filter_map(|(n, l)| {
                    let c: Vec<&str> = l.split('\t').collect();
                    if c.len() < 7 {
                        return None;
                    }
                    let (kind, turn) = if c[5] == "event" {
                        let first = c
                            .get(8)
                            .and_then(|e| e.split([',', ' ']).next())
                            .unwrap_or("");
                        let (k, t) = first.split_once('@').unwrap_or(("", "-1"));
                        (k.to_string(), t.parse().unwrap_or(-1))
                    } else {
                        ("win".to_string(), c[6].parse().unwrap_or(-1))
                    };
                    Some((
                        (c[0].to_string(), c[1].to_string(), c[4].to_string(), n),
                        (c[5].to_string(), kind, turn),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut worlds = BTreeMap::new();
    let (mut total, mut ok, mut fail, mut skipped) = (0, 0, 0, 0);
    for (n, line) in text.lines().skip(1).enumerate() {
        let c: Vec<&str> = line.split('\t').collect();
        if c.len() < 6 {
            continue;
        }
        let (world, stem, name, source, inputs) = (c[0], c[1], c[2], c[4], c[5]);
        if let Some(f) = &filter
            && !format!("{world}/{stem}").contains(f.as_str())
        {
            continue;
        }
        let (status, kind, win_turn) = ver
            .get(&(world.to_string(), stem.to_string(), source.to_string(), n))
            .cloned()
            .unwrap_or(("?".into(), "win".into(), -1));
        // win-seed: depends on the oracle's random stream; nowin: no outcome.
        if status != "win" && status != "event" && status != "?" {
            skipped += 1;
            continue;
        }
        total += 1;
        let w = worlds
            .entry(world.to_string())
            .or_insert_with(|| install.world(world).expect("world"));
        let data = match w.load_level(stem) {
            Ok(d) => d,
            Err(e) => {
                println!("FAIL {world}/{stem} ({name}): load error {e}");
                fail += 1;
                continue;
            }
        };
        let src = Arc::new(Source::from_level(&objects, &data, world));
        let mut lvl = Level::start(src, Context::default());
        let mut won_at = -1i64;
        let mut turn = 0i64;
        for ch in inputs.chars() {
            let Some(i) = Input::from_char(ch) else {
                continue;
            };
            turn += 1;
            let out = lvl.step(i);
            let hit = match kind.as_str() {
                "levelconversion" => out.transition.is_some(),
                "end" => out.ended,
                "allisdone" => out.all_done,
                _ => out.won,
            };
            if hit {
                won_at = turn;
                break;
            }
        }
        let good = won_at > 0 && (win_turn < 0 || won_at == win_turn);
        if good {
            ok += 1;
            if verbose {
                println!("ok   {world}/{stem} ({name}) {kind}@{won_at}");
            }
        } else {
            fail += 1;
            println!(
                "FAIL {world}/{stem} ({name}) [{source}]: {kind}@{won_at}, oracle {kind}@{win_turn}, {} inputs",
                inputs.len()
            );
        }
    }
    println!(
        "corpus: {ok}/{total} solutions reach the oracle's outcome on the same turn ({fail} fail, {skipped} skipped: win-seed/nowin)"
    );
    if fail > 0 {
        std::process::exit(1);
    }
}
