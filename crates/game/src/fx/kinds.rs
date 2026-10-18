//! Particle kinds of the original (`effect_<name>` objects,
//! `docs/spec/presentation.md` §9): engine images from `Assets.dat` and
//! motion measured on the original. Units: level pixels at 24 px cells and
//! 60 Hz frames; a particle moves `velocity × STEP` per frame.

/// Position change per frame per unit of velocity (§9.1).
pub const STEP: f32 = 0.2;

/// How a kind's images are shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anim {
    /// The images in order, spread evenly over the particle's life.
    Seq,
    /// The first three images cycle with the global wobble; the remaining
    /// images play at the end of the life, `FADE_FRAMES` each.
    Wobble,
}

/// Frames each end image of a wobbling kind is shown.
pub const FADE_FRAMES: u16 = 4;

/// One particle kind.
#[derive(Clone, Copy, Debug)]
pub struct Kind {
    pub name: &'static str,
    /// Engine image numbers.
    pub images: &'static [u32],
    pub anim: Anim,
    /// Initial speed range (velocity units).
    pub speed: (f32, f32),
    /// Direction: angle of the spawn offset from the cell centre plus a
    /// uniform jitter of ± this many degrees (≥ 180: any direction).
    pub jitter: f32,
    /// Velocity factor per frame.
    pub decay: f32,
    /// Added to the vertical velocity per frame (positive = down).
    pub gravity: f32,
    /// Life range in frames.
    pub life: (u16, u16),
    /// Frames before the particle becomes visible.
    pub hidden: u16,
}

#[allow(clippy::too_many_arguments)]
const fn k(
    name: &'static str,
    images: &'static [u32],
    anim: Anim,
    speed: (f32, f32),
    jitter: f32,
    decay: f32,
    gravity: f32,
    life: (u16, u16),
    hidden: u16,
) -> Kind {
    Kind {
        name,
        images,
        anim,
        speed,
        jitter,
        decay,
        gravity,
        life,
        hidden,
    }
}

use Anim::{Seq, Wobble};

/// Every kind, measured on the original (see the spec's capture notes).
pub const KINDS: &[Kind] = &[
    k(
        "win",
        &[1304, 1305, 1306, 1307, 1309, 1310, 1311],
        Wobble,
        (4.1, 15.1),
        35.0,
        0.98,
        0.0,
        (41, 121),
        0,
    ),
    k(
        "unlock",
        &[1302, 1303, 1292, 1265, 1277, 1296],
        Seq,
        (2.4, 5.5),
        22.0,
        1.0,
        0.0,
        (18, 36),
        0,
    ),
    k(
        "glow",
        &[298, 570, 575, 576, 580],
        Seq,
        (3.0, 5.9),
        15.0,
        0.98,
        0.0,
        (23, 53),
        0,
    ),
    k(
        "hot",
        &[532, 548, 799, 800, 801],
        Seq,
        (0.6, 2.3),
        35.0,
        1.0,
        0.0,
        (32, 32),
        0,
    ),
    k(
        "bonus",
        &[1302, 1303, 1292, 1296],
        Wobble,
        (0.6, 2.9),
        40.0,
        1.0,
        0.0,
        (66, 142),
        0,
    ),
    k(
        "wonder",
        &[1468, 1469, 1470, 1471, 1472, 1473],
        Wobble,
        (2.0, 6.4),
        180.0,
        1.0,
        0.0,
        (40, 53),
        2,
    ),
    k(
        "tear",
        &[2229, 2234, 2236, 2235],
        Wobble,
        (3.2, 5.8),
        10.0,
        0.98,
        0.2,
        (42, 54),
        0,
    ),
    k(
        "sleep",
        &[1279, 1280, 1281, 1064, 1065],
        Wobble,
        (1.0, 3.5),
        180.0,
        0.98,
        -0.027,
        (120, 290),
        2,
    ),
    k(
        "error",
        &[2373, 2374, 2376, 2377, 2385, 2389],
        Seq,
        (0.2, 6.9),
        180.0,
        0.95,
        0.0,
        (29, 29),
        0,
    ),
    k(
        "pet",
        &[1854, 1854, 1854, 1296],
        Wobble,
        (0.0, 0.0),
        0.0,
        1.0,
        0.0,
        (50, 50),
        0,
    ),
    k(
        "angry",
        &[1863, 1864, 1865, 1866, 1296],
        Wobble,
        (0.9, 3.0),
        25.0,
        1.0,
        0.0,
        (57, 126),
        2,
    ),
    k(
        "happy",
        &[1853, 1856, 1858, 1859, 1870, 2000],
        Wobble,
        (0.6, 2.8),
        33.0,
        1.0,
        0.0,
        (55, 119),
        2,
    ),
    k(
        "electricity",
        &[1901, 1904, 1905, 1906, 1907, 1909, 1910],
        Wobble,
        (3.1, 5.4),
        15.0,
        1.0,
        0.0,
        (22, 40),
        2,
    ),
    k(
        "confetti",
        &[1283, 558, 1283, 1296],
        Wobble,
        (0.5, 0.5),
        180.0,
        1.0,
        0.0,
        (61, 89),
        0,
    ),
    k(
        "destroy",
        &[103, 1995, 1996, 1296],
        Seq,
        (2.7, 4.7),
        10.0,
        1.0,
        0.0,
        (21, 21),
        0,
    ),
    k(
        "smoke",
        &[701, 702, 703, 704, 705, 706, 402],
        Seq,
        (0.9, 3.3),
        40.0,
        1.0,
        0.0,
        (44, 51),
        0,
    ),
    k(
        "eat",
        &[453, 456, 457, 458, 1293],
        Seq,
        (1.05, 1.9),
        16.0,
        1.0,
        0.0,
        (26, 26),
        0,
    ),
    k(
        "music",
        &[1952, 1954, 1953, 1939, 1940],
        Wobble,
        (1.3, 2.8),
        180.0,
        0.98,
        -0.026,
        (40, 72),
        2,
    ),
    k(
        "bling",
        &[1297, 1299, 1300, 1298, 1301, 1293],
        Seq,
        (1.5, 5.2),
        30.0,
        0.95,
        0.0,
        (38, 38),
        0,
    ),
];

/// A kind by effect name (`bling` for unknown names).
pub fn kind(name: &str) -> &'static Kind {
    KINDS
        .iter()
        .find(|k| k.name == name)
        .unwrap_or(&KINDS[KINDS.len() - 1])
}

/// Continuous property effects (§9.4): property, particle kind, count,
/// 1-in-N chance (≤ 1 = always), period in frames, colour (`None`: the
/// unit's own colour), static (no jitter, follows the unit), inwards.
pub struct Continuous {
    pub prop: &'static str,
    pub kind: &'static str,
    pub count: u8,
    pub chance: u8,
    pub period: u32,
    pub colour: Option<(u8, u8)>,
    pub fixed: bool,
    pub inwards: bool,
}

const fn c(
    prop: &'static str,
    kind: &'static str,
    count: u8,
    chance: u8,
    period: u32,
    colour: Option<(u8, u8)>,
) -> Continuous {
    Continuous {
        prop,
        kind,
        count,
        chance,
        period,
        colour,
        fixed: false,
        inwards: false,
    }
}

pub const CONTINUOUS: &[Continuous] = &[
    c("win", "unlock", 1, 2, 20, Some((2, 4))),
    c("best", "unlock", 6, 30, 2, Some((2, 4))),
    c("tele", "glow", 1, 5, 20, Some((1, 4))),
    c("hot", "hot", 1, 80, 10, Some((0, 1))),
    c("bonus", "bonus", 1, 2, 20, Some((4, 1))),
    c("wonder", "wonder", 1, 10, 5, Some((0, 3))),
    c("sad", "tear", 1, 2, 20, Some((3, 2))),
    c("sleep", "sleep", 1, 2, 60, Some((3, 2))),
    c("broken", "error", 3, 10, 8, None),
    Continuous {
        prop: "pet",
        kind: "pet",
        count: 1,
        chance: 0,
        period: 50,
        colour: Some((3, 1)),
        fixed: true,
        inwards: false,
    },
    c("angry", "angry", 1, 10, 5, Some((2, 2))),
    c("happy", "happy", 1, 10, 5, Some((5, 4))),
    c("power", "electricity", 2, 5, 8, Some((2, 4))),
    c("power2", "electricity", 2, 5, 8, Some((5, 4))),
    c("power3", "electricity", 2, 5, 8, Some((4, 4))),
    Continuous {
        prop: "end",
        kind: "unlock",
        count: 1,
        chance: 0,
        period: 10,
        colour: None,
        fixed: false,
        inwards: true,
    },
];
