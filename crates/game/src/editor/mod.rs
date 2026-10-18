//! The level editor (`docs/spec/editor.md`): the editing model and its
//! tools, independent of the app. The screens (menus, the main view,
//! the palette, the object list, pickers, the tutorial) are driven from
//! `app::editor_ui`, which owns the renderer, texts and settings.
//!
//! | module | what |
//! |---|---|
//! | [`model`] | the level: grid, metadata, palette, quickbar, files |
//! | [`tools`] | line / rectangle / flood fill geometry, the floating selection |
//! | [`undo`] | editor undo groups |
//! | [`view`] | drawing: the level scene, bars, palette and pickers |
//! | [`pad`] | gamepad binding groups `[gamepad_editor]` / `[gamepad_currobjlist]` |
//! | [`tutorial`] | the ten lessons and the first-time intro |
//! | [`files`] | level/pack/theme folders, ids, previews |

pub mod files;
pub mod model;
pub mod pad;
pub mod tools;
pub mod tutorial;
pub mod undo;
pub mod view;

use std::path::PathBuf;

use baba_formats::leveldata::{LevelLink, MapPath, Special};
use baba_formats::{ObjectTable, PalColour};

use model::{Catalogue, EdLevel, Obj, Place, QUICK};
use tools::{Selection, Tool};
use undo::Undo;

/// What the brush places (`§5.4`, extended mode only for the map ones).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Brush {
    Objects,
    Levels,
    Paths,
    Special,
    /// Sets the map cursor's start cell (`selectorX/Y`, "Set cursor").
    Cursor,
}

/// The current object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    /// Nothing picked yet.
    None,
    /// "Empty": tools erase (Q).
    Empty,
    /// A palette entry, by slot.
    Slot(String),
}

/// Editor screens drawn by the editor itself (list menus are app menus).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Screen {
    /// Level list and other menus only (no level open yet).
    Menus,
    /// The main editor view.
    Main,
    /// The object palette (Tab).
    Palette,
    /// The full object list (Add).
    ObjList,
    /// Colour picker for the edited object / icon.
    Colour(ColourFor),
    /// Sprite picker for the edited object.
    Sprites,
}

/// What a colour pick sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColourFor {
    Base,
    Active,
    Icon(usize),
    IconClear(usize),
}

/// A drag in progress on the level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stroke {
    /// Pen / eraser: last painted cell.
    Paint { last: (i32, i32), erase: bool },
    /// Line / rectangles / select: start cell.
    Span { start: (i32, i32), tool: Tool },
    /// Ctrl+drag of one object.
    Drag {
        from: (i32, i32),
        layer: usize,
        obj: Obj,
    },
}

/// Palette screen state.
#[derive(Clone, Debug, Default)]
pub struct PalView {
    /// Full view (objects and words in separate cells) instead of pairs.
    pub full: bool,
    /// In the paired view, show the words on top instead of the objects.
    pub words: bool,
    pub sel: (i32, i32),
    pub search: String,
    /// Entry being dragged (Ctrl+drag).
    pub drag: Option<usize>,
}

/// Object list state (`§5.5`).
#[derive(Clone, Debug, Default)]
pub struct ListView {
    pub page: usize,
    pub sel: usize,
    pub search: String,
    pub tags: Vec<String>,
}

/// Sprite picker state.
#[derive(Clone, Debug, Default)]
pub struct SpriteView {
    pub page: usize,
    pub sel: usize,
    pub search: String,
    /// Sprite names (root ones, then the world's), computed on open.
    pub list: Vec<(String, bool)>,
    /// Picking a map icon picture (`[icons]` entry, file stems) for the
    /// level icon being set up, instead of an object's sprite.
    pub icon: bool,
    /// Picking the picture of this `[icons]` slot (the map's icon setup).
    pub slot: Option<u32>,
}

/// Where the level list is (`§3.1`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Collection {
    /// World folder (`levels` for single levels).
    pub world: String,
    pub dir: PathBuf,
}

impl Collection {
    pub fn is_pack(&self) -> bool {
        self.world != crate::packs::LEVELS
    }
}

/// The editor.
pub struct Editor {
    pub cat: Catalogue,
    pub level: Option<EdLevel>,
    pub screen: Screen,
    /// Level list / pack being edited.
    pub coll: Option<Collection>,
    pub tool: Tool,
    pub layer: usize,
    /// Placement direction (`§4.5`).
    pub dir: u8,
    pub brush: Brush,
    pub pick: Pick,
    /// Active quickbar slot.
    pub qsel: usize,
    /// Cell under the cursor (mouse, touch or pad).
    pub cursor: (i32, i32),
    /// Pointer position in logical units, if a mouse/touch is in use.
    pub pointer: Option<(f32, f32)>,
    pub stroke: Option<Stroke>,
    pub selection: Option<Selection>,
    pub undo: Undo,
    pub unsaved: bool,
    /// F toggles; held F shows while held.
    pub show_dirs: bool,
    pub notice: Option<(String, u32)>,
    pub pal: PalView,
    pub list: ListView,
    pub sprites: SpriteView,
    /// Entry being edited (Edit object).
    pub editing: Option<usize>,
    /// Map icon / path being set up.
    pub setup_icon: Option<usize>,
    pub setup_path: Option<usize>,
    /// Modifier keys.
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// Level list search and sort (`§3.1`).
    pub level_search: String,
    pub sort_name: bool,
    pub sort_type: bool,
    /// The next level clicked in the list becomes the pack's first level
    /// / world map (`§3.1`).
    pub list_mode: ListMode,
    /// A test play is running (`§12`): upload after a win.
    pub testing: Option<TestRun>,
    pub tutorial: Option<tutorial::Run>,
    /// Gamepad state (bindings, held buttons, alt).
    pub pad: pad::PadState,
    /// Ticks since the editor opened (blinking, repeat).
    pub ticks: u64,
    /// Theme the new level should start from (`§3.2`).
    pub pending_new: Option<(String, String)>,
    /// Path object for new paths (`§13.2`).
    pub path_object: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ListMode {
    #[default]
    Edit,
    SetFirst,
    SetMap,
}

/// A running test play.
#[derive(Clone, Debug)]
pub struct TestRun {
    pub upload: bool,
    pub won: bool,
}

impl Editor {
    pub fn new(data_dir: &std::path::Path) -> Editor {
        Editor {
            cat: Catalogue::load(data_dir),
            level: None,
            screen: Screen::Menus,
            coll: None,
            tool: Tool::Pen,
            layer: 0,
            dir: 3,
            brush: Brush::Objects,
            pick: Pick::None,
            qsel: 0,
            cursor: (1, 1),
            pointer: None,
            stroke: None,
            selection: None,
            undo: Undo::default(),
            unsaved: false,
            show_dirs: false,
            notice: None,
            // The original opens the palette in the full view.
            pal: PalView {
                full: true,
                ..Default::default()
            },
            list: ListView::default(),
            sprites: SpriteView::default(),
            editing: None,
            setup_icon: None,
            setup_path: None,
            ctrl: false,
            shift: false,
            alt: false,
            level_search: String::new(),
            sort_name: false,
            sort_type: false,
            list_mode: ListMode::Edit,
            testing: None,
            tutorial: None,
            pad: pad::PadState::default(),
            ticks: 0,
            pending_new: None,
            path_object: None,
        }
    }

    /// Opens a level in the main view (`§11.4`: undo cleared).
    pub fn open_level(&mut self, l: EdLevel) {
        self.cursor = (l.width() as i32 / 2, l.height() as i32 / 2);
        self.level = Some(l);
        self.screen = Screen::Main;
        self.undo.clear();
        self.unsaved = false;
        self.selection = None;
        self.stroke = None;
        self.layer = 0;
        self.qsel = 0;
        self.brush = Brush::Objects;
        self.pick_quick(0);
        self.update_path_object();
    }

    /// The path object (`§13.2`): the palette's `line`, else its first
    /// entry.
    pub fn update_path_object(&mut self) {
        self.path_object = self.level.as_ref().and_then(|l| {
            l.palette
                .iter()
                .find(|e| e.name == "line")
                .or(l.palette.first())
                .map(|e| e.slot.clone())
        });
    }

    pub fn note(&mut self, s: impl Into<String>) {
        self.notice = Some((s.into(), 120));
    }

    /// The picked palette entry.
    pub fn picked_entry(&self) -> Option<usize> {
        let l = self.level.as_ref()?;
        match &self.pick {
            Pick::Slot(s) => l.entry_by_slot(s),
            _ => None,
        }
    }

    /// Picks the object of quickbar slot `i`.
    pub fn pick_quick(&mut self, i: usize) {
        self.qsel = i % QUICK;
        if let Some(q) = self.level.as_ref().and_then(|l| l.quick[self.qsel].clone()) {
            self.pick = Pick::Slot(q.slot);
        }
    }

    /// Picks a palette entry: it goes into the active quickbar slot unless
    /// that one is locked (then the first free unlocked slot, `§4.3`).
    pub fn pick_entry(&mut self, i: usize) {
        let Some(l) = self.level.as_mut() else { return };
        let Some(e) = l.palette.get(i) else { return };
        let slot = e.slot.clone();
        self.pick = Pick::Slot(slot.clone());
        if self.brush != Brush::Objects {
            return;
        }
        if let Some(k) = l
            .quick
            .iter()
            .position(|q| q.as_ref().is_some_and(|q| q.slot == slot))
        {
            self.qsel = k;
            return;
        }
        let target = if l.quick[self.qsel].as_ref().is_some_and(|q| q.locked) {
            (0..QUICK)
                .map(|d| (self.qsel + d) % QUICK)
                .find(|k| !l.quick[*k].as_ref().is_some_and(|q| q.locked))
        } else {
            Some(self.qsel)
        };
        if let Some(k) = target {
            l.quick[k] = Some(model::Quick {
                slot,
                locked: false,
            });
            self.qsel = k;
        }
    }

    /// Scrolls the quickbar (wheel, shoulders).
    pub fn scroll_quick(&mut self, d: i32) {
        let k = (self.qsel as i32 + d).rem_euclid(QUICK as i32) as usize;
        self.qsel = k;
        if let Some(q) = self.level.as_ref().and_then(|l| l.quick[k].clone()) {
            self.pick = Pick::Slot(q.slot);
        } else {
            self.pick = Pick::None;
        }
    }

    pub fn toggle_lock(&mut self, k: usize) {
        if let Some(l) = self.level.as_mut()
            && let Some(Some(q)) = l.quick.get_mut(k)
        {
            q.locked = !q.locked;
            self.unsaved = true;
        }
    }

    pub fn clear_quick(&mut self, k: usize) {
        if let Some(l) = self.level.as_mut()
            && k < QUICK
        {
            l.quick[k] = None;
            self.unsaved = true;
        }
    }

    /// Swap (`§5.7`): noun ↔ text, or the next word of a cycle.
    pub fn swap(&mut self) {
        let Some(i) = self.picked_entry() else { return };
        let Some(j) = self.level.as_ref().and_then(|l| l.swap_target(i)) else {
            return;
        };
        let slot = self.level.as_ref().map(|l| l.palette[j].slot.clone());
        if let (Some(l), Some(slot)) = (self.level.as_mut(), slot) {
            if l.quick[self.qsel]
                .as_ref()
                .is_some_and(|q| !q.locked && Some(q.slot.as_str()) != Some(slot.as_str()))
            {
                l.quick[self.qsel] = Some(model::Quick {
                    slot: slot.clone(),
                    locked: false,
                });
            }
            self.pick = Pick::Slot(slot);
        }
    }

    /// The object the pen places, as a grid object.
    fn pen_obj(&self) -> Option<Obj> {
        let l = self.level.as_ref()?;
        match &self.pick {
            Pick::Slot(s) => {
                let i = l.entry_by_slot(s)?;
                Some(Obj {
                    tile: l.tile_of(i)?,
                    dir: self.dir,
                })
            }
            _ => None,
        }
    }

    /// Erasing picked (Q, eraser tool).
    pub fn erasing(&self) -> bool {
        self.tool == Tool::Eraser || self.pick == Pick::Empty
    }

    /// Paints one cell with the pen (or erases).
    pub fn paint(&mut self, x: i32, y: i32, erase: bool) {
        let obj = if erase { None } else { self.pen_obj() };
        if !erase && obj.is_none() {
            return;
        }
        let layer = self.layer;
        match self.brush {
            Brush::Objects => {
                let Some(l) = self.level.as_mut() else { return };
                if l.get(layer, x, y) != obj {
                    l.set(layer, x, y, obj);
                    self.unsaved = true;
                }
            }
            _ if erase => self.erase_map_thing(x, y),
            _ => {}
        }
    }

    /// Applies the current tool to a set of cells (line, rectangles, fill).
    pub fn paint_cells(&mut self, cells: &[(i32, i32)]) {
        let erase = self.erasing();
        for &(x, y) in cells {
            self.paint(x, y, erase);
        }
    }

    /// Copies (picks) the object under the cursor on the current layer,
    /// with its direction (`§6.5`).
    pub fn copy_at(&mut self, x: i32, y: i32) {
        let Some(l) = self.level.as_ref() else { return };
        match l.get(self.layer, x, y) {
            Some(o) => {
                if let Some(i) = l.entry_by_tile(o.tile) {
                    self.dir = o.dir;
                    self.brush = Brush::Objects;
                    self.pick_entry(i);
                }
            }
            None => self.pick = Pick::Empty,
        }
    }

    /// Cut: copy and remove.
    pub fn cut_at(&mut self, x: i32, y: i32) {
        self.copy_at(x, y);
        if let Some(l) = self.level.as_mut()
            && l.set(self.layer, x, y, None).is_some()
        {
            self.unsaved = true;
        }
    }

    /// Clears all layers of a cell (Ctrl+middle click).
    pub fn clear_tile(&mut self, x: i32, y: i32) {
        let Some(l) = self.level.as_mut() else { return };
        for z in 0..model::LAYERS {
            if l.set(z, x, y, None).is_some() {
                self.unsaved = true;
            }
        }
    }

    /// Sets the direction of the object under the cursor? No: rotates the
    /// placement direction (arrow keys, `§4.5`).
    pub fn set_dir(&mut self, d: u8) {
        self.dir = d & 3;
    }

    pub fn move_all(&mut self, d: u8) {
        if let Some(l) = self.level.as_mut() {
            l.move_all(d);
            self.unsaved = true;
        }
    }

    // ----------------------------------------------------------- map things

    /// Index of the level icon at a cell.
    pub fn icon_at(&self, x: i32, y: i32) -> Option<usize> {
        self.level
            .as_ref()?
            .info
            .levels
            .iter()
            .position(|l| (l.x, l.y) == (x, y))
    }

    pub fn path_at(&self, x: i32, y: i32) -> Option<usize> {
        self.level
            .as_ref()?
            .info
            .paths
            .iter()
            .position(|p| (p.x, p.y) == (x, y))
    }

    pub fn special_at(&self, x: i32, y: i32) -> Option<usize> {
        self.level
            .as_ref()?
            .info
            .specials
            .iter()
            .position(|s| (s.x, s.y) == (x, y))
    }

    /// Places a level icon (`§13.1`). Returns its index and whether it is
    /// new (an existing icon opens its setup instead).
    pub fn place_icon(&mut self, x: i32, y: i32) -> Option<(usize, bool)> {
        if let Some(i) = self.icon_at(x, y) {
            return Some((i, false));
        }
        let dir = self.dir;
        let l = self.level.as_mut()?;
        if !l.inside(x, y) {
            return None;
        }
        l.info.levels.push(LevelLink {
            name: String::new(),
            file: String::new(),
            colour: Some(PalColour::new(1, 2)),
            clear_colour: Some(PalColour::new(1, 3)),
            x,
            y,
            z: 3,
            state: 1,
            number: 0,
            style: 0,
            dir: dir as i32,
        });
        self.unsaved = true;
        Some((l.info.levels.len() - 1, true))
    }

    /// Places a path (`§13.2`).
    pub fn place_path(&mut self, x: i32, y: i32) -> Option<(usize, bool)> {
        if let Some(i) = self.path_at(x, y) {
            return Some((i, false));
        }
        let dir = self.dir;
        let obj = self.path_object.clone().or_else(|| match &self.pick {
            Pick::Slot(s) => Some(s.clone()),
            _ => None,
        })?;
        let l = self.level.as_mut()?;
        if !l.inside(x, y) {
            return None;
        }
        l.info.paths.push(MapPath {
            object: obj,
            x,
            y,
            z: 0,
            dir: dir as i32,
            style: 1,
            gate: 0,
            requirement: 0,
            unlock: String::new(),
        });
        self.unsaved = true;
        Some((l.info.paths.len() - 1, true))
    }

    /// Places a special marker (`§13.3`).
    pub fn place_special(&mut self, x: i32, y: i32) -> Option<(usize, bool)> {
        if let Some(i) = self.special_at(x, y) {
            return Some((i, false));
        }
        let l = self.level.as_mut()?;
        if !l.inside(x, y) {
            return None;
        }
        l.info.specials.push(Special {
            x,
            y,
            z: 4,
            data: String::new(),
        });
        self.unsaved = true;
        Some((l.info.specials.len() - 1, true))
    }

    /// Erases the brush's map thing at a cell.
    pub fn erase_map_thing(&mut self, x: i32, y: i32) {
        let brush = self.brush;
        let Some(l) = self.level.as_mut() else { return };
        let n = (
            l.info.levels.len(),
            l.info.paths.len(),
            l.info.specials.len(),
        );
        match brush {
            Brush::Levels => l.info.levels.retain(|i| (i.x, i.y) != (x, y)),
            Brush::Paths => l.info.paths.retain(|p| (p.x, p.y) != (x, y)),
            Brush::Special => l.info.specials.retain(|s| (s.x, s.y) != (x, y)),
            Brush::Cursor => {
                if l.info.general.selector == Some((x, y)) {
                    l.info.general.selector = None;
                }
            }
            Brush::Objects => {}
        }
        if n != (
            l.info.levels.len(),
            l.info.paths.len(),
            l.info.specials.len(),
        ) {
            self.unsaved = true;
        }
    }

    // ------------------------------------------------------------- strokes

    /// Pointer / pad press on a level cell with the main button.
    pub fn press(&mut self, base: &ObjectTable, x: i32, y: i32) -> Press {
        let _ = base;
        if !self.level.as_ref().is_some_and(|l| l.inside(x, y)) {
            return Press::None;
        }
        if let Some(sel) = self.selection.clone() {
            self.undo_begin();
            let layer = self.layer;
            let ignore = self.shift;
            if let Some(l) = self.level.as_mut() {
                sel.paste(l, layer, (x, y), ignore);
            }
            self.unsaved = true;
            self.undo_commit();
            return Press::Done;
        }
        if self.ctrl {
            if let Some(o) = self.level.as_ref().and_then(|l| l.get(self.layer, x, y)) {
                self.undo_begin();
                self.stroke = Some(Stroke::Drag {
                    from: (x, y),
                    layer: self.layer,
                    obj: o,
                });
            }
            return Press::Done;
        }
        if self.shift && self.brush == Brush::Objects {
            self.copy_at(x, y);
            return Press::Done;
        }
        match self.brush {
            Brush::Levels if !self.erasing() => {
                self.undo_begin();
                let r = self.place_icon(x, y);
                self.undo_commit();
                return match r {
                    Some((i, true)) if self.shift => {
                        let _ = i;
                        Press::Done
                    }
                    Some((i, _)) => Press::IconSetup(i),
                    None => Press::None,
                };
            }
            Brush::Paths if !self.erasing() => {
                self.undo_begin();
                let r = self.place_path(x, y);
                self.undo_commit();
                return match r {
                    Some((_, true)) => Press::Done,
                    Some((i, false)) => Press::PathSetup(i),
                    None => Press::None,
                };
            }
            Brush::Cursor if !self.erasing() => {
                if let Some(l) = self.level.as_mut()
                    && l.info.general.selector != Some((x, y))
                {
                    l.info.general.selector = Some((x, y));
                    self.unsaved = true;
                }
                return Press::Done;
            }
            Brush::Special if !self.erasing() => {
                self.undo_begin();
                let r = self.place_special(x, y);
                self.undo_commit();
                return match r {
                    Some((i, _)) => Press::SpecialSetup(i),
                    None => Press::None,
                };
            }
            _ => {}
        }
        self.undo_begin();
        match self.tool {
            Tool::Pen | Tool::Eraser => {
                let erase = self.erasing();
                self.paint(x, y, erase);
                self.stroke = Some(Stroke::Paint {
                    last: (x, y),
                    erase,
                });
            }
            Tool::Fill => {
                let cells = match &self.level {
                    Some(l) => tools::flood(l, self.layer, (x, y)),
                    None => Vec::new(),
                };
                self.paint_cells(&cells);
                self.undo_commit();
            }
            t => {
                self.stroke = Some(Stroke::Span {
                    start: (x, y),
                    tool: t,
                });
            }
        }
        Press::Done
    }

    /// Pointer moved onto a new cell while pressed.
    pub fn drag_to(&mut self, x: i32, y: i32) {
        if let Some(Stroke::Paint { last, erase }) = self.stroke.clone() {
            for (cx, cy) in tools::line(last, (x, y)) {
                if self.level.as_ref().is_some_and(|l| l.inside(cx, cy)) {
                    self.paint(cx, cy, erase);
                }
            }
            self.stroke = Some(Stroke::Paint {
                last: (x, y),
                erase,
            });
        }
    }

    /// Main button released: finishes spans, drags and strokes.
    pub fn release(&mut self, x: i32, y: i32) {
        let Some(s) = self.stroke.take() else {
            self.undo_commit();
            return;
        };
        let clamp = |l: &EdLevel, (x, y): (i32, i32)| {
            (
                x.clamp(1, l.width() as i32 - 2),
                y.clamp(1, l.height() as i32 - 2),
            )
        };
        match s {
            Stroke::Paint { .. } => {}
            Stroke::Span { start, tool } => {
                let Some(l) = self.level.as_ref() else { return };
                let end = clamp(l, (x, y));
                match tool {
                    Tool::Line => self.paint_cells(&tools::line(start, end)),
                    Tool::Rect => self.paint_cells(&tools::rect(start, end, false)),
                    Tool::FillRect => self.paint_cells(&tools::rect(start, end, true)),
                    Tool::Select => {
                        let layer = self.layer;
                        if let Some(l) = self.level.as_mut() {
                            let sel = Selection::lift(l, layer, start, end);
                            if !sel.is_empty() {
                                self.selection = Some(sel);
                                self.unsaved = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
            Stroke::Drag { from, layer, obj } => {
                let target_layer = self.layer;
                if let Some(l) = self.level.as_mut()
                    && l.inside(x, y)
                    && (x, y) != from
                {
                    l.set(layer, from.0, from.1, None);
                    l.set(target_layer, x, y, Some(obj));
                    self.unsaved = true;
                }
            }
        }
        self.undo_commit();
    }

    /// Cells a span would touch now (preview).
    pub fn span_preview(&self) -> Vec<(i32, i32)> {
        let Some(Stroke::Span { start, tool }) = &self.stroke else {
            return Vec::new();
        };
        let end = self.cursor;
        match tool {
            Tool::Line => tools::line(*start, end),
            Tool::Rect => tools::rect(*start, end, false),
            Tool::FillRect | Tool::Select => tools::rect(*start, end, true),
            _ => Vec::new(),
        }
    }

    /// Fill preview (`§6.4`).
    pub fn fill_preview(&self) -> Vec<(i32, i32)> {
        match (&self.level, self.tool, &self.stroke, self.brush) {
            (Some(l), Tool::Fill, None, Brush::Objects) => tools::flood(l, self.layer, self.cursor),
            _ => Vec::new(),
        }
    }

    /// Drops the floating selection (`§6.6`: cancelling discards it; it
    /// was lifted out of the level as one undo group).
    pub fn cancel_selection(&mut self) {
        self.selection = None;
    }

    pub fn undo_begin(&mut self) {
        if let Some(l) = &self.level {
            self.undo.begin(l);
        }
    }

    pub fn undo_commit(&mut self) {
        if let Some(l) = &self.level {
            self.undo.commit(l);
        }
    }

    pub fn do_undo(&mut self) -> bool {
        self.stroke = None;
        // A floating selection goes with the undo (its objects return).
        self.selection = None;
        match self.level.as_mut() {
            Some(l) => {
                let r = self.undo.undo(l);
                if r {
                    self.unsaved = true;
                }
                r
            }
            None => false,
        }
    }

    /// Words typed for "add words by string" (`§6.8`): each becomes the
    /// text `text_<word>` (added to the palette if needed) and the words
    /// a one-row selection.
    pub fn autopick(&mut self, base: &ObjectTable, text: &str) -> usize {
        let mut objs = Vec::new();
        for w in text.split_whitespace() {
            let w = w.to_lowercase();
            let w = w.strip_prefix("text_").unwrap_or(&w).to_string();
            let name = format!("text_{w}");
            let Some(l) = self.level.as_mut() else { break };
            let i = match l.entry_by_name(&name) {
                Some(i) => Some(i),
                None => match self.cat.by_name(&name).filter(|o| !o.redacted) {
                    Some(o) => {
                        let id = o.index as i32;
                        l.add_object(base, &self.cat, id, false)
                            .ok()
                            .and_then(|v| v.first().copied())
                    }
                    None => None,
                },
            };
            if let Some(i) = i
                && let Some(t) = l.tile_of(i)
            {
                objs.push(Obj {
                    tile: t,
                    dir: self.dir,
                });
            }
        }
        let n = objs.len();
        if n > 0 {
            self.selection = Some(Selection::row(objs));
            self.unsaved = true;
        }
        n
    }

    /// Quick picks I / U / Y (`§6.7`), if in the palette.
    pub fn quick_pick(&mut self, name: &str) {
        if let Some(i) = self.level.as_ref().and_then(|l| l.entry_by_name(name)) {
            self.brush = Brush::Objects;
            self.pick_entry(i);
        }
    }
}

/// What a press on the level asks the app to do next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    None,
    Done,
    IconSetup(usize),
    PathSetup(usize),
    SpecialSetup(usize),
}

/// The world a level lives in (`Place`) from a collection.
pub fn place_of(c: &Collection) -> Place {
    Place {
        world: c.world.clone(),
        dir: c.dir.clone(),
    }
}
