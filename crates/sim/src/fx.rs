//! Presentation bookkeeping of a turn (`docs/spec/presentation.md` §9.2–
//! §9.3, `audio.md` §4.1): the effect history ("pm" multipliers and
//! `_short` sounds), the two sound slots and the particle bursts of each
//! effect. Pure output: nothing here is read by the rules, hashed or
//! stored in undo records; no randomness is drawn.

use std::collections::BTreeMap;

use crate::events::{Burst, Cause, Colour, Event, MoveKind, Particle, Sound, TextState, sounds};
use crate::level::Level;
use crate::unit::{Handle, Unit};

/// Colour words in the order the visual pass applies them
/// (`presentation.md` §5.6), with their palette colours.
pub const COLOUR_WORDS: [(&str, Colour); 15] = [
    ("red", (2, 2)),
    ("orange", (2, 3)),
    ("yellow", (2, 4)),
    ("lime", (5, 3)),
    ("green", (5, 2)),
    ("cyan", (1, 4)),
    ("blue", (3, 2)),
    ("purple", (3, 1)),
    ("pink", (4, 1)),
    ("rosy", (4, 2)),
    ("black", (0, 4)),
    ("grey", (0, 1)),
    ("silver", (0, 2)),
    ("white", (0, 3)),
    ("brown", (6, 1)),
];

/// The order in which the visual pass queries a unit's colour properties
/// (it matters only for OFTEN/SELDOM draws); indexes into
/// [`COLOUR_WORDS`].
pub(crate) const UNIT_QUERY_ORDER: [usize; 15] = [0, 6, 3, 4, 2, 7, 8, 9, 10, 11, 12, 13, 14, 1, 5];

/// The same for the level's colour properties.
pub(crate) const LEVEL_QUERY_ORDER: [usize; 15] =
    [0, 6, 4, 3, 2, 7, 8, 9, 10, 11, 12, 13, 14, 1, 5];

/// Colours of a colour-word bitmask, in application order.
pub fn colours_of(mask: u16) -> Vec<Colour> {
    COLOUR_WORDS
        .iter()
        .enumerate()
        .filter(|(i, _)| mask & (1 << i) != 0)
        .map(|(_, c)| c.1)
        .collect()
}

/// Per-level presentation state.
#[derive(Clone, Debug, Default)]
pub(crate) struct Fx {
    /// Effect history in half units: 4 = used this turn, 3 = used last
    /// turn ("recent"); +2 per further use.
    history: BTreeMap<&'static str, u8>,
    /// Turn sound slot.
    pub turn_sound: Option<Sound>,
    /// Removal sound slot.
    pub removal_sound: Option<Sound>,
    /// How moves are currently shown.
    pub move_kind: Option<MoveKind>,
    /// In the move phase (movement-time variants of WEAK/EAT bursts).
    pub moving: bool,
    /// Level colour words (bitmask over [`COLOUR_WORDS`]).
    pub level_paint: u16,
}

impl Fx {
    /// Uses an effect kind: returns (count scale in percent, short sound).
    pub fn check(&mut self, id: &'static str) -> (u8, bool) {
        match self.history.get_mut(id) {
            Some(v) => {
                *v += 2;
                let short = *v % 2 == 1;
                (if *v > 10 { 20 } else { 50 }, short)
            }
            None => {
                self.history.insert(id, 4);
                (100, false)
            }
        }
    }

    /// End-of-turn decay (`turn.md` §4.2 step 3).
    pub fn decay(&mut self) {
        self.history.retain(|_, v| {
            if *v > 3 {
                *v = 3;
                true
            } else {
                false
            }
        });
    }

    /// Undo forgets the history.
    pub fn reset(&mut self) {
        self.history.clear();
    }

    /// New turn: empty sound slots.
    pub fn begin_turn(&mut self) {
        self.turn_sound = None;
        self.removal_sound = None;
        self.move_kind = None;
        self.moving = false;
    }
}

impl Level {
    /// The colour a unit is drawn in right now, ignoring blinking: its
    /// first colour property, else its definition's colour (text: active
    /// colour when part of a valid rule).
    pub fn unit_colour(&self, h: Handle) -> Colour {
        let u = self.unit(h);
        if let Some(&c) = colours_of(u.paint).first() {
            return c;
        }
        let k = self.kind(h);
        if k.is_text && u.active {
            k.active_colour
        } else {
            k.colour
        }
    }

    /// Background colours from LEVEL IS <colour> (empty: default
    /// background; several: blink; one non-black colour is shown dim,
    /// `presentation.md` §5.7).
    pub fn level_colours(&self) -> Vec<Colour> {
        colours_of(self.fx.level_paint)
    }

    pub(crate) fn text_state(&self, u: &Unit) -> TextState {
        if u.disabled {
            TextState::Disabled
        } else if u.active {
            TextState::Active
        } else {
            TextState::Inactive
        }
    }

    /// Particle burst with an effect-history kind (`None`: always ×1).
    /// Returns whether the kind's sound should be the short variant.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn burst(
        &mut self,
        kind: Particle,
        hist: Option<&'static str>,
        at: (i32, i32),
        count: u16,
        colour: Colour,
        shake: u8,
        attach: Option<i32>,
    ) -> bool {
        let (scale, short) = match hist {
            Some(id) => self.fx.check(id),
            None => (100, false),
        };
        self.events.push(Event::Particles(Burst {
            kind,
            at,
            count,
            scale,
            colour,
            chance: 1,
            shake,
            attach,
        }));
        short
    }

    /// Sets the removal slot (last assignment wins).
    pub(crate) fn removal_sound(&mut self, s: Sound, short: bool) {
        self.fx.removal_sound = Some(Sound { short, ..s });
    }

    /// Sets the turn slot (last assignment wins).
    pub(crate) fn turn_sound(&mut self, s: Sound, short: bool) {
        self.fx.turn_sound = Some(Sound { short, ..s });
    }

    /// Effects of a unit destroyed by `cause` (`presentation.md` §9.3,
    /// `audio.md` §4.1). `snap` is the unit as it was.
    pub(crate) fn fx_removed(&mut self, snap: &Unit, colour: Colour, cause: Cause) {
        let at = (snap.x, snap.y);
        let moving = self.fx.moving;
        match cause {
            Cause::Sink => {
                let s = self.burst(Particle::Destroy, Some("sink"), at, 15, colour, 3, None);
                self.removal_sound(sounds::PLOP, s);
            }
            Cause::Boom => {
                self.burst(Particle::Smoke, None, at, 4, (0, 2), 0, None);
                self.removal_sound(sounds::POP, false);
            }
            Cause::Weak => {
                let shake = if moving { 3 } else { 4 };
                let s = self.burst(Particle::Destroy, Some("weak"), at, 5, (0, 3), shake, None);
                self.removal_sound(sounds::POP, s);
            }
            Cause::Melt => {
                let s = self.burst(Particle::Smoke, Some("hot"), at, 5, (0, 1), 5, None);
                self.removal_sound(sounds::BURN, s);
            }
            Cause::Defeat => {
                let s = self.burst(Particle::Destroy, Some("defeat"), at, 5, (0, 3), 5, None);
                self.removal_sound(sounds::POP, s);
            }
            Cause::Unlock => {
                let s = self.burst(Particle::Unlock, Some("unlock"), at, 15, (2, 4), 8, None);
                self.turn_sound(sounds::LOCK, s);
            }
            Cause::Eaten => {
                let (n, shake) = if moving { (10, 3) } else { (5, 4) };
                let s = self.burst(Particle::Eat, Some("eat"), at, n, (0, 3), shake, None);
                self.removal_sound(sounds::POP, s);
            }
            Cause::Bonus => {
                self.burst(Particle::Bonus, Some("bonus"), at, 10, (4, 1), 5, None);
                self.removal_sound(Sound::SILENCE, false);
            }
            Cause::Destroyed | Cause::Done | Cause::Converted | Cause::Silent | Cause::Undo => {}
        }
    }

    /// The current move presentation.
    pub(crate) fn move_kind(&self) -> MoveKind {
        self.fx.move_kind.unwrap_or(MoveKind::Step)
    }
}
