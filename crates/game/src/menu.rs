//! Generic menus (`docs/spec/ui.md` §4): a grid of items in rows and
//! columns, a selection, an escape item, sliders driven by left/right,
//! toggles and radio items shown "on", disabled items skipped. Menus are
//! rebuilt from the game state every frame by the app (so language and
//! setting changes show at once); only the selection is kept.

use baba_progress::{Action, PadAction, Toggle, Zoom};

use crate::input::Cmd;
use crate::lang::Texts;
use crate::ui::{Align, Gfx, Ui};

/// Which menu (the stack stores these plus the selection).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuId {
    Main,
    Slots,
    Erase,
    EraseConfirm(usize),
    Packs {
        page: usize,
    },
    Settings,
    Controls,
    KeySetup,
    PadSetup,
    Language,
    Pause,
    RestartConfirm,
    /// Several levels under the cursor: `(target stem, unit id)`.
    SelectLevel(Vec<(String, i32)>),
    IntroPrompt {
        world: String,
    },
    Rename {
        slot: usize,
        text: String,
    },
    /// "Press any key..." while rebinding.
    Waiting,
    QuitConfirm,
    /// Play levels, level codes, save import, pack notices (M9).
    Ext(crate::app::extras::ExtMenu),
    /// Level editor menus (M8).
    Editor(crate::app::editor_ui::EdMenu),
    /// Online play: host, join, room browser (M7).
    Online(crate::app::online::OnMenu),
}

/// Slider identities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slider {
    Music,
    Sound,
    Delay,
}

/// What activating an item does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    None,
    Close,
    Open(MenuId),
    Continue,
    StartGame,
    Credits,
    Exit,
    PickSlot(usize),
    EraseSlot(usize),
    PickPack(String),
    PackPage(usize),
    SwitchSlot,
    Slider(Slider),
    Toggle(Toggle),
    Zoom(Zoom),
    Language(String),
    BindKey(Action, bool),
    BindPad(PadAction, bool),
    DefaultKeys,
    DefaultPad,
    TogglePad,
    DetectPad,
    Resume,
    ReturnToMap,
    Restart,
    RestartNow,
    ReturnToMenu,
    EnterLevel(String, i32),
    Intro(bool, String),
    RenameDone,
    /// M9 actions (`app::extras`).
    Ext(crate::app::extras::ExtAct),
    /// Level editor actions (`app::editor_ui`).
    Editor(crate::app::editor_ui::EdAct),
    /// Open the multiplayer join screen.
    Multiplayer,
    /// Multiplayer: this player leaves the session.
    LeaveGame,
    /// Online actions (`app::online`).
    Online(crate::app::online::OnAct),
}

#[derive(Clone, Debug)]
pub struct Item {
    pub label: String,
    pub act: Act,
    pub enabled: bool,
    /// Toggle / radio "on" (colour 4,1).
    pub on: bool,
    /// Text colour override (palette coordinates).
    pub colour: Option<(u32, u32)>,
    /// Right-hand value (slider value, key name).
    pub value: Option<String>,
    /// A tall button (44 px, rows 48 apart: the editor start screen).
    pub tall: bool,
}

impl Item {
    pub fn new(label: impl Into<String>, act: Act) -> Item {
        Item {
            label: label.into(),
            act,
            enabled: true,
            on: false,
            colour: None,
            value: None,
            tall: false,
        }
    }
    pub fn tall(mut self) -> Item {
        self.tall = true;
        self
    }
    pub fn enabled(mut self, e: bool) -> Item {
        self.enabled = e;
        self
    }
    pub fn on(mut self, on: bool) -> Item {
        self.on = on;
        self
    }
    pub fn colour(mut self, c: (u32, u32)) -> Item {
        self.colour = Some(c);
        self
    }
    pub fn value(mut self, v: impl Into<String>) -> Item {
        self.value = Some(v.into());
        self
    }
}

/// Layout family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// The title screen grid (two columns under the logo).
    Title,
    /// A centred column list with header lines (pause, settings, …).
    List,
}

/// Button rectangles (logical x, y, w, h) by (row, column).
type Layout = Vec<((usize, usize), (f32, f32, f32, f32))>;

/// A built menu.
pub struct Menu {
    pub header: Vec<String>,
    pub rows: Vec<Vec<Item>>,
    /// Extra space before these rows (logical px).
    pub gaps: Vec<(usize, f32)>,
    pub escape: Act,
    pub style: Style,
    /// Lines drawn under the buttons (pause: the rules).
    pub footer: Vec<String>,
    /// Minimum button width (logical).
    pub min_width: f32,
    pub top: f32,
    /// Extra space between the header and the buttons (logical px).
    pub header_gap: f32,
    /// Button border colour (the fill is 0.45 of it); `None`: the
    /// style's palette colour.
    pub border: Option<[u8; 4]>,
    /// Border of the escape item's button (Return / Close: (3,1) in the
    /// editor, measured).
    pub escape_border: Option<[u8; 4]>,
    /// Items without an action that carry a value are drawn as plain
    /// "label: value" lines, not buttons (the editor's settings).
    pub plain_info: bool,
}

impl Menu {
    pub fn new(style: Style) -> Menu {
        Menu {
            header: Vec::new(),
            rows: Vec::new(),
            gaps: Vec::new(),
            escape: Act::Close,
            style,
            footer: Vec::new(),
            min_width: 238.0,
            top: 40.0,
            header_gap: 0.0,
            border: None,
            escape_border: None,
            plain_info: false,
        }
    }

    pub fn row(&mut self, items: Vec<Item>) {
        self.rows.push(items);
    }

    pub fn item(&mut self, item: Item) {
        self.rows.push(vec![item]);
    }

    pub fn gap(&mut self, px: f32) {
        self.gaps.push((self.rows.len(), px));
    }

    /// Clamps a selection to an enabled item.
    pub fn fix(&self, sel: (usize, usize)) -> (usize, usize) {
        if self.rows.is_empty() {
            return (0, 0);
        }
        let r = sel.0.min(self.rows.len() - 1);
        let c = sel.1.min(self.rows[r].len().saturating_sub(1));
        if self.rows[r].get(c).is_some_and(|i| i.enabled) {
            return (r, c);
        }
        if let Some(cc) = self.rows[r].iter().position(|i| i.enabled) {
            return (r, cc);
        }
        for d in 1..self.rows.len() {
            for rr in [r + d, r.wrapping_sub(d)] {
                if let Some(row) = self.rows.get(rr)
                    && let Some(cc) = row.iter().position(|i| i.enabled)
                {
                    return (rr, cc);
                }
            }
        }
        (r, c)
    }

    pub fn selected(&self, sel: (usize, usize)) -> Option<&Item> {
        self.rows.get(sel.0)?.get(sel.1)
    }

    /// Applies a navigation command. Returns the activated act, if any,
    /// and a slider step for left/right on sliders.
    pub fn navigate(&self, sel: &mut (usize, usize), cmd: Cmd) -> Option<(Act, i32)> {
        if self.rows.is_empty() {
            return match cmd {
                Cmd::Back | Cmd::Pause => Some((self.escape.clone(), 0)),
                _ => None,
            };
        }
        *sel = self.fix(*sel);
        match cmd {
            Cmd::Dir(d @ (1 | 3)) => {
                let n = self.rows.len();
                let mut r = sel.0;
                for _ in 0..n {
                    r = if d == 1 { (r + n - 1) % n } else { (r + 1) % n };
                    if self.rows[r].iter().any(|i| i.enabled) {
                        break;
                    }
                }
                let c = sel.1.min(self.rows[r].len() - 1);
                *sel = self.fix((r, c));
                None
            }
            Cmd::Dir(d @ (0 | 2)) => {
                let row = &self.rows[sel.0];
                if let Some(it) = row.get(sel.1)
                    && matches!(it.act, Act::Slider(_))
                {
                    return Some((it.act.clone(), if d == 0 { 1 } else { -1 }));
                }
                let n = row.len();
                if n > 1 {
                    let mut c = sel.1;
                    for _ in 0..n {
                        c = if d == 2 { (c + n - 1) % n } else { (c + 1) % n };
                        if row[c].enabled {
                            break;
                        }
                    }
                    sel.1 = c;
                }
                None
            }
            Cmd::Confirm => self
                .selected(*sel)
                .filter(|i| i.enabled)
                .map(|i| (i.act.clone(), 0)),
            Cmd::Back | Cmd::Undo => Some((self.escape.clone(), 0)),
            _ => None,
        }
    }

    /// Draws the menu. `cursor` = the sprite region drawn left of the
    /// selected button (baba).
    pub fn draw(
        &self,
        ui: &mut Ui,
        g: &mut Gfx,
        texts: &Texts,
        sel: (usize, usize),
        cursor: Option<baba_render::RegionId>,
    ) {
        let sel = self.fix(sel);
        match self.style {
            Style::Title => self.draw_title(ui, g, texts, sel, cursor),
            Style::List => self.draw_list(ui, g, texts, sel, cursor),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn button(
        &self,
        ui: &mut Ui,
        g: &mut Gfx,
        texts: &Texts,
        it: &Item,
        rect: (f32, f32, f32, f32),
        selected: bool,
        border: (u32, u32),
    ) {
        let (x, y, w, h) = rect;
        if self.plain_info && it.act == Act::None && it.value.is_some() {
            // An information line: the label in yellow, the value white.
            let ty = y + (h - 16.0 * 1.2) / 2.0;
            ui.text(
                g,
                Some(texts),
                &it.label,
                x,
                ty,
                16.0,
                g.pal(2, 4),
                Align::Left,
            );
            if let Some(v) = &it.value {
                ui.text(
                    g,
                    Some(texts),
                    v,
                    x + w * 0.5,
                    ty,
                    16.0,
                    g.pal(0, 3),
                    Align::Left,
                );
            }
            return;
        }
        let escape = it.act == self.escape && it.act != Act::None;
        let b = if it.on {
            g.pal(4, 1)
        } else if !it.enabled {
            g.pal(0, 1)
        } else if let (true, Some(c)) = (escape, self.escape_border) {
            c
        } else if let Some(c) = self.border {
            c
        } else {
            g.pal(border.0, border.1)
        };
        let fill = [
            (b[0] as f32 * 0.45) as u8,
            (b[1] as f32 * 0.45) as u8,
            (b[2] as f32 * 0.45) as u8,
            255,
        ];
        let b = if selected {
            [
                b[0].saturating_add(40),
                b[1].saturating_add(40),
                b[2].saturating_add(40),
                255,
            ]
        } else {
            b
        };
        ui.frame(g, x, y, w, h, fill, b);
        let mut col = match it.colour {
            Some((cx, cy)) => g.pal(cx, cy),
            None => g.pal(0, 3),
        };
        if !it.enabled {
            col = g.pal(0, 1);
        }
        let px = 16.0;
        let ty = y + (h - px * 1.2) / 2.0;
        match &it.value {
            Some(v) => {
                ui.text(
                    g,
                    Some(texts),
                    &it.label,
                    x + 10.0,
                    ty,
                    px,
                    col,
                    Align::Left,
                );
                ui.text(g, Some(texts), v, x + w - 10.0, ty, px, col, Align::Right);
            }
            None => {
                ui.text(
                    g,
                    Some(texts),
                    &it.label,
                    x + w / 2.0,
                    ty,
                    px,
                    col,
                    Align::Center,
                );
            }
        }
    }

    fn draw_cursor(
        ui: &mut Ui,
        cursor: Option<baba_render::RegionId>,
        x: f32,
        y: f32,
        h: f32,
        g: &Gfx,
    ) {
        if let Some(r) = cursor {
            ui.sprite(r, x - 30.0, y + (h - 24.0) / 2.0, 24.0, 24.0, g.pal(0, 3));
        }
    }

    fn draw_title(
        &self,
        ui: &mut Ui,
        g: &mut Gfx,
        texts: &Texts,
        sel: (usize, usize),
        cursor: Option<baba_render::RegionId>,
    ) {
        for (rc, rect) in self.title_layout() {
            let it = &self.rows[rc.0][rc.1];
            let s = rc == sel;
            self.button(ui, g, texts, it, rect, s, (3, 2));
            if s {
                Self::draw_cursor(ui, cursor, rect.0, rect.1, rect.3, g);
            }
        }
    }

    /// Button rectangles of the title grid (logical units).
    fn title_layout(&self) -> Layout {
        // Rows of the original's PC title menu: buttons 261×32, columns
        // at 140 and 453, rows 36 apart from 296; a full-width
        // "Continue" row sits above at 248.
        let (w, h) = (261.0, 32.0);
        let mut out = Vec::new();
        let mut y = if self.rows.len() > 3 { 248.0 } else { 296.0 };
        for (r, row) in self.rows.iter().enumerate() {
            let xs: Vec<f32> = if row.len() == 1 {
                vec![427.0 - w / 2.0 + 0.5]
            } else {
                vec![140.0, 453.0]
            };
            for c in 0..row.len() {
                out.push(((r, c), (xs[c.min(xs.len() - 1)], y, w, h)));
            }
            y += if r == 0 && self.rows.len() > 3 {
                48.0
            } else {
                36.0
            };
        }
        out
    }

    /// Button rectangles of a list menu (logical units) and the y of the
    /// footer.
    fn list_layout(&self, ui: &Ui, g: &mut Gfx, texts: &Texts) -> (Layout, f32) {
        let px = 16.0;
        let mut y = self.top + 24.0 * self.header.len() as f32;
        if !self.header.is_empty() {
            y += 8.0 + self.header_gap;
        }
        // Width: widest single-column label + margins, at least min_width.
        let mut wmax = self.min_width;
        for row in &self.rows {
            if row.len() == 1 {
                let it = &row[0];
                let mut tw = ui.measure(g, Some(texts), &it.label, px) + 40.0;
                if let Some(v) = &it.value {
                    tw += ui.measure(g, Some(texts), v, px) + 30.0;
                }
                wmax = wmax.max(tw);
            }
        }
        let wmax = wmax.min(800.0);
        let mut out = Vec::new();
        for (r, row) in self.rows.iter().enumerate() {
            if let Some((_, gap)) = self.gaps.iter().find(|(i, _)| *i == r) {
                y += gap;
            }
            let tall = row.iter().any(|i| i.tall);
            let h = if tall { 44.0 } else { 22.0 };
            let n = row.len() as f32;
            let total = if n > 1.0 {
                (wmax * 2.0 + 12.0).min(800.0)
            } else {
                wmax
            };
            let w = (total - 12.0 * (n - 1.0)) / n;
            for c in 0..row.len() {
                let x = 429.0 - total / 2.0 + c as f32 * (w + 12.0);
                out.push(((r, c), (x, y, w, h)));
            }
            y += if tall { 48.0 } else { 24.0 };
        }
        (out, y + 16.0)
    }

    /// The overlay a menu is drawn (and hit-tested) on. List menus grow on
    /// screens narrower than 16:9 (4:3 and 1:1 handhelds, where the 854×480
    /// canvas leaves bands above and below): up to the full height, as far
    /// as their widest line and their last line still fit. Elsewhere `base`.
    pub fn ui_for(&self, base: &Ui, g: &mut Gfx, texts: &Texts) -> Ui {
        let mut ui = Ui {
            quads: Vec::new(),
            scale: base.scale,
            width: base.width,
            height: base.height,
            x0: base.x0,
        };
        if self.style != Style::List || base.width / base.height >= 854.0 / 480.0 - 0.01 {
            return ui;
        }
        let (rects, foot) = self.list_layout(base, g, texts);
        let mut half: f32 = 0.0;
        for (_, (x, _, w, _)) in &rects {
            // The cursor sprite sits 30 units left of a button.
            half = half.max(427.0 - x + 30.0).max(x + w - 427.0);
        }
        for line in self.header.iter().chain(&self.footer) {
            half = half.max(base.measure(g, Some(texts), line, 16.0) / 2.0);
        }
        let bottom = foot + 20.0 * self.footer.len() as f32 + 4.0;
        let mut s = (base.height / 480.0).min(base.width / 2.0 / (half + 8.0));
        if bottom > 240.0 {
            s = s.min(base.height / 2.0 / (bottom - 240.0));
        }
        if s > base.scale {
            ui.scale = s;
        }
        ui
    }

    /// The item under a screen pixel (touch, mouse) and where on it, 0..1
    /// from its left edge (sliders: left half lowers, right half raises).
    pub fn hit(
        &self,
        ui: &Ui,
        g: &mut Gfx,
        texts: &Texts,
        x: f32,
        y: f32,
    ) -> Option<((usize, usize), f32)> {
        let rects = match self.style {
            Style::Title => self.title_layout(),
            Style::List => self.list_layout(ui, g, texts).0,
        };
        // Logical coordinates; a little slack between rows for fingers.
        let lx = (x - ui.x(0.0)) / ui.scale;
        let ly = (y - ui.y(0.0)) / ui.scale;
        rects.into_iter().find_map(|(rc, (bx, by, bw, bh))| {
            (lx >= bx && lx < bx + bw && ly >= by - 1.0 && ly < by + bh + 1.0)
                .then(|| (rc, (lx - bx) / bw))
        })
    }

    fn draw_list(
        &self,
        ui: &mut Ui,
        g: &mut Gfx,
        texts: &Texts,
        sel: (usize, usize),
        cursor: Option<baba_render::RegionId>,
    ) {
        let px = 16.0;
        let mut y = self.top;
        for line in &self.header {
            ui.text(
                g,
                Some(texts),
                line,
                427.0,
                y,
                px,
                g.pal(0, 3),
                Align::Center,
            );
            y += 24.0;
        }
        let (rects, mut y) = self.list_layout(ui, g, texts);
        for (rc, rect) in rects {
            let it = &self.rows[rc.0][rc.1];
            let s = rc == sel;
            self.button(ui, g, texts, it, rect, s, (1, 3));
            if s {
                Self::draw_cursor(ui, cursor, rect.0, rect.1, rect.3, g);
            }
        }
        for line in &self.footer {
            ui.text(
                g,
                Some(texts),
                line,
                427.0,
                y,
                px,
                g.pal(0, 3),
                Align::Center,
            );
            y += 20.0;
        }
    }
}
