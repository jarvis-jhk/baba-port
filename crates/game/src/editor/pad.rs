//! Gamepad editing (`docs/spec/editor.md` §15, `saves.md` §4.5): the
//! binding groups `[gamepad_editor]` and `[gamepad_currobjlist]` with an
//! "alt" modifier, resolved into editor actions.
//!
//! Values: `b<N>` = button N in our positional numbering
//! (`input::pad_index`: 0 A, 1 B, 2 X, 3 Y, 4/5 shoulders, 6 Back,
//! 8/10 stick clicks, 9 Start), `a4+` / `a5+` = left / right trigger,
//! `h0.1` / `h0.2` / `h0.4` / `h0.8` = d-pad up / right / down / left,
//! `-` = unbound. Existing values from the original's settings are kept
//! verbatim (`import`); values we do not understand simply never match.
//! The left stick always moves the cursor and the right stick turns the
//! placement direction. The d-pad moves the cursor unless a direction is
//! bound to an action (the original's own defaults bind the d-pad to
//! alt, layers and move-all and move with the stick); with the alt button
//! held the d-pad turns the placement direction (pads without a right
//! stick).

use baba_platform::{Axis, Button, Event};
use baba_progress::Settings;

use crate::input::pad_index;

/// `[gamepad_editor]` actions in the original's order.
pub const EDITOR_ACTIONS: [&str; 26] = [
    "move",
    "rotate",
    "place",
    "copy",
    "drag",
    "undo",
    "scrollright_hotbar",
    "scrollleft_hotbar",
    "scrollright_tool",
    "scrollleft_tool",
    "currobjlist",
    "quickmenu",
    "swap",
    "scrollright_layer",
    "scrollleft_layer",
    "moveall",
    "altpress",
    "emptytile",
    "showdir",
    "cut",
    "autopick",
    "pickempty",
    "lock",
    "empty_hotbar",
    "save",
    "test",
];

/// `[gamepad_currobjlist]` actions in the original's order.
pub const LIST_ACTIONS: [&str; 17] = [
    "move",
    "select",
    "swap",
    "drag",
    "tooltip",
    "scrollleft",
    "scrollright",
    "closemenu",
    "tags",
    "remove",
    "edit",
    "addnew",
    "search",
    "autoadd",
    "altsave",
    "settings",
    "menu",
];

/// Alt actions and the base action whose button they share (`§15`).
pub const ALT_PAIRS: [(&str, &str); 10] = [
    ("cut", "copy"),
    ("swap", "place"),
    ("emptytile", "undo"),
    ("showdir", "drag"),
    ("lock", "scrollright_tool"),
    ("empty_hotbar", "scrollleft_tool"),
    ("pickempty", "scrollright_hotbar"),
    ("autopick", "scrollleft_hotbar"),
    ("save", "quickmenu"),
    ("test", "currobjlist"),
];

/// Our default for an editor action (alt actions name their base).
pub fn default_editor(a: &str) -> &'static str {
    match a {
        "move" => "dpad",
        "rotate" => "a1",
        "place" => "b0",
        "undo" => "b1",
        "copy" => "b2",
        "currobjlist" => "b3",
        "scrollleft_hotbar" => "b4",
        "scrollright_hotbar" => "b5",
        "scrollleft_tool" => "a4+",
        "scrollright_tool" => "a5+",
        "altpress" => "b6",
        "drag" => "b8",
        "quickmenu" => "b9",
        "scrollright_layer" => "b10",
        "scrollleft_layer" => "-",
        "moveall" => "-",
        _ => "alt",
    }
}

/// Our default for a palette / object list action.
pub fn default_list(a: &str) -> &'static str {
    match a {
        "move" => "dpad",
        "select" => "b0",
        "closemenu" => "b1",
        "edit" => "b2",
        "addnew" => "b3",
        "scrollleft" => "b4",
        "scrollright" => "b5",
        "swap" => "a5+",
        "tags" => "a4+",
        "remove" => "b8",
        "search" => "b10",
        "menu" => "b9",
        _ => "-",
    }
}

/// The value bound to `action` in `group` (setting, else our default).
pub fn binding(s: &Settings, group: &str, action: &str) -> String {
    let v = s.store.get(group, action).map(str::trim).unwrap_or("");
    if !v.is_empty() {
        return v.to_string();
    }
    match group {
        "gamepad_editor" => {
            if let Some((_, base)) = ALT_PAIRS.iter().find(|(a, _)| *a == action) {
                return binding(s, group, base);
            }
            default_editor(action).to_string()
        }
        _ => default_list(action).to_string(),
    }
}

/// Writes our defaults into both groups ("Default gamepad controls").
pub fn reset(s: &mut Settings) {
    for a in EDITOR_ACTIONS {
        let v = match ALT_PAIRS.iter().find(|(x, _)| *x == a) {
            Some((_, base)) => default_editor(base),
            None => default_editor(a),
        };
        s.store.set("gamepad_editor", a, v);
    }
    for a in LIST_ACTIONS {
        s.store.set("gamepad_currobjlist", a, default_list(a));
    }
}

/// A readable name of a binding value.
pub fn value_name(v: &str) -> String {
    match v {
        "a4+" => "lt".into(),
        "a5+" => "rt".into(),
        "-" | "" => "-".into(),
        v => match v.strip_prefix('b') {
            Some(n) => crate::input::pad_name(n),
            None => crate::input::pad_name(v),
        },
    }
}

fn actions_of(group: &str) -> &'static [&'static str] {
    if group == "gamepad_editor" {
        &EDITOR_ACTIONS
    } else {
        &LIST_ACTIONS
    }
}

/// One pad input as a binding value.
fn button_value(b: Button) -> String {
    format!("b{}", pad_index(b))
}

/// Resolved inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadIn {
    /// A bound action pressed (`alt` held or not decides alt actions).
    Press(&'static str),
    /// A bound hold action released (`drag`, `moveall`, `altpress`).
    Release(&'static str),
    /// Cursor move (d-pad / left stick), 0 right 1 up 2 left 3 down.
    Move(u8),
    /// Placement direction (right stick, alt + d-pad).
    Rotate(u8),
}

/// Held pad state of the editor.
#[derive(Clone, Debug, Default)]
pub struct PadState {
    /// Button values currently down.
    pub down: Vec<String>,
    pub alt: bool,
    stick: [i8; 2],
    rstick: [i8; 2],
    triggers: [bool; 2],
    /// Held move direction and its repeat timer.
    held: Option<(u8, i32)>,
    /// A pad was used last (button hints, cursor shown).
    pub active: bool,
}

const REPEAT_FIRST: i32 = 14;
const REPEAT: i32 = 5;

impl PadState {
    /// Feeds an event; returns the resolved inputs. `group` is
    /// `gamepad_editor` (main view) or `gamepad_currobjlist`.
    pub fn event(&mut self, e: &Event, s: &Settings, group: &str) -> Vec<PadIn> {
        let mut out = Vec::new();
        let (value, down) = match *e {
            Event::Button { button, down, .. } => {
                self.active = true;
                let dir = match button {
                    Button::DPadRight => Some(0),
                    Button::DPadUp => Some(1),
                    Button::DPadLeft => Some(2),
                    Button::DPadDown => Some(3),
                    _ => None,
                };
                if let Some(d) = dir {
                    let hat = ["h0.2", "h0.1", "h0.8", "h0.4"][d as usize];
                    let bound = actions_of(group)
                        .iter()
                        .any(|a| *a != "move" && *a != "rotate" && binding(s, group, a) == hat);
                    if !bound {
                        if down {
                            out.push(self.dir_input(d));
                            self.held = Some((d, REPEAT_FIRST));
                        } else if self.held.is_some_and(|(h, _)| h == d) {
                            self.held = None;
                        }
                        return out;
                    }
                    (hat.to_string(), down)
                } else {
                    (button_value(button), down)
                }
            }
            Event::Axis { axis, value, .. } => {
                self.active = true;
                match axis {
                    Axis::LeftX | Axis::LeftY => {
                        let i = if axis == Axis::LeftX { 0 } else { 1 };
                        let v = if value > 0.5 {
                            1
                        } else if value < -0.5 {
                            -1
                        } else {
                            0
                        };
                        if v != self.stick[i] {
                            self.stick[i] = v;
                            if v != 0 {
                                let d = match (i, v) {
                                    (0, 1) => 0,
                                    (1, -1) => 1,
                                    (0, _) => 2,
                                    _ => 3,
                                };
                                out.push(self.dir_input(d));
                                self.held = Some((d, REPEAT_FIRST));
                            } else {
                                self.held = None;
                            }
                        }
                        return out;
                    }
                    Axis::RightX | Axis::RightY => {
                        let i = if axis == Axis::RightX { 0 } else { 1 };
                        let v = if value > 0.6 {
                            1
                        } else if value < -0.6 {
                            -1
                        } else {
                            0
                        };
                        if v != self.rstick[i] {
                            self.rstick[i] = v;
                            if v != 0 {
                                out.push(PadIn::Rotate(match (i, v) {
                                    (0, 1) => 0,
                                    (1, -1) => 1,
                                    (0, _) => 2,
                                    _ => 3,
                                }));
                            }
                        }
                        return out;
                    }
                    Axis::TriggerLeft | Axis::TriggerRight => {
                        let i = if axis == Axis::TriggerLeft { 0 } else { 1 };
                        let on = value > 0.5;
                        if on == self.triggers[i] {
                            return out;
                        }
                        self.triggers[i] = on;
                        (format!("a{}+", 4 + i), on)
                    }
                }
            }
            _ => return out,
        };
        if down {
            if !self.down.contains(&value) {
                self.down.push(value.clone());
            }
        } else {
            self.down.retain(|v| *v != value);
        }
        let actions = actions_of(group);
        if group == "gamepad_editor" && binding(s, group, "altpress") == value {
            self.alt = down;
            return out;
        }
        for &a in actions {
            if a == "move" || a == "rotate" || a == "altpress" {
                continue;
            }
            let is_alt = ALT_PAIRS.iter().any(|(x, _)| *x == a);
            if group == "gamepad_editor" && is_alt != self.alt {
                continue;
            }
            if binding(s, group, a) == value {
                out.push(if down {
                    PadIn::Press(a)
                } else {
                    PadIn::Release(a)
                });
                if down {
                    break;
                }
            }
        }
        out
    }

    fn dir_input(&self, d: u8) -> PadIn {
        if self.alt {
            PadIn::Rotate(d)
        } else {
            PadIn::Move(d)
        }
    }

    /// Per tick: repeated moves while a direction is held.
    pub fn tick(&mut self) -> Option<PadIn> {
        let (d, t) = self.held.as_mut()?;
        *t -= 1;
        if *t <= 0 {
            *t = REPEAT;
            let d = *d;
            return Some(self.dir_input(d));
        }
        None
    }

    pub fn held_value(&self, s: &Settings, group: &str, action: &str) -> bool {
        let v = binding(s, group, action);
        self.down.contains(&v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baba_platform::DeviceId;

    fn press(p: &mut PadState, s: &Settings, b: Button, down: bool) -> Vec<PadIn> {
        p.event(
            &Event::Button {
                device: DeviceId::Controller(1),
                button: b,
                down,
            },
            s,
            "gamepad_editor",
        )
    }

    #[test]
    fn alt_combos() {
        let s = Settings::defaults();
        let mut p = PadState::default();
        assert_eq!(
            press(&mut p, &s, Button::South, true),
            vec![PadIn::Press("place")]
        );
        press(&mut p, &s, Button::South, false);
        press(&mut p, &s, Button::Back, true);
        assert!(p.alt);
        assert_eq!(
            press(&mut p, &s, Button::South, true),
            vec![PadIn::Press("swap")]
        );
        assert_eq!(
            press(&mut p, &s, Button::West, true),
            vec![PadIn::Press("cut")]
        );
        assert_eq!(
            press(&mut p, &s, Button::DPadLeft, true),
            vec![PadIn::Rotate(2)]
        );
        press(&mut p, &s, Button::Back, false);
        assert_eq!(
            press(&mut p, &s, Button::DPadLeft, true),
            vec![PadIn::Move(2)]
        );
        assert_eq!(binding(&s, "gamepad_editor", "test"), "b3");
    }

    #[test]
    fn original_hat_bindings() {
        // The original's own defaults: d-pad up = alt, right = next layer.
        let mut s = Settings::defaults();
        s.store.set("gamepad_editor", "altpress", "h0.1");
        s.store.set("gamepad_editor", "scrollright_layer", "h0.2");
        let mut p = PadState::default();
        assert_eq!(
            press(&mut p, &s, Button::DPadRight, true),
            vec![PadIn::Press("scrollright_layer")]
        );
        press(&mut p, &s, Button::DPadUp, true);
        assert!(p.alt);
        assert_eq!(
            press(&mut p, &s, Button::DPadDown, true),
            vec![PadIn::Rotate(3)]
        );
    }
}
