//! Local players for multiplayer: which device belongs to whom, the
//! keyboard split into two halves, per-player key repeat, joining,
//! leaving and rejoining (hot-plug), and the join screen shown before a
//! session ([`Lobby`]).
//!
//! * Up to [`MAX_PLAYERS`] seats. A seat is a player: stable number
//!   (P1–P12), colour, name, save slot and world (mode 3), and an input
//!   source: a controller (SDL instance id) or one half of the keyboard.
//! * Keyboard halves (fixed bindings, shown on the join screen): left =
//!   W A S D move, E / Space wait and confirm, Q / Z undo, R restart, Tab
//!   pause, Esc back; right = arrows, Enter wait and confirm, Backspace
//!   undo, 0 restart, P pause. Controllers use the gamepad bindings of the
//!   settings (enabled even if "disable gamepad" is on).
//! * Commands keep the order in which the platform delivered the events
//!   (SDL event order): every event is turned into commands at once and
//!   appended to one queue `(seat, command)`; held-key repeats are added
//!   at the tick, in seat order.
//! * Joining: an unassigned device presses A / Start (controller), E /
//!   Space (left half) or Enter (right half). During a session a join
//!   first takes over a seat whose controller was unplugged (rejoin),
//!   then a seat that left with the same device, else a new seat.

use baba_platform::{Button, DeviceId, Event, Key};
use baba_progress::{Action, Profile, Settings};

use crate::input::{Cmd, Input, keycode};
use crate::lang::Texts;
use crate::session::MpMode;
use crate::session::saves::{ALL_SLOTS, ExtraSlots, slot as slot_ref};
use crate::ui::{Align, Gfx, Ui};

/// Most local players at once.
pub const MAX_PLAYERS: usize = 12;

/// Player colours (default palette coordinates), by seat.
pub const COLOURS: [(u32, u32); MAX_PLAYERS] = [
    (2, 2),
    (1, 4),
    (2, 4),
    (5, 3),
    (4, 1),
    (3, 3),
    (6, 1),
    (0, 3),
    (5, 1),
    (2, 3),
    (6, 2),
    (4, 2),
];

/// Default player names, by seat.
pub const NAMES: [&str; MAX_PLAYERS] = [
    "Baba", "Keke", "Me", "Jiji", "Fofo", "Bird", "Badbad", "Robot", "Ghost", "Bug", "Bat", "Cat",
];

/// A half of the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Half {
    Left,
    Right,
}

/// Where a player's input comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Keys(Half),
    /// Controller by SDL instance id.
    Pad(u32),
}

impl Source {
    pub fn describe(self) -> &'static str {
        match self {
            Source::Keys(Half::Left) => "keys left",
            Source::Keys(Half::Right) => "keys right",
            Source::Pad(_) => "controller",
        }
    }
}

/// The half a physical key belongs to.
pub fn half_of(k: Key) -> Option<Half> {
    match k {
        Key::W
        | Key::A
        | Key::S
        | Key::D
        | Key::E
        | Key::Q
        | Key::Z
        | Key::R
        | Key::Tab
        | Key::Escape
        | Key::Space => Some(Half::Left),
        Key::Up
        | Key::Down
        | Key::Left
        | Key::Right
        | Key::Enter
        | Key::Backspace
        | Key::P
        | Key::Char(b'0') => Some(Half::Right),
        _ => None,
    }
}

/// Bindings of a keyboard half.
pub fn half_settings(h: Half) -> Settings {
    let mut s = Settings::defaults();
    let set = |s: &mut Settings, a: Action, p: Key, q: Key| {
        s.set_key(a, false, keycode(p));
        if a.has_secondary() {
            s.set_key(a, true, keycode(q));
        }
    };
    match h {
        Half::Left => {
            set(&mut s, Action::Right, Key::D, Key::D);
            set(&mut s, Action::Up, Key::W, Key::W);
            set(&mut s, Action::Left, Key::A, Key::A);
            set(&mut s, Action::Down, Key::S, Key::S);
            set(&mut s, Action::Idle, Key::E, Key::Space);
            set(&mut s, Action::Confirm, Key::E, Key::Space);
            set(&mut s, Action::Undo, Key::Q, Key::Z);
            set(&mut s, Action::Restart, Key::R, Key::R);
            set(&mut s, Action::Pause, Key::Tab, Key::Tab);
        }
        Half::Right => {
            set(&mut s, Action::Right, Key::Right, Key::Right);
            set(&mut s, Action::Up, Key::Up, Key::Up);
            set(&mut s, Action::Left, Key::Left, Key::Left);
            set(&mut s, Action::Down, Key::Down, Key::Down);
            set(&mut s, Action::Idle, Key::Enter, Key::Enter);
            set(&mut s, Action::Confirm, Key::Enter, Key::Enter);
            set(&mut s, Action::Undo, Key::Backspace, Key::Backspace);
            set(&mut s, Action::Restart, Key::Char(b'0'), Key::Char(b'0'));
            set(&mut s, Action::Pause, Key::P, Key::P);
        }
    }
    s
}

/// One player.
pub struct Seat {
    /// 0–11 (shown as P1–P12); stable for the session.
    pub id: usize,
    /// `None` while the controller is unplugged.
    pub source: Option<Source>,
    /// Index into [`COLOURS`].
    pub colour: usize,
    /// Index into [`NAMES`].
    pub name: usize,
    /// Save slot (mode 3; modes 1/2 use the session's slot).
    pub slot: usize,
    /// World (mode 3; modes 1/2 use the session's world).
    pub world: String,
    /// Online: a player of another machine (no device here).
    pub remote: bool,
    /// Online: the name the player chose (instead of [`NAMES`]).
    pub label: Option<String>,
    input: Input,
}

impl Seat {
    pub fn colour(&self) -> (u32, u32) {
        COLOURS[self.colour % MAX_PLAYERS]
    }

    pub fn name(&self) -> &str {
        match &self.label {
            Some(l) => l,
            None => NAMES[self.name % MAX_PLAYERS],
        }
    }

    /// The sim player this seat drives in co-op: alternating YOU / YOU2
    /// (YOU2 falls back to YOU in levels without it).
    pub fn you(&self) -> u8 {
        1 + (self.id % 2) as u8
    }
}

/// Something about devices and seats the session reacts to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Happening {
    /// An unassigned device pressed its join button.
    Join(Source),
    /// A seat's controller was unplugged.
    Disconnected(usize),
    /// Back on an unassigned device (Esc, controller B).
    UnseatedBack,
    /// A player toggled the zoom on their own view (controller Back/Select).
    Focus(usize),
    /// F2: cycle the zoomed view.
    FocusCycle,
    /// F3: online, this machine's views large / all views equal.
    Layout,
    /// O / F4 / Y on an unassigned controller: go online with these
    /// players (join screen).
    Online,
    /// F5: online host, start now (machines without players watch).
    StartNow,
}

/// All local players and their input.
pub struct Players {
    pub seats: Vec<Seat>,
    queue: Vec<(usize, Cmd)>,
    pub happenings: Vec<Happening>,
    left: Settings,
    right: Settings,
    pads: Settings,
    /// Default world for new seats.
    pub world: String,
}

fn settings_for<'a>(
    src: Source,
    left: &'a Settings,
    right: &'a Settings,
    pads: &'a Settings,
) -> &'a Settings {
    match src {
        Source::Keys(Half::Left) => left,
        Source::Keys(Half::Right) => right,
        Source::Pad(_) => pads,
    }
}

impl Players {
    /// `settings`: the profile's (gamepad bindings, repeat delay).
    pub fn new(settings: &Settings, world: &str) -> Players {
        let mut pads = settings.clone();
        pads.set_pad_disabled(false);
        let delay = settings.delay();
        let mut left = half_settings(Half::Left);
        let mut right = half_settings(Half::Right);
        left.set_delay(delay);
        right.set_delay(delay);
        Players {
            seats: Vec::new(),
            queue: Vec::new(),
            happenings: Vec::new(),
            left,
            right,
            pads,
            world: world.to_string(),
        }
    }

    pub fn index_of_source(&self, src: Source) -> Option<usize> {
        self.seats.iter().position(|s| s.source == Some(src))
    }

    pub fn by_id(&self, id: usize) -> Option<&Seat> {
        self.seats.iter().find(|s| s.id == id)
    }

    pub fn by_id_mut(&mut self, id: usize) -> Option<&mut Seat> {
        self.seats.iter_mut().find(|s| s.id == id)
    }

    /// The first seat (the host: chooses the mode, slot and world of
    /// modes 1/2; may end the session).
    pub fn host(&self) -> Option<usize> {
        self.seats.first().map(|s| s.id)
    }

    /// Feeds one platform event.
    pub fn event(&mut self, e: &Event) {
        let src = match e {
            Event::Key {
                key: Key::F(2),
                down: true,
                repeat: false,
            } => {
                self.happenings.push(Happening::FocusCycle);
                return;
            }
            Event::Key {
                key: Key::F(3),
                down: true,
                repeat: false,
            } => {
                self.happenings.push(Happening::Layout);
                return;
            }
            Event::Key {
                key: Key::F(5),
                down: true,
                repeat: false,
            } => {
                self.happenings.push(Happening::StartNow);
                return;
            }
            Event::Key {
                key: Key::F(4) | Key::Char(b'o'),
                down: true,
                repeat: false,
            } => {
                self.happenings.push(Happening::Online);
                return;
            }
            Event::Key { key, .. } => match half_of(*key) {
                Some(h) => Source::Keys(h),
                None => return,
            },
            Event::Button {
                device: DeviceId::Controller(id),
                button,
                down,
            } => {
                let src = Source::Pad(*id);
                if *button == Button::Back && *down {
                    if let Some(i) = self.index_of_source(src) {
                        self.happenings.push(Happening::Focus(self.seats[i].id));
                    }
                    return;
                }
                src
            }
            Event::Axis {
                device: DeviceId::Controller(id),
                ..
            } => Source::Pad(*id),
            Event::ControllerRemoved(DeviceId::Controller(id)) => {
                if let Some(i) = self.index_of_source(Source::Pad(*id)) {
                    let s = &mut self.seats[i];
                    s.source = None;
                    s.input.clear_held();
                    self.happenings.push(Happening::Disconnected(s.id));
                }
                return;
            }
            Event::Focus(false) => {
                for s in &mut self.seats {
                    s.input.clear_held();
                }
                return;
            }
            _ => return,
        };
        if let Some(i) = self.index_of_source(src) {
            let st = settings_for(src, &self.left, &self.right, &self.pads);
            let seat = &mut self.seats[i];
            seat.input.event(e, st);
            seat.input.typed.clear();
            let id = seat.id;
            for c in seat.input.take() {
                self.queue.push((id, c));
            }
            return;
        }
        let join = match e {
            Event::Key {
                key,
                down: true,
                repeat: false,
            } => match key {
                Key::E | Key::Space | Key::Enter => true,
                Key::Escape => {
                    self.happenings.push(Happening::UnseatedBack);
                    false
                }
                _ => false,
            },
            Event::Button {
                button, down: true, ..
            } => match button {
                Button::South | Button::Start => true,
                Button::East => {
                    self.happenings.push(Happening::UnseatedBack);
                    false
                }
                Button::North => {
                    self.happenings.push(Happening::Online);
                    false
                }
                _ => false,
            },
            _ => false,
        };
        if join {
            self.happenings.push(Happening::Join(src));
        }
    }

    /// One 60 Hz frame: key repeat of every seat.
    pub fn tick(&mut self) {
        for seat in &mut self.seats {
            let Some(src) = seat.source else { continue };
            let st = settings_for(src, &self.left, &self.right, &self.pads);
            seat.input.tick(st);
            for c in seat.input.take() {
                self.queue.push((seat.id, c));
            }
        }
    }

    /// A command for a seat as if its device sent it (online bots, tests).
    pub fn push(&mut self, seat: usize, c: Cmd) {
        if self.by_id(seat).is_some() {
            self.queue.push((seat, c));
        }
    }

    /// Commands since the last call, in arrival order: `(seat id, cmd)`.
    pub fn take(&mut self) -> Vec<(usize, Cmd)> {
        std::mem::take(&mut self.queue)
    }

    pub fn take_happenings(&mut self) -> Vec<Happening> {
        std::mem::take(&mut self.happenings)
    }

    /// Adds a seat for `src` (lowest free number). `None` when full.
    pub fn add(&mut self, src: Source) -> Option<usize> {
        if self.seats.len() >= MAX_PLAYERS {
            return None;
        }
        let id = (0..MAX_PLAYERS).find(|i| !self.seats.iter().any(|s| s.id == *i))?;
        let used_c: Vec<usize> = self.seats.iter().map(|s| s.colour).collect();
        let colour = (0..MAX_PLAYERS)
            .map(|k| (id + k) % MAX_PLAYERS)
            .find(|c| !used_c.contains(c))
            .unwrap_or(id);
        let used_n: Vec<usize> = self.seats.iter().map(|s| s.name).collect();
        let name = (0..MAX_PLAYERS)
            .map(|k| (id + k) % MAX_PLAYERS)
            .find(|c| !used_n.contains(c))
            .unwrap_or(id);
        let seat = Seat {
            id,
            source: Some(src),
            colour,
            name,
            slot: 0,
            world: self.world.clone(),
            remote: false,
            label: None,
            input: Input::default(),
        };
        let at = self
            .seats
            .iter()
            .position(|s| s.id > id)
            .unwrap_or(self.seats.len());
        self.seats.insert(at, seat);
        Some(id)
    }

    /// Online: a session player as a seat (`id` = session player id):
    /// this machine's (with its device, or `None` while unplugged) or
    /// another machine's (`remote`).
    pub fn add_net(
        &mut self,
        id: usize,
        source: Option<Source>,
        remote: bool,
        name: &str,
        colour: usize,
    ) {
        self.remove(id);
        let seat = Seat {
            id,
            source,
            colour,
            name: id % MAX_PLAYERS,
            slot: 0,
            world: self.world.clone(),
            remote,
            label: Some(name.to_string()),
            input: Input::default(),
        };
        let at = self
            .seats
            .iter()
            .position(|s| s.id > id)
            .unwrap_or(self.seats.len());
        self.seats.insert(at, seat);
    }

    /// Puts a seat back (rejoin after leaving).
    pub fn restore(&mut self, mut seat: Seat, src: Source) {
        seat.source = Some(src);
        seat.input = Input::default();
        let at = self
            .seats
            .iter()
            .position(|s| s.id > seat.id)
            .unwrap_or(self.seats.len());
        self.seats.insert(at, seat);
    }

    pub fn remove(&mut self, id: usize) -> Option<Seat> {
        let i = self.seats.iter().position(|s| s.id == id)?;
        Some(self.seats.remove(i))
    }

    /// Gives a seat a (new) device.
    pub fn assign(&mut self, id: usize, src: Source) {
        if let Some(s) = self.by_id_mut(id) {
            s.source = Some(src);
            s.input = Input::default();
        }
    }

    /// The lowest slot no other seat uses (mode 3).
    pub fn free_slot(&self, except: usize, from: usize) -> usize {
        (0..ALL_SLOTS)
            .map(|k| (from + k) % ALL_SLOTS)
            .find(|n| !self.seats.iter().any(|s| s.id != except && s.slot == *n))
            .unwrap_or(from)
    }
}

// ---------------------------------------------------------------- lobby

/// A row of a player's card on the join screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Mode,
    Slot,
    World,
    Colour,
    Name,
    Ready,
}

/// A player's card.
pub struct Card {
    pub id: usize,
    pub row: Row,
    pub ready: bool,
}

/// The join screen.
pub struct Lobby {
    pub players: Players,
    pub cards: Vec<Card>,
    pub mode: MpMode,
    /// Modes 1/2: the shared save slot and world (the host chooses).
    pub slot: usize,
    pub world: String,
    /// Selectable worlds: (id, display name).
    pub packs: Vec<(String, String)>,
    pub extra: ExtraSlots,
    countdown: Option<u32>,
    /// Leave to the title screen.
    pub exit: bool,
    /// Everybody is ready: start the session.
    pub start: bool,
    pub log: Vec<String>,
    /// Online room lobby (M7): no countdown here (the host starts when
    /// every machine is ready), header and help say so.
    pub online: bool,
    /// Online and not the host: mode, save and world are the host's.
    pub guest: bool,
    /// Online: the host's countdown, shown in the header.
    pub online_countdown: Option<u32>,
    /// The player asked to go online with these seats.
    pub to_online: bool,
    /// Online host: F5 starts without waiting for everybody.
    pub force_start: bool,
}

const COUNTDOWN: u32 = 45;

impl Lobby {
    pub fn new(
        settings: &Settings,
        packs: Vec<(String, String)>,
        slot: usize,
        extra: ExtraSlots,
    ) -> Lobby {
        let world = packs
            .first()
            .map(|p| p.0.clone())
            .unwrap_or_else(|| "baba".into());
        Lobby {
            players: Players::new(settings, &world),
            cards: Vec::new(),
            mode: MpMode::Coop,
            slot,
            world,
            packs,
            extra,
            countdown: None,
            exit: false,
            start: false,
            log: Vec::new(),
            online: false,
            guest: false,
            online_countdown: None,
            to_online: false,
            force_start: false,
        }
    }

    /// Every seat is ready (and there is at least one).
    pub fn all_ready(&self) -> bool {
        !self.cards.is_empty() && self.cards.iter().all(|c| c.ready)
    }

    /// Un-readies every seat (back in an online room after a session).
    pub fn unready(&mut self) {
        for c in &mut self.cards {
            c.ready = false;
        }
    }

    pub fn event(&mut self, e: &Event) {
        self.players.event(e);
    }

    fn rows(&self, id: usize) -> Vec<Row> {
        let host = self.players.host() == Some(id) && !self.guest;
        let own = self.mode == MpMode::OwnGames;
        let mut v = Vec::new();
        if host {
            v.push(Row::Mode);
        }
        if host || own {
            v.push(Row::Slot);
            v.push(Row::World);
        }
        v.extend([Row::Colour, Row::Name, Row::Ready]);
        v
    }

    fn join(&mut self, src: Source) {
        let Some(id) = self.players.add(src) else {
            return;
        };
        let from = self.slot;
        let slot = self.players.free_slot(id, from);
        if let Some(s) = self.players.by_id_mut(id) {
            s.slot = slot;
        }
        self.cards.push(Card {
            id,
            row: Row::Ready,
            ready: false,
        });
        self.cards.sort_by_key(|c| c.id);
        self.log
            .push(format!("join p{} {}", id + 1, src.describe()));
    }

    fn leave(&mut self, id: usize) {
        self.players.remove(id);
        self.cards.retain(|c| c.id != id);
        self.log.push(format!("leave p{}", id + 1));
    }

    fn set_mode(&mut self, m: MpMode) {
        self.mode = m;
        if m == MpMode::OwnGames {
            // Distinct slots: the host keeps the shared one.
            let ids: Vec<usize> = self.players.seats.iter().map(|s| s.id).collect();
            for (k, id) in ids.iter().enumerate() {
                let want = if k == 0 { self.slot } else { 0 };
                if let Some(s) = self.players.by_id_mut(*id) {
                    s.slot = usize::MAX;
                }
                let n = self.players.free_slot(*id, want);
                if let Some(s) = self.players.by_id_mut(*id) {
                    s.slot = n;
                    s.world = self.world.clone();
                }
            }
        }
        // Rows that are no longer editable.
        for i in 0..self.cards.len() {
            let rows = self.rows(self.cards[i].id);
            if !rows.contains(&self.cards[i].row) {
                self.cards[i].row = Row::Ready;
            }
        }
        self.log.push(format!("mode {m:?}"));
    }

    fn change(&mut self, id: usize, row: Row, step: i32) {
        let n = MAX_PLAYERS as i32;
        let wrap = |v: usize, k: i32, m: usize| ((v as i32 + k).rem_euclid(m as i32)) as usize;
        match row {
            Row::Mode => {
                let all = [MpMode::Coop, MpMode::SharedSave, MpMode::OwnGames];
                let i = all.iter().position(|m| *m == self.mode).unwrap_or(0);
                self.set_mode(all[wrap(i, step, 3)]);
            }
            Row::Slot => {
                if self.mode == MpMode::OwnGames {
                    let Some(cur) = self.players.by_id(id).map(|s| s.slot) else {
                        return;
                    };
                    let mut v = cur;
                    for _ in 0..ALL_SLOTS {
                        v = wrap(v, step, ALL_SLOTS);
                        if !self.players.seats.iter().any(|s| s.id != id && s.slot == v) {
                            break;
                        }
                    }
                    if let Some(s) = self.players.by_id_mut(id) {
                        s.slot = v;
                    }
                    if self.players.host() == Some(id) {
                        self.slot = v;
                    }
                } else {
                    self.slot = wrap(self.slot, step, ALL_SLOTS);
                }
            }
            Row::World => {
                if self.packs.is_empty() {
                    return;
                }
                let cur = if self.mode == MpMode::OwnGames {
                    self.players
                        .by_id(id)
                        .map(|s| s.world.clone())
                        .unwrap_or_default()
                } else {
                    self.world.clone()
                };
                let i = self.packs.iter().position(|p| p.0 == cur).unwrap_or(0);
                let w = self.packs[wrap(i, step, self.packs.len())].0.clone();
                if self.mode == MpMode::OwnGames {
                    if let Some(s) = self.players.by_id_mut(id) {
                        s.world = w.clone();
                    }
                    if self.players.host() == Some(id) {
                        self.world = w;
                    }
                } else {
                    self.world = w;
                }
            }
            Row::Colour | Row::Name => {
                let Some(seat) = self.players.by_id(id) else {
                    return;
                };
                let cur = if row == Row::Colour {
                    seat.colour
                } else {
                    seat.name
                };
                let used: Vec<usize> = self
                    .players
                    .seats
                    .iter()
                    .filter(|s| s.id != id)
                    .map(|s| if row == Row::Colour { s.colour } else { s.name })
                    .collect();
                let mut v = cur as i32;
                for _ in 0..n {
                    v = (v + step).rem_euclid(n);
                    if !used.contains(&(v as usize)) {
                        break;
                    }
                }
                if let Some(s) = self.players.by_id_mut(id) {
                    if row == Row::Colour {
                        s.colour = v as usize;
                    } else {
                        s.name = v as usize;
                    }
                }
            }
            Row::Ready => {}
        }
    }

    /// One frame of the join screen.
    pub fn tick(&mut self) {
        self.players.tick();
        for h in self.players.take_happenings() {
            match h {
                Happening::Join(src) => self.join(src),
                // On the join screen an unplugged controller leaves.
                Happening::Disconnected(id) => self.leave(id),
                Happening::UnseatedBack => {
                    if self.players.seats.is_empty() {
                        self.exit = true;
                    }
                }
                Happening::Focus(_) | Happening::FocusCycle | Happening::Layout => {}
                Happening::Online => self.to_online = !self.online,
                Happening::StartNow => self.force_start = self.online && !self.guest,
            }
        }
        for (id, cmd) in self.players.take() {
            let Some(ci) = self.cards.iter().position(|c| c.id == id) else {
                continue;
            };
            let rows = self.rows(id);
            let row = self.cards[ci].row;
            match cmd {
                Cmd::Dir(d @ (1 | 3)) => {
                    let i = rows.iter().position(|r| *r == row).unwrap_or(0) as i32;
                    let k = if d == 1 { -1 } else { 1 };
                    self.cards[ci].row = rows[(i + k).rem_euclid(rows.len() as i32) as usize];
                }
                Cmd::Dir(d) => {
                    if !self.cards[ci].ready || row != Row::Ready {
                        self.cards[ci].ready = false;
                        self.change(id, row, if d == 0 { 1 } else { -1 });
                    }
                }
                Cmd::Confirm | Cmd::Pause => {
                    self.cards[ci].ready = !self.cards[ci].ready;
                    self.cards[ci].row = Row::Ready;
                }
                Cmd::Back | Cmd::Undo => {
                    if self.cards[ci].ready {
                        self.cards[ci].ready = false;
                    } else {
                        self.leave(id);
                    }
                }
                Cmd::Idle | Cmd::Restart => {}
            }
        }
        let all_ready = !self.cards.is_empty() && self.cards.iter().all(|c| c.ready);
        if all_ready && !self.online {
            let t = self.countdown.unwrap_or(COUNTDOWN);
            if t <= 1 {
                self.start = true;
                self.countdown = None;
            } else {
                self.countdown = Some(t - 1);
            }
        } else {
            self.countdown = None;
        }
    }

    fn slot_text(&self, profile: &Profile, texts: &Texts, slot: usize, world: &str) -> String {
        let s = slot_ref(profile, &self.extra, slot);
        let st = s.stats(world);
        let name = profile.slot_label(slot, &texts.get("slot"));
        if st.prize == 0 && st.clears == 0 {
            format!("{name} (new)")
        } else {
            format!("{name} {}\u{273D}", st.prize)
        }
    }

    fn world_name(&self, w: &str) -> String {
        self.packs
            .iter()
            .find(|p| p.0 == w)
            .map(|p| p.1.clone())
            .unwrap_or_else(|| w.to_string())
    }

    /// Draws the join screen (logical 854 × 480 units).
    pub fn draw(&self, ui: &mut Ui, g: &mut Gfx, texts: &Texts, profile: &Profile) {
        let white = g.pal(0, 3);
        let grey = g.pal(0, 1);
        let dark = g.pal(1, 1);
        ui.fill(g, g.pal(1, 0));
        ui.text(
            g,
            None,
            if self.online {
                "Players on this machine"
            } else {
                "Multiplayer"
            },
            427.0,
            6.0,
            30.0,
            g.pal(4, 1),
            Align::Center,
        );
        let mode = match self.mode {
            MpMode::Coop => "Co-op: one level, everybody moves the same YOU",
            MpMode::SharedSave => "Shared save: one save, own map cursor and level each",
            MpMode::OwnGames => "Own games: own save each, side by side",
        };
        let line = match self.countdown.or(self.online_countdown) {
            Some(t) => format!("{mode}   -   starting in {}", t / 15 + 1),
            None => mode.to_string(),
        };
        ui.text(g, None, &line, 427.0, 44.0, 15.0, white, Align::Center);
        let (cw, ch) = (202.0, 118.0);
        let x0 = (854.0 - (4.0 * cw + 3.0 * 8.0)) / 2.0;
        for k in 0..MAX_PLAYERS {
            let x = x0 + (k % 4) as f32 * (cw + 8.0);
            let y = 70.0 + (k / 4) as f32 * (ch + 6.0);
            let seat = self.players.seats.get(k);
            let card = seat.and_then(|s| self.cards.iter().find(|c| c.id == s.id));
            let (Some(seat), Some(card)) = (seat, card) else {
                ui.rect(g, x, y, cw, ch, dark);
                ui.text(
                    g,
                    None,
                    "press A, E or Enter",
                    x + cw / 2.0,
                    y + ch / 2.0 - 8.0,
                    12.0,
                    grey,
                    Align::Center,
                );
                continue;
            };
            let c = g.pal(seat.colour().0, seat.colour().1);
            ui.rect(g, x, y, cw, ch, c);
            ui.rect(g, x + 3.0, y + 3.0, cw - 6.0, ch - 6.0, g.pal(1, 0));
            ui.text(
                g,
                None,
                &format!("P{} {}", seat.id + 1, seat.name()),
                x + 8.0,
                y + 5.0,
                16.0,
                c,
                Align::Left,
            );
            let src = seat.source.map(|s| s.describe()).unwrap_or("unplugged");
            ui.text(
                g,
                None,
                src,
                x + cw - 8.0,
                y + 8.0,
                11.0,
                grey,
                Align::Right,
            );
            let own = self.mode == MpMode::OwnGames;
            let (slot, world) = if own {
                (seat.slot, seat.world.clone())
            } else {
                (self.slot, self.world.clone())
            };
            let rows = self.rows(seat.id);
            for (i, row) in rows.iter().enumerate() {
                let ry = y + 28.0 + i as f32 * 14.5;
                let (label, value) = match row {
                    Row::Mode => (
                        "Mode",
                        match self.mode {
                            MpMode::Coop => "co-op".to_string(),
                            MpMode::SharedSave => "shared save".to_string(),
                            MpMode::OwnGames => "own games".to_string(),
                        },
                    ),
                    Row::Slot => ("Save", self.slot_text(profile, texts, slot, &world)),
                    Row::World => ("World", self.world_name(&world)),
                    Row::Colour => ("Colour", "\u{25A0}\u{25A0}\u{25A0}".to_string()),
                    Row::Name => ("Name", seat.name().to_string()),
                    Row::Ready => (
                        "",
                        if card.ready {
                            "READY".to_string()
                        } else {
                            "not ready".to_string()
                        },
                    ),
                };
                let sel = card.row == *row;
                if sel {
                    ui.text(g, None, ">", x + 7.0, ry, 12.0, c, Align::Left);
                }
                let col = if *row == Row::Ready && card.ready {
                    g.pal(5, 3)
                } else if sel {
                    white
                } else {
                    grey
                };
                ui.text(g, None, label, x + 18.0, ry, 12.0, col, Align::Left);
                let vc = if *row == Row::Colour { c } else { col };
                let vx = if label.is_empty() { x + 18.0 } else { x + 70.0 };
                let value: String = value.chars().take(20).collect();
                ui.text(g, Some(texts), &value, vx, ry, 12.0, vc, Align::Left);
            }
        }
        let help = if self.online {
            "join: A / E / Enter    ready: A    leave: B    rows: up/down, left/right    host: F5 starts now    Esc: leave the room"
        } else {
            "join: A / E / Enter    ready: A    leave: B    up/down: row    left/right: change    zoom: Back / F2    online: O / Y"
        };
        ui.text(g, None, help, 427.0, 448.0, 11.0, grey, Align::Center);
        ui.text(
            g,
            None,
            "keys left: WASD  E wait  Q undo  R restart  Tab pause      keys right: arrows  Enter wait  Bksp undo  0 restart  P pause",
            427.0,
            463.0,
            11.0,
            grey,
            Align::Center,
        );
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

    fn button(id: u32, b: Button, down: bool) -> Event {
        Event::Button {
            device: DeviceId::Controller(id),
            button: b,
            down,
        }
    }

    /// Commands from several devices come out in the order the events
    /// arrived, each tagged with its seat.
    #[test]
    fn arrival_order_and_seats() {
        let mut p = Players::new(&Settings::defaults(), "baba");
        // Joins.
        p.event(&button(7, Button::South, true));
        p.event(&key(Key::Enter, true));
        p.event(&key(Key::Space, true));
        assert_eq!(
            p.take_happenings(),
            vec![
                Happening::Join(Source::Pad(7)),
                Happening::Join(Source::Keys(Half::Right)),
                Happening::Join(Source::Keys(Half::Left)),
            ]
        );
        assert_eq!(p.add(Source::Pad(7)), Some(0));
        assert_eq!(p.add(Source::Keys(Half::Right)), Some(1));
        assert_eq!(p.add(Source::Keys(Half::Left)), Some(2));
        // Interleaved presses.
        p.event(&button(7, Button::South, false));
        p.event(&key(Key::Enter, false));
        p.event(&key(Key::D, true));
        p.event(&button(7, Button::DPadUp, true));
        p.event(&key(Key::Left, true));
        p.event(&button(7, Button::East, true));
        p.event(&key(Key::Q, true));
        p.event(&key(Key::Char(b'0'), true));
        assert_eq!(
            p.take(),
            vec![
                (2, Cmd::Dir(0)),
                (0, Cmd::Dir(1)),
                (1, Cmd::Dir(2)),
                (0, Cmd::Undo),
                (2, Cmd::Undo),
                (1, Cmd::Restart),
            ]
        );
        // Unbound keys of the other half do nothing; F2 cycles the focus.
        p.event(&key(Key::M, true));
        p.event(&key(Key::F(2), true));
        assert!(p.take().is_empty());
        assert_eq!(p.take_happenings(), vec![Happening::FocusCycle]);
    }

    #[test]
    fn repeat_per_seat_and_unplug() {
        let s = Settings::defaults();
        let mut p = Players::new(&s, "baba");
        p.add(Source::Pad(1));
        p.add(Source::Pad(2));
        p.event(&button(1, Button::DPadRight, true));
        p.event(&button(2, Button::DPadDown, true));
        assert_eq!(p.take(), vec![(0, Cmd::Dir(0)), (1, Cmd::Dir(3))]);
        for _ in 0..s.delay() {
            p.tick();
        }
        assert_eq!(p.take(), vec![(0, Cmd::Dir(0)), (1, Cmd::Dir(3))]);
        p.event(&Event::ControllerRemoved(DeviceId::Controller(2)));
        assert_eq!(p.take_happenings(), vec![Happening::Disconnected(1)]);
        for _ in 0..s.delay() {
            p.tick();
        }
        assert_eq!(p.take(), vec![(0, Cmd::Dir(0))]);
        // A new controller takes the seat over.
        p.assign(1, Source::Pad(9));
        p.event(&button(9, Button::South, true));
        assert_eq!(p.take(), vec![(1, Cmd::Idle), (1, Cmd::Confirm)]);
    }

    #[test]
    fn twelve_seats_and_free_slots() {
        let mut p = Players::new(&Settings::defaults(), "baba");
        for i in 0..14 {
            let r = p.add(Source::Pad(100 + i));
            assert_eq!(r.is_some(), i < 12);
        }
        let colours: std::collections::BTreeSet<usize> = p.seats.iter().map(|s| s.colour).collect();
        assert_eq!(colours.len(), 12);
        for (k, s) in p.seats.iter_mut().enumerate() {
            s.slot = k;
        }
        p.remove(4);
        assert_eq!(p.add(Source::Pad(200)), Some(4));
        assert_eq!(p.free_slot(4, 0), 4);
    }

    #[test]
    fn lobby_join_ready_start() {
        let mut l = Lobby::new(
            &Settings::defaults(),
            vec![("baba".into(), "Baba".into())],
            0,
            ExtraSlots::default(),
        );
        l.event(&button(3, Button::South, true));
        l.event(&key(Key::Enter, true));
        l.tick();
        assert_eq!(l.cards.len(), 2);
        // Host: the mode row (up from Ready wraps through the rows).
        l.event(&button(3, Button::South, false));
        l.event(&button(3, Button::DPadUp, true));
        l.tick();
        l.event(&button(3, Button::DPadUp, false));
        for _ in 0..4 {
            l.event(&button(3, Button::DPadUp, true));
            l.event(&button(3, Button::DPadUp, false));
        }
        l.tick();
        assert_eq!(l.cards[0].row, Row::Mode);
        l.event(&button(3, Button::DPadRight, true));
        l.event(&button(3, Button::DPadRight, false));
        l.event(&button(3, Button::DPadRight, true));
        l.tick();
        assert_eq!(l.mode, MpMode::OwnGames);
        let slots: Vec<usize> = l.players.seats.iter().map(|s| s.slot).collect();
        assert_ne!(slots[0], slots[1]);
        // Both ready → countdown → start.
        l.event(&button(3, Button::South, true));
        l.event(&key(Key::Enter, false));
        l.event(&key(Key::Enter, true));
        for _ in 0..COUNTDOWN + 2 {
            l.tick();
        }
        assert!(l.start);
    }
}
