//! The level editor's screens and their glue into [`App`]
//! (`docs/spec/editor.md`): the start screen, help and settings, level
//! and pack lists, the main view with its keyboard / mouse / touch /
//! gamepad controls, the palette and object list, object editing, level
//! settings and size, themes, map editing (icons, paths, specials, map
//! setup), dialogs, text prompts (with an on-screen keyboard for pads and
//! touch), test play through the normal play path, saving, the tutorial.
//! The editing model lives in [`crate::editor`]; this module only drives
//! it, so `app.rs` needs a handful of hooks.

use std::path::PathBuf;

use baba_platform::{Event, Key, TouchPhase};
use baba_render::{PostEffect, Rect, View};

use super::{App, Mode};
use crate::editor::model::{self, AddError, EdLevel, MAX_PALETTE};
use crate::editor::tutorial::{self, Bg, Demo};
use crate::editor::view::{self, BtnAct, EdGfx};
use crate::editor::{
    Brush, Collection, ColourFor, Editor, ListMode, Pick, Press, Screen, TestRun, files, pad,
};
use crate::input::Cmd;
use crate::menu::{Act, Item, Menu, MenuId, Style};
use crate::packs;
use crate::play::{Env, Play, SingleLevel};
use crate::ui::{Align, Ui};
use crate::world::WorldInfo;

/// Levels per page in the level list.
const LEVELS_PER_PAGE: usize = 8;
/// Entries per page in the music / palette / particle / theme lists.
const LIST_PER_PAGE: usize = 12;

/// Editor menus (inside [`MenuId::Editor`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EdMenu {
    /// "Welcome to the level editor!" (`§2.1`).
    Start,
    /// Help & general settings (`§2.2`).
    Help,
    /// Lessons and the shortcut list (`§2.3`).
    Tutorials,
    /// The shortcut list (`§15`).
    Hotkeys {
        page: usize,
    },
    /// Editor gamepad bindings.
    PadBinds {
        page: usize,
    },
    /// Level list (`§3.1`).
    Levels {
        page: usize,
    },
    /// Levelpack list (`§3.3`).
    Packs {
        page: usize,
    },
    /// Level menu (Escape, `§8.1`).
    LevelMenu,
    /// Quick menu (gamepad, `§8.2`).
    QuickMenu,
    /// Level settings (F1, `§7`).
    Settings,
    /// Level size (`§7.8`).
    Size {
        w: u32,
        h: u32,
    },
    Music {
        page: usize,
    },
    Particles,
    Palettes {
        page: usize,
    },
    /// Themes (`§5.8`).
    Themes,
    ThemeList {
        delete: bool,
        page: usize,
    },
    /// New level: which theme to start from (`§3.2`).
    NewTheme {
        page: usize,
    },
    Confirm(Confirm),
    /// Text entry; `upper` = the on-screen keyboard's shift.
    Prompt {
        kind: Prompt,
        text: String,
        upper: bool,
    },
    /// Edit object (`§5.6`).
    EditObject,
    /// Level icon setup (`§13.1`).
    LevelSetup,
    /// Path setup (`§13.2`).
    PathSetup,
    /// Map setup (`§13.4`).
    MapSetup,
    /// The map's icon pictures (`[icons]` slots, "Map icon setup").
    MapIcons,
    /// Picking a level of the pack.
    PickLevel {
        purpose: PickFor,
        page: usize,
    },
    /// Object list tags (`§5.5`).
    Tags {
        page: usize,
    },
    Notice(String),
    /// Upload confirmation (`levelcodes.md` §6.1).
    Upload,
}

/// Confirmation dialogs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Confirm {
    /// Unsaved changes; leave to the level list.
    LeaveList,
    /// Unsaved changes; leave to the editor start screen.
    LeaveMain,
    /// Unsaved changes; open another level (E over an icon).
    OpenOther(String),
    Delete,
    /// Delete a level from the list.
    DeleteListed(String),
    Copy,
    LoadTheme(PathBuf),
    EmptyLevel,
    OverwriteTheme(String),
    DeleteTheme(PathBuf),
}

/// What a text prompt sets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Prompt {
    NewLevel,
    NewPack,
    RenameLevel,
    Author,
    Subtitle,
    PackName,
    PackAuthor,
    ObjName,
    ObjType,
    ThemeName,
    PalSearch,
    ListSearch,
    SpriteSearch,
    LevelSearch,
    /// Palette F4: add objects by string.
    AutoAdd,
    /// Main view F4: add words by string.
    AutoPick,
    /// Text of a special object.
    Special(usize),
    /// Default author for new levels and packs (`[editor] author`).
    EditorAuthor,
}

impl Prompt {
    /// Header language key, max length, lower case only.
    fn spec(&self) -> (&'static str, usize, bool) {
        match self {
            Prompt::NewLevel => ("namegiving_newlevelname", 24, false),
            Prompt::NewPack => ("namegiving_newworldname", 24, false),
            Prompt::RenameLevel => ("namegiving_changelevelname", 24, false),
            Prompt::Author => ("namegiving_levelauthor", 16, false),
            Prompt::Subtitle => ("namegiving_subtitle", 26, false),
            Prompt::PackName => ("namegiving_changeworldname", 24, false),
            Prompt::PackAuthor => ("namegiving_worldauthor", 16, false),
            Prompt::ObjName => ("namegiving_changeunitname", 24, true),
            Prompt::ObjType => ("namegiving_changeunittype", 12, true),
            Prompt::ThemeName => ("namegiving_newthemename", 16, false),
            Prompt::PalSearch | Prompt::ListSearch => ("namegiving_objectsearchstring", 12, true),
            Prompt::SpriteSearch => ("namegiving_spritesearchstring", 24, true),
            Prompt::LevelSearch => ("namegiving_levelsearchstring", 24, false),
            Prompt::AutoAdd => ("namegiving_currobjlist_autoadd", 64, true),
            Prompt::AutoPick => ("namegiving_editor_autopick", 64, true),
            Prompt::Special(_) => ("editor_entername", 64, false),
            Prompt::EditorAuthor => ("namegiving_levelauthor", 16, false),
        }
    }
}

/// What a level pick sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickFor {
    IconTarget,
    ReturnTo,
}

/// Editor actions (inside [`Act::Editor`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EdAct {
    /// Main menu → Level editor.
    Open,
    /// Leave the editor for the title.
    Quit,
    EditLevels,
    EditPacks,
    Open_(EdMenu),
    /// Pops the top menu.
    Back,
    // Level list.
    LevelPage(usize),
    PackPage(usize),
    OpenLevel(String),
    OpenPack(String),
    NewLevel,
    NewPack,
    SortName,
    SortType,
    SetListMode(ListMode),
    ClearSearch,
    // Settings toggles ([editor] keys).
    ToggleSetting(&'static str),
    DefaultPad,
    BindPad(&'static str),
    Lesson(u8),
    // Level menu.
    CloseMenu,
    Test,
    Upload,
    UploadGo,
    Save,
    ToList,
    ToStart,
    Delete,
    Copy,
    // Level settings.
    SetMusic(String, bool),
    SetParticles(String),
    SetPalette(String, bool),
    ToggleGen(&'static str),
    AutoDelay(i32),
    SizeStep(i32, i32),
    SizeSet(u32, u32),
    SizeApply,
    MusicPage(usize),
    PalettePage(usize),
    // Themes.
    ThemePage(bool, usize),
    PickTheme(PathBuf, bool),
    NewWithTheme(Option<PathBuf>),
    // Dialog answers.
    Yes(Confirm),
    // Prompts.
    Prompt(Prompt),
    TypeChar(char),
    TypeBack,
    TypeShift,
    TypeDone,
    // Edit object.
    Sprite,
    Colour(ColourFor),
    Anim(i32),
    TextType(i32),
    TypeStep(i32),
    Layer(i32),
    ResetObject,
    // Map editing.
    IconState(i32),
    IconStyle(i32),
    IconNumber(i32),
    IconRemove,
    /// Custom icon picture of the icon being set up.
    IconPicture,
    /// Choose the picture of `[icons]` slot N (the map's icon setup).
    IconSlot(u32),
    /// Empty `[icons]` slot N.
    IconSlotReset(u32),
    PickLevel(PickFor, String),
    PickPage(PickFor, usize),
    PathState(i32),
    PathGate(i32),
    PathReq(i32),
    PathObject(i32),
    PathRemove,
    MapType(i32),
    ClearLimit(i32),
    ReturnTo(String),
    // Tags.
    Tag(String),
    ClearTags,
    TagsPage(usize),
    /// Pause menu during a test: back to the editor.
    EndTest,
}

/// Editor state kept by the app.
#[derive(Default)]
pub struct EdHost {
    pub ed: Option<Box<Editor>>,
    pub gfx: EdGfx,
    pub fit: Option<baba_render::Fit>,
    /// Raw events for the editor's own screens, in arrival order.
    events: Vec<Event>,
    /// Logical size of the last frame (pointer mapping).
    screen: (u32, u32),
    /// Waiting for a pad input to bind (editor action).
    capture: Option<&'static str>,
    /// Level stashed while a tutorial runs on its demo level.
    stash: Option<(EdLevel, Screen, bool)>,
    /// Menu shown (not active) behind a tutorial slide.
    pub tut_menu: Option<EdMenu>,
    /// Colour picker selection.
    colour: (u8, u8),
}

fn em(m: EdMenu) -> MenuId {
    MenuId::Editor(m)
}

fn ea(a: EdAct) -> Act {
    Act::Editor(a)
}

fn open(m: EdMenu) -> Act {
    Act::Editor(EdAct::Open_(m))
}

/// US-layout character for a key (prompts).
fn key_char(k: Key, shift: bool) -> Option<char> {
    let c = match k {
        Key::Char(c) => c as char,
        Key::W => 'w',
        Key::A => 'a',
        Key::S => 's',
        Key::D => 'd',
        Key::Z => 'z',
        Key::R => 'r',
        Key::Q => 'q',
        Key::E => 'e',
        Key::P => 'p',
        Key::M => 'm',
        Key::Space => ' ',
        Key::Other(45) => {
            return Some(if shift { '_' } else { '-' });
        }
        Key::Other(46) => {
            return Some(if shift { '+' } else { '=' });
        }
        Key::Other(51) => {
            return Some(if shift { ':' } else { ';' });
        }
        Key::Other(52) => {
            return Some(if shift { '"' } else { '\'' });
        }
        Key::Other(54) => {
            return Some(if shift { '<' } else { ',' });
        }
        Key::Other(55) => {
            return Some(if shift { '>' } else { '.' });
        }
        Key::Other(56) => {
            return Some(if shift { '?' } else { '/' });
        }
        _ => return None,
    };
    if c.is_ascii_digit() && shift {
        return Some(match c {
            '1' => '!',
            '2' => '@',
            '3' => '#',
            '4' => '$',
            '5' => '%',
            '6' => '^',
            '7' => '&',
            '8' => '*',
            '9' => '(',
            _ => ')',
        });
    }
    Some(if shift { c.to_ascii_uppercase() } else { c })
}

/// The on-screen keyboard rows.
const KEYBOARD: [&str; 4] = ["1234567890", "qwertyuiop", "asdfghjkl-", "zxcvbnm_.,"];

impl App {
    fn ed(&self) -> Option<&Editor> {
        self.edh.ed.as_deref()
    }

    fn ed_mut(&mut self) -> Option<&mut Editor> {
        self.edh.ed.as_deref_mut()
    }

    fn user_dir_ed(&self) -> Option<PathBuf> {
        self.cfg.data_dir.clone()
    }

    fn setting(&self, key: &str) -> bool {
        self.profile.settings.store.int("editor", key) != 0
    }

    /// Extended features (`§1.2`): on by default in the port (custom world
    /// packs are in scope, `§18.4`); `[editor] mode=0` turns them off.
    pub(super) fn ed_extended(&self) -> bool {
        self.profile
            .settings
            .store
            .get_int("editor", "mode")
            .unwrap_or(1)
            != 0
    }

    /// Advanced words (`§1.3`): the setting, which ALL IS DONE turns on.
    fn ed_advanced(&self) -> bool {
        self.setting("special") || self.setting("advanced")
    }

    fn ed_author(&self) -> String {
        self.profile
            .settings
            .store
            .get("editor", "author")
            .unwrap_or("")
            .to_string()
    }

    // ------------------------------------------------------------ hooks

    /// Every platform event while in the editor: queued for the editor's
    /// own screens. Pointer events only when no menu is open (menus take
    /// taps and clicks themselves); keys also while a prompt is open.
    pub(super) fn ed_event(&mut self, e: &Event) {
        if !matches!(self.mode, Mode::Editor) {
            return;
        }
        let prompt = matches!(
            self.menus.last(),
            Some((
                MenuId::Editor(EdMenu::Prompt { .. } | EdMenu::Levels { .. }),
                _
            ))
        );
        let capture = self.edh.capture.is_some();
        let keep = match e {
            Event::Key { .. } | Event::Button { .. } | Event::Axis { .. } => {
                self.menus.is_empty() || prompt || capture
            }
            Event::MouseMove { .. }
            | Event::MouseButton { .. }
            | Event::MouseWheel { .. }
            | Event::Touch { .. } => self.menus.is_empty(),
            _ => false,
        };
        // Modifiers are tracked even while menus are open.
        if let Event::Key { key, down, .. } = e
            && let Some(ed) = self.ed_mut()
        {
            match key {
                Key::Ctrl => ed.ctrl = *down,
                Key::Shift => ed.shift = *down,
                Key::Alt => ed.alt = *down,
                _ => {}
            }
        }
        if keep {
            self.edh.events.push(e.clone());
        }
    }

    /// Opens the editor from the main menu (`§1.1`).
    fn ed_open(&mut self) {
        if self.edh.ed.is_none() {
            self.edh.ed = Some(Box::new(Editor::new(&self.data)));
        }
        self.mode = Mode::Editor;
        self.menus = vec![(em(EdMenu::Start), (0, 0))];
        self.edh.events.clear();
        self.audio.music("editorsong");
        self.log.push("editor".into());
        if let Some(u) = self.user_dir_ed() {
            let _ = packs::ensure_user_levels(&u);
            files::clean_test_files(&packs::user_levels_dir(&u));
        }
        if let Some(ed) = self.ed_mut() {
            ed.undo.clear();
            ed.screen = Screen::Menus;
        }
        // First visit: the intro tutorial (`§2.4`).
        if !self.setting("firsttime") {
            self.profile.settings.store.set("editor", "firsttime", 1);
            self.profile.save_settings();
            self.ed_start_lesson(1, true);
        }
    }

    fn ed_quit(&mut self) {
        self.mode = Mode::Title;
        self.menus = vec![(MenuId::Main, (0, 0))];
        self.audio.music("menu");
        self.edh.events.clear();
        self.log.push("editor quit".into());
    }

    /// A level's play from the editor ended (`§12`): back to the editor.
    /// True if it was an editor test.
    pub(super) fn editor_test_done(&mut self, won: bool) -> bool {
        let Some(run) = self.ed_mut().and_then(|e| e.testing.take()) else {
            return false;
        };
        self.log.push(format!("editor test end won={won}"));
        self.back_to_editor();
        if run.upload && won {
            self.ed_upload_after_win();
        }
        true
    }

    fn back_to_editor(&mut self) {
        self.mode = Mode::Editor;
        self.menus.clear();
        self.trans = None;
        self.edh.events.clear();
        if let Some(ed) = self.ed_mut() {
            ed.testing = None;
            ed.screen = Screen::Main;
            ed.stroke = None;
        }
        self.ed_music();
    }

    fn ed_music(&mut self) {
        let custom = !self.setting("nomusic");
        let lvl = self.ed().and_then(|e| e.level.as_ref()).map(|l| {
            (
                l.place.dir.clone(),
                l.info.general.music.clone(),
                l.gstr("localmusic") == "1",
            )
        });
        match lvl {
            Some((dir, m, local)) if custom && !m.is_empty() => {
                self.audio.level_music(&dir.to_string_lossy(), &m, local)
            }
            _ => self.audio.music("editorsong"),
        }
    }

    /// The pause menu's "return" item during a test.
    pub(super) fn editor_pause_item(&self) -> Option<Item> {
        self.ed()?.testing.as_ref()?;
        Some(Item::new(
            self.texts
                .opt("pause_returneditor")
                .unwrap_or_else(|| "Return to editor".into()),
            ea(EdAct::EndTest),
        ))
    }

    /// `state()` text for the editor.
    pub(super) fn editor_state(&self) -> String {
        match self.ed() {
            Some(ed) => {
                let lvl = ed
                    .level
                    .as_ref()
                    .map(|l| format!(" level={}/{}", l.place.world, l.stem))
                    .unwrap_or_default();
                let pick = match &ed.pick {
                    Pick::None => "-".to_string(),
                    Pick::Empty => "empty".to_string(),
                    Pick::Slot(s) => ed
                        .level
                        .as_ref()
                        .and_then(|l| l.table.get(s))
                        .map(|d| d.name.clone())
                        .unwrap_or_else(|| s.clone()),
                };
                let at = ed
                    .level
                    .as_ref()
                    .and_then(|l| l.get(ed.layer, ed.cursor.0, ed.cursor.1))
                    .and_then(|o| ed.level.as_ref()?.table.by_tile(o.tile))
                    .map(|d| d.name.clone())
                    .unwrap_or_else(|| "-".into());
                format!(
                    "editor {:?}{lvl} layer={} tool={:?} brush={:?} undo={}{} cursor={},{} on={at} pick={pick} dir={} q={}{}",
                    ed.screen,
                    ed.layer + 1,
                    ed.tool,
                    ed.brush,
                    ed.undo.len(),
                    if ed.unsaved { " unsaved" } else { "" },
                    ed.cursor.0,
                    ed.cursor.1,
                    ed.dir,
                    ed.qsel,
                    if ed.selection.is_some() {
                        " selection"
                    } else {
                        ""
                    },
                )
            }
            None => "editor".into(),
        }
    }

    // ------------------------------------------------------------- tick

    /// Text prompts: typing, the on-screen keyboard, Enter / Escape.
    /// True if it took the frame.
    pub(super) fn ed_text(&mut self, _cmds: &[Cmd]) -> bool {
        if let Some(a) = self.edh.capture {
            self.ed_capture(a);
            return true;
        }
        // Level list: Delete removes the selected level (after a confirm).
        if let Some((MenuId::Editor(EdMenu::Levels { page }), sel)) = self.menus.last().cloned() {
            for e in std::mem::take(&mut self.edh.events) {
                if let Event::Key {
                    key: Key::Other(76),
                    down: true,
                    ..
                } = e
                    && let Some(stem) = self.listed_level(page, sel.0)
                {
                    self.ed_confirm(Confirm::DeleteListed(stem));
                    return true;
                }
            }
            return false;
        }
        if !matches!(
            self.menus.last(),
            Some((MenuId::Editor(EdMenu::Prompt { .. }), _))
        ) {
            return false;
        }
        let shift = self.ed().is_some_and(|e| e.shift);
        let events = std::mem::take(&mut self.edh.events);
        for e in events {
            let Some((MenuId::Editor(EdMenu::Prompt { kind, .. }), _)) = self.menus.last() else {
                break;
            };
            let kind = kind.clone();
            match e {
                Event::Key {
                    key, down: true, ..
                } => match key {
                    Key::Enter => self.ed_activate(EdAct::TypeDone),
                    Key::Escape | Key::Back => {
                        self.menus.pop();
                    }
                    Key::Backspace => self.ed_activate(EdAct::TypeBack),
                    Key::Up | Key::Down | Key::Left | Key::Right => {
                        let d = match key {
                            Key::Right => 0,
                            Key::Up => 1,
                            Key::Left => 2,
                            _ => 3,
                        };
                        self.menu_cmd(Cmd::Dir(d));
                    }
                    k => {
                        if let Some(c) = key_char(k, shift) {
                            let lower = kind.spec().2;
                            self.ed_activate(EdAct::TypeChar(if lower {
                                c.to_ascii_lowercase()
                            } else {
                                c
                            }));
                        }
                    }
                },
                Event::Button {
                    button, down: true, ..
                } => {
                    use baba_platform::Button as B;
                    match button {
                        B::DPadUp => self.menu_cmd(Cmd::Dir(1)),
                        B::DPadDown => self.menu_cmd(Cmd::Dir(3)),
                        B::DPadLeft => self.menu_cmd(Cmd::Dir(2)),
                        B::DPadRight => self.menu_cmd(Cmd::Dir(0)),
                        B::South => self.menu_cmd(Cmd::Confirm),
                        B::East => self.ed_activate(EdAct::TypeBack),
                        B::West => self.ed_activate(EdAct::TypeShift),
                        B::Start => self.ed_activate(EdAct::TypeDone),
                        B::Back => {
                            self.menus.pop();
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        true
    }

    /// Captures the next pad button for an editor binding.
    fn ed_capture(&mut self, action: &'static str) {
        let events = std::mem::take(&mut self.edh.events);
        for e in events {
            let v = match e {
                Event::Button {
                    button, down: true, ..
                } => format!("b{}", crate::input::pad_index(button)),
                Event::Axis {
                    axis: baba_platform::Axis::TriggerLeft,
                    value,
                    ..
                } if value > 0.5 => "a4+".into(),
                Event::Axis {
                    axis: baba_platform::Axis::TriggerRight,
                    value,
                    ..
                } if value > 0.5 => "a5+".into(),
                Event::Key {
                    key: Key::Escape,
                    down: true,
                    ..
                } => "-".into(),
                _ => continue,
            };
            self.profile
                .settings
                .store
                .set("gamepad_editor", action, &v);
            self.profile.save_settings();
            self.edh.capture = None;
            if matches!(self.menus.last(), Some((MenuId::Waiting, _))) {
                self.menus.pop();
            }
            return;
        }
    }

    /// The editor's own screens, every tick.
    pub(super) fn tick_editor(&mut self, _cmds: &[Cmd]) {
        let Some(ed) = self.ed_mut() else {
            self.ed_quit();
            return;
        };
        ed.ticks += 1;
        if let Some((_, t)) = &mut ed.notice {
            *t = t.saturating_sub(1);
            if *t == 0 {
                ed.notice = None;
            }
        }
        if !self.menus.is_empty() {
            self.edh.events.clear();
            return;
        }
        let screen = self.ed().map(|e| e.screen.clone()).unwrap_or(Screen::Menus);
        if screen == Screen::Menus {
            // No level open and no menu: back to the start screen.
            self.edh.events.clear();
            self.menus.push((em(EdMenu::Start), (0, 0)));
            return;
        }
        // Held pad directions repeat.
        if let Some(pi) = self.ed_mut().and_then(|e| e.pad.tick()) {
            self.ed_pad(pi, &screen);
        }
        let events = std::mem::take(&mut self.edh.events);
        for e in events {
            if !self.menus.is_empty() || !matches!(self.mode, Mode::Editor) {
                break;
            }
            if self.ed().is_some_and(|e| e.tutorial.is_some()) {
                self.ed_tutorial_event(&e);
                continue;
            }
            let screen = self.ed().map(|e| e.screen.clone()).unwrap_or(Screen::Menus);
            let group = if screen == Screen::Main {
                "gamepad_editor"
            } else {
                "gamepad_currobjlist"
            };
            if matches!(e, Event::Button { .. } | Event::Axis { .. }) {
                let settings = self.profile.settings.clone();
                let ins = match self.ed_mut() {
                    Some(ed) => ed.pad.event(&e, &settings, group),
                    None => Vec::new(),
                };
                for pi in ins {
                    self.ed_pad(pi, &screen);
                }
                continue;
            }
            if let Some(ed) = self.ed_mut()
                && matches!(
                    e,
                    Event::Key { .. } | Event::MouseButton { .. } | Event::Touch { .. }
                )
            {
                ed.pad.active = false;
            }
            match screen {
                Screen::Main => self.ed_main_event(&e),
                Screen::Palette => self.ed_palette_event(&e),
                Screen::ObjList => self.ed_list_event(&e),
                Screen::Sprites => self.ed_sprites_event(&e),
                Screen::Colour(w) => self.ed_colour_event(&e, w),
                Screen::Menus => {}
            }
        }
    }

    fn logical(&self, px: f32, py: f32) -> (f32, f32) {
        let ui = Ui::new(self.edh.screen.0.max(1), self.edh.screen.1.max(1));
        view::to_logical(&ui, px, py)
    }

    fn cell_of(&self, lx: f32, ly: f32) -> Option<(i32, i32)> {
        let fit = self.edh.fit?;
        if !(view::TOP..480.0 - view::BOTTOM).contains(&ly) {
            return None;
        }
        let ui = Ui::new(self.edh.screen.0.max(1), self.edh.screen.1.max(1));
        Some(view::cell_at(&ui, &fit, lx, ly))
    }

    // -------------------------------------------------------- main view

    fn ed_main_event(&mut self, e: &Event) {
        let ext = self.ed_extended();
        match *e {
            Event::Key {
                key,
                down,
                repeat: false,
            } => {
                if down {
                    self.ed_main_key(key);
                } else if key == Key::Char(b'f')
                    && let Some(ed) = self.ed_mut()
                {
                    let _ = ed;
                }
            }
            Event::MouseMove { x, y } => self.ed_pointer_move(x, y),
            Event::MouseButton { button, down, x, y } => {
                self.ed_pointer_move(x, y);
                self.ed_pointer_button(button, down, ext);
            }
            Event::MouseWheel { dy, .. } => {
                let Some(ed) = self.ed_mut() else { return };
                let d = if dy > 0.0 { -1 } else { 1 };
                if ed.ctrl {
                    ed.swap();
                } else if ed.shift {
                    ed.tool = ed.tool.cycle(d);
                } else {
                    ed.scroll_quick(d);
                }
            }
            Event::Touch { phase, x, y, .. } => {
                let (w, h) = self.edh.screen;
                let (px, py) = (x * w as f32, y * h as f32);
                self.ed_pointer_move(px, py);
                match phase {
                    TouchPhase::Down => self.ed_pointer_button(1, true, ext),
                    TouchPhase::Up => self.ed_pointer_button(1, false, ext),
                    TouchPhase::Move => {}
                }
            }
            _ => {}
        }
    }

    fn ed_pointer_move(&mut self, px: f32, py: f32) {
        let (lx, ly) = self.logical(px, py);
        let cell = self.cell_of(lx, ly);
        let Some(ed) = self.ed_mut() else { return };
        ed.pointer = Some((lx, ly));
        if let Some(c) = cell
            && c != ed.cursor
        {
            ed.cursor = c;
            ed.drag_to(c.0, c.1);
        }
    }

    fn ed_pointer_button(&mut self, button: u8, down: bool, ext: bool) {
        let Some((lx, ly)) = self.ed().and_then(|e| e.pointer) else {
            return;
        };
        let btns = view::main_buttons(self.ed().unwrap(), &self.texts, ext);
        let hit = btns.iter().find(|b| b.hit(lx, ly)).cloned();
        let cell = self.cell_of(lx, ly);
        match (button, down) {
            (1, true) => {
                if let Some(b) = hit {
                    if b.enabled {
                        self.ed_main_button(b.act);
                    }
                    return;
                }
                if let Some((x, y)) = cell {
                    self.ed_press_at(x, y);
                }
            }
            (1, false) => {
                let c = self.ed().map(|e| e.cursor).unwrap_or((0, 0));
                if let Some(ed) = self.ed_mut() {
                    ed.release(c.0, c.1);
                }
            }
            (3, true) => match hit.map(|b| b.act) {
                Some(BtnAct::Quick(i)) => {
                    if let Some(ed) = self.ed_mut() {
                        ed.toggle_lock(i)
                    }
                }
                _ => {
                    if let (Some((x, y)), Some(ed)) = (cell, self.ed_mut()) {
                        if ed.brush == Brush::Objects {
                            ed.copy_at(x, y);
                        } else {
                            ed.undo_begin();
                            ed.erase_map_thing(x, y);
                            ed.undo_commit();
                        }
                    }
                }
            },
            (2, true) => match hit.map(|b| b.act) {
                Some(BtnAct::Quick(i)) => {
                    if let Some(ed) = self.ed_mut() {
                        ed.clear_quick(i)
                    }
                }
                _ => {
                    if let (Some((x, y)), Some(ed)) = (cell, self.ed_mut()) {
                        ed.undo_begin();
                        if ed.ctrl {
                            ed.clear_tile(x, y);
                        } else {
                            ed.cut_at(x, y);
                        }
                        ed.undo_commit();
                    }
                }
            },
            _ => {}
        }
    }

    /// A press on a level cell (mouse, touch, pad).
    fn ed_press_at(&mut self, x: i32, y: i32) {
        let objects = &self.objects;
        let Some(ed) = self.edh.ed.as_deref_mut() else {
            return;
        };
        ed.cursor = (x, y);
        match ed.press(objects, x, y) {
            Press::IconSetup(i) => {
                ed.setup_icon = Some(i);
                self.menus.push((em(EdMenu::LevelSetup), (0, 0)));
            }
            Press::PathSetup(i) => {
                ed.setup_path = Some(i);
                self.menus.push((em(EdMenu::PathSetup), (0, 0)));
            }
            Press::SpecialSetup(i) => {
                let text = ed
                    .level
                    .as_ref()
                    .and_then(|l| l.info.specials.get(i))
                    .map(|s| s.data.clone())
                    .unwrap_or_default();
                self.menus.push((
                    em(EdMenu::Prompt {
                        kind: Prompt::Special(i),
                        text,
                        upper: false,
                    }),
                    (0, 0),
                ));
            }
            _ => {}
        }
    }

    fn ed_main_button(&mut self, act: BtnAct) {
        match act {
            BtnAct::Layer(i) => {
                if let Some(ed) = self.ed_mut() {
                    ed.layer = i;
                }
            }
            BtnAct::Tool(t) => {
                if let Some(ed) = self.ed_mut() {
                    ed.tool = t;
                    if t != crate::editor::tools::Tool::Select {
                        ed.cancel_selection();
                    }
                }
            }
            BtnAct::Quick(i) => {
                if let Some(ed) = self.ed_mut() {
                    ed.brush = Brush::Objects;
                    ed.pick_quick(i);
                }
            }
            BtnAct::Menu => self.menus.push((em(EdMenu::LevelMenu), (0, 0))),
            BtnAct::Settings => self.menus.push((em(EdMenu::Settings), (0, 0))),
            BtnAct::Objects => self.ed_open_palette(),
            BtnAct::Save => self.ed_save(),
            BtnAct::Undo => {
                if let Some(ed) = self.ed_mut() {
                    ed.do_undo();
                }
            }
            BtnAct::Swap => {
                if let Some(ed) = self.ed_mut() {
                    ed.swap();
                }
            }
            BtnAct::Test => self.ed_test(false),
            _ => {}
        }
    }

    fn ed_open_palette(&mut self) {
        if let Some(ed) = self.ed_mut() {
            ed.screen = Screen::Palette;
            ed.stroke = None;
            if let Some(i) = ed.picked_entry()
                && let Some(l) = &ed.level
            {
                let e = &l.palette[i];
                ed.pal.sel = if ed.pal.full { e.gf } else { e.go };
            }
        }
    }

    fn ed_main_key(&mut self, key: Key) {
        let ext = self.ed_extended();
        let Some(ed) = self.ed_mut() else { return };
        let (ctrl, shift) = (ed.ctrl, ed.shift);
        if ctrl {
            match key {
                Key::Z => {
                    ed.do_undo();
                }
                Key::S => self.ed_save(),
                Key::Other(76) => self.ed_confirm(Confirm::EmptyLevel),
                Key::Char(c @ b'1'..=b'3') => ed.layer = (c - b'1') as usize,
                _ => {}
            }
            return;
        }
        match key {
            Key::Escape | Key::Back => {
                if ed.selection.is_some() {
                    ed.cancel_selection();
                } else {
                    self.menus.push((em(EdMenu::LevelMenu), (0, 0)));
                }
            }
            Key::Tab => self.ed_open_palette(),
            Key::F(1) => self.menus.push((em(EdMenu::Settings), (0, 0))),
            Key::F(3) => self.ed_test(false),
            Key::F(4) => self.ed_prompt(Prompt::AutoPick, String::new()),
            Key::R => self.ed_open_list(),
            Key::Backspace => self.ed_leave(false),
            Key::Up | Key::Down | Key::Left | Key::Right => {
                let d = match key {
                    Key::Right => 0,
                    Key::Up => 1,
                    Key::Left => 2,
                    _ => 3,
                };
                if let Some(sel) = ed.selection.as_mut() {
                    match d {
                        0 => sel.rotate(true),
                        2 => sel.rotate(false),
                        1 => sel.mirror(true),
                        _ => sel.mirror(false),
                    }
                } else {
                    ed.set_dir(d);
                }
            }
            Key::Char(b'i') => ed.quick_pick("text_is"),
            Key::Char(b'u') => ed.quick_pick("text_and"),
            Key::Char(b'y') => ed.quick_pick("text_not"),
            Key::Q => {
                ed.pick = Pick::Empty;
            }
            Key::Char(b'f') => ed.show_dirs = !ed.show_dirs,
            Key::W | Key::A | Key::S | Key::D => {
                let d = match key {
                    Key::D => 0,
                    Key::W => 1,
                    Key::A => 2,
                    _ => 3,
                };
                ed.undo_begin();
                ed.move_all(d);
                ed.undo_commit();
            }
            Key::E if ext => {
                let (x, y) = ed.cursor;
                if let Some(i) = ed.icon_at(x, y) {
                    let target = ed.level.as_ref().map(|l| l.info.levels[i].file.clone());
                    if let Some(t) = target.filter(|t| !t.is_empty()) {
                        if ed.unsaved {
                            self.ed_confirm(Confirm::OpenOther(t));
                        } else {
                            self.ed_open_stem(&t);
                        }
                    }
                }
            }
            Key::Char(c @ b'0'..=b'9') => {
                let i = if c == b'0' { 9 } else { (c - b'1') as usize };
                ed.brush = Brush::Objects;
                ed.pick_quick(i);
            }
            _ => {}
        }
        let _ = shift;
    }

    /// Gamepad inputs (`§15`).
    fn ed_pad(&mut self, pi: pad::PadIn, screen: &Screen) {
        use pad::PadIn;
        match screen {
            Screen::Main => {}
            Screen::Palette => return self.ed_palette_pad(pi),
            Screen::ObjList => return self.ed_list_pad(pi),
            Screen::Sprites => return self.ed_sprites_pad(pi),
            Screen::Colour(w) => return self.ed_colour_pad(pi, *w),
            Screen::Menus => return,
        }
        let settings = self.profile.settings.clone();
        let Some(ed) = self.ed_mut() else { return };
        ed.pointer = None;
        match pi {
            PadIn::Move(d) => {
                let moveall = ed.pad.held_value(&settings, "gamepad_editor", "moveall");
                if moveall {
                    ed.undo_begin();
                    ed.move_all(d);
                    ed.undo_commit();
                    return;
                }
                let (dx, dy) = baba_sim::dir_delta(d);
                if let Some(l) = &ed.level {
                    let (w, h) = (l.width() as i32, l.height() as i32);
                    ed.cursor = (
                        (ed.cursor.0 + dx).clamp(1, w - 2),
                        (ed.cursor.1 + dy).clamp(1, h - 2),
                    );
                }
                let (x, y) = ed.cursor;
                ed.drag_to(x, y);
            }
            PadIn::Rotate(d) => {
                if let Some(sel) = ed.selection.as_mut() {
                    match d {
                        0 => sel.rotate(true),
                        2 => sel.rotate(false),
                        1 => sel.mirror(true),
                        _ => sel.mirror(false),
                    }
                } else {
                    ed.set_dir(d);
                }
            }
            PadIn::Press(a) => {
                let (x, y) = ed.cursor;
                let sel = ed.selection.is_some();
                match a {
                    "place" => self.ed_press_at(x, y),
                    "copy" if sel => ed.cancel_selection(),
                    "copy" => ed.copy_at(x, y),
                    "cut" => {
                        ed.undo_begin();
                        ed.cut_at(x, y);
                        ed.undo_commit();
                    }
                    "drag" if sel => ed.cancel_selection(),
                    "drag" => {
                        if let Some(o) = ed.level.as_ref().and_then(|l| l.get(ed.layer, x, y)) {
                            ed.undo_begin();
                            ed.stroke = Some(crate::editor::Stroke::Drag {
                                from: (x, y),
                                layer: ed.layer,
                                obj: o,
                            });
                        }
                    }
                    "undo" if sel => {
                        // Paste ignoring empty cells.
                        let s = ed.shift;
                        ed.shift = true;
                        self.ed_press_at(x, y);
                        if let Some(ed) = self.ed_mut() {
                            ed.shift = s;
                        }
                    }
                    "undo" => {
                        ed.do_undo();
                    }
                    "emptytile" => {
                        ed.undo_begin();
                        ed.clear_tile(x, y);
                        ed.undo_commit();
                    }
                    "scrollright_hotbar" if sel => {
                        if let Some(s) = ed.selection.as_mut() {
                            s.mirror(true)
                        }
                    }
                    "scrollleft_hotbar" if sel => {
                        if let Some(s) = ed.selection.as_mut() {
                            s.mirror(false)
                        }
                    }
                    "scrollright_hotbar" => ed.scroll_quick(1),
                    "scrollleft_hotbar" => ed.scroll_quick(-1),
                    "scrollright_tool" if sel => {
                        if let Some(s) = ed.selection.as_mut() {
                            s.rotate(true)
                        }
                    }
                    "scrollleft_tool" if sel => {
                        if let Some(s) = ed.selection.as_mut() {
                            s.rotate(false)
                        }
                    }
                    "scrollright_tool" => ed.tool = ed.tool.cycle(1),
                    "scrollleft_tool" => ed.tool = ed.tool.cycle(-1),
                    "scrollright_layer" => ed.layer = (ed.layer + 1) % 3,
                    "scrollleft_layer" => ed.layer = (ed.layer + 2) % 3,
                    "currobjlist" => self.ed_open_palette(),
                    "quickmenu" => self.menus.push((em(EdMenu::QuickMenu), (0, 0))),
                    "swap" => ed.swap(),
                    "showdir" => ed.show_dirs = !ed.show_dirs,
                    "pickempty" => ed.pick = Pick::Empty,
                    "lock" => {
                        let k = ed.qsel;
                        ed.toggle_lock(k);
                    }
                    "empty_hotbar" => {
                        let k = ed.qsel;
                        ed.clear_quick(k);
                    }
                    "autopick" => self.ed_prompt(Prompt::AutoPick, String::new()),
                    "save" => self.ed_save(),
                    "test" => self.ed_test(false),
                    _ => {}
                }
            }
            PadIn::Release(a) => {
                if matches!(a, "place" | "drag") {
                    let (x, y) = ed.cursor;
                    ed.release(x, y);
                }
            }
        }
    }

    // ---------------------------------------------------------- palette

    fn ed_palette_event(&mut self, e: &Event) {
        let ext = self.ed_extended();
        match *e {
            Event::Key {
                key,
                down: true,
                repeat: false,
            } => {
                let Some(ed) = self.ed_mut() else { return };
                match key {
                    Key::Tab | Key::Escape | Key::Back => ed.screen = Screen::Main,
                    Key::Enter | Key::Space => {
                        let sel = ed.pal.sel;
                        self.ed_palette_pick(sel);
                    }
                    Key::Up | Key::Down | Key::Left | Key::Right => {
                        let (dx, dy) = match key {
                            Key::Right => (1, 0),
                            Key::Up => (0, -1),
                            Key::Left => (-1, 0),
                            _ => (0, 1),
                        };
                        ed.pal.sel = (
                            (ed.pal.sel.0 + dx).clamp(0, 14),
                            (ed.pal.sel.1 + dy).clamp(0, 9),
                        );
                    }
                    Key::F(4) => self.ed_prompt(Prompt::AutoAdd, String::new()),
                    Key::Backspace => {
                        ed.pal.search.pop();
                    }
                    Key::Other(76) => {
                        let sel = ed.pal.sel;
                        self.ed_palette_remove(sel);
                    }
                    k => {
                        if let Some(c) = key_char(k, false)
                            && c.is_ascii_alphanumeric()
                            && ed.pal.search.len() < 12
                        {
                            ed.pal.search.push(c);
                        }
                    }
                }
            }
            Event::MouseWheel { .. } => {
                if let Some(ed) = self.ed_mut() {
                    ed.pal.words = !ed.pal.words;
                }
            }
            Event::MouseMove { x, y } => {
                let (lx, ly) = self.logical(x, y);
                if let Some(ed) = self.ed_mut() {
                    ed.pointer = Some((lx, ly));
                }
            }
            Event::MouseButton { button, down, x, y } => {
                let (lx, ly) = self.logical(x, y);
                self.ed_palette_click(button, down, lx, ly, ext);
            }
            Event::Touch { phase, x, y, .. } => {
                let (w, h) = self.edh.screen;
                let (lx, ly) = self.logical(x * w as f32, y * h as f32);
                match phase {
                    TouchPhase::Down => self.ed_palette_click(1, true, lx, ly, ext),
                    TouchPhase::Up => self.ed_palette_click(1, false, lx, ly, ext),
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn ed_palette_click(&mut self, button: u8, down: bool, lx: f32, ly: f32, ext: bool) {
        let Some(ed) = self.ed() else { return };
        let btns = view::palette_buttons(ed, &self.texts, ext);
        let hit = btns.iter().find(|b| b.hit(lx, ly)).cloned();
        let ctrl = ed.ctrl;
        let shift = ed.shift;
        let Some(b) = hit else {
            if !down && let Some(ed) = self.ed_mut() {
                ed.pal.drag = None;
            }
            return;
        };
        if !b.enabled {
            return;
        }
        match (button, down, b.act.clone()) {
            (1, true, BtnAct::Cell(x, y)) if ctrl => {
                if let Some(ed) = self.ed_mut() {
                    ed.pal.drag = view::palette_entry_at(ed, (x, y));
                    ed.pal.sel = (x, y);
                }
            }
            (1, false, BtnAct::Cell(x, y)) => {
                if let Some(ed) = self.ed_mut()
                    && let Some(i) = ed.pal.drag.take()
                {
                    let full = ed.pal.full;
                    if let Some(l) = ed.level.as_mut() {
                        l.move_entry(i, (x, y), !full);
                        ed.unsaved = true;
                    }
                    ed.pal.sel = (x, y);
                }
            }
            (1, true, BtnAct::Cell(x, y)) => self.ed_palette_pick((x, y)),
            (3, true, BtnAct::Cell(x, y)) => {
                if ctrl || shift {
                    self.ed_palette_remove((x, y));
                } else {
                    if let Some(ed) = self.ed_mut() {
                        ed.pal.sel = (x, y);
                    }
                    self.ed_edit_object();
                }
            }
            (1, true, act) => self.ed_palette_button(act),
            _ => {}
        }
    }

    fn ed_palette_button(&mut self, act: BtnAct) {
        let Some(ed) = self.ed_mut() else { return };
        match act {
            BtnAct::Tool(t) => ed.tool = t,
            BtnAct::Search => {
                let s = ed.pal.search.clone();
                self.ed_prompt(Prompt::PalSearch, s);
            }
            BtnAct::ClearSearch => ed.pal.search.clear(),
            BtnAct::Tags => self.ed_open_list(),
            BtnAct::Add => self.ed_open_list(),
            BtnAct::Remove => {
                let sel = ed.pal.sel;
                self.ed_palette_remove(sel);
            }
            BtnAct::Edit => self.ed_edit_object(),
            BtnAct::Pairs => ed.pal.full = !ed.pal.full,
            BtnAct::SwapView => ed.pal.words = !ed.pal.words,
            BtnAct::Return => ed.screen = Screen::Main,
            BtnAct::Dir(d) => ed.dir = d,
            BtnAct::Brush(b) => {
                ed.brush = b;
                if b != Brush::Objects {
                    ed.screen = Screen::Main;
                }
                if b == Brush::Paths {
                    self.ed_ensure_line();
                }
            }
            BtnAct::PathCog => {
                // Path setup of the newest path, else nothing.
                if let Some(n) = ed.level.as_ref().map(|l| l.info.paths.len())
                    && n > 0
                {
                    ed.setup_path = Some(n - 1);
                    self.menus.push((em(EdMenu::PathSetup), (0, 0)));
                }
            }
            _ => {}
        }
    }

    /// Paths need a path object: the palette's `line` (`§13.2`). A
    /// palette without one gets it added (the original falls back to the
    /// palette's first entry, which is rarely meant).
    fn ed_ensure_line(&mut self) {
        let objects = &self.objects;
        let Some(ed) = self.edh.ed.as_deref_mut() else {
            return;
        };
        let Some(l) = ed.level.as_mut() else { return };
        if l.entry_by_name("line").is_none()
            && let Some(id) = ed.cat.by_name("line").map(|o| o.index as i32)
            && l.add_object(objects, &ed.cat, id, true).is_ok()
        {
            ed.unsaved = true;
            ed.undo.clear();
            self.log.push("palette add line".into());
        }
        ed.update_path_object();
    }

    fn ed_palette_pick(&mut self, c: (i32, i32)) {
        let Some(ed) = self.ed_mut() else { return };
        ed.pal.sel = c;
        match view::palette_entry_at(ed, c) {
            Some(i) => {
                if ed.brush != Brush::Objects && ed.brush != Brush::Paths {
                    ed.brush = Brush::Objects;
                }
                if ed.brush == Brush::Paths {
                    ed.path_object = ed.level.as_ref().map(|l| l.palette[i].slot.clone());
                }
                ed.pick_entry(i);
                ed.screen = Screen::Main;
            }
            None => {
                // An empty cell: "pick nothing" (eraser).
                ed.pick = Pick::Empty;
                ed.screen = Screen::Main;
            }
        }
    }

    fn ed_palette_remove(&mut self, c: (i32, i32)) {
        let pairs = self.ed().is_some_and(|e| !e.pal.full);
        let objects = &self.objects;
        let Some(ed) = self.edh.ed.as_deref_mut() else {
            return;
        };
        let Some(i) = view::palette_entry_at(ed, c) else {
            return;
        };
        if let Some(l) = ed.level.as_mut() {
            let removed = l.remove_entry(objects, i, pairs);
            self.log.push(format!("palette remove {removed:?}"));
            ed.unsaved = true;
            ed.undo.clear();
            if let Pick::Slot(s) = &ed.pick
                && removed.contains(s)
            {
                ed.pick = Pick::None;
            }
        }
        ed.update_path_object();
    }

    fn ed_palette_pad(&mut self, pi: pad::PadIn) {
        use pad::PadIn;
        let Some(ed) = self.ed_mut() else { return };
        match pi {
            PadIn::Move(d) | PadIn::Rotate(d) => {
                let (dx, dy) = baba_sim::dir_delta(d);
                ed.pal.sel = (
                    (ed.pal.sel.0 + dx).clamp(0, 14),
                    (ed.pal.sel.1 + dy).clamp(0, 9),
                );
            }
            PadIn::Press(a) => match a {
                "select" => {
                    let s = ed.pal.sel;
                    self.ed_palette_pick(s);
                }
                "closemenu" | "menu" => ed.screen = Screen::Main,
                "edit" => self.ed_edit_object(),
                "addnew" => self.ed_open_list(),
                "remove" => {
                    let s = ed.pal.sel;
                    self.ed_palette_remove(s);
                }
                "swap" => ed.pal.words = !ed.pal.words,
                "tags" => ed.pal.full = !ed.pal.full,
                "scrollleft" => ed.tool = ed.tool.cycle(-1),
                "scrollright" => ed.tool = ed.tool.cycle(1),
                "search" => {
                    let s = ed.pal.search.clone();
                    self.ed_prompt(Prompt::PalSearch, s);
                }
                "autoadd" => self.ed_prompt(Prompt::AutoAdd, String::new()),
                _ => {}
            },
            PadIn::Release(_) => {}
        }
    }

    fn ed_edit_object(&mut self) {
        let Some(ed) = self.ed_mut() else { return };
        let sel = ed.pal.sel;
        let Some(i) = view::palette_entry_at(ed, sel) else {
            return;
        };
        ed.editing = Some(i);
        self.menus.push((em(EdMenu::EditObject), (0, 0)));
    }

    // ------------------------------------------------------ object list

    fn ed_open_list(&mut self) {
        if let Some(ed) = self.ed_mut() {
            ed.screen = Screen::ObjList;
            ed.list.sel = ed.list.page * view::LIST_PAGE;
        }
    }

    fn list_count(&self) -> usize {
        let Some(ed) = self.ed() else { return 0 };
        view::list_items(
            &ed.cat,
            &ed.list.search,
            &ed.list.tags,
            self.ed_extended(),
            self.ed_advanced(),
        )
        .len()
    }

    /// Adds object list entry `i` to the palette; `stay` keeps browsing.
    fn ed_list_add(&mut self, i: usize, stay: bool) {
        let (ext, adv) = (self.ed_extended(), self.ed_advanced());
        // Adding always brings the noun/text partner (`§5.3`); the view
        // toggle only changes how the palette is shown.
        let pairs = true;
        let objects = &self.objects;
        let Some(ed) = self.edh.ed.as_deref_mut() else {
            return;
        };
        let id = {
            let items = view::list_items(&ed.cat, &ed.list.search, &ed.list.tags, ext, adv);
            match items.get(i) {
                Some(o) => o.index as i32,
                None => return,
            }
        };
        let r = match ed.level.as_mut() {
            Some(l) => l.add_object(objects, &ed.cat, id, pairs),
            None => return,
        };
        match r {
            Ok(added) => {
                self.log
                    .push(format!("palette add {id} ({} entries)", added.len()));
                ed.unsaved = true;
                ed.undo.clear();
                if let Some(&first) = added.first() {
                    ed.pick_entry(first);
                    if let Some(l) = &ed.level {
                        let e = &l.palette[first];
                        ed.pal.sel = if ed.pal.full { e.gf } else { e.go };
                    }
                }
                ed.update_path_object();
            }
            Err(AddError::Present(k)) => ed.pick_entry(k),
            Err(AddError::Full) => ed.note(format!("{MAX_PALETTE} / {MAX_PALETTE}")),
            Err(AddError::Unknown) => {}
        }
        if !stay {
            ed.screen = Screen::Palette;
        }
    }

    fn ed_list_event(&mut self, e: &Event) {
        let n = self.list_count();
        match *e {
            Event::Key {
                key,
                down: true,
                repeat: false,
            } => {
                let Some(ed) = self.ed_mut() else { return };
                let ctrl = ed.ctrl;
                match key {
                    Key::Escape | Key::Tab | Key::Back => ed.screen = Screen::Palette,
                    Key::Enter | Key::Space => {
                        let s = ed.list.sel;
                        self.ed_list_add(s, ctrl);
                    }
                    Key::Up | Key::Down | Key::Left | Key::Right => {
                        let d: i64 = match key {
                            Key::Right => 1,
                            Key::Left => -1,
                            Key::Up => -(view::LIST_COLS as i64),
                            _ => view::LIST_COLS as i64,
                        };
                        let s =
                            (ed.list.sel as i64 + d).clamp(0, n.saturating_sub(1) as i64) as usize;
                        ed.list.sel = s;
                        ed.list.page = s / view::LIST_PAGE;
                    }
                    Key::Backspace => {
                        ed.list.search.pop();
                        ed.list.page = 0;
                        ed.list.sel = 0;
                    }
                    k => {
                        if let Some(c) = key_char(k, false)
                            && c.is_ascii_alphanumeric()
                            && ed.list.search.len() < 12
                        {
                            ed.list.search.push(c);
                            ed.list.page = 0;
                            ed.list.sel = 0;
                        }
                    }
                }
            }
            Event::MouseButton {
                button: 1,
                down: true,
                x,
                y,
            } => {
                let (lx, ly) = self.logical(x, y);
                self.ed_list_click(lx, ly, n);
            }
            Event::Touch {
                phase: TouchPhase::Down,
                x,
                y,
                ..
            } => {
                let (w, h) = self.edh.screen;
                let (lx, ly) = self.logical(x * w as f32, y * h as f32);
                self.ed_list_click(lx, ly, n);
            }
            _ => {}
        }
    }

    fn ed_list_click(&mut self, lx: f32, ly: f32, n: usize) {
        let Some(ed) = self.ed() else { return };
        let btns = view::list_buttons(n, ed.list.page, &self.texts, false);
        let ctrl = ed.ctrl;
        let Some(b) = btns.into_iter().find(|b| b.hit(lx, ly) && b.enabled) else {
            return;
        };
        self.ed_list_button(b.act, n, ctrl);
    }

    fn ed_list_button(&mut self, act: BtnAct, n: usize, ctrl: bool) {
        let Some(ed) = self.ed_mut() else { return };
        match act {
            BtnAct::Item(i) => {
                ed.list.sel = i;
                self.ed_list_add(i, ctrl);
            }
            BtnAct::Page(d) => {
                let pages = n.div_ceil(view::LIST_PAGE).max(1) as i64;
                ed.list.page = (ed.list.page as i64 + d as i64).clamp(0, pages - 1) as usize;
                ed.list.sel = ed.list.page * view::LIST_PAGE;
            }
            BtnAct::Return => ed.screen = Screen::Palette,
            BtnAct::Search => {
                let s = ed.list.search.clone();
                self.ed_prompt(Prompt::ListSearch, s);
            }
            BtnAct::ClearSearch => {
                ed.list.search.clear();
                ed.list.page = 0;
            }
            BtnAct::Tags => self.menus.push((em(EdMenu::Tags { page: 0 }), (0, 0))),
            _ => {}
        }
    }

    fn ed_list_pad(&mut self, pi: pad::PadIn) {
        use pad::PadIn;
        let n = self.list_count();
        let Some(ed) = self.ed_mut() else { return };
        match pi {
            PadIn::Move(d) | PadIn::Rotate(d) => {
                let delta: i64 = match d {
                    0 => 1,
                    2 => -1,
                    1 => -(view::LIST_COLS as i64),
                    _ => view::LIST_COLS as i64,
                };
                let s = (ed.list.sel as i64 + delta).clamp(0, n.saturating_sub(1) as i64) as usize;
                ed.list.sel = s;
                ed.list.page = s / view::LIST_PAGE;
            }
            PadIn::Press(a) => match a {
                "select" => {
                    let s = ed.list.sel;
                    self.ed_list_add(s, false);
                }
                "swap" | "addnew" => {
                    let s = ed.list.sel;
                    self.ed_list_add(s, true);
                }
                "closemenu" | "menu" => ed.screen = Screen::Palette,
                "scrollleft" => self.ed_list_button(BtnAct::Page(-1), n, false),
                "scrollright" => self.ed_list_button(BtnAct::Page(1), n, false),
                "tags" => self.menus.push((em(EdMenu::Tags { page: 0 }), (0, 0))),
                "search" => {
                    let s = ed.list.search.clone();
                    self.ed_prompt(Prompt::ListSearch, s);
                }
                _ => {}
            },
            PadIn::Release(_) => {}
        }
    }

    // ---------------------------------------------------- sprite picker

    fn ed_open_sprites(&mut self) {
        let coll = self
            .ed()
            .and_then(|e| e.level.as_ref())
            .map(|l| l.place.dir.clone());
        let list = files::sprites(&self.data, coll.as_deref());
        let Some(ed) = self.ed_mut() else { return };
        // Start on the current sprite.
        let cur = ed.editing.and_then(|i| {
            ed.level
                .as_ref()
                .and_then(|l| l.def(i))
                .map(|d| d.sprite.clone())
        });
        ed.sprites.list = list;
        ed.sprites.icon = false;
        ed.sprites.search.clear();
        let sel = cur
            .and_then(|c| ed.sprites.list.iter().position(|(n, _)| *n == c))
            .unwrap_or(0);
        ed.sprites.sel = sel;
        ed.sprites.page = sel / 80;
        ed.screen = Screen::Sprites;
        self.menus.clear();
    }

    fn ed_sprite_pick(&mut self, i: usize) {
        let objects = &self.objects;
        let Some(ed) = self.edh.ed.as_deref_mut() else {
            return;
        };
        let list = view::sprite_items(ed);
        let Some((name, root)) = list.get(i).cloned() else {
            return;
        };
        if ed.sprites.icon
            && let Some(n) = ed.sprites.slot.take()
        {
            // A slot of the map's own icon setup.
            if let Some(l) = ed.level.as_mut() {
                l.info.icons.retain(|c| c.index != n);
                l.info.icons.push(baba_formats::leveldata::Icon {
                    index: n,
                    file: name,
                    root,
                });
                l.info.icons.sort_by_key(|c| c.index);
                ed.unsaved = true;
            }
            ed.screen = Screen::Main;
            self.menus.push((em(EdMenu::MapIcons), (0, 0)));
            return;
        }
        if ed.sprites.icon {
            if let (Some(k), Some(l)) = (ed.setup_icon, ed.level.as_mut())
                && let Some(icon) = l.info.levels.get(k)
            {
                let n = icon.number.max(0) as u32;
                l.info.icons.retain(|c| c.index != n);
                l.info.icons.push(baba_formats::leveldata::Icon {
                    index: n,
                    file: name,
                    root,
                });
                l.info.icons.sort_by_key(|c| c.index);
                ed.unsaved = true;
            }
            ed.screen = Screen::Main;
            self.menus.push((em(EdMenu::LevelSetup), (0, 0)));
            return;
        }
        if let (Some(k), Some(l)) = (ed.editing, ed.level.as_mut())
            && let Some(mut d) = l.def(k).cloned()
        {
            d.sprite = name.clone();
            d.sprite_in_root = root;
            // Adopt the sprite's usual animation style when the catalogue
            // knows it.
            if let Some(o) = ed
                .cat
                .list
                .objects
                .iter()
                .find(|o| o.sprite.as_deref().unwrap_or(&o.name) == name)
            {
                d.tiling = o.tiling;
            }
            l.set_def(objects, k, d);
            ed.unsaved = true;
        }
        ed.screen = Screen::Palette;
        self.menus.push((em(EdMenu::EditObject), (0, 0)));
    }

    fn ed_sprites_event(&mut self, e: &Event) {
        let n = self.ed().map(|e| view::sprite_items(e).len()).unwrap_or(0);
        match *e {
            Event::Key {
                key,
                down: true,
                repeat: false,
            } => {
                let Some(ed) = self.ed_mut() else { return };
                match key {
                    Key::Escape | Key::Back => self.ed_sprites_back(),
                    Key::Enter => {
                        let s = ed.sprites.sel;
                        self.ed_sprite_pick(s);
                    }
                    Key::Up | Key::Down | Key::Left | Key::Right => {
                        let d: i64 = match key {
                            Key::Right => 1,
                            Key::Left => -1,
                            Key::Up => -10,
                            _ => 10,
                        };
                        let s = (ed.sprites.sel as i64 + d).clamp(0, n.saturating_sub(1) as i64)
                            as usize;
                        ed.sprites.sel = s;
                        ed.sprites.page = s / 80;
                    }
                    Key::Backspace => {
                        ed.sprites.search.pop();
                        ed.sprites.sel = 0;
                        ed.sprites.page = 0;
                    }
                    k => {
                        if let Some(c) = key_char(k, false)
                            && (c.is_ascii_alphanumeric() || c == '_')
                        {
                            ed.sprites.search.push(c);
                            ed.sprites.sel = 0;
                            ed.sprites.page = 0;
                        }
                    }
                }
            }
            Event::MouseButton {
                button: 1,
                down: true,
                x,
                y,
            } => {
                let (lx, ly) = self.logical(x, y);
                self.ed_sprites_click(lx, ly, n);
            }
            Event::Touch {
                phase: TouchPhase::Down,
                x,
                y,
                ..
            } => {
                let (w, h) = self.edh.screen;
                let (lx, ly) = self.logical(x * w as f32, y * h as f32);
                self.ed_sprites_click(lx, ly, n);
            }
            _ => {}
        }
    }

    fn ed_sprites_click(&mut self, lx: f32, ly: f32, n: usize) {
        let Some(ed) = self.ed() else { return };
        let btns = view::list_buttons(n, ed.sprites.page, &self.texts, true);
        let Some(b) = btns.into_iter().find(|b| b.hit(lx, ly) && b.enabled) else {
            return;
        };
        let Some(ed) = self.ed_mut() else { return };
        match b.act {
            BtnAct::Item(i) => self.ed_sprite_pick(i),
            BtnAct::Page(d) => {
                let pages = n.div_ceil(80).max(1) as i64;
                ed.sprites.page = (ed.sprites.page as i64 + d as i64).clamp(0, pages - 1) as usize;
                ed.sprites.sel = ed.sprites.page * 80;
            }
            BtnAct::Return => self.ed_sprites_back(),
            BtnAct::Search => {
                let s = ed.sprites.search.clone();
                self.ed_prompt(Prompt::SpriteSearch, s);
            }
            BtnAct::ClearSearch => {
                ed.sprites.search.clear();
                ed.sprites.page = 0;
            }
            _ => {}
        }
    }

    fn ed_sprites_pad(&mut self, pi: pad::PadIn) {
        use pad::PadIn;
        let n = self.ed().map(|e| view::sprite_items(e).len()).unwrap_or(0);
        let Some(ed) = self.ed_mut() else { return };
        match pi {
            PadIn::Move(d) | PadIn::Rotate(d) => {
                let delta: i64 = match d {
                    0 => 1,
                    2 => -1,
                    1 => -10,
                    _ => 10,
                };
                let s =
                    (ed.sprites.sel as i64 + delta).clamp(0, n.saturating_sub(1) as i64) as usize;
                ed.sprites.sel = s;
                ed.sprites.page = s / 80;
            }
            PadIn::Press("select") => {
                let s = ed.sprites.sel;
                self.ed_sprite_pick(s);
            }
            PadIn::Press("closemenu" | "menu") => self.ed_sprites_back(),
            _ => {}
        }
    }

    fn ed_sprites_back(&mut self) {
        let Some(ed) = self.ed_mut() else { return };
        if ed.sprites.icon {
            ed.screen = Screen::Main;
            self.menus.push((em(EdMenu::LevelSetup), (0, 0)));
        } else {
            ed.screen = Screen::Palette;
            self.menus.push((em(EdMenu::EditObject), (0, 0)));
        }
    }

    /// The map icon picture picker (`§13.1`).
    fn ed_open_icon_pics(&mut self) {
        let coll = self
            .ed()
            .and_then(|e| e.level.as_ref())
            .map(|l| l.place.dir.clone());
        let list = files::icon_sprites(&self.data, coll.as_deref());
        let Some(ed) = self.ed_mut() else { return };
        ed.sprites.list = list;
        ed.sprites.icon = true;
        ed.sprites.search.clear();
        ed.sprites.sel = 0;
        ed.sprites.page = 0;
        ed.screen = Screen::Sprites;
        self.menus.clear();
    }

    // ---------------------------------------------------- colour picker

    fn ed_colour_set(&mut self, w: ColourFor, x: u8, y: u8) {
        let objects = &self.objects;
        let Some(ed) = self.edh.ed.as_deref_mut() else {
            return;
        };
        let c = baba_formats::PalColour::new(x, y);
        let Some(l) = ed.level.as_mut() else { return };
        match w {
            ColourFor::Base | ColourFor::Active => {
                if let Some(k) = ed.editing
                    && let Some(mut d) = l.def(k).cloned()
                {
                    if w == ColourFor::Base {
                        d.colour = c;
                    } else {
                        d.active_colour = Some(c);
                    }
                    l.set_def(objects, k, d);
                }
                ed.screen = Screen::Palette;
                self.menus.push((em(EdMenu::EditObject), (0, 0)));
            }
            ColourFor::Icon(i) | ColourFor::IconClear(i) => {
                if let Some(icon) = l.info.levels.get_mut(i) {
                    if matches!(w, ColourFor::Icon(_)) {
                        icon.colour = Some(c);
                    } else {
                        icon.clear_colour = Some(c);
                    }
                }
                ed.screen = Screen::Main;
                self.menus.push((em(EdMenu::LevelSetup), (0, 0)));
            }
        }
        if let Some(ed) = self.ed_mut() {
            ed.unsaved = true;
        }
    }

    fn ed_colour_back(&mut self, w: ColourFor) {
        let Some(ed) = self.ed_mut() else { return };
        match w {
            ColourFor::Base | ColourFor::Active => {
                ed.screen = Screen::Palette;
                self.menus.push((em(EdMenu::EditObject), (0, 0)));
            }
            _ => {
                ed.screen = Screen::Main;
                self.menus.push((em(EdMenu::LevelSetup), (0, 0)));
            }
        }
    }

    fn ed_colour_event(&mut self, e: &Event, w: ColourFor) {
        let (lx, ly) = match *e {
            Event::Key {
                key: Key::Escape | Key::Back,
                down: true,
                ..
            } => return self.ed_colour_back(w),
            Event::MouseButton {
                button: 1,
                down: true,
                x,
                y,
            } => self.logical(x, y),
            Event::Touch {
                phase: TouchPhase::Down,
                x,
                y,
                ..
            } => {
                let (sw, sh) = self.edh.screen;
                self.logical(x * sw as f32, y * sh as f32)
            }
            Event::Key {
                key: Key::Enter,
                down: true,
                ..
            } => {
                let (x, y) = self.edh.colour_sel();
                return self.ed_colour_set(w, x, y);
            }
            Event::Key {
                key, down: true, ..
            } => {
                let d = match key {
                    Key::Right => 0,
                    Key::Up => 1,
                    Key::Left => 2,
                    Key::Down => 3,
                    _ => return,
                };
                return self.edh.colour_move(d);
            }
            _ => return,
        };
        let btns = view::colour_buttons(self.edh.gfx.palette.as_ref(), &self.texts);
        match btns.iter().find(|b| b.hit(lx, ly)).map(|b| b.act.clone()) {
            Some(BtnAct::Colour(x, y)) => self.ed_colour_set(w, x, y),
            Some(BtnAct::Return) => self.ed_colour_back(w),
            _ => {}
        }
    }

    fn ed_colour_pad(&mut self, pi: pad::PadIn, w: ColourFor) {
        use pad::PadIn;
        match pi {
            PadIn::Move(d) | PadIn::Rotate(d) => self.edh.colour_move(d),
            PadIn::Press("select") => {
                let (x, y) = self.edh.colour_sel();
                self.ed_colour_set(w, x, y);
            }
            PadIn::Press("closemenu" | "menu") => self.ed_colour_back(w),
            _ => {}
        }
    }

    // ---------------------------------------------------------- tutorial

    fn ed_start_lesson(&mut self, lesson: u8, intro: bool) {
        let place = self.user_dir_ed().map(|u| packs::user_levels_dir(&u));
        let objects = &self.objects;
        let Some(ed) = self.edh.ed.as_deref_mut() else {
            return;
        };
        // The lesson edits a scratch level; the open one is kept aside.
        if let Some(l) = ed.level.take() {
            self.edh.stash = Some((l, ed.screen.clone(), ed.unsaved));
        }
        let demo = EdLevel::new(
            "_tutorial_",
            model::Place {
                world: packs::LEVELS.into(),
                dir: place.unwrap_or_default(),
            },
            "Tutorial",
            "",
            objects,
            &ed.cat,
        );
        ed.open_level(demo);
        ed.tutorial = Some(tutorial::Run::new(lesson, intro));
        self.menus.clear();
        self.log.push(format!("tutorial {lesson}"));
        self.ed_tutorial_slide();
    }

    fn ed_tutorial_slide(&mut self) {
        let Some(run) = self.ed().and_then(|e| e.tutorial.clone()) else {
            return;
        };
        let (bg, demos) = tutorial::script(run.lesson, run.slide);
        self.menus.clear();
        if let Some(ed) = self.ed_mut() {
            match bg {
                Bg::Keep => {}
                Bg::Main => ed.screen = Screen::Main,
                Bg::Palette => ed.screen = Screen::Palette,
                Bg::ObjList => ed.screen = Screen::ObjList,
                Bg::Start | Bg::Levels | Bg::Settings | Bg::LevelMenu => ed.screen = Screen::Main,
            }
        }
        let shown = match bg {
            Bg::Start => Some(EdMenu::Start),
            Bg::Levels => Some(EdMenu::Levels { page: 0 }),
            Bg::Settings => Some(EdMenu::Settings),
            Bg::LevelMenu => Some(EdMenu::LevelMenu),
            _ => None,
        };
        self.edh.tut_menu = shown;
        for d in demos {
            self.ed_demo(*d);
        }
    }

    fn ed_demo(&mut self, d: Demo) {
        let objects = &self.objects;
        let Some(ed) = self.edh.ed.as_deref_mut() else {
            return;
        };
        let place = |ed: &mut Editor, name: &str, x: i32, y: i32, dir: u8| {
            let Some(l) = ed.level.as_mut() else { return };
            let i = match l.entry_by_name(name) {
                Some(i) => Some(i),
                None => ed
                    .cat
                    .by_name(name)
                    .map(|o| o.index as i32)
                    .and_then(|id| l.add_object(objects, &ed.cat, id, true).ok())
                    .and_then(|v| v.first().copied()),
            };
            if let Some(i) = i
                && let Some(t) = l.tile_of(i)
            {
                l.set(ed.layer, x, y, Some(model::Obj { tile: t, dir }));
            }
        };
        match d {
            Demo::Place(n, x, y, dir) => place(ed, n, x, y, dir),
            Demo::Erase(x, y) => {
                let z = ed.layer;
                if let Some(l) = ed.level.as_mut() {
                    l.set(z, x, y, None);
                }
            }
            Demo::Line(n, a, b) => {
                for (x, y) in crate::editor::tools::line(a, b) {
                    place(ed, n, x, y, 0);
                }
            }
            Demo::Rect(n, a, b, filled) => {
                for (x, y) in crate::editor::tools::rect(a, b, filled) {
                    place(ed, n, x, y, 0);
                }
            }
            Demo::Clear => {
                if let Some(l) = ed.level.as_mut() {
                    l.clear();
                }
            }
            Demo::Cursor(x, y) => ed.cursor = (x, y),
            Demo::Pick(n) => ed.quick_pick(n),
            Demo::Layer(z) => ed.layer = z,
        }
    }

    fn ed_tutorial_next(&mut self, skip: bool) {
        let Some(ed) = self.ed_mut() else { return };
        let Some(run) = ed.tutorial.as_mut() else {
            return;
        };
        if skip || run.last() {
            let intro = run.intro;
            ed.tutorial = None;
            ed.level = None;
            if let Some((l, screen, unsaved)) = self.edh.stash.take() {
                let ed = self.ed_mut().unwrap();
                ed.open_level(l);
                ed.screen = screen;
                ed.unsaved = unsaved;
            } else if let Some(ed) = self.ed_mut() {
                ed.screen = Screen::Menus;
            }
            self.edh.tut_menu = None;
            self.log.push("tutorial end".into());
            self.menus.clear();
            if self.ed().is_some_and(|e| e.level.is_none()) {
                self.menus.push((em(EdMenu::Start), (0, 0)));
                // The intro too ends in the tutorials list (captured).
                let _ = intro;
                self.menus.push((em(EdMenu::Help), (0, 0)));
                self.menus.push((em(EdMenu::Tutorials), (0, 0)));
            }
            return;
        }
        run.slide += 1;
        self.ed_tutorial_slide();
    }

    fn ed_tutorial_event(&mut self, e: &Event) {
        let (lx, ly) = match *e {
            Event::Key {
                key, down: true, ..
            } => {
                match key {
                    Key::Enter | Key::Space | Key::Right => self.ed_tutorial_next(false),
                    Key::Escape | Key::Back => self.ed_tutorial_next(true),
                    _ => {}
                }
                return;
            }
            Event::Button {
                button, down: true, ..
            } => {
                use baba_platform::Button as B;
                match button {
                    B::South | B::DPadRight => self.ed_tutorial_next(false),
                    B::East | B::Start => self.ed_tutorial_next(true),
                    _ => {}
                }
                return;
            }
            Event::MouseButton {
                button: 1,
                down: true,
                x,
                y,
            } => self.logical(x, y),
            Event::Touch {
                phase: TouchPhase::Down,
                x,
                y,
                ..
            } => {
                let (w, h) = self.edh.screen;
                self.logical(x * w as f32, y * h as f32)
            }
            _ => return,
        };
        let last = self
            .ed()
            .and_then(|e| e.tutorial.as_ref())
            .is_some_and(|r| r.last());
        match view::tutorial_buttons(&self.texts, last)
            .into_iter()
            .find(|b| b.hit(lx, ly))
            .map(|b| b.act)
        {
            Some(BtnAct::TutContinue) => self.ed_tutorial_next(false),
            Some(BtnAct::TutSkip) => self.ed_tutorial_next(true),
            _ => {}
        }
    }

    // ------------------------------------------------------- level files

    /// The level of the level list at menu row `row`.
    fn listed_level(&self, page: usize, row: usize) -> Option<String> {
        let c = self.ed_collection()?;
        let ed = self.ed()?;
        let list = files::sorted(
            files::levels(&c.dir),
            ed.sort_name,
            ed.sort_type,
            &ed.level_search,
        );
        let first = if c.is_pack() { 4 } else { 2 };
        let i = row.checked_sub(first)?;
        if i >= LEVELS_PER_PAGE {
            return None;
        }
        list.get(page * LEVELS_PER_PAGE + i).map(|l| l.stem.clone())
    }

    fn ed_collection(&self) -> Option<Collection> {
        self.ed()?.coll.clone()
    }

    /// Opens a level of the current collection in the main view.
    fn ed_open_stem(&mut self, stem: &str) {
        let Some(c) = self.ed_collection() else {
            return;
        };
        let objects = &self.objects;
        let Some(ed) = self.edh.ed.as_deref_mut() else {
            return;
        };
        match EdLevel::load(crate::editor::place_of(&c), stem, objects, &ed.cat) {
            Ok(l) => {
                ed.open_level(l);
                self.menus.clear();
                self.log.push(format!("editor open {}/{stem}", c.world));
                self.ed_music();
            }
            Err(e) => {
                log::error!("editor: {e}");
                self.ed_notice(format!("{stem}: {e}"));
            }
        }
    }

    fn ed_notice(&mut self, s: String) {
        self.menus.push((em(EdMenu::Notice(s)), (0, 0)));
    }

    /// Saves the open level: `.l`, `.ld` and the preview (`§8.6`, `§10`).
    fn ed_save(&mut self) {
        let Some(l) = self.edh.ed.as_deref_mut().and_then(|e| e.level.as_mut()) else {
            return;
        };
        if l.stem.starts_with('_') {
            return; // the tutorial's scratch level
        }
        let r = l.save();
        let (dir, stem) = (l.place.dir.clone(), l.stem.clone());
        match r {
            Ok(()) => {
                self.edh.gfx.sync(l, &self.data);
                let scene = self.edh.gfx.scene(&mut self.gfx.atlas, l);
                let png = view::preview(&scene, &self.gfx.atlas);
                if let Err(e) = png.save_png(dir.join(format!("{stem}.png"))) {
                    log::warn!("preview: {e}");
                }
                if let Some(ed) = self.ed_mut() {
                    ed.unsaved = false;
                    let t = self.texts.get("editor_saved");
                    if let Some(ed) = self.ed_mut() {
                        ed.note(t);
                    }
                }
                self.log.push(format!("editor saved {stem}"));
                self.audio.sound("confirm");
            }
            Err(e) => {
                log::error!("saving {stem}: {e}");
                self.ed_notice(format!("{e}"));
            }
        }
    }

    /// Leaves the level for the list (`back = false`) or the start screen,
    /// asking first with unsaved changes (`§8.3`).
    fn ed_leave(&mut self, full: bool) {
        if self.ed().is_some_and(|e| e.unsaved) {
            self.ed_confirm(if full {
                Confirm::LeaveMain
            } else {
                Confirm::LeaveList
            });
            return;
        }
        self.ed_leave_now(full);
    }

    fn ed_leave_now(&mut self, full: bool) {
        if let Some(ed) = self.ed_mut() {
            ed.level = None;
            ed.screen = Screen::Menus;
            ed.selection = None;
            ed.undo.clear();
            ed.unsaved = false;
        }
        self.menus = vec![(em(EdMenu::Start), (0, 0))];
        if !full {
            if self.ed_collection().is_some_and(|c| c.is_pack()) {
                self.menus.push((em(EdMenu::Packs { page: 0 }), (0, 0)));
            }
            self.menus.push((em(EdMenu::Levels { page: 0 }), (0, 0)));
        }
        self.audio.music("editorsong");
    }

    fn ed_confirm(&mut self, c: Confirm) {
        self.menus.push((em(EdMenu::Confirm(c)), (0, 0)));
    }

    fn ed_prompt(&mut self, kind: Prompt, text: String) {
        self.menus.push((
            em(EdMenu::Prompt {
                kind,
                text,
                upper: false,
            }),
            (1, 0),
        ));
    }

    /// Test play (`§12`): the current state is written to a temporary
    /// level next to the real one (same folder, so pack sprites, palettes
    /// and music resolve), played with `Play::single` (the normal turn
    /// path, results not recorded) and removed at once.
    fn ed_test(&mut self, upload: bool) {
        let Some(l) = self.edh.ed.as_deref_mut().and_then(|e| e.level.as_mut()) else {
            return;
        };
        let dir = l.place.dir.clone();
        let world = l.place.world.clone();
        let (lb, ldb) = l.file_bytes();
        let stem = files::TEST_STEM;
        let write = std::fs::create_dir_all(&dir)
            .and_then(|_| std::fs::write(dir.join(format!("{stem}.l")), &lb))
            .and_then(|_| std::fs::write(dir.join(format!("{stem}.ld")), &ldb));
        if let Err(e) = write {
            self.ed_notice(format!("{e}"));
            return;
        }
        let Some(w) = WorldInfo::open_dir(&dir, &world) else {
            files::clean_test_files(&dir);
            return;
        };
        let slot = self.profile.last_slot();
        let mut env = Env {
            data_dir: &self.data,
            objects: &self.objects,
            atlas: &mut self.gfx.atlas,
        };
        let r = Play::single(
            &mut env,
            w,
            slot,
            stem,
            SingleLevel {
                code: None,
                temp: true,
            },
        );
        files::clean_test_files(&dir);
        match r {
            Ok(mut p) => {
                // No title card for a test.
                p.title = None;
                self.log.append(&mut p.log);
                self.log.push(format!(
                    "editor test{}",
                    if upload { " upload" } else { "" }
                ));
                if let Some(ed) = self.ed_mut() {
                    ed.testing = Some(TestRun { upload, won: false });
                    ed.stroke = None;
                }
                self.menus.clear();
                self.mode = Mode::Play(Box::new(p));
                self.level_music();
            }
            Err(e) => {
                log::error!("test: {e}");
                self.ed_notice(e);
            }
        }
    }

    #[cfg(feature = "upload")]
    fn ed_upload_after_win(&mut self) {
        if let Some(l) = self.ed().and_then(|e| e.level.as_ref()) {
            let (dir, stem) = (l.place.dir.clone(), l.stem.clone());
            self.start_upload(&dir, &stem);
        }
    }

    #[cfg(not(feature = "upload"))]
    fn ed_upload_after_win(&mut self) {}

    // ------------------------------------------------------------ menus

    fn page_row(m: &mut Menu, page: usize, pages: usize, mk: impl Fn(usize) -> Act) {
        if pages > 1 {
            m.gap(4.0);
            m.row(vec![
                Item::new("<<", mk(page.saturating_sub(5))).enabled(page > 0),
                Item::new("<", mk(page.saturating_sub(1))).enabled(page > 0),
                Item::new(format!("{}/{}", page + 1, pages), Act::None),
                Item::new(">", mk((page + 1).min(pages - 1))).enabled(page + 1 < pages),
                Item::new(">>", mk((page + 5).min(pages - 1))).enabled(page + 1 < pages),
            ]);
        }
    }

    /// An editor menu in the original's button art (`editor.md` §19,
    /// `presentation.md` §15.29): buttons in the level palette's editor
    /// colour (3,2), Return / Close in (3,1), information lines as text.
    pub(super) fn build_ed(&self, id: &EdMenu) -> Menu {
        let mut m = self.build_ed_items(id);
        let c = |x: u8, y: u8| match &self.edh.gfx.palette {
            Some(p) => {
                let [r, g, b] = p.rgb(baba_formats::PalColour::new(x, y));
                [r, g, b, 255]
            }
            None => self.gfx.pal(x as u32, y as u32),
        };
        m.border = Some(c(3, 2));
        m.escape_border = Some(c(3, 1));
        m.plain_info = true;
        m
    }

    fn build_ed_items(&self, id: &EdMenu) -> Menu {
        let t = &self.texts;
        let mut m = Menu::new(Style::List);
        m.top = 30.0;
        m.min_width = 360.0;
        let ed = self.ed();
        let lvl = ed.and_then(|e| e.level.as_ref());
        let ext = self.ed_extended();
        match id {
            EdMenu::Start => {
                // The original's geometry (captured): title at 64, five
                // 382×44 buttons from 121, 48 apart, Return (22 high) at
                // 373.
                m.top = 64.0;
                m.header_gap = 25.0;
                m.min_width = 382.0;
                m.escape = ea(EdAct::Quit);
                m.header.push(t.get("editor_start_title"));
                m.item(Item::new(t.get("editor_start_level"), ea(EdAct::EditLevels)).tall());
                m.item(
                    Item::new(t.get("editor_start_world"), ea(EdAct::EditPacks))
                        .enabled(ext)
                        .tall(),
                );
                m.item(
                    Item::new(
                        t.get("customlevels_play_get"),
                        Act::Open(MenuId::Ext(super::extras::ExtMenu::GetLevels)),
                    )
                    .tall(),
                );
                m.item(
                    Item::new(
                        t.get("customlevels_play_getlist"),
                        Act::Open(MenuId::Ext(super::extras::ExtMenu::History)),
                    )
                    .tall(),
                );
                m.item(Item::new(t.get("editor_start_settings"), open(EdMenu::Help)).tall());
                m.gap(12.0);
                m.item(Item::new(t.get("return"), ea(EdAct::Quit)));
            }
            EdMenu::Help => {
                m.top = 20.0;
                m.item(Item::new(t.get("return"), Act::Close));
                m.header.push(match &self.pad {
                    Some(n) => format!(
                        "{} {}",
                        t.get("controls_gamepadname"),
                        n.chars().take(25).collect::<String>().to_lowercase()
                    ),
                    None => t.get("controls_noconnectedgamepad"),
                });
                m.item(Item::new(
                    t.get("editor_settings_defaultpad"),
                    ea(EdAct::DefaultPad),
                ));
                m.item(Item::new(
                    t.get("editor_settings_controls"),
                    open(EdMenu::PadBinds { page: 0 }),
                ));
                m.item(Item::new(
                    t.get("editor_settings_help"),
                    open(EdMenu::Tutorials),
                ));
                for (key, label) in [
                    ("nohints", "editor_settings_tips"),
                    ("noslide", "editor_settings_slide"),
                    ("nomusic", "editor_settings_music"),
                    ("advanced", "editor_settings_advanced"),
                ] {
                    let on = if key == "advanced" {
                        self.ed_advanced()
                    } else {
                        self.setting(key)
                    };
                    m.item(Item::new(t.get(label), ea(EdAct::ToggleSetting(key))).on(on));
                }
                m.item(
                    Item::new(
                        t.get("editor_settings_mod"),
                        ea(EdAct::ToggleSetting("mode")),
                    )
                    .on(ext),
                );
                m.item(
                    Item::new(t.get("author"), ea(EdAct::Prompt(Prompt::EditorAuthor)))
                        .value(self.ed_author()),
                );
            }
            EdMenu::Tutorials => {
                m.top = 20.0;
                m.item(Item::new(t.get("return"), Act::Close));
                m.item(Item::new(
                    t.get("editor_settings_hotkeys"),
                    open(EdMenu::Hotkeys { page: 0 }),
                ));
                m.gap(6.0);
                for i in 1..=10u8 {
                    m.item(Item::new(
                        t.get(&format!("editor_tutorial{i}")),
                        ea(EdAct::Lesson(i)),
                    ));
                }
            }
            EdMenu::Hotkeys { page } => {
                m.top = 8.0;
                m.min_width = 200.0;
                let keys = hotkey_keys();
                let per = 15;
                let pages = keys.len().div_ceil(per).max(1);
                let page = (*page).min(pages - 1);
                for k in keys.iter().skip(page * per).take(per) {
                    m.footer.push(t.get(k));
                }
                m.item(Item::new(t.get("return"), Act::Close));
                Self::page_row(&mut m, page, pages, |p| open(EdMenu::Hotkeys { page: p }));
            }
            EdMenu::PadBinds { page } => {
                m.top = 8.0;
                m.min_width = 300.0;
                m.item(Item::new(t.get("return"), Act::Close));
                let per = 13;
                let acts: Vec<&str> = pad::EDITOR_ACTIONS
                    .iter()
                    .copied()
                    .filter(|a| *a != "move" && *a != "rotate")
                    .collect();
                let pages = acts.len().div_ceil(per).max(1);
                let page = (*page).min(pages - 1);
                for a in acts.iter().skip(page * per).take(per) {
                    let label = t
                        .opt(&format!("buttons_editor_{a}"))
                        .unwrap_or_else(|| a.to_string());
                    let v = pad::binding(&self.profile.settings, "gamepad_editor", a);
                    let alt = pad::ALT_PAIRS.iter().any(|(x, _)| x == a);
                    let shown = if alt {
                        format!("alt + {}", pad::value_name(&v))
                    } else {
                        pad::value_name(&v)
                    };
                    m.item(Item::new(label, ea(EdAct::BindPad(a_static(a)))).value(shown));
                }
                Self::page_row(&mut m, page, pages, |p| open(EdMenu::PadBinds { page: p }));
            }
            EdMenu::Levels { page } => {
                m.top = 4.0;
                m.min_width = 420.0;
                let Some(c) = self.ed_collection() else {
                    m.item(Item::new(t.get("return"), Act::Close));
                    return m;
                };
                let e = ed.unwrap();
                let pack = c.is_pack();
                let wd = files::world_store(&c.dir);
                m.header.push(if pack {
                    let name = wd.get("general", "name").unwrap_or("").to_string();
                    let author = wd.get("general", "author").unwrap_or("").to_string();
                    if author.is_empty() {
                        name
                    } else {
                        format!("{name} {} {author}", t.get("by"))
                    }
                } else {
                    t.get("editor_levellist_levels")
                });
                m.item(Item::new(
                    t.get(if pack {
                        "editor_levellist_returnworld"
                    } else {
                        "editor_levellist_return"
                    }),
                    Act::Close,
                ));
                if pack {
                    m.row(vec![
                        Item::new(
                            t.get("editor_levellist_setauthor"),
                            ea(EdAct::Prompt(Prompt::PackAuthor)),
                        ),
                        Item::new(
                            t.get("editor_levellist_setname"),
                            ea(EdAct::Prompt(Prompt::PackName)),
                        ),
                    ]);
                    m.row(vec![
                        Item::new(
                            t.get("editor_levellist_setstart"),
                            ea(EdAct::SetListMode(ListMode::SetFirst)),
                        )
                        .on(e.list_mode == ListMode::SetFirst),
                        Item::new(
                            t.get("editor_levellist_setmap"),
                            ea(EdAct::SetListMode(ListMode::SetMap)),
                        )
                        .on(e.list_mode == ListMode::SetMap),
                    ]);
                }
                let mut row = vec![
                    Item::new(t.get("editor_levellist_sort"), ea(EdAct::SortName)).on(e.sort_name),
                    Item::new(t.get("editor_levellist_new"), ea(EdAct::NewLevel)),
                ];
                if pack {
                    row.push(
                        Item::new(t.get("editor_levellist_sorttypes"), ea(EdAct::SortType))
                            .on(e.sort_type),
                    );
                }
                m.row(row);
                let list = files::sorted(
                    files::levels(&c.dir),
                    e.sort_name,
                    e.sort_type,
                    &e.level_search,
                );
                let pages = list.len().div_ceil(LEVELS_PER_PAGE).max(1);
                let page = (*page).min(pages - 1);
                let first = wd.get("general", "firstlevel").unwrap_or("").to_string();
                let start = wd.get("general", "start").unwrap_or("").to_string();
                m.gap(4.0);
                for l in list
                    .iter()
                    .skip(page * LEVELS_PER_PAGE)
                    .take(LEVELS_PER_PAGE)
                {
                    let mut name = if l.name.is_empty() {
                        l.stem.clone()
                    } else {
                        l.name.clone()
                    };
                    if l.is_map {
                        name.push_str(" (map)");
                    }
                    let mut tag = l.stem.clone();
                    if pack && l.stem == first {
                        tag.push_str(" *");
                    }
                    if pack && l.stem == start {
                        tag.push_str(" @");
                    }
                    m.item(Item::new(name, ea(EdAct::OpenLevel(l.stem.clone()))).value(tag));
                }
                Self::page_row(&mut m, page, pages, |p| ea(EdAct::LevelPage(p)));
                m.gap(4.0);
                m.row(vec![
                    Item::new(
                        t.get("editor_levellist_search"),
                        ea(EdAct::Prompt(Prompt::LevelSearch)),
                    ),
                    Item::new(
                        t.get("editor_levellist_removesearch"),
                        ea(EdAct::ClearSearch),
                    )
                    .enabled(!e.level_search.is_empty()),
                ]);
                if !e.level_search.is_empty() {
                    m.footer.push(format!(
                        "{} {}",
                        t.get("editor_objectlist_result"),
                        e.level_search
                    ));
                }
            }
            EdMenu::Packs { page } => {
                m.top = 20.0;
                m.header.push(t.get("editor_editworld"));
                m.item(Item::new(t.get("editor_levellist_return"), Act::Close));
                m.item(Item::new(t.get("editor_newworld"), ea(EdAct::NewPack)));
                let list = self
                    .user_dir_ed()
                    .map(|u| files::packs(&u, &self.data, &self.ed_author()))
                    .unwrap_or_default();
                if list.is_empty() {
                    m.footer.push(t.get("editor_editworld_none"));
                }
                let per = 10;
                let pages = list.len().div_ceil(per).max(1);
                let page = (*page).min(pages - 1);
                m.gap(4.0);
                for p in list.iter().skip(page * per).take(per) {
                    let name = if p.name.is_empty() {
                        t.get("world_unknown")
                    } else {
                        p.name.clone()
                    };
                    let value = if p.install {
                        format!("{} (copy)", p.author)
                    } else {
                        p.author.clone()
                    };
                    m.item(Item::new(name, ea(EdAct::OpenPack(p.id.clone()))).value(value));
                }
                Self::page_row(&mut m, page, pages, |p| ea(EdAct::PackPage(p)));
            }
            EdMenu::LevelMenu => {
                m.top = 108.0;
                m.min_width = 380.0;
                m.item(Item::new(t.get("editor_menu_close"), Act::Close));
                m.item(Item::new(t.get("editor_menu_test"), ea(EdAct::Test)));
                let up = Item::new(t.get("editor_menu_upload"), ea(EdAct::Upload));
                let saved = lvl.is_some_and(|l| l.on_disk);
                m.item(if super::extras::UPLOAD {
                    up.enabled(saved)
                } else {
                    up.enabled(false).value("(not in this build)")
                });
                m.gap(12.0);
                m.item(Item::new(t.get("editor_menu_themes"), open(EdMenu::Themes)));
                m.gap(12.0);
                m.item(Item::new(t.get("editor_menu_return"), ea(EdAct::ToList)));
                m.item(Item::new(
                    t.get("editor_menu_returnfull"),
                    ea(EdAct::ToStart),
                ));
                m.gap(12.0);
                m.item(Item::new(t.get("editor_menu_delete"), ea(EdAct::Delete)).enabled(saved));
                m.item(Item::new(t.get("editor_menu_copy"), ea(EdAct::Copy)).enabled(saved));
            }
            EdMenu::QuickMenu => {
                m.top = 140.0;
                m.item(Item::new(t.get("editor_menu_close"), Act::Close));
                m.item(Item::new(t.get("editor_savelevel"), ea(EdAct::Save)));
                m.item(Item::new(t.get("editor_test"), ea(EdAct::Test)));
                m.item(Item::new(
                    t.get("editor_settingsmenu"),
                    open(EdMenu::Settings),
                ));
                m.item(Item::new(
                    t.get("editor_editormenu"),
                    open(EdMenu::LevelMenu),
                ));
            }
            EdMenu::Settings => {
                m.top = 4.0;
                m.min_width = 480.0;
                let Some(l) = lvl else {
                    m.item(Item::new(t.get("return"), Act::Close));
                    return m;
                };
                let g = &l.info.general;
                let none = t.get("editor_levelmenu_subtitle_none");
                let or = |s: &str, d: &str| {
                    if s.is_empty() {
                        d.to_string()
                    } else {
                        s.to_string()
                    }
                };
                m.top = 40.0;
                m.min_width = 300.0;
                m.item(Item::new(t.get("editor_menu_close"), Act::Close));
                let row = |m: &mut Menu, label: &str, value: String, act: Act, button: &str| {
                    m.row(vec![
                        Item::new(format!("$2,4{label}$0,3 {value}"), act.clone()),
                        Item::new(t.get(button), act),
                    ]);
                };
                row(
                    &mut m,
                    &t.get("editor_levelmenu_name"),
                    g.name.clone(),
                    ea(EdAct::Prompt(Prompt::RenameLevel)),
                    "editor_levelmenu_rename",
                );
                row(
                    &mut m,
                    &t.get("editor_levelmenu_author"),
                    or(&g.author, &t.get("noauthor")),
                    ea(EdAct::Prompt(Prompt::Author)),
                    "editor_levelmenu_changeauthor",
                );
                row(
                    &mut m,
                    &t.get("editor_levelmenu_subtitle"),
                    or(&g.subtitle, &none),
                    ea(EdAct::Prompt(Prompt::Subtitle)),
                    "editor_levelmenu_changesubtitle",
                );
                let music = t
                    .opt(&format!("music_{}", g.music))
                    .unwrap_or_else(|| g.music.clone());
                m.gap(6.0);
                row(
                    &mut m,
                    &t.get("editor_levelmenu_music"),
                    music,
                    open(EdMenu::Music { page: 0 }),
                    "editor_levelmenu_changemusic",
                );
                let part = if g.particles.is_empty() {
                    t.get("particles_none")
                } else {
                    t.opt(&format!("particles_{}", g.particles))
                        .unwrap_or_else(|| g.particles.clone())
                };
                row(
                    &mut m,
                    &t.get("editor_levelmenu_particles"),
                    part,
                    open(EdMenu::Particles),
                    "editor_levelmenu_changeparticles",
                );
                let stem = g.palette.trim_end_matches(".png");
                let pal = t
                    .opt(&format!("palettes_{stem}"))
                    .unwrap_or_else(|| stem.to_string());
                row(
                    &mut m,
                    &t.get("editor_levelmenu_palette"),
                    pal,
                    open(EdMenu::Palettes { page: 0 }),
                    "editor_levelmenu_changepalette",
                );
                row(
                    &mut m,
                    &t.get("editor_levelmenu_levelsize"),
                    format!(
                        "{} {}, {} {}",
                        t.get("editor_levelmenu_levelwidth"),
                        l.width() - 2,
                        t.get("editor_levelmenu_levelheight"),
                        l.height() - 2
                    ),
                    open(EdMenu::Size {
                        w: l.width() - 2,
                        h: l.height() - 2,
                    }),
                    "editor_levelmenu_changelevelsize",
                );
                m.gap(10.0);
                for (k, label) in [
                    ("disableparticles", "editor_levelmenu_disableparticles"),
                    ("disableruleeffect", "editor_levelmenu_disableruleeffect"),
                    ("disableshake", "editor_levelmenu_disableshake"),
                ] {
                    m.item(Item::new(t.get(label), ea(EdAct::ToggleGen(k))).on(l.gen_int(k) != 0));
                }
                let delay = l
                    .info
                    .ini
                    .get_int("general", "rhythm")
                    .filter(|v| *v > 0)
                    .unwrap_or(10);
                m.row(vec![
                    Item::new(
                        format!("{} {delay}", t.get("editor_levelmenu_autodelay")),
                        Act::None,
                    ),
                    Item::new("-", ea(EdAct::AutoDelay(-1))),
                    Item::new("+", ea(EdAct::AutoDelay(1))),
                ]);
                if ext {
                    m.gap(6.0);
                    m.item(Item::new(
                        t.get("editor_levelmenu_mapsetup"),
                        open(EdMenu::MapSetup),
                    ));
                }
            }
            EdMenu::Size { w, h } => {
                m.top = 30.0;
                m.item(Item::new(t.get("return"), Act::Close));
                m.header.push(format!(
                    "{} {w}   {} {h}",
                    t.get("editor_levelsize_width"),
                    t.get("editor_levelsize_height")
                ));
                m.row(vec![
                    Item::new(
                        format!("{} --", t.get("editor_levelmenu_levelwidth")),
                        ea(EdAct::SizeStep(-5, 0)),
                    ),
                    Item::new("-", ea(EdAct::SizeStep(-1, 0))),
                    Item::new("+", ea(EdAct::SizeStep(1, 0))),
                    Item::new("++", ea(EdAct::SizeStep(5, 0))),
                ]);
                m.row(vec![
                    Item::new(
                        format!("{} --", t.get("editor_levelmenu_levelheight")),
                        ea(EdAct::SizeStep(0, -5)),
                    ),
                    Item::new("-", ea(EdAct::SizeStep(0, -1))),
                    Item::new("+", ea(EdAct::SizeStep(0, 1))),
                    Item::new("++", ea(EdAct::SizeStep(0, 5))),
                ]);
                m.item(Item::new(
                    t.get("editor_levelsize_apply"),
                    ea(EdAct::SizeApply),
                ));
                m.gap(8.0);
                m.footer.push(t.get("editor_levelsize_quick"));
                let presets = [
                    (12, 6),
                    (15, 8),
                    (14, 14),
                    (24, 14),
                    (28, 16),
                    (18, 18),
                    (24, 18),
                    (33, 18),
                ];
                for half in presets.chunks(4) {
                    m.row(
                        half.iter()
                            .map(|(pw, ph)| {
                                Item::new(format!("{pw}x{ph}"), ea(EdAct::SizeSet(*pw, *ph)))
                                    .on((*pw, *ph) == (*w, *h))
                            })
                            .collect(),
                    );
                }
            }
            EdMenu::Music { page } => {
                m.top = 8.0;
                m.header.push(t.get("editor_music_select"));
                m.item(Item::new(t.get("return"), Act::Close));
                let coll = lvl.map(|l| l.place.dir.clone());
                let list = files::music(&self.data, coll.as_deref());
                let pages = list.len().div_ceil(LIST_PER_PAGE).max(1);
                let page = (*page).min(pages - 1);
                let cur = lvl
                    .map(|l| l.info.general.music.clone())
                    .unwrap_or_default();
                for (name, local) in list.iter().skip(page * LIST_PER_PAGE).take(LIST_PER_PAGE) {
                    let label = t
                        .opt(&format!("music_{name}"))
                        .unwrap_or_else(|| name.clone());
                    m.item(
                        Item::new(label, ea(EdAct::SetMusic(name.clone(), *local)))
                            .on(*name == cur)
                            .value(if *local { "(pack)" } else { "" }),
                    );
                }
                Self::page_row(&mut m, page, pages, |p| ea(EdAct::MusicPage(p)));
            }
            EdMenu::Particles => {
                m.top = 8.0;
                m.header.push(t.get("editor_particles_select"));
                m.item(Item::new(t.get("return"), Act::Close));
                let cur = lvl
                    .map(|l| l.info.general.particles.clone())
                    .unwrap_or_default();
                m.item(
                    Item::new(
                        t.get("particles_none"),
                        ea(EdAct::SetParticles(String::new())),
                    )
                    .on(cur.is_empty()),
                );
                for p in files::PARTICLES.chunks(2) {
                    m.row(
                        p.iter()
                            .map(|n| {
                                Item::new(
                                    t.opt(&format!("particles_{n}"))
                                        .unwrap_or_else(|| n.to_string()),
                                    ea(EdAct::SetParticles(n.to_string())),
                                )
                                .on(cur == *n)
                            })
                            .collect(),
                    );
                }
            }
            EdMenu::Palettes { page } => {
                m.top = 8.0;
                m.header.push(t.get("editor_palette_select"));
                m.item(Item::new(t.get("return"), Act::Close));
                let coll = lvl.map(|l| l.place.dir.clone());
                let list = files::palettes(&self.data, coll.as_deref());
                let pages = list.len().div_ceil(LIST_PER_PAGE).max(1);
                let page = (*page).min(pages - 1);
                let cur = lvl
                    .map(|l| l.info.general.palette.clone())
                    .unwrap_or_default();
                for (file, root) in list.iter().skip(page * LIST_PER_PAGE).take(LIST_PER_PAGE) {
                    let stem = file.trim_end_matches(".png");
                    let label = t
                        .opt(&format!("palettes_{stem}"))
                        .unwrap_or_else(|| stem.to_string());
                    m.item(
                        Item::new(label, ea(EdAct::SetPalette(file.clone(), *root)))
                            .on(*file == cur)
                            .value(if *root { "" } else { "(pack)" }),
                    );
                }
                Self::page_row(&mut m, page, pages, |p| ea(EdAct::PalettePage(p)));
            }
            EdMenu::Themes => {
                m.top = 130.0;
                m.footer.push(t.get("editor_theme_warning"));
                m.item(Item::new(
                    t.get("editor_theme_load"),
                    ea(EdAct::ThemePage(false, 0)),
                ));
                m.item(Item::new(
                    t.get("editor_theme_save"),
                    ea(EdAct::Prompt(Prompt::ThemeName)),
                ));
                m.item(Item::new(
                    t.get("editor_theme_delete"),
                    ea(EdAct::ThemePage(true, 0)),
                ));
                m.item(Item::new(t.get("return"), Act::Close));
            }
            EdMenu::ThemeList { .. } | EdMenu::NewTheme { .. } => {
                let (new, delete, page) = match id {
                    EdMenu::NewTheme { page } => (true, false, *page),
                    EdMenu::ThemeList { delete, page } => (false, *delete, *page),
                    _ => (false, false, 0),
                };
                m.top = 8.0;
                let coll = self.ed_collection().map(|c| c.dir);
                let list: Vec<files::ThemeItem> = files::themes(&self.data, coll.as_deref())
                    .into_iter()
                    .filter(|th| !delete || th.own)
                    .collect();
                m.header.push(t.get(if new {
                    "editor_theme_use"
                } else if delete {
                    "editor_theme_themedelete"
                } else {
                    "editor_theme_themeload"
                }));
                if new {
                    m.item(Item::new(
                        t.get("editor_theme_done"),
                        ea(EdAct::NewWithTheme(None)),
                    ));
                } else {
                    m.item(Item::new(t.get("return"), Act::Close));
                }
                if list.is_empty() {
                    m.footer.push(t.get("editor_theme_none"));
                }
                let pages = list.len().div_ceil(LIST_PER_PAGE).max(1);
                let page = page.min(pages - 1);
                for th in list.iter().skip(page * LIST_PER_PAGE).take(LIST_PER_PAGE) {
                    let key = format!("themes_{}", th.name.to_lowercase().replace(' ', ""));
                    let label = t.opt(&key).unwrap_or_else(|| th.name.clone());
                    let act = if new {
                        ea(EdAct::NewWithTheme(Some(th.path.clone())))
                    } else {
                        ea(EdAct::PickTheme(th.path.clone(), delete))
                    };
                    m.item(Item::new(label, act).value(if th.own { "(pack)" } else { "" }));
                }
                Self::page_row(&mut m, page, pages, |p| {
                    if new {
                        open(EdMenu::NewTheme { page: p })
                    } else {
                        ea(EdAct::ThemePage(delete, p))
                    }
                });
            }
            EdMenu::Confirm(c) => {
                m.top = 150.0;
                m.escape = Act::Close;
                let (head, hint) = match c {
                    Confirm::LeaveList | Confirm::LeaveMain | Confirm::OpenOther(_) => {
                        ("editor_unsaved_confirm", None)
                    }
                    Confirm::Delete | Confirm::DeleteListed(_) => ("editor_delete_confirm", None),
                    Confirm::Copy => ("editor_copy_confirm", None),
                    Confirm::LoadTheme(_) => (
                        "editor_theme_load_confirm",
                        Some("editor_theme_load_confirm_hint"),
                    ),
                    Confirm::EmptyLevel => ("editor_theme_load_confirm_hint", None),
                    Confirm::OverwriteTheme(_) => ("editor_theme_save_confirm", None),
                    Confirm::DeleteTheme(_) => ("editor_theme_delete_confirm", None),
                };
                m.header.push(t.get(head));
                if let Some(h) = hint {
                    m.header.push(t.get(h));
                }
                if let Confirm::Copy = c
                    && let Some(l) = lvl
                {
                    m.header.push(format!(
                        "{} {}",
                        t.get("editor_copy_confirm_name"),
                        files::copy_name(&l.info.general.name)
                    ));
                }
                m.row(vec![
                    Item::new(t.get("no"), Act::Close),
                    Item::new(t.get("yes"), ea(EdAct::Yes(c.clone()))),
                ]);
            }
            EdMenu::Prompt { kind, text, upper } => {
                m.top = 70.0;
                m.min_width = 290.0;
                let (head, max, _) = kind.spec();
                m.header.push(t.get(head));
                m.header
                    .push(format!("{text}_  ({}/{max})", text.chars().count()));
                for row in KEYBOARD {
                    m.row(
                        row.chars()
                            .map(|c| {
                                let c = if *upper { c.to_ascii_uppercase() } else { c };
                                Item::new(c.to_string(), ea(EdAct::TypeChar(c)))
                            })
                            .collect(),
                    );
                }
                m.row(vec![
                    Item::new("Shift", ea(EdAct::TypeShift)).on(*upper),
                    Item::new("Space", ea(EdAct::TypeChar(' '))),
                    Item::new("Del", ea(EdAct::TypeBack)),
                    Item::new("OK", ea(EdAct::TypeDone)),
                ]);
                m.footer.push("Enter: OK    Escape: cancel".into());
            }
            EdMenu::EditObject => {
                m.top = 4.0;
                m.min_width = 300.0;
                let Some((l, i)) = lvl.zip(ed.and_then(|e| e.editing)) else {
                    m.item(Item::new(t.get("return"), Act::Close));
                    return m;
                };
                let Some(d) = l.def(i) else {
                    m.item(Item::new(t.get("return"), Act::Close));
                    return m;
                };
                m.header
                    .push(format!("{}  ({})", d.name, l.palette[i].slot));
                m.item(Item::new(t.get("return"), Act::Close));
                m.row(vec![
                    Item::new(t.get("editor_object_sprite"), ea(EdAct::Sprite)),
                    Item::new(
                        t.get("editor_object_name"),
                        ea(EdAct::Prompt(Prompt::ObjName)),
                    )
                    .enabled(ext),
                    Item::new(
                        t.get("editor_object_type"),
                        ea(EdAct::Prompt(Prompt::ObjType)),
                    )
                    .enabled(ext),
                ]);
                if d.is_text() {
                    m.row(vec![
                        Item::new(
                            t.get("editor_object_colour_base"),
                            ea(EdAct::Colour(ColourFor::Base)),
                        ),
                        Item::new(
                            t.get("editor_object_colour_active"),
                            ea(EdAct::Colour(ColourFor::Active)),
                        ),
                    ]);
                } else {
                    m.item(Item::new(
                        t.get("editor_object_colour_onlybase"),
                        ea(EdAct::Colour(ColourFor::Base)),
                    ));
                }
                if ext {
                    let anims: Vec<Item> = model::ANIM_STYLES
                        .iter()
                        .map(|(tl, key)| {
                            Item::new(t.get(key), ea(EdAct::Anim(tl.to_i32()))).on(d.tiling == *tl)
                        })
                        .collect();
                    m.footer.push(t.get("editor_object_animation"));
                    for half in anims.chunks(3) {
                        m.row(half.to_vec());
                    }
                    if d.is_text() {
                        let types: Vec<Item> = model::TEXT_TYPES
                            .iter()
                            .map(|(n, w)| {
                                Item::new(w.to_string(), ea(EdAct::TextType(*n)))
                                    .on(d.word_type == *n)
                            })
                            .collect();
                        m.row(types);
                        m.row(vec![
                            Item::new(
                                format!(
                                    "{} {}",
                                    t.get("editor_object_text_manualtype"),
                                    d.word_type
                                ),
                                Act::None,
                            ),
                            Item::new("-", ea(EdAct::TypeStep(-1))),
                            Item::new("+", ea(EdAct::TypeStep(1))),
                        ]);
                    }
                }
                m.row(vec![
                    Item::new(
                        format!("{} {}", t.get("editor_object_zlevel"), d.layer),
                        Act::None,
                    ),
                    Item::new("<<", ea(EdAct::Layer(-5))),
                    Item::new("<", ea(EdAct::Layer(-1))),
                    Item::new(">", ea(EdAct::Layer(1))),
                    Item::new(">>", ea(EdAct::Layer(5))),
                ]);
                m.item(Item::new(
                    t.get("editor_object_reset"),
                    ea(EdAct::ResetObject),
                ));
            }
            EdMenu::LevelSetup => {
                m.top = 4.0;
                m.min_width = 380.0;
                let Some((l, i)) = lvl.zip(ed.and_then(|e| e.setup_icon)) else {
                    m.item(Item::new(t.get("return"), Act::Close));
                    return m;
                };
                let Some(icon) = l.info.levels.get(i) else {
                    m.item(Item::new(t.get("return"), Act::Close));
                    return m;
                };
                m.header.push(t.get("editor_level_levelsetup"));
                m.item(Item::new(t.get("return"), Act::Close));
                let target = if icon.file.is_empty() {
                    "-".to_string()
                } else {
                    let name =
                        baba_formats::Ini::load(&l.place.dir.join(format!("{}.ld", icon.file)))
                            .ok()
                            .and_then(|ini| ini.get("general", "name").map(str::to_string))
                            .unwrap_or_default();
                    format!("{name} ({})", icon.file)
                };
                m.item(
                    Item::new(
                        t.get("editor_level_leveltarget"),
                        open(EdMenu::PickLevel {
                            purpose: PickFor::IconTarget,
                            page: 0,
                        }),
                    )
                    .value(target),
                );
                m.row(vec![
                    Item::new(
                        t.get("editor_level_setcolour"),
                        ea(EdAct::Colour(ColourFor::Icon(i))),
                    ),
                    Item::new(
                        t.get("editor_level_setclearcolour"),
                        ea(EdAct::Colour(ColourFor::IconClear(i))),
                    ),
                ]);
                m.row(
                    [
                        (0, "editor_level_initialstate_hidden"),
                        (1, "editor_level_initialstate_normal"),
                        (2, "editor_level_initialstate_opened"),
                    ]
                    .iter()
                    .map(|(s, k)| {
                        Item::new(t.get(k), ea(EdAct::IconState(*s))).on(icon.state == *s)
                    })
                    .collect(),
                );
                m.row(
                    [
                        (3, "editor_level_levelsymbol_none"),
                        (0, "editor_level_levelsymbol_numbers"),
                        (1, "editor_level_levelsymbol_text"),
                        (2, "editor_level_levelsymbol_dots"),
                    ]
                    .iter()
                    .map(|(s, k)| {
                        Item::new(t.get(k), ea(EdAct::IconStyle(*s))).on(icon.style == *s)
                    })
                    .chain(std::iter::once(
                        Item::new("...", ea(EdAct::IconStyle(-1))).on(icon.style == -1),
                    ))
                    .collect(),
                );
                if icon.style == -1 {
                    let file = l
                        .info
                        .icons
                        .iter()
                        .find(|c| c.index as i32 == icon.number)
                        .map(|c| c.file.clone())
                        .unwrap_or_else(|| "-".into());
                    m.item(
                        Item::new(t.get("editor_map_mapicon"), ea(EdAct::IconPicture)).value(file),
                    );
                }
                m.row(vec![
                    Item::new(
                        format!(
                            "{} {}",
                            t.get("editor_level_levelsymbol_symbol"),
                            icon.number
                        ),
                        Act::None,
                    ),
                    Item::new("<<", ea(EdAct::IconNumber(-10))),
                    Item::new("<", ea(EdAct::IconNumber(-1))),
                    Item::new(">", ea(EdAct::IconNumber(1))),
                    Item::new(">>", ea(EdAct::IconNumber(10))),
                ]);
                m.item(Item::new(
                    t.get("editor_level_remove"),
                    ea(EdAct::IconRemove),
                ));
            }
            EdMenu::PathSetup => {
                m.top = 20.0;
                m.min_width = 380.0;
                let Some((l, i)) = lvl.zip(ed.and_then(|e| e.setup_path)) else {
                    m.item(Item::new(t.get("return"), Act::Close));
                    return m;
                };
                let Some(p) = l.info.paths.get(i) else {
                    m.item(Item::new(t.get("return"), Act::Close));
                    return m;
                };
                m.header.push(t.get("editor_path_settings"));
                m.item(Item::new(t.get("return"), Act::Close));
                let obj = l
                    .table
                    .get(&p.object)
                    .map(|d| d.name.clone())
                    .unwrap_or_default();
                m.row(vec![
                    Item::new(format!("{} {obj}", t.get("editor_path_object")), Act::None),
                    Item::new("<", ea(EdAct::PathObject(-1))),
                    Item::new(">", ea(EdAct::PathObject(1))),
                ]);
                m.footer.push(t.get("editor_path_pathstate"));
                m.row(vec![
                    Item::new(
                        t.get("editor_path_pathstate_hidden"),
                        ea(EdAct::PathState(0)),
                    )
                    .on(p.style == 0),
                    Item::new(
                        t.get("editor_path_pathstate_visible"),
                        ea(EdAct::PathState(1)),
                    )
                    .on(p.style == 1),
                ]);
                m.row(
                    [
                        (0, "no"),
                        (1, "editor_path_locked_levels"),
                        (2, "editor_path_locked_maps"),
                        (3, "editor_path_locked_orbs"),
                        (4, "editor_path_locked_loclevels"),
                    ]
                    .iter()
                    .map(|(g, k)| Item::new(t.get(k), ea(EdAct::PathGate(*g))).on(p.gate == *g))
                    .collect(),
                );
                m.row(vec![
                    Item::new(
                        format!("{} {}", t.get("editor_path_locked"), p.requirement),
                        Act::None,
                    ),
                    Item::new("-", ea(EdAct::PathReq(-1))),
                    Item::new("+", ea(EdAct::PathReq(1))),
                ]);
                m.item(Item::new(
                    t.get("editor_level_remove"),
                    ea(EdAct::PathRemove),
                ));
            }
            EdMenu::MapSetup => {
                m.top = 20.0;
                m.min_width = 380.0;
                let Some(l) = lvl else {
                    m.item(Item::new(t.get("return"), Act::Close));
                    return m;
                };
                let g = &l.info.general;
                m.item(Item::new(t.get("return"), Act::Close));
                m.footer.push(t.get("editor_map_leveltype"));
                m.row(vec![
                    Item::new(t.get("editor_map_level"), ea(EdAct::MapType(0)))
                        .on(g.level_type == 0),
                    Item::new(t.get("editor_map_map"), ea(EdAct::MapType(1))).on(g.level_type == 1),
                ]);
                m.row(vec![
                    Item::new(
                        format!("{} {}", t.get("editor_map_clearlimit"), g.unlock_count),
                        Act::None,
                    ),
                    Item::new("-", ea(EdAct::ClearLimit(-1))),
                    Item::new("+", ea(EdAct::ClearLimit(1))),
                ]);
                let cp = &g.custom_parent;
                m.row(vec![
                    Item::new(
                        t.get("editor_map_parentlevel"),
                        ea(EdAct::ReturnTo(String::new())),
                    )
                    .on(cp.is_empty()),
                    Item::new(
                        t.get("editor_map_winlevel"),
                        ea(EdAct::ReturnTo("<win>".into())),
                    )
                    .on(cp == "<win>"),
                    Item::new(
                        t.get("editor_map_selectlevel"),
                        open(EdMenu::PickLevel {
                            purpose: PickFor::ReturnTo,
                            page: 0,
                        }),
                    )
                    .on(!cp.is_empty() && cp != "<win>")
                    .value(if cp != "<win>" {
                        cp.clone()
                    } else {
                        String::new()
                    }),
                ]);
                m.header.push(t.get("editor_map_returnto"));
                m.gap(8.0);
                m.item(Item::new(
                    t.get("editor_map_mapicon"),
                    open(EdMenu::MapIcons),
                ));
            }
            EdMenu::MapIcons => {
                // The map's icon pictures (`[icons]`): a slot per picture,
                // used by level icons with the picture style (`§13.1`).
                m.top = 20.0;
                m.min_width = 380.0;
                m.item(Item::new(t.get("return"), Act::Close));
                m.header.push(t.get("editor_map_mapicon"));
                if let Some(l) = lvl {
                    let mut next = 0;
                    for c in &l.info.icons {
                        m.row(vec![
                            Item::new(format!("{}", c.index), ea(EdAct::IconSlot(c.index)))
                                .value(c.file.clone()),
                            Item::new("x", ea(EdAct::IconSlotReset(c.index))),
                        ]);
                        next = next.max(c.index + 1);
                    }
                    m.gap(8.0);
                    m.item(Item::new("+", ea(EdAct::IconSlot(next))));
                }
            }
            EdMenu::PickLevel { purpose, page } => {
                m.top = 8.0;
                m.min_width = 400.0;
                m.header.push(t.get("editor_levellist_edit"));
                m.item(Item::new(t.get("return"), Act::Close));
                let list = self
                    .ed_collection()
                    .map(|c| files::levels(&c.dir))
                    .unwrap_or_default();
                let pages = list.len().div_ceil(LIST_PER_PAGE).max(1);
                let page = (*page).min(pages - 1);
                for l in list.iter().skip(page * LIST_PER_PAGE).take(LIST_PER_PAGE) {
                    m.item(
                        Item::new(
                            if l.name.is_empty() {
                                l.stem.clone()
                            } else {
                                l.name.clone()
                            },
                            ea(EdAct::PickLevel(*purpose, l.stem.clone())),
                        )
                        .value(l.stem.clone()),
                    );
                }
                let p = *purpose;
                Self::page_row(&mut m, page, pages, |pg| ea(EdAct::PickPage(p, pg)));
            }
            EdMenu::Tags { page } => {
                m.top = 8.0;
                m.min_width = 160.0;
                m.item(Item::new(t.get("return"), Act::Close));
                m.item(Item::new(
                    t.get("editor_objectlist_tags_cleartags"),
                    ea(EdAct::ClearTags),
                ));
                let tags = ed.map(|e| e.cat.tags()).unwrap_or_default();
                let sel: Vec<String> = ed.map(|e| e.list.tags.clone()).unwrap_or_default();
                let per = 33;
                let pages = tags.len().div_ceil(per).max(1);
                let page = (*page).min(pages - 1);
                let items: Vec<Item> = tags
                    .iter()
                    .skip(page * per)
                    .take(per)
                    .map(|tg| {
                        let label = t.opt(&format!("tag_{tg}")).unwrap_or_else(|| tg.clone());
                        Item::new(label, ea(EdAct::Tag(tg.clone()))).on(sel.contains(tg))
                    })
                    .collect();
                for row in items.chunks(3) {
                    m.row(row.to_vec());
                }
                Self::page_row(&mut m, page, pages, |p| ea(EdAct::TagsPage(p)));
            }
            EdMenu::Notice(s) => {
                m.top = 170.0;
                for line in s.lines() {
                    m.header.push(line.to_string());
                }
                m.item(Item::new("OK", Act::Close));
            }
            EdMenu::Upload => {
                m.top = 110.0;
                m.header.push(t.get("editor_upload_confirm"));
                if let Some(l) = lvl {
                    let g = &l.info.general;
                    m.header
                        .push(format!("{} {}", t.get("editor_upload_name"), g.name));
                    m.header
                        .push(format!("{} {}", t.get("editor_upload_author"), g.author));
                    m.header.push(format!(
                        "{} {}",
                        t.get("editor_upload_subtitle"),
                        g.subtitle
                    ));
                }
                m.header.push(t.get("editor_upload_confirm_note"));
                m.row(vec![
                    Item::new(t.get("no"), Act::Close),
                    Item::new(t.get("yes"), ea(EdAct::UploadGo)),
                ]);
            }
        }
        m
    }

    fn ed_replace_top(&mut self, m: EdMenu) {
        if let Some(top) = self.menus.last_mut() {
            top.0 = em(m);
        }
    }

    fn with_level(&mut self, f: impl FnOnce(&mut EdLevel)) {
        if let Some(ed) = self.ed_mut()
            && let Some(l) = ed.level.as_mut()
        {
            f(l);
            ed.unsaved = true;
        }
    }

    fn with_editing(&mut self, f: impl FnOnce(&mut baba_formats::ObjectDef)) {
        let objects = &self.objects;
        let Some(ed) = self.edh.ed.as_deref_mut() else {
            return;
        };
        let (Some(i), Some(l)) = (ed.editing, ed.level.as_mut()) else {
            return;
        };
        if let Some(mut d) = l.def(i).cloned() {
            f(&mut d);
            l.set_def(objects, i, d);
            ed.unsaved = true;
        }
    }

    pub(super) fn ed_activate(&mut self, a: EdAct) {
        match a {
            EdAct::Open => self.ed_open(),
            EdAct::Quit => self.ed_quit(),
            EdAct::Back => {
                self.menus.pop();
            }
            EdAct::Open_(m) => self.menus.push((em(m), (0, 0))),
            EdAct::EditLevels => {
                let Some(u) = self.user_dir_ed() else {
                    return;
                };
                let dir =
                    packs::ensure_user_levels(&u).unwrap_or_else(|_| packs::user_levels_dir(&u));
                if let Some(ed) = self.ed_mut() {
                    ed.coll = Some(Collection {
                        world: packs::LEVELS.into(),
                        dir,
                    });
                    ed.list_mode = ListMode::Edit;
                }
                self.menus.push((em(EdMenu::Levels { page: 0 }), (0, 0)));
            }
            EdAct::EditPacks => self.menus.push((em(EdMenu::Packs { page: 0 }), (0, 0))),
            EdAct::LevelPage(p) => self.ed_replace_top(EdMenu::Levels { page: p }),
            EdAct::PackPage(p) => self.ed_replace_top(EdMenu::Packs { page: p }),
            EdAct::OpenPack(id) => {
                let Some(u) = self.user_dir_ed() else {
                    return;
                };
                let dir = packs::user_worlds_dir(&u).join(&id);
                // Install packs are edited as a copy in the data dir.
                let src = self.data.join("Worlds").join(&id);
                if !dir.is_dir()
                    && src.is_dir()
                    && let Err(e) = files::copy_dir(&src, &dir)
                {
                    return self.ed_notice(format!("{e}"));
                }
                files::clean_test_files(&dir);
                if let Some(ed) = self.ed_mut() {
                    ed.coll = Some(Collection { world: id, dir });
                    ed.list_mode = ListMode::Edit;
                }
                self.menus.push((em(EdMenu::Levels { page: 0 }), (0, 0)));
            }
            EdAct::OpenLevel(stem) => {
                let mode = self.ed().map(|e| e.list_mode).unwrap_or_default();
                let Some(c) = self.ed_collection() else {
                    return;
                };
                match mode {
                    ListMode::Edit => self.ed_open_stem(&stem),
                    ListMode::SetFirst | ListMode::SetMap => {
                        let key = if mode == ListMode::SetFirst {
                            "firstlevel"
                        } else {
                            "start"
                        };
                        let _ = files::set_world(&c.dir, |st| st.set("general", key, &stem));
                        self.log.push(format!("pack {key}={stem}"));
                        if let Some(ed) = self.ed_mut() {
                            ed.list_mode = ListMode::Edit;
                        }
                    }
                }
            }
            EdAct::NewLevel => self.ed_prompt(Prompt::NewLevel, String::new()),
            EdAct::NewPack => self.ed_prompt(Prompt::NewPack, String::new()),
            EdAct::SortName => {
                if let Some(ed) = self.ed_mut() {
                    ed.sort_name = !ed.sort_name;
                }
            }
            EdAct::SortType => {
                if let Some(ed) = self.ed_mut() {
                    ed.sort_type = !ed.sort_type;
                }
            }
            EdAct::SetListMode(m) => {
                if let Some(ed) = self.ed_mut() {
                    ed.list_mode = if ed.list_mode == m { ListMode::Edit } else { m };
                }
            }
            EdAct::ClearSearch => {
                if let Some(ed) = self.ed_mut() {
                    ed.level_search.clear();
                }
            }
            EdAct::ToggleSetting(k) => {
                let st = &mut self.profile.settings.store;
                let v = match k {
                    "mode" => st.get_int("editor", "mode").unwrap_or(1) == 0,
                    "advanced" => {
                        !(st.int("editor", "advanced") != 0 || st.int("editor", "special") != 0)
                    }
                    _ => st.int("editor", k) == 0,
                };
                st.set("editor", k, if v { 1 } else { 0 });
                if k == "advanced" && !v {
                    st.set("editor", "special", 0);
                }
                self.profile.save_settings();
                if k == "nomusic" {
                    self.ed_music();
                }
            }
            EdAct::DefaultPad => {
                pad::reset(&mut self.profile.settings);
                self.profile.save_settings();
            }
            EdAct::BindPad(a) => {
                self.edh.capture = Some(a);
                self.menus.push((MenuId::Waiting, (0, 0)));
            }
            EdAct::Lesson(i) => self.ed_start_lesson(i, false),
            EdAct::CloseMenu => {
                self.menus.clear();
            }
            EdAct::Test => {
                self.menus.clear();
                self.ed_test(false);
            }
            EdAct::Upload => self.menus.push((em(EdMenu::Upload), (0, 0))),
            EdAct::UploadGo => {
                self.menus.clear();
                if super::extras::UPLOAD {
                    self.ed_save();
                    self.ed_test(true);
                }
            }
            EdAct::Save => {
                self.menus.clear();
                self.ed_save();
            }
            EdAct::ToList => {
                self.menus.clear();
                self.ed_leave(false);
            }
            EdAct::ToStart => {
                self.menus.clear();
                self.ed_leave(true);
            }
            EdAct::Delete => self.ed_confirm(Confirm::Delete),
            EdAct::Copy => self.ed_confirm(Confirm::Copy),
            EdAct::SetMusic(name, local) => {
                self.with_level(|l| {
                    l.info.general.music = name;
                    l.set_gen("localmusic", if local { 1 } else { 0 });
                });
                self.menus.pop();
                self.ed_music();
            }
            EdAct::SetParticles(p) => {
                self.with_level(|l| l.info.general.particles = p);
                self.menus.pop();
            }
            EdAct::SetPalette(file, root) => {
                self.with_level(|l| {
                    l.info.general.palette = file;
                    l.info.general.palette_root = root;
                });
                self.menus.pop();
            }
            EdAct::ToggleGen(k) => self.with_level(|l| {
                let v = l.gen_int(k) == 0;
                l.set_gen(k, if v { 1 } else { 0 });
            }),
            EdAct::AutoDelay(d) => self.with_level(|l| {
                let v = l
                    .info
                    .ini
                    .get_int("general", "rhythm")
                    .filter(|v| *v > 0)
                    .unwrap_or(10);
                l.set_gen("rhythm", (v + d as i64).clamp(1, 30));
            }),
            EdAct::SizeStep(dw, dh) => {
                if let Some((MenuId::Editor(EdMenu::Size { w, h }), _)) = self.menus.last_mut() {
                    *w = (*w as i32 + dw).clamp(1, model::MAX_W as i32) as u32;
                    *h = (*h as i32 + dh).clamp(1, model::MAX_H as i32) as u32;
                }
            }
            EdAct::SizeSet(w, h) => self.ed_replace_top(EdMenu::Size { w, h }),
            EdAct::SizeApply => {
                if let Some((MenuId::Editor(EdMenu::Size { w, h }), _)) = self.menus.last().cloned()
                {
                    if let Some(ed) = self.ed_mut() {
                        ed.undo_begin();
                        if let Some(l) = ed.level.as_mut() {
                            l.resize(w, h);
                        }
                        ed.undo_commit();
                        ed.undo.clear();
                        ed.unsaved = true;
                        if let Some(l) = &ed.level {
                            ed.cursor = (
                                ed.cursor.0.clamp(1, l.width() as i32 - 2),
                                ed.cursor.1.clamp(1, l.height() as i32 - 2),
                            );
                        }
                    }
                    self.log.push(format!("level size {w}x{h}"));
                    self.menus.pop();
                }
            }
            EdAct::MusicPage(p) => self.ed_replace_top(EdMenu::Music { page: p }),
            EdAct::PalettePage(p) => self.ed_replace_top(EdMenu::Palettes { page: p }),
            EdAct::ThemePage(delete, page) => {
                if matches!(
                    self.menus.last(),
                    Some((MenuId::Editor(EdMenu::ThemeList { .. }), _))
                ) {
                    self.ed_replace_top(EdMenu::ThemeList { delete, page });
                } else {
                    self.menus
                        .push((em(EdMenu::ThemeList { delete, page }), (0, 0)));
                }
            }
            EdAct::PickTheme(path, delete) => self.ed_confirm(if delete {
                Confirm::DeleteTheme(path)
            } else {
                Confirm::LoadTheme(path)
            }),
            EdAct::NewWithTheme(theme) => self.ed_create_level(theme),
            EdAct::Yes(c) => self.ed_yes(c),
            EdAct::Prompt(kind) => {
                let text = self.prompt_initial(&kind);
                self.ed_prompt(kind, text);
            }
            EdAct::TypeChar(c) => {
                if let Some((MenuId::Editor(EdMenu::Prompt { kind, text, upper }), _)) =
                    self.menus.last_mut()
                {
                    let (_, max, lower) = kind.spec();
                    let c = if lower { c.to_ascii_lowercase() } else { c };
                    if text.chars().count() < max {
                        text.push(c);
                    }
                    *upper = false;
                }
            }
            EdAct::TypeBack => {
                if let Some((MenuId::Editor(EdMenu::Prompt { text, .. }), _)) =
                    self.menus.last_mut()
                {
                    text.pop();
                }
            }
            EdAct::TypeShift => {
                if let Some((MenuId::Editor(EdMenu::Prompt { upper, .. }), _)) =
                    self.menus.last_mut()
                {
                    *upper = !*upper;
                }
            }
            EdAct::TypeDone => {
                if let Some((MenuId::Editor(EdMenu::Prompt { kind, text, .. }), _)) =
                    self.menus.pop()
                {
                    self.ed_prompt_done(kind, text);
                }
            }
            EdAct::Sprite => self.ed_open_sprites(),
            EdAct::Colour(w) => {
                let cur = self.ed().and_then(|e| {
                    let l = e.level.as_ref()?;
                    match w {
                        ColourFor::Base => e.editing.and_then(|i| l.def(i)).map(|d| d.colour),
                        ColourFor::Active => e
                            .editing
                            .and_then(|i| l.def(i))
                            .and_then(|d| d.active_colour),
                        ColourFor::Icon(i) => l.info.levels.get(i).and_then(|c| c.colour),
                        ColourFor::IconClear(i) => {
                            l.info.levels.get(i).and_then(|c| c.clear_colour)
                        }
                    }
                });
                self.edh.colour = cur.map(|c| (c.x, c.y)).unwrap_or((0, 3));
                self.menus.clear();
                if let Some(ed) = self.ed_mut() {
                    ed.screen = Screen::Colour(w);
                }
            }
            EdAct::Anim(v) => self.with_editing(|d| d.tiling = baba_formats::Tiling::from_i32(v)),
            EdAct::TextType(v) => self.with_editing(|d| d.word_type = v),
            EdAct::TypeStep(s) => {
                self.with_editing(|d| d.word_type = (d.word_type + s).clamp(0, 20))
            }
            EdAct::Layer(s) => self.with_editing(|d| d.layer = (d.layer + s).clamp(0, 30)),
            EdAct::ResetObject => {
                let objects = &self.objects;
                if let Some(ed) = self.edh.ed.as_deref_mut()
                    && let (Some(i), Some(l)) = (ed.editing, ed.level.as_mut())
                {
                    l.reset_def(objects, &ed.cat, i);
                    ed.unsaved = true;
                }
            }
            EdAct::IconState(s) => self.with_icon(|i| i.state = s),
            EdAct::IconStyle(s) => self.with_icon(|i| i.style = s),
            EdAct::IconNumber(d) => self.with_icon(|i| i.number = (i.number + d).clamp(0, 99)),
            EdAct::IconPicture => {
                if let Some(ed) = self.ed_mut() {
                    ed.sprites.slot = None;
                }
                self.ed_open_icon_pics();
            }
            EdAct::IconSlot(n) => {
                if let Some(ed) = self.ed_mut() {
                    ed.sprites.slot = Some(n);
                }
                self.ed_open_icon_pics();
            }
            EdAct::IconSlotReset(n) => {
                if let Some(ed) = self.ed_mut()
                    && let Some(l) = ed.level.as_mut()
                {
                    l.info.icons.retain(|c| c.index != n);
                    ed.unsaved = true;
                }
            }
            EdAct::IconRemove => {
                if let Some(ed) = self.ed_mut()
                    && let (Some(i), Some(l)) = (ed.setup_icon.take(), ed.level.as_mut())
                    && i < l.info.levels.len()
                {
                    ed.undo.begin(l);
                    l.info.levels.remove(i);
                    ed.undo.commit(l);
                    ed.unsaved = true;
                }
                self.menus.pop();
            }
            EdAct::PickLevel(purpose, stem) => {
                match purpose {
                    PickFor::IconTarget => self.with_icon(|i| i.file = stem),
                    PickFor::ReturnTo => self.with_level(|l| l.info.general.custom_parent = stem),
                }
                self.menus.pop();
            }
            EdAct::PickPage(purpose, page) => {
                self.ed_replace_top(EdMenu::PickLevel { purpose, page })
            }
            EdAct::PathState(s) => self.with_path(|p| p.style = s),
            EdAct::PathGate(g) => self.with_path(|p| p.gate = g),
            EdAct::PathReq(d) => self.with_path(|p| p.requirement = (p.requirement + d).max(0)),
            EdAct::PathObject(d) => {
                let slots: Vec<String> = self
                    .ed()
                    .and_then(|e| e.level.as_ref())
                    .map(|l| l.palette.iter().map(|e| e.slot.clone()).collect())
                    .unwrap_or_default();
                if !slots.is_empty() {
                    self.with_path(|p| {
                        let k = slots.iter().position(|s| *s == p.object).unwrap_or(0) as i32;
                        let n = slots.len() as i32;
                        p.object = slots[(k + d).rem_euclid(n) as usize].clone();
                    });
                }
            }
            EdAct::PathRemove => {
                if let Some(ed) = self.ed_mut()
                    && let (Some(i), Some(l)) = (ed.setup_path.take(), ed.level.as_mut())
                    && i < l.info.paths.len()
                {
                    ed.undo.begin(l);
                    l.info.paths.remove(i);
                    ed.undo.commit(l);
                    ed.unsaved = true;
                }
                self.menus.pop();
            }
            EdAct::MapType(v) => self.with_level(|l| l.info.general.level_type = v),
            EdAct::ClearLimit(d) => self.with_level(|l| {
                l.info.general.unlock_count = (l.info.general.unlock_count + d).max(0)
            }),
            EdAct::ReturnTo(s) => self.with_level(|l| l.info.general.custom_parent = s),
            EdAct::Tag(tg) => {
                if let Some(ed) = self.ed_mut() {
                    if let Some(k) = ed.list.tags.iter().position(|x| *x == tg) {
                        ed.list.tags.remove(k);
                    } else {
                        ed.list.tags.push(tg);
                    }
                    ed.list.page = 0;
                    ed.list.sel = 0;
                }
            }
            EdAct::ClearTags => {
                if let Some(ed) = self.ed_mut() {
                    ed.list.tags.clear();
                }
            }
            EdAct::TagsPage(p) => self.ed_replace_top(EdMenu::Tags { page: p }),
            EdAct::EndTest => {
                self.log.push("editor test end".into());
                self.back_to_editor();
            }
        }
    }

    fn with_icon(&mut self, f: impl FnOnce(&mut baba_formats::leveldata::LevelLink)) {
        if let Some(ed) = self.ed_mut()
            && let (Some(i), Some(l)) = (ed.setup_icon, ed.level.as_mut())
            && let Some(icon) = l.info.levels.get_mut(i)
        {
            f(icon);
            ed.unsaved = true;
        }
    }

    fn with_path(&mut self, f: impl FnOnce(&mut baba_formats::leveldata::MapPath)) {
        if let Some(ed) = self.ed_mut()
            && let (Some(i), Some(l)) = (ed.setup_path, ed.level.as_mut())
            && let Some(p) = l.info.paths.get_mut(i)
        {
            f(p);
            ed.unsaved = true;
        }
    }

    fn prompt_initial(&self, kind: &Prompt) -> String {
        let ed = self.ed();
        let l = ed.and_then(|e| e.level.as_ref());
        let g = l.map(|l| &l.info.general);
        let c = self.ed_collection();
        let wd = c.as_ref().map(|c| files::world_store(&c.dir));
        match kind {
            Prompt::RenameLevel => g.map(|g| g.name.clone()).unwrap_or_default(),
            Prompt::Author => g.map(|g| g.author.clone()).unwrap_or_default(),
            Prompt::Subtitle => g.map(|g| g.subtitle.clone()).unwrap_or_default(),
            Prompt::PackName => wd
                .and_then(|w| w.get("general", "name").map(str::to_string))
                .unwrap_or_default(),
            Prompt::PackAuthor => wd
                .and_then(|w| w.get("general", "author").map(str::to_string))
                .unwrap_or_default(),
            Prompt::ObjName => ed
                .and_then(|e| e.editing.and_then(|i| l?.def(i)).map(|d| d.name.clone()))
                .unwrap_or_default(),
            Prompt::ObjType => ed
                .and_then(|e| {
                    e.editing
                        .and_then(|i| l?.def(i))
                        .map(|d| d.unittype.clone())
                })
                .unwrap_or_default(),
            Prompt::LevelSearch => ed.map(|e| e.level_search.clone()).unwrap_or_default(),
            Prompt::EditorAuthor => self.ed_author(),
            _ => String::new(),
        }
    }

    /// A prompt was confirmed (`§16` limits and sanitising).
    fn ed_prompt_done(&mut self, kind: Prompt, text: String) {
        let clean = model::sanitize(&text);
        let name = if clean.trim().is_empty() {
            "EMPTY".to_string()
        } else {
            clean.trim().to_string()
        };
        match kind {
            Prompt::NewLevel => {
                let themes =
                    files::themes(&self.data, self.ed_collection().map(|c| c.dir).as_deref());
                if let Some(ed) = self.ed_mut() {
                    ed.pending_new = Some((name, String::new()));
                }
                if themes.is_empty() {
                    self.ed_create_level(None);
                } else {
                    self.menus.push((em(EdMenu::NewTheme { page: 0 }), (0, 0)));
                }
            }
            Prompt::NewPack => {
                let Some(u) = self.user_dir_ed() else { return };
                match files::create_pack(&u, &name, &self.ed_author()) {
                    Ok(p) => {
                        self.log.push(format!("pack created {}", p.id));
                        self.ed_activate(EdAct::OpenPack(p.id));
                    }
                    Err(e) => self.ed_notice(format!("{e}")),
                }
            }
            Prompt::RenameLevel => self.with_level(|l| l.info.general.name = name),
            Prompt::Author => self.with_level(|l| l.info.general.author = clean.trim().to_string()),
            Prompt::Subtitle => {
                self.with_level(|l| l.info.general.subtitle = clean.trim().to_string())
            }
            Prompt::PackName | Prompt::PackAuthor => {
                if let Some(c) = self.ed_collection() {
                    let key = if kind == Prompt::PackName {
                        "name"
                    } else {
                        "author"
                    };
                    let v = if kind == Prompt::PackName {
                        name
                    } else {
                        clean.trim().to_string()
                    };
                    let _ = files::set_world(&c.dir, |st| st.set("general", key, &v));
                }
            }
            Prompt::EditorAuthor => {
                self.profile
                    .settings
                    .store
                    .set("editor", "author", clean.trim());
                self.profile.save_settings();
            }
            Prompt::ObjName => {
                let objects = &self.objects;
                let Some(ed) = self.edh.ed.as_deref_mut() else {
                    return;
                };
                if let (Some(i), Some(l)) = (ed.editing, ed.level.as_mut()) {
                    if !l.rename(objects, i, &text) {
                        let msg = self.texts.get("editor_object_name_invalid");
                        self.ed_notice(msg);
                    } else if let Some(ed) = self.ed_mut() {
                        ed.unsaved = true;
                    }
                }
            }
            Prompt::ObjType => {
                let v: String = text.trim().to_lowercase().chars().take(12).collect();
                if !v.is_empty() {
                    self.with_editing(|d| d.unittype = v);
                }
            }
            Prompt::ThemeName => {
                let coll = self.ed_collection();
                let exists = coll.as_ref().is_some_and(|c| {
                    files::themes(std::path::Path::new("/nonexistent"), Some(&c.dir))
                        .iter()
                        .any(|t| t.name == name)
                });
                if exists {
                    self.ed_confirm(Confirm::OverwriteTheme(name));
                } else {
                    self.ed_save_theme(&name);
                }
            }
            Prompt::PalSearch => {
                if let Some(ed) = self.ed_mut() {
                    ed.pal.search = clean.trim().to_lowercase();
                }
            }
            Prompt::ListSearch => {
                if let Some(ed) = self.ed_mut() {
                    ed.list.search = clean.trim().to_lowercase();
                    ed.list.page = 0;
                    ed.list.sel = 0;
                }
            }
            Prompt::SpriteSearch => {
                if let Some(ed) = self.ed_mut() {
                    ed.sprites.search = clean.trim().to_lowercase();
                    ed.sprites.page = 0;
                    ed.sprites.sel = 0;
                }
            }
            Prompt::LevelSearch => {
                if let Some(ed) = self.ed_mut() {
                    ed.level_search = clean.trim().to_string();
                }
            }
            Prompt::AutoAdd => self.ed_autoadd(&text),
            Prompt::AutoPick => {
                let objects = &self.objects;
                if let Some(ed) = self.edh.ed.as_deref_mut() {
                    let n = ed.autopick(objects, &text);
                    self.log.push(format!("autopick {n}"));
                }
            }
            Prompt::Special(i) => self.with_level(|l| {
                if let Some(s) = l.info.specials.get_mut(i) {
                    s.data = text.trim().to_string();
                }
            }),
        }
    }

    /// Palette F4: adds each known word (with or without `text_`) that is
    /// not yet in the palette (`§5.4`).
    fn ed_autoadd(&mut self, text: &str) {
        let pairs = true;
        let objects = &self.objects;
        let Some(ed) = self.edh.ed.as_deref_mut() else {
            return;
        };
        for w in text.split_whitespace() {
            let w = w.to_lowercase();
            let w = w.strip_prefix("text_").unwrap_or(&w).to_string();
            let Some(l) = ed.level.as_mut() else { break };
            if l.palette.len() >= MAX_PALETTE {
                break;
            }
            if l.entry_by_name(&w).is_some() || l.entry_by_name(&format!("text_{w}")).is_some() {
                continue;
            }
            let o = ed
                .cat
                .by_name(&w)
                .or_else(|| ed.cat.by_name(&format!("text_{w}")))
                .filter(|o| !o.redacted);
            if let Some(o) = o {
                let id = o.index as i32;
                if l.add_object(objects, &ed.cat, id, pairs).is_ok() {
                    ed.unsaved = true;
                }
            }
        }
        ed.undo.clear();
        ed.update_path_object();
    }

    fn ed_save_theme(&mut self, name: &str) {
        let Some(c) = self.ed_collection() else {
            return;
        };
        let Some(text) = self
            .edh
            .ed
            .as_deref_mut()
            .and_then(|e| e.level.as_mut())
            .map(|l| l.theme_text(name))
        else {
            return;
        };
        match files::save_theme(&c.dir, name, &text) {
            Ok(p) => {
                self.log.push(format!("theme saved {}", p.display()));
                if let Some(ed) = self.ed_mut() {
                    ed.note(name.to_string());
                }
            }
            Err(e) => self.ed_notice(format!("{e}")),
        }
    }

    /// Creates the pending new level (`§3.2`) from a theme or the default
    /// palette, and opens it.
    fn ed_create_level(&mut self, theme: Option<PathBuf>) {
        let Some(c) = self.ed_collection() else {
            return;
        };
        let Some((name, _)) = self.ed_mut().and_then(|e| e.pending_new.take()) else {
            return;
        };
        let stem = match files::new_stem(&c.dir) {
            Ok(s) => s,
            Err(e) => return self.ed_notice(format!("{e}")),
        };
        let author = self.ed_author();
        let objects = &self.objects;
        let Some(ed) = self.edh.ed.as_deref_mut() else {
            return;
        };
        let mut l = EdLevel::new(
            &stem,
            crate::editor::place_of(&c),
            &name,
            &author,
            objects,
            &ed.cat,
        );
        if let Some(p) = &theme
            && let Ok(info) = baba_formats::LevelInfo::load(p)
        {
            l.apply_theme(objects, &ed.cat, &info);
            l.info.general.name = name.clone();
        }
        ed.open_level(l);
        ed.unsaved = true;
        self.menus.clear();
        self.log.push(format!("editor new {}/{stem}", c.world));
        // The new level exists on disk right away.
        self.ed_save();
        self.ed_music();
    }

    fn ed_yes(&mut self, c: Confirm) {
        self.menus.pop();
        match c {
            Confirm::LeaveList => self.ed_leave_now(false),
            Confirm::LeaveMain => self.ed_leave_now(true),
            Confirm::OpenOther(stem) => {
                if let Some(ed) = self.ed_mut() {
                    ed.unsaved = false;
                }
                self.ed_open_stem(&stem);
            }
            Confirm::Delete => {
                if let Some(l) = self.ed().and_then(|e| e.level.as_ref()) {
                    files::delete_level(&l.place.dir, &l.stem);
                    self.log.push(format!("editor deleted {}", l.stem));
                }
                if let Some(ed) = self.ed_mut() {
                    ed.unsaved = false;
                }
                self.ed_leave_now(false);
            }
            Confirm::DeleteListed(stem) => {
                if let Some(c) = self.ed_collection() {
                    files::delete_level(&c.dir, &stem);
                }
            }
            Confirm::Copy => {
                let Some(l) = self.ed().and_then(|e| e.level.as_ref()) else {
                    return;
                };
                let (dir, stem, name) = (
                    l.place.dir.clone(),
                    l.stem.clone(),
                    l.info.general.name.clone(),
                );
                match files::copy_level(&dir, &stem, &files::copy_name(&name)) {
                    Ok(new) => {
                        self.log.push(format!("editor copied {stem} to {new}"));
                        let msg = self.texts.get("editor_copy_done");
                        self.ed_notice(msg);
                    }
                    Err(e) => self.ed_notice(format!("{e}")),
                }
            }
            Confirm::LoadTheme(p) => {
                let objects = &self.objects;
                if let Some(ed) = self.edh.ed.as_deref_mut()
                    && let Ok(info) = baba_formats::LevelInfo::load(&p)
                    && let Some(l) = ed.level.as_mut()
                {
                    l.apply_theme(objects, &ed.cat, &info);
                    ed.undo.clear();
                    ed.unsaved = true;
                    ed.update_path_object();
                    ed.pick_quick(0);
                    self.log.push(format!("theme loaded {}", p.display()));
                }
                self.menus.clear();
                self.ed_music();
            }
            Confirm::EmptyLevel => {
                if let Some(ed) = self.ed_mut() {
                    ed.undo_begin();
                    if let Some(l) = ed.level.as_mut() {
                        l.clear();
                    }
                    ed.undo_commit();
                    ed.unsaved = true;
                }
            }
            Confirm::OverwriteTheme(name) => self.ed_save_theme(&name),
            Confirm::DeleteTheme(p) => {
                let _ = std::fs::remove_file(&p);
                self.log.push(format!("theme deleted {}", p.display()));
            }
        }
    }

    // ------------------------------------------------------------ render

    /// Draws the editor's level view (before the overlays).
    pub(super) fn render_editor(&mut self, w: u32, h: u32, wob: u32) {
        self.edh.screen = (w, h);
        let objects = &self.objects;
        let Some(ed) = self.edh.ed.as_deref_mut() else {
            return;
        };
        let Some(l) = ed.level.as_mut() else {
            self.edh.fit = None;
            return;
        };
        self.edh.gfx.sync(l, &self.data);
        self.edh.gfx.update_actives(l, objects);
        let scene = self.edh.gfx.scene(&mut self.gfx.atlas, l);
        self.renderer.sync_atlas(&mut self.gfx.atlas);
        let ui = Ui::new(w, h);
        let top = ui.y(view::TOP).round() as i32;
        let bottom = ui.y(480.0 - view::BOTTOM).round() as i32;
        // The level is drawn as it is; menus and the palette veil it
        // (`draw_editor`).
        let dim = false;
        let v = View {
            scene: &scene,
            wobble: if self.setting("noslide") { 0 } else { wob },
            rect: Rect::new(0, top, w as i32, (bottom - top).max(1)),
            scale: self.scale_mode(),
            effect: PostEffect {
                tint: if dim { [110, 110, 110, 255] } else { [255; 4] },
                ..Default::default()
            },
        };
        self.edh.fit = Some(self.renderer.draw_view(0, &self.gfx.atlas, &v));
    }

    /// Editor overlays (bars, cursor, palette and picker screens,
    /// tutorial box).
    pub(super) fn draw_editor(&mut self, ui: &mut Ui) {
        // The menu shown behind a tutorial slide (not active).
        let tut_menu = self.edh.tut_menu.clone().map(|m| self.build_ed(&m));
        let ext = self.ed_extended();
        let adv = self.ed_advanced();
        let hints = !self.setting("nohints");
        let fit = self.edh.fit;
        let Some(ed) = self.edh.ed.as_deref() else {
            return;
        };
        let g = &mut self.gfx;
        if ed.level.is_none() {
            // The original's editor menus: a near-black page on the
            // outside colour.
            ui.fill(g, g.pal(1, 0));
            ui.rect(g, 8.0, -200.0, 838.0, 880.0, [8, 8, 8, 255]);
            return;
        }
        // Menus and the palette veil the level in its outside colour, like
        // the original (captured).
        if !self.menus.is_empty() || ed.screen == Screen::Palette {
            let mut c = self.edh.gfx.colour(baba_formats::PalColour::new(1, 0));
            c[3] = 200;
            ui.fill(g, c);
        }
        let eg = &mut self.edh.gfx;
        let menu_open = !self.menus.is_empty();
        match &ed.screen {
            // With a menu open only the title stays (the original hides
            // the bars, captured).
            Screen::Main | Screen::Menus if menu_open => {
                let l = ed.level.as_ref().unwrap();
                let title = format!("{} ({})", l.info.general.name, l.stem);
                ui.text(
                    g,
                    Some(&self.texts),
                    &title,
                    12.0,
                    2.0,
                    16.0,
                    g.pal(0, 3),
                    Align::Left,
                );
            }
            Screen::Main | Screen::Menus => {
                view::draw_main(ui, g, &self.texts, ed, eg, fit, ext, hints)
            }
            Screen::Palette => view::draw_palette(ui, g, &self.texts, ed, eg, ext),
            Screen::ObjList => view::draw_list(ui, g, &self.texts, ed, eg, &self.objects, ext, adv),
            Screen::Sprites => view::draw_sprites(ui, g, &self.texts, ed, eg, &self.data),
            Screen::Colour(w) => {
                let cur = self.edh.colour;
                view::draw_colours(
                    ui,
                    g,
                    &self.texts,
                    eg,
                    *w,
                    Some(baba_formats::PalColour::new(cur.0, cur.1)),
                );
            }
        }
        if let Some(run) = &ed.tutorial {
            if let Some(menu) = tut_menu {
                let c = g.pal(1, 0);
                ui.fill(g, [c[0], c[1], c[2], 170]);
                let mut mui = menu.ui_for(ui, g, &self.texts);
                menu.draw(&mut mui, g, &self.texts, (usize::MAX, 0), None);
                ui.quads.append(&mut mui.quads);
            }
            view::draw_tutorial(ui, g, &self.texts, run, ed.pad.active);
            // Pointer arrows at cells or buttons of the slide.
            let targets: Vec<(f32, f32)> =
                crate::editor::tutorial::pointers(run.lesson, run.slide, ed.pad.active)
                    .iter()
                    .filter_map(|p| match *p {
                        crate::editor::tutorial::Pointer::At(x, y) => {
                            Some((427.0 + x * 24.0, 240.0 + y * 24.0))
                        }
                        crate::editor::tutorial::Pointer::Button(name) => {
                            let act = match name {
                                "objects" => view::BtnAct::Objects,
                                "menu" => view::BtnAct::Menu,
                                _ => view::BtnAct::Add,
                            };
                            let btns = if name == "add" {
                                view::palette_buttons(ed, &self.texts, ext)
                            } else {
                                view::main_buttons(ed, &self.texts, ext)
                            };
                            btns.iter().find(|b| b.act == act).map(|b| {
                                let (bx, by) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
                                (427.0 + (bx - 427.0) * 0.9, 240.0 + (by - 240.0) * 0.9)
                            })
                        }
                    })
                    .collect();
            view::draw_pointers(ui, g, &targets);
        }
    }

    /// Screen pixel of the centre of level cell `(x, y)` in the editor
    /// (scripted clicks).
    pub fn editor_cell_px(&self, x: i32, y: i32) -> Option<(f32, f32)> {
        let fit = self.edh.fit?;
        let c = 24.0 * fit.scale;
        Some((
            fit.rect.x as f32 + (x as f32 + 0.5) * c,
            fit.rect.y as f32 + (y as f32 + 0.5) * c,
        ))
    }

    /// The edited level's files as written now (`.l`, `.ld`), for checks.
    pub fn editor_level_bytes(&mut self) -> Option<(Vec<u8>, Vec<u8>)> {
        let l = self.edh.ed.as_deref_mut()?.level.as_mut()?;
        Some(l.file_bytes())
    }

    /// The edited level's folder and stem.
    pub fn editor_level_path(&self) -> Option<(PathBuf, String)> {
        let l = self.ed()?.level.as_ref()?;
        Some((l.place.dir.clone(), l.stem.clone()))
    }

    /// Extra drawing over editor menus: the edited object's preview, the
    /// selected level's thumbnail.
    pub(super) fn ed_menu_draw(&mut self, ui: &mut Ui, id: &EdMenu, sel: (usize, usize)) {
        match id {
            EdMenu::EditObject => {
                let Some(ed) = self.edh.ed.as_deref() else {
                    return;
                };
                let (Some(l), Some(i)) = (ed.level.as_ref(), ed.editing) else {
                    return;
                };
                let Some(d) = l.def(i).cloned() else { return };
                let eg = &mut self.edh.gfx;
                let r = eg.thumb(&mut self.gfx.atlas, &d, ed.dir);
                let c = eg.colour(d.colour);
                ui.rect(&self.gfx, 30.0, 30.0, 100.0, 100.0, [0, 0, 0, 200]);
                ui.sprite(r, 38.0, 38.0, 84.0, 84.0, c);
                if let Some(a) = d.active_colour {
                    let ac = eg.colour(a);
                    ui.sprite(r, 140.0, 82.0, 48.0, 48.0, ac);
                }
            }
            EdMenu::Levels { page } => {
                let Some(c) = self.ed_collection() else {
                    return;
                };
                let Some(ed) = self.ed() else { return };
                let list = files::sorted(
                    files::levels(&c.dir),
                    ed.sort_name,
                    ed.sort_type,
                    &ed.level_search,
                );
                let first_row = if c.is_pack() { 4 } else { 2 };
                if sel.0 < first_row {
                    return;
                }
                let Some(l) = list.get(page * LEVELS_PER_PAGE + sel.0 - first_row) else {
                    return;
                };
                let p = c.dir.join(format!("{}.png", l.stem));
                if let Some(r) = self.edh.gfx.pic(&mut self.gfx.atlas, &p) {
                    ui.sprite(r, 630.0, 360.0, 216.0, 112.0, [255; 4]);
                }
            }
            _ => {}
        }
    }
}

impl EdHost {
    fn colour_sel(&self) -> (u8, u8) {
        self.colour
    }

    fn colour_move(&mut self, d: u8) {
        let (pw, ph) = self
            .gfx
            .palette
            .as_ref()
            .map(|p| (p.width as i32, p.height as i32))
            .unwrap_or((7, 5));
        let (dx, dy) = baba_sim::dir_delta(d);
        self.colour = (
            (self.colour.0 as i32 + dx).clamp(0, pw - 1) as u8,
            (self.colour.1 as i32 + dy).clamp(0, ph - 1) as u8,
        );
    }
}

/// The shortcut list's language keys (`§15`).
fn hotkey_keys() -> Vec<&'static str> {
    vec![
        "hotkeys_editor_category_editor",
        "hotkeys_editor_rotate",
        "hotkeys_editor_copy",
        "hotkeys_editor_cut",
        "hotkeys_editor_cleartile",
        "hotkeys_editor_drag",
        "hotkeys_editor_quickbar_scroll",
        "hotkeys_editor_quickbar_lock",
        "hotkeys_editor_quickbar_reset",
        "hotkeys_editor_swap",
        "hotkeys_editor_changetool",
        "hotkeys_editor_changelayer",
        "hotkeys_editor_pickis",
        "hotkeys_editor_pickand",
        "hotkeys_editor_picknot",
        "hotkeys_editor_pickempty",
        "hotkeys_editor_massdir",
        "hotkeys_editor_moveall",
        "hotkeys_editor_rotateselection",
        "hotkeys_editor_flipselection",
        "hotkeys_editor_altselection",
        "hotkeys_editor_opencurrobjlist",
        "hotkeys_editor_openeditormenu",
        "hotkeys_editor_openeditorsettingsmenu",
        "hotkeys_editor_testlevel",
        "hotkeys_editor_undo",
        "hotkeys_editor_save",
        "hotkeys_editor_deleteall",
        "hotkeys_editor_addobject",
        "hotkeys_editor_return",
        "hotkeys_editor_editlevel",
        "hotkeys_editor_quickaddlevel",
        "hotkeys_editor_autopick",
        "hotkeys_editor_category_currobjlist",
        "hotkeys_currobjlist_swap",
        "hotkeys_currobjlist_drag",
        "hotkeys_currobjlist_leave",
        "hotkeys_currobjlist_edit",
        "hotkeys_currobjlist_remove",
        "hotkeys_currobjlist_search",
        "hotkeys_currobjlist_autoadd",
        "hotkeys_objlist_quickadd",
        "hotkeys_editor_category_levellist",
        "hotkeys_levellist_search",
        "hotkeys_levellist_leave",
        "hotkeys_levellist_delete",
    ]
}

/// The static name of an editor action.
fn a_static(a: &str) -> &'static str {
    pad::EDITOR_ACTIONS
        .iter()
        .copied()
        .find(|x| *x == a)
        .unwrap_or("place")
}
