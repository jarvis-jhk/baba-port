//! A running local multiplayer session.
//!
//! * **Instances and views.** Mode 1 has one instance (a [`Play`]) shown
//!   full screen, driven by every seat; modes 2/3 have one instance per
//!   seat, tiled ([`crate::split`]). A [`View`] is an instance plus its
//!   presentation: transition, title card, victory delay, menu.
//! * **Turns.** Commands arrive as `(seat, command)` in SDL event order
//!   ([`Players`]); each becomes at most one [`Turn`] applied at once
//!   through `Play::apply` (moves, undo) or at the cover of a transition
//!   (enter, leave, restart, proceed). In mode 1 a turn from a seat that
//!   arrives while the instance is busy (transition, title card,
//!   animation, victory delay) is dropped, like a key press in single
//!   player.
//! * **Gating.** Undo is shared and immediate (anyone undoes the last turn,
//!   whoever made it); restart asks the presser (unless "do not prompt"),
//!   then restarts for everyone; any seat may enter a level from the map.
//!   Pause in mode 1 is global (the opener drives the menu; the others
//!   wait); in modes 2/3 it is per view and the other views keep going.
//!   "End session" is offered to the host (the lowest seat) only.
//! * **Shared save (mode 2).** All instances play on one slot. Progress
//!   (wins, gates, conversions) is written by the instance that made it;
//!   every other instance standing on a map re-reads it at once
//!   (`Play::refresh_map`, cursor kept; newly met gates animate), or as
//!   soon as its own animation ends. Level tree, `Previous` and
//!   `levelsurrounds` are per player (`Play::own_nav`; the first seat's go
//!   to the slot so Continue resumes there). Play time counts once (first
//!   view).
//! * **Audio.** One music track: the zoomed view's, else the first view's
//!   (its no-player fade too). Turn sounds come from every instance; per
//!   tick each sound family plays once and at most six sounds play
//!   (`AudioDirector::set_mixing`); `_short` variants follow each
//!   instance's own previous turn (the sim's sound slots).
//! * **Endings** do not leave the session: the view shows a banner and
//!   stays in the level (single player rolls the credits).
//! * **Online** (M7, [`crate::session::online`]): the same session with a
//!   [`NetSide`]. Turns made here are not applied but sent (the outbox);
//!   every instance changes only when the replica applies a stream entry
//!   ([`Multi::apply_now`]). Seats are session players (local ones with a
//!   device, remote ones without), views play on their own copy of the
//!   save ([`View::mem`]); a transition waits at full cover until its
//!   turn comes back, and turns from other machines that load a level
//!   show the opening half of a transition.

use std::path::Path;

use baba_formats::ObjectTable;
use baba_progress::{Profile, Slot, Toggle};
use baba_render::{Fit, PostEffect, Rect, RegionId, Renderer, ScaleMode, View as RView};
use baba_sim::CosmeticRng;

use crate::audio::AudioDirector;
use crate::credits::Kind as CreditsKind;
use crate::fx::transition::Iris;
use crate::input::Cmd;
use crate::join::{Happening, Lobby, Players, Seat};
use crate::lang::Texts;
use crate::menu::{Act, Item, Menu, Style};
use crate::play::{Env, Play};
use crate::session::online::NetSide;
use crate::session::saves::{self, ExtraSlots};
use crate::session::{Applied, Fx, MpMode, Pending, Turn};
use crate::split;
use crate::ui::{Align, Gfx, Ui};
use crate::world::WorldInfo;

const WIN_FRAMES: u32 = 80;
/// Online: longest wait at full cover for our own turn to come back.
const COVER_WAIT: u32 = 600;

/// What the session needs from the app.
pub struct Cx<'a> {
    pub data: &'a Path,
    pub objects: &'a ObjectTable,
    pub gfx: &'a mut Gfx,
    pub texts: &'a Texts,
    pub profile: &'a mut Profile,
    pub audio: &'a mut AudioDirector,
    pub cursor: Option<RegionId>,
}

/// A view's transition: the blob iris laid out for the view's size; the
/// turn is applied at full cover (`presentation.md` §11, §15.9).
struct Trans {
    iris: Iris,
    turn: Option<Turn>,
    /// The seat whose action started it (online: who sends the turn).
    seat: Option<usize>,
    /// Online: ticks waited at full cover for the turn to come back (0 =
    /// not waiting).
    waiting: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum MpMenu {
    Pause,
    RestartConfirm,
    Select(Vec<(String, i32)>),
}

struct OpenMenu {
    id: MpMenu,
    sel: (usize, usize),
    owner: usize,
}

/// One instance and its presentation.
pub struct View {
    /// Seat ids playing this instance (mode 1: everybody).
    pub seats: Vec<usize>,
    pub play: Play,
    /// Save slot of the instance.
    pub slot: usize,
    trans: Option<Trans>,
    menu: Option<OpenMenu>,
    /// Mode 2: the shared save changed; re-read the map when possible.
    stale: bool,
    pub fit: Option<Fit>,
    /// Online: the instance's own copy of its save (replicated, not this
    /// machine's slot file); `None` = the profile's slot `slot`.
    pub mem: Option<Slot>,
    /// Online: write `mem` to this machine's slot file `disk` (the
    /// instance belongs to this machine).
    pub disk: Option<usize>,
    /// Online: the player left; the instance is kept (late stream
    /// entries) but not shown.
    pub hidden: bool,
    /// Online, mode 1: shown instead of the confirmed level while our own
    /// turns are on their way to the host (confirmed + pending turns;
    /// `docs/net.md` "Mode 1 prediction"). Never fed back.
    pub predict: Option<baba_sim::Level>,
    /// Size of the view's rectangle at the last render (transitions are
    /// laid out for it).
    size: (f32, f32),
    /// Frames since a turn left no player (the undo hint, `ui.md` §6.3).
    no_player: Option<u32>,
    /// Frames since the last auto turn (LEVEL IS AUTO).
    auto_t: u32,
    rng: CosmeticRng,
}

impl View {
    fn new(seats: Vec<usize>, play: Play, slot: usize) -> View {
        let rng = CosmeticRng::new(0xB10B ^ ((slot as u64) << 8) ^ seats.len() as u64);
        View {
            seats,
            play,
            slot,
            trans: None,
            menu: None,
            stale: false,
            fit: None,
            mem: None,
            disk: None,
            hidden: false,
            predict: None,
            size: (854.0, 480.0),
            no_player: None,
            auto_t: 0,
            rng,
        }
    }

    /// An online instance on its own copy of a save.
    pub fn with_mem(seats: Vec<usize>, play: Play, mem: Slot, disk: Option<usize>) -> View {
        let mut v = View::new(seats, play, disk.unwrap_or(0));
        v.mem = Some(mem);
        v.disk = disk;
        v
    }

    /// The save the instance plays on.
    pub fn slot_ref<'a>(&'a self, profile: &'a Profile, extra: &'a ExtraSlots) -> &'a Slot {
        match &self.mem {
            Some(s) => s,
            None => saves::slot(profile, extra, self.slot),
        }
    }

    /// A transition (at full cover) waits for its turn to come back.
    pub fn waiting(&self) -> bool {
        self.trans.as_ref().is_some_and(|t| t.waiting > 0)
    }

    /// Input is blocked (transition, title card, animation, victory delay).
    pub fn busy(&self) -> bool {
        self.trans.is_some()
            || self.play.title.is_some()
            || self.play.anim.is_some()
            || self.play.won.is_some()
            || matches!(self.play.pending, Some(Pending::Showcase(_)))
    }

    fn start_trans(&mut self, turn: Turn, fast: bool, seat: Option<usize>) {
        let (w, h) = self.size;
        self.trans = Some(Trans {
            iris: Iris::new(w, h, fast, &mut self.rng),
            turn: Some(turn),
            seat,
            waiting: 0,
        });
    }

    /// The opening half of a transition (a turn from another machine
    /// loaded something).
    fn open_trans(&mut self, fast: bool) {
        let (w, h) = self.size;
        let mut iris = Iris::new(w, h, fast, &mut self.rng);
        while !iris.tick() {}
        self.trans = Some(Trans {
            iris,
            turn: None,
            seat: None,
            waiting: 0,
        });
    }
}

/// The session.
pub struct Multi {
    pub mode: MpMode,
    pub players: Players,
    pub views: Vec<View>,
    /// Seats that left (with their view in modes 2/3), for rejoining.
    departed: Vec<(Seat, Option<View>)>,
    /// Zoomed view, by seat id.
    pub focus: Option<usize>,
    pub extra: ExtraSlots,
    /// Modes 1/2: the session's world and slot.
    pub world: String,
    pub slot: usize,
    /// Back to the title screen.
    pub end: bool,
    pub log: Vec<String>,
    /// Turns applied (all views).
    pub turns: u64,
    /// Online play ([`crate::session::online`]).
    pub net: Option<NetSide>,
    /// Online: views of this machine's players large, the others in a
    /// strip (F3 switches to equal tiles).
    pub local_large: bool,
}

fn start_play(
    cx: &mut Cx,
    extra: &mut ExtraSlots,
    world: &str,
    slot: usize,
    own: Option<Slot>,
) -> Result<Play, String> {
    let w = WorldInfo::open(cx.data, world).ok_or_else(|| format!("no world {world}"))?;
    let mut env = Env {
        data_dir: cx.data,
        objects: cx.objects,
        atlas: &mut cx.gfx.atlas,
    };
    let s = saves::slot_mut(cx.profile, extra, slot);
    let p = Play::start_with(&mut env, w, slot, s, own)?;
    saves::save(cx.profile, extra, slot);
    Ok(p)
}

impl Multi {
    /// Starts the session the join screen set up.
    pub fn start(lobby: Lobby, cx: &mut Cx) -> Result<Multi, String> {
        let Lobby {
            mut players,
            mode,
            slot,
            world,
            mut extra,
            ..
        } = lobby;
        players.world = world.clone();
        let mut views = Vec::new();
        match mode {
            MpMode::Coop => {
                let play = start_play(cx, &mut extra, &world, slot, None)?;
                let seats = players.seats.iter().map(|s| s.id).collect();
                views.push(View::new(seats, play, slot));
            }
            MpMode::SharedSave => {
                for (k, s) in players.seats.iter().enumerate() {
                    let own = (k > 0).then(|| saves::slot(cx.profile, &extra, slot).clone());
                    let play = start_play(cx, &mut extra, &world, slot, own)?;
                    views.push(View::new(vec![s.id], play, slot));
                }
            }
            MpMode::OwnGames => {
                for s in &players.seats {
                    let play = start_play(cx, &mut extra, &s.world, s.slot, None)?;
                    views.push(View::new(vec![s.id], play, s.slot));
                }
            }
        }
        cx.audio.set_mixing(true);
        let mut m = Multi {
            mode,
            players,
            views,
            departed: Vec::new(),
            focus: None,
            extra,
            world,
            slot,
            end: false,
            log: Vec::new(),
            turns: 0,
            net: None,
            local_large: true,
        };
        let seats: Vec<String> = m
            .players
            .seats
            .iter()
            .map(|s| format!("p{}", s.id + 1))
            .collect();
        m.log.push(format!(
            "session {:?} {} slot {} players {}",
            m.mode,
            m.world,
            m.slot,
            seats.join(",")
        ));
        for v in &mut m.views {
            m.log.append(&mut v.play.log);
        }
        Ok(m)
    }

    fn view_of(&self, seat: usize) -> Option<usize> {
        self.views.iter().position(|v| v.seats.contains(&seat))
    }

    /// The instance a seat plays.
    pub fn play_of(&self, seat: usize) -> Option<&Play> {
        self.view_of(seat).map(|i| &self.views[i].play)
    }

    /// The view that sets the music (the zoomed one, else the first).
    fn lead(&self) -> usize {
        self.focus.and_then(|s| self.view_of(s)).unwrap_or(0)
    }

    fn counts_time(&self, vi: usize) -> bool {
        self.mode == MpMode::OwnGames || vi == 0
    }

    fn save_view(&mut self, vi: usize, cx: &mut Cx) {
        let counts = self.counts_time(vi);
        let v = &mut self.views[vi];
        let f = std::mem::take(&mut v.play.frames);
        let wid = v.play.world.id.clone();
        if let Some(mem) = &v.mem {
            // Online: only the machine an instance belongs to writes it
            // (its slot file keeps its own play time).
            let Some(d) = v.disk else {
                return;
            };
            let s = saves::slot_mut(cx.profile, &mut self.extra, d);
            let (t, tf) = (s.store.int(&wid, "time"), s.store.int(&wid, "time_frames"));
            *s = mem.clone();
            s.store.remove(&wid, "time");
            s.store.remove(&wid, "time_frames");
            if t > 0 || tf > 0 {
                s.store.set(&wid, "time", t);
                s.store.set(&wid, "time_frames", tf);
            }
            s.add_frames(&wid, f);
            saves::save(cx.profile, &self.extra, d);
            return;
        }
        let s = saves::slot_mut(cx.profile, &mut self.extra, v.slot);
        if counts {
            s.add_frames(&wid, f);
        }
        saves::save(cx.profile, &self.extra, v.slot);
    }

    /// Saves one instance.
    pub fn save_one(&mut self, vi: usize, cx: &mut Cx) {
        if vi < self.views.len() {
            self.save_view(vi, cx);
        }
    }

    /// An online session (no players yet: they come with the stream).
    pub fn new_net(
        mode: MpMode,
        players: Players,
        world: &str,
        extra: ExtraSlots,
        net: NetSide,
        cx: &mut Cx,
    ) -> Multi {
        cx.audio.set_mixing(true);
        Multi {
            mode,
            players,
            views: Vec::new(),
            departed: Vec::new(),
            focus: None,
            extra,
            world: world.to_string(),
            slot: 0,
            end: false,
            log: Vec::new(),
            turns: 0,
            net: Some(net),
            local_large: true,
        }
    }

    /// Saves every instance (session end).
    pub fn save_all(&mut self, cx: &mut Cx) {
        for vi in 0..self.views.len() {
            self.save_view(vi, cx);
        }
    }

    // -------------------------------------------------------------- tick

    /// One 1/60 s step.
    pub fn tick(&mut self, cx: &mut Cx) {
        cx.audio.begin_tick();
        self.players.tick();
        for h in self.players.take_happenings() {
            self.happen(h, cx);
        }
        for vi in 0..self.views.len() {
            self.tick_view(vi, cx);
        }
        for (seat, cmd) in self.players.take() {
            self.command(seat, cmd, cx);
            if self.end {
                return;
            }
        }
        if self.mode == MpMode::SharedSave && self.net.is_none() {
            for vi in 0..self.views.len() {
                if !self.views[vi].stale {
                    continue;
                }
                let v = &mut self.views[vi];
                let s = saves::slot_mut(cx.profile, &mut self.extra, v.slot);
                if let Some(changed) = v.play.refresh_map(s) {
                    v.stale = false;
                    if changed {
                        cx.audio.sound("whoosh_alt1");
                        self.log.append(&mut v.play.log);
                    }
                }
            }
        }
        if let Some(v) = self.views.get(self.lead()) {
            let m = &v.play.cur.meta;
            cx.audio
                .level_music(&v.play.world.id, &m.music, m.local_music);
        }
        if self.net.is_some() {
            self.net_tick();
        } else if self.views.is_empty() || self.players.seats.is_empty() {
            self.end = true;
        }
    }

    /// Online: the seat that sends a turn of view `vi` started by `seat`
    /// (`None`: nobody pressed anything, e.g. Proceed after a win). Only
    /// this machine's players send; a turn nobody pressed is sent by the
    /// view's owner (modes 2/3) or the leader (mode 1).
    fn sender(&self, vi: usize, seat: Option<usize>) -> Option<usize> {
        let net = self.net.as_ref()?;
        if let Some(s) = seat {
            return net.local.contains_key(&s).then_some(s);
        }
        match self.mode {
            MpMode::Coop => {
                if net.leader {
                    net.local.keys().next().copied()
                } else {
                    None
                }
            }
            _ => self
                .views
                .get(vi)?
                .seats
                .first()
                .copied()
                .filter(|s| net.local.contains_key(s)),
        }
    }

    /// Online, every tick: a view left with a pending win, conversion or
    /// ending (its victory delay over, or the machine that would have sent
    /// Proceed is gone) gets its closing transition from the machine
    /// responsible for it now.
    fn net_tick(&mut self) {
        for vi in 0..self.views.len() {
            let v = &self.views[vi];
            let stuck = v.play.pending.is_some()
                && v.play.won.is_none()
                && v.play.anim.is_none()
                && v.trans.is_none()
                && v.menu.is_none();
            if stuck && self.sender(vi, None).is_some() {
                self.views[vi].start_trans(Turn::Proceed, false, None);
            }
        }
    }

    fn tick_view(&mut self, vi: usize, cx: &mut Cx) {
        let v = &mut self.views[vi];
        v.play.frames += 1;
        // Each view's own effects (tweens, particles, shake; M5).
        let s = &cx.profile.settings;
        let fx = &mut v.play.cur.fx;
        fx.user = crate::fx::FxOptions {
            particles: !s.toggle(Toggle::NoParticles),
            shake: !s.toggle(Toggle::NoShake),
            rule_effect: true,
        };
        fx.tick(&v.play.cur.level);
        if let Some((_, t)) = &mut v.play.banner {
            *t = t.saturating_sub(1);
            if *t == 0 {
                v.play.banner = None;
            }
        }
        if let Some(n) = &mut v.no_player {
            *n += 1;
        }
        if let Some(tr) = &mut v.trans {
            if tr.waiting > 0 {
                // Online: hold the cover until our turn comes back.
                tr.waiting += 1;
                if tr.waiting > COVER_WAIT {
                    tr.waiting = 0;
                }
                return;
            }
            // Closing, covered (the turn; the title card counts down under
            // the cover), opening: as in single player.
            let turn = if tr.iris.tick() { tr.turn.take() } else { None };
            let seat = tr.seat;
            if let Some(turn) = turn {
                v.menu = None;
                self.apply(vi, seat, turn, cx);
            }
            let v = &mut self.views[vi];
            if let Some(tr) = &mut v.trans {
                if tr.iris.covered() && tr.turn.is_none() {
                    match &mut v.play.title {
                        Some(t) if *t > 1 => *t -= 1,
                        Some(_) => {
                            v.play.title = None;
                            tr.iris.release();
                        }
                        None => tr.iris.release(),
                    }
                }
                if tr.iris.done() {
                    v.trans = None;
                }
            }
            return;
        }
        if let Some(t) = &mut v.play.title {
            *t = t.saturating_sub(1);
            if *t == 0 {
                v.play.title = None;
            }
            return;
        }
        if v.play.anim.is_some() {
            let sounds = v.play.tick_anim();
            for (s, pitch) in sounds {
                cx.audio.sound_pitch(&s, pitch);
            }
            return;
        }
        // Map-clear showcase: the next step once this one has shown
        // (online: `net_tick` sends it from the right machine).
        if matches!(v.play.pending, Some(Pending::Showcase(_))) && v.play.won.is_none() {
            if self.net.is_none() {
                v.start_trans(Turn::Proceed, false, None);
            }
            return;
        }
        if let Some(t) = &mut v.play.won {
            *t = t.saturating_sub(1);
            if *t == 0 {
                v.play.won = None;
                // Online: `net_tick` sends Proceed from the right machine.
                if v.play.pending.is_some() && self.net.is_none() {
                    v.start_trans(Turn::Proceed, false, None);
                }
            }
            return;
        }
        // LEVEL IS AUTO: the view's own auto turns (as single player).
        if v.play.cur.level.auto_active() && v.play.pending.is_none() {
            if v.menu.is_none() {
                v.auto_t += 1;
                let n = v
                    .play
                    .cur
                    .info
                    .ini
                    .get_int("general", "rhythm")
                    .unwrap_or(10)
                    .max(0) as u32
                    + 1;
                if v.auto_t >= n {
                    v.auto_t = 0;
                    self.apply(vi, None, Turn::Auto, cx);
                }
            }
        } else {
            v.auto_t = 0;
        }
    }

    fn happen(&mut self, h: Happening, cx: &mut Cx) {
        match h {
            Happening::Join(src) => {
                if let Some(id) = self
                    .players
                    .seats
                    .iter()
                    .find(|s| s.source.is_none() && !s.remote)
                    .map(|s| s.id)
                {
                    if let Some(n) = &mut self.net
                        && let Some(&local) = n.local.get(&id)
                        && let Some(slot) = n.sources.get_mut(local)
                    {
                        *slot = Some(src);
                    }
                    self.players.assign(id, src);
                    self.log
                        .push(format!("rejoin p{} {}", id + 1, src.describe()));
                    return;
                }
                if let Some(n) = &mut self.net {
                    // Online: a new local player; it gets a seat when the
                    // host admits it (`Joined`).
                    if !n.sources.contains(&Some(src)) {
                        n.joins.push(src);
                        self.log.push(format!("join request {}", src.describe()));
                    }
                    return;
                }
                if let Some(k) = self
                    .departed
                    .iter()
                    .position(|(s, _)| s.source == Some(src))
                {
                    let (seat, view) = self.departed.remove(k);
                    let id = seat.id;
                    self.players.restore(seat, src);
                    match view {
                        Some(v) => {
                            let at = self
                                .views
                                .iter()
                                .position(|w| w.seats[0] > id)
                                .unwrap_or(self.views.len());
                            self.views.insert(at, v);
                        }
                        None => {
                            if let Some(v) = self.views.first_mut() {
                                v.seats.push(id);
                                v.seats.sort();
                            }
                        }
                    }
                    self.log
                        .push(format!("rejoin p{} {}", id + 1, src.describe()));
                    return;
                }
                let Some(id) = self.players.add(src) else {
                    return;
                };
                let r = match self.mode {
                    MpMode::Coop => {
                        if let Some(v) = self.views.first_mut() {
                            v.seats.push(id);
                            v.seats.sort();
                        }
                        Ok(())
                    }
                    MpMode::SharedSave => {
                        let own = saves::slot(cx.profile, &self.extra, self.slot).clone();
                        let (w, s) = (self.world.clone(), self.slot);
                        start_play(cx, &mut self.extra, &w, s, Some(own))
                            .map(|p| self.insert_view(id, p, s))
                    }
                    MpMode::OwnGames => {
                        let used: Vec<usize> = self.views.iter().map(|v| v.slot).collect();
                        let s = (0..saves::ALL_SLOTS)
                            .find(|n| !used.contains(n))
                            .unwrap_or(0);
                        if let Some(seat) = self.players.by_id_mut(id) {
                            seat.slot = s;
                        }
                        let w = self.world.clone();
                        start_play(cx, &mut self.extra, &w, s, None)
                            .map(|p| self.insert_view(id, p, s))
                    }
                };
                match r {
                    Ok(()) => self
                        .log
                        .push(format!("join p{} {}", id + 1, src.describe())),
                    Err(e) => {
                        self.log.push(format!("error: {e}"));
                        self.players.remove(id);
                    }
                }
            }
            Happening::Disconnected(id) => self.log.push(format!("unplugged p{}", id + 1)),
            Happening::Focus(id) => {
                self.focus = if self.focus == Some(id) {
                    None
                } else {
                    Some(id)
                };
                self.log
                    .push(format!("focus {:?}", self.focus.map(|s| s + 1)));
            }
            Happening::FocusCycle => {
                let ids: Vec<usize> = self
                    .views
                    .iter()
                    .filter(|v| !v.hidden)
                    .filter_map(|v| v.seats.first().copied())
                    .collect();
                self.focus = match self.focus.and_then(|f| self.view_of(f)) {
                    None => ids.first().copied(),
                    Some(i) => ids.get(i + 1).copied(),
                };
                if ids.len() < 2 {
                    self.focus = None;
                }
                self.log
                    .push(format!("focus {:?}", self.focus.map(|s| s + 1)));
            }
            Happening::UnseatedBack => {
                // Online, a machine that only watches: Esc leaves.
                if let Some(n) = &mut self.net
                    && n.local.is_empty()
                {
                    n.leave = true;
                }
            }
            Happening::Online | Happening::StartNow => {}
            Happening::Layout => {
                self.local_large = !self.local_large;
                self.log.push(format!("local large {}", self.local_large));
            }
        }
    }

    fn insert_view(&mut self, id: usize, mut play: Play, slot: usize) {
        self.log.append(&mut play.log);
        let at = self
            .views
            .iter()
            .position(|w| w.seats[0] > id)
            .unwrap_or(self.views.len());
        self.views.insert(at, View::new(vec![id], play, slot));
    }

    /// A seat leaves (pause menu "Leave game"); it can rejoin with the
    /// same device and gets its seat (and game) back.
    fn leave(&mut self, seat: usize, cx: &mut Cx) {
        let Some(vi) = self.view_of(seat) else {
            return;
        };
        let view = if self.mode == MpMode::Coop {
            self.views[vi].seats.retain(|s| *s != seat);
            if let Some(m) = &self.views[vi].menu
                && m.owner == seat
            {
                self.views[vi].menu = None;
            }
            None
        } else {
            self.save_view(vi, cx);
            Some(self.views.remove(vi))
        };
        if let Some(s) = self.players.remove(seat) {
            self.departed.push((s, view));
        }
        if self.focus == Some(seat) {
            self.focus = None;
        }
        self.log.push(format!("leave p{}", seat + 1));
    }

    fn command(&mut self, seat: usize, cmd: Cmd, cx: &mut Cx) {
        let Some(vi) = self.view_of(seat) else {
            return;
        };
        let you = match self.mode {
            MpMode::Coop => self.players.by_id(seat).map(|s| s.you()).unwrap_or(1),
            _ => 1,
        };
        if let Some(m) = &self.views[vi].menu {
            if m.owner == seat {
                self.menu_cmd(vi, seat, cmd, cx);
            }
            return;
        }
        let v = &mut self.views[vi];
        if v.busy() {
            return;
        }
        let is_map = v.play.cur.is_map();
        let turn = match cmd {
            Cmd::Dir(d) => Some(Turn::Move { dir: d, you }),
            Cmd::Idle => {
                if is_map && !v.play.cur.candidates(&v.play.world).is_empty() {
                    None
                } else {
                    Some(Turn::Move { dir: 4, you })
                }
            }
            Cmd::Undo => Some(Turn::Undo),
            Cmd::Confirm => {
                if is_map {
                    let cands = v.play.cur.candidates(&v.play.world);
                    match cands.len() {
                        0 => {}
                        1 => {
                            let (target, unit) = cands[0].clone();
                            cx.audio.sound("confirm");
                            v.start_trans(Turn::Enter { target, unit }, false, Some(seat));
                            self.log.push(format!("p{} confirm", seat + 1));
                        }
                        _ => {
                            v.menu = Some(OpenMenu {
                                id: MpMenu::Select(cands),
                                sel: (1, 0),
                                owner: seat,
                            })
                        }
                    }
                }
                None
            }
            Cmd::Restart => {
                if cx.profile.settings.toggle(Toggle::RestartNoAsk) {
                    cx.audio.sound("restart");
                    v.start_trans(Turn::Restart, true, Some(seat));
                } else {
                    v.menu = Some(OpenMenu {
                        id: MpMenu::RestartConfirm,
                        sel: (0, 0),
                        owner: seat,
                    });
                }
                None
            }
            Cmd::Pause | Cmd::Back => {
                v.menu = Some(OpenMenu {
                    id: MpMenu::Pause,
                    sel: (0, 0),
                    owner: seat,
                });
                None
            }
        };
        if let Some(t) = turn {
            self.apply(vi, Some(seat), t, cx);
        }
    }

    /// A turn of view `vi` made here (by `seat`, or by the presentation:
    /// `None`): applied at once, or online sent to the stream (and applied
    /// when it comes back, [`Multi::apply_now`]).
    fn apply(&mut self, vi: usize, seat: Option<usize>, turn: Turn, cx: &mut Cx) {
        if self.net.is_none() {
            self.apply_now(vi, seat, turn, cx);
            return;
        }
        let Some(from) = self.sender(vi, seat) else {
            return;
        };
        let cover = matches!(
            turn,
            Turn::Enter { .. } | Turn::Leave | Turn::Restart | Turn::Proceed
        );
        if cover && let Some(tr) = &mut self.views[vi].trans {
            tr.waiting = 1;
        }
        self.log.push(format!("send p{} {turn:?}", from + 1));
        if let Some(n) = &mut self.net {
            n.outbox.push((from, turn));
        }
    }

    /// Applies a turn to view `vi` and presents the result (online: an
    /// entry of the stream, from any machine). `seat`: who made it.
    pub fn apply_now(
        &mut self,
        vi: usize,
        seat: Option<usize>,
        turn: Turn,
        cx: &mut Cx,
    ) -> Option<Applied> {
        let lead = vi == self.lead();
        let quiet = self.net.as_ref().is_some_and(|n| n.quiet);
        let online = self.net.is_some();
        let v = self.views.get_mut(vi)?;
        let cover = matches!(
            turn,
            Turn::Enter { .. } | Turn::Leave | Turn::Restart | Turn::Proceed
        );
        if online && cover {
            // Our own transition waited at full cover for this turn; a
            // turn from elsewhere shows the opening half.
            match &mut v.trans {
                Some(tr) if tr.waiting > 0 => tr.waiting = 0,
                Some(_) => {}
                None if !quiet => v.open_trans(turn == Turn::Restart),
                None => {}
            }
        }
        let View {
            play, mem, slot, ..
        } = &mut *v;
        let s = match mem {
            Some(m) => m,
            None => saves::slot_mut(cx.profile, &mut self.extra, *slot),
        };
        let mut env = Env {
            data_dir: cx.data,
            objects: cx.objects,
            atlas: &mut cx.gfx.atlas,
        };
        let a = play.apply(&turn, &mut env, s, cx.texts);
        if a.ignored {
            return Some(a);
        }
        self.turns += 1;
        let who = match seat {
            Some(s) => format!("p{}", s + 1),
            None => format!("v{vi}"),
        };
        self.log.push(format!("turn {who} {turn:?}"));
        for l in v.play.log.drain(..) {
            self.log.push(format!("{who} {l}"));
        }
        if quiet {
            // Catching up (late join, resync): states only; the effects
            // start from the caught-up level (no burst of replayed ones).
            v.play.title = None;
            v.play.cur.fx.start(&v.play.cur.level);
            for fx in &a.fx {
                if let Fx::Credits(kind) = fx {
                    self.log.push(format!("{who} credits {kind:?}"));
                }
            }
        }
        for fx in a.fx.iter().filter(|_| !quiet) {
            match fx {
                Fx::UndoSound => cx.audio.sound_variant("undo", 5),
                Fx::Turn(out) => {
                    // `_short` variants follow the instance's own previous
                    // turn (the sim's sound slots).
                    cx.audio.turn(out);
                    let cursor = v.play.cur.is_map() && !v.play.cur.cursors().is_empty();
                    if lead {
                        cx.audio.player_state(out.no_player && !cursor);
                    }
                    // The view's own undo hint (single player: `App`).
                    v.no_player = if out.no_player && !cursor {
                        v.no_player.or(Some(0))
                    } else {
                        None
                    };
                }
                Fx::Banner(b, n) => v.play.banner = Some((b.clone(), *n)),
                Fx::Won => {
                    v.play.banner = Some((cx.texts.get("ingame_victory"), WIN_FRAMES + 30));
                    v.play.won = Some(WIN_FRAMES);
                    cx.audio.sound("winner2");
                }
                Fx::ProceedAfterTransition => {
                    // Online: `net_tick` starts it on the right machine.
                    if !online {
                        v.start_trans(Turn::Proceed, false, None);
                    }
                }
                Fx::Credits(kind) => {
                    let b = match kind {
                        CreditsKind::End => "The End",
                        CreditsKind::Done => "All is done",
                        _ => "Congratulations!",
                    };
                    v.play.banner = Some((b.to_string(), 240));
                    self.log.push(format!("{who} credits {kind:?}"));
                }
                Fx::Loaded => {
                    if turn == Turn::Refresh {
                        cx.audio.sound("whoosh_alt1");
                    }
                }
                Fx::EditorSpecial => {
                    cx.profile.settings.store.set("editor", "special", 1);
                    cx.profile.save_settings();
                }
                // Sessions play worlds, never single levels (`Play::single`).
                Fx::LeaveSingle { .. } => {}
            }
        }
        let slot = v.slot;
        if a.save {
            self.save_view(vi, cx);
        }
        if a.progress && self.mode == MpMode::SharedSave && !online {
            for (k, w) in self.views.iter_mut().enumerate() {
                if k != vi && w.slot == slot {
                    w.stale = true;
                }
            }
        }
        Some(a)
    }

    // ------------------------------------------------------------- menus

    fn build_menu(&self, vi: usize, menu: &OpenMenu, texts: &Texts) -> Menu {
        let t = texts;
        let v = &self.views[vi];
        let p = &v.play;
        let mut m = Menu::new(Style::List);
        match &menu.id {
            MpMenu::Pause => {
                m.top = 6.0;
                m.escape = Act::Resume;
                let name = p.level_name(t, &p.cur.stem);
                let head = match (p.cur.is_map(), p.label(t)) {
                    (false, Some(l)) => format!("{l}: {name}"),
                    _ => name,
                };
                m.header.push(head);
                if let Some(s) = self.players.by_id(menu.owner) {
                    m.header.push(format!("P{} {}", s.id + 1, s.name()));
                }
                m.item(Item::new(t.get("resume"), Act::Resume));
                m.item(
                    Item::new(t.get("pause_returnmap"), Act::ReturnToMap).enabled(p.can_return()),
                );
                m.item(Item::new(t.get("controls_restart"), Act::Restart));
                if let Some(n) = &self.net {
                    m.item(Item::new("Leave online session", Act::LeaveGame));
                    if n.host {
                        m.gap(12.0);
                        m.item(Item::new("End session", Act::ReturnToMenu));
                    }
                } else {
                    m.item(Item::new("Leave game", Act::LeaveGame));
                    if self.players.host() == Some(menu.owner) {
                        m.gap(12.0);
                        m.item(Item::new("End session", Act::ReturnToMenu));
                    }
                }
                let mut lines: Vec<(String, usize)> = Vec::new();
                for r in &p.cur.level.rules().visual {
                    let l = crate::app::rule_line(r);
                    match lines.iter_mut().find(|(x, _)| *x == l) {
                        Some(e) => e.1 += 1,
                        None => lines.push((l, 1)),
                    }
                }
                if !lines.is_empty() {
                    m.footer
                        .push(t.get("rules_colon").replace("(rules_colon)", "Rules:"));
                    for (l, n) in lines.into_iter().take(10) {
                        m.footer.push(if n > 1 { format!("{n} x {l}") } else { l });
                    }
                }
            }
            MpMenu::RestartConfirm => {
                m.top = 150.0;
                m.header.push(t.get("restart_confirm"));
                m.row(vec![
                    Item::new(t.get("no"), Act::Close),
                    Item::new(t.get("yes"), Act::RestartNow),
                ]);
            }
            MpMenu::Select(c) => {
                m.top = 60.0;
                m.header.push(t.get("enterlevel_multiple"));
                m.item(Item::new(t.get("return"), Act::Close));
                for (stem, unit) in c {
                    m.item(Item::new(
                        p.level_name(t, stem),
                        Act::EnterLevel(stem.clone(), *unit),
                    ));
                }
            }
        }
        m
    }

    fn menu_cmd(&mut self, vi: usize, seat: usize, cmd: Cmd, cx: &mut Cx) {
        let Some(open) = self.views[vi].menu.take() else {
            return;
        };
        let menu = self.build_menu(vi, &open, cx.texts);
        let mut sel = open.sel;
        let res = menu.navigate(&mut sel, cmd);
        self.views[vi].menu = Some(OpenMenu { sel, ..open });
        if matches!(cmd, Cmd::Dir(_)) && res.is_none() {
            cx.audio.sound_variant("click_", 5);
        }
        let Some((act, _)) = res else {
            return;
        };
        let v = &mut self.views[vi];
        match act {
            Act::Resume | Act::Close => v.menu = None,
            Act::ReturnToMap => {
                v.menu = None;
                v.start_trans(Turn::Leave, false, Some(seat));
            }
            Act::Restart => {
                if cx.profile.settings.toggle(Toggle::RestartNoAsk) {
                    v.menu = None;
                    cx.audio.sound("restart");
                    v.start_trans(Turn::Restart, true, Some(seat));
                } else {
                    v.menu = Some(OpenMenu {
                        id: MpMenu::RestartConfirm,
                        sel: (0, 0),
                        owner: seat,
                    });
                }
            }
            Act::RestartNow => {
                v.menu = None;
                cx.audio.sound("restart");
                v.start_trans(Turn::Restart, true, Some(seat));
            }
            Act::EnterLevel(target, unit) => {
                v.menu = None;
                cx.audio.sound("confirm");
                v.start_trans(Turn::Enter { target, unit }, false, Some(seat));
            }
            Act::LeaveGame => {
                v.menu = None;
                if let Some(n) = &mut self.net {
                    n.leave = true;
                    self.log.push(format!("leave online p{}", seat + 1));
                    return;
                }
                self.leave(seat, cx);
            }
            Act::ReturnToMenu => {
                v.menu = None;
                if let Some(n) = &mut self.net {
                    n.end = true;
                    self.log.push(format!("end online session p{}", seat + 1));
                    return;
                }
                self.save_all(cx);
                self.log.push(format!("end session p{}", seat + 1));
                self.end = true;
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------ render

    /// Draws every view into `size` and their overlays into `ui`.
    pub fn render(
        &mut self,
        cx: &mut Cx,
        r: &mut Renderer,
        ui: &mut Ui,
        size: (u32, u32),
        wobble: u32,
        scale: ScaleMode,
    ) {
        let (w, h) = size;
        // Shown views (online: players who left keep a hidden instance).
        let vis: Vec<usize> = (0..self.views.len())
            .filter(|i| !self.views[*i].hidden)
            .collect();
        let focus = self
            .focus
            .and_then(|s| self.view_of(s))
            .and_then(|f| vis.iter().position(|i| *i == f));
        let shown = match (&self.net, focus) {
            // Online: this machine's views large, the others in a strip.
            (Some(n), None) if self.local_large && self.mode != MpMode::Coop => {
                let big: Vec<usize> = (0..vis.len())
                    .filter(|k| {
                        self.views[vis[*k]]
                            .seats
                            .first()
                            .is_some_and(|s| n.local.contains_key(s))
                    })
                    .collect();
                split::layout_big(w as i32, h as i32, vis.len(), &big)
            }
            _ => split::layout(w as i32, h as i32, vis.len(), focus),
        };
        let mut rects: Vec<Option<Rect>> = vec![None; self.views.len()];
        for (k, i) in vis.iter().enumerate() {
            rects[*i] = shown.get(k).copied();
        }
        let coop_menu = self.mode == MpMode::Coop;
        for (i, v) in self.views.iter_mut().enumerate() {
            let Some(rect) = rects.get(i).copied().flatten() else {
                continue;
            };
            let states = v.play.cur.path_states();
            // Online mode 1 shows the predicted level while our turns are
            // on their way; its effects stay those of the confirmed one.
            let g = &mut *cx.gfx;
            let cur = &mut v.play.cur;
            let level = v.predict.as_ref().unwrap_or(&cur.level);
            let scene = cur.gfx.scene(
                &mut g.atlas,
                cx.data,
                level,
                &states,
                Some(&mut cur.fx),
                &mut g.art,
                wobble,
            );
            r.sync_atlas(&mut cx.gfx.atlas);
            let dim = v.menu.is_some() || v.play.title.is_some();
            // Shake, scroll and rotation of this view (M5), in pixels of
            // its last fit.
            let fx = &v.play.cur.fx;
            let (sx, sy) = fx.shake_offset();
            let (ox, oy) = fx.view_scroll();
            let k = v.fit.map_or(1.0, |f| f.scale);
            let view = RView {
                scene: &scene,
                wobble,
                rect,
                scale,
                effect: PostEffect {
                    tint: if dim { [90, 90, 90, 255] } else { [255; 4] },
                    offset: [(sx as f32 + ox * 24.0) * k, (sy as f32 + oy * 24.0) * k],
                    rotation: -fx.view_rotation().to_radians(),
                    ..Default::default()
                },
            };
            let fit = r.draw_view(i, &cx.gfx.atlas, &view);
            v.fit = Some(fit);
            v.size = (rect.w.max(1) as f32, rect.h.max(1) as f32);
            // 3D mode: the first-person view over this view's level (§8).
            crate::overlays::vision(
                r,
                cx.gfx,
                &v.play.cur,
                fit.rect,
                fit.scale,
                scene.level_bg,
                wobble,
            );
        }
        let many = vis.len() > 1;
        for (i, v) in self.views.iter().enumerate() {
            let Some(rect) = rects.get(i).copied().flatten() else {
                continue;
            };
            if many {
                let c = self
                    .players
                    .by_id(v.seats.first().copied().unwrap_or(usize::MAX))
                    .map(|s| s.colour())
                    .unwrap_or((0, 1));
                let col = cx.gfx.pal(c.0, c.1);
                let b = 2.0;
                let (x, y, ww, hh) = (rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32);
                ui.rect_px(cx.gfx, x, y, ww, b, col);
                ui.rect_px(cx.gfx, x, y + hh - b, ww, b, col);
                ui.rect_px(cx.gfx, x, y, b, hh, col);
                ui.rect_px(cx.gfx, x + ww - b, y, b, hh, col);
            }
            // Fills and transitions cover the whole view; text and menus
            // sit on the level area (several views) or the screen's
            // 854x480 box (one view, like single player).
            let inner = match v.fit {
                Some(f) if many => clip(f.rect, rect),
                _ => rect,
            };
            let new_ui = |r: Rect| Ui::new(r.w.max(1) as u32, r.h.max(1) as u32);
            let (mut bg, mut mid, mut top) = (new_ui(rect), new_ui(inner), new_ui(rect));
            if many {
                mid.scale = split::ui_scale_exact(inner);
            }
            self.draw_view_overlay(i, [&mut bg, &mut mid, &mut top], cx, coop_menu, wobble);
            for (layer, r) in [(bg, rect), (mid, inner), (top, rect)] {
                for mut q in layer.quads {
                    q.x += r.x as f32;
                    q.y += r.y as f32;
                    ui.quads.push(q);
                }
            }
        }
    }

    /// `layers`: background fills (view), text and menus (level area),
    /// transition (view).
    fn draw_view_overlay(
        &self,
        vi: usize,
        layers: [&mut Ui; 3],
        cx: &mut Cx,
        full_menu: bool,
        wobble: u32,
    ) {
        let [bg, ui, top] = layers;
        let v = &self.views[vi];
        let p = &v.play;
        let g = &mut *cx.gfx;
        let texts = cx.texts;
        let white = g.pal(0, 3);
        let s = v.slot_ref(cx.profile, &self.extra);
        if p.cur.is_map() && p.title.is_none() {
            // Narrow views keep the HUD in their corners (as single player).
            let name = p.hud_name(texts);
            let mut left = ui.edge(false, 640.0);
            left.text(g, Some(texts), &name, 20.0, 2.0, 16.0, white, Align::Left);
            ui.quads.append(&mut left.quads);
            let wid = &p.world.id;
            let counts = [s.prize_total(wid), s.clears_total(wid), s.bonus_total(wid)];
            let mut right = ui.edge(true, 640.0);
            crate::overlays::hud_counters(&mut right, g, counts, wobble);
            ui.quads.append(&mut right.quads);
        }
        if p.title.is_none() && !p.cur.is_map() {
            let lines = crate::overlays::sign_lines(texts, &p.cur.info, &p.cur.level);
            crate::overlays::signs(ui, g, texts, &lines);
        }
        // Player tags, bottom left (several rows with many players).
        let (px, step) = if v.seats.len() > 6 {
            (13.0, 8.0)
        } else {
            (18.0, 11.0)
        };
        let (mut x, mut y) = (8.0, 480.0 - px * 1.35);
        for id in &v.seats {
            let Some(seat) = self.players.by_id(*id) else {
                continue;
            };
            let c = seat.colour();
            let mut tag = format!("P{} {}", seat.id + 1, seat.name());
            if seat.source.is_none() && !seat.remote {
                tag.push_str(" (unplugged)");
            }
            let w = px * 1.6 + step * tag.chars().count() as f32;
            if x > 8.0 && x + w > 760.0 {
                x = 8.0;
                y -= px * 1.3;
            }
            ui.text(g, None, &tag, x, y, px, g.pal(c.0, c.1), Align::Left);
            x += w;
        }
        if let Some((b, _)) = &p.banner {
            ui.text(
                g,
                Some(texts),
                b,
                427.0,
                200.0,
                40.0,
                g.pal(4, 1),
                Align::Center,
            );
        }
        if v.no_player
            .is_some_and(|n| n >= crate::overlays::UNDO_HINT_FRAMES)
            && p.title.is_none()
        {
            crate::overlays::undo_hint(ui, g, texts, &cx.profile.settings, wobble);
        }
        if p.title.is_some() {
            let label = p.label(texts);
            let name = p.level_name(texts, &p.cur.stem);
            crate::overlays::title_card(
                ui,
                Some(bg),
                g,
                label.as_deref(),
                &name,
                &p.cur.meta.subtitle,
            );
        }
        if let Some(m) = &v.menu {
            let mut c = g.pal(1, 0);
            c[3] = 200;
            bg.fill(g, c);
            let menu = self.build_menu(vi, m, texts);
            menu.draw(ui, g, texts, m.sel, cx.cursor);
            if full_menu && let Some(seat) = self.players.by_id(m.owner) {
                let c = seat.colour();
                let t = format!("P{} {} paused", seat.id + 1, seat.name());
                ui.text(
                    g,
                    None,
                    &t,
                    846.0,
                    456.0,
                    16.0,
                    g.pal(c.0, c.1),
                    Align::Right,
                );
            }
        }
        if let Some(tr) = &v.trans {
            crate::overlays::iris(top, g, &tr.iris, p.title.is_none());
        }
    }

    // ----------------------------------------------------------- queries

    /// No view is busy.
    pub fn idle(&self) -> bool {
        self.views.iter().all(|v| !v.busy())
    }

    /// The seat's view is not busy.
    pub fn player_idle(&self, seat: usize) -> bool {
        self.view_of(seat).is_some_and(|i| !self.views[i].busy())
    }

    /// `multi <mode> players=N views=M | v0 p1,p2 world/stem (map) ...`.
    pub fn describe(&self) -> String {
        let mut s = format!(
            "multi {:?} players={} views={}",
            self.mode,
            self.players.seats.len(),
            self.views.len()
        );
        if let Some(f) = self.focus {
            s.push_str(&format!(" focus=p{}", f + 1));
        }
        if let Some(n) = &self.net {
            s.push_str(if n.host { " online host" } else { " online" });
        }
        for (i, v) in self.views.iter().enumerate() {
            let seats: Vec<String> = v.seats.iter().map(|s| format!("p{}", s + 1)).collect();
            s.push_str(&format!(
                " | v{i} {} slot{} {}/{}{}",
                seats.join(","),
                v.slot,
                v.play.world.id,
                v.play.cur.stem,
                if v.play.cur.is_map() { " (map)" } else { "" }
            ));
            if let Some(m) = &v.menu {
                s.push_str(&format!(" menu={:?}", m.id));
            }
            if v.busy() {
                s.push_str(" busy");
            }
        }
        s
    }
}

/// The part of `a` inside `b`.
fn clip(a: Rect, b: Rect) -> Rect {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.w).min(b.x + b.w);
    let y1 = (a.y + a.h).min(b.y + b.h);
    Rect::new(x0, y0, (x1 - x0).max(1), (y1 - y0).max(1))
}
