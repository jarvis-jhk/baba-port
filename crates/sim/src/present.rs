//! Read-only queries for presentation (`docs/spec/presentation.md` §9.4):
//! which units and cells emit a property's continuous particles. Nothing
//! here changes state or draws randomness.

use crate::level::Level;

/// Emitters of one property's continuous effect.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EffectSources {
    /// Persistent ids of units with the property, once per matching rule
    /// (so stacked rules emit more), in rule order then unit order.
    pub units: Vec<i32>,
    /// Empty interior cells that emit (EMPTY IS X with its conditions,
    /// or every empty interior cell for LEVEL IS X).
    pub cells: Vec<(i32, i32)>,
}

impl Level {
    /// Units and empty cells that show `prop`'s continuous particles.
    pub fn effect_sources(&self, prop: &str) -> EffectSources {
        let mut out = EffectSources {
            units: self
                .units_with(prop)
                .into_iter()
                .map(|h| self.unit(h).id)
                .collect(),
            cells: Vec::new(),
        };
        let empty_rule = !self.find(Some("empty"), "is", Some(prop)).is_empty();
        let level = self.level_prop(prop);
        if !empty_rule && !level {
            return out;
        }
        for y in 1..self.h - 1 {
            for x in 1..self.w - 1 {
                if !self.tile_empty(x, y) {
                    continue;
                }
                if level || self.empty_prop(prop, x, y) {
                    out.cells.push((x, y));
                }
            }
        }
        out
    }
}
