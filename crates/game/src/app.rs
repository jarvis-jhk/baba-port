//! The platform-agnostic game: boot card, title screen and menus, settings
//! and controls, slot/pack selection, play sessions with pause menu,
//! transitions, title cards, banners, HUD, credits and endings. Driven by
//! any [`Platform`] (SDL2 on desktop, the Android backend, or the
//! headless driver) through [`App::frame`], or tick by tick by scripted
//! tests through [`App::handle_event`] / [`App::tick`] / [`App::render`].

use std::path::{Path, PathBuf};

use baba_formats::ObjectTable;
use baba_platform::{Event, Platform, TouchPhase};
use baba_progress::{Action, PadAction, Profile, Toggle, Zoom};
use baba_render::{Atlas, PostEffect, Rect, RegionId, Renderer, RgbaImage, ScaleMode, View};

use crate::audio::AudioDirector;
use crate::credits::{Credits, Kind as CreditsKind};
use crate::cutscene::Cutscene;
use crate::input::{Capture, Captured, Cmd, Input, key_name, pad_name};
use crate::join::{Lobby, Source};
use crate::lang::Texts;
use crate::menu::{Act, Item, Menu, MenuId, Slider, Style};
use crate::osk;
use crate::play::{Current, Env, Play};
use crate::session::multi::Cx;
use crate::session::saves::ExtraSlots;
use crate::session::{Fx, Multi, Pending, Turn};
use crate::sounds::SoundBank;
use crate::ui::{Align, Gfx, Ui};
use crate::world::{WorldInfo, list_packs};

pub mod editor_ui;
pub mod extras;
use extras::{ExtAct, ExtMenu};
pub mod online;
mod prompt;

/// Logic rate (the original's engine frame rate).
pub const TICK_HZ: f64 = 60.0;
use crate::fx::WOBBLE_TICKS;
const BOOT_FRAMES: u32 = 90;
const WIN_FRAMES: u32 = 80;

/// How the app is set up.
#[derive(Clone, Debug)]
pub struct AppConfig {
    /// The game install (with `Data/`).
    pub game_dir: PathBuf,
    /// Where saves and settings live (`None` = in memory).
    pub data_dir: Option<PathBuf>,
    /// Skip the boot card.
    pub skip_boot: bool,
}

/// What a closing transition does once the screen is covered.
enum Then {
    StartWorld(usize, String),
    Continue,
    /// Apply a turn to the instance being played (enter, leave, restart,
    /// proceed after a win / conversion / ending).
    Turn(Turn),
    ToTitle,
    Credits(CreditsKind),
}

struct Trans {
    iris: crate::fx::transition::Iris,
    then: Option<Then>,
}

enum Mode {
    Boot(u32),
    Title,
    Play(Box<Play>),
    Credits(Credits),
    /// Multiplayer join screen.
    Join(Box<Lobby>),
    /// Local multiplayer session (online too: `Multi::net`).
    Multi(Box<Multi>),
    /// The level editor (state in `App::edh`).
    Editor,
    /// Online room lobby (`app::online`).
    Online,
}

/// The game.
pub struct App {
    pub cfg: AppConfig,
    data: PathBuf,
    objects: ObjectTable,
    renderer: Renderer,
    gfx: Gfx,
    pub texts: Texts,
    pub profile: Profile,
    pub audio: AudioDirector,
    input: Input,
    mode: Mode,
    menus: Vec<(MenuId, (usize, usize))>,
    trans: Option<Trans>,
    title_bg: Option<Current>,
    pub ticks: u64,
    acc: f64,
    last_time: Option<f64>,
    pub quit: bool,
    fullscreen: Option<bool>,
    /// Events for scripted checks (`enter 0level`, `win 0level`, …).
    pub log: Vec<String>,
    pad: Option<String>,
    cursor_sprite: Option<RegionId>,
    fast_trans: bool,
    view_fit: Option<baba_render::Fit>,
    /// Play levels, level codes, import, packs (M9; `extras`).
    ext: extras::ExtState,
    /// The level editor (M8; `editor_ui`).
    edh: editor_ui::EdHost,
    /// Online play (M7; `online`).
    online: online::OnState,
    /// Taps and clicks (screen pixels) since the last tick, and the size of
    /// the last rendered frame to map them onto menus.
    pointer: (Vec<(f32, f32)>, (u32, u32)),
    /// Last drawn screen size (transitions are laid out for it).
    screen: (u32, u32),
    /// Frames since a turn left no player (undo hint, `ui.md` §6.3).
    no_player: Option<u32>,
    cosmetic: baba_sim::CosmeticRng,
    /// The running cutscene (intro, END, All Is Done; `crate::cutscene`).
    cut: Option<Cutscene>,
    cut_tex: Option<crate::cutscene::Tex>,
    /// The next world start plays the intro ("watch the intro?" → Yes).
    intro_next: bool,
    /// Frames a key has been held during the intro (hold to skip).
    hold: u32,
    /// The on-screen keyboard's shift (slot names; one key, then off).
    osk_shift: bool,
    /// Frames since the last auto turn (LEVEL IS AUTO).
    auto_t: u32,
    /// Menu slide-in: the top menu last seen and its offset (logical px).
    slide_top: Option<MenuId>,
    slide: (f32, f32),
}

/// Logical width the title screen needs (logo and the two button columns),
/// and the width the HUD and title card are laid out for on narrow
/// screens (PortMaster 640×480, 720×720).
const TITLE_SPAN: f32 = 600.0;
const HUD_SPAN: f32 = 640.0;

impl App {
    /// The overlay for a `w`×`h` screen: the 854×480 canvas, widened on
    /// narrow screens for the title screen (its content spans the middle
    /// 600 px).
    fn screen_ui(&self, w: u32, h: u32) -> Ui {
        let ui = Ui::new(w, h);
        match self.mode {
            Mode::Title if ui.narrow() => ui.widened(TITLE_SPAN),
            _ => ui,
        }
    }
}

/// Where a menu slides in from (`presentation.md` §15.27; the original's
/// menu data declares it per menu): right `(1, 0)` or bottom `(0, 1)`.
fn slide_dir(id: &MenuId) -> Option<(f32, f32)> {
    use crate::app::editor_ui::EdMenu as E;
    use crate::app::extras::ExtMenu as X;
    match id {
        MenuId::Slots | MenuId::Packs { .. } => Some((1.0, 0.0)),
        MenuId::Ext(X::PlayLevels) => Some((0.0, 1.0)),
        MenuId::Ext(X::GetLevels) => Some((1.0, 0.0)),
        MenuId::Editor(e) => match e {
            E::LevelMenu | E::QuickMenu => Some((0.0, 1.0)),
            E::Help
            | E::Music { .. }
            | E::Particles
            | E::Palettes { .. }
            | E::Size { .. }
            | E::MapSetup
            | E::LevelSetup
            | E::Upload
            | E::EditObject
            | E::Themes
            | E::ThemeList { .. }
            | E::NewTheme { .. }
            | E::Tags { .. }
            | E::Packs { .. } => Some((1.0, 0.0)),
            _ => None,
        },
        _ => None,
    }
}

pub(crate) fn rule_line(r: &str) -> String {
    // "subj verb obj [c1(p|q),c2]" → "C2 SUBJ C1 P & Q VERB OBJ"
    let (main, conds) = match r.split_once(" [") {
        Some((m, c)) => (m, c.trim_end_matches(']')),
        None => (r, ""),
    };
    let mut parts = main.splitn(3, ' ');
    let s = parts.next().unwrap_or("");
    let v = parts.next().unwrap_or("");
    let o = parts.next().unwrap_or("");
    let mut prefix = Vec::new();
    let mut infix = Vec::new();
    if !conds.is_empty() {
        for c in conds.split(',') {
            match c.split_once('(') {
                Some((n, p)) => infix.push(format!(
                    "{n} {}",
                    p.trim_end_matches(')').replace('|', " & ")
                )),
                None => prefix.push(c.to_string()),
            }
        }
    }
    let mut out = String::new();
    for p in prefix {
        out.push_str(&p);
        out.push(' ');
    }
    out.push_str(s);
    if !infix.is_empty() {
        out.push(' ');
        out.push_str(&infix.join(" & "));
    }
    out.push_str(&format!(" {v} {o}"));
    out.replace("text_", "")
}

/// `[specials] controls,<name>` hints (`progression.md` §3.6): a key cap
/// with the bound key and, for `move` (under the down arrow) and the
/// other actions, the localised action name below.
fn controls_overlay(
    ui: &mut Ui,
    g: &mut Gfx,
    texts: &Texts,
    s: &baba_progress::Settings,
    info: &baba_formats::LevelInfo,
    fit: baba_render::Fit,
) {
    let cell = 24.0 * fit.scale;
    let white = g.pal(0, 3);
    for sp in &info.specials {
        let Some(name) = sp.data.strip_prefix("controls,") else {
            continue;
        };
        let base = name.trim_end_matches('2');
        let secondary = name.ends_with('2');
        let action = match base {
            "right" => Action::Right,
            "up" => Action::Up,
            "left" => Action::Left,
            "down" => Action::Down,
            "idle" => Action::Idle,
            "undo" => Action::Undo,
            "restart" => Action::Restart,
            "pause" => Action::Pause,
            "move" => Action::Down,
            _ => continue,
        };
        let code = s.key(action, secondary);
        let cap = match key_name(code).as_str() {
            "up" => "\u{2191}".to_string(),
            "down" => "\u{2193}".to_string(),
            "left" => "\u{2190}".to_string(),
            "right" => "\u{2192}".to_string(),
            "space" => "Space".to_string(),
            "return" => "Enter".to_string(),
            "backspace" => "Bksp".to_string(),
            k => k.to_uppercase(),
        };
        // Cell centre in logical UI units.
        let cx = fit.rect.x as f32 + (sp.x as f32 + 0.5) * cell;
        let cy = fit.rect.y as f32 + (sp.y as f32 + 0.5) * cell;
        let (lx, ly) = ((cx - ui.x(0.0)) / ui.scale, (cy - ui.y(0.0)) / ui.scale);
        let k = fit.scale / ui.scale;
        // The original's key cap images (engine art, presentation.md
        // §12.3); other keys get a drawn cap.
        let image = crate::keycaps::image(&key_name(code));
        if let Some(a) = image.and_then(|n| g.art.get(&mut g.atlas, n)) {
            ui.sprite(
                a.region,
                lx - a.hx as f32 * k,
                ly - a.hy as f32 * k,
                a.w as f32 * k,
                a.h as f32 * k,
                white,
            );
        } else {
            let wide = cap.chars().count() > 1;
            let (w, h) = (if wide { 46.0 } else { 22.0 } * k, 22.0 * k);
            let (x0, y0) = (lx - w / 2.0, ly - h / 2.0);
            let t = 2.0 * k;
            ui.rect(g, x0, y0, w, t, white);
            ui.rect(g, x0, y0 + h - t, w, t, white);
            ui.rect(g, x0, y0, t, h, white);
            ui.rect(g, x0 + w - t, y0, t, h, white);
            let px = if wide { 14.0 } else { 16.0 } * k;
            ui.text(g, None, &cap, lx, ly - px * 0.62, px, white, Align::Center);
        }
        let label = match base {
            "down" | "move" => Some("move"),
            "idle" => Some("idle"),
            "undo" => Some("undo"),
            "restart" => Some("restart"),
            "pause" => Some("pause"),
            _ => None,
        };
        if let Some(l) = label {
            ui.text(
                g,
                Some(texts),
                &texts.get(l),
                if g.fonts.pixel { lx + 2.0 * k } else { lx },
                if g.fonts.pixel {
                    ly + 16.0 * k
                } else {
                    ly + cell / ui.scale * 0.75
                },
                16.0 * k,
                white,
                Align::Center,
            );
        }
    }
}

use crate::overlays::{UNDO_HINT_FRAMES, undo_hint};

impl App {
    /// Builds the app on a current GL context.
    pub fn new(gl: baba_platform::glow::Context, cfg: AppConfig) -> Result<App, String> {
        let renderer = Renderer::new(gl).map_err(|e| e.to_string())?;
        Self::with_renderer(renderer, cfg)
    }

    /// Builds the app around an existing renderer (an entry point that drew
    /// its own screens first, e.g. Android's data import).
    pub fn with_renderer(renderer: Renderer, cfg: AppConfig) -> Result<App, String> {
        let data = cfg.game_dir.join("Data");
        let objects = ObjectTable::load(&data.join("values.lua")).map_err(|e| e.to_string())?;
        let atlas = Atlas::new(renderer.atlas_page_size());
        let mut gfx = Gfx::new(atlas, &data);
        let profile = match &cfg.data_dir {
            Some(d) => Profile::load(d),
            None => Profile::in_memory(),
        };
        let texts = Texts::load(&data, &profile.settings.language());
        gfx.upper = !texts.current.ttf;
        gfx.fonts.use_sc = texts.current.code == "zhcn";
        let mut audio = AudioDirector::new(&cfg.game_dir, SoundBank::load(&cfg.game_dir));
        gfx.art = crate::art::Art::new(audio.bank.assets());
        gfx.fonts.art = crate::art::Art::new(audio.bank.assets());
        gfx.fonts.pixel = !texts.current.ttf;
        audio.set_volumes(profile.settings.music(), profile.settings.sound());
        let cursor_sprite = gfx.sprites.get(&mut gfx.atlas, "baba", 0).map(|f| f[0]);
        let mut app = App {
            mode: if cfg.skip_boot {
                Mode::Title
            } else {
                Mode::Boot(0)
            },
            cfg,
            data,
            objects,
            renderer,
            gfx,
            texts,
            profile,
            audio,
            input: Input::default(),
            menus: Vec::new(),
            trans: None,
            title_bg: None,
            ticks: 0,
            acc: 0.0,
            last_time: None,
            quit: false,
            fullscreen: None,
            log: Vec::new(),
            pad: None,
            cursor_sprite,
            fast_trans: false,
            view_fit: None,
            ext: Default::default(),
            edh: Default::default(),
            online: Default::default(),
            pointer: (Vec::new(), (854, 480)),
            screen: (854, 480),
            no_player: None,
            cosmetic: baba_sim::CosmeticRng::new(0xB10B),
            cut: None,
            cut_tex: None,
            intro_next: false,
            hold: 0,
            osk_shift: false,
            slide_top: None,
            slide: (0.0, 0.0),
            auto_t: 0,
        };
        app.fullscreen = Some(app.profile.settings.toggle(Toggle::Fullscreen));
        if app.cfg.skip_boot {
            app.after_boot();
        }
        Ok(app)
    }

    pub fn renderer(&mut self) -> &mut Renderer {
        &mut self.renderer
    }

    /// Requested fullscreen state (applied by [`App::frame`]).
    pub fn take_fullscreen_request(&mut self) -> Option<bool> {
        self.fullscreen.take()
    }

    // ------------------------------------------------------------ driving

    /// One display frame on a platform: events, fixed-rate ticks, render.
    /// Returns false when the game wants to quit. The caller swaps.
    pub fn frame(&mut self, p: &mut dyn Platform) -> bool {
        let mut events = Vec::new();
        p.poll_events(&mut events);
        for e in &events {
            if matches!(e, Event::Quit) {
                self.quit = true;
            }
            if let Event::ControllerAdded(d) = e {
                self.pad = Some(d.name.clone());
            }
            // Android: the GL context was lost while in the background.
            if let Event::Resume { gl_lost: true } = e
                && let Some(gl) = p.take_restored_gl()
            {
                match Renderer::new(gl) {
                    Ok(r) => {
                        self.renderer = r;
                        self.gfx.atlas.mark_all_dirty();
                        log::info!("GL context restored: new renderer, atlas re-uploaded");
                    }
                    Err(e) => log::error!("renderer after context loss: {e}"),
                }
            }
            self.handle_event(e);
        }
        let now = p.time().as_secs_f64();
        let dt = match self.last_time {
            Some(l) => (now - l).clamp(0.0, 0.25),
            None => 1.0 / TICK_HZ,
        };
        self.last_time = Some(now);
        self.acc += dt * TICK_HZ;
        let mut n = 0;
        while self.acc >= 1.0 && n < 8 {
            self.acc -= 1.0;
            self.tick();
            n += 1;
        }
        if let Some(fs) = self.fullscreen.take() {
            p.set_fullscreen(fs);
        }
        self.audio.flush(p.audio());
        let size = p.drawable_size();
        self.render(size);
        !self.quit
    }

    pub fn handle_event(&mut self, e: &Event) {
        let (w, h) = self.pointer.1;
        match *e {
            Event::Touch {
                phase: TouchPhase::Down,
                x,
                y,
                ..
            } => self.pointer.0.push((x * w as f32, y * h as f32)),
            Event::MouseButton {
                button: 1,
                down: true,
                x,
                y,
            } => self.pointer.0.push((x, y)),
            _ => {}
        }
        self.ed_event(e);
        match &mut self.mode {
            // Multiplayer: every device belongs to a seat (`join`).
            Mode::Join(l) => l.event(e),
            Mode::Multi(m) => m.players.event(e),
            Mode::Online => {
                if let Some(l) = &mut self.online.lobby {
                    l.event(e)
                }
            }
            _ => {
                self.input.text_entry = self.prompt_text().is_some();
                self.input.event(e, &self.profile.settings)
            }
        }
    }

    /// Whether on-screen touch controls make sense now (playing, no menu);
    /// elsewhere touches go to the menus ([`App::handle_event`]).
    pub fn wants_touch_controls(&self) -> bool {
        matches!(self.mode, Mode::Play(_)) && self.menus.is_empty()
    }

    /// A tap or click at screen pixel `(x, y)`: activates the menu item
    /// under it (sliders: left half lowers, right half raises); outside
    /// menus it stands for the key the screen waits for (returned).
    fn tap(&mut self, x: f32, y: f32) -> Option<Cmd> {
        if self.input.capture.is_some() {
            return None;
        }
        let Some((id, _)) = self.menus.last().cloned() else {
            return match &self.mode {
                Mode::Boot(t) if *t > 10 => {
                    self.after_boot();
                    None
                }
                Mode::Title => Some(Cmd::Confirm),
                Mode::Credits(_) => Some(Cmd::Back),
                _ => None,
            };
        };
        if matches!(id, MenuId::Waiting) {
            return None;
        }
        let menu = self.build_menu(&id);
        let base = self.screen_ui(self.pointer.1.0, self.pointer.1.1);
        let ui = menu.ui_for(&base, &mut self.gfx, &self.texts);
        let (rc, at) = menu.hit(&ui, &mut self.gfx, &self.texts, x, y)?;
        let item = menu.selected(rc)?.clone();
        if let Some(top) = self.menus.last_mut() {
            top.1 = rc;
        }
        if !item.enabled {
            return None;
        }
        let step = match item.act {
            Act::Slider(_) if at < 0.5 => -1,
            Act::Slider(_) => 1,
            _ => 0,
        };
        self.activate(item.act, step);
        None
    }

    /// One 1/60 s step of game logic.
    pub fn tick(&mut self) {
        self.ticks += 1;
        crate::scene::HIGH_CONTRAST.store(
            self.profile.settings.toggle(Toggle::Contrast),
            std::sync::atomic::Ordering::Relaxed,
        );
        self.ext_poll();
        self.online_poll();
        self.input.tick(&self.profile.settings);
        let cmds = self.input.take();
        // Transitions block input; at full cover they run their action.
        if let Some(tr) = &mut self.trans {
            if tr.iris.tick()
                && let Some(then) = tr.then.take()
            {
                self.run_then(then);
            }
            // The covered screen waits for the title card (§11.1, measured).
            let title = matches!(&self.mode, Mode::Play(p) if p.title.is_some());
            if let Some(tr) = &mut self.trans {
                if tr.iris.covered() && tr.then.is_none() && !title {
                    tr.iris.release();
                }
                if tr.iris.done() {
                    self.trans = None;
                }
            }
            self.tick_mode(&[]);
            self.pointer.0.clear();
            return;
        }
        let mut cmds = cmds;
        for (x, y) in std::mem::take(&mut self.pointer.0) {
            cmds.extend(self.tap(x, y));
        }
        self.tick_mode(&cmds);
        self.tick_slide();
    }

    /// A menu that slides starts a screen away and closes 35 % of the
    /// remaining distance per frame (measured on the slot menu).
    fn tick_slide(&mut self) {
        let top = self.menus.last().map(|m| m.0.clone());
        let same_kind = match (&top, &self.slide_top) {
            (Some(a), Some(b)) => std::mem::discriminant(a) == std::mem::discriminant(b),
            _ => false,
        };
        let no_slide = matches!(self.mode, Mode::Editor)
            && self
                .profile
                .settings
                .store
                .get("editor", "noslide")
                .is_some_and(|v| v.trim() == "1");
        if top != self.slide_top && !same_kind {
            self.slide = match top.as_ref().and_then(slide_dir) {
                Some((x, y)) if !no_slide => (x * 854.0, y * 480.0),
                _ => (0.0, 0.0),
            };
        } else {
            for v in [&mut self.slide.0, &mut self.slide.1] {
                *v *= 0.65;
                if v.abs() < 0.5 {
                    *v = 0.0;
                }
            }
        }
        self.slide_top = top;
    }

    fn tick_mode(&mut self, cmds: &[Cmd]) {
        // Rebinding captures.
        if let Some(c) = self.input.captured.take() {
            let cap = self.input.capture.take();
            match (cap, c) {
                (Some(Capture::Key(a, sec)), Captured::Key(code)) => {
                    self.profile.settings.set_key(a, sec, code)
                }
                (Some(Capture::Pad(a, sec)), Captured::Pad(v)) => {
                    self.profile.settings.set_pad(a, sec, &v)
                }
                _ => {}
            }
            self.profile.save_settings();
            if matches!(self.menus.last(), Some((MenuId::Waiting, _))) {
                self.menus.pop();
            }
            return;
        }
        if self.input.capture.is_some() {
            return;
        }
        if self.ed_text(cmds) {
            return;
        }
        if matches!(self.menus.last(), Some((MenuId::Waiting, _))) && self.input.capture.is_none() {
            self.menus.pop();
        }
        // Text prompts (slot names, level and room codes).
        if self.prompt_tick(cmds) {
            return;
        }
        self.input.typed.clear();
        if !self.menus.is_empty() {
            for c in cmds {
                self.menu_cmd(*c);
                if self.menus.is_empty() || self.trans.is_some() {
                    break;
                }
            }
            return;
        }
        match &mut self.mode {
            Mode::Boot(t) => {
                *t += 1;
                let skip = self.input.any_down > 0 && *t > 10;
                if *t >= BOOT_FRAMES || skip {
                    self.after_boot();
                }
            }
            Mode::Title => {
                for c in cmds {
                    if matches!(c, Cmd::Confirm | Cmd::Back | Cmd::Pause | Cmd::Dir(_)) {
                        self.menus.push((MenuId::Main, (0, 0)));
                        break;
                    }
                }
            }
            Mode::Credits(c) => {
                c.fast = self.input.any_down > 0;
                if c.show_running() && cmds.contains(&Cmd::Back) {
                    c.skip_show();
                    self.audio.music("ending");
                    return;
                }
                let (w, h) = self.screen;
                for (s, pitch) in c.tick(w as f32, h as f32) {
                    match s.strip_prefix("music:") {
                        Some(m) => self.audio.music(m),
                        None => self.audio.sound_pitch(&s, pitch),
                    }
                }
                let done = c.finished || (cmds.contains(&Cmd::Back) && !c.show_running());
                let kind = c.kind;
                if done {
                    if kind == CreditsKind::Menu {
                        self.mode = Mode::Title;
                        self.audio.music("menu");
                        self.menus.push((MenuId::Main, (0, 0)));
                    } else {
                        self.start_trans(Then::ToTitle);
                    }
                }
            }
            Mode::Play(_) => self.tick_play(cmds),
            Mode::Join(_) | Mode::Multi(_) => self.tick_multi(),
            Mode::Editor => self.tick_editor(cmds),
            Mode::Online => self.tick_online(),
        }
    }

    // -------------------------------------------------------- multiplayer

    fn open_lobby(&mut self) {
        let packs = list_packs(&self.data)
            .into_iter()
            .map(|w| {
                let name = WorldInfo::open(&self.data, &w)
                    .map(|d| self.texts.pack_name(&w, &d.data.name))
                    .unwrap_or_else(|| w.clone());
                (w, name)
            })
            .collect();
        let extra = ExtraSlots::load(self.cfg.data_dir.as_deref());
        let l = Lobby::new(
            &self.profile.settings,
            packs,
            self.profile.last_slot(),
            extra,
        );
        self.menus.clear();
        self.input = Input::default();
        self.mode = Mode::Join(Box::new(l));
        self.log.push("multiplayer".into());
    }

    fn back_to_title(&mut self) {
        self.audio.set_mixing(false);
        self.input = Input::default();
        self.mode = Mode::Title;
        self.menus = vec![(MenuId::Main, (0, 0))];
        self.audio.music("menu");
    }

    fn tick_multi(&mut self) {
        match &mut self.mode {
            Mode::Multi(m) if m.net.is_some() => {
                self.tick_online_session();
                return;
            }
            Mode::Join(l) if l.to_online => {
                let Mode::Join(l) = std::mem::replace(&mut self.mode, Mode::Title) else {
                    return;
                };
                self.open_online(Some(l));
                return;
            }
            _ => {}
        }
        let mut cx = Cx {
            data: &self.data,
            objects: &self.objects,
            gfx: &mut self.gfx,
            texts: &self.texts,
            profile: &mut self.profile,
            audio: &mut self.audio,
            cursor: self.cursor_sprite,
        };
        let mut title = false;
        match &mut self.mode {
            Mode::Join(l) => {
                l.tick();
                self.log.append(&mut l.log);
                if l.exit {
                    title = true;
                } else if l.start {
                    let Mode::Join(l) = std::mem::replace(&mut self.mode, Mode::Title) else {
                        return;
                    };
                    match Multi::start(*l, &mut cx) {
                        Ok(mut m) => {
                            self.log.append(&mut m.log);
                            self.mode = Mode::Multi(Box::new(m));
                        }
                        Err(e) => {
                            log::error!("{e}");
                            self.log.push(format!("error: {e}"));
                            title = true;
                        }
                    }
                }
            }
            Mode::Multi(m) => {
                m.tick(&mut cx);
                self.log.append(&mut m.log);
                if m.end {
                    m.save_all(&mut cx);
                    title = true;
                }
            }
            _ => {}
        }
        if title {
            self.back_to_title();
        }
    }

    fn after_boot(&mut self) {
        if self.profile.first_time() {
            // An original save is offered for import first (M9).
            if !self.ext_offer_import() {
                self.first_launch();
            }
        } else {
            self.mode = Mode::Title;
            self.audio.music("menu");
            self.menus = vec![(MenuId::Main, (0, 0))];
        }
    }

    fn first_launch(&mut self) {
        {
            // First launch: no menu; slot 0 starts the base world
            // (ui.md §1.2). The intro cutscene is not reproduced; its
            // flags are recorded.
            self.profile.set_first_time_done();
            self.profile.set_last_slot(0);
            let s = &mut self.profile.slots[0];
            s.set_flag("baba", "intro", true);
            self.profile.save_global();
            self.log.push("first launch".into());
            self.start_world_now(0, "baba");
            self.begin_intro();
        }
    }

    fn start_trans(&mut self, then: Then) {
        let (w, h) = self.screen;
        let iris = crate::fx::transition::Iris::new(
            w as f32,
            h as f32,
            self.fast_trans,
            &mut self.cosmetic,
        );
        self.fast_trans = false;
        self.trans = Some(Trans {
            iris,
            then: Some(then),
        });
    }

    fn save_slot_now(&mut self) {
        if let Mode::Play(p) = &mut self.mode {
            let f = std::mem::take(&mut p.frames);
            let wid = p.world.id.clone();
            self.profile.slots[p.slot].add_frames(&wid, f);
            self.profile.save_slot(p.slot);
        }
    }

    fn start_world_now(&mut self, slot: usize, world: &str) {
        let Some(w) = self.open_world(world) else {
            self.log.push(format!("error: no world {world}"));
            return;
        };
        self.profile.settings.set_last_session(world, slot);
        self.profile.set_last_slot(slot);
        self.profile.save_settings();
        self.profile.save_global();
        let mut env = Env {
            data_dir: &self.data,
            objects: &self.objects,
            atlas: &mut self.gfx.atlas,
        };
        match Play::start(&mut env, w, slot, &mut self.profile.slots[slot]) {
            Ok(mut p) => {
                self.log.append(&mut p.log);
                self.menus.clear();
                self.mode = Mode::Play(Box::new(p));
                self.level_music();
                self.save_slot_now();
                if std::mem::take(&mut self.intro_next) {
                    self.begin_intro();
                }
            }
            Err(e) => {
                log::error!("{e}");
                self.log.push(format!("error: {e}"));
            }
        }
    }

    fn level_music(&mut self) {
        if let Mode::Play(p) = &self.mode {
            let m = &p.cur.meta;
            self.audio
                .level_music(&p.world.res_key(), &m.music, m.local_music);
        }
    }

    fn sync_play_log(&mut self) {
        if let Mode::Play(p) = &mut self.mode {
            self.log.append(&mut p.log);
        }
    }

    fn run_then(&mut self, then: Then) {
        match then {
            Then::StartWorld(slot, w) => self.start_world_now(slot, &w),
            Then::Continue => {
                if let Some((w, slot)) = self.profile.settings.last_session() {
                    self.start_world_now(slot, &w);
                }
            }
            Then::ToTitle => {
                self.save_slot_now();
                self.mode = Mode::Title;
                self.menus = vec![(MenuId::Main, (0, 0))];
                self.audio.music("menu");
                self.ext_after_title();
            }
            Then::Credits(kind) => {
                let text = self.credits_text(kind);
                let mut credits = Credits::new(kind, &text);
                // After the base world's END: the island and the logo
                // credits first (the music starts with the logos).
                let show = matches!(self.cut, Some(Cutscene::End(_))) && kind == CreditsKind::End;
                if show && let Some(s) = self.end_show() {
                    credits = credits.with_show(s);
                }
                self.cut = None;
                self.mode = Mode::Credits(credits);
                self.menus.clear();
                if kind == CreditsKind::Menu {
                    self.audio.music("menu");
                } else if show {
                    self.audio.stop_music();
                } else {
                    self.audio.music("ending");
                }
            }
            Then::Turn(t) => {
                self.menus.clear();
                self.apply_turn(t);
            }
        }
    }

    fn credits_text(&self, kind: CreditsKind) -> String {
        let base = WorldInfo::open(&self.data, "baba");
        let file = match kind {
            CreditsKind::Menu => "credits.txt",
            CreditsKind::End => "credits_short.txt",
            CreditsKind::Done => "credits_short2.txt",
            CreditsKind::Pack => "credits.txt",
        };
        if kind == CreditsKind::Pack
            && let Mode::Play(p) = &self.mode
            && let Some(t) = p.world.credits(file)
        {
            return t;
        }
        base.and_then(|w| w.credits(file)).unwrap_or_default()
    }

    // --------------------------------------------------------------- play

    /// Starts the intro on the base world's first level (just loaded).
    fn begin_intro(&mut self) {
        let Mode::Play(p) = &mut self.mode else {
            return;
        };
        if p.cur.is_map() || !p.world.is_base() || p.cur.stem != p.world.data.first_level {
            return;
        }
        p.title = None;
        let seed = self.ticks ^ 0x1A7_0B0B;
        self.cut = Some(Cutscene::Intro(Box::new(crate::cutscene::Intro::new(
            &p.cur.level,
            seed,
        ))));
        self.log.push("intro".into());
    }

    /// The island show after END: area flowers from the slot, the spreading
    /// flowers' points and the island images from the install.
    fn end_show(&mut self) -> Option<crate::cutscene::EndShow> {
        use crate::cutscene::EndShow;
        let Mode::Play(p) = &self.mode else {
            return None;
        };
        let slot = &self.profile.slots[p.slot];
        let areas: Vec<bool> = EndShow::area_levels()
            .iter()
            .map(|l| slot.complete("baba", l))
            .collect();
        let spread = std::fs::read_to_string(self.data.join("endingflowers.txt"))
            .map(|t| EndShow::spread_points(&t))
            .unwrap_or_default();
        let mut show = EndShow::new(&areas, spread, self.ticks);
        let dir = self.data.join("Worlds/baba/Images");
        for n in 1..=3 {
            if let Some(r) = self.gfx.image(dir.join(format!("island_{n}.png"))) {
                show.island.push(r);
            }
            if let Some(r) = self.gfx.image(dir.join(format!("island_decor_{n}.png"))) {
                show.decor.push(r);
            }
        }
        Some(show)
    }

    /// One frame of the running cutscene. Returns true when it holds the
    /// level (input blocked; the effects were ticked here).
    fn tick_cut(&mut self, cmds: &[Cmd]) -> bool {
        let Mode::Play(p) = &mut self.mode else {
            self.cut = None;
            return false;
        };
        let Some(cut) = &mut self.cut else {
            return false;
        };
        let mut cues = Vec::new();
        let mut block = true;
        let mut proceed = None;
        let mut finished = false;
        match cut {
            Cutscene::Intro(i) => {
                if !i.released {
                    // Hold any key to skip (`ui.md` §1).
                    self.hold = if self.input.any_down > 0 {
                        self.hold + 1
                    } else {
                        0
                    };
                    if self.hold >= 60 {
                        i.skip();
                        self.hold = 0;
                        self.log.push("intro skipped".into());
                    }
                }
                cues = i.tick(&mut p.cur.fx);
                block = !i.released;
                finished = i.done;
            }
            Cutscene::End(e) => {
                for c in cmds {
                    if let Cmd::Dir(d) = c {
                        cues.extend(e.input(*d));
                    }
                }
                cues.extend(e.tick(&mut p.cur.fx));
                // Holding a key other than the arrows skips to the island.
                self.hold = if self.input.any_down > 0 && !self.input.dir_held() {
                    self.hold + 1
                } else {
                    0
                };
                if std::mem::take(&mut e.proceed) || self.hold == 90 {
                    proceed = Some(true);
                }
            }
            Cutscene::Done(d) => {
                cues = d.tick();
                self.hold = if self.input.any_down > 0 {
                    self.hold + 1
                } else {
                    0
                };
                if std::mem::take(&mut d.proceed) || self.hold == 90 {
                    d.proceed = false;
                    proceed = Some(false);
                }
            }
        }
        if block {
            p.frames += 1;
            p.cur.fx.tick(&p.cur.level);
        }
        for (s, pitch) in cues {
            self.audio.sound_pitch(&s, pitch);
        }
        if finished {
            self.cut = None;
            self.log.push("intro done".into());
        }
        match proceed {
            // END closes the screen, the credits screen continues.
            Some(true) if self.trans.is_none() => self.start_trans(Then::Turn(Turn::Proceed)),
            // All Is Done goes straight to the credits.
            Some(false) => self.run_then(Then::Turn(Turn::Proceed)),
            _ => {}
        }
        block
    }

    fn tick_play(&mut self, cmds: &[Cmd]) {
        if self.cut.is_some() && self.tick_cut(cmds) {
            return;
        }
        let Mode::Play(p) = &mut self.mode else {
            return;
        };
        p.frames += 1;
        let s = &self.profile.settings;
        p.cur.fx.user = crate::fx::FxOptions {
            particles: !s.toggle(Toggle::NoParticles),
            shake: !s.toggle(Toggle::NoShake),
            rule_effect: true,
        };
        p.cur.fx.tick(&p.cur.level);
        if let Some(n) = &mut self.no_player {
            *n += 1;
        }
        if let Some((_, t)) = &mut p.banner {
            *t = t.saturating_sub(1);
            if *t == 0 {
                p.banner = None;
            }
        }
        if let Some(t) = &mut p.title {
            *t = t.saturating_sub(1);
            if *t == 0 {
                p.title = None;
            }
            return;
        }
        if p.anim.is_some() {
            for (s, pitch) in p.tick_anim() {
                self.audio.sound_pitch(&s, pitch);
            }
            return;
        }
        // Map-clear showcase: each step is a `Proceed` at full cover once
        // the previous step's animation has run.
        if matches!(p.pending, Some(Pending::Showcase(_))) && p.won.is_none() {
            if self.trans.is_none() {
                self.start_trans(Then::Turn(Turn::Proceed));
            }
            return;
        }
        if let Some(t) = &mut p.won {
            *t = t.saturating_sub(1);
            if *t == 0 {
                p.won = None;
                if p.pending.is_some() {
                    self.start_trans(Then::Turn(Turn::Proceed));
                }
            }
            return;
        }
        // LEVEL IS AUTO: an auto turn every `rhythm` + 1 frames (measured:
        // 11 frames at the default 10) while it holds.
        let auto_period = (p.cur.level.auto_active() && p.pending.is_none()).then(|| {
            p.cur
                .info
                .ini
                .get_int("general", "rhythm")
                .unwrap_or(10)
                .max(0) as u32
                + 1
        });
        match auto_period {
            Some(n) if self.trans.is_none() && self.menus.is_empty() => {
                self.auto_t += 1;
                if self.auto_t >= n {
                    self.auto_t = 0;
                    self.apply_turn(Turn::Auto);
                }
            }
            Some(_) => {}
            None => self.auto_t = 0,
        }
        for &c in cmds {
            if self.trans.is_some() || !self.menus.is_empty() {
                break;
            }
            let Mode::Play(p) = &mut self.mode else {
                return;
            };
            let is_map = p.cur.is_map();
            let input = match c {
                Cmd::Dir(d) => Some(Turn::Move { dir: d, you: 1 }),
                Cmd::Idle => {
                    if is_map && !p.cur.candidates(&p.world).is_empty() {
                        None // confirm enters instead
                    } else {
                        Some(Turn::wait())
                    }
                }
                Cmd::Undo => Some(Turn::Undo),
                Cmd::Confirm => {
                    if is_map {
                        let cands = p.cur.candidates(&p.world);
                        match cands.len() {
                            0 => {}
                            1 => {
                                let (target, unit) = cands[0].clone();
                                self.audio.sound("confirm");
                                self.start_trans(Then::Turn(Turn::Enter { target, unit }));
                            }
                            _ => self.menus.push((MenuId::SelectLevel(cands), (1, 0))),
                        }
                    }
                    None
                }
                Cmd::Restart => {
                    if self.profile.settings.toggle(Toggle::RestartNoAsk) {
                        self.restart_now();
                    } else {
                        self.menus.push((MenuId::RestartConfirm, (0, 0)));
                    }
                    None
                }
                Cmd::Pause | Cmd::Back => {
                    self.menus.push((MenuId::Pause, (0, 0)));
                    None
                }
            };
            if let Some(t) = input {
                self.apply_turn(t);
            }
        }
    }

    fn restart_now(&mut self) {
        self.audio.sound("restart");
        self.fast_trans = true;
        self.start_trans(Then::Turn(Turn::Restart));
    }

    /// Plays one input in the current level (also used by scripts).
    /// Debug/test helper: starts `world` in `slot` and goes straight into
    /// `stem` (below the world's root map).
    pub fn goto(&mut self, slot: usize, world: &str, stem: &str) -> Result<(), String> {
        self.trans = None;
        self.start_world_now(slot, world);
        let Mode::Play(p) = &mut self.mode else {
            return Err(format!("could not start {world}"));
        };
        let mut env = Env {
            data_dir: &self.data,
            objects: &self.objects,
            atlas: &mut self.gfx.atlas,
        };
        p.goto(&mut env, &mut self.profile.slots[slot], stem)?;
        self.sync_play_log();
        self.level_music();
        Ok(())
    }

    /// Applies a turn to the instance being played (the single path for
    /// everything that changes it, `session::apply`) and presents the
    /// result.
    pub fn apply_turn(&mut self, t: Turn) {
        let Mode::Play(p) = &mut self.mode else {
            return;
        };
        let slot = p.slot;
        let mut env = Env {
            data_dir: &self.data,
            objects: &self.objects,
            atlas: &mut self.gfx.atlas,
        };
        let a = p.apply(&t, &mut env, &mut self.profile.slots[slot], &self.texts);
        self.sync_play_log();
        let mut loaded = false;
        let mut credits = None;
        let mut leave_single = None;
        for fx in a.fx {
            let Mode::Play(p) = &mut self.mode else {
                return;
            };
            match fx {
                Fx::UndoSound => self.audio.sound_variant("undo", 5),
                Fx::Turn(out) => {
                    self.audio.turn(&out);
                    // A map cursor counts as a player (maps keep their music).
                    let cursor = p.cur.is_map() && !p.cur.cursors().is_empty();
                    self.audio.player_state(out.no_player && !cursor);
                    // The undo hint appears 160 frames after a turn without
                    // a player (measured) and goes when a player exists again.
                    self.no_player = if out.no_player && !cursor {
                        self.no_player.or(Some(0))
                    } else {
                        None
                    };
                }
                Fx::Banner(b, n) => p.banner = Some((b, n)),
                Fx::Won => {
                    p.banner = Some((self.texts.get("ingame_victory"), WIN_FRAMES + 30));
                    p.won = Some(WIN_FRAMES);
                    self.audio.sound("winner2");
                }
                Fx::ProceedAfterTransition => match &p.pending {
                    // The base world's endings play their sequences first
                    // (`progression.md` §10.1–§10.2).
                    Some(Pending::Ending(CreditsKind::End)) => {
                        let seed = self.ticks ^ 0xE0D;
                        self.cut = Some(Cutscene::End(Box::new(crate::cutscene::EndScene::new(
                            &p.cur.level,
                            seed,
                        ))));
                        self.log.push("ending sequence end".into());
                    }
                    Some(Pending::Ending(CreditsKind::Done)) => {
                        let seed = self.ticks ^ 0xD0E;
                        self.cut = Some(Cutscene::Done(Box::new(crate::cutscene::DoneScene::new(
                            seed,
                        ))));
                        self.log.push("ending sequence done".into());
                    }
                    // A pack's END / ALL IS DONE: the level is won (victory
                    // banner), then the credits (§10.3).
                    Some(Pending::Ending(CreditsKind::Pack)) => {
                        p.banner = Some((self.texts.get("ingame_victory"), WIN_FRAMES + 30));
                        p.won = Some(WIN_FRAMES);
                        self.audio.sound("winner2");
                    }
                    _ => self.start_trans(Then::Turn(Turn::Proceed)),
                },
                Fx::Credits(kind) => credits = Some(kind),
                Fx::Loaded => loaded = true,
                Fx::EditorSpecial => {
                    self.profile.settings.store.set("editor", "special", 1);
                    self.profile.save_settings();
                }
                Fx::LeaveSingle { won } => leave_single = Some(won),
            }
        }
        if a.save {
            self.save_slot_now();
        }
        if let Some(won) = leave_single {
            if self.editor_test_done(won) {
                return;
            }
            self.ext_single_done(won);
            return;
        }
        if loaded {
            self.level_music();
        }
        if let Some(kind) = credits {
            self.run_then(Then::Credits(kind));
        }
    }

    // -------------------------------------------------------------- menus

    fn build_menu(&mut self, id: &MenuId) -> Menu {
        let t = &self.texts;
        let s = &self.profile.settings;
        match id {
            MenuId::Main => {
                let mut m = Menu::new(Style::Title);
                m.escape = Act::None;
                let cont = s.last_session().is_some_and(|(w, _)| {
                    w != "levels"
                        && crate::packs::find_dir(&self.data, self.cfg.data_dir.as_deref(), &w)
                            .is_some_and(|(d, _)| d.join("world_data.txt").is_file())
                });
                if cont {
                    m.item(Item::new(t.get("main_continue"), Act::Continue));
                }
                m.row(vec![
                    Item::new(t.get("main_start"), Act::StartGame).colour((2, 4)),
                    Item::new(
                        t.get("main_custom"),
                        Act::Open(MenuId::Ext(ExtMenu::PlayLevels)),
                    )
                    .colour((3, 4)),
                ]);
                m.row(vec![
                    Item::new(t.get("main_editor"), Act::Editor(editor_ui::EdAct::Open)),
                    Item::new(t.get("settings"), Act::Open(MenuId::Settings)),
                ]);
                m.row(vec![
                    Item::new(t.get("credits"), Act::Credits),
                    Item::new(t.get("main_exit"), Act::Exit),
                ]);
                m.item(Item::new("Multiplayer", Act::Multiplayer).colour((4, 1)));
                m.item(Item::new("Online", Act::Online(online::OnAct::Open)).colour((5, 3)));
                m
            }
            MenuId::Slots | MenuId::Erase => {
                let mut m = Menu::new(Style::List);
                m.top = 60.0;
                m.min_width = 360.0;
                let erase = *id == MenuId::Erase;
                m.header.push(t.get(if erase {
                    "erase_select"
                } else {
                    "slots_select"
                }));
                for i in 0..3 {
                    let label = self.slot_label(i);
                    let act = if erase {
                        Act::Open(MenuId::EraseConfirm(i))
                    } else {
                        Act::PickSlot(i)
                    };
                    m.item(Item::new(label, act));
                }
                m.gap(12.0);
                m.item(Item::new(t.get("return"), Act::Close));
                if !erase {
                    m.item(Item::new(t.get("slots_erase"), Act::Open(MenuId::Erase)));
                }
                m
            }
            MenuId::EraseConfirm(i) => {
                let mut m = Menu::new(Style::List);
                m.top = 150.0;
                m.header.push(t.get("erase_confirm"));
                m.header.push(t.get("erase_tip"));
                m.row(vec![
                    Item::new(t.get("no"), Act::Close),
                    Item::new(t.get("yes"), Act::EraseSlot(*i)),
                ]);
                m
            }
            MenuId::Packs { page } => {
                let slot = self.profile.last_slot();
                let packs = crate::packs::list(&self.data, self.cfg.data_dir.as_deref());
                let pages = packs.len().div_ceil(3).max(1);
                let page = (*page).min(pages - 1);
                let mut m = Menu::new(Style::List);
                m.top = 30.0;
                m.min_width = 460.0;
                m.header.push(format!(
                    "{} {}",
                    t.get("slot_current"),
                    self.slot_name(slot)
                ));
                m.header.push(t.get("customlevels_pack"));
                for p in packs.iter().skip(page * 3).take(3) {
                    let w = &p.id;
                    let mut name = if p.name.is_empty() {
                        t.get("world_unknown")
                    } else {
                        t.pack_name(w, &p.name)
                    };
                    if p.lua {
                        name.push_str(" (Lua)");
                    }
                    let stats = self
                        .stats_text(slot, w)
                        .unwrap_or_else(|| t.get("customlevels_pack_emptysave"));
                    m.item(
                        Item::new(name, Act::PickPack(w.clone()))
                            .value(stats)
                            .enabled(p.playable),
                    );
                }
                if pages > 1 {
                    m.gap(6.0);
                    m.row(vec![
                        Item::new("<", Act::PackPage(page.saturating_sub(1))).enabled(page > 0),
                        Item::new(format!("{}/{}", page + 1, pages), Act::None),
                        Item::new(">", Act::PackPage(page + 1)).enabled(page + 1 < pages),
                    ]);
                }
                m.gap(12.0);
                m.item(Item::new(t.get("slot_change"), Act::SwitchSlot));
                m.item(Item::new(
                    t.opt("slot_rename")
                        .unwrap_or_else(|| "Change slot name".into()),
                    Act::Open(MenuId::Rename {
                        slot,
                        text: String::new(),
                    }),
                ));
                m.item(Item::new(t.get("return"), Act::Close));
                m
            }
            MenuId::Settings => {
                let mut m = Menu::new(Style::List);
                m.top = 8.0;
                m.min_width = 330.0;
                m.header.push(t.get("settings_colon"));
                m.item(
                    Item::new(t.get("settings_music"), Act::Slider(Slider::Music))
                        .value(s.music().to_string()),
                );
                m.item(
                    Item::new(t.get("settings_sound"), Act::Slider(Slider::Sound))
                        .value(s.sound().to_string()),
                );
                m.item(
                    Item::new(t.get("settings_repeat"), Act::Slider(Slider::Delay))
                        .value(s.delay().to_string()),
                );
                m.item(Item::new(
                    t.get("settings_language"),
                    Act::Open(MenuId::Language),
                ));
                m.item(Item::new(
                    t.get("controls_setup").trim_end_matches(':').to_string(),
                    Act::Open(MenuId::Controls),
                ));
                for (tg, key) in [
                    (Toggle::Fullscreen, "settings_fullscreen"),
                    (Toggle::Grid, "settings_grid"),
                    (Toggle::NoWobble, "settings_wobble"),
                    (Toggle::NoParticles, "settings_particles"),
                    (Toggle::NoShake, "settings_shake"),
                    (Toggle::Contrast, "settings_palette"),
                    (Toggle::NoBlinking, "settings_blinking"),
                    (Toggle::RestartNoAsk, "settings_restart"),
                ] {
                    m.item(Item::new(t.get(key), Act::Toggle(tg)).on(s.toggle(tg)));
                }
                m.item(
                    Item::new(
                        "Import Baba Is You progress",
                        Act::Open(MenuId::Ext(ExtMenu::Import)),
                    )
                    .enabled(!matches!(self.mode, Mode::Play(_))),
                );
                let z = s.zoom();
                m.row(vec![
                    Item::new(t.get("zoom1"), Act::Zoom(Zoom::None)).on(z == Zoom::None),
                    Item::new(t.get("zoom2"), Act::Zoom(Zoom::Default)).on(z == Zoom::Default),
                    Item::new(t.get("zoom3"), Act::Zoom(Zoom::Stretch)).on(z == Zoom::Stretch),
                ]);
                m.item(Item::new(t.get("return"), Act::Close));
                m
            }
            MenuId::Controls => {
                let mut m = Menu::new(Style::List);
                m.top = 40.0;
                m.min_width = 330.0;
                m.header.push(t.get("controls_setup"));
                m.header.push(match &self.pad {
                    Some(n) => format!(
                        "{} {}",
                        t.get("controls_gamepadname"),
                        n.chars().take(25).collect::<String>().to_lowercase()
                    ),
                    None => t.get("controls_noconnectedgamepad"),
                });
                m.item(Item::new(t.get("controls_detectgamepad"), Act::DetectPad));
                m.item(Item::new(
                    t.get("controls_gamepadsetup"),
                    Act::Open(MenuId::PadSetup),
                ));
                m.item(Item::new(t.get("controls_defaultgamepad"), Act::DefaultPad));
                let off = s.pad_disabled();
                m.item(Item::new(
                    t.get(if off {
                        "controls_disablegamepad_on"
                    } else {
                        "controls_disablegamepad_off"
                    }),
                    Act::TogglePad,
                ));
                m.item(Item::new(
                    t.get("controls_keysetup"),
                    Act::Open(MenuId::KeySetup),
                ));
                m.item(Item::new(t.get("controls_defaultkey"), Act::DefaultKeys));
                m.gap(8.0);
                m.item(Item::new(t.get("return"), Act::Close));
                m
            }
            MenuId::KeySetup => {
                let mut m = Menu::new(Style::List);
                m.top = 20.0;
                m.min_width = 200.0;
                m.item(Item::new(t.get("return"), Act::Close));
                m.gap(6.0);
                for a in Action::ALL {
                    let label = t.get(match a {
                        Action::Idle => "idle",
                        other => other.key(),
                    });
                    let mut row = vec![
                        Item::new(label.clone(), Act::BindKey(a, false))
                            .value(key_name(s.key(a, false))),
                    ];
                    if a.has_secondary() {
                        row.push(
                            Item::new(label, Act::BindKey(a, true)).value(key_name(s.key(a, true))),
                        );
                    }
                    m.row(row);
                }
                m
            }
            MenuId::PadSetup => {
                let mut m = Menu::new(Style::List);
                m.top = 40.0;
                m.min_width = 200.0;
                m.item(Item::new(t.get("return"), Act::Close));
                m.gap(6.0);
                for a in PadAction::ALL {
                    let label = t.get(match a {
                        PadAction::Idle => "idle",
                        other => other.key(),
                    });
                    let mut row = vec![
                        Item::new(label.clone(), Act::BindPad(a, false))
                            .value(pad_name(&s.pad(a, false))),
                    ];
                    if a.has_secondary() {
                        row.push(
                            Item::new(label, Act::BindPad(a, true))
                                .value(pad_name(&s.pad(a, true))),
                        );
                    }
                    m.row(row);
                }
                m
            }
            MenuId::Language => {
                let mut m = Menu::new(Style::List);
                m.top = 20.0;
                m.min_width = 200.0;
                m.header.push(t.get("lang_setup"));
                let cur = t.current.code.clone();
                let langs: Vec<Item> = t
                    .available
                    .iter()
                    .map(|(code, name)| {
                        Item::new(name.clone(), Act::Language(code.clone())).on(*code == cur)
                    })
                    .collect();
                for pair in langs.chunks(2) {
                    m.row(pair.to_vec());
                }
                m.gap(8.0);
                m.item(Item::new(t.get("return"), Act::Close));
                m
            }
            MenuId::Pause => {
                let mut m = Menu::new(Style::List);
                m.top = 9.0;
                m.header_gap = 20.0;
                m.escape = Act::Resume;
                if let Mode::Play(p) = &self.mode {
                    let name = p.level_name(t, &p.cur.stem);
                    let mut head = match (p.cur.is_map(), p.label(t)) {
                        (false, Some(l)) => format!("{l}: {name}"),
                        _ => name,
                    };
                    let single = self.ext.single.as_ref();
                    if let Some(c) = single.and_then(|s| s.code.as_ref()) {
                        head.push_str(&format!(" ({c})"));
                    }
                    m.header.push(head);
                    m.item(Item::new(t.get("resume"), Act::Resume));
                    m.item(if let Some(it) = self.editor_pause_item() {
                        it
                    } else if single.is_some() {
                        Item::new(
                            t.get("pause_returnplaylevels"),
                            Act::Ext(ExtAct::LeaveSingle),
                        )
                    } else {
                        Item::new(t.get("pause_returnmap"), Act::ReturnToMap)
                            .enabled(p.can_return())
                    });
                    m.item(Item::new(t.get("controls_restart"), Act::Restart));
                    m.item(Item::new(t.get("settings"), Act::Open(MenuId::Settings)));
                    // An editor test has no "return to menu" (editor.md §12.3).
                    if self.editor_pause_item().is_none() {
                        m.gap(12.0);
                        m.item(Item::new(t.get("pause_returnmain"), Act::ReturnToMenu));
                    }
                    let mut lines: Vec<(String, usize)> = Vec::new();
                    for r in &p.cur.level.rules().visual {
                        let l = rule_line(r);
                        match lines.iter_mut().find(|(x, _)| *x == l) {
                            Some(e) => e.1 += 1,
                            None => lines.push((l, 1)),
                        }
                    }
                    if !lines.is_empty() {
                        m.footer
                            .push(t.get("rules_colon").replace("(rules_colon)", "Rules:"));
                        for (l, n) in lines.into_iter().take(12) {
                            m.footer.push(if n > 1 { format!("{n} x {l}") } else { l });
                        }
                    }
                }
                m
            }
            MenuId::RestartConfirm => {
                let mut m = Menu::new(Style::List);
                m.top = 150.0;
                m.header.push(t.get("restart_confirm"));
                m.header.push(t.get("restart_tip"));
                m.row(vec![
                    Item::new(t.get("no"), Act::Close),
                    Item::new(t.get("yes"), Act::RestartNow),
                ]);
                m
            }
            MenuId::SelectLevel(c) => {
                let mut m = Menu::new(Style::List);
                m.top = 60.0;
                m.header.push(t.get("enterlevel_multiple"));
                m.item(Item::new(t.get("return"), Act::Close));
                if let Mode::Play(p) = &self.mode {
                    for (stem, unit) in c {
                        m.item(Item::new(
                            p.level_name(t, stem),
                            Act::EnterLevel(stem.clone(), *unit),
                        ));
                    }
                }
                m
            }
            MenuId::IntroPrompt { world } => {
                let mut m = Menu::new(Style::List);
                m.top = 150.0;
                m.escape = Act::Close;
                m.header.push(t.get("intro_confirm"));
                m.row(vec![
                    Item::new(t.get("yes"), Act::Intro(true, world.clone())),
                    Item::new(t.get("no"), Act::Intro(false, world.clone())),
                    Item::new(t.get("cancel"), Act::Close),
                ]);
                m
            }
            MenuId::Rename { text, .. } => {
                let mut m = Menu::new(Style::List);
                m.top = 60.0;
                m.header.push(
                    t.opt("slot_rename")
                        .unwrap_or_else(|| "Change slot name".into()),
                );
                m.header.push(format!("{text}_"));
                osk::add_rows(
                    &mut m,
                    osk::Charset::Name,
                    self.osk_shift,
                    "OK",
                    &t.get("cancel"),
                );
                m
            }
            MenuId::Waiting => {
                let mut m = Menu::new(Style::List);
                m.top = 200.0;
                m.header.push(t.get("controls_pressany"));
                m
            }
            MenuId::QuitConfirm => Menu::new(Style::List),
            MenuId::Ext(e) => self.build_ext(e),
            MenuId::Editor(e) => self.build_ed(e),
            MenuId::Online(o) => self.build_online(o),
        }
    }

    fn slot_name(&self, i: usize) -> String {
        self.profile.slot_label(i, &self.texts.get("slot"))
    }

    /// `H:MM⏲  P🌱  C✿ (+B)` (`ui.md` §3.1), `None` without spores.
    fn stats_text(&self, slot: usize, world: &str) -> Option<String> {
        let st = self.profile.slots[slot].stats(world);
        if st.prize == 0 && st.clears == 0 {
            return None;
        }
        let mut s = format!(
            "{}:{:02}   {} \u{273D}   {} \u{273F}",
            st.seconds / 3600,
            (st.seconds / 60) % 60,
            st.prize,
            st.clears
        );
        if st.bonus > 0 {
            s.push_str(&format!("  (+{})", st.bonus));
        }
        if st.ended {
            s.push_str("  *");
        }
        Some(s)
    }

    fn slot_label(&self, i: usize) -> String {
        let name = self.slot_name(i);
        match self.stats_text(i, "baba") {
            Some(st) => format!("{name}   {st}"),
            None => name,
        }
    }

    fn menu_cmd(&mut self, c: Cmd) {
        let Some((id, mut sel)) = self.menus.last().cloned() else {
            return;
        };
        let menu = self.build_menu(&id);
        let res = menu.navigate(&mut sel, c);
        if let Some(top) = self.menus.last_mut() {
            top.1 = sel;
        }
        if matches!(c, Cmd::Dir(_)) && res.is_none() {
            self.audio.sound_variant("click_", 5);
        }
        if let Some((act, step)) = res {
            self.activate(act, step);
        }
    }

    fn apply_settings(&mut self) {
        let s = &self.profile.settings;
        self.audio.set_volumes(s.music(), s.sound());
        self.profile.save_settings();
    }

    fn activate(&mut self, act: Act, step: i32) {
        match act {
            Act::None => {}
            Act::Close => {
                self.menus.pop();
                if self.menus.is_empty() && matches!(self.mode, Mode::Title) {
                    self.menus.push((MenuId::Main, (0, 0)));
                }
            }
            Act::Open(id) => {
                let sel = match &id {
                    MenuId::EraseConfirm(_) => (0, 0),
                    _ => (0, 0),
                };
                self.menus.push((id, sel));
            }
            Act::Continue => {
                self.audio.sound("confirm");
                self.start_trans(Then::Continue);
            }
            Act::StartGame => self
                .menus
                .push((MenuId::Slots, (self.profile.last_slot(), 0))),
            Act::Credits => self.start_trans(Then::Credits(CreditsKind::Menu)),
            Act::Exit => self.quit = true,
            Act::PickSlot(i) => {
                self.profile.set_last_slot(i);
                self.profile.save_global();
                self.menus.push((MenuId::Packs { page: 0 }, (0, 0)));
            }
            Act::EraseSlot(i) => {
                self.profile.slots[i].erase();
                self.profile.save_slot(i);
                self.log.push(format!("erase slot {i}"));
                self.menus.pop();
            }
            Act::PickPack(w) => {
                let slot = self.profile.last_slot();
                if self.ext_pack_notice(&w) {
                    return;
                }
                if w == "baba" && !self.profile.slots[slot].flag("baba", "intro") {
                    self.menus.push((MenuId::IntroPrompt { world: w }, (1, 0)));
                } else {
                    self.audio.sound("confirm");
                    self.start_trans(Then::StartWorld(slot, w));
                }
            }
            Act::Intro(watch, w) => {
                // Both answers start the world and record the intro as
                // seen; Yes plays it in the first level.
                let slot = self.profile.last_slot();
                self.profile.slots[slot].set_flag("baba", "intro", true);
                self.intro_next = watch;
                self.start_trans(Then::StartWorld(slot, w));
            }
            Act::PackPage(p) => {
                if let Some(top) = self.menus.last_mut() {
                    top.0 = MenuId::Packs { page: p };
                }
            }
            Act::SwitchSlot => {
                self.menus.pop();
            }
            Act::RenameDone => {}
            Act::Slider(sl) => {
                let s = &mut self.profile.settings;
                match sl {
                    Slider::Music => s.set_music(s.music() + 10 * step as i64),
                    Slider::Sound => s.set_sound(s.sound() + 10 * step as i64),
                    Slider::Delay => s.set_delay(s.delay() + step as i64),
                }
                self.apply_settings();
            }
            Act::Toggle(tg) => {
                let s = &mut self.profile.settings;
                let v = !s.toggle(tg);
                s.set_toggle(tg, v);
                if tg == Toggle::Fullscreen {
                    self.fullscreen = Some(v);
                }
                self.apply_settings();
            }
            Act::Zoom(z) => {
                self.profile.settings.set_zoom(z);
                self.apply_settings();
            }
            Act::Language(code) => {
                if self.texts.switch(&code) {
                    self.profile.settings.set_language(&code);
                    self.gfx.upper = !self.texts.current.ttf;
                    self.gfx.fonts.use_sc = code == "zhcn";
                    self.gfx.fonts.pixel = !self.texts.current.ttf;
                    self.apply_settings();
                    self.log.push(format!("language {code}"));
                }
            }
            Act::BindKey(a, sec) => {
                self.input.capture = Some(Capture::Key(a, sec));
                self.menus.push((MenuId::Waiting, (0, 0)));
            }
            Act::BindPad(a, sec) => {
                self.input.capture = Some(Capture::Pad(a, sec));
                self.menus.push((MenuId::Waiting, (0, 0)));
            }
            Act::DefaultKeys => {
                self.profile.settings.reset_keys();
                self.apply_settings();
            }
            Act::DefaultPad => {
                self.profile.settings.reset_pad();
                self.apply_settings();
            }
            Act::TogglePad => {
                let v = !self.profile.settings.pad_disabled();
                self.profile.settings.set_pad_disabled(v);
                self.apply_settings();
            }
            Act::DetectPad => {}
            Act::Resume => {
                self.menus.clear();
            }
            Act::ReturnToMap => {
                self.menus.clear();
                self.start_trans(Then::Turn(Turn::Leave));
            }
            Act::Restart => {
                if self.profile.settings.toggle(Toggle::RestartNoAsk) {
                    self.menus.clear();
                    self.restart_now();
                } else {
                    self.menus.push((MenuId::RestartConfirm, (0, 0)));
                }
            }
            Act::RestartNow => {
                self.menus.clear();
                self.restart_now();
            }
            Act::ReturnToMenu => {
                self.menus.clear();
                self.start_trans(Then::ToTitle);
            }
            Act::EnterLevel(target, unit) => {
                self.menus.clear();
                self.start_trans(Then::Turn(Turn::Enter { target, unit }));
            }
            Act::Ext(a) => self.ext_activate(a),
            Act::Editor(a) => self.ed_activate(a),
            Act::Multiplayer => self.open_lobby(),
            Act::Online(a) => self.online_activate(a),
            Act::Osk(k) => self.osk_key(k),
            Act::LeaveGame => {}
        }
    }

    // ------------------------------------------------------------- render

    fn scale_mode(&self) -> ScaleMode {
        match self.profile.settings.zoom() {
            Zoom::None => ScaleMode::Integer,
            Zoom::Stretch => ScaleMode::Fit,
            Zoom::Default => ScaleMode::default(),
        }
    }

    fn wobble(&self) -> u32 {
        if self.profile.settings.toggle(Toggle::NoWobble) {
            0
        } else {
            (self.ticks / WOBBLE_TICKS) as u32
        }
    }

    fn ensure_title_bg(&mut self) {
        if self.title_bg.is_some() {
            return;
        }
        if let Some(w) = WorldInfo::open(&self.data, "baba") {
            let first = w.data.first_level.clone();
            let mut env = Env {
                data_dir: &self.data,
                objects: &self.objects,
                atlas: &mut self.gfx.atlas,
            };
            self.title_bg = Current::load(&mut env, &w, &first, None, Vec::new(), None, None).ok();
        }
    }

    /// Draws a frame of `size` pixels on the current target.
    pub fn render(&mut self, size: (u32, u32)) {
        self.pointer.1 = size;
        let (w, h) = size;
        self.screen = size;
        let bg = self.gfx.pal(1, 0);
        self.renderer.begin_frame(w, h, bg);
        let mut ui = self.screen_ui(w, h);
        let wob = self.wobble();
        let mode_scale = self.scale_mode();
        match &mut self.mode {
            Mode::Boot(_) => {}
            Mode::Title => {}
            Mode::Credits(c) => {
                if let Some(s) = c.show.as_ref().filter(|s| !s.scrolling) {
                    // The island / logos, then the flowers (rotated).
                    let tex = *self
                        .cut_tex
                        .get_or_insert_with(|| crate::cutscene::Tex::new(&mut self.gfx.atlas));
                    let mut back = Ui::new(w, h);
                    s.draw_back(&mut back, &mut self.gfx, wob);
                    let rot = s.flowers(&back, &self.gfx, &tex);
                    self.renderer.sync_atlas(&mut self.gfx.atlas);
                    self.renderer.draw_overlay(&self.gfx.atlas, &back.quads);
                    self.renderer.draw_overlay_rotated(&self.gfx.atlas, &rot);
                }
            }
            Mode::Play(p) => {
                let states = p.cur.path_states();
                // A cutscene hides, moves or takes over some units.
                p.cur.gfx.overrides = match &self.cut {
                    Some(Cutscene::Intro(i)) => Some(i.overrides()),
                    Some(Cutscene::End(e)) => Some(e.overrides()),
                    _ => None,
                };
                let tex = *self
                    .cut_tex
                    .get_or_insert_with(|| crate::cutscene::Tex::new(&mut self.gfx.atlas));
                let blossom = self.gfx.sprites.get(&mut self.gfx.atlas, "blossom", 0);
                let scene = p.cur.gfx.scene(
                    &mut self.gfx.atlas,
                    &self.data,
                    &p.cur.level,
                    &states,
                    Some(&mut p.cur.fx),
                    &mut self.gfx.art,
                    wob,
                );
                let (sx, sy) = p.cur.fx.shake_offset();
                self.renderer.sync_atlas(&mut self.gfx.atlas);

                let view_scale = self.view_fit.map_or(1.0, |f| f.scale);
                // All Is Done moves the level out of view (phase 2).
                let lift = match &self.cut {
                    Some(Cutscene::Done(d)) if d.level_gone() => 4.0 * h as f32 / view_scale,
                    _ => 0.0,
                };
                let view = View {
                    scene: &scene,
                    wobble: wob,
                    rect: Rect::new(0, 0, w as i32, h as i32),
                    scale: mode_scale,
                    effect: PostEffect {
                        tint: [255; 4],
                        offset: [
                            (sx as f32 + p.cur.fx.view_scroll().0 * 24.0) * view_scale,
                            (sy as f32 + p.cur.fx.view_scroll().1 * 24.0 - lift) * view_scale,
                        ],
                        rotation: -p.cur.fx.view_rotation().to_radians(),
                        ..Default::default()
                    },
                };
                let fit = self.renderer.draw_view(0, &self.gfx.atlas, &view);
                self.view_fit = Some(fit);
                // The cutscene's own pieces over the level (rotated).
                let place = crate::cutscene::Place {
                    x0: fit.rect.x as f32 + sx as f32 * fit.scale,
                    y0: fit.rect.y as f32 + sy as f32 * fit.scale,
                    k: fit.scale,
                };
                let gp = &p.cur.gfx;
                let pal = |c: (u8, u8)| gp.pal(c.0, c.1);
                let atlas = &self.gfx.atlas;
                let size = |r: RegionId| {
                    let x = atlas.region(r);
                    (x.w as u32, x.h as u32)
                };
                let mut decor: Vec<(baba_render::Quad, f32)> =
                    crate::cutscene::decor_flowers(&p.cur.info)
                        .iter()
                        .flat_map(|f| f.quads(&tex, &pal, &place))
                        .collect();
                let rot = match &mut self.cut {
                    Some(Cutscene::Intro(i)) => {
                        let det = gp
                            .overrides
                            .as_ref()
                            .map(|o| o.detached.clone())
                            .unwrap_or_default();
                        i.quads(&det, &size, wob, &tex, &pal, &place)
                    }
                    Some(Cutscene::End(e)) => e.quads(blossom, &size, wob, &tex, &pal, &place),
                    _ => Vec::new(),
                };
                decor.extend(rot);
                self.renderer.draw_overlay_rotated(&self.gfx.atlas, &decor);
                // 3D mode draws the first-person view over the level (§8).
                crate::overlays::vision(
                    &mut self.renderer,
                    &self.gfx,
                    &p.cur,
                    Rect::new(0, 0, w as i32, h as i32),
                    view_scale,
                    scene.level_bg,
                    wob,
                );
            }
            Mode::Multi(m) => {
                let mut cx = Cx {
                    data: &self.data,
                    objects: &self.objects,
                    gfx: &mut self.gfx,
                    texts: &self.texts,
                    profile: &mut self.profile,
                    audio: &mut self.audio,
                    cursor: self.cursor_sprite,
                };
                m.render(
                    &mut cx,
                    &mut self.renderer,
                    &mut ui,
                    (w, h),
                    wob,
                    mode_scale,
                );
            }
            Mode::Join(_) | Mode::Online => {}
            Mode::Editor => self.render_editor(w, h, wob),
        }
        if matches!(self.mode, Mode::Title) && !self.gfx.art.available() {
            self.ensure_title_bg();
            if let Some(cur) = &mut self.title_bg {
                cur.fx.tick(&cur.level);
                let scene = cur.gfx.scene(
                    &mut self.gfx.atlas,
                    &self.data,
                    &cur.level,
                    &[],
                    Some(&mut cur.fx),
                    &mut self.gfx.art,
                    wob,
                );
                self.renderer.sync_atlas(&mut self.gfx.atlas);
                let view = View {
                    scene: &scene,
                    wobble: wob,
                    rect: Rect::new(0, 0, w as i32, h as i32),
                    scale: mode_scale,
                    effect: PostEffect {
                        tint: [70, 70, 80, 255],
                        ..Default::default()
                    },
                };
                self.renderer.draw_view(0, &self.gfx.atlas, &view);
            }
        }
        self.draw_overlay(&mut ui, wob);
        self.renderer.sync_atlas(&mut self.gfx.atlas);
        self.renderer.draw_overlay(&self.gfx.atlas, &ui.quads);
    }

    fn draw_overlay(&mut self, ui: &mut Ui, wob: u32) {
        if matches!(self.mode, Mode::Editor) {
            self.draw_editor(ui);
        }
        let g = &mut self.gfx;
        let white = g.pal(0, 3);
        match &self.mode {
            Mode::Boot(_) => {
                ui.fill(g, g.pal(1, 0));
                ui.text(
                    g,
                    None,
                    "baba-port",
                    427.0,
                    150.0,
                    40.0,
                    g.pal(4, 1),
                    Align::Center,
                );
                ui.text(
                    g,
                    None,
                    "an unofficial reimplementation of Baba Is You",
                    427.0,
                    215.0,
                    16.0,
                    white,
                    Align::Center,
                );
                ui.text(
                    g,
                    None,
                    "game by Arvi Teikari (Hempuli) - data from your own copy",
                    427.0,
                    240.0,
                    16.0,
                    g.pal(0, 1),
                    Align::Center,
                );
                let hold = self.texts.get("intro_hold");
                ui.text(
                    g,
                    Some(&self.texts),
                    &hold,
                    427.0,
                    440.0,
                    14.0,
                    g.pal(0, 1),
                    Align::Center,
                );
            }
            Mode::Title => {
                // Engine art when available (title.rs), else the logo in
                // large letters with a dashed line under it.
                crate::title::background(ui, g, self.ticks, wob);
                if !crate::title::logo(ui, g, wob) {
                    ui.text(
                        g,
                        None,
                        "$4,1BABA $0,3IS $4,1YOU",
                        427.0,
                        52.0,
                        110.0,
                        white,
                        Align::Center,
                    );
                    let pink = g.pal(0, 3);
                    let mut x = 228.0;
                    while x < 626.0 {
                        ui.rect(g, x, 192.0, 14.0, 4.0, pink);
                        x += 22.0;
                    }
                }
                let ver = format!(
                    "{} baba-port {}",
                    self.texts.get("main_version"),
                    env!("CARGO_PKG_VERSION")
                );
                // At the left edge on narrow screens too, clear of the
                // buttons: in the band under the canvas (1:1), else at the
                // top (4:3, where the buttons reach the bottom).
                let mut corner = ui.edge(false, TITLE_SPAN);
                let y = if !ui.narrow() {
                    458.0
                } else if corner.height - corner.y(480.0) >= 20.0 * corner.scale {
                    corner.ly(corner.height) - 22.0
                } else {
                    corner.ly(0.0) + 6.0
                };
                corner.text(g, Some(&self.texts), &ver, 6.0, y, 14.0, white, Align::Left);
                ui.quads.append(&mut corner.quads);
            }
            Mode::Credits(c) => c.draw(ui, g, &self.texts, wob),
            Mode::Join(l) => l.draw(ui, g, &self.texts, &self.profile),
            Mode::Multi(_) | Mode::Online | Mode::Editor => {}
            Mode::Play(p) => {
                let s = &self.profile.slots[p.slot];
                // On 4:3 and 1:1 screens the HUD keeps to the screen's
                // corners at a readable size.
                if p.cur.is_map() && p.title.is_none() {
                    let name = p.hud_name(&self.texts);
                    let mut left = ui.edge(false, HUD_SPAN);
                    left.text(
                        g,
                        Some(&self.texts),
                        &name,
                        21.0,
                        2.0,
                        16.0,
                        white,
                        Align::Left,
                    );
                    ui.quads.append(&mut left.quads);
                }
                // Counters on maps and on levels flagged `showhud`.
                let hud = p.cur.is_map()
                    || p.cur
                        .info
                        .ini
                        .get("general", "showhud")
                        .is_some_and(|v| v.trim() == "1");
                if hud && p.title.is_none() {
                    let wid = &p.world.id;
                    let counts = [s.prize_total(wid), s.clears_total(wid), s.bonus_total(wid)];
                    let mut right = ui.edge(true, HUD_SPAN);
                    crate::overlays::hud_counters(&mut right, g, counts, wob);
                    ui.quads.append(&mut right.quads);
                }
                // Sign texts beside a player (museum, New Adventures).
                if p.title.is_none() && !p.cur.is_map() {
                    let lines = crate::overlays::sign_lines(&self.texts, &p.cur.info, &p.cur.level);
                    crate::overlays::signs(ui, g, &self.texts, &lines);
                }
                let intro = matches!(&self.cut, Some(Cutscene::Intro(i)) if !i.released);
                if let Some(fit) = self.view_fit
                    && !intro
                {
                    controls_overlay(ui, g, &self.texts, &self.profile.settings, &p.cur.info, fit);
                }
                if let Some(Cutscene::Done(d)) = &self.cut {
                    let tex = self
                        .cut_tex
                        .unwrap_or_else(|| crate::cutscene::Tex::new(&mut g.atlas));
                    let dir = self.data.join("Worlds/baba/Images");
                    let island: Vec<RegionId> = (1..=3)
                        .filter_map(|n| g.image(dir.join(format!("island_full_{n}.png"))))
                        .collect();
                    d.draw(ui, g, &island, &tex, wob);
                }
                if self.no_player.is_some_and(|n| n >= UNDO_HINT_FRAMES) && p.title.is_none() {
                    undo_hint(ui, g, &self.texts, &self.profile.settings, wob);
                }
                if let Some((b, left)) = &p.banner {
                    // The original's banner art where there is one (the
                    // victory banner lasts WIN_FRAMES + 30, the clear one
                    // 120 frames: the age follows from what is left).
                    let clear = *b == self.texts.get("ingame_clear");
                    let won = *b == self.texts.get("ingame_victory");
                    let start = if clear { 120 } else { WIN_FRAMES + 30 };
                    let art = (clear || won)
                        && crate::overlays::art_banner(
                            ui,
                            g,
                            clear,
                            start.saturating_sub(*left),
                            wob,
                        );
                    if !art {
                        ui.text(
                            g,
                            Some(&self.texts),
                            b,
                            427.0,
                            200.0,
                            40.0,
                            g.pal(4, 1),
                            Align::Center,
                        );
                    }
                }
                if p.title.is_some() {
                    let label = p.label(&self.texts);
                    let name = p.level_name(&self.texts, &p.cur.stem);
                    let mut card = ui.widened(HUD_SPAN);
                    crate::overlays::title_card(
                        &mut card,
                        Some(&mut *ui),
                        g,
                        label.as_deref(),
                        &name,
                        &p.cur.meta.subtitle,
                    );
                    ui.quads.append(&mut card.quads);
                }
            }
        }
        // Online: the room lobby, connection notes in sessions.
        match &self.mode {
            Mode::Online => self.draw_online(ui),
            Mode::Multi(m) if m.net.is_some() => self.draw_online_status(ui),
            _ => {}
        }
        // Menus (topmost only), dimmed background in play.
        if let Some((id, sel)) = self.menus.last().cloned() {
            if matches!(self.mode, Mode::Play(_)) || id != MenuId::Main {
                let mut c = self.gfx.pal(1, 0);
                c[3] = if matches!(self.mode, Mode::Play(_)) {
                    200
                } else {
                    170
                };
                // The editor draws its own backdrop behind its menus.
                if id != MenuId::Main && !matches!(self.mode, Mode::Editor) {
                    ui.fill(&self.gfx, c);
                }
            }
            let menu = self.build_menu(&id);
            let mut mui = menu.ui_for(ui, &mut self.gfx, &self.texts);
            menu.draw(
                &mut mui,
                &mut self.gfx,
                &self.texts,
                sel,
                self.cursor_sprite,
            );
            if let MenuId::Ext(e) = &id {
                self.ext_draw(&mut mui, e, menu.fix(sel));
            }
            if let MenuId::Editor(e) = &id {
                self.ed_menu_draw(&mut mui, e, menu.fix(sel));
            }
            if self.slide != (0.0, 0.0) {
                let (dx, dy) = (self.slide.0 * mui.scale, self.slide.1 * mui.scale);
                for q in &mut mui.quads {
                    q.x += dx;
                    q.y += dy;
                }
            }
            ui.quads.append(&mut mui.quads);
        }
        // Transition: blobs of colour (1,0) closing to the centre, then
        // opening (presentation.md §11.1, measured).
        if let Some(tr) = &self.trans {
            let title = matches!(&self.mode, Mode::Play(p) if p.title.is_some());
            // The title card draws its own background under its text.
            crate::overlays::iris(ui, &mut self.gfx, &tr.iris, !title);
        }
    }

    /// Reads the current frame back (after [`App::render`]).
    pub fn read_screen(&self) -> RgbaImage {
        self.renderer.read_screen()
    }

    // ----------------------------------------------------- script helpers

    /// A short description of where the game is (for scripts and logs).
    pub fn state(&self) -> String {
        let menu = self
            .menus
            .last()
            .map(|(m, sel)| format!(" menu={m:?} sel={sel:?}"))
            .unwrap_or_default();
        let mode = match &self.mode {
            Mode::Boot(_) => "boot".to_string(),
            Mode::Title => "title".to_string(),
            Mode::Credits(c) => format!("credits {:?}", c.kind),
            Mode::Play(p) => format!(
                "play {}/{}{} tree={}",
                p.world.id,
                p.cur.stem,
                if p.cur.is_map() { " (map)" } else { "" },
                p.tree.stems().join(",")
            ),
            Mode::Join(l) => format!(
                "join {:?} players={} ready={}",
                l.mode,
                l.cards.len(),
                l.cards.iter().filter(|c| c.ready).count()
            ),
            Mode::Multi(m) if m.net.is_some() => {
                format!("{} | {}", m.describe(), self.online_state())
            }
            Mode::Multi(m) => m.describe(),
            Mode::Editor => self.editor_state(),
            Mode::Online => self.online_state(),
        };
        let online = match &self.mode {
            Mode::Title if self.online_active() => format!(" | {}", self.online_state()),
            _ => String::new(),
        };
        let cut = match &self.cut {
            Some(Cutscene::Intro(_)) => " cutscene=intro",
            Some(Cutscene::End(_)) => " cutscene=end",
            Some(Cutscene::Done(_)) => " cutscene=done",
            None => "",
        };
        let show = match &self.mode {
            Mode::Credits(c) if c.show_running() => " show",
            _ => "",
        };
        format!(
            "{mode}{menu}{cut}{show}{}{online}",
            if self.trans.is_some() {
                " transition"
            } else {
                ""
            }
        )
    }

    /// True while nothing blocks input (no transition, title card,
    /// animation or win delay).
    pub fn idle(&self) -> bool {
        if self.trans.is_some() {
            return false;
        }
        let cut = match &self.cut {
            Some(Cutscene::Intro(i)) => !i.released,
            Some(_) => true,
            None => false,
        };
        match &self.mode {
            Mode::Boot(_) => false,
            Mode::Play(p) => !cut && p.title.is_none() && p.anim.is_none() && p.won.is_none(),
            Mode::Credits(c) => !c.show_running(),
            Mode::Multi(m) => m.idle(),
            _ => true,
        }
    }

    /// The instance a player plays: single player (`None` or seat 0), or
    /// a multiplayer seat.
    fn play_of(&self, seat: Option<usize>) -> Option<&Play> {
        match &self.mode {
            Mode::Play(p) if seat.unwrap_or(0) == 0 => Some(p),
            Mode::Multi(m) => match seat {
                Some(s) => m.play_of(s),
                None => m.views.first().map(|v| &v.play),
            },
            _ => None,
        }
    }

    /// Multiplayer: seat ids and devices of the joined players.
    pub fn seats(&self) -> Vec<(usize, Option<Source>)> {
        let players = match &self.mode {
            Mode::Join(l) => &l.players,
            Mode::Multi(m) => &m.players,
            Mode::Online => match &self.online.lobby {
                Some(l) => &l.players,
                None => return Vec::new(),
            },
            _ => return Vec::new(),
        };
        players.seats.iter().map(|s| (s.id, s.source)).collect()
    }

    /// The seat's instance accepts input.
    pub fn player_idle(&self, seat: usize) -> bool {
        match &self.mode {
            Mode::Multi(m) => self.trans.is_none() && m.player_idle(seat),
            _ => self.idle(),
        }
    }

    /// The current level of a player's instance.
    pub fn player_level(&self, seat: Option<usize>) -> Option<(String, String, bool)> {
        self.play_of(seat)
            .map(|p| (p.world.id.clone(), p.cur.stem.clone(), p.cur.is_map()))
    }

    /// State of the icon of `target` on a player's current map.
    pub fn player_icon_state(&self, seat: Option<usize>, target: &str) -> Option<i32> {
        let p = self.play_of(seat)?;
        p.cur.layout.as_ref()?.icon_state(target)
    }

    /// Hash of a player's instance (`Play::state_hash`).
    pub fn player_hash(&self, seat: Option<usize>) -> Option<u64> {
        self.play_of(seat).map(|p| p.state_hash())
    }

    /// The current level stem, if playing.
    pub fn current_level(&self) -> Option<(String, String, bool)> {
        self.player_level(None)
    }

    /// Cursor tile and what it is on (maps).
    pub fn cursor(&self) -> Option<((i32, i32), Option<String>)> {
        self.player_cursor(None)
    }

    /// A player's map cursor tile and what it is on.
    pub fn player_cursor(&self, seat: Option<usize>) -> Option<((i32, i32), Option<String>)> {
        let p = self.play_of(seat)?;
        let c = p.cur.cursors().first().map(|u| (u.x, u.y))?;
        Some((
            c,
            p.cur.candidates(&p.world).first().map(|(t, _)| t.clone()),
        ))
    }

    /// Icon position of `target` on the current map.
    pub fn icon_pos(&self, target: &str) -> Option<(i32, i32)> {
        self.player_icon_pos(None, target)
    }

    /// Icon position of `target` on a player's map.
    pub fn player_icon_pos(&self, seat: Option<usize>, target: &str) -> Option<(i32, i32)> {
        let p = self.play_of(seat)?;
        p.cur
            .level
            .unit_records()
            .into_iter()
            .find(|u| !u.dead && u.icon.as_ref().is_some_and(|i| i.file == target))
            .map(|u| (u.x, u.y))
    }

    /// Walkable cells of the current map for the cursor (icons with state
    /// > 1, paths with state > 1).
    pub fn walkable(&self) -> Vec<(i32, i32)> {
        self.player_walkable(None)
    }

    /// Walkable cells of a player's map.
    pub fn player_walkable(&self, seat: Option<usize>) -> Vec<(i32, i32)> {
        let Some(p) = self.play_of(seat) else {
            return Vec::new();
        };
        let mut v: Vec<(i32, i32)> = p
            .cur
            .level
            .unit_records()
            .into_iter()
            .filter(|u| !u.dead && u.visible && u.completion() > 1)
            .map(|u| (u.x, u.y))
            .collect();
        if let Some(l) = &p.cur.layout {
            for path in &l.paths {
                let open = if path.gate != 0 {
                    path.state >= 3
                } else {
                    path.state > 0
                };
                if open {
                    v.push((path.x, path.y));
                }
            }
        }
        v
    }

    pub fn profile_dir(&self) -> Option<&Path> {
        self.cfg.data_dir.as_deref()
    }
}
