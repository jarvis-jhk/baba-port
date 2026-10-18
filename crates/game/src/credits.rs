//! Credits and endings (`docs/spec/ui.md` §10, `progression.md` §10): the
//! main-menu credits scroll and the end credits. After the base world's
//! END the scroll is preceded by the island and the logo credits
//! ([`crate::cutscene::EndShow`]) and carries the decorations of the
//! original (big Keke, skull, flag, key and rocket tiles) and a Baba over
//! "Thank you for playing!". All Is Done and pack endings roll straight
//! (a pack's own `credits.txt`).

use crate::cutscene::{Cue, EndShow};
use crate::lang::Texts;
use crate::ui::{Align, Gfx, Ui};

/// Which roll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// From the main menu.
    Menu,
    /// After the base world's END.
    End,
    /// After All Is Done.
    Done,
    /// After a pack's ending.
    Pack,
}

pub struct Credits {
    pub kind: Kind,
    pub lines: Vec<String>,
    pub t: u32,
    /// Scroll position in logical px.
    pub y: f32,
    pub fast: bool,
    pub finished: bool,
    /// The island and logo credits before the scroll (END).
    pub show: Option<Box<EndShow>>,
}

/// Frames of the "The End" card before the scroll (pack endings).
const CARD: u32 = 240;

impl Credits {
    pub fn new(kind: Kind, text: &str) -> Credits {
        Credits {
            kind,
            lines: text
                .lines()
                .map(|l| l.trim_end_matches('\r').to_string())
                .collect(),
            t: 0,
            y: 0.0,
            fast: false,
            finished: false,
            show: None,
        }
    }

    /// With the END show first.
    pub fn with_show(mut self, show: EndShow) -> Credits {
        self.show = Some(Box::new(show));
        self
    }

    /// The show (island, logo credits) is running.
    pub fn show_running(&self) -> bool {
        self.show.as_ref().is_some_and(|s| !s.scrolling)
    }

    fn card(&self) -> u32 {
        if self.kind == Kind::Pack { CARD } else { 0 }
    }

    /// Total scroll length (lines every 48 px plus a screen).
    fn length(&self) -> f32 {
        self.lines.len() as f32 * 48.0 + 480.0 + 160.0
    }

    /// Skips the show (Escape during it).
    pub fn skip_show(&mut self) {
        if let Some(s) = &mut self.show {
            s.scrolling = true;
        }
    }

    /// One frame (`w`×`h`: the screen); sounds and `music:` requests of
    /// the show come back.
    pub fn tick(&mut self, w: f32, h: f32) -> Vec<Cue> {
        if let Some(s) = &mut self.show
            && !s.scrolling
        {
            return s.tick(w, h);
        }
        self.t += 1;
        if self.t < self.card() {
            return Vec::new();
        }
        self.y += if self.fast { 6.0 } else { 1.0 };
        if self.y > self.length() + 180.0 {
            self.finished = true;
        }
        Vec::new()
    }

    pub fn draw(&self, ui: &mut Ui, g: &mut Gfx, texts: &Texts, wobble: u32) {
        if let Some(s) = &self.show
            && !s.scrolling
        {
            // The island / logo screens are drawn by the app in passes
            // (`EndShow::draw_back`, flowers, `draw_front`).
            s.draw_front(ui, g, wobble);
            return;
        }
        ui.fill(g, g.pal(1, 0));
        if self.t < self.card() {
            if self.t > 40 {
                ui.text(
                    g,
                    None,
                    "THE END",
                    427.0,
                    200.0,
                    48.0,
                    g.pal(0, 3),
                    Align::Center,
                );
            }
            return;
        }
        if self.show.is_some() {
            // Decorations along the scroll (`presentation.md` §15.23).
            for (name, x, y) in crate::cutscene::ending::DECORATIONS {
                let cy = 240.0 + y * 96.0 - self.y;
                if cy > -60.0 && cy < 540.0 {
                    crate::cutscene::ending::draw_tile(ui, g, name, 427.0 + x * 96.0, cy, wobble);
                }
            }
        }
        let mut y = 480.0 - self.y;
        for l in &self.lines {
            if y > -60.0 && y < 500.0 {
                ui.text(
                    g,
                    Some(texts),
                    l,
                    427.0,
                    y,
                    24.0,
                    g.pal(0, 3),
                    Align::Center,
                );
            }
            y += 48.0;
        }
        y += 160.0;
        if y > -60.0 && y < 500.0 {
            let ty = y.max(220.0);
            if self.show.is_some() {
                crate::cutscene::ending::draw_tile(ui, g, "baba_front", 427.0, ty - 70.0, wobble);
            }
            ui.text(
                g,
                Some(texts),
                &texts.get("ending"),
                427.0,
                ty,
                32.0,
                g.pal(0, 3),
                Align::Center,
            );
        }
    }
}
