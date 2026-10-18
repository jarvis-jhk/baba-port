//! Tool geometry (`docs/spec/editor.md` §6): line, rectangles, flood fill
//! and the floating selection with its rotations and mirrors.

use super::model::{EdLevel, Obj};

/// Drawing tools, in toolbar order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Pen,
    Line,
    Rect,
    FillRect,
    Select,
    Fill,
    Eraser,
}

impl Tool {
    pub const ALL: [Tool; 7] = [
        Tool::Pen,
        Tool::Line,
        Tool::Rect,
        Tool::FillRect,
        Tool::Select,
        Tool::Fill,
        Tool::Eraser,
    ];

    pub fn index(self) -> usize {
        Tool::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    /// Language key of the tooltip.
    pub fn key(self) -> &'static str {
        match self {
            Tool::Pen => "tooltip_currobjlist_tool_normal",
            Tool::Line => "tooltip_currobjlist_tool_line",
            Tool::Rect => "tooltip_currobjlist_tool_rectangle",
            Tool::FillRect => "tooltip_currobjlist_tool_fillrectangle",
            Tool::Select => "tooltip_currobjlist_tool_select",
            Tool::Fill => "tooltip_currobjlist_tool_fill",
            Tool::Eraser => "tooltip_currobjlist_tool_erase",
        }
    }

    /// Short label for the toolbar.
    pub fn label(self) -> &'static str {
        match self {
            Tool::Pen => "Pen",
            Tool::Line => "Line",
            Tool::Rect => "Rect",
            Tool::FillRect => "Fill rect",
            Tool::Select => "Select",
            Tool::Fill => "Fill",
            Tool::Eraser => "Erase",
        }
    }

    /// Next / previous tool (Shift+wheel, shoulder buttons).
    pub fn cycle(self, d: i32) -> Tool {
        let n = Tool::ALL.len() as i32;
        Tool::ALL[(self.index() as i32 + d).rem_euclid(n) as usize]
    }
}

/// Cells on the straight line from `a` to `b`, stepping one cell at a time
/// along the longer axis (`§6.3`).
pub fn line(a: (i32, i32), b: (i32, i32)) -> Vec<(i32, i32)> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let n = dx.abs().max(dy.abs());
    if n == 0 {
        return vec![a];
    }
    (0..=n)
        .map(|i| {
            // i·d/n rounded half away from zero, in integers.
            let r = |d: i32| d.signum() * ((2 * d.abs() * i + n) / (2 * n));
            (a.0 + r(dx), a.1 + r(dy))
        })
        .collect()
}

/// Cells of the rectangle spanned by `a` and `b`: outline or filled.
pub fn rect(a: (i32, i32), b: (i32, i32), filled: bool) -> Vec<(i32, i32)> {
    let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
    let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
    let mut v = Vec::new();
    for y in y0..=y1 {
        for x in x0..=x1 {
            if filled || x == x0 || x == x1 || y == y0 || y == y1 {
                v.push((x, y));
            }
        }
    }
    v
}

/// Name of what a cell holds on a layer ("" = empty).
fn name_at(l: &EdLevel, layer: usize, x: i32, y: i32) -> String {
    l.get(layer, x, y)
        .and_then(|o| l.table.by_tile(o.tile))
        .map(|d| d.name.clone())
        .unwrap_or_default()
}

/// 4-connected flood fill from `start` over cells holding the same object
/// name on `layer` (or over empty cells), inside the interior (`§6.4`).
pub fn flood(l: &EdLevel, layer: usize, start: (i32, i32)) -> Vec<(i32, i32)> {
    if !l.inside(start.0, start.1) {
        return Vec::new();
    }
    let want = name_at(l, layer, start.0, start.1);
    let (w, h) = (l.width() as i32, l.height() as i32);
    let mut seen = vec![false; (w * h) as usize];
    let mut out = Vec::new();
    let mut stack = vec![start];
    while let Some((x, y)) = stack.pop() {
        if !l.inside(x, y) || seen[(y * w + x) as usize] {
            continue;
        }
        seen[(y * w + x) as usize] = true;
        if name_at(l, layer, x, y) != want {
            continue;
        }
        out.push((x, y));
        stack.extend([(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)]);
    }
    out.sort_by_key(|&(x, y)| (y, x));
    out
}

/// A floating selection (`§6.6`): objects lifted out of one layer, kept
/// with their relative positions and facings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub w: i32,
    pub h: i32,
    /// Row-major, `w * h`.
    pub cells: Vec<Option<Obj>>,
}

impl Selection {
    pub fn get(&self, x: i32, y: i32) -> Option<Obj> {
        self.cells[(y * self.w + x) as usize]
    }

    /// Lifts the rectangle `a`–`b` of `layer` out of the level.
    pub fn lift(l: &mut EdLevel, layer: usize, a: (i32, i32), b: (i32, i32)) -> Selection {
        let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
        let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
        let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
        let mut cells = Vec::with_capacity((w * h) as usize);
        for y in y0..=y1 {
            for x in x0..=x1 {
                cells.push(l.set(layer, x, y, None));
            }
        }
        Selection { w, h, cells }
    }

    /// A one-row selection (add words by string, `§6.8`).
    pub fn row(objs: Vec<Obj>) -> Selection {
        Selection {
            w: objs.len() as i32,
            h: 1,
            cells: objs.into_iter().map(Some).collect(),
        }
    }

    /// Rotates 90° clockwise (`cw`) or counter-clockwise; facings turn too.
    pub fn rotate(&mut self, cw: bool) {
        let (w, h) = (self.w, self.h);
        let mut cells = vec![None; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let (nx, ny) = if cw { (h - 1 - y, x) } else { (y, w - 1 - x) };
                cells[(ny * h + nx) as usize] = self.get(x, y).map(|o| Obj {
                    tile: o.tile,
                    dir: if cw { (o.dir + 3) % 4 } else { (o.dir + 1) % 4 },
                });
            }
        }
        self.w = h;
        self.h = w;
        self.cells = cells;
    }

    /// Mirrors left–right (`horizontal`) or top–bottom; facings on that
    /// axis are reversed.
    pub fn mirror(&mut self, horizontal: bool) {
        let (w, h) = (self.w, self.h);
        let mut cells = vec![None; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let (nx, ny) = if horizontal {
                    (w - 1 - x, y)
                } else {
                    (x, h - 1 - y)
                };
                cells[(ny * w + nx) as usize] = self.get(x, y).map(|o| Obj {
                    tile: o.tile,
                    dir: match (horizontal, o.dir) {
                        (true, 0) => 2,
                        (true, 2) => 0,
                        (false, 1) => 3,
                        (false, 3) => 1,
                        (_, d) => d,
                    },
                });
            }
        }
        self.cells = cells;
    }

    /// Places it with its bottom-right cell at `at` (`§6.6`): occupied
    /// targets are replaced; empty selection cells erase the target unless
    /// `ignore_empty`.
    pub fn paste(&self, l: &mut EdLevel, layer: usize, at: (i32, i32), ignore_empty: bool) {
        let (ox, oy) = (at.0 - self.w + 1, at.1 - self.h + 1);
        for y in 0..self.h {
            for x in 0..self.w {
                let o = self.get(x, y);
                if o.is_none() && ignore_empty {
                    continue;
                }
                l.set(layer, ox + x, oy + y, o);
            }
        }
    }

    /// True if it holds nothing.
    pub fn is_empty(&self) -> bool {
        self.cells.iter().all(Option::is_none)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baba_formats::TileId;

    #[test]
    fn lines_and_rects() {
        assert_eq!(line((1, 1), (4, 1)), vec![(1, 1), (2, 1), (3, 1), (4, 1)]);
        assert_eq!(line((1, 1), (1, 1)), vec![(1, 1)]);
        let d = line((0, 0), (4, 2));
        assert_eq!(d.first(), Some(&(0, 0)));
        assert_eq!(d.last(), Some(&(4, 2)));
        assert_eq!(d.len(), 5);
        let back = line((4, 2), (0, 0));
        assert_eq!(back.last(), Some(&(0, 0)));
        assert_eq!(rect((1, 1), (3, 3), false).len(), 8);
        assert_eq!(rect((3, 3), (1, 1), true).len(), 9);
    }

    #[test]
    fn selection_turns() {
        let o = |t: u16, dir: u8| {
            Some(Obj {
                tile: TileId(t),
                dir,
            })
        };
        // 2 × 1: A(right) B(up)
        let mut s = Selection {
            w: 2,
            h: 1,
            cells: vec![o(1, 0), o(2, 1)],
        };
        s.rotate(true);
        assert_eq!((s.w, s.h), (1, 2));
        // Clockwise: A on top facing down, B below facing right.
        assert_eq!(s.cells, vec![o(1, 3), o(2, 0)]);
        s.rotate(false);
        assert_eq!(s.cells, vec![o(1, 0), o(2, 1)]);
        s.mirror(true);
        assert_eq!(s.cells, vec![o(2, 1), o(1, 2)]);
        s.mirror(false);
        assert_eq!(s.cells, vec![o(2, 3), o(1, 2)]);
    }
}
