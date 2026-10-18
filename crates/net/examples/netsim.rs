//! Multi-machine test runs of baba-net with the toy game and bots.
//!
//! ```sh
//! source env.sh; export WARPZONE_SERVER=http://127.0.0.1:3036   # scripts/net/dev-server.sh
//! netsim run --locals 4x3 --mode coop --secs 20             # one OS process per machine
//! netsim run --locals 2,4,6 --mode shared --spectators 1 --delay 40 --jitter 10 --loss 5
//! netsim run --locals 4x1 --mode own --leave 0@6 --relay    # the host leaves: migration
//! netsim memory --locals 12x1 --mode coop                    # one process, no server
//! ```
//!
//! Options: `--locals NxK | a,b,c` (machine 0 hosts), `--spectators N`,
//! `--mode coop|shared|own`, `--secs S` (the host ends the session),
//! `--delay MS --jitter MS --loss PCT` (link conditioner on every machine),
//! `--relay` (forced Warpzone relay), `--public` (listed room joined by id;
//! default private: odd machines type the code, even ones get
//! `--warpzone-join <invite link>`), `--leave M@S`, `--crash M@S`,
//! `--desync M@S`, `--late M@S` (repeatable; seconds after the session
//! start), `--abort-on-desync`, `--tag NAME`, `--verbose`.
//! Prints `SUMMARY PASS ...` or `SUMMARY FAIL ...`.

use baba_net::harness::{JoinHow, Peer, PeerResult, PeerStatus, RunSpec, run_memory};
use baba_net::{Mode, NetSession};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn usage() -> ! {
    eprintln!(
        "usage: netsim run|memory [options] | netsim peer --index I [options] [--code C | --launch ARG...]"
    );
    std::process::exit(2)
}

struct Opts {
    spec: RunSpec,
    tag: String,
    verbose: bool,
    /// Original option words (passed on to peers).
    words: Vec<String>,
}

fn parse_at(v: &str) -> (usize, f64) {
    let (m, s) = v.split_once('@').unwrap_or_else(|| usage());
    (
        m.parse().unwrap_or_else(|_| usage()),
        s.parse().unwrap_or_else(|_| usage()),
    )
}

fn parse(args: &[String]) -> (Opts, Vec<String>) {
    let mut locals = vec![1, 1];
    let mut spectators = 0;
    let mut spec = RunSpec::new(&[1, 1], Mode::Coop);
    spec.secs = 20.0;
    let mut tag = "run".to_string();
    let mut verbose = false;
    let mut words = Vec::new();
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = |words: &mut Vec<String>| {
            let v = it.next().cloned().unwrap_or_else(|| usage());
            words.push(a.clone());
            words.push(v.clone());
            v
        };
        match a.as_str() {
            "--locals" => {
                let v = val(&mut words);
                locals = if let Some((n, k)) = v.split_once('x') {
                    vec![k.parse().unwrap(); n.parse().unwrap()]
                } else {
                    v.split(',').map(|x| x.parse().unwrap()).collect()
                };
            }
            "--spectators" => spectators = val(&mut words).parse().unwrap(),
            "--mode" => {
                spec.mode = match val(&mut words).as_str() {
                    "coop" | "1" => Mode::Coop,
                    "shared" | "2" => Mode::SharedSave,
                    "own" | "3" => Mode::OwnGames,
                    _ => usage(),
                }
            }
            "--secs" => spec.secs = val(&mut words).parse().unwrap(),
            "--delay" => spec.link.delay_ms = val(&mut words).parse().unwrap(),
            "--jitter" => spec.link.jitter_ms = val(&mut words).parse().unwrap(),
            "--loss" => spec.link.loss = val(&mut words).parse::<f32>().unwrap() / 100.0,
            "--leave" => spec.leave.push(parse_at(&val(&mut words))),
            "--crash" => spec.crash.push(parse_at(&val(&mut words))),
            "--desync" => spec.desync.push(parse_at(&val(&mut words))),
            "--late" => spec.late.push(parse_at(&val(&mut words))),
            "--tag" => tag = val(&mut words),
            "--relay" => {
                spec.relay = true;
                words.push(a.clone());
            }
            "--public" => {
                spec.public = true;
                words.push(a.clone());
            }
            "--abort-on-desync" => {
                spec.abort_on_desync = true;
                words.push(a.clone());
            }
            "--verbose" => verbose = true,
            _ => rest.push(a.clone()),
        }
    }
    locals.extend(std::iter::repeat_n(0, spectators));
    spec.locals = locals;
    (
        Opts {
            spec,
            tag,
            verbose,
            words,
        },
        rest,
    )
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else { usage() };
    let (o, rest) = parse(&args[1..]);
    match cmd.as_str() {
        "memory" => memory(o),
        "run" => run(o),
        "peer" => peer(o, rest),
        _ => usage(),
    }
}

fn describe(o: &Opts) -> String {
    let s = &o.spec;
    format!(
        "{}: machines {:?} mode {:?} {}s link {:?}{}{}",
        o.tag,
        s.locals,
        s.mode,
        s.secs,
        s.link,
        if s.relay { " relay" } else { "" },
        if s.public { " public" } else { "" }
    )
}

fn summary(o: &Opts, res: &[PeerStatus], secs: f64) -> bool {
    let turns: u64 = res
        .iter()
        .filter_map(|r| match r {
            PeerStatus::Ended(r) => Some(r.turns),
            _ => None,
        })
        .sum();
    let migrations = res
        .iter()
        .filter_map(|r| match r {
            PeerStatus::Ended(r) => Some(r.migrations),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let desyncs = res
        .iter()
        .filter_map(|r| match r {
            PeerStatus::Ended(r) => Some(r.desyncs),
            _ => None,
        })
        .sum::<u32>();
    match o.spec.check(res) {
        Ok(s) => {
            println!(
                "SUMMARY PASS {} | {s}; migrations {migrations}, desync events {desyncs}, {secs:.1}s",
                o.tag
            );
            true
        }
        Err(e) => {
            println!(
                "SUMMARY FAIL {} | {e}; turns {turns}, migrations {migrations}, desync events {desyncs}",
                o.tag
            );
            for (i, r) in res.iter().enumerate() {
                println!("  m{i}: {r:?}");
            }
            false
        }
    }
}

fn memory(o: Opts) {
    println!("{}", describe(&o));
    let t = Instant::now();
    let res = run_memory(&o.spec, o.verbose);
    let ok = summary(&o, &res, t.elapsed().as_secs_f64());
    std::process::exit(if ok { 0 } else { 1 });
}

// ------------------------------------------------------------------ peer

fn home_name(home: &std::path::Path, index: usize) -> String {
    let f = home.join("netsim-name");
    if let Ok(n) = std::fs::read_to_string(&f) {
        return n.trim().to_string();
    }
    let r = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos()
        ^ std::process::id();
    let n = format!("bn{index}-{:06x}", r & 0xff_ffff);
    let _ = std::fs::create_dir_all(home);
    let _ = std::fs::write(&f, &n);
    n
}

fn result_line(index: usize, st: &PeerStatus) -> String {
    match st {
        PeerStatus::Ended(r) => format!(
            "RESULT {index} ended agreed={} desyncs={} final={:016x} applied={} migrations={} turns={} players={} aborted={}",
            r.agreed as u8,
            r.desyncs,
            r.final_hash,
            r.applied,
            r.migrations,
            r.turns,
            r.players,
            r.aborted.as_deref().unwrap_or("-").replace(' ', "_")
        ),
        PeerStatus::Left => format!("RESULT {index} left"),
        PeerStatus::Crashed => format!("RESULT {index} crashed"),
        PeerStatus::Failed(why) => format!("RESULT {index} failed {why}"),
        PeerStatus::Running => format!("RESULT {index} running"),
    }
}

fn parse_result(line: &str) -> Option<(usize, PeerStatus)> {
    let mut w = line.split_whitespace();
    if w.next()? != "RESULT" {
        return None;
    }
    let i: usize = w.next()?.parse().ok()?;
    let st = match w.next()? {
        "left" => PeerStatus::Left,
        "crashed" => PeerStatus::Crashed,
        "ended" => {
            let kv: std::collections::BTreeMap<&str, &str> =
                w.filter_map(|x| x.split_once('=')).collect();
            let num = |k: &str| kv.get(k).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
            PeerStatus::Ended(PeerResult {
                machine: None,
                agreed: num("agreed") == 1,
                desyncs: num("desyncs") as u32,
                final_hash: u64::from_str_radix(kv.get("final").unwrap_or(&"0"), 16).unwrap_or(0),
                applied: num("applied"),
                migrations: num("migrations") as u32,
                turns: num("turns"),
                players: num("players") as usize,
                aborted: kv
                    .get("aborted")
                    .filter(|a| **a != "-")
                    .map(|a| a.replace('_', " ")),
            })
        }
        _ => PeerStatus::Failed(w.collect::<Vec<_>>().join(" ")),
    };
    Some((i, st))
}

fn peer(o: Opts, rest: Vec<String>) {
    let mut index = None;
    let mut join = JoinHow::Host;
    let mut it = rest.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--index" => index = it.next().and_then(|v| v.parse().ok()),
            "--code" => join = JoinHow::Code(it.next().unwrap_or_else(|| usage())),
            "--launch" => join = JoinHow::Launch(it.by_ref().collect()),
            _ => usage(),
        }
    }
    let Some(index) = index else { usage() };
    let home = std::env::var_os("WARPZONE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| usage());
    let mut cfg = o.spec.net_config();
    cfg.auto_register = Some(home_name(&home, index));
    let net = match NetSession::new(cfg) {
        Ok(n) => n,
        Err(e) => {
            println!("RESULT {index} failed warpzone: {e}");
            std::process::exit(1);
        }
    };
    let plan = o.spec.plan(index, join);
    let mut p = Peer::new(net, plan);
    let mut said_room = false;
    let mut out = std::io::stdout();
    let mut last_stat = Instant::now();
    while p.running() {
        p.step();
        for l in p.log.drain(..) {
            let _ = writeln!(out, "{l}");
        }
        if !said_room && let Some((id, code, invite)) = p.room() {
            said_room = true;
            let _ = writeln!(out, "ROOM {id} {} {invite}", code.as_deref().unwrap_or("-"));
        }
        if last_stat.elapsed() > Duration::from_secs(5) {
            last_stat = Instant::now();
            let s = p.net.stats();
            let _ = writeln!(
                out,
                "[m{index}] stats: entries {} applied {} pending {} sent {} msgs / {} B, recv {} msgs, sim-lost {} peers {:?}",
                s.entries,
                s.applied,
                s.pending,
                s.traffic.sent_msgs,
                s.traffic.sent_bytes,
                s.traffic.recv_msgs,
                s.traffic.sim_lost,
                s.peers
                    .iter()
                    .map(|x| format!(
                        "{}:{}ms{}",
                        x.machine,
                        x.rtt_ms.unwrap_or(0),
                        if x.relayed == Some(true) { "R" } else { "" }
                    ))
                    .collect::<Vec<_>>()
            );
        }
        let _ = out.flush();
        std::thread::sleep(Duration::from_millis(2));
    }
    let st = p.status().clone();
    let _ = writeln!(out, "{}", result_line(index, &st));
    let _ = out.flush();
    if st == PeerStatus::Crashed {
        // No goodbye: the others must notice by time-outs.
        std::process::exit(0);
    }
    // Stay on the network a moment: our last messages (the host's `Ended`,
    // a graceful leave) still have to go out.
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(3) {
        p.step();
        if st == PeerStatus::Left {
            p.net.poll();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    std::process::exit(match st {
        PeerStatus::Failed(_) => 1,
        _ => 0,
    });
}

// ------------------------------------------------------------------ run

struct Proc {
    child: Child,
}

fn spawn(
    o: &Opts,
    index: usize,
    join: &[String],
    homes: &std::path::Path,
    tx: &mpsc::Sender<(usize, String)>,
) -> Proc {
    let exe = std::env::current_exe().unwrap();
    let home = homes.join(format!("m{index}"));
    std::fs::create_dir_all(&home).unwrap();
    let mut cmd = Command::new(exe);
    cmd.arg("peer")
        .args(&o.words)
        .args(["--index", &index.to_string()])
        .args(join)
        .env("WARPZONE_HOME", &home)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut child = cmd.spawn().expect("spawn peer");
    let stdout = child.stdout.take().unwrap();
    let tx = tx.clone();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send((index, line)).is_err() {
                break;
            }
        }
        let _ = tx.send((index, "EXIT".into()));
    });
    Proc { child }
}

fn run(o: Opts) {
    println!("{}", describe(&o));
    let server = std::env::var("WARPZONE_SERVER").unwrap_or_default();
    if server.is_empty() {
        eprintln!("set WARPZONE_SERVER (scripts/net/dev-server.sh: http://127.0.0.1:3036)");
        std::process::exit(2);
    }
    let root = std::env::var_os("BABA_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let homes = root.join("scratch/net/homes");
    let n = o.spec.locals.len();
    let (tx, rx) = mpsc::channel::<(usize, String)>();
    let t_start = Instant::now();
    let mut procs: Vec<Option<Proc>> = (0..n).map(|_| None).collect();
    procs[0] = Some(spawn(&o, 0, &[], &homes, &tx));
    let mut room: Option<(String, Option<String>, String)> = None;
    let mut t0: Option<Instant> = None;
    let mut results: Vec<Option<PeerStatus>> = vec![None; n];
    let mut exited = vec![false; n];
    let deadline = Duration::from_secs_f64(o.spec.secs + 180.0);
    let log_path = root.join(format!("scratch/net/{}.log", o.tag));
    let mut logf = std::fs::File::create(&log_path).ok();
    loop {
        while let Ok((i, line)) = rx.recv_timeout(Duration::from_millis(20)) {
            if let Some(f) = logf.as_mut() {
                let _ = writeln!(f, "{line}");
            }
            if o.verbose {
                println!("{line}");
            }
            if line == "EXIT" {
                exited[i] = true;
            } else if let Some(rest) = line.strip_prefix("ROOM ") {
                let w: Vec<&str> = rest.split_whitespace().collect();
                if room.is_none() && w.len() == 3 {
                    let code = (w[1] != "-").then(|| w[1].to_string());
                    room = Some((w[0].into(), code, w[2].into()));
                }
            } else if line.contains("started session") && t0.is_none() {
                t0 = Some(Instant::now());
            } else if let Some((idx, st)) = parse_result(&line) {
                results[idx] = Some(st);
            } else if !o.verbose
                && (line.contains("DESYNC")
                    || line.contains("FAIL")
                    || line.contains("host ")
                    || line.contains("ended:")
                    || line.contains("crash")
                    || line.contains("leaving"))
            {
                println!("{line}");
            }
        }
        if let Some((id, code, invite)) = &room {
            #[allow(clippy::needless_range_loop)]
            for m in 1..n {
                if procs[m].is_some() {
                    continue;
                }
                let due = match o.spec.late_at(m) {
                    None => true,
                    Some(t) => t0.is_some_and(|t0| t0.elapsed().as_secs_f64() >= t),
                };
                if due {
                    let join = match o.spec.join_for(m, id, code.as_deref(), invite) {
                        JoinHow::Host => vec![],
                        JoinHow::Code(c) => vec!["--code".into(), c],
                        JoinHow::Launch(a) => {
                            let mut v = vec!["--launch".to_string()];
                            v.extend(a);
                            v
                        }
                    };
                    procs[m] = Some(spawn(&o, m, &join, &homes, &tx));
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
        let all_started = procs.iter().all(|p| p.is_some());
        let all_exited = (0..n).all(|i| exited[i] || procs[i].is_none());
        if all_started && all_exited {
            break;
        }
        if exited[0] && room.is_none() {
            break;
        }
        if t_start.elapsed() > deadline {
            println!("deadline reached, stopping peers");
            break;
        }
    }
    // Stop what is still running (our own children, by handle).
    for p in procs.iter_mut().flatten() {
        if let Ok(None) = p.child.try_wait() {
            let _ = p.child.kill();
        }
        let _ = p.child.wait();
    }
    let res: Vec<PeerStatus> = results
        .into_iter()
        .map(|r| r.unwrap_or(PeerStatus::Failed("no result".into())))
        .collect();
    let ok = summary(&o, &res, t_start.elapsed().as_secs_f64());
    println!("log: {}", log_path.display());
    std::process::exit(if ok { 0 } else { 1 });
}
