//! `SettingsC.txt`: options, bindings, slot names, last session
//! (`docs/spec/saves.md` §4). Kept as a [`Store`] so groups we do not
//! interpret (editor bindings, code history, …) survive a save, with typed
//! accessors and the defaults a fresh install writes.

use crate::store::Store;

/// SDL keycodes used by the defaults (`saves.md` §4.3).
pub mod keycode {
    pub const RIGHT: i64 = 1073741903;
    pub const LEFT: i64 = 1073741904;
    pub const DOWN: i64 = 1073741905;
    pub const UP: i64 = 1073741906;
    pub const SPACE: i64 = 32;
    pub const RETURN: i64 = 13;
    pub const BACKSPACE: i64 = 8;
    pub const ESCAPE: i64 = 27;
}

/// In-game actions that have bindings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Action {
    Right,
    Up,
    Left,
    Down,
    Idle,
    Undo,
    Restart,
    Confirm,
    Pause,
}

impl Action {
    pub const ALL: [Action; 9] = [
        Action::Right,
        Action::Up,
        Action::Left,
        Action::Down,
        Action::Idle,
        Action::Undo,
        Action::Restart,
        Action::Confirm,
        Action::Pause,
    ];

    /// Key name in `[keyboard]` (secondary binding: name + `2`).
    pub fn key(self) -> &'static str {
        match self {
            Action::Right => "right",
            Action::Up => "up",
            Action::Left => "left",
            Action::Down => "down",
            Action::Idle => "idle",
            Action::Undo => "undo",
            Action::Restart => "restart",
            Action::Confirm => "confirm",
            Action::Pause => "pause",
        }
    }

    /// Direction 0 right, 1 up, 2 left, 3 down.
    pub fn dir(self) -> Option<u8> {
        match self {
            Action::Right => Some(0),
            Action::Up => Some(1),
            Action::Left => Some(2),
            Action::Down => Some(3),
            _ => None,
        }
    }

    /// Pause has a single binding.
    pub fn has_secondary(self) -> bool {
        self != Action::Pause
    }
}

/// Default keyboard binding (`saves.md` §4.3).
pub fn default_key(action: Action, secondary: bool) -> i64 {
    use keycode::*;
    match (action, secondary) {
        (Action::Right, false) => RIGHT,
        (Action::Up, false) => UP,
        (Action::Left, false) => LEFT,
        (Action::Down, false) => DOWN,
        (Action::Right, true) => 'd' as i64,
        (Action::Up, true) => 'w' as i64,
        (Action::Left, true) => 'a' as i64,
        (Action::Down, true) => 's' as i64,
        (Action::Idle, false) | (Action::Confirm, false) => SPACE,
        (Action::Idle, true) | (Action::Confirm, true) => RETURN,
        (Action::Undo, false) => 'z' as i64,
        (Action::Undo, true) => BACKSPACE,
        (Action::Restart, _) => 'r' as i64,
        (Action::Pause, _) => 'p' as i64,
    }
}

/// Gamepad actions (`[gamepad]`, `saves.md` §4.4): `move` takes a stick
/// (`a0`) or `dpad`; the others a raw button index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PadAction {
    Move,
    Idle,
    Undo,
    Restart,
    Confirm,
    Pause,
}

impl PadAction {
    pub const ALL: [PadAction; 6] = [
        PadAction::Move,
        PadAction::Idle,
        PadAction::Undo,
        PadAction::Restart,
        PadAction::Confirm,
        PadAction::Pause,
    ];

    pub fn key(self) -> &'static str {
        match self {
            PadAction::Move => "move",
            PadAction::Idle => "idle",
            PadAction::Undo => "undo",
            PadAction::Restart => "restart",
            PadAction::Confirm => "confirm",
            PadAction::Pause => "pause",
        }
    }

    pub fn has_secondary(self) -> bool {
        self != PadAction::Pause
    }
}

/// Fresh gamepad defaults (`saves.md` §4.4).
pub fn default_pad(action: PadAction, secondary: bool) -> &'static str {
    match (action, secondary) {
        (PadAction::Move, false) => "a0",
        (PadAction::Move, true) => "dpad",
        (PadAction::Idle, false) | (PadAction::Confirm, false) => "0",
        (PadAction::Idle, true) | (PadAction::Confirm, true) => "2",
        (PadAction::Undo, false) => "1",
        (PadAction::Undo, true) => "3",
        (PadAction::Restart, false) => "5",
        (PadAction::Restart, true) => "4",
        (PadAction::Pause, _) => "9",
    }
}

/// Zoom modes (`ui.md` §5): stored 0 Default, 1 None, 2 Stretch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zoom {
    Default,
    None,
    Stretch,
}

/// Boolean options of `[settings]` (`saves.md` §4.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    Fullscreen,
    Grid,
    /// 1 = wobble disabled.
    NoWobble,
    /// 1 = particles disabled.
    NoParticles,
    /// 1 = screen shake disabled.
    NoShake,
    Contrast,
    /// 1 = no blinking colours.
    NoBlinking,
    /// 1 = restart without asking.
    RestartNoAsk,
}

impl Toggle {
    pub const ALL: [Toggle; 8] = [
        Toggle::Fullscreen,
        Toggle::Grid,
        Toggle::NoWobble,
        Toggle::NoParticles,
        Toggle::NoShake,
        Toggle::Contrast,
        Toggle::NoBlinking,
        Toggle::RestartNoAsk,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Toggle::Fullscreen => "fullscreen",
            Toggle::Grid => "grid",
            Toggle::NoWobble => "wobble",
            Toggle::NoParticles => "particles",
            Toggle::NoShake => "shake",
            Toggle::Contrast => "contrast",
            Toggle::NoBlinking => "blinking",
            Toggle::RestartNoAsk => "restartask",
        }
    }
}

/// The settings file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub store: Store,
}

const S: &str = "settings";

impl Settings {
    /// Parses the file and fills in missing defaults (like a fresh run).
    pub fn parse(text: &str) -> Self {
        let mut s = Self {
            store: Store::parse(text),
        };
        s.ensure_defaults();
        s
    }

    /// Defaults only.
    pub fn defaults() -> Self {
        Self::parse("")
    }

    pub fn to_text(&self) -> String {
        self.store.to_text()
    }

    /// Writes the keys a fresh install has (`init`, volumes, bindings).
    pub fn ensure_defaults(&mut self) {
        let st = &mut self.store;
        for (k, v) in [
            ("music", "70"),
            ("sound", "100"),
            ("delay", "10"),
            ("fullscreen", "0"),
            ("grid", "0"),
            ("lang", "en"),
            ("init", "1"),
        ] {
            if st.get(S, k).is_none() {
                st.set(S, k, v);
            }
        }
        if st.get(S, "controlinit").is_none() {
            st.set(S, "controlinit", 1);
            for a in Action::ALL {
                for sec in [false, true] {
                    if sec && !a.has_secondary() {
                        continue;
                    }
                    let k = key_name(a.key(), sec);
                    if st.get("keyboard", &k).is_none() {
                        st.set("keyboard", &k, default_key(a, sec));
                    }
                }
            }
        }
        if st.get(S, "gamepadprofileinit").is_none() {
            st.set(S, "gamepadprofileinit", 1);
            for a in PadAction::ALL {
                for sec in [false, true] {
                    if sec && !a.has_secondary() {
                        continue;
                    }
                    let k = key_name(a.key(), sec);
                    if st.get("gamepad", &k).is_none() {
                        st.set("gamepad", &k, default_pad(a, sec));
                    }
                }
            }
        }
    }

    fn clamp(&self, key: &str, lo: i64, hi: i64, default: i64) -> i64 {
        self.store.get_int(S, key).unwrap_or(default).clamp(lo, hi)
    }

    pub fn music(&self) -> i64 {
        self.clamp("music", 0, 100, 70)
    }
    pub fn set_music(&mut self, v: i64) {
        self.store.set(S, "music", v.clamp(0, 100));
    }
    pub fn sound(&self) -> i64 {
        self.clamp("sound", 0, 100, 100)
    }
    pub fn set_sound(&mut self, v: i64) {
        self.store.set(S, "sound", v.clamp(0, 100));
    }
    /// Input repeat delay in frames (7–20).
    pub fn delay(&self) -> i64 {
        self.clamp("delay", 7, 20, 10)
    }
    pub fn set_delay(&mut self, v: i64) {
        self.store.set(S, "delay", v.clamp(7, 20));
    }
    pub fn toggle(&self, t: Toggle) -> bool {
        self.store.int(S, t.key()) != 0
    }
    pub fn set_toggle(&mut self, t: Toggle, on: bool) {
        self.store.set(S, t.key(), on as i32);
    }
    pub fn zoom(&self) -> Zoom {
        match self.store.int(S, "zoom") {
            1 => Zoom::None,
            2 => Zoom::Stretch,
            _ => Zoom::Default,
        }
    }
    pub fn set_zoom(&mut self, z: Zoom) {
        let v = match z {
            Zoom::Default => 0,
            Zoom::None => 1,
            Zoom::Stretch => 2,
        };
        self.store.set(S, "zoom", v);
    }
    pub fn language(&self) -> String {
        match self.store.get(S, "lang") {
            Some(l) if !l.is_empty() => l.to_string(),
            _ => "en".into(),
        }
    }
    pub fn set_language(&mut self, code: &str) {
        self.store.set(S, "lang", code);
    }

    /// Keyboard binding (SDL keycode); `secondary` reads `<action>2`.
    pub fn key(&self, a: Action, secondary: bool) -> i64 {
        self.store
            .get_int("keyboard", &key_name(a.key(), secondary))
            .unwrap_or_else(|| default_key(a, secondary))
    }
    pub fn set_key(&mut self, a: Action, secondary: bool, code: i64) {
        self.store
            .set("keyboard", &key_name(a.key(), secondary), code);
    }
    pub fn reset_keys(&mut self) {
        for a in Action::ALL {
            for sec in [false, true] {
                if sec && !a.has_secondary() {
                    continue;
                }
                self.set_key(a, sec, default_key(a, sec));
            }
        }
    }
    /// Gamepad binding value (`a0`, `dpad` or a button index).
    pub fn pad(&self, a: PadAction, secondary: bool) -> String {
        self.store
            .get("gamepad", &key_name(a.key(), secondary))
            .map(str::to_string)
            .unwrap_or_else(|| default_pad(a, secondary).to_string())
    }
    pub fn set_pad(&mut self, a: PadAction, secondary: bool, v: &str) {
        self.store.set("gamepad", &key_name(a.key(), secondary), v);
    }
    pub fn reset_pad(&mut self) {
        for a in PadAction::ALL {
            for sec in [false, true] {
                if sec && !a.has_secondary() {
                    continue;
                }
                self.set_pad(a, sec, default_pad(a, sec));
            }
        }
    }
    /// Gamepad input disabled. The original keeps this in memory only
    /// (`saves.md` §4.1); we store it under our own key `[port] nopad`.
    pub fn pad_disabled(&self) -> bool {
        self.store.int("port", "nopad") != 0
    }
    pub fn set_pad_disabled(&mut self, v: bool) {
        self.store.set("port", "nopad", v as i32);
    }

    /// Player-given slot name, if any.
    pub fn slot_name(&self, slot: usize) -> Option<&str> {
        self.store
            .get("slotnames", &slot.to_string())
            .filter(|s| !s.is_empty())
    }
    pub fn set_slot_name(&mut self, slot: usize, name: &str) {
        let name: String = name.chars().take(16).collect();
        self.store.set("slotnames", &slot.to_string(), name);
    }

    /// `[savegame]`: last world and slot (Continue, `saves.md` §4.2).
    pub fn last_session(&self) -> Option<(String, usize)> {
        let w = self.store.get("savegame", "world")?;
        if w.is_empty() {
            return None;
        }
        let slot = self.store.int("savegame", "slot").clamp(0, 2) as usize;
        Some((w.to_string(), slot))
    }
    pub fn set_last_session(&mut self, world: &str, slot: usize) {
        self.store.set("savegame", "world", world);
        self.store.set("savegame", "slot", slot);
    }
}

fn key_name(base: &str, secondary: bool) -> String {
    if secondary {
        format!("{base}2")
    } else {
        base.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_defaults_match_the_original() {
        let s = Settings::defaults();
        assert_eq!(s.music(), 70);
        assert_eq!(s.sound(), 100);
        assert_eq!(s.delay(), 10);
        assert_eq!(s.key(Action::Up, false), keycode::UP);
        assert_eq!(s.key(Action::Undo, true), keycode::BACKSPACE);
        assert_eq!(s.key(Action::Pause, false), 'p' as i64);
        assert_eq!(s.pad(PadAction::Move, true), "dpad");
        assert_eq!(s.pad(PadAction::Restart, false), "5");
        let t = s.to_text();
        assert!(t.contains("[keyboard]\n"));
        assert!(t.contains("right=1073741903\n"));
        assert!(!t.contains("pause2"));
    }

    #[test]
    fn keeps_unknown_groups_and_values() {
        let mut s = Settings::parse(
            "[settings]\nmusic=40\nlang=de\nzoom=2\n[get_d]\ntotal=1\n0code=ABCD-EFGH\n",
        );
        assert_eq!(s.music(), 40);
        assert_eq!(s.language(), "de");
        assert_eq!(s.zoom(), Zoom::Stretch);
        s.set_toggle(Toggle::RestartNoAsk, true);
        let t = s.to_text();
        assert!(t.contains("0code=ABCD-EFGH"));
        assert!(t.contains("restartask=1"));
    }
}
