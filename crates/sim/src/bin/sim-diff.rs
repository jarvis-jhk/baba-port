//! Dev tool: replays a level + inputs through the sim and the reference
//! oracle (`docs/spec/oracle/oracle.py`, private) and reports the first
//! turn whose unit state or rule set differs.
//!
//! ```text
//! sim-diff WORLD LEVEL INPUTS [--oracle FILE.jsonl] [--all] [--rules] [--nodir]
//! ```
//! Without `--oracle` the oracle is run through `tools/venv/bin/python`
//! (or `python3`) from the project root. Units are compared as multisets of
//! (id, name, x, y, dir, float); rules as multisets of display strings
//! (`--rules` to compare them, off by default because hash-order
//! deviations reorder expansions). `--all` lists every differing turn.
//! The oracle runs with `--port-rng` (the sim's PRNG replaces the game's
//! rules randomness, and the generator state is compared every turn;
//! `--no-rng` skips that comparison, `--native-rng` uses the game's own
//! `math.random`). It also runs with `--sorted-pairs` (hash-order
//! iterations in ascending key order, the port's choice; `--hash-pairs`
//! keeps the oracle's fixed string-hash order). `--data DIR` runs the
//! oracle on another game `Data/` (scenario levels from `sim-scenario`;
//! point `$BABA_GAME_DIR` at the same install for the sim side).

use std::collections::BTreeMap;
use std::process::Command;
use std::sync::Arc;

use baba_formats::GameInstall;
use baba_sim::{Context, Input, Level, Source};

#[derive(Debug, Clone)]
enum J {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<J>),
    Obj(BTreeMap<String, J>),
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && (self.s[self.i] as char).is_whitespace() {
            self.i += 1;
        }
    }
    fn val(&mut self) -> J {
        self.ws();
        match self.s[self.i] {
            b'{' => {
                self.i += 1;
                let mut m = BTreeMap::new();
                loop {
                    self.ws();
                    if self.s[self.i] == b'}' {
                        self.i += 1;
                        break;
                    }
                    let J::Str(k) = self.val() else { panic!("key") };
                    self.ws();
                    self.i += 1; // :
                    let v = self.val();
                    m.insert(k, v);
                    self.ws();
                    if self.s[self.i] == b',' {
                        self.i += 1;
                    }
                }
                J::Obj(m)
            }
            b'[' => {
                self.i += 1;
                let mut v = Vec::new();
                loop {
                    self.ws();
                    if self.s[self.i] == b']' {
                        self.i += 1;
                        break;
                    }
                    v.push(self.val());
                    self.ws();
                    if self.s[self.i] == b',' {
                        self.i += 1;
                    }
                }
                J::Arr(v)
            }
            b'"' => {
                self.i += 1;
                let mut out = String::new();
                while self.s[self.i] != b'"' {
                    if self.s[self.i] == b'\\' {
                        self.i += 1;
                        match self.s[self.i] {
                            b'u' => {
                                let h =
                                    std::str::from_utf8(&self.s[self.i + 1..self.i + 5]).unwrap();
                                out.push(
                                    char::from_u32(u32::from_str_radix(h, 16).unwrap())
                                        .unwrap_or('?'),
                                );
                                self.i += 4;
                            }
                            b'n' => out.push('\n'),
                            c => out.push(c as char),
                        }
                    } else {
                        out.push(self.s[self.i] as char);
                    }
                    self.i += 1;
                }
                self.i += 1;
                J::Str(out)
            }
            b't' => {
                self.i += 4;
                J::Bool(true)
            }
            b'f' => {
                self.i += 5;
                J::Bool(false)
            }
            b'n' => {
                self.i += 4;
                J::Null
            }
            _ => {
                let st = self.i;
                while self.i < self.s.len() && b"+-0123456789.eE".contains(&self.s[self.i]) {
                    self.i += 1;
                }
                J::Num(
                    std::str::from_utf8(&self.s[st..self.i])
                        .unwrap()
                        .parse()
                        .unwrap(),
                )
            }
        }
    }
}

impl J {
    fn get(&self, k: &str) -> &J {
        match self {
            J::Obj(m) => m.get(k).unwrap_or(&J::Null),
            _ => &J::Null,
        }
    }
    fn arr(&self) -> &[J] {
        match self {
            J::Arr(v) => v,
            _ => &[],
        }
    }
    fn num(&self) -> i64 {
        match self {
            J::Num(n) => *n as i64,
            _ => 0,
        }
    }
    fn str(&self) -> String {
        match self {
            J::Str(s) => s.clone(),
            _ => String::new(),
        }
    }
    fn bool(&self) -> bool {
        matches!(self, J::Bool(true))
    }
}

type Key = (i64, String, i64, i64, i64, i64);

struct State {
    units: Vec<Key>,
    rules: Vec<String>,
    win: bool,
    rng: Option<String>,
    draws: i64,
    noplayer: bool,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let value = |f: &str| {
        args.iter()
            .position(|a| a == f)
            .and_then(|i| args.get(i + 1).cloned())
    };
    let oracle_file = value("--oracle");
    let data_dir = value("--data");
    let pos: Vec<&String> = args
        .iter()
        .enumerate()
        .filter(|(i, a)| {
            !a.starts_with("--")
                && (*i == 0 || (args[i - 1] != "--oracle" && args[i - 1] != "--data"))
        })
        .map(|(_, a)| a)
        .collect();
    if pos.len() < 2 {
        eprintln!("usage: sim-diff WORLD LEVEL [INPUTS] [--oracle FILE] [--all] [--rules]");
        std::process::exit(2);
    }
    let (world, stem) = (pos[0].as_str(), pos[1].as_str());
    let nodir = flag("--nodir");
    let inputs = pos.get(2).map(|s| s.as_str()).unwrap_or("");
    let text = match oracle_file {
        Some(f) => std::fs::read_to_string(f).expect("oracle file"),
        None => {
            let root = std::env::var("BABA_ROOT").unwrap_or_else(|_| ".".into());
            let py = format!("{root}/tools/venv/bin/python");
            let py = if std::path::Path::new(&py).exists() {
                py
            } else {
                "python3".into()
            };
            let mut cmd = Command::new(py);
            cmd.args([
                "-I",
                &format!("{root}/docs/spec/oracle/oracle.py"),
                world,
                stem,
                inputs,
                "--no-stop",
            ]);
            if !flag("--native-rng") {
                cmd.arg("--port-rng");
            }
            if !flag("--hash-pairs") {
                cmd.arg("--sorted-pairs");
            }
            if let Some(d) = &data_dir {
                cmd.args(["--data", d]);
            }
            let out = cmd.output().expect("run oracle");
            if !out.status.success() {
                eprintln!(
                    "sim-diff: oracle failed: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
                std::process::exit(3);
            }
            String::from_utf8_lossy(&out.stdout).into_owned()
        }
    };
    let oracle: Vec<State> = text
        .lines()
        .filter(|l| l.starts_with('{'))
        .map(|l| {
            let j = P {
                s: l.as_bytes(),
                i: 0,
            }
            .val();
            let mut units: Vec<Key> = j
                .get("units")
                .arr()
                .iter()
                .map(|u| {
                    (
                        u.get("id").num(),
                        u.get("name").str(),
                        u.get("x").num(),
                        u.get("y").num(),
                        if nodir { 0 } else { u.get("dir").num() },
                        u.get("float").num(),
                    )
                })
                .collect();
            units.sort();
            let mut rules: Vec<String> = j.get("rules").arr().iter().map(J::str).collect();
            rules.sort();
            let rng = match j.get("rng") {
                J::Str(s) => Some(s.clone()),
                _ => None,
            };
            State {
                units,
                rules,
                win: j.get("win").bool(),
                rng,
                draws: j.get("draws").num(),
                noplayer: j.get("noplayer").bool(),
            }
        })
        .collect();
    let Some(Ok(install)) = GameInstall::from_env() else {
        eprintln!("sim-diff: $BABA_GAME_DIR not set");
        std::process::exit(2);
    };
    let objects = install.load_objects().expect("objects");
    let w = install.world(world).expect("world");
    let data = w.load_level(stem).expect("level");
    let src = Arc::new(Source::from_level(&objects, &data, world));
    let mut lvl = Level::start(src, Context::default());
    let snap = |l: &Level, won: bool| {
        let mut units: Vec<Key> = l
            .units()
            .into_iter()
            .map(|u| {
                (
                    u.id as i64,
                    u.name,
                    u.x as i64,
                    u.y as i64,
                    if nodir { 0 } else { u.dir as i64 },
                    u.float as i64,
                )
            })
            .collect();
        units.sort();
        let mut rules = l.rule_strings();
        rules.sort();
        State {
            units,
            rules,
            win: won,
            rng: Some(format!("{:016x}", l.rng_state())),
            draws: 0,
            noplayer: l.no_player(),
        }
    };
    let mut sim = vec![snap(&lvl, false)];
    let chars: Vec<char> = inputs
        .chars()
        .filter(|c| Input::from_char(*c).is_some())
        .collect();
    // The oracle's win flag is sticky; mirror that.
    let mut won = false;
    for &c in &chars {
        let input = Input::from_char(c).unwrap();
        if input == Input::Restart {
            won = false;
        }
        let out = lvl.step(input);
        won |= out.won;
        sim.push(snap(&lvl, won));
    }
    let mut diffs = 0;
    for (t, (o, s)) in oracle.iter().zip(&sim).enumerate() {
        let mut msg = Vec::new();
        if o.units != s.units {
            let only_o: Vec<&Key> = o
                .units
                .iter()
                .filter(|k| !s.units.contains(k))
                .take(10)
                .collect();
            let only_s: Vec<&Key> = s
                .units
                .iter()
                .filter(|k| !o.units.contains(k))
                .take(10)
                .collect();
            msg.push(format!(
                "  oracle only: {only_o:?}\n  sim only:    {only_s:?}"
            ));
        }
        if flag("--rules") && o.rules != s.rules {
            let a: Vec<&String> = o.rules.iter().filter(|r| !s.rules.contains(r)).collect();
            let b: Vec<&String> = s.rules.iter().filter(|r| !o.rules.contains(r)).collect();
            msg.push(format!(
                "  rules oracle-sim: {a:?}\n  rules sim-oracle: {b:?}"
            ));
        }
        if let (Some(a), Some(b)) = (&o.rng, &s.rng)
            && a != b
            && !flag("--no-rng")
        {
            msg.push(format!(
                "  rng: oracle {a} sim {b} (oracle draws so far: {})",
                o.draws
            ));
        }
        if o.noplayer != s.noplayer {
            msg.push(format!(
                "  no player: oracle {} sim {}",
                o.noplayer, s.noplayer
            ));
        }
        if o.win != s.win {
            msg.push(format!("  win: oracle {} sim {}", o.win, s.win));
        }
        if !msg.is_empty() {
            diffs += 1;
            let inp = if t == 0 {
                "(start)".to_string()
            } else {
                chars[t - 1].to_string()
            };
            let before: String = chars[t.saturating_sub(13)..t.saturating_sub(1)]
                .iter()
                .collect();
            println!("turn {t} input {inp} (before: {before}):");
            for m in msg {
                println!("{m}");
            }
            if !flag("--all") {
                println!("  oracle rules: {:?}", o.rules);
                println!("  sim rules:    {:?}", s.rules);
                break;
            }
        }
    }
    if oracle.len() != sim.len() {
        println!(
            "note: oracle produced {} states, sim {}",
            oracle.len(),
            sim.len()
        );
    }
    if oracle.is_empty() {
        println!("{world}/{stem}: the oracle produced no states");
        std::process::exit(3);
    }
    if diffs == 0 {
        println!(
            "{world}/{stem}: no difference over {} states",
            oracle.len().min(sim.len())
        );
    } else {
        std::process::exit(1);
    }
}
