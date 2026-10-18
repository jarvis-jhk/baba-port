//! Scripted runs (`baba-port --drive SCRIPT`): the full app, headless on
//! an EGL surfaceless context or (`--window`, [`run_on`]) on the real
//! platform window, fed with synthetic key/pad/touch events at a fixed 60
//! ticks per second, with screenshots, comparisons against captures of
//! the original and checks of the state, the log and the saved files.
//!
//! Script language (one command per line, `#` comments):
//!
//! | command | effect |
//! |---|---|
//! | `size WxH` | screen size (default 854x480) |
//! | `wait N` | run N ticks |
//! | `idle [N]` | run until input is accepted (no transition/card/animation), at most N ticks (600) |
//! | `key NAME [N]` | press and release a key (N times); names: `up down left right space enter escape backspace` or a letter/digit |
//! | `hold NAME N` | hold a key for N ticks |
//! | `pad NAME [N]` | press and release a controller button (N times): `a b x y start select guide l r up down left right` |
//! | `tap FX FY` | touch down and up at (FX, FY) in 0..1 of the screen (menus by touch) |
//! | `moves STR` | turns in corpus notation `R U L D W Z r`, each followed by `idle` |
//! | `goto WORLD LEVEL [SLOT]` | starts WORLD in SLOT and jumps into LEVEL |
//! | `solve [VARIANT]` | plays the corpus solution of the current level (`$BABA_CORPUS` or `ref/tas/corpus/corpus.tsv`) |
//! | `walk STEM` | moves the map cursor to the icon of STEM (shortest path over open tiles) |
//! | `shot NAME` | renders and saves `OUT/NAME.png` |
//! | `compare NAME REF.png [THRESHOLD]` | compares shot NAME with a reference image |
//! | `expect state TEXT` | the state description contains TEXT |
//! | `expect log TEXT` / `expect sound NAME` / `expect music NAME` | the event / audio log contains it |
//! | `expect save SLOT GROUP KEY VALUE` | the slot file on disk has KEY=VALUE in [GROUP] (`SLOT` = `global` for `ba.ba`, `settings`; VALUE `-` = absent) |
//! | `print` | prints the state |
//! | `quit` | stops |
//!
//! Level editor (M8): `key ctrl+s` style combos, `type`, `click`, `cell`,
//! `paint`, `wheel`, `snap`, `expect file` / `expect snap` (see
//! `drive/ed.rs`).
//!
//! Multiplayer (M6): SDL virtual controllers (input subsystems only, no
//! window) are polled every tick (headless runs; with `--window` the
//! platform's own events are used), so their events reach the game in SDL
//! order like real pads. `pad` with a number first is the command below.
//!
//! | command | effect |
//! |---|---|
//! | `pads N` | plugs in N virtual controllers (numbered 1.. in plug order) |
//! | `unplug K` | unplugs virtual controller K |
//! | `pad K BUTTON [N]` | presses a button of controller K (N times): `a b x y back start lb rb up down left right` |
//! | `chord BUTTON K1 K2 ...` | presses BUTTON on several controllers in the same tick; checks that the turns were made in the order SDL delivered the presses |
//! | `pN moves STR` / `pN solve [VARIANT]` / `pN walk STEM` | like `moves`/`solve`/`walk`, for seat N (P1–P12) through its own device (controller or keyboard half) |
//! | `pN press ACTION [N]` | one action of seat N: `right up left down wait undo restart pause back confirm` |
//! | `pN idle [N]` | runs until seat N's view accepts input |
//! | `pN expect level WORLD/STEM` | seat N plays that level or map |
//! | `pN expect icon STEM STATE` | the icon of STEM on seat N's map has STATE |
//! | `expect same pA pB` / `expect differ pA pB` | the two seats' instances have equal / different state hashes |
//!
//! Online (M7; one process per machine against a Warpzone server, see
//! `scripts/net/online-test.sh`). While online, ticks run in real time
//! (60 per second) so the network keeps pace with the game. A line
//! `@TAG[,TAG] command` runs only on machines whose `DRIVE_TAGS`
//! (comma-separated, e.g. `m0,host`) contain one of the tags (`@!TAG`:
//! none of them). `lN ...` is `pN ...` for this machine's local player N.
//!
//! | command | effect |
//! |---|---|
//! | `online open` | the online menu (main menu → Online) |
//! | `online host private\|public FILE` | hosts a room; writes `room<TAB>code<TAB>invite` to FILE |
//! | `online join code\|invite\|room FILE [S] [rejected]` | waits (S seconds) for FILE, then joins by code / invite link (launcher path) / room id (`rejected`: expects the host to refuse us) |
//! | `online seats N` | plugs in N virtual controllers, joins them on the room's join screen and readies them |
//! | `online mode coop\|shared\|own` | host: the next session's mode |
//! | `online wait S TEXT` | waits up to S seconds until the state contains TEXT |
//! | `online wait-level S lN WORLD/STEM` / `online wait-icon S lN STEM STATE` | waits for a local player's view |
//! | `online settle [S]` | waits until everything is applied, nothing of ours is in flight and no entry arrived for 1.5 s |
//! | `online bots S [SEED]` | every local player presses random directions, waits and undos for S seconds |
//! | `online end` / `online end-if-host` | host: ends the session |
//! | `online leave` / `online crash` | leaves the room / vanishes without a goodbye |
//! | `online corrupt` | changes this machine's game state behind the stream's back (desync test) |
//! | `online expect agreed\|disagreed\|desync\|resynced\|host` | the last session ended with every machine agreeing, … |

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

use baba_platform::{Audio, Button, Event, Key, Platform, TouchPhase};
use baba_render::headless::{HeadlessApi, HeadlessContext};
use baba_render::{RgbaImage, diff, diff_image};

use crate::app::{App, AppConfig};

mod ed;
use crate::join::{Half, Source};
#[cfg(all(target_os = "linux", feature = "sdl"))]
use crate::vpads::{PadHub, button_number};

#[cfg(not(all(target_os = "linux", feature = "sdl")))]
struct PadHub;

#[cfg(not(all(target_os = "linux", feature = "sdl")))]
impl PadHub {
    fn new() -> Result<PadHub, String> {
        Err("built without SDL".into())
    }
    fn attach(&mut self) -> Result<(), String> {
        Err("built without SDL".into())
    }
    fn detach(&mut self, _: usize) -> Result<(), String> {
        Err("built without SDL".into())
    }
    fn set(&mut self, _: usize, _: u8, _: bool) -> Result<(), String> {
        Err("built without SDL".into())
    }
    fn set_by_id(&mut self, _: u32, _: u8, _: bool) -> Result<(), String> {
        Err("built without SDL".into())
    }
    fn poll(&mut self) -> Vec<Event> {
        Vec::new()
    }
    fn count(&self) -> usize {
        0
    }
}

#[cfg(not(all(target_os = "linux", feature = "sdl")))]
fn button_number(_: &str) -> Option<u8> {
    None
}

/// One action of a player, sent through the player's own device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PAct {
    Dir(u8),
    Wait,
    Undo,
    Restart,
    Pause,
    Back,
}

fn pact(name: &str) -> Option<PAct> {
    Some(match name {
        "right" | "R" => PAct::Dir(0),
        "up" | "U" => PAct::Dir(1),
        "left" | "L" => PAct::Dir(2),
        "down" | "D" => PAct::Dir(3),
        "wait" | "confirm" | "W" | "." => PAct::Wait,
        "undo" | "Z" => PAct::Undo,
        "restart" | "r" => PAct::Restart,
        "pause" => PAct::Pause,
        "back" => PAct::Back,
        _ => return None,
    })
}

/// Result of a run.
#[derive(Debug, Default)]
pub struct Report {
    pub failures: Vec<String>,
    pub shots: Vec<PathBuf>,
    pub notes: Vec<String>,
}

struct Driver<'a> {
    app: App,
    size: (u32, u32),
    out: PathBuf,
    audio: Audio,
    report: Report,
    corpus: Option<PathBuf>,
    pads: Option<PadHub>,
    /// Controller button-down events in delivery order (while `record`).
    delivered: Vec<u32>,
    record: bool,
    /// Window mode ([`run_on`]): the real platform (its events reach the
    /// app; frames are presented every `every` ticks).
    platform: Option<&'a mut dyn Platform>,
    every: u32,
    ticks: u64,
    /// Online: the wall-clock time of the next tick (real-time pacing).
    next: Option<std::time::Instant>,
    rng: u64,
}

/// Controller button names for `pad` (positional, SDL style).
fn parse_button(name: &str) -> Option<Button> {
    Some(match name.to_ascii_lowercase().as_str() {
        "a" | "south" => Button::South,
        "b" | "east" => Button::East,
        "x" | "west" => Button::West,
        "y" | "north" => Button::North,
        "start" => Button::Start,
        "select" | "back" => Button::Back,
        "guide" => Button::Guide,
        "l" | "lb" => Button::LeftShoulder,
        "r" | "rb" => Button::RightShoulder,
        "up" => Button::DPadUp,
        "down" => Button::DPadDown,
        "left" => Button::DPadLeft,
        "right" => Button::DPadRight,
        _ => return None,
    })
}

fn parse_key(name: &str) -> Option<Key> {
    Some(match name.to_ascii_lowercase().as_str() {
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        "space" => Key::Space,
        "enter" | "return" => Key::Enter,
        "escape" | "esc" => Key::Escape,
        "backspace" => Key::Backspace,
        "w" => Key::W,
        "a" => Key::A,
        "s" => Key::S,
        "d" => Key::D,
        "z" => Key::Z,
        "r" => Key::R,
        "q" => Key::Q,
        "e" => Key::E,
        "p" => Key::P,
        "m" => Key::M,
        "tab" => Key::Tab,
        "ctrl" => Key::Ctrl,
        "shift" => Key::Shift,
        "alt" => Key::Alt,
        "delete" => Key::Other(76),
        "minus" => Key::Other(45),
        f if f.len() >= 2 && f.starts_with('f') && f[1..].parse::<u8>().is_ok() => {
            Key::F(f[1..].parse().unwrap_or(1))
        }
        s if s.len() == 1 && s.as_bytes()[0].is_ascii_alphanumeric() => Key::Char(s.as_bytes()[0]),
        _ => return None,
    })
}

impl Driver<'_> {
    fn tick(&mut self, n: u32) {
        for _ in 0..n {
            self.ticks += 1;
            if self.app.online_active() {
                // Online: real time, so the network keeps pace.
                let step = std::time::Duration::from_micros(16_667);
                let now = std::time::Instant::now();
                let at = self.next.unwrap_or(now).max(now - step * 4);
                if at > now {
                    std::thread::sleep(at - now);
                }
                self.next = Some(at + step);
            } else {
                self.next = None;
            }
            // Headless: the virtual pads' SDL events; window mode: the
            // platform's (real devices, the window).
            let mut events = Vec::new();
            match self.platform.as_deref_mut() {
                Some(p) => p.poll_events(&mut events),
                None => {
                    if let Some(h) = &mut self.pads {
                        events = h.poll();
                    }
                }
            }
            for e in &events {
                if self.record
                    && let Event::Button {
                        device: baba_platform::DeviceId::Controller(id),
                        down: true,
                        ..
                    } = *e
                {
                    self.delivered.push(id);
                }
                self.app.handle_event(e);
            }
            self.app.tick();
            match self.platform.as_deref_mut() {
                None => {
                    self.app.audio.flush(Some(&mut self.audio));
                    let _ = self.audio.advance_offline(1.0 / 60.0);
                }
                Some(p) => {
                    self.app.audio.flush(p.audio());
                    if self.ticks.is_multiple_of(self.every as u64) {
                        self.app.render(p.drawable_size());
                        p.swap_buffers();
                    }
                }
            }
        }
    }

    fn pad(&mut self, b: Button, hold: u32) {
        let device = baba_platform::DeviceId::Controller(u32::MAX);
        for down in [true, false] {
            self.app.handle_event(&Event::Button {
                device,
                button: b,
                down,
            });
            self.tick(if down { hold.max(1) } else { 1 });
        }
    }

    /// A tap at (fx, fy) in 0..1 of the screen.
    fn tap(&mut self, fx: f32, fy: f32) {
        if self.platform.is_none() {
            // The app maps touches with the size of its last frame.
            self.app.render(self.size);
        }
        for phase in [TouchPhase::Down, TouchPhase::Up] {
            self.app.handle_event(&Event::Touch {
                finger: 1,
                phase,
                x: fx,
                y: fy,
            });
            self.tick(1);
        }
    }

    fn idle(&mut self, max: u32) -> bool {
        for _ in 0..max {
            if self.app.idle() {
                return true;
            }
            self.tick(1);
        }
        self.app.idle()
    }

    fn press(&mut self, k: Key, hold: u32) {
        self.app.handle_event(&Event::Key {
            key: k,
            down: true,
            repeat: false,
        });
        self.tick(hold.max(1));
        self.app.handle_event(&Event::Key {
            key: k,
            down: false,
            repeat: false,
        });
        self.tick(1);
    }

    fn fail(&mut self, msg: String) {
        println!("FAIL: {msg}");
        self.report.failures.push(msg);
    }

    fn shot(&mut self, name: &str) -> Option<RgbaImage> {
        if let Some(p) = self.platform.as_deref() {
            self.size = p.drawable_size();
        }
        self.app.render(self.size);
        let img = self.app.read_screen();
        let p = self.out.join(format!("{name}.png"));
        if let Err(e) = img.save_png(&p) {
            self.fail(format!("saving {}: {e}", p.display()));
            return None;
        }
        println!("shot {} ({})", p.display(), self.app.state());
        self.report.shots.push(p);
        Some(img)
    }

    fn moves(&mut self, s: &str) {
        self.moves_for(None, s)
    }

    fn player_idle(&mut self, seat: usize, max: u32) -> bool {
        for _ in 0..max {
            if self.app.player_idle(seat) {
                return true;
            }
            self.tick(1);
        }
        self.app.player_idle(seat)
    }

    /// One action through seat `seat`'s device.
    fn player_press(&mut self, seat: usize, a: PAct, hold: u32) {
        let src = self
            .app
            .seats()
            .into_iter()
            .find(|(id, _)| *id == seat)
            .and_then(|(_, s)| s);
        match src {
            Some(Source::Pad(id)) => {
                let b = match a {
                    PAct::Dir(0) => 14,
                    PAct::Dir(1) => 11,
                    PAct::Dir(2) => 13,
                    PAct::Dir(_) => 12,
                    PAct::Wait => 0,
                    PAct::Undo | PAct::Back => 1,
                    PAct::Restart => 10,
                    PAct::Pause => 6,
                };
                let Some(h) = &mut self.pads else {
                    self.fail("no virtual pads".into());
                    return;
                };
                if let Err(e) = h.set_by_id(id, b, true) {
                    self.fail(format!("pad: {e}"));
                    return;
                }
                self.tick(hold.max(1));
                if let Some(h) = &mut self.pads {
                    let _ = h.set_by_id(id, b, false);
                }
                self.tick(1);
            }
            Some(Source::Keys(half)) => {
                let left = half == Half::Left;
                let k = match a {
                    PAct::Dir(0) => [Key::D, Key::Right],
                    PAct::Dir(1) => [Key::W, Key::Up],
                    PAct::Dir(2) => [Key::A, Key::Left],
                    PAct::Dir(_) => [Key::S, Key::Down],
                    PAct::Wait => [Key::E, Key::Enter],
                    PAct::Undo => [Key::Q, Key::Backspace],
                    PAct::Restart => [Key::R, Key::Char(b'0')],
                    PAct::Pause => [Key::Tab, Key::P],
                    PAct::Back => [Key::Escape, Key::Backspace],
                }[if left { 0 } else { 1 }];
                self.press(k, hold);
            }
            None => self.fail(format!("p{}: no such seat or unplugged", seat + 1)),
        }
    }

    /// Corpus moves for the single player (`None`) or a seat.
    fn moves_for(&mut self, seat: Option<usize>, s: &str) {
        let start = self.app.player_level(seat);
        for (n, c) in s.chars().enumerate() {
            // An ending sequence took over (END / All Is Done).
            let st = self.app.state();
            if st.contains("cutscene=end") || st.contains("cutscene=done") {
                println!("moves: ending sequence, {} inputs left unused", s.len() - n);
                return;
            }
            let k = match c {
                'R' => Key::Right,
                'U' => Key::Up,
                'L' => Key::Left,
                'D' => Key::Down,
                'W' | '.' => Key::Space,
                'Z' => Key::Z,
                'r' => Key::R,
                _ => continue,
            };
            let ok = match seat {
                None => self.idle(900),
                Some(p) => self.player_idle(p, 900),
            };
            if !ok {
                self.fail(format!("not idle before move {c}: {}", self.app.state()));
                return;
            }
            if self.app.player_level(seat) != start {
                println!("moves: level changed, {} inputs left unused", s.len() - n);
                return;
            }
            match seat {
                None => self.press(k, 1),
                Some(p) => self.player_press(p, pact(&c.to_string()).unwrap_or(PAct::Wait), 1),
            }
            if c == 'r' {
                // Restart asks for confirmation unless disabled: answer Yes.
                if self.app.state().contains("RestartConfirm") {
                    match seat {
                        None => {
                            self.press(Key::Right, 1);
                            self.press(Key::Enter, 1);
                        }
                        Some(p) => {
                            self.player_press(p, PAct::Dir(0), 1);
                            self.player_press(p, PAct::Wait, 1);
                        }
                    }
                }
            }
        }
    }

    fn solution(&self, world: &str, level: &str, variant: &str) -> Option<String> {
        let path = self.corpus.as_ref()?;
        let text = std::fs::read_to_string(path).ok()?;
        let mut best: Option<String> = None;
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() >= 6 && f[0] == world && f[1] == level && f[3].eq_ignore_ascii_case(variant)
            {
                let s = f[5].to_string();
                if best.as_ref().is_none_or(|b| s.len() < b.len()) {
                    best = Some(s);
                }
            }
        }
        best
    }

    fn walk(&mut self, target: &str) {
        self.walk_for(None, target)
    }

    fn walk_for(&mut self, seat: Option<usize>, target: &str) {
        let Some(goal) = self.app.player_icon_pos(seat, target) else {
            self.fail(format!("walk: no icon for {target}"));
            return;
        };
        let Some((start, _)) = self.app.player_cursor(seat) else {
            self.fail("walk: no cursor".into());
            return;
        };
        let open: HashSet<(i32, i32)> = self.app.player_walkable(seat).into_iter().collect();
        let mut prev = std::collections::HashMap::new();
        let mut q = VecDeque::from([start]);
        let mut seen = HashSet::from([start]);
        while let Some(c) = q.pop_front() {
            if c == goal {
                break;
            }
            for (d, (dx, dy)) in [(0, (1, 0)), (1, (0, -1)), (2, (-1, 0)), (3, (0, 1))] {
                let n = (c.0 + dx, c.1 + dy);
                if open.contains(&n) && seen.insert(n) {
                    prev.insert(n, (c, d));
                    q.push_back(n);
                }
            }
        }
        if !seen.contains(&goal) {
            self.fail(format!(
                "walk: {target} at {goal:?} not reachable from {start:?}"
            ));
            return;
        }
        let mut path = Vec::new();
        let mut c = goal;
        while c != start {
            let (p, d) = prev[&c];
            path.push(d);
            c = p;
        }
        path.reverse();
        let s: String = path
            .iter()
            .map(|d| ['R', 'U', 'L', 'D'][*d as usize])
            .collect();
        println!("walk {target}: {s}");
        self.moves_for(seat, &s);
        match seat {
            None => self.idle(600),
            Some(p) => self.player_idle(p, 600),
        };
        match self.app.player_cursor(seat) {
            Some((_, Some(t))) if t == target => {}
            other => self.fail(format!("walk: cursor ended on {other:?}, wanted {target}")),
        }
    }

    fn expect_save(&mut self, args: &[&str]) {
        let [slot, group, key, value] = args else {
            self.fail("expect save SLOT GROUP KEY VALUE".into());
            return;
        };
        let Some(dir) = self.app.profile_dir().map(Path::to_path_buf) else {
            self.fail("expect save: in-memory profile".into());
            return;
        };
        let file = match *slot {
            "global" => "ba.ba".to_string(),
            "settings" => "SettingsC.txt".to_string(),
            n => format!("{n}ba.ba"),
        };
        let text = std::fs::read_to_string(dir.join(&file)).unwrap_or_default();
        let store = baba_progress::Store::parse(&text);
        let got = store.get(group, key).map(str::to_string);
        let want = (*value != "-").then_some(*value);
        if got.as_deref() != want {
            self.fail(format!(
                "{file} [{group}] {key}: expected {value}, got {got:?}"
            ));
        } else {
            println!("ok: {file} [{group}] {key}={value}");
        }
    }

    fn compare(&mut self, name: &str, reference: &str, threshold: u8) {
        let a = RgbaImage::load_png(self.out.join(format!("{name}.png")));
        let b = RgbaImage::load_png(reference);
        match (a, b) {
            (Ok(a), Ok(b)) if (a.width, a.height) == (b.width, b.height) => {
                let d = diff(&a, &b, threshold);
                let line = format!("compare {name} vs {reference}: {d}");
                println!("{line}");
                self.report.notes.push(line);
                let _ = diff_image(&a, &b, threshold)
                    .save_png(self.out.join(format!("{name}.diff.png")));
            }
            (Ok(a), Ok(b)) => self.fail(format!(
                "compare {name}: size {}x{} vs {}x{}",
                a.width, a.height, b.width, b.height
            )),
            (a, b) => self.fail(format!("compare {name}: {:?} / {:?}", a.err(), b.err())),
        }
    }

    /// Presses a button on several pads within one tick and checks the
    /// turn order against SDL's delivery order.
    fn chord(&mut self, b: u8, ks: &[usize]) {
        let before = self.app.log.len();
        self.delivered.clear();
        self.record = true;
        for k in ks {
            if let Some(h) = &mut self.pads
                && let Err(e) = h.set(*k, b, true)
            {
                self.fail(format!("chord: {e}"));
            }
        }
        self.tick(1);
        for k in ks {
            if let Some(h) = &mut self.pads {
                let _ = h.set(*k, b, false);
            }
        }
        self.tick(1);
        self.record = false;
        let seats = self.app.seats();
        let want: Vec<String> = self
            .delivered
            .iter()
            .filter_map(|inst| {
                seats
                    .iter()
                    .find(|(_, s)| *s == Some(Source::Pad(*inst)))
                    .map(|(id, _)| format!("p{}", id + 1))
            })
            .collect();
        let got: Vec<String> = self.app.log[before..]
            .iter()
            .filter_map(|l| l.strip_prefix("turn "))
            .filter_map(|l| l.split_whitespace().next())
            .map(str::to_string)
            .collect();
        if want.len() != ks.len() || want != got {
            self.fail(format!(
                "chord: SDL delivered {want:?}, turns made by {got:?}"
            ));
        } else {
            println!("ok: chord turns in SDL order {}", got.join(","));
        }
    }

    fn solve_for(&mut self, seat: Option<usize>, args: &[&str]) {
        let variant = if args.is_empty() {
            "-".to_string()
        } else {
            args.join(" ")
        };
        match self.app.player_level(seat) {
            Some((w, l, _)) => match self.solution(&w, &l, &variant) {
                Some(s) => {
                    println!("solve {w}/{l}: {s}");
                    self.moves_for(seat, &s);
                }
                None => self.fail(format!("no corpus solution for {w}/{l}")),
            },
            None => self.fail("solve: not playing".into()),
        }
    }

    /// `pN ...` lines.
    fn player_line(&mut self, seat: usize, args: &[&str]) {
        let num = |i: usize, d: u32| args.get(i).and_then(|v| v.parse().ok()).unwrap_or(d);
        match args.first().copied() {
            Some("moves") => self.moves_for(Some(seat), args.get(1).copied().unwrap_or("")),
            Some("solve") => self.solve_for(Some(seat), &args[1..]),
            Some("walk") => {
                if let Some(t) = args.get(1) {
                    self.walk_for(Some(seat), t);
                }
            }
            Some("press") => match args.get(1).and_then(|a| pact(a)) {
                Some(a) => {
                    for _ in 0..num(2, 1) {
                        self.player_press(seat, a, 1);
                    }
                }
                None => self.fail(format!("p{}: bad action {:?}", seat + 1, args.get(1))),
            },
            Some("idle") => {
                if !self.player_idle(seat, num(1, 600)) {
                    self.fail(format!("p{} still busy: {}", seat + 1, self.app.state()));
                }
            }
            Some("expect") => {
                let got = self.app.player_level(Some(seat));
                match args.get(1).copied() {
                    Some("level") => {
                        let want = args.get(2).copied().unwrap_or("");
                        let have = got.map(|(w, l, _)| format!("{w}/{l}")).unwrap_or_default();
                        if have != want {
                            self.fail(format!("p{} plays {have:?}, wanted {want}", seat + 1));
                        } else {
                            println!("ok: p{} in {want}", seat + 1);
                        }
                    }
                    Some("icon") => {
                        let stem = args.get(2).copied().unwrap_or("");
                        let want: i32 = num(3, 0) as i32;
                        let have = self.app.player_icon_state(Some(seat), stem);
                        if have != Some(want) {
                            self.fail(format!(
                                "p{} icon {stem}: {have:?}, wanted {want}",
                                seat + 1
                            ));
                        } else {
                            println!("ok: p{} icon {stem}={want}", seat + 1);
                        }
                    }
                    _ => self.fail(format!("bad p{} expect", seat + 1)),
                }
            }
            _ => self.fail(format!("bad p{} command {args:?}", seat + 1)),
        }
    }

    /// Ticks (in real time while online) until `f` holds or `secs` pass.
    fn wait_until(&mut self, secs: f64, mut f: impl FnMut(&App) -> bool) -> bool {
        let t0 = std::time::Instant::now();
        loop {
            if f(&self.app) {
                return true;
            }
            if t0.elapsed().as_secs_f64() > secs {
                return false;
            }
            self.tick(1);
            if !self.app.online_active() {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }

    fn bot_rand(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    /// `online ...` lines.
    fn online_line(&mut self, a: &[&str]) {
        let num = |i: usize, d: f64| a.get(i).and_then(|v| v.parse().ok()).unwrap_or(d);
        match a.first().copied() {
            Some("open") => self.app.open_online(None),
            Some("host") => {
                let public = a.get(1) == Some(&"public");
                let file = a.get(2).copied().unwrap_or("");
                self.app.open_online(None);
                if !self.wait_until(30.0, |app| app.online_signed_in()) {
                    self.fail(format!("online: not signed in: {}", self.app.state()));
                    return;
                }
                self.app.online_host(public);
                let ok = self.wait_until(30.0, |app| {
                    app.online_room()
                        .is_some_and(|r| r.1.is_some() || !r.2.is_empty())
                        && app.online_state().contains("online room")
                });
                match self.app.online_room() {
                    Some((room, code, invite)) if ok => {
                        let line = format!("{room}\t{}\t{invite}\n", code.unwrap_or_default());
                        println!("online: hosted {}", line.trim());
                        if !file.is_empty() {
                            let tmp = format!("{file}.tmp");
                            let r = std::fs::write(&tmp, &line)
                                .and_then(|_| std::fs::rename(&tmp, file));
                            if let Err(e) = r {
                                self.fail(format!("online: writing {file}: {e}"));
                            }
                        }
                    }
                    _ => self.fail(format!("online: hosting failed: {}", self.app.state())),
                }
            }
            Some("join") => {
                let how = a.get(1).copied().unwrap_or("code");
                let file = a.get(2).copied().unwrap_or("");
                let secs = num(3, 60.0);
                let mut text = String::new();
                let found = self.wait_until(secs, |_| {
                    text = std::fs::read_to_string(file).unwrap_or_default();
                    !text.trim().is_empty()
                });
                if !found {
                    self.fail(format!("online: no room file {file}"));
                    return;
                }
                let f: Vec<&str> = text.trim_end_matches('\n').split('\t').collect();
                let (room, code, invite) = (
                    f.first().copied().unwrap_or(""),
                    f.get(1).copied().unwrap_or(""),
                    f.get(2).copied().unwrap_or(""),
                );
                self.app.open_online(None);
                if !self.wait_until(30.0, |app| app.online_signed_in()) {
                    self.fail(format!("online: not signed in: {}", self.app.state()));
                    return;
                }
                match how {
                    "code" => self.app.online_join_code(code),
                    "invite" => self
                        .app
                        .launch_online(&["--warpzone-join".to_string(), invite.to_string()]),
                    _ => self.app.online_join_room(room),
                }
                if a.get(4) == Some(&"rejected") {
                    let ok = self.wait_until(60.0, |app| {
                        app.log.iter().any(|l| l.starts_with("online: rejected"))
                    });
                    if ok {
                        println!("ok: online rejected");
                    } else {
                        self.fail(format!("online: not rejected: {}", self.app.state()));
                    }
                    return;
                }
                let ok = self.wait_until(60.0, |app| {
                    let s = app.online_state();
                    (s.contains("online room") && !s.contains("me=-"))
                        || s.contains("online session")
                });
                if ok {
                    println!("online: joined ({how}) {}", self.app.online_state());
                } else {
                    self.fail(format!("online: joining failed: {}", self.app.state()));
                }
            }
            Some("seats") => {
                let n = num(1, 1.0) as usize;
                if n == 0 {
                    return;
                }
                if self.pads.is_none() {
                    match PadHub::new() {
                        Ok(h) => self.pads = Some(h),
                        Err(e) => {
                            self.fail(format!("pads: {e}"));
                            return;
                        }
                    }
                }
                let first = self.app.seats().len();
                let pad0 = self.pads.as_ref().map_or(0, |h| h.count());
                for _ in 0..n {
                    if let Some(Err(e)) = self.pads.as_mut().map(|h| h.attach()) {
                        self.fail(format!("pads: {e}"));
                    }
                }
                self.tick(2);
                // In the room: join, then ready. In a running session
                // (late join): join only; the host admits the players.
                let rounds = if self.app.online_state().contains("online room") {
                    2
                } else {
                    1
                };
                for round in 0..rounds {
                    for k in pad0..pad0 + n {
                        if let Some(h) = &mut self.pads {
                            let _ = h.set(k, 0, true);
                        }
                        self.tick(1);
                        if let Some(h) = &mut self.pads {
                            let _ = h.set(k, 0, false);
                        }
                        self.tick(1);
                    }
                    if round == 0 && !self.wait_until(20.0, |app| app.seats().len() >= first + n) {
                        self.fail(format!("online seats: {} seats", self.app.seats().len()));
                    }
                }
            }
            Some("mode") => {
                let m = match a.get(1).copied() {
                    Some("shared") => crate::session::MpMode::SharedSave,
                    Some("own") => crate::session::MpMode::OwnGames,
                    _ => crate::session::MpMode::Coop,
                };
                self.app.online_set_mode(m);
                self.tick(2);
            }
            Some("wait") => {
                let secs = num(1, 30.0);
                let text = a[2.min(a.len())..].join(" ");
                if self.wait_until(secs, |app| app.state().contains(&text)) {
                    println!("ok: online {text}");
                } else {
                    self.fail(format!("online wait {text:?}: {}", self.app.state()));
                }
            }
            Some(w @ ("wait-level" | "wait-icon")) => {
                let secs = num(1, 30.0);
                let k = a
                    .get(2)
                    .and_then(|l| l.strip_prefix('l'))
                    .and_then(|n| n.parse::<usize>().ok())
                    .unwrap_or(1)
                    .saturating_sub(1);
                let what = a.get(3).copied().unwrap_or("").to_string();
                let state: i32 = a.get(4).and_then(|v| v.parse().ok()).unwrap_or(3);
                let icon = w == "wait-icon";
                let ok = self.wait_until(secs, |app| {
                    let seat = app.local_seat(k);
                    if icon {
                        seat.is_some() && app.player_icon_state(seat, &what) == Some(state)
                    } else {
                        seat.is_some()
                            && app
                                .player_level(seat)
                                .is_some_and(|(w, l, _)| format!("{w}/{l}") == what)
                    }
                });
                if ok {
                    println!("ok: online {w} l{} {what}", k + 1);
                } else {
                    self.fail(format!(
                        "online {w} l{} {what}: {}",
                        k + 1,
                        self.app.state()
                    ));
                }
            }
            Some("settle") => {
                let secs = num(1, 60.0);
                let t0 = std::time::Instant::now();
                let mut last = (self.app.online_entries(), std::time::Instant::now());
                loop {
                    self.tick(1);
                    let e = self.app.online_entries();
                    if e != last.0 || !self.app.online_quiet() {
                        last = (e, std::time::Instant::now());
                    }
                    if last.1.elapsed().as_secs_f64() > 1.5 {
                        println!("ok: online settled at {e} entries");
                        break;
                    }
                    if t0.elapsed().as_secs_f64() > secs {
                        self.fail(format!("online settle: {}", self.app.state()));
                        break;
                    }
                }
            }
            Some("bots") => {
                let secs = num(1, 5.0);
                let seed = num(2, 0.0) as u64;
                let tags = std::env::var("DRIVE_TAGS").unwrap_or_default();
                self.rng = 0x9E37_79B9_7F4A_7C15
                    ^ seed
                    ^ tags
                        .bytes()
                        .fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64));
                let seats = self.app.local_seats();
                let mut next: Vec<u64> = seats.iter().map(|_| 0).collect();
                let t0 = std::time::Instant::now();
                let mut presses = 0u64;
                while t0.elapsed().as_secs_f64() < secs {
                    for (i, seat) in seats.iter().enumerate() {
                        if self.ticks < next[i] {
                            continue;
                        }
                        let r = self.bot_rand();
                        next[i] = self.ticks + 5 + r % 8;
                        let c = match (r >> 8) % 20 {
                            0..=13 => crate::input::Cmd::Dir(((r >> 16) % 4) as u8),
                            14..=16 => crate::input::Cmd::Undo,
                            _ => crate::input::Cmd::Idle,
                        };
                        self.app.online_push(*seat, c);
                        presses += 1;
                    }
                    self.tick(1);
                }
                println!("online bots: {presses} presses by {} players", seats.len());
            }
            Some("end") => {
                if !self.app.online_is_host() {
                    self.fail("online end: not the host".into());
                }
                self.app.online_end();
            }
            Some("end-if-host") => {
                if self.app.online_is_host() {
                    println!("online: ending as host");
                    self.app.online_end();
                }
            }
            Some("leave") => self.app.online_leave(),
            Some("crash") => self.app.online_crash(),
            Some("corrupt") => {
                if !self.app.online_corrupt() {
                    self.fail("online corrupt: nothing changed".into());
                }
            }
            Some("expect") => {
                let r = self.app.online_result();
                let (ok, what) = match a.get(1).copied() {
                    Some("agreed") => (r.as_ref().is_some_and(|r| r.agreed), "agreed"),
                    Some("disagreed") => (r.as_ref().is_some_and(|r| !r.agreed), "disagreed"),
                    Some("desync") => (self.app.online_desyncs().0 > 0, "desync seen"),
                    Some("resynced") => (self.app.online_desyncs().1 > 0, "resynced"),
                    Some("host") => (self.app.online_is_host(), "host"),
                    other => {
                        self.fail(format!("online expect {other:?}"));
                        return;
                    }
                };
                if ok {
                    println!("ok: online {what}");
                } else {
                    self.fail(format!("online expect {what}: {r:?} {}", self.app.state()));
                }
            }
            Some("print") => println!("online: {}", self.app.online_state()),
            Some("touch") => {
                if let Some(f) = a.get(1)
                    && let Err(e) = std::fs::write(f, "1\n")
                {
                    self.fail(format!("online touch {f}: {e}"));
                }
            }
            Some("wait-file") => {
                let f = a.get(1).copied().unwrap_or("").to_string();
                if !self.wait_until(num(2, 120.0), |_| std::path::Path::new(&f).exists()) {
                    self.fail(format!("online wait-file {f}"));
                }
            }
            other => self.fail(format!("bad online command {other:?}")),
        }
    }

    fn run_line(&mut self, line: &str) -> bool {
        let line = expand(line.split('#').next().unwrap_or("").trim());
        let line = line.as_str();
        if line.is_empty() {
            return true;
        }
        if let Some(rest) = line.strip_prefix('@') {
            let (tags, cmd) = rest.split_once(' ').unwrap_or((rest, ""));
            let have = std::env::var("DRIVE_TAGS").unwrap_or_default();
            let have: Vec<&str> = have.split(',').map(str::trim).collect();
            let (neg, tags) = match tags.strip_prefix('!') {
                Some(t) => (true, t),
                None => (false, tags),
            };
            let hit = tags.split(',').any(|t| have.contains(&t));
            if hit == neg {
                return true;
            }
            return self.run_line(cmd);
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        let num = |i: usize, d: u32| parts.get(i).and_then(|v| v.parse().ok()).unwrap_or(d);
        match parts[0] {
            "online" => self.online_line(&parts[1..]),
            l if l.starts_with('l') && l[1..].parse::<usize>().is_ok() => {
                let k = l[1..].parse::<usize>().unwrap_or(1).saturating_sub(1);
                match self.app.local_seat(k) {
                    Some(seat) => self.player_line(seat, &parts[1..]),
                    None => self.fail(format!("{l}: no such local player: {}", self.app.state())),
                }
            }
            // Window mode: the window's size wins.
            "size" if self.platform.is_some() => {}
            "size" => {
                if let Some((w, h)) = parts.get(1).and_then(|s| s.split_once('x')) {
                    self.size = (w.parse().unwrap_or(854), h.parse().unwrap_or(480));
                }
            }
            "wait" => self.tick(num(1, 1)),
            "idle" => {
                if !self.idle(num(1, 600)) {
                    self.fail(format!("still busy: {}", self.app.state()));
                }
            }
            "key" if parts.get(1).is_some_and(|k| k.len() > 1 && k.contains('+')) => {
                self.combo(parts[1], num(2, 1))
            }
            "key" => match parts.get(1).and_then(|k| parse_key(k)) {
                Some(k) => {
                    for _ in 0..num(2, 1) {
                        self.press(k, 1);
                    }
                }
                None => self.fail(format!("unknown key in {line}")),
            },
            "hold" => match parts.get(1).and_then(|k| parse_key(k)) {
                Some(k) => self.press(k, num(2, 30)),
                None => self.fail(format!("unknown key in {line}")),
            },
            // `pad NAME [N]` (one synthetic controller); `pad K BUTTON [N]`
            // (virtual controller K, below) when the first word is a number.
            "pad" if parts.get(1).is_some_and(|w| w.contains('+')) => {
                self.pad_combo(parts[1]);
            }
            "pad" if parts.get(1).is_some_and(|w| w.parse::<usize>().is_err()) => {
                match parts.get(1).and_then(|b| parse_button(b)) {
                    Some(b) => {
                        for _ in 0..num(2, 1) {
                            self.pad(b, 1);
                        }
                    }
                    None => self.fail(format!("unknown button in {line}")),
                }
            }
            "tap" => {
                let f = |i: usize| parts.get(i).and_then(|v| v.parse::<f32>().ok());
                match (f(1), f(2)) {
                    (Some(x), Some(y)) => self.tap(x, y),
                    _ => self.fail(format!("tap FX FY (0..1): {line}")),
                }
            }
            "moves" => self.moves(parts.get(1).copied().unwrap_or("")),
            "goto" => {
                let slot = num(3, 0) as usize;
                match (parts.get(1), parts.get(2)) {
                    (Some(w), Some(l)) => {
                        if let Err(e) = self.app.goto(slot, w, l) {
                            self.fail(format!("goto: {e}"));
                        }
                    }
                    _ => self.fail("goto WORLD LEVEL [SLOT]".into()),
                }
            }
            "solve" => self.solve_for(None, &parts[1..]),
            "pads" => {
                if self.pads.is_none() {
                    match PadHub::new() {
                        Ok(h) => self.pads = Some(h),
                        Err(e) => {
                            self.fail(format!("pads: {e}"));
                            return true;
                        }
                    }
                }
                for _ in 0..num(1, 1) {
                    if let Some(Err(e)) = self.pads.as_mut().map(|h| h.attach()) {
                        self.fail(format!("pads: {e}"));
                    }
                }
                self.tick(2);
            }
            "unplug" => {
                let k = num(1, 1) as usize;
                let r = match &mut self.pads {
                    Some(h) => h.detach(k.saturating_sub(1)),
                    None => Err("no pads".into()),
                };
                if let Err(e) = r {
                    self.fail(format!("unplug: {e}"));
                }
                self.tick(2);
            }
            "pad" => {
                let k = num(1, 1) as usize;
                let Some(b) = parts.get(2).and_then(|b| button_number(b)) else {
                    self.fail(format!("bad button in {line}"));
                    return true;
                };
                for _ in 0..num(3, 1) {
                    let r = match &mut self.pads {
                        Some(h) => h.set(k.saturating_sub(1), b, true),
                        None => Err("no pads".into()),
                    };
                    if let Err(e) = r {
                        self.fail(format!("pad: {e}"));
                        break;
                    }
                    self.tick(1);
                    if let Some(h) = &mut self.pads {
                        let _ = h.set(k.saturating_sub(1), b, false);
                    }
                    self.tick(1);
                }
            }
            "chord" => {
                let Some(b) = parts.get(1).and_then(|b| button_number(b)) else {
                    self.fail(format!("bad button in {line}"));
                    return true;
                };
                let ks: Vec<usize> = parts[2..]
                    .iter()
                    .filter_map(|k| k.parse::<usize>().ok())
                    .map(|k| k.saturating_sub(1))
                    .collect();
                self.chord(b, &ks);
            }
            p if p.starts_with('p') && p[1..].parse::<usize>().is_ok() => {
                let seat = p[1..].parse::<usize>().unwrap_or(1).saturating_sub(1);
                self.player_line(seat, &parts[1..]);
            }
            "walk" => {
                if let Some(t) = parts.get(1) {
                    self.walk(t);
                }
            }
            "shot" => {
                let _ = self.shot(parts.get(1).copied().unwrap_or("shot"));
            }
            "compare" => {
                if parts.len() >= 3 {
                    self.compare(parts[1], parts[2], num(3, 8) as u8);
                }
            }
            "expect" => {
                let rest = parts[2..].join(" ");
                match parts.get(1).copied() {
                    Some("state") => {
                        let s = self.app.state();
                        if !s.contains(&rest) {
                            self.fail(format!("state {s:?} lacks {rest:?}"));
                        } else {
                            println!("ok: state {s}");
                        }
                    }
                    Some("log") => {
                        if !self.app.log.iter().any(|l| l.contains(&rest)) {
                            self.fail(format!("log lacks {rest:?}: {:?}", self.app.log));
                        } else {
                            println!("ok: log {rest}");
                        }
                    }
                    Some(k @ ("sound" | "music")) => {
                        let want = format!("{k}:{rest}");
                        if !self.app.audio.log.contains(&want) {
                            self.fail(format!("audio log lacks {want}"));
                        } else {
                            println!("ok: {want}");
                        }
                    }
                    Some("save") => self.expect_save(&parts[2..]),
                    Some(k @ ("same" | "differ")) => {
                        let seat = |s: Option<&&str>| {
                            s.and_then(|s| s.strip_prefix('p'))
                                .and_then(|n| n.parse::<usize>().ok())
                                .map(|n| n.saturating_sub(1))
                        };
                        let a = self.app.player_hash(seat(parts.get(2)));
                        let b = self.app.player_hash(seat(parts.get(3)));
                        let same = a.is_some() && a == b;
                        if same != (k == "same") {
                            self.fail(format!("expect {k} {rest}: {a:?} vs {b:?}"));
                        } else {
                            println!("ok: {k} {rest}");
                        }
                    }
                    Some("snap" | "file" | "nofile") => self.editor_expect(&parts[1..]),
                    _ => self.fail(format!("bad expect: {line}")),
                }
            }
            c if ed::COMMANDS.contains(&c) => self.editor_line(&parts),
            "print" => println!("state: {}", self.app.state()),
            "quit" => {
                // Online: stay on the network a moment so our last messages
                // (the host's end report) reach everybody.
                if self.app.online_active() {
                    self.tick(120);
                }
                return false;
            }
            other => self.fail(format!("unknown command {other}")),
        }
        true
    }
}

/// Expands `${VAR}` from the environment.
fn expand(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("${") {
        out.push_str(&rest[..i]);
        match rest[i + 2..].find('}') {
            Some(j) => {
                let var = &rest[i + 2..i + 2 + j];
                out.push_str(&std::env::var(var).unwrap_or_default());
                rest = &rest[i + 3 + j..];
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Runs a script headless. `out` receives the screenshots.
pub fn run(
    script: &str,
    cfg: AppConfig,
    out: &Path,
    corpus: Option<PathBuf>,
) -> Result<Report, String> {
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let (_ctx, gl) = HeadlessContext::new(HeadlessApi::from_env())?;
    let mut app = App::new(gl, cfg)?;
    app.renderer()
        .use_offscreen_screen(854, 480)
        .map_err(|e| e.to_string())?;
    let d = Driver {
        app,
        size: (854, 480),
        out: out.to_path_buf(),
        audio: Audio::offline(None).map_err(|e| e.to_string())?,
        report: Report::default(),
        corpus,
        pads: None,
        delivered: Vec::new(),
        record: false,
        platform: None,
        every: 1,
        ticks: 0,
        next: None,
        rng: 1,
    };
    run_script(d, script)
}

/// Runs a script on a real platform window (SDL on desktop, PortMaster,
/// Wine): the platform's events reach the app too, frames are presented
/// every `every` ticks, `size` lines are ignored (the window's size
/// counts). Game time stays at a fixed 60 ticks per second.
pub fn run_on(
    platform: &mut dyn Platform,
    gl: baba_platform::glow::Context,
    every: u32,
    script: &str,
    cfg: AppConfig,
    out: &Path,
    corpus: Option<PathBuf>,
) -> Result<Report, String> {
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let app = App::new(gl, cfg)?;
    let d = Driver {
        app,
        size: platform.drawable_size(),
        out: out.to_path_buf(),
        audio: Audio::offline(None).map_err(|e| e.to_string())?,
        report: Report::default(),
        corpus,
        pads: None,
        delivered: Vec::new(),
        record: false,
        platform: Some(platform),
        every: every.max(1),
        ticks: 0,
        next: None,
        rng: 1,
    };
    run_script(d, script)
}

fn run_script(mut d: Driver<'_>, script: &str) -> Result<Report, String> {
    for (n, line) in script.lines().enumerate() {
        let sz = d.size;
        if !d.run_line(line) {
            break;
        }
        if d.size != sz && d.platform.is_none() {
            d.app
                .renderer()
                .use_offscreen_screen(d.size.0, d.size.1)
                .map_err(|e| e.to_string())?;
        }
        if d.report.failures.len() > 20 {
            println!("too many failures, stopping at line {}", n + 1);
            break;
        }
    }
    println!("log: {}", d.app.log.join(" | "));
    let sounds: Vec<&String> = d.app.audio.log.iter().collect();
    println!(
        "audio ({} requests): {:?}",
        sounds.len(),
        sounds.iter().take(60).collect::<Vec<_>>()
    );
    Ok(d.report)
}
