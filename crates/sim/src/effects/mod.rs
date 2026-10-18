//! Property families that act outside movement: the status pass, the
//! block effects, the level block and the visual refresh.

pub(crate) mod block;
pub(crate) mod level;
pub(crate) mod status;

use crate::fx::{COLOUR_WORDS, LEVEL_QUERY_ORDER, UNIT_QUERY_ORDER};
use crate::level::Level;

/// Visual refresh ("effectblock", `turn.md` §4.2 step 10,
/// `presentation.md` §5.6–§5.7): level colours and LEVEL IS HIDE, then
/// per unit `new` cleared, visibility reset, colour properties (and the
/// PHANTOM render flag), then HIDE. The queries run in the original's
/// order because OFTEN/SELDOM conditions draw on a cache miss.
pub(crate) fn visual_refresh(lvl: &mut Level) {
    let mut level_hide = false;
    let mut level_paint = 0u16;
    if lvl.rules.index.contains_key("level") {
        level_hide = lvl.level_prop("hide");
        for &i in &LEVEL_QUERY_ORDER {
            if lvl.level_prop(COLOUR_WORDS[i].0) {
                level_paint |= 1 << i;
            }
        }
    }
    lvl.fx.level_paint = level_paint;
    let level_kind = lvl.defs.level_kind;
    for h in lvl.order.clone() {
        {
            let u = lvl.unit_mut(h);
            u.new = false;
            u.visible = !level_hide;
        }
        if lvl.unit(h).kind == level_kind {
            continue;
        }
        let mut paint = 0u16;
        for &i in &UNIT_QUERY_ORDER {
            if lvl.prop(h, COLOUR_WORDS[i].0) {
                paint |= 1 << i;
            }
        }
        let _phantom = lvl.prop(h, "phantom");
        lvl.unit_mut(h).paint = paint;
    }
    if !level_hide {
        for h in lvl.units_with("hide") {
            lvl.unit_mut(h).visible = false;
        }
    }
}
