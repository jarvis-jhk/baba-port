//! Input: platform events → game commands through the player's bindings
//! (`docs/spec/ui.md` §6, `saves.md` §4.3/§4.4), with key repeat while a
//! direction or undo is held (`delay` frames between repeats).
//!
//! Keyboard bindings are stored as SDL keycodes like the original. Our
//! platform layer reports physical keys, so they are converted with the
//! US layout (letters, digits, named keys; other scancodes use SDL's
//! "scancode | 1<<30" rule). Gamepad bindings use the original's value
//! format with our own positional button numbering (see [`pad_index`]).

use baba_platform::{Axis, Button, Event, Key};
use baba_progress::{Action, PadAction, Settings};

/// A command for the current screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    /// 0 right, 1 up, 2 left, 3 down.
    Dir(u8),
    Idle,
    Undo,
    Restart,
    Confirm,
    Pause,
    /// Escape / Android back: pause in game, back in menus.
    Back,
}

/// SDL keycode of a physical key (US layout).
pub fn keycode(k: Key) -> i64 {
    const S: i64 = 1 << 30;
    match k {
        Key::Up => 1073741906,
        Key::Down => 1073741905,
        Key::Left => 1073741904,
        Key::Right => 1073741903,
        Key::W => 'w' as i64,
        Key::A => 'a' as i64,
        Key::S => 's' as i64,
        Key::D => 'd' as i64,
        Key::Z => 'z' as i64,
        Key::R => 'r' as i64,
        Key::Q => 'q' as i64,
        Key::E => 'e' as i64,
        Key::P => 'p' as i64,
        Key::M => 'm' as i64,
        Key::Space => 32,
        Key::Enter => 13,
        Key::Escape => 27,
        Key::Backspace => 8,
        Key::Tab => 9,
        Key::Ctrl => S | 224,
        Key::Shift => S | 225,
        Key::Alt => S | 226,
        Key::Char(c) => c.to_ascii_lowercase() as i64,
        Key::F(n) => S | (57 + n as i64),
        Key::Back => S | 270,
        Key::Menu => S | 118,
        Key::Other(sc) => S | sc as i64,
    }
}

/// Readable name of a keycode (key-cap text in the keyboard setup).
pub fn key_name(code: i64) -> String {
    match code {
        1073741906 => "up".into(),
        1073741905 => "down".into(),
        1073741904 => "left".into(),
        1073741903 => "right".into(),
        32 => "space".into(),
        13 => "return".into(),
        27 | -1 => "esc".into(),
        8 => "backspace".into(),
        9 => "tab".into(),
        c if (33..127).contains(&c) => (c as u8 as char).to_string(),
        c => format!("key {}", c & 0xffff),
    }
}

/// Positional controller buttons as raw indices (our numbering, chosen so
/// the original's default values mean A/B/X/Y, shoulders and Start).
pub fn pad_index(b: Button) -> i32 {
    match b {
        Button::South => 0,
        Button::East => 1,
        Button::West => 2,
        Button::North => 3,
        Button::LeftShoulder => 4,
        Button::RightShoulder => 5,
        Button::Back => 6,
        Button::Guide => 7,
        Button::LeftStick => 8,
        Button::Start => 9,
        Button::RightStick => 10,
        Button::DPadUp => 11,
        Button::DPadDown => 12,
        Button::DPadLeft => 13,
        Button::DPadRight => 14,
        Button::Misc => 15,
        Button::Other(n) => 16 + n as i32,
    }
}

pub fn pad_name(v: &str) -> String {
    match v {
        "a0" => "left stick".into(),
        "a1" => "right stick".into(),
        "dpad" => "d-pad".into(),
        "0" => "a".into(),
        "1" => "b".into(),
        "2" => "x".into(),
        "3" => "y".into(),
        "4" => "lb".into(),
        "5" => "rb".into(),
        "6" => "back".into(),
        "7" => "guide".into(),
        "8" => "ls".into(),
        "9" => "start".into(),
        "10" => "rs".into(),
        v => format!("button {v}"),
    }
}

/// What a rebinding screen waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capture {
    Key(Action, bool),
    Pad(PadAction, bool),
}

/// Result of feeding an event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Captured {
    Key(i64),
    Pad(String),
}

/// Held-key state and repeat timers.
#[derive(Default)]
pub struct Input {
    /// Held directions, most recent last, with their source id.
    held_dirs: Vec<(u8, u32)>,
    dir_timer: i64,
    undo_held: u32,
    undo_timer: i64,
    /// Commands produced since the last `take`.
    queue: Vec<Cmd>,
    /// Raw characters typed (text entry).
    pub typed: Vec<char>,
    /// A text prompt is open (set by the app before each event): keys
    /// type (Enter as `'\n'`) instead of running their bound actions;
    /// arrows still move and Escape / Back still go back.
    pub text_entry: bool,
    shift_held: bool,
    pub capture: Option<Capture>,
    pub captured: Option<Captured>,
    stick: [i8; 2],
    /// Any key or button is down (hold-to-skip).
    pub any_down: u32,
}

/// Source ids for held tracking.
fn key_src(k: Key) -> u32 {
    (keycode(k) & 0xffff_ffff) as u32
}

impl Input {
    pub fn take(&mut self) -> Vec<Cmd> {
        std::mem::take(&mut self.queue)
    }

    /// A direction key or button is held.
    pub fn dir_held(&self) -> bool {
        !self.held_dirs.is_empty()
    }

    pub fn clear_held(&mut self) {
        self.held_dirs.clear();
        self.undo_held = 0;
    }

    fn press_dir(&mut self, d: u8, src: u32, delay: i64) {
        self.held_dirs.retain(|(_, s)| *s != src);
        self.held_dirs.push((d, src));
        self.queue.push(Cmd::Dir(d));
        self.dir_timer = delay;
    }

    fn release(&mut self, src: u32) {
        self.held_dirs.retain(|(_, s)| *s != src);
    }

    /// Feeds one platform event.
    pub fn event(&mut self, e: &Event, s: &Settings) {
        let delay = s.delay();
        match e {
            Event::Key { key, down, repeat } => {
                if *repeat {
                    return;
                }
                let code = keycode(*key);
                if *down {
                    self.any_down += 1;
                } else {
                    self.any_down = self.any_down.saturating_sub(1);
                }
                if *down && let Some(_c) = self.capture {
                    if *key != Key::Escape {
                        self.captured = Some(Captured::Key(code));
                    } else {
                        self.capture = None;
                    }
                    return;
                }
                if *key == Key::Shift {
                    self.shift_held = *down;
                }
                if *down && self.text_entry {
                    match key {
                        Key::Up => self.press_dir(1, key_src(*key), delay),
                        Key::Down => self.press_dir(3, key_src(*key), delay),
                        Key::Left => self.press_dir(2, key_src(*key), delay),
                        Key::Right => self.press_dir(0, key_src(*key), delay),
                        Key::Escape | Key::Back => self.queue.push(Cmd::Back),
                        Key::Enter => self.typed.push('\n'),
                        Key::Space => self.typed.push(' '),
                        Key::Backspace => self.typed.push('\u{8}'),
                        _ => {
                            if let Some(c) =
                                char::from_u32(code as u32).filter(|c| c.is_ascii_alphanumeric())
                            {
                                self.typed.push(if self.shift_held {
                                    c.to_ascii_uppercase()
                                } else {
                                    c
                                });
                            }
                        }
                    }
                    return;
                }
                if !*down && self.text_entry {
                    self.release(key_src(*key));
                    return;
                }
                if *down {
                    match key {
                        Key::Char(c) => self.typed.push((*c as char).to_ascii_lowercase()),
                        Key::W
                        | Key::A
                        | Key::S
                        | Key::D
                        | Key::Z
                        | Key::R
                        | Key::Q
                        | Key::E
                        | Key::P
                        | Key::M => {
                            if let Some(c) = char::from_u32(code as u32) {
                                self.typed.push(c);
                            }
                        }
                        Key::Space => self.typed.push(' '),
                        Key::Backspace => self.typed.push('\u{8}'),
                        _ => {}
                    }
                }
                let src = key_src(*key);
                if !*down {
                    self.release(src);
                    if s.key(Action::Undo, false) == code || s.key(Action::Undo, true) == code {
                        self.undo_held = 0;
                    }
                    return;
                }
                if matches!(key, Key::Escape | Key::Back) {
                    self.queue.push(Cmd::Back);
                    return;
                }
                for a in Action::ALL {
                    let hit =
                        s.key(a, false) == code || (a.has_secondary() && s.key(a, true) == code);
                    if !hit {
                        continue;
                    }
                    match a.dir() {
                        Some(d) => self.press_dir(d, src, delay),
                        None => {
                            let c = match a {
                                Action::Idle => Cmd::Idle,
                                Action::Undo => {
                                    self.undo_held = src;
                                    self.undo_timer = delay;
                                    Cmd::Undo
                                }
                                Action::Restart => Cmd::Restart,
                                Action::Confirm => Cmd::Confirm,
                                _ => Cmd::Pause,
                            };
                            // Idle and confirm share keys by default; both
                            // are queued, screens use the one they need.
                            self.queue.push(c);
                        }
                    }
                }
            }
            Event::Button {
                device: _,
                button,
                down,
            } => {
                if s.pad_disabled() {
                    return;
                }
                let idx = pad_index(*button);
                if *down {
                    self.any_down += 1;
                } else {
                    self.any_down = self.any_down.saturating_sub(1);
                }
                if *down && let Some(Capture::Pad(a, _)) = self.capture {
                    let is_dpad = matches!(
                        button,
                        Button::DPadUp | Button::DPadDown | Button::DPadLeft | Button::DPadRight
                    );
                    if a == PadAction::Move {
                        if is_dpad {
                            self.captured = Some(Captured::Pad("dpad".into()));
                        }
                    } else if !is_dpad {
                        self.captured = Some(Captured::Pad(idx.to_string()));
                    }
                    return;
                }
                let src = 0x8000_0000 | idx as u32;
                let dpad_dir = match button {
                    Button::DPadRight => Some(0),
                    Button::DPadUp => Some(1),
                    Button::DPadLeft => Some(2),
                    Button::DPadDown => Some(3),
                    _ => None,
                };
                let dpad_bound = s.pad(PadAction::Move, false) == "dpad"
                    || s.pad(PadAction::Move, true) == "dpad";
                if let Some(d) = dpad_dir
                    && dpad_bound
                {
                    if *down {
                        self.press_dir(d, src, delay);
                    } else {
                        self.release(src);
                    }
                    return;
                }
                if !*down {
                    self.release(src);
                    if self.undo_held == src {
                        self.undo_held = 0;
                    }
                    return;
                }
                let v = idx.to_string();
                // Select/Back (handhelds, Android pads) is the pad's Escape
                // unless the player bound it to an action.
                if *button == Button::Back
                    && !PadAction::ALL
                        .iter()
                        .any(|&a| s.pad(a, false) == v || s.pad(a, true) == v)
                {
                    self.queue.push(Cmd::Back);
                    return;
                }
                for a in PadAction::ALL {
                    if a == PadAction::Move {
                        continue;
                    }
                    let hit = s.pad(a, false) == v || (a.has_secondary() && s.pad(a, true) == v);
                    if hit {
                        let c = match a {
                            PadAction::Idle => Cmd::Idle,
                            PadAction::Undo => {
                                self.undo_held = src;
                                self.undo_timer = delay;
                                Cmd::Undo
                            }
                            PadAction::Restart => Cmd::Restart,
                            PadAction::Confirm => Cmd::Confirm,
                            _ => Cmd::Pause,
                        };
                        self.queue.push(c);
                    }
                }
            }
            Event::Axis { axis, value, .. } => {
                if s.pad_disabled() {
                    return;
                }
                let stick =
                    s.pad(PadAction::Move, false) == "a0" || s.pad(PadAction::Move, true) == "a0";
                if !stick {
                    return;
                }
                let i = match axis {
                    Axis::LeftX => 0,
                    Axis::LeftY => 1,
                    _ => return,
                };
                let v: i8 = if *value > 0.5 {
                    1
                } else if *value < -0.5 {
                    -1
                } else if value.abs() < 0.3 {
                    0
                } else {
                    self.stick[i]
                };
                if v == self.stick[i] {
                    return;
                }
                if let Some(Capture::Pad(PadAction::Move, _)) = self.capture
                    && v != 0
                {
                    self.captured = Some(Captured::Pad("a0".into()));
                    return;
                }
                let src = 0x4000_0000 | i as u32;
                self.stick[i] = v;
                if v == 0 {
                    self.release(src);
                } else {
                    let d = match (i, v) {
                        (0, 1) => 0,
                        (1, -1) => 1,
                        (0, -1) => 2,
                        _ => 3,
                    };
                    self.press_dir(d, src, delay);
                }
            }
            Event::Focus(false) => self.clear_held(),
            _ => {}
        }
    }

    /// One 60 Hz frame: repeats held directions and undo.
    pub fn tick(&mut self, s: &Settings) {
        let delay = s.delay();
        if let Some(&(d, _)) = self.held_dirs.last() {
            self.dir_timer -= 1;
            if self.dir_timer <= 0 {
                self.queue.push(Cmd::Dir(d));
                self.dir_timer = delay;
            }
        }
        if self.undo_held != 0 {
            self.undo_timer -= 1;
            if self.undo_timer <= 0 {
                self.queue.push(Cmd::Undo);
                self.undo_timer = delay;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(k: Key, down: bool) -> Event {
        Event::Key {
            key: k,
            down,
            repeat: false,
        }
    }

    #[test]
    fn bindings_and_repeat() {
        let s = Settings::defaults();
        let mut i = Input::default();
        i.event(&key(Key::Right, true), &s);
        assert_eq!(i.take(), vec![Cmd::Dir(0)]);
        for _ in 0..9 {
            i.tick(&s);
        }
        assert!(i.take().is_empty());
        i.tick(&s);
        assert_eq!(i.take(), vec![Cmd::Dir(0)]);
        i.event(&key(Key::W, true), &s);
        assert_eq!(i.take(), vec![Cmd::Dir(1)]);
        i.event(&key(Key::W, false), &s);
        for _ in 0..10 {
            i.tick(&s);
        }
        // Back to the still-held right arrow.
        assert_eq!(i.take(), vec![Cmd::Dir(0)]);
        i.event(&key(Key::Right, false), &s);
        i.event(&key(Key::Space, true), &s);
        assert_eq!(i.take(), vec![Cmd::Idle, Cmd::Confirm]);
        i.event(&key(Key::Escape, true), &s);
        assert_eq!(i.take(), vec![Cmd::Back]);
        i.event(&key(Key::Z, true), &s);
        for _ in 0..10 {
            i.tick(&s);
        }
        assert_eq!(i.take(), vec![Cmd::Undo, Cmd::Undo]);
    }

    #[test]
    fn rebinding_uses_keycodes() {
        let mut s = Settings::defaults();
        s.set_key(Action::Undo, false, keycode(Key::Char(b'u')));
        let mut i = Input::default();
        i.event(&key(Key::Char(b'u'), true), &s);
        assert_eq!(i.take(), vec![Cmd::Undo]);
        i.event(&key(Key::Z, true), &s);
        assert!(i.take().is_empty());
    }
}
