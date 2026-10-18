//! 3D ("vision") mode as far as it is simulation state
//! (`docs/spec/movement.md` §2.1, §19.2; `presentation.md` §8;
//! `undo.md` §2): the on flag, the camera target, the cyclic target list,
//! their undo lines and the hooks the turn pipeline calls. Rendering the
//! first-person view is presentation.

use crate::level::Level;
use crate::undo::Line;
use crate::unit::Handle;

/// What the camera follows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Cam {
    /// No target.
    #[default]
    None,
    /// The unit with this persistent id (it may no longer exist: a stale
    /// target simply has no camera mover until the targets are refreshed).
    Unit(i32),
    /// A fixed camera left behind where the last target was destroyed.
    Fixed {
        /// Tile x.
        x: i32,
        /// Tile y.
        y: i32,
        /// Facing.
        dir: u8,
    },
}

/// 3D state of a level.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Vision {
    /// 3D mode is on.
    pub on: bool,
    /// Camera target.
    pub cam: Cam,
    /// Target cycle (unit list snapshot of the 3D units when the mode or
    /// the targets were last set up; may hold duplicates).
    pub targets: Vec<Handle>,
    /// Last camera position (x, y, dir), kept up to date with the target.
    pub pos: (i32, i32, u8),
}

/// The 3D camera for presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Camera {
    /// Tile x.
    pub x: i32,
    /// Tile y.
    pub y: i32,
    /// Facing (0 right .. 3 down).
    pub dir: u8,
    /// Persistent id of the target unit (none for a fixed camera).
    pub target: Option<i32>,
}

impl Level {
    /// The 3D camera while 3D mode is on.
    pub fn camera(&self) -> Option<Camera> {
        if !self.vision.on {
            return None;
        }
        let (x, y, dir) = self.cam_pos();
        let target = match self.vision.cam {
            Cam::Unit(id) if self.find_id(id).is_some() => Some(id),
            _ => None,
        };
        Some(Camera { x, y, dir, target })
    }

    /// 3D mode is on.
    pub fn vision_on(&self) -> bool {
        self.vision.on
    }

    fn cam_pos(&self) -> (i32, i32, u8) {
        match self.vision.cam {
            Cam::Unit(id) => match self.find_id(id) {
                Some(h) => {
                    let u = self.unit(h);
                    (u.x, u.y, u.dir)
                }
                None => self.vision.pos,
            },
            Cam::Fixed { x, y, dir } => (x, y, dir),
            Cam::None => self.vision.pos,
        }
    }

    /// Unit handle of the current camera target, if it exists.
    pub(crate) fn cam_unit(&self) -> Option<Handle> {
        match self.vision.cam {
            Cam::Unit(id) => self.find_id(id),
            _ => None,
        }
    }

    fn set_cam(&mut self, cam: Cam) {
        self.vision.cam = cam;
        self.vision.pos = self.cam_pos();
    }

    /// Ids of the target list (stored in each undo record).
    pub(crate) fn vision_ids(&self) -> Vec<i32> {
        self.vision
            .targets
            .iter()
            .filter(|&&h| self.exists(h))
            .map(|&h| self.unit(h).id)
            .collect()
    }
}

/// Units that are 3D (rule order, then unit list; duplicates kept).
fn three_d_units(lvl: &Level) -> Vec<Handle> {
    lvl.units_with("3d")
}

/// Turns 3D mode on (no-op when already on): the target list is set up
/// and its first unit becomes the camera target.
pub(crate) fn start(lvl: &mut Level, record: bool) {
    if lvl.vision.on {
        return;
    }
    lvl.vision.targets = three_d_units(lvl);
    if let Some(&h) = lvl.vision.targets.first() {
        let id = lvl.unit(h).id;
        lvl.set_cam(Cam::Unit(id));
        if record {
            lvl.line(Line::StartVision {
                cam: lvl.vision.cam,
            });
            lvl.turn_changed = true;
        }
    }
    lvl.vision.on = true;
}

/// Turns 3D mode off (no-op when already off).
pub(crate) fn stop(lvl: &mut Level, record: bool) {
    if !lvl.vision.on {
        return;
    }
    lvl.vision.targets.clear();
    if record {
        let pos = lvl.cam_pos();
        lvl.line(Line::StopVision {
            cam: lvl.vision.cam,
            pos,
        });
        lvl.turn_changed = true;
    }
    lvl.vision.pos = lvl.cam_pos();
    lvl.vision.cam = Cam::None;
    lvl.vision.on = false;
}

/// Target refresh after a rule recomputation that mentions 3D, and when
/// the camera target leaves through DONE (`dying`): keep the current
/// target if it is still a 3D unit, else take the first one; with none
/// left 3D mode ends (unless the camera is fixed).
pub(crate) fn refresh_targets(lvl: &mut Level, dying: Option<Handle>) {
    lvl.vision.targets = three_d_units(lvl);
    let cur = match lvl.vision.cam {
        Cam::Unit(id) => Some(id),
        _ => None,
    };
    let idx = cur
        .and_then(|id| {
            lvl.vision
                .targets
                .iter()
                .position(|&h| lvl.unit(h).id == id && Some(h) != dying)
        })
        .unwrap_or(0);
    let mut stopping = true;
    if let Some(&h) = lvl.vision.targets.get(idx)
        && Some(h) != dying
    {
        let id = lvl.unit(h).id;
        lvl.set_cam(Cam::Unit(id));
        stopping = false;
    }
    if stopping && lvl.vision.on && !matches!(lvl.vision.cam, Cam::Fixed { .. }) {
        stop(lvl, true);
    }
}

/// The original refreshes the targets after every real rule
/// recomputation when some rule mentions 3D.
pub(crate) fn after_recompute(lvl: &mut Level) {
    if lvl.rules.index.contains_key("3d") {
        refresh_targets(lvl, None);
    }
}

/// Next camera target ("down" in 3D mode, or the target being destroyed:
/// `not_this`). Records a target line.
pub(crate) fn change_target(lvl: &mut Level, not_this: Option<Handle>) {
    let current = lvl.vision.cam;
    // None = "no entry" (index past the list), Some(None) = no targets.
    let next: Option<Option<Handle>> = if lvl.vision.targets.is_empty() {
        Some(None)
    } else {
        match lvl.cam_unit() {
            None => Some(Some(lvl.vision.targets[0])),
            Some(f) => match lvl.vision.targets.iter().position(|&t| t == f) {
                Some(i) => {
                    let j = if i + 1 >= lvl.vision.targets.len() {
                        0
                    } else {
                        i + 1
                    };
                    Some(Some(lvl.vision.targets[j]))
                }
                None => None,
            },
        }
    };
    let new = match next {
        Some(Some(h)) if Some(h) == not_this => {
            let (x, y, dir) = lvl.cam_pos();
            Cam::Fixed { x, y, dir }
        }
        None => {
            stop(lvl, true);
            Cam::None
        }
        Some(Some(h)) => {
            if lvl.alive(h) {
                Cam::Unit(lvl.unit(h).id)
            } else {
                Cam::None
            }
        }
        Some(None) => Cam::None,
    };
    lvl.set_cam(new);
    lvl.line(Line::VisionTarget { old: current, new });
    lvl.turn_changed = true;
}

/// End of the move phase: 3D starts when some unit is 3D and ends when no
/// rule makes anything 3D any more.
pub(crate) fn check(lvl: &mut Level) {
    if lvl.order.is_empty() {
        return;
    }
    if !three_d_units(lvl).is_empty() {
        start(lvl, true);
    } else if lvl.vision.on && !lvl.any_is("3d") {
        stop(lvl, true);
    }
}

/// Player-existence check (`turn.md` §4.2 step 9, `undo.md` §5,
/// `audio.md` §3): YOU, YOU2, then 3D rules (EMPTY IS 3D left out) are
/// looked at in rule order; every unit of a rule's subject is tested
/// (OFTEN/SELDOM draw), the level and empty tiles by their conditions.
/// The first unit of a 3D rule becomes the camera target if there is
/// none (or a fixed one); a player without any 3D one ends 3D mode
/// (undo line unless `undoing`). Returns whether a player exists.
pub(crate) fn find_player(lvl: &mut Level, undoing: bool) -> bool {
    use crate::conditions::Subj;
    let rules = lvl.rules.clone();
    let mut list: Vec<(usize, bool)> = Vec::new();
    for (p, three_d) in [("you", false), ("you2", false), ("3d", true)] {
        for &ri in rules.list(p) {
            let r = &rules.features[ri];
            if r.verb == "is" && r.object == p && !(three_d && r.subject == "empty") {
                list.push((ri, three_d));
            }
        }
    }
    let mut found = false;
    let mut found_3d = false;
    for (ri, three_d) in list {
        let r = &rules.features[ri];
        match r.subject.as_str() {
            "level" => {
                if lvl.holds(&r.conds, r.cid, Subj::Level) {
                    found = true;
                }
            }
            "empty" => {
                let mut any = false;
                'tiles: for y in 1..lvl.h - 1 {
                    for x in 1..lvl.w - 1 {
                        if lvl.tile_empty(x, y) && lvl.holds(&r.conds, r.cid, Subj::Empty(x, y)) {
                            any = true;
                            break 'tiles;
                        }
                    }
                }
                found |= any;
            }
            s => {
                let units: Vec<Handle> = lvl
                    .units_named(s)
                    .iter()
                    .copied()
                    .filter(|&h| lvl.holds(&r.conds, r.cid, Subj::Unit(h)))
                    .collect();
                if let Some(&first) = units.first() {
                    found = true;
                    if three_d {
                        found_3d = true;
                        if matches!(lvl.vision.cam, Cam::None | Cam::Fixed { .. }) {
                            let id = lvl.unit(first).id;
                            lvl.set_cam(Cam::Unit(id));
                        }
                    }
                }
            }
        }
    }
    if found && !found_3d {
        stop(lvl, !undoing);
    }
    found
}

/// Level start ("visionblock"): silently on or off.
pub(crate) fn level_start(lvl: &mut Level) {
    if !three_d_units(lvl).is_empty() {
        start(lvl, false);
    } else {
        stop(lvl, false);
    }
}

/// A unit leaves the unit list: drop it from the target cycle (the
/// original's removal loop skips the entry after each removed one).
pub(crate) fn forget(lvl: &mut Level, h: Handle) {
    let t = &mut lvl.vision.targets;
    let mut i = 0;
    while i < t.len() {
        if t[i] == h {
            t.remove(i);
            i += 1;
        } else {
            i += 1;
        }
    }
}

/// A destroyed unit that is the camera target hands the camera on.
pub(crate) fn on_destroy(lvl: &mut Level, h: Handle) {
    if let Cam::Unit(id) = lvl.vision.cam
        && lvl.unit(h).id == id
    {
        change_target(lvl, Some(h));
    }
}

/// The camera target leaves through DONE.
pub(crate) fn on_done(lvl: &mut Level, h: Handle) {
    if lvl.vision.on
        && let Cam::Unit(id) = lvl.vision.cam
        && lvl.unit(h).id == id
    {
        refresh_targets(lvl, Some(h));
    }
}

/// Undo of a 3D line (`undo.md` §4).
pub(crate) fn undo_line(lvl: &mut Level, l: &Line) {
    match *l {
        Line::StartVision { .. } => {
            if lvl.vision.on {
                lvl.vision.targets.clear();
                lvl.vision.pos = lvl.cam_pos();
                lvl.vision.cam = Cam::None;
                lvl.vision.on = false;
            }
        }
        Line::StopVision { cam, pos } => {
            if !lvl.vision.on {
                lvl.vision.targets.clear();
                let cam = match cam {
                    Cam::Unit(id) => match lvl.find_id(id) {
                        Some(_) => Cam::Unit(id),
                        None => Cam::None,
                    },
                    other => other,
                };
                match cam {
                    Cam::None => {
                        lvl.vision.targets = three_d_units(lvl);
                        if let Some(&h) = lvl.vision.targets.first() {
                            let id = lvl.unit(h).id;
                            lvl.set_cam(Cam::Unit(id));
                        }
                    }
                    Cam::Unit(id) => lvl.set_cam(Cam::Unit(id)),
                    Cam::Fixed { .. } => {
                        lvl.set_cam(Cam::Fixed {
                            x: pos.0,
                            y: pos.1,
                            dir: pos.2,
                        });
                    }
                }
                lvl.vision.on = true;
            }
        }
        Line::VisionTarget { old, .. } => {
            if lvl.vision.on
                && let Cam::Unit(id) = old
                && lvl.find_id(id).is_some()
            {
                lvl.set_cam(old);
            }
        }
        _ => {}
    }
}
