//! The editor tutorial (`docs/spec/editor.md` §14): ten lessons of
//! numbered slides plus the first-time intro (lesson 1). Slide texts come
//! from the player's language file (`tutorial_<lesson>_<slide><part>`,
//! `_gpad` variants when a pad is in use); what each slide shows behind
//! the text and the demo edits it makes on a scratch level are ours,
//! following the slide outlines of §14.3. Every slide has Continue and
//! Skip; the last one ends the lesson.

use crate::lang::Texts;

/// Slides per lesson (`§14.3`).
pub const SLIDES: [usize; 10] = [16, 14, 16, 16, 19, 8, 10, 13, 11, 9];

/// A running lesson.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    /// 1..=10.
    pub lesson: u8,
    /// 1-based.
    pub slide: usize,
    /// Started automatically on the first editor visit.
    pub intro: bool,
}

impl Run {
    pub fn new(lesson: u8, intro: bool) -> Run {
        Run {
            lesson: lesson.clamp(1, 10),
            slide: 1,
            intro,
        }
    }

    pub fn count(&self) -> usize {
        SLIDES[self.lesson as usize - 1]
    }

    pub fn last(&self) -> bool {
        self.slide >= self.count()
    }
}

/// Language key prefix of a lesson.
pub fn prefix(lesson: u8) -> String {
    if lesson == 1 {
        "tutorial_intro".into()
    } else {
        format!("tutorial_{lesson}")
    }
}

/// The text paragraphs of a slide (pad variants preferred with a pad).
pub fn texts(t: &Texts, lesson: u8, slide: usize, pad: bool) -> Vec<String> {
    let p = prefix(lesson);
    let mut out = Vec::new();
    for part in 'a'..='h' {
        let key = format!("{p}_{slide}{part}");
        let v = if pad {
            t.opt(&format!("{key}_gpad")).or_else(|| t.opt(&key))
        } else {
            t.opt(&key)
        };
        match v {
            Some(s) => out.push(s),
            None => break,
        }
    }
    out
}

/// What is shown behind a slide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bg {
    /// As before.
    Keep,
    /// The editor start screen.
    Start,
    /// The level list.
    Levels,
    /// The main view on the demo level.
    Main,
    Palette,
    ObjList,
    Settings,
    LevelMenu,
}

/// A scripted edit on the demo level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Demo {
    /// Place an object by name (added to the palette if missing).
    Place(&'static str, i32, i32, u8),
    Erase(i32, i32),
    Line(&'static str, (i32, i32), (i32, i32)),
    Rect(&'static str, (i32, i32), (i32, i32), bool),
    /// Empty the demo level.
    Clear,
    /// Show the cursor on a cell.
    Cursor(i32, i32),
    /// Pick an object.
    Pick(&'static str),
    Layer(usize),
}

/// The script of a slide: background and demo edits.
pub fn script(lesson: u8, slide: usize) -> (Bg, &'static [Demo]) {
    use Demo::*;
    match (lesson, slide) {
        // Lesson 1: the intro (overview of making a level).
        (1, 1) => (Bg::Start, &[]),
        (1, 2) => (Bg::Start, &[]),
        (1, 3) => (Bg::Levels, &[]),
        (1, 4) | (1, 5) => (Bg::Main, &[Clear]),
        (1, 6) => (Bg::Main, &[]),
        (1, 7) => (Bg::Palette, &[]),
        (1, 8) => (
            Bg::Main,
            &[Pick("baba"), Cursor(8, 9), Place("baba", 8, 9, 0)],
        ),
        (1, 9) => (Bg::Palette, &[]),
        (1, 10) | (1, 11) => (Bg::ObjList, &[]),
        (1, 12) => (
            Bg::Main,
            &[
                Line("wall", (5, 6), (20, 6)),
                Line("wall", (5, 12), (20, 12)),
            ],
        ),
        (1, 13) => (
            Bg::Main,
            &[
                Place("text_baba", 6, 3, 0),
                Place("text_is", 7, 3, 0),
                Place("text_you", 8, 3, 0),
                Place("text_flag", 12, 3, 0),
                Place("text_is", 13, 3, 0),
                Place("text_win", 14, 3, 0),
                Place("text_wall", 6, 15, 0),
                Place("text_is", 7, 15, 0),
                Place("text_stop", 8, 15, 0),
                Place("flag", 17, 9, 0),
            ],
        ),
        (1, 14) | (1, 15) | (1, 16) => (Bg::Main, &[]),
        // Lesson 2: level editing.
        (2, 1) | (2, 2) => (Bg::Main, &[Clear]),
        (2, 3) => (
            Bg::Main,
            &[
                Cursor(10, 8),
                Place("baba", 10, 8, 0),
                Place("baba", 11, 8, 0),
            ],
        ),
        (2, 4) => (Bg::Main, &[Erase(11, 8), Pick("baba")]),
        (2, 5) => (Bg::Main, &[Place("baba", 14, 8, 0)]),
        (2, 6) => (Bg::Main, &[Erase(14, 8)]),
        (2, 8) => (Bg::Main, &[Layer(1)]),
        (2, 9) => (Bg::Main, &[Layer(0)]),
        // Lesson 3: tools.
        (3, 1) | (3, 2) => (Bg::Main, &[Clear]),
        (3, 3) => (
            Bg::Main,
            &[
                Cursor(10, 10),
                Place("baba", 10, 10, 0),
                Place("baba", 11, 10, 0),
                Place("baba", 12, 10, 0),
            ],
        ),
        (3, 4) => (Bg::Main, &[Erase(11, 10)]),
        (3, 5) => (Bg::Main, &[Line("wall", (4, 4), (16, 4))]),
        (3, 6) => (Bg::Main, &[Rect("wall", (18, 3), (26, 8), false)]),
        (3, 7) => (Bg::Main, &[Rect("rock", (20, 5), (24, 6), true)]),
        (3, 8) => (Bg::Main, &[Rect("flag", (4, 12), (9, 15), true)]),
        // Lesson 4: the palette.
        (4, 1) => (Bg::Main, &[]),
        (4, 2..=4) | (4, 6..=9) | (4, 13..=15) => (Bg::Palette, &[]),
        (4, 5) | (4, 10..=12) => (Bg::ObjList, &[]),
        // Lesson 5: level settings and menus.
        (5, 1..=11) => (Bg::Settings, &[]),
        (5, 12..=18) => (Bg::LevelMenu, &[]),
        // Everything else is text over the main view.
        (_, 1) => (Bg::Main, &[]),
        _ => (Bg::Keep, &[]),
    }
}

/// Word-wraps a paragraph to `width` characters.
/// Where a tutorial pointer arrow points (`editor.md` §14.6).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pointer {
    /// UI tiles (24 px) from the screen centre.
    At(f32, f32),
    /// At a button of the current screen (90 % of the way from the
    /// centre).
    Button(&'static str),
}

/// The pointer arrows of a slide (keyboard prompts; the gamepad variants
/// of lesson 1 point nowhere).
pub fn pointers(lesson: u8, slide: usize, pad: bool) -> &'static [Pointer] {
    use Pointer::*;
    match (lesson, slide) {
        (1, 6) | (4, 3) if !pad => &[Button("objects")],
        (1, 7) => &[At(-8.0, -3.0)],
        (1, 9) if !pad => &[Button("add")],
        (1, 11) => &[At(-8.0, -2.0)],
        (2, 7) | (3, 2) => &[At(16.0, -1.0)],
        (2, 8) => &[At(16.0, -6.5)],
        (2, 9) | (2, 10) => &[At(9.0, -8.0)],
        (2, 11) => &[At(6.0, -8.0)],
        (3, 4) => &[At(16.0, 4.5)],
        (3, 5) => &[At(16.0, -3.0)],
        (3, 7) => &[At(16.0, -1.5)],
        (3, 9) => &[At(16.0, 0.0)],
        (3, 10) => &[At(16.0, 3.0)],
        (3, 11) => &[At(16.0, 1.5)],
        (4, 4) => &[At(-8.0, -3.0)],
        (4, 5) => &[At(10.0, -7.0)],
        (4, 6) => &[At(12.0, -7.0)],
        (4, 7) => &[At(4.0, -7.0)],
        (4, 9) => &[At(-1.0, -6.0)],
        (4, 10) => &[At(-9.0, -3.0)],
        (4, 11) => &[At(6.0, -7.0)],
        (4, 12) => &[At(8.0, -7.0)],
        (4, 15) => &[At(-9.5, -3.0), At(-5.0, -0.5)],
        (5, 4) => &[At(6.0, -5.0)],
        (5, 5) => &[At(6.0, -4.0)],
        (5, 6) => &[At(6.0, -3.0)],
        (5, 7) => &[At(6.0, -2.0)],
        (5, 8) => &[At(6.0, -1.0)],
        (5, 9) => &[At(6.0, 0.0)],
        (5, 10) => &[At(6.0, 1.0)],
        (5, 11) => &[At(0.0, 3.0)],
        (5, 14) | (6, 3) => &[At(2.0, -2.0)],
        (5, 15) => &[At(2.0, -1.0)],
        (5, 17) => &[At(2.0, 2.5)],
        (5, 18) => &[At(2.0, 3.5)],
        (6, 2) => &[Button("menu")],
        (6, 7) => &[At(2.0, 1.5)],
        _ => &[],
    }
}

pub fn wrap(s: &str, width: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    for w in s.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + w.chars().count() > width {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(w);
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lessons() {
        let r = Run::new(3, false);
        assert_eq!(r.count(), 16);
        assert_eq!(prefix(1), "tutorial_intro");
        assert_eq!(prefix(10), "tutorial_10");
        assert_eq!(script(3, 5).0, Bg::Main);
        assert_eq!(wrap("aa bb cc", 5), vec!["aa bb", "cc"]);
    }
}
