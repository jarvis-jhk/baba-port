//! Drive commands for the level editor (M8).
//!
//! | command | effect |
//! |---|---|
//! | `key MOD+KEY [N]` | a key with modifiers held (`ctrl+s`, `shift+tab`, `ctrl+delete`) |
//! | `type TEXT` | types TEXT as key presses (letters, digits, space, `_ - . ,`; capitals with shift) |
//! | `click LX LY [BUTTON]` | mouse click at logical (854×480 canvas) coordinates; BUTTON 1 left (default), 2 middle, 3 right |
//! | `cell X Y [BUTTON]` | mouse click on level cell (X, Y) of the editor's level view |
//! | `paint X1 Y1 X2 Y2` | left drag from cell (X1, Y1) to cell (X2, Y2) through the cells between |
//! | `wheel DY` | mouse wheel |
//! | `touchcell X Y` | a finger down and up on level cell (X, Y) |
//! | `trigger L\|R` | pulls and releases the left / right trigger of the synthetic controller |
//! | `pad HELD+BUTTON` | holds controller button HELD while pressing BUTTON (editor alt combos: `pad select+a`) |
//! | `snap NAME` | copies the editor level's `.l` and `.ld` on disk to `OUT/NAME.l/.ld` |
//! | `expect snap NAME` | the editor level's files on disk equal snapshot NAME byte for byte |
//! | `expect file PATH TEXT` | PATH (relative to the data dir) exists and contains TEXT (`-` = just exists) |
//! | `expect nofile PATH` | PATH (relative to the data dir) does not exist |

use baba_platform::{Event, Key};

use super::{Driver, parse_key};
use crate::ui::Ui;

/// Commands handled here.
pub const COMMANDS: [&str; 8] = [
    "type",
    "click",
    "cell",
    "paint",
    "wheel",
    "snap",
    "touchcell",
    "trigger",
];

impl Driver<'_> {
    fn key_event(&mut self, key: Key, down: bool) {
        self.app.handle_event(&Event::Key {
            key,
            down,
            repeat: false,
        });
    }

    /// `ctrl+s`, `shift+ctrl+z`: modifiers down, key, modifiers up.
    pub(super) fn combo(&mut self, spec: &str, n: u32) {
        let parts: Vec<&str> = spec.split('+').collect();
        let keys: Vec<Key> = parts.iter().filter_map(|p| parse_key(p)).collect();
        if keys.len() != parts.len() || keys.is_empty() {
            self.fail(format!("bad key combo {spec}"));
            return;
        }
        let (mods, last) = keys.split_at(keys.len() - 1);
        for _ in 0..n.max(1) {
            for m in mods {
                self.key_event(*m, true);
            }
            self.press(last[0], 1);
            for m in mods.iter().rev() {
                self.key_event(*m, false);
            }
            self.tick(1);
        }
    }

    fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            let (key, shift) = match c {
                ' ' => (Key::Space, false),
                '_' => (Key::Other(45), true),
                '-' => (Key::Other(45), false),
                '.' => (Key::Other(55), false),
                ',' => (Key::Other(54), false),
                c if c.is_ascii_alphanumeric() => {
                    let l = c.to_ascii_lowercase();
                    let k = parse_key(&l.to_string()).unwrap_or(Key::Char(l as u8));
                    (k, c.is_ascii_uppercase())
                }
                _ => continue,
            };
            if shift {
                self.key_event(Key::Shift, true);
            }
            self.press(key, 1);
            if shift {
                self.key_event(Key::Shift, false);
            }
        }
        self.tick(1);
    }

    fn mouse(&mut self, px: f32, py: f32, button: u8, down: bool) {
        self.app.handle_event(&Event::MouseMove { x: px, y: py });
        self.app.handle_event(&Event::MouseButton {
            button,
            down,
            x: px,
            y: py,
        });
    }

    fn click_px(&mut self, px: f32, py: f32, button: u8) {
        // The app maps pointers with the size of its last frame.
        self.app.render(self.size);
        self.mouse(px, py, button, true);
        self.tick(1);
        self.mouse(px, py, button, false);
        self.tick(1);
    }

    fn cell_px(&mut self, x: i32, y: i32) -> Option<(f32, f32)> {
        self.app.render(self.size);
        let p = self.app.editor_cell_px(x, y);
        if p.is_none() {
            self.fail(format!("cell {x} {y}: no editor level view"));
        }
        p
    }

    /// `select+a`: HELD down, BUTTON pressed and released, HELD up.
    pub(super) fn pad_combo(&mut self, spec: &str) {
        let Some((held, b)) = spec.split_once('+') else {
            return;
        };
        let (Some(held), Some(b)) = (super::parse_button(held), super::parse_button(b)) else {
            self.fail(format!("bad pad combo {spec}"));
            return;
        };
        let device = baba_platform::DeviceId::Controller(u32::MAX);
        self.app.handle_event(&Event::Button {
            device,
            button: held,
            down: true,
        });
        self.tick(1);
        self.pad(b, 1);
        self.app.handle_event(&Event::Button {
            device,
            button: held,
            down: false,
        });
        self.tick(1);
    }

    fn data_path(&self, rel: &str) -> Option<std::path::PathBuf> {
        self.app.profile_dir().map(|d| d.join(rel))
    }

    pub(super) fn editor_line(&mut self, parts: &[&str]) {
        let num = |i: usize| parts.get(i).and_then(|v| v.parse::<f32>().ok());
        match parts[0] {
            "type" => {
                let line = parts[1..].join(" ");
                self.type_text(&line);
            }
            "click" => match (num(1), num(2)) {
                (Some(lx), Some(ly)) => {
                    let ui = Ui::new(self.size.0, self.size.1);
                    let b = num(3).unwrap_or(1.0) as u8;
                    self.click_px(ui.x(lx), ui.y(ly), b);
                }
                _ => self.fail("click LX LY [BUTTON]".into()),
            },
            "cell" => match (num(1), num(2)) {
                (Some(x), Some(y)) => {
                    let b = num(3).unwrap_or(1.0) as u8;
                    if let Some((px, py)) = self.cell_px(x as i32, y as i32) {
                        self.click_px(px, py, b);
                    }
                }
                _ => self.fail("cell X Y [BUTTON]".into()),
            },
            "paint" => match (num(1), num(2), num(3), num(4)) {
                (Some(x1), Some(y1), Some(x2), Some(y2)) => {
                    let (a, b) = ((x1 as i32, y1 as i32), (x2 as i32, y2 as i32));
                    let Some(p0) = self.cell_px(a.0, a.1) else {
                        return;
                    };
                    self.mouse(p0.0, p0.1, 1, true);
                    self.tick(1);
                    for (cx, cy) in crate::editor::tools::line(a, b).into_iter().skip(1) {
                        if let Some((px, py)) = self.app.editor_cell_px(cx, cy) {
                            self.app.handle_event(&Event::MouseMove { x: px, y: py });
                            self.tick(1);
                        }
                    }
                    if let Some((px, py)) = self.app.editor_cell_px(b.0, b.1) {
                        self.mouse(px, py, 1, false);
                    }
                    self.tick(1);
                }
                _ => self.fail("paint X1 Y1 X2 Y2".into()),
            },
            "touchcell" => match (num(1), num(2)) {
                (Some(x), Some(y)) => {
                    if let Some((px, py)) = self.cell_px(x as i32, y as i32) {
                        let (w, h) = (self.size.0 as f32, self.size.1 as f32);
                        for phase in [
                            baba_platform::TouchPhase::Down,
                            baba_platform::TouchPhase::Up,
                        ] {
                            self.app.handle_event(&Event::Touch {
                                finger: 1,
                                phase,
                                x: px / w,
                                y: py / h,
                            });
                            self.tick(1);
                        }
                    }
                }
                _ => self.fail("touchcell X Y".into()),
            },
            "trigger" => {
                let axis = if parts.get(1).is_some_and(|s| s.eq_ignore_ascii_case("l")) {
                    baba_platform::Axis::TriggerLeft
                } else {
                    baba_platform::Axis::TriggerRight
                };
                for value in [1.0, 0.0] {
                    self.app.handle_event(&Event::Axis {
                        device: baba_platform::DeviceId::Controller(u32::MAX),
                        axis,
                        value,
                    });
                    self.tick(1);
                }
            }
            "wheel" => {
                let dy = num(1).unwrap_or(1.0);
                self.app.handle_event(&Event::MouseWheel { dx: 0.0, dy });
                self.tick(1);
            }
            "snap" => {
                let name = parts.get(1).copied().unwrap_or("snap");
                let Some((dir, stem)) = self.app.editor_level_path() else {
                    self.fail("snap: no editor level".into());
                    return;
                };
                for ext in ["l", "ld"] {
                    let from = dir.join(format!("{stem}.{ext}"));
                    let to = self.out.join(format!("{name}.{ext}"));
                    if let Err(e) = std::fs::copy(&from, &to) {
                        self.fail(format!("snap {}: {e}", from.display()));
                    }
                }
                println!("snap {name}: {}/{stem}", dir.display());
            }
            other => self.fail(format!("unknown editor command {other}")),
        }
    }

    pub(super) fn editor_expect(&mut self, args: &[&str]) {
        match args.first().copied() {
            Some("snap") => {
                let name = args.get(1).copied().unwrap_or("snap");
                let Some((dir, stem)) = self.app.editor_level_path() else {
                    self.fail("expect snap: no editor level".into());
                    return;
                };
                for ext in ["l", "ld"] {
                    let a = std::fs::read(dir.join(format!("{stem}.{ext}")));
                    let b = std::fs::read(self.out.join(format!("{name}.{ext}")));
                    match (a, b) {
                        (Ok(a), Ok(b)) if a == b => {
                            println!("ok: {stem}.{ext} equals snapshot {name}")
                        }
                        (Ok(a), Ok(b)) => self.fail(format!(
                            "{stem}.{ext} differs from snapshot {name} ({} vs {} bytes)",
                            a.len(),
                            b.len()
                        )),
                        (a, b) => {
                            self.fail(format!("expect snap {name}: {:?} {:?}", a.err(), b.err()))
                        }
                    }
                }
            }
            Some("file") => {
                let (Some(rel), Some(_)) = (args.get(1), args.get(2)) else {
                    self.fail("expect file PATH TEXT".into());
                    return;
                };
                let text = args[2..].join(" ");
                let Some(p) = self.data_path(rel) else {
                    self.fail("expect file: in-memory profile".into());
                    return;
                };
                match std::fs::read(&p) {
                    Ok(b) if text == "-" || String::from_utf8_lossy(&b).contains(&text) => {
                        println!("ok: {} has {text}", p.display())
                    }
                    Ok(_) => self.fail(format!("{} lacks {text:?}", p.display())),
                    Err(e) => self.fail(format!("{}: {e}", p.display())),
                }
            }
            Some("nofile") => {
                let Some(p) = args.get(1).and_then(|r| self.data_path(r)) else {
                    self.fail("expect nofile PATH".into());
                    return;
                };
                if p.exists() {
                    self.fail(format!("{} exists", p.display()));
                } else {
                    println!("ok: no {}", p.display());
                }
            }
            _ => self.fail(format!("bad expect: {args:?}")),
        }
    }
}
