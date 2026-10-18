//! On-screen touch controls (Android, any touchscreen): a d-pad and Wait /
//! Undo / Restart / Menu buttons. Pure logic: layout in drawable pixels, hit
//! testing, finger tracking, key repeat and auto-hide. Touches on a control
//! become ordinary keyboard [`Event::Key`]s (arrows, Space, Z, R, Escape:
//! the game's default keys, `docs/spec/ui.md` §6.1), so the game needs no
//! touch-specific input code; touches elsewhere pass through as
//! [`Event::Touch`]. Drawing is the renderer's job (`baba_render::ui`): the
//! game asks [`TouchControls::items`] what to draw each frame.
//!
//! The controls hide as soon as a controller or a physical key is used and
//! come back on the next touch.

use std::time::Duration;

use crate::{Event, Key, TouchPhase};

/// One on-screen control.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TouchControl {
    Up,
    Down,
    Left,
    Right,
    Wait,
    Undo,
    Restart,
    Menu,
}

impl TouchControl {
    pub const ALL: [TouchControl; 8] = [
        TouchControl::Up,
        TouchControl::Down,
        TouchControl::Left,
        TouchControl::Right,
        TouchControl::Wait,
        TouchControl::Undo,
        TouchControl::Restart,
        TouchControl::Menu,
    ];

    /// The key this control presses.
    pub fn key(self) -> Key {
        match self {
            TouchControl::Up => Key::Up,
            TouchControl::Down => Key::Down,
            TouchControl::Left => Key::Left,
            TouchControl::Right => Key::Right,
            TouchControl::Wait => Key::Space,
            TouchControl::Undo => Key::Z,
            TouchControl::Restart => Key::R,
            TouchControl::Menu => Key::Escape,
        }
    }

    fn repeats(self) -> bool {
        !matches!(self, TouchControl::Restart | TouchControl::Menu)
    }
}

/// What to draw for one control (drawable pixels, origin top-left).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TouchItem {
    pub control: TouchControl,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub pressed: bool,
}

/// Key repeat while a control is held (like a keyboard's auto-repeat).
const REPEAT_DELAY: Duration = Duration::from_millis(350);
const REPEAT_EVERY: Duration = Duration::from_millis(150);

#[derive(Clone, Copy, Debug, PartialEq)]
enum Grab {
    /// Finger on the d-pad, pressing this direction (None = dead zone).
    Pad(Option<TouchControl>),
    Button(TouchControl),
    /// Not on a control: the game gets the raw touch.
    Pass,
}

#[derive(Clone, Copy, Debug)]
struct Finger {
    id: u64,
    grab: Grab,
    /// When the next repeat is due.
    next_repeat: Duration,
}

/// The touch control state machine. Feed it every platform event through
/// [`TouchControls::filter`] and call [`TouchControls::tick`] once per poll.
#[derive(Clone, Debug)]
pub struct TouchControls {
    enabled: bool,
    visible: bool,
    width: f32,
    height: f32,
    /// D-pad centre and radius of its touch zone; size of one arrow.
    pad: (f32, f32, f32),
    arrow: f32,
    buttons: Vec<(TouchControl, [f32; 4])>,
    fingers: Vec<Finger>,
    now: Duration,
    /// Key-ups owed after [`TouchControls::set_enabled`]`(false)`.
    pending: Vec<Event>,
}

impl TouchControls {
    /// Controls for a `width` x `height` drawable, shown until a controller
    /// or key is used.
    pub fn new(width: u32, height: u32) -> Self {
        let mut t = Self {
            enabled: true,
            visible: true,
            width: 1.0,
            height: 1.0,
            pad: (0.0, 0.0, 0.0),
            arrow: 0.0,
            buttons: Vec::new(),
            fingers: Vec::new(),
            now: Duration::ZERO,
            pending: Vec::new(),
        };
        t.resize(width, height);
        t
    }

    /// Turns the controls on or off entirely (off: every touch passes).
    /// Held controls are released (their key-ups come with the next
    /// [`TouchControls::tick`]); fingers still down then pass through.
    pub fn set_enabled(&mut self, on: bool) {
        if self.enabled && !on {
            let mut ups = Vec::new();
            self.release_all(&mut ups);
            self.pending
                .extend(ups.into_iter().filter(|e| matches!(e, Event::Key { .. })));
        }
        self.enabled = on;
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Shown (enabled and not hidden by controller/keyboard use).
    pub fn visible(&self) -> bool {
        self.enabled && self.visible
    }

    /// Lays the controls out for a new drawable size.
    pub fn resize(&mut self, width: u32, height: u32) {
        let (w, h) = (width.max(1) as f32, height.max(1) as f32);
        self.width = w;
        self.height = h;
        // One unit ~ 11 % of the short side: about 1.5 cm on a phone.
        let u = (w.min(h) * 0.11).max(24.0);
        let margin = u * 0.35;
        self.arrow = u;
        let r = u * 1.5;
        self.pad = (margin + r, h - margin - r, r);
        let b = u * 1.1;
        let right = w - margin;
        let bottom = h - margin;
        let sq = |cx: f32, cy: f32, s: f32| [cx - s / 2.0, cy - s / 2.0, s, s];
        self.buttons = vec![
            (TouchControl::Wait, sq(right - b * 0.5, bottom - b * 0.5, b)),
            (
                TouchControl::Undo,
                sq(right - b * 1.75, bottom - b * 0.5, b),
            ),
            (
                TouchControl::Restart,
                sq(right - b * 0.5, bottom - b * 1.75, b),
            ),
            (
                TouchControl::Menu,
                sq(right - u * 0.4, margin + u * 0.4, u * 0.8),
            ),
        ];
    }

    /// The d-pad direction for a point, `None` in the dead zone or outside.
    fn pad_dir(&self, x: f32, y: f32) -> Option<TouchControl> {
        let (cx, cy, r) = self.pad;
        let (dx, dy) = (x - cx, y - cy);
        let d2 = dx * dx + dy * dy;
        if d2 < (r * 0.2) * (r * 0.2) {
            return None;
        }
        Some(if dx.abs() > dy.abs() {
            if dx > 0.0 {
                TouchControl::Right
            } else {
                TouchControl::Left
            }
        } else if dy > 0.0 {
            TouchControl::Down
        } else {
            TouchControl::Up
        })
    }

    fn in_pad(&self, x: f32, y: f32) -> bool {
        let (cx, cy, r) = self.pad;
        let (dx, dy) = (x - cx, y - cy);
        // A little bigger than drawn: thumbs are imprecise.
        dx * dx + dy * dy <= (r * 1.25) * (r * 1.25)
    }

    fn button_at(&self, x: f32, y: f32) -> Option<TouchControl> {
        self.buttons.iter().find_map(|&(c, [bx, by, bw, bh])| {
            let pad = bw * 0.12;
            (x >= bx - pad && x <= bx + bw + pad && y >= by - pad && y <= by + bh + pad)
                .then_some(c)
        })
    }

    fn key(out: &mut Vec<Event>, c: TouchControl, down: bool, repeat: bool) {
        out.push(Event::Key {
            key: c.key(),
            down,
            repeat,
        });
    }

    /// Routes one platform event: touches on controls become key events,
    /// everything else is appended unchanged. Controller and keyboard use
    /// hides the controls.
    pub fn filter(&mut self, ev: Event, out: &mut Vec<Event>) {
        match ev {
            Event::Touch {
                finger,
                phase,
                x,
                y,
            } if self.enabled => self.touch(finger, phase, x, y, out),
            Event::Resized { width, height } => {
                self.resize(width, height);
                out.push(ev);
            }
            Event::Button { down: true, .. }
            | Event::Key {
                down: true,
                repeat: false,
                ..
            } => {
                if !matches!(ev, Event::Key { key: Key::Back, .. }) {
                    self.hide();
                }
                out.push(ev);
            }
            Event::Axis { value, .. } if value.abs() > 0.5 => {
                self.hide();
                out.push(ev);
            }
            Event::Suspend | Event::Focus(false) => {
                self.release_all(out);
                out.push(ev);
            }
            _ => out.push(ev),
        }
    }

    fn hide(&mut self) {
        self.visible = false;
    }

    /// Releases every held control (focus loss, suspend).
    pub fn release_all(&mut self, out: &mut Vec<Event>) {
        for f in std::mem::take(&mut self.fingers) {
            match f.grab {
                Grab::Pad(Some(c)) | Grab::Button(c) => Self::key(out, c, false, false),
                Grab::Pass => out.push(Event::Touch {
                    finger: f.id,
                    phase: TouchPhase::Up,
                    x: 0.0,
                    y: 0.0,
                }),
                Grab::Pad(None) => {}
            }
        }
    }

    fn touch(&mut self, finger: u64, phase: TouchPhase, nx: f32, ny: f32, out: &mut Vec<Event>) {
        let (x, y) = (nx * self.width, ny * self.height);
        let idx = self.fingers.iter().position(|f| f.id == finger);
        let pass = Event::Touch {
            finger,
            phase,
            x: nx,
            y: ny,
        };
        match phase {
            TouchPhase::Down => {
                if let Some(i) = idx {
                    // A stale finger with the same id: release it first.
                    let f = self.fingers.remove(i);
                    if let Grab::Pad(Some(c)) | Grab::Button(c) = f.grab {
                        Self::key(out, c, false, false);
                    }
                }
                self.visible = true;
                let grab = if let Some(c) = self.button_at(x, y) {
                    Grab::Button(c)
                } else if self.in_pad(x, y) {
                    Grab::Pad(self.pad_dir(x, y))
                } else {
                    Grab::Pass
                };
                match grab {
                    Grab::Pad(Some(c)) | Grab::Button(c) => Self::key(out, c, true, false),
                    Grab::Pass => out.push(pass),
                    Grab::Pad(None) => {}
                }
                self.fingers.push(Finger {
                    id: finger,
                    grab,
                    next_repeat: self.now + REPEAT_DELAY,
                });
            }
            TouchPhase::Move => match idx.map(|i| (i, self.fingers[i].grab)) {
                Some((i, Grab::Pad(old))) => {
                    let new = if self.in_pad(x, y) {
                        self.pad_dir(x, y)
                    } else {
                        // Sliding off the pad keeps the direction held.
                        old
                    };
                    if new != old {
                        if let Some(c) = old {
                            Self::key(out, c, false, false);
                        }
                        if let Some(c) = new {
                            Self::key(out, c, true, false);
                        }
                        self.fingers[i].grab = Grab::Pad(new);
                        self.fingers[i].next_repeat = self.now + REPEAT_DELAY;
                    }
                }
                Some((_, Grab::Button(_))) => {}
                Some((_, Grab::Pass)) | None => out.push(pass),
            },
            TouchPhase::Up => match idx {
                Some(i) => {
                    let f = self.fingers.remove(i);
                    match f.grab {
                        Grab::Pad(Some(c)) | Grab::Button(c) => Self::key(out, c, false, false),
                        Grab::Pass => out.push(pass),
                        Grab::Pad(None) => {}
                    }
                }
                None => out.push(pass),
            },
        }
    }

    /// Advances the clock (platform time) and emits key repeats for held
    /// controls.
    pub fn tick(&mut self, now: Duration, out: &mut Vec<Event>) {
        self.now = now;
        out.append(&mut self.pending);
        for f in &mut self.fingers {
            let c = match f.grab {
                Grab::Pad(Some(c)) | Grab::Button(c) if c.repeats() => c,
                _ => continue,
            };
            while now >= f.next_repeat {
                Self::key(out, c, true, true);
                f.next_repeat += REPEAT_EVERY;
            }
        }
    }

    /// Controls currently held down.
    pub fn held(&self) -> Vec<TouchControl> {
        self.fingers
            .iter()
            .filter_map(|f| match f.grab {
                Grab::Pad(Some(c)) | Grab::Button(c) => Some(c),
                _ => None,
            })
            .collect()
    }

    /// What to draw this frame (empty when hidden).
    pub fn items(&self) -> Vec<TouchItem> {
        if !self.visible() {
            return Vec::new();
        }
        let held = self.held();
        let (cx, cy, _) = self.pad;
        let a = self.arrow;
        let mut v: Vec<TouchItem> = [
            (TouchControl::Up, 0.0, -1.0),
            (TouchControl::Down, 0.0, 1.0),
            (TouchControl::Left, -1.0, 0.0),
            (TouchControl::Right, 1.0, 0.0),
        ]
        .into_iter()
        .map(|(c, dx, dy)| TouchItem {
            control: c,
            x: cx + dx * a - a / 2.0,
            y: cy + dy * a - a / 2.0,
            w: a,
            h: a,
            pressed: held.contains(&c),
        })
        .collect();
        v.extend(self.buttons.iter().map(|&(c, [x, y, w, h])| TouchItem {
            control: c,
            x,
            y,
            w,
            h,
            pressed: held.contains(&c),
        }));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn centre(t: &TouchControls, c: TouchControl) -> (f32, f32) {
        let it = t.items().into_iter().find(|i| i.control == c).unwrap();
        (
            (it.x + it.w / 2.0) / t.width,
            (it.y + it.h / 2.0) / t.height,
        )
    }

    fn touch(t: &mut TouchControls, id: u64, phase: TouchPhase, p: (f32, f32)) -> Vec<Event> {
        let mut out = Vec::new();
        t.filter(
            Event::Touch {
                finger: id,
                phase,
                x: p.0,
                y: p.1,
            },
            &mut out,
        );
        out
    }

    fn keys(ev: &[Event]) -> Vec<(Key, bool, bool)> {
        ev.iter()
            .filter_map(|e| match e {
                Event::Key { key, down, repeat } => Some((*key, *down, *repeat)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn every_control_presses_its_key() {
        let mut t = TouchControls::new(2560, 1600);
        for c in TouchControl::ALL {
            let p = centre(&t, c);
            let down = touch(&mut t, 1, TouchPhase::Down, p);
            assert_eq!(keys(&down), vec![(c.key(), true, false)], "{c:?}");
            let up = touch(&mut t, 1, TouchPhase::Up, p);
            assert_eq!(keys(&up), vec![(c.key(), false, false)], "{c:?}");
        }
    }

    #[test]
    fn disabling_releases_held_controls() {
        let mut t = TouchControls::new(1280, 720);
        let p = centre(&t, TouchControl::Menu);
        touch(&mut t, 1, TouchPhase::Down, p);
        t.set_enabled(false);
        let mut out = Vec::new();
        t.tick(Duration::from_millis(10), &mut out);
        assert_eq!(keys(&out), vec![(Key::Escape, false, false)]);
        // The finger now passes through; the menu below may use it.
        let up = touch(&mut t, 1, TouchPhase::Up, p);
        assert!(matches!(up[..], [Event::Touch { .. }]));
        t.set_enabled(true);
        let mut out = Vec::new();
        t.tick(Duration::from_millis(20), &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn touches_elsewhere_pass_through() {
        let mut t = TouchControls::new(1280, 720);
        let ev = touch(&mut t, 3, TouchPhase::Down, (0.5, 0.4));
        assert!(matches!(ev[..], [Event::Touch { finger: 3, .. }]));
        let ev = touch(&mut t, 3, TouchPhase::Move, (0.52, 0.4));
        assert!(matches!(ev[..], [Event::Touch { .. }]));
        let ev = touch(&mut t, 3, TouchPhase::Up, (0.52, 0.4));
        assert!(matches!(
            ev[..],
            [Event::Touch {
                phase: TouchPhase::Up,
                ..
            }]
        ));
    }

    #[test]
    fn sliding_on_the_pad_changes_direction() {
        let mut t = TouchControls::new(1280, 720);
        let l = centre(&t, TouchControl::Left);
        let u = centre(&t, TouchControl::Up);
        assert_eq!(
            keys(&touch(&mut t, 1, TouchPhase::Down, l)),
            vec![(Key::Left, true, false)]
        );
        assert_eq!(
            keys(&touch(&mut t, 1, TouchPhase::Move, u)),
            vec![(Key::Left, false, false), (Key::Up, true, false)]
        );
        assert_eq!(
            keys(&touch(&mut t, 1, TouchPhase::Up, u)),
            vec![(Key::Up, false, false)]
        );
    }

    #[test]
    fn multitouch_and_repeat() {
        let mut t = TouchControls::new(1280, 720);
        let r = centre(&t, TouchControl::Right);
        let z = centre(&t, TouchControl::Undo);
        touch(&mut t, 1, TouchPhase::Down, r);
        touch(&mut t, 2, TouchPhase::Down, z);
        assert_eq!(t.held(), vec![TouchControl::Right, TouchControl::Undo]);
        let mut out = Vec::new();
        t.tick(REPEAT_DELAY + REPEAT_EVERY, &mut out);
        assert_eq!(
            keys(&out),
            vec![
                (Key::Right, true, true),
                (Key::Right, true, true),
                (Key::Z, true, true),
                (Key::Z, true, true)
            ]
        );
        let mut out = Vec::new();
        t.filter(Event::Focus(false), &mut out);
        assert_eq!(
            keys(&out),
            vec![(Key::Right, false, false), (Key::Z, false, false)]
        );
    }

    #[test]
    fn controller_hides_touch_shows() {
        let mut t = TouchControls::new(1280, 720);
        assert!(!t.items().is_empty());
        let mut out = Vec::new();
        t.filter(
            Event::Button {
                device: crate::DeviceId::Controller(7),
                button: crate::Button::South,
                down: true,
            },
            &mut out,
        );
        assert_eq!(out.len(), 1);
        assert!(t.items().is_empty());
        // The first touch only brings the controls back unless it hits one.
        let ev = touch(&mut t, 1, TouchPhase::Down, (0.5, 0.5));
        assert!(matches!(ev[..], [Event::Touch { .. }]));
        assert!(!t.items().is_empty());
    }
}
