//! Screen layout: tiling N viewports (splitscreen) and fitting a level into
//! a viewport (letterboxing, integer-ish scaling).

/// A pixel rectangle, origin top-left.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }
}

/// How a level is scaled into its viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScaleMode {
    /// Largest integer scale that fits (at least 1, which may crop on
    /// screens smaller than the level).
    Integer,
    /// Integer scale when it fills at least `min_fill` of the fractional
    /// fit (e.g. 0.85), else the fractional fit. Down-scales are always
    /// fractional.
    IntegerIsh { min_fill: f32 },
    /// Always the exact fit (fractional).
    Fit,
}

impl Default for ScaleMode {
    fn default() -> Self {
        ScaleMode::IntegerIsh { min_fill: 0.85 }
    }
}

/// Placement of a level inside a viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    /// Where the level lands (centred; may exceed the viewport for
    /// `Integer` on tiny screens).
    pub rect: Rect,
    pub scale: f32,
    /// True when `scale` is a whole number (pixel-exact output).
    pub integer: bool,
}

/// Fits content of `cw` x `ch` pixels into `view`.
pub fn fit(view: Rect, cw: u32, ch: u32, mode: ScaleMode) -> Fit {
    let exact = (view.w as f32 / cw.max(1) as f32).min(view.h as f32 / ch.max(1) as f32);
    let int = exact.floor().max(1.0);
    let scale = match mode {
        ScaleMode::Integer => int,
        ScaleMode::Fit => exact,
        ScaleMode::IntegerIsh { min_fill } => {
            if exact >= 1.0 && int / exact >= min_fill {
                int
            } else {
                exact
            }
        }
    };
    let w = (cw as f32 * scale).round() as i32;
    let h = (ch as f32 * scale).round() as i32;
    Fit {
        rect: Rect::new(view.x + (view.w - w) / 2, view.y + (view.h - h) / 2, w, h),
        scale,
        integer: scale.fract() == 0.0,
    }
}

/// Splits a `w` x `h` screen into `n` viewports for content of aspect
/// `cw`:`ch`, choosing the column count that gives each view the largest
/// fitted scale. Cells are separated by `gap` pixels; an incomplete last
/// row is centred.
pub fn tile(w: i32, h: i32, n: usize, cw: u32, ch: u32, gap: i32) -> Vec<Rect> {
    if n == 0 {
        return Vec::new();
    }
    let mut best = (1usize, f32::MIN);
    for cols in 1..=n {
        let rows = n.div_ceil(cols);
        let cell_w = (w - gap * (cols as i32 - 1)) as f32 / cols as f32;
        let cell_h = (h - gap * (rows as i32 - 1)) as f32 / rows as f32;
        let s = (cell_w / cw.max(1) as f32).min(cell_h / ch.max(1) as f32);
        if s > best.1 + 1e-6 {
            best = (cols, s);
        }
    }
    let cols = best.0;
    let rows = n.div_ceil(cols);
    let cell_w = (w - gap * (cols as i32 - 1)) / cols as i32;
    let cell_h = (h - gap * (rows as i32 - 1)) / rows as i32;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let (r, c) = (i / cols, i % cols);
        let in_row = if r == rows - 1 { n - r * cols } else { cols };
        let row_w = in_row as i32 * cell_w + (in_row as i32 - 1) * gap;
        let x0 = (w - row_w) / 2;
        out.push(Rect::new(
            x0 + c as i32 * (cell_w + gap),
            r as i32 * (cell_h + gap),
            cell_w,
            cell_h,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_modes() {
        // A 33x18-tile level (792x432) on 1920x1080: 2.42x exact.
        let v = Rect::new(0, 0, 1920, 1080);
        let f = fit(v, 792, 432, ScaleMode::IntegerIsh { min_fill: 0.85 });
        assert!(!f.integer, "2/2.42 < 0.85 -> fractional");
        let f = fit(v, 792, 432, ScaleMode::Integer);
        assert_eq!(f.rect, Rect::new(168, 108, 1584, 864));
        // 640x480 handheld: down-scale is fractional.
        let f = fit(Rect::new(0, 0, 640, 480), 792, 432, ScaleMode::default());
        assert!((f.scale - 640.0 / 792.0).abs() < 1e-6);
        assert_eq!(f.rect.w, 640);
        // Exact multiple.
        let f = fit(Rect::new(0, 0, 1584, 900), 792, 432, ScaleMode::default());
        assert!(f.integer && f.scale == 2.0);
    }

    #[test]
    fn tiling() {
        assert_eq!(
            tile(1920, 1080, 1, 792, 432, 0),
            vec![Rect::new(0, 0, 1920, 1080)]
        );
        let two = tile(1920, 1080, 2, 792, 432, 0);
        // Wide content: two rows beat two columns.
        assert_eq!(
            two,
            vec![Rect::new(0, 0, 1920, 540), Rect::new(0, 540, 1920, 540)]
        );
        let three = tile(1920, 1080, 3, 792, 432, 4);
        assert_eq!(three.len(), 3);
        let twelve = tile(1920, 1080, 12, 792, 432, 2);
        assert_eq!(twelve.len(), 12);
        for r in &twelve {
            assert!(r.x >= 0 && r.y >= 0 && r.x + r.w <= 1920 && r.y + r.h <= 1080);
        }
    }
}
