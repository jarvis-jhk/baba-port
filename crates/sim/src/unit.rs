//! Units: object instances on the grid (`docs/spec/turn.md` §1.1).

use std::sync::Arc;

use crate::defs::{KindId, NameId};

/// Runtime handle of a unit: unique while the unit is alive, not persistent
/// across undo (a restored unit gets a new handle).
pub type Handle = u32;

/// Level data carried by level icons (and objects acting as icons).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct IconData {
    /// Target level file stem (empty = none).
    pub file: String,
    /// Display name.
    pub name: String,
    /// Visual number / symbol index.
    pub number: i32,
    /// Visual style (-1 map picture, 0 number, 1 letter, 2 dot).
    pub style: i32,
    /// Completion state (0 hidden, 1 locked, 2 open, 3 done).
    pub completion: i32,
    /// Colour `x,y` before completion.
    pub colour: String,
    /// Colour `x,y` after completion.
    pub clear_colour: String,
    /// Shows a map.
    pub map_level: bool,
}

/// One unit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    /// Runtime handle.
    pub handle: Handle,
    /// Persistent id (≥ 1).
    pub id: i32,
    /// Object type.
    pub kind: KindId,
    /// Tile x.
    pub x: i32,
    /// Tile y.
    pub y: i32,
    /// Facing 0 right, 1 up, 2 left, 3 down.
    pub dir: u8,
    /// 0 ground, 1 FLOAT, 2 leaving (DONE).
    pub float: u8,
    /// Removed (dead units are never obstacles).
    pub dead: bool,
    /// Created or converted this turn.
    pub converted: bool,
    /// Created this turn (cleared at the visual refresh).
    pub new: bool,
    /// Name at level load, inherited through conversions (`empty` for
    /// things made from empty tiles).
    pub original: NameId,
    /// FOLLOW memory: id of the followed unit, -1 none.
    pub followed: i32,
    /// BACK start (undo history index), 0 = not backing.
    pub back_start: u32,
    /// HOLD carrier id, 0 = none.
    pub holder: i32,
    /// BROKEN applies (cached by the status pass).
    pub broken: bool,
    /// Text that is part of a valid rule (presentation).
    pub active: bool,
    /// Visible (not HIDE; presentation, and level icons for STOP/SELECT).
    pub visible: bool,
    /// Text that is part of a disabled rule: drawn with a cross
    /// (presentation).
    pub disabled: bool,
    /// Colour properties (bitmask over `fx::COLOUR_WORDS`; presentation).
    pub paint: u16,
    /// Level icon data.
    pub icon: Option<Arc<IconData>>,
}

impl Unit {
    /// Has a target level file.
    pub fn has_level_file(&self) -> bool {
        self.icon.as_ref().is_some_and(|i| !i.file.is_empty())
    }

    /// Completion state (0 for units without icon data).
    pub fn completion(&self) -> i32 {
        self.icon.as_ref().map_or(0, |i| i.completion)
    }
}

/// Direction offsets (0 right, 1 up, 2 left, 3 down; 4 none).
pub fn dir_delta(d: u8) -> (i32, i32) {
    match d {
        0 => (1, 0),
        1 => (0, -1),
        2 => (-1, 0),
        3 => (0, 1),
        _ => (0, 0),
    }
}

/// Opposite direction (none stays none).
pub fn opposite(d: u8) -> u8 {
    if d >= 4 { d } else { (d + 2) % 4 }
}

/// Direction name.
pub fn dir_name(d: u8) -> &'static str {
    match d {
        0 => "right",
        1 => "up",
        2 => "left",
        3 => "down",
        _ => "none",
    }
}
