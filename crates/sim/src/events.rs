//! What a turn reports to presentation and game flow
//! (`docs/spec/presentation.md` §5–§9, `audio.md` §3–§4).
//!
//! Everything here is derived from the turn; none of it feeds back into the
//! simulation. The sim never draws cosmetic randomness: sound variants,
//! "1 in N" particle chances, jitter and wobble frames are rolled by the
//! presentation with its own [`CosmeticRng`](crate::rng::CosmeticRng).

/// A palette coordinate (column, row).
pub type Colour = (u8, u8);

/// Why the level was destroyed (`docs/spec/turn.md` §11.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DestroyStyle {
    /// Effect-caused (EAT LEVEL, LEVEL IS WEAK, ...).
    Plain,
    /// "infinite loop" guards.
    Infinity,
    /// "too complex" guards.
    TooComplex,
    /// LEVEL IS EMPTY.
    Empty,
    /// LEVEL IS BONUS.
    Bonus,
}

/// Cause of a unit's removal (presentation: particles / sounds).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cause {
    /// Generic destruction (block effects, bounds, ...).
    Destroyed,
    /// SINK.
    Sink,
    /// BOOM.
    Boom,
    /// WEAK (also crushed).
    Weak,
    /// MELT by HOT.
    Melt,
    /// DEFEAT.
    Defeat,
    /// OPEN/SHUT unlock.
    Unlock,
    /// EAT.
    Eaten,
    /// BONUS pickup.
    Bonus,
    /// DONE (floats away).
    Done,
    /// Converted into something else (removal effect "poof",
    /// `presentation.md` §9.7).
    Converted,
    /// Overflow trimming / level destruction / BACK replacement.
    Silent,
    /// Taken away by undo (it was created in the undone turn).
    Undo,
}

/// How a position/facing change should be shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MoveKind {
    /// A normal step or turn (tween; a move puff at the old cell when the
    /// position changed, `presentation.md` §9.3, not in 3D mode).
    Step,
    /// One FALL step.
    Fall,
    /// TELE: jump without tween.
    Teleport,
    /// BACK replay step.
    Back,
    /// Restored by undo.
    Undo,
}

/// How a unit came into being.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CreateCause {
    /// MAKE, HAS, MORE, X IS ALL and other generic creation.
    Make,
    /// Product of a unit conversion (X IS Y).
    Convert,
    /// Product of an EMPTY conversion.
    EmptyConvert,
    /// Recreated by a BACK replay.
    Back,
    /// Brought back by undo.
    Undo,
}

/// Particle kinds (`effect_<name>` objects, `presentation.md` §9).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Particle {
    /// Sparkles (rule formed, level destroyed on the map).
    Bling,
    /// TELE.
    Glow,
    /// WEAK, SINK, DEFEAT.
    Destroy,
    /// BOOM, HOT/MELT.
    Smoke,
    /// OPEN/SHUT, END, paradox.
    Unlock,
    /// EAT.
    Eat,
    /// BONUS.
    Bonus,
    /// WIN.
    Win,
    /// PLAY.
    Music,
    /// BACK replay step.
    Wonder,
    /// BACK replay blocked.
    Hot,
    /// PARTY (`presentation.md` §9.6): each particle takes a random colour
    /// of [`CONFETTI`] and random speeds (cosmetic rolls).
    Confetti,
}

/// Confetti colours (`presentation.md` §9.6).
pub const CONFETTI: [Colour; 5] = [(1, 4), (2, 2), (2, 4), (4, 1), (5, 3)];

impl Particle {
    /// Effect name (`effect_<name>`).
    pub fn name(self) -> &'static str {
        match self {
            Particle::Bling => "bling",
            Particle::Glow => "glow",
            Particle::Destroy => "destroy",
            Particle::Smoke => "smoke",
            Particle::Unlock => "unlock",
            Particle::Eat => "eat",
            Particle::Bonus => "bonus",
            Particle::Win => "win",
            Particle::Music => "music",
            Particle::Wonder => "wonder",
            Particle::Hot => "hot",
            Particle::Confetti => "confetti",
        }
    }
}

/// One particle burst request (`presentation.md` §9.1–§9.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Burst {
    /// Particle kind.
    pub kind: Particle,
    /// Cell.
    pub at: (i32, i32),
    /// Base particle count.
    pub count: u16,
    /// Effect-history multiplier in percent (100, 50 or 20; §9.2): the
    /// shown count is `count × scale / 100`.
    pub scale: u8,
    /// Palette colour.
    pub colour: Colour,
    /// 1 = always; N = shown with probability 1/N (cosmetic roll).
    pub chance: u8,
    /// Screen-shake strength to set (0 = none).
    pub shake: u8,
    /// Attached to (follows) this unit.
    pub attach: Option<i32>,
}

/// A sound family with numbered variants (`audio.md` §4.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Sound {
    /// Family (`pop`, `move`, ...; empty = silence).
    pub family: &'static str,
    /// Number of variants (1..=n appended; 0 = name used as is).
    pub variants: u8,
    /// Use the `_short` variant (same effect in the previous turn).
    pub short: bool,
}

impl Sound {
    /// A family.
    pub const fn new(family: &'static str, variants: u8) -> Sound {
        Sound {
            family,
            variants,
            short: false,
        }
    }

    /// Silence (an explicit empty slot that overrides earlier choices).
    pub const SILENCE: Sound = Sound::new("", 0);

    /// Plays nothing.
    pub fn is_silent(&self) -> bool {
        self.family.is_empty()
    }

    /// The asset name, rolling the variant with the cosmetic generator.
    pub fn name(&self, rng: &mut crate::rng::CosmeticRng) -> String {
        if self.is_silent() {
            return String::new();
        }
        let mut s = self.family.to_string();
        if self.variants > 0 {
            s.push_str(&rng.range(1, self.variants as i32).to_string());
        }
        if self.short {
            s.push_str("_short");
        }
        s
    }
}

/// Turn-slot families (`audio.md` §4.1).
pub mod sounds {
    use super::Sound;
    /// Something was destroyed.
    pub const POP: Sound = Sound::new("pop", 5);
    /// SINK.
    pub const PLOP: Sound = Sound::new("plop", 4);
    /// Unused turn family.
    pub const TURN: Sound = Sound::new("turn", 0);
    /// Something was pushed or pulled.
    pub const MOVE_HI: Sound = Sound::new("move_hi", 6);
    /// TELE.
    pub const TELE: Sound = Sound::new("tele", 4);
    /// OPEN/SHUT.
    pub const LOCK: Sound = Sound::new("lock", 4);
    /// Plain move.
    pub const MOVE: Sound = Sound::new("move", 6);
    /// HOT/MELT.
    pub const BURN: Sound = Sound::new("burn", 6);
    /// DONE.
    pub const DONE: Sound = Sound::new("done", 4);
    /// PLAY suppresses the move sound.
    pub const SILENT: Sound = Sound::new("silent", 0);
    /// A new rule formed.
    pub const RULE: Sound = Sound::new("rule", 5);
    /// Bonus collected (played directly).
    pub const BONUS: Sound = Sound::new("bonus", 0);
    /// Empty tiles became DONE (played directly).
    pub const DONE_EMPTY: Sound = Sound::new("done", 4);
    /// ALL IS DONE.
    pub const DONE_ALL: Sound = Sound::new("doneall_c", 0);
    /// Infinite loop / too complex message.
    pub const INFINITY: Sound = Sound::new("infinity", 0);
    /// Undo.
    pub const UNDO: Sound = Sound::new("undo", 5);
}

/// Display state of a text unit (`presentation.md` §5.4, §7.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextState {
    /// Not part of any valid rule (inactive colour).
    #[default]
    Inactive,
    /// Part of at least one valid rule (active colour).
    Active,
    /// Part of a rule disabled by NOT / BROKEN: drawn with a cross.
    Disabled,
}

/// One thing that happened in a turn, in execution order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A unit changed position and/or facing.
    Moved {
        /// Persistent id.
        id: i32,
        /// Old position.
        from: (i32, i32),
        /// New position.
        to: (i32, i32),
        /// Old facing.
        from_dir: u8,
        /// New facing.
        dir: u8,
        /// How to show it.
        kind: MoveKind,
    },
    /// A unit appeared.
    Created {
        /// Persistent id.
        id: i32,
        /// Unit name.
        name: String,
        /// Position.
        at: (i32, i32),
        /// Facing.
        dir: u8,
        /// Why.
        cause: CreateCause,
    },
    /// A unit was removed.
    Removed {
        /// Persistent id.
        id: i32,
        /// Unit name.
        name: String,
        /// Position.
        at: (i32, i32),
        /// Why.
        cause: Cause,
    },
    /// A conversion (`from` → `to`) at a position.
    Converted {
        /// Source id (-1 for an empty tile).
        from_id: i32,
        /// Source name (`empty` for empty tiles).
        from: String,
        /// Product id.
        to_id: i32,
        /// Target name.
        to: String,
        /// Position.
        at: (i32, i32),
    },
    /// Text units joined a rule that was not active before (5 bling
    /// particles each in the active colour, plus the rule sound unless
    /// only PLAY rules formed; `presentation.md` §7.1).
    RuleFormed {
        /// Ids of the text units.
        texts: Vec<i32>,
        /// The rule sound (none when only PLAY rules formed).
        sound: Option<Sound>,
    },
    /// A particle burst.
    Particles(Burst),
    /// A sound played immediately (not through the turn slots).
    Sound(Sound),
    /// X PLAY note: one sound per (subject, note) group with sounding
    /// units (`audio.md` §4.3; [`play_sample`], [`note_freq`]); each unit
    /// also shows one music particle.
    Play {
        /// Subject name (picks the sample).
        name: String,
        /// Note word (picks the frequency).
        note: String,
        /// Sounding unit ids.
        units: Vec<i32>,
    },
    /// The level bonus was collected.
    BonusCollected,
    /// The level was destroyed (executed at the end of the turn).
    LevelDestroyed(DestroyStyle),
    /// The level scrolled / rotated (level as an object).
    LevelMoved {
        /// New screen offset.
        offset: (i32, i32),
        /// New map direction.
        mapdir: u8,
        /// New rotation in degrees.
        rotation: i32,
    },
    /// 3D mode started, stopped or changed its camera target.
    Camera {
        /// 3D mode on after the change.
        on: bool,
        /// Target unit id (none: off or a fixed camera).
        target: Option<i32>,
    },
}

/// Requests from the level to game flow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transition {
    /// The level converts (LEVEL IS X): leave to the parent map, where the
    /// icon becomes these targets (`["revert"]` = undo a stored conversion).
    LevelConversion(Vec<String>),
}

/// What kind of input the outcome answers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TurnKind {
    /// A normal turn (or an AUTO tick).
    #[default]
    Turn,
    /// The key was only stored for the next AUTO tick (or a tick while
    /// AUTO does not hold): nothing happened.
    Stored,
    /// Undo.
    Undo,
    /// Restart.
    Restart,
}

/// Result of one input.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TurnOutcome {
    /// Events in execution order.
    pub events: Vec<Event>,
    /// What the input did.
    pub kind: TurnKind,
    /// The level was won this turn.
    pub won: bool,
    /// END reached (game ending in the base world; "ended" elsewhere).
    pub ended: bool,
    /// ALL IS DONE took effect.
    pub all_done: bool,
    /// No unit / empty / level is YOU-like (music state 1, `audio.md` §3).
    pub no_player: bool,
    /// The turn was committed as an undo step.
    pub undo_step: bool,
    /// Turn sound slot (`audio.md` §4.1).
    pub turn_sound: Option<Sound>,
    /// Removal sound slot (`audio.md` §4.1).
    pub removal_sound: Option<Sound>,
    /// Level transition requested.
    pub transition: Option<Transition>,
}

/// PLAY sample for a subject name (`audio.md` §4.3).
pub fn play_sample(name: &str) -> &'static str {
    match name {
        "baba" => "beep5",
        "box" | "arm" => "drum_kick",
        "rock" | "drum" => "drum_snare",
        "key" | "hihat" => "drum_hat",
        "keke" => "tune_blop",
        "skull" => "tune_short2",
        "piano" => "piano2",
        "guitar" => "guitar2",
        "sax" => "sax2",
        "trumpet" => "trumpet",
        "frog" => "frog",
        "fire" => "burn6",
        "lava" => "burn5",
        "crystal" | "gem" | "shell" | "vase" | "mirror" => "clink",
        "car" => "honk",
        "water" | "fish" | "bubble" | "crab" | "bottle" => "splash",
        "bunny" => "boing",
        _ => "beep",
    }
}

/// PLAY playback frequency in Hz for a note word (`audio.md` §4.3):
/// equal temperament from C = 24000·2^(3/12) (octave 5 or unsuffixed),
/// octave 4 half, octave 6 double, each value floored; flats alias the
/// sharp below with the quirks Cb = same octave's B, Fb = E, B# = C,
/// E# = F; unknown notes 24000. Presentation helper, not rules logic.
pub fn note_freq(note: &str) -> u32 {
    let (base, octave) = match note.as_bytes().last() {
        Some(c @ b'4'..=b'6') => (&note[..note.len() - 1], (c - b'0') as i32),
        _ => (note, 5),
    };
    let idx = match base {
        "c" | "bsharp" => 0,
        "csharp" | "dflat" => 1,
        "d" => 2,
        "dsharp" | "eflat" => 3,
        "e" | "fflat" => 4,
        "f" | "esharp" => 5,
        "fsharp" | "gflat" => 6,
        "g" => 7,
        "gsharp" | "aflat" => 8,
        "a" => 9,
        "asharp" | "bflat" => 10,
        "b" | "cflat" => 11,
        _ => return 24000,
    };
    let f = 24000.0 * 2f64.powf((3 + idx) as f64 / 12.0) * 2f64.powi(octave - 5);
    f.floor() as u32
}
