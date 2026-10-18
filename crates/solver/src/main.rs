//! `baba-solver`: dev tool that searches for level solutions with
//! `baba-sim`, and renders levels as text for solving by hand. Found
//! solutions are only candidates: they are verified in the private Lua
//! oracle (`scripts/dev/corpus_solved.py`) before they enter the corpus.
//!
//! ```text
//! baba-solver list  [--world W]... [--unsolved] [--stale]
//! baba-solver dump  WORLD LEVEL [INPUTS] [--face NAME,...] [--explain]
//! baba-solver solve WORLD LEVEL [--prefix INPUTS] [--goal G]... [--plan STAGES | --plan-file F]
//!                   [--strategy auto|bfs|greedy|bfws] [--time SECS] [--nodes N] [--mem MB]
//!                   [--dir|--nodir] [--nowait] [--nomacro] [--no-alive] [--best]
//! baba-solver batch [--world W]... [--levels W/L,...] [--threads N] [--time SECS] [--mem MB]
//!                   [--out FILE] [--all] [--stale]
//! baba-solver bench WORLD LEVEL [--time SECS [--sample]]
//! ```
//! Goals: `win` (default), `rule:baba is win`, `norule:wall is stop`,
//! `at:baba:5,6`, `notat:NAME|*:5,6`, `gone:skull`, `exists:baba`,
//! `count:text_is:6`, `pair:keke`, `near:baba:keke`, `face:belt:R`; several
//! `--goal`s must all hold. A plan is `;`-separated stages of `&`-joined
//! goals, each searched from the end of the previous one; a stage starting
//! with `&` inherits the previous stage's goals and `-goal` drops one.
//! Needs `$BABA_GAME_DIR` (and `$BABA_ROOT` for the corpus files; both set
//! by `env.sh`).

mod dump;
mod game;
mod search;
mod state;

use std::io::Write as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use baba_sim::Level;

use game::{Game, LevelRef, move_char, parse_inputs, root, solved_set};
use search::{Limits, Strategy, search};
use state::{Goal, KeyOpts};

struct Args {
    pos: Vec<String>,
    opts: Vec<(String, String)>,
    flags: Vec<String>,
}

const VALUED: &[&str] = &[
    "--world",
    "--prefix",
    "--goal",
    "--plan",
    "--plan-file",
    "--strategy",
    "--time",
    "--nodes",
    "--mem",
    "--threads",
    "--out",
    "--levels",
    "--face",
];

impl Args {
    fn parse(a: Vec<String>) -> Args {
        let (mut pos, mut opts, mut flags) = (vec![], vec![], vec![]);
        let mut it = a.into_iter();
        while let Some(x) = it.next() {
            if VALUED.contains(&x.as_str()) {
                opts.push((x, it.next().unwrap_or_default()));
            } else if x.starts_with("--") {
                flags.push(x);
            } else {
                pos.push(x);
            }
        }
        Args { pos, opts, flags }
    }
    fn get(&self, k: &str) -> Option<&str> {
        self.opts
            .iter()
            .rev()
            .find(|(a, _)| a == k)
            .map(|(_, v)| v.as_str())
    }
    fn all(&self, k: &str) -> Vec<&str> {
        self.opts
            .iter()
            .filter(|(a, _)| a == k)
            .map(|(_, v)| v.as_str())
            .collect()
    }
    fn flag(&self, k: &str) -> bool {
        self.flags.iter().any(|f| f == k)
    }
    fn num(&self, k: &str, d: f64) -> f64 {
        self.get(k).and_then(|v| v.parse().ok()).unwrap_or(d)
    }
}

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("baba-solver: {msg}");
    std::process::exit(2)
}

fn main() {
    let mut a: Vec<String> = std::env::args().skip(1).collect();
    if a.is_empty() {
        die("usage: baba-solver list|dump|solve|batch ... (see the crate docs)");
    }
    let cmd = a.remove(0);
    let args = Args::parse(a);
    let game = Game::open().unwrap_or_else(|e| die(e));
    match cmd.as_str() {
        "list" => list(&game, &args),
        "dump" => dump_cmd(&game, &args),
        "solve" => solve_cmd(&game, &args),
        "batch" => batch(game, &args),
        "bench" => bench(&game, &args),
        _ => die(format!("unknown command {cmd}")),
    }
}

/// Mechanics present as text that the sim (3D) or the oracle (AUTO, which
/// the game drives from a timer) do not cover.
fn uncovered(lvl: &Level) -> Vec<&'static str> {
    let d = lvl.defs();
    let mut notes = Vec::new();
    for u in lvl.unit_records() {
        let k = d.kind(u.kind);
        for (name, note) in [("text_3d", "3d"), ("text_auto", "auto")] {
            if k.is_text && k.name == name && !notes.contains(&note) {
                notes.push(note);
            }
        }
    }
    notes
}

fn worlds(args: &Args) -> Vec<String> {
    let w = args.all("--world");
    if w.is_empty() {
        vec!["baba".into(), "new_adv".into(), "museum".into()]
    } else {
        w.iter().map(|s| s.to_string()).collect()
    }
}

fn list(game: &Game, args: &Args) {
    let solved = solved_set(&root());
    for w in worlds(args) {
        let lv = game
            .playable(&w, args.flag("--stale"))
            .unwrap_or_else(|e| die(e));
        let n = lv.len();
        let mut done = 0;
        for l in &lv {
            let s = solved.contains(&(l.world.clone(), l.stem.clone()));
            done += s as usize;
            if args.flag("--unsolved") && s {
                continue;
            }
            let (lvl, _) = game.start(&w, &l.stem).unwrap_or_else(|e| die(e));
            let notes = uncovered(&lvl);
            println!(
                "{}\t{}\t{}\t{}x{}\t{} units\t{}\t{}",
                w,
                l.stem,
                l.name,
                lvl.width(),
                lvl.height(),
                lvl.unit_records().len(),
                if s { "solved" } else { "-" },
                notes.join(",")
            );
        }
        eprintln!("{w}: {done}/{n} solved");
    }
}

/// Replays inputs, reporting wins; returns the level after them.
fn replay(lvl: &mut Level, inputs: &str, verbose: bool) {
    for (t, i) in parse_inputs(inputs).into_iter().enumerate() {
        let out = lvl.step(i);
        if verbose && (out.won || out.ended || out.all_done || out.transition.is_some()) {
            println!(
                "turn {}: won {} ended {} all_done {} transition {:?}",
                t + 1,
                out.won,
                out.ended,
                out.all_done,
                out.transition
            );
        }
    }
}

fn dump_cmd(game: &Game, args: &Args) {
    let (w, l) = (args.pos.first(), args.pos.get(1));
    let (Some(w), Some(l)) = (w, l) else {
        die("dump WORLD LEVEL [INPUTS]")
    };
    let (mut lvl, name) = game.start(w, l).unwrap_or_else(|e| die(e));
    println!("{w}/{l} \"{name}\"");
    if let Some(inp) = args.pos.get(2) {
        replay(&mut lvl, inp, true);
        println!("after {} inputs", parse_inputs(inp).len());
    }
    print!("{}", dump::render(&lvl));
    if args.flag("--explain") {
        let h = state::heuristic(&lvl, &[Goal::Win], &mut Vec::new());
        println!("h(win) = {h}");
    }
    if let Some(f) = args.get("--face") {
        let names: Vec<String> = f.split(',').map(String::from).collect();
        println!("{}", dump::facings(&lvl, &names));
    }
}

fn limits(args: &Args, lvl: &Level, time: f64) -> Limits {
    let mut key = KeyOpts::for_level(lvl);
    if args.flag("--dir") {
        key.dir = true;
    }
    if args.flag("--nodir") {
        key.dir = false;
    }
    Limits {
        time: Duration::from_secs_f64(time),
        nodes: args.num("--nodes", 20_000_000.0) as usize,
        mem_bytes: (args.num("--mem", 1500.0) * 1e6) as usize,
        wait: !args.flag("--nowait"),
        macros: !args.flag("--nomacro"),
        alive: !args.flag("--no-alive"),
        key,
    }
}

/// Portfolio: (strategy, share of the time budget).
fn portfolio(s: &str) -> Vec<(Strategy, f64)> {
    match Strategy::parse(s) {
        Some(st) => vec![(st, 1.0)],
        None => vec![
            (Strategy::Bfs, 0.2),
            (Strategy::Bfws, 0.5),
            (Strategy::Greedy, 0.3),
        ],
    }
}

struct Outcome {
    inputs: String,
    won: bool,
    strategy: &'static str,
    log: String,
}

fn solve_level(lvl: &Level, goals: &[Goal], args: &Args, time: f64) -> Option<Outcome> {
    let mut log = String::new();
    for (st, share) in portfolio(args.get("--strategy").unwrap_or("auto")) {
        let mut lim = limits(args, lvl, time * share);
        let (mut found, mut stats) = search(lvl, goals, st, &lim);
        // Macro moves rest on a walking model that can be wrong; an
        // exhausted macro search is retried with single inputs.
        if found.is_none() && stats.stop == "exhausted" && lim.macros {
            let left = (time * share - stats.secs).max(1.0);
            lim.macros = false;
            lim.time = Duration::from_secs_f64(left);
            log += &format!("{}(macro): exhausted after {:.1}s; ", st.name(), stats.secs);
            (found, stats) = search(lvl, goals, st, &lim);
        }
        log += &format!(
            "{}: {} exp {} (macro {}) gen {} steps {} {:.1}s; ",
            st.name(),
            stats.stop,
            stats.expanded,
            stats.macro_expanded,
            stats.generated,
            stats.steps,
            stats.secs
        );
        if let Some(f) = found {
            return Some(Outcome {
                inputs: f.moves.iter().map(|&m| move_char(m)).collect(),
                won: f.won,
                strategy: st.name(),
                log,
            });
        }
        if args.flag("--best") && stats.best_h < i32::MAX {
            let b: String = stats.best.iter().map(|&m| move_char(m)).collect();
            log += &format!("best h {} at {b}; ", stats.best_h);
        }
    }
    eprintln!("{log}");
    None
}

fn solve_cmd(game: &Game, args: &Args) {
    let (Some(w), Some(l)) = (args.pos.first(), args.pos.get(1)) else {
        die("solve WORLD LEVEL")
    };
    let (mut lvl, name) = game.start(w, l).unwrap_or_else(|e| die(e));
    let mut prefix = args.get("--prefix").unwrap_or("").to_string();
    replay(&mut lvl, &prefix, false);
    let parse = |g: &str| Goal::parse(g.trim()).unwrap_or_else(|e| die(e));
    // Stages: `--plan "g1&g2;g3;win"` (each stage searched from the end of
    // the previous one), or the `--goal`s as one stage.
    // A stage starting with `&` inherits the previous stage's goals; a goal
    // written `-goal` removes that goal from the inherited set.
    let plan_text = match args.get("--plan-file") {
        Some(f) => std::fs::read_to_string(f)
            .unwrap_or_else(|e| die(e))
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join(";"),
        None => args.get("--plan").unwrap_or("").to_string(),
    };
    let mut stages: Vec<Vec<Goal>> = if plan_text.is_empty() {
        vec![args.all("--goal").iter().map(|g| parse(g)).collect()]
    } else {
        let mut prev: Vec<String> = Vec::new();
        let mut out = Vec::new();
        for s in plan_text.split(';') {
            let s = s.trim();
            let (inherit, body) = match s.strip_prefix('&') {
                Some(b) => (true, b),
                None => (false, s),
            };
            let mut cur: Vec<String> = if inherit { prev.clone() } else { Vec::new() };
            for g in body.split('&').map(str::trim).filter(|g| !g.is_empty()) {
                match g.strip_prefix('-') {
                    Some(r) => cur.retain(|x| x != r),
                    None => {
                        if !cur.iter().any(|x| x == g) {
                            cur.push(g.to_string());
                        }
                    }
                }
            }
            out.push(cur.iter().map(|g| parse(g)).collect());
            prev = cur;
        }
        out
    };
    if stages.last().is_none_or(|s| s.is_empty()) {
        stages.retain(|s| !s.is_empty());
        stages.push(vec![Goal::Win]);
    }
    let t = args.num("--time", 60.0);
    for (i, goals) in stages.iter().enumerate() {
        match solve_level(&lvl, goals, args, t) {
            Some(o) => {
                eprintln!(
                    "{}/{} \"{}\" stage {}: +{} {}{}",
                    w,
                    l,
                    name,
                    i + 1,
                    o.inputs.len(),
                    o.log,
                    if o.won { " WON" } else { "" }
                );
                replay(&mut lvl, &o.inputs, false);
                prefix += &o.inputs;
                if o.won {
                    break;
                }
            }
            None => {
                eprintln!("{w}/{l}: stage {} not found; reached so far:", i + 1);
                println!("{prefix}");
                std::process::exit(1);
            }
        }
    }
    println!("{prefix}");
}

/// Per-operation timings on one level (development aid).
fn bench(game: &Game, args: &Args) {
    let (Some(w), Some(l)) = (args.pos.first(), args.pos.get(1)) else {
        die("bench WORLD LEVEL")
    };
    let (lvl, _) = game.start(w, l).unwrap_or_else(|e| die(e));
    if let Some(secs) = args.get("--time").and_then(|s| s.parse::<f64>().ok()) {
        // Spin on clone+step for a profiler; with --sample, signal ourselves
        // (SIGUSR1 via kill(1)) every 20 ms so gdb can take stack samples.
        if args.flag("--sample") {
            let pid = std::process::id().to_string();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_millis(20));
                    let _ = std::process::Command::new("kill")
                        .args(["-USR1", &pid])
                        .status();
                }
            });
        }
        let t = Instant::now();
        let mut i = 0u8;
        let mut c = lvl.clone();
        while t.elapsed().as_secs_f64() < secs {
            if i.is_multiple_of(32) {
                c = lvl.clone();
            }
            c.step(search::input(i % 4));
            i = i.wrapping_add(1);
        }
        return;
    }
    let n = 2000;
    let ko = KeyOpts::for_level(&lvl);
    let time = |name: &str, f: &mut dyn FnMut()| {
        let t = Instant::now();
        for _ in 0..n {
            f();
        }
        println!(
            "{name:>12}: {:8.1} us",
            t.elapsed().as_secs_f64() * 1e6 / n as f64
        );
    };
    time("clone", &mut || {
        std::hint::black_box(lvl.clone());
    });
    let mut i = 0u8;
    time("clone+step", &mut || {
        let mut c = lvl.clone();
        c.step(search::input(i % 4));
        i += 1;
    });
    time("key", &mut || {
        std::hint::black_box(state::key(&lvl, &ko));
    });
    let mut hb = Vec::new();
    time("heuristic", &mut || {
        std::hint::black_box(state::heuristic(&lvl, &[Goal::Win], &mut hb));
    });
    time("walk_model", &mut || {
        std::hint::black_box(state::walk_model(&lvl).is_some());
    });
    time("unit_records", &mut || {
        std::hint::black_box(lvl.unit_records().len());
    });
}

fn batch(game: Game, args: &Args) {
    let solved = solved_set(&root());
    let mut todo: Vec<LevelRef> = Vec::new();
    let only: Vec<String> = args
        .get("--levels")
        .map(|s| s.split(',').map(String::from).collect())
        .unwrap_or_default();
    for w in worlds(args) {
        for l in game
            .playable(&w, args.flag("--stale"))
            .unwrap_or_else(|e| die(e))
        {
            if !only.is_empty() && !only.contains(&format!("{}/{}", l.world, l.stem)) {
                continue;
            }
            if args.flag("--all") || !solved.contains(&(l.world.clone(), l.stem.clone())) {
                // 3D (not in the sim) and AUTO (not in the oracle) levels
                // cannot produce verified solutions.
                if let Ok((lvl, _)) = game.start(&l.world, &l.stem)
                    && !uncovered(&lvl).is_empty()
                {
                    continue;
                }
                todo.push(l);
            }
        }
    }
    let out = args.get("--out").map(String::from).unwrap_or_else(|| {
        root()
            .join("scratch/solver/candidates.tsv")
            .display()
            .to_string()
    });
    if let Some(p) = std::path::Path::new(&out).parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let new = !std::path::Path::new(&out).exists();
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&out)
        .unwrap_or_else(|e| die(e));
    if new {
        let _ = writeln!(f, "world\tlevel\tname\tvariant\tsource\tinputs");
    }
    let file = Arc::new(Mutex::new(f));
    let total = todo.len();
    todo.reverse();
    let queue = Arc::new(Mutex::new(todo));
    let threads = args.num("--threads", 6.0) as usize;
    let time = args.num("--time", 60.0);
    let game = Arc::new(game);
    let t0 = Instant::now();
    let found = Arc::new(Mutex::new(0usize));
    std::thread::scope(|s| {
        for _ in 0..threads {
            let (queue, file, game, found) =
                (queue.clone(), file.clone(), game.clone(), found.clone());
            s.spawn(move || {
                loop {
                    let Some(l) = queue.lock().unwrap().pop() else {
                        break;
                    };
                    let Ok((lvl, _)) = game.start(&l.world, &l.stem) else {
                        continue;
                    };
                    let r = solve_level(&lvl, &[Goal::Win], args, time);
                    let line = match &r {
                        Some(o) => {
                            *found.lock().unwrap() += 1;
                            let _ = writeln!(
                                file.lock().unwrap(),
                                "{}\t{}\t{}\t-\tsolver:{}\t{}",
                                l.world,
                                l.stem,
                                l.name,
                                o.strategy,
                                o.inputs
                            );
                            format!("FOUND {} len {} [{}]", o.strategy, o.inputs.len(), o.log)
                        }
                        None => "none".to_string(),
                    };
                    eprintln!(
                        "[{:>6.0}s] {}/{} \"{}\": {}",
                        t0.elapsed().as_secs_f64(),
                        l.world,
                        l.stem,
                        l.name,
                        line
                    );
                }
            });
        }
    });
    eprintln!(
        "batch: {}/{} levels solved -> {out}",
        found.lock().unwrap(),
        total
    );
}
