//! A play session in one world and slot: the level tree, the current level
//! or map (a `baba_sim::Level`), map loading with saved states, cursor
//! placement, selecting/entering, winning and returning with the unlock
//! data, gates, level conversions on the parent map, and the timelines of
//! the return/gate animations (`docs/spec/progression.md` §3–§7, §11).

use std::path::Path;
use std::sync::Arc;

use baba_formats::leveldata::LevelInfo;
use baba_formats::{ObjectTable, PalColour};
use baba_progress::LevelTree;
use baba_progress::{Change, Counters, MapLayout, Slot, Unlock, level_id, map::Icon};
use baba_render::Atlas;
use baba_sim::{Context, IconData, Input, Level, Placed, Source, Surroundings, TurnOutcome};

use crate::fx::{FxOptions, LevelFx};
use crate::lang::Texts;
use crate::scene::LevelGfx;
use crate::session::Pending;
use crate::world::{Meta, WorldInfo};

/// What loading needs from the app.
pub struct Env<'a> {
    pub data_dir: &'a Path,
    pub objects: &'a ObjectTable,
    pub atlas: &'a mut Atlas,
}

/// The level or map being played.
pub struct Current {
    pub stem: String,
    pub info: LevelInfo,
    pub meta: Meta,
    /// Source as loaded (specials applied), before save states.
    base: Arc<Source>,
    pub level: Level,
    pub gfx: LevelGfx,
    /// Presentation effects of this view (tweens, particles, shake).
    pub fx: LevelFx,
    /// Maps: the displayed icon/path states.
    pub layout: Option<MapLayout>,
    focus: Option<String>,
    conversions: Vec<(String, Vec<String>)>,
    surroundings: Option<Surroundings>,
    /// Maps: where the cursor stays when the map is rebuilt from new save
    /// states while a player stands on it (shared save, mode 2).
    cursor_at: Option<(i32, i32)>,
}

/// `level,<target>,<style>,<number>,<state>,<cx>,<cy>,<ccx>,<ccy>`
/// specials (§3.6).
fn special_icons(info: &LevelInfo) -> Vec<(i32, i32, IconData)> {
    let mut out = Vec::new();
    for s in &info.specials {
        let f: Vec<&str> = s.data.split(',').collect();
        if f.first() != Some(&"level") || f.len() < 2 {
            continue;
        }
        let num = |i: usize| f.get(i).and_then(|v| v.trim().parse::<i32>().ok());
        let col = |i: usize| match (num(i), num(i + 1)) {
            (Some(a), Some(b)) => format!("{a},{b}"),
            _ => String::new(),
        };
        out.push((
            s.x,
            s.y,
            IconData {
                file: f[1].to_string(),
                name: String::new(),
                number: num(3).unwrap_or(0),
                style: num(2).unwrap_or(0),
                completion: num(4).unwrap_or(0),
                colour: col(5),
                clear_colour: col(7),
                map_level: false,
            },
        ));
    }
    out
}

/// The layout of a map from its `.ld` and the slot (saved states,
/// re-propagation). Gates are not evaluated here.
pub fn map_layout(stem: &str, info: &LevelInfo, slot: &Slot, world: &str) -> MapLayout {
    let extra = special_icons(info)
        .into_iter()
        .map(|(x, y, d)| Icon {
            x,
            y,
            target: d.file,
            state: d.completion.clamp(0, 3),
        })
        .collect();
    let mut l = MapLayout::from_info(stem, info, extra);
    l.apply_save(slot, world);
    l.repropagate();
    l
}

impl Current {
    /// Loads `stem` of `world`. Maps take their states from `layout`.
    pub fn load(
        env: &mut Env,
        world: &WorldInfo,
        stem: &str,
        layout: Option<MapLayout>,
        conversions: Vec<(String, Vec<String>)>,
        focus: Option<String>,
        surroundings: Option<Surroundings>,
    ) -> Result<Current, String> {
        let lvl = world.load(stem)?;
        let meta = world.meta(stem).unwrap_or_default();
        let mut src = Source::from_level(env.objects, &lvl, &world.id);
        let specials = special_icons(&lvl.info);
        if !specials.is_empty() {
            let lk = src.defs.level_kind;
            for p in &mut src.units {
                if p.kind == lk || p.icon.is_some() {
                    continue;
                }
                if let Some((_, _, d)) = specials.iter().find(|(x, y, _)| *x == p.x && *y == p.y) {
                    p.icon = Some(Arc::new(d.clone()));
                }
            }
        }
        let base = Arc::new(src);
        let layout = if lvl.info.is_map() { layout } else { None };
        let (source, ctx) = build_source(
            &base,
            layout.as_ref(),
            &conversions,
            focus.as_deref(),
            surroundings.clone(),
        );
        let level = Level::start(Arc::new(source), ctx);
        let gfx = LevelGfx::new(
            env.atlas,
            env.data_dir,
            &world.res_key(),
            env.objects,
            &lvl.info,
            &level,
        );
        let mut fx = LevelFx::new(fx_seed(&world.id, stem));
        fx.options = fx_options(&lvl.info);
        fx.style = lvl.info.general.particles.clone();
        fx.start(&level);
        Ok(Current {
            stem: stem.to_string(),
            info: lvl.info,
            meta,
            base,
            level,
            gfx,
            fx,
            layout,
            focus,
            conversions,
            surroundings,
            cursor_at: None,
        })
    }

    pub fn is_map(&self) -> bool {
        self.info.is_map()
    }

    /// Restarts the sim level from the (display) layout; used by the
    /// unlock animations (input is blocked, nothing is lost).
    pub fn rebuild(&mut self) {
        let (mut source, ctx) = build_source(
            &self.base,
            self.layout.as_ref(),
            &self.conversions,
            self.focus.as_deref(),
            self.surroundings.clone(),
        );
        if let Some((x, y)) = self.cursor_at
            && let Some(cur) = source.defs.kind_by_name("cursor")
            && let Some(c) = source.units.iter_mut().find(|p| p.kind == cur)
        {
            c.x = x;
            c.y = y;
        }
        self.level = Level::start(Arc::new(source), ctx);
    }

    /// Path states by `[paths]` index for drawing.
    pub fn path_states(&self) -> Vec<i32> {
        let n = self.info.paths.len();
        match &self.layout {
            Some(l) => (0..n).map(|i| l.path_state(i)).collect(),
            None => vec![0; n],
        }
    }

    /// Cursor units (`cursor` objects; the base rule makes them SELECT).
    pub fn cursors(&self) -> Vec<&baba_sim::Unit> {
        let defs = self.level.defs();
        self.level
            .unit_records()
            .into_iter()
            .filter(|u| !u.dead && defs.kind(u.kind).name == "cursor")
            .collect()
    }

    /// Enterable things under the cursors (§3.5): `(target, unit id)`.
    pub fn candidates(&self, world: &WorldInfo) -> Vec<(String, i32)> {
        let mut out = Vec::new();
        let defs = self.level.defs();
        let units = self.level.unit_records();
        for c in self.cursors() {
            for u in &units {
                if u.dead || u.id == c.id || (u.x, u.y) != (c.x, c.y) {
                    continue;
                }
                if defs.kind(u.kind).name == "cursor" {
                    continue;
                }
                let Some(icon) = &u.icon else { continue };
                if icon.file.is_empty() || icon.completion <= 1 || !world.exists(&icon.file) {
                    continue;
                }
                if !out.iter().any(|(t, _): &(String, i32)| *t == icon.file) {
                    out.push((icon.file.clone(), u.id));
                }
            }
        }
        out
    }

    pub fn unit(&self, id: i32) -> Option<&baba_sim::Unit> {
        self.level.unit_records().into_iter().find(|u| u.id == id)
    }

    /// Surroundings of a unit for `levelsurrounds` (§7.1).
    pub fn surroundings_of(&self, id: i32) -> Option<Surroundings> {
        let u = self.unit(id)?;
        let defs = self.level.defs();
        let offs = [
            (1, 0),
            (0, -1),
            (-1, 0),
            (0, 1),
            (1, 1),
            (1, -1),
            (-1, -1),
            (-1, 1),
            (0, 0),
        ];
        let mut cells: [Vec<String>; 9] = Default::default();
        for other in self.level.unit_records() {
            if other.dead || other.id == u.id {
                continue;
            }
            for (i, (dx, dy)) in offs.iter().enumerate() {
                if (other.x, other.y) == (u.x + dx, u.y + dy) {
                    cells[i].push(defs.kind(other.kind).name.clone());
                }
            }
        }
        Some(Surroundings { dir: u.dir, cells })
    }
}

/// Applies display states, conversions and the cursor position to a
/// source (§3.2 steps 4–8, §3.4).
fn build_source(
    base: &Source,
    layout: Option<&MapLayout>,
    conversions: &[(String, Vec<String>)],
    focus: Option<&str>,
    surroundings: Option<Surroundings>,
) -> (Source, Context) {
    let mut src = base.clone();
    let mut ctx = Context {
        surroundings,
        ..Default::default()
    };
    let Some(layout) = layout else {
        return (src, ctx);
    };
    let lk = src.defs.level_kind;
    let mut units: Vec<Placed> = Vec::new();
    for p in &src.units {
        let Some(icon) = p.icon.as_ref().filter(|i| !i.file.is_empty()) else {
            units.push(p.clone());
            continue;
        };
        let state = layout.icon_state(&icon.file).unwrap_or(icon.completion);
        if p.kind == lk && state <= 0 {
            continue; // hidden icons are not on the map
        }
        let mut d = (**icon).clone();
        d.completion = state;
        let d = Arc::new(d);
        if let Some((_, objs)) = conversions.iter().find(|(s, _)| *s == icon.file) {
            for o in objs {
                if let Some(k) = src.defs.kind_by_name(o) {
                    let mut c = (*d).clone();
                    c.completion = c.completion.max(1);
                    units.push(Placed {
                        kind: k,
                        icon: Some(Arc::new(c)),
                        ..p.clone()
                    });
                }
            }
            continue;
        }
        units.push(Placed {
            icon: Some(d),
            ..p.clone()
        });
    }
    if let Some(focus) = focus
        && let Some(cur) = src.defs.kind_by_name("cursor")
    {
        let spot = units
            .iter()
            .find(|p| {
                p.kind != cur
                    && p.icon
                        .as_ref()
                        .is_some_and(|i| i.file == focus && i.completion >= 2)
            })
            .map(|p| (p.x, p.y));
        if let Some((x, y)) = spot
            && let Some(c) = units.iter_mut().find(|p| p.kind == cur)
        {
            c.x = x;
            c.y = y;
        }
    }
    src.units = units;
    ctx.path_states = layout
        .paths
        .iter()
        .filter(|p| p.state > 0)
        .map(|p| (p.index, p.state))
        .collect();
    (src, ctx)
}

/// Opens the gates of `layout` whose counters are met (§4, §5.4). Returns
/// the layout to display first (newly opened gates still closed) and their
/// opening animation, if any; `layout` itself gets the final states.
fn gate_anim(
    layout: &mut MapLayout,
    slot: &Slot,
    world: &str,
    stem: &str,
) -> (MapLayout, Option<Anim>) {
    let counters = Counters::read(slot, world, stem);
    let opened = layout.open_gates(&counters);
    let newly: Vec<usize> = opened.iter().filter(|(_, n)| *n).map(|(i, _)| *i).collect();
    let mut display = layout.clone();
    if newly.is_empty() {
        return (display, None);
    }
    let mut unlock = Unlock::default();
    let mut gate_changes = Vec::new();
    for i in &newly {
        if let Some(p) = layout.paths.iter().find(|p| p.index == *i).cloned() {
            gate_changes.push(Change::Path {
                index: p.index,
                x: p.x,
                y: p.y,
                from: 1,
                to: 3,
            });
            let u = layout.unlock_from(p.x, p.y);
            unlock.opened.extend(u.opened);
            unlock.rings.extend(u.rings);
        }
    }
    for p in &mut display.paths {
        if newly.contains(&p.index) {
            p.state = 1;
        }
    }
    let mut anim = Anim::gates(layout.clone(), gate_changes, &unlock);
    anim.gate_kinds = anim
        .gates
        .iter()
        .map(|&(x, y)| {
            layout
                .paths
                .iter()
                .find(|p| (p.x, p.y) == (x, y) && p.gate != 0)
                .map_or((1, 1), |p| (p.gate, p.requirement))
        })
        .collect();
    (display, Some(anim))
}

/// One animation step: frame, changes to apply, sounds (name, rate).
pub type Step = (u32, Vec<Change>, Vec<(String, f32)>);

/// A scheduled map animation (§5.2–§5.4): at frame `t` apply the changes
/// and play the sounds; input is blocked until `end`.
pub struct Anim {
    pub t: u32,
    pub steps: Vec<Step>,
    pub end: u32,
    /// Banner shown from the first step ("Map clear!").
    pub banner: Option<String>,
    /// Final, saved layout (applied at the end).
    pub layout: MapLayout,
    /// Icon of the level just won (return animation effects).
    pub origin: Option<(i32, i32)>,
    /// Gates opening (gate animation effects).
    pub gates: Vec<(i32, i32)>,
    /// Gate type and requirement per opening gate (ring of symbols,
    /// particle colour).
    pub gate_kinds: Vec<(i32, i32)>,
}

impl Anim {
    /// The return-to-map sequence after a win.
    pub fn after_win(
        target: &str,
        before: &MapLayout,
        after: MapLayout,
        u: &Unlock,
        clear: bool,
        banner: Option<String>,
    ) -> Anim {
        let mut steps: Vec<Step> = Vec::new();
        steps.push((10, Vec::new(), vec![("roll".into(), 1.0)]));
        let icon = before
            .icons
            .iter()
            .find(|i| i.target == target)
            .map(|i| Change::Icon {
                target: target.to_string(),
                x: i.x,
                y: i.y,
                from: i.state,
                to: 3,
            });
        let won_sound = if clear { "clear" } else { "winnery_fast" };
        steps.push((
            70,
            icon.into_iter().collect(),
            vec![(won_sound.into(), 1.0), ("pop3".into(), 1.0)],
        ));
        let mut t = 110;
        for ring in &u.rings {
            steps.push((t, ring.clone(), vec![("whoosh_quiet1".into(), 1.0)]));
            t += 10;
        }
        // Neighbours: up, left, down, right (§5.2 130/145/160/175).
        let origin = before
            .icons
            .iter()
            .find(|i| i.target == target)
            .map(|i| (i.x, i.y))
            .unwrap_or((0, 0));
        let mut t = t.max(130);
        for (dx, dy) in [(0, -1), (-1, 0), (0, 1), (1, 0)] {
            let here: Vec<Change> = u
                .opened
                .iter()
                .filter(|c| {
                    let (x, y) = match c {
                        Change::Icon { x, y, .. } | Change::Path { x, y, .. } => (*x, *y),
                    };
                    (x, y) == (origin.0 + dx, origin.1 + dy)
                })
                .cloned()
                .collect();
            if !here.is_empty() {
                steps.push((t, here, vec![("whoosh_alt1".into(), 1.0)]));
                t += 15;
            }
        }
        Anim {
            t: 0,
            steps,
            end: t.max(180),
            banner,
            layout: after,
            origin: Some(origin),
            gates: Vec::new(),
            gate_kinds: Vec::new(),
        }
    }

    /// One map of the map-clear showcase (§5.3): roll and particles on
    /// the icon, then it completes (lowered `winnery_fast`, `pop3`).
    pub fn showcase(origin: (i32, i32), target: &str, after: MapLayout) -> Anim {
        let change = Change::Icon {
            target: target.to_string(),
            x: origin.0,
            y: origin.1,
            from: 2,
            to: 3,
        };
        Anim {
            t: 0,
            steps: vec![
                (10, Vec::new(), vec![("roll".into(), 1.0)]),
                (
                    70,
                    vec![change],
                    vec![
                        ("winnery_fast".into(), 38000.0 / 44000.0),
                        ("pop3".into(), 1.0),
                    ],
                ),
            ],
            end: 130,
            banner: None,
            layout: after,
            origin: Some(origin),
            gates: Vec::new(),
            gate_kinds: Vec::new(),
        }
    }

    /// Gates opening on map load (§5.4).
    pub fn gates(after: MapLayout, opened: Vec<Change>, u: &Unlock) -> Anim {
        let mut steps = vec![(10, Vec::new(), vec![("roll".to_string(), 1.0)])];
        let gates = opened
            .iter()
            .map(|c| match c {
                Change::Icon { x, y, .. } | Change::Path { x, y, .. } => (*x, *y),
            })
            .collect();
        steps.push((130, opened, vec![("gate".into(), 1.0)]));
        let mut t = 140;
        for ring in u.rings.iter().chain(std::iter::once(&u.opened)) {
            steps.push((t, ring.clone(), vec![("whoosh_quiet1".into(), 1.0)]));
            t += 10;
        }
        Anim {
            t: 0,
            steps,
            end: t + 20,
            banner: None,
            layout: after,
            origin: None,
            gates,
            gate_kinds: Vec::new(),
        }
    }
}

/// The session.
pub struct Play {
    pub slot: usize,
    pub world: WorldInfo,
    pub tree: LevelTree,
    pub cur: Current,
    pub anim: Option<Anim>,
    /// Frames left before leaving a won level.
    pub won: Option<u32>,
    pub banner: Option<(String, u32)>,
    /// Frames left of the title card.
    pub title: Option<u32>,
    /// Play time not yet written to the save (frames).
    pub frames: u64,
    pub log: Vec<String>,
    /// What [`crate::session::Turn::Proceed`] will do (after a win, a
    /// level conversion or an ending).
    pub pending: Option<Pending>,
    /// Shared save (mode 2), players other than the first: the level
    /// tree, `Previous` and `levelsurrounds` are this player's own and
    /// live here instead of in the shared slot (progress stays shared).
    pub own_nav: Option<Slot>,
    /// Set when one level is played on its own (custom levels, downloads;
    /// `Play::single`): wins, endings and conversions are recorded for the
    /// level and then the play ends (`session::apply`).
    pub single: Option<SingleLevel>,
}

/// A level played on its own, outside a map (`worldpacks.md` §6,
/// `levelcodes.md`).
#[derive(Clone, Debug, Default)]
pub struct SingleLevel {
    /// Level code (downloads), recorded in `[levels_won]`.
    pub code: Option<String>,
    /// The temporary download (`_TEMP_`): only `[levels_won]` is written.
    pub temp: bool,
}

/// The slot that holds navigation state (tree, previous level,
/// surroundings): the player's own one in mode 2, else the save slot.
fn nav<'a>(own: &'a mut Option<Slot>, shared: &'a mut Slot) -> &'a mut Slot {
    own.as_mut().unwrap_or(shared)
}

/// The map-clear showcase (`progression.md` §5.3): the maps of the
/// cleared map's `unlocklevels` are visited in turn, the cleared map's
/// icon switching to its cleared look on each, then the view returns.
/// Pending as [`Pending::Showcase`]; each step is a `Proceed` turn.
#[derive(Clone, Debug, Default)]
pub struct Showcase {
    /// Maps still to visit.
    pub queue: Vec<String>,
    /// The cleared map (its icon is the one shown completing).
    pub cleared: String,
}

/// What happens after a level is won.
pub enum AfterWin {
    /// Back on the parent map with the unlock animation.
    Map,
    /// The pack is beaten (`customparent=<win>` or the root).
    PackEnd,
}

impl Play {
    /// Level label suffix for the current tree.
    pub fn label(&self, texts: &Texts) -> Option<String> {
        self.tree.label_suffix().map(|s| {
            let s = s.replace("extra", &texts.get("mapid_extra"));
            format!("{} {}", texts.get("level"), s)
        })
    }

    /// Localised name of a level.
    pub fn level_name(&self, texts: &Texts, stem: &str) -> String {
        let m = self.world.meta(stem).unwrap_or_default();
        texts.level_name(&self.world.id, stem, &m.name)
    }

    /// Map HUD: the name of what the cursor is on (else the map's name).
    pub fn hud_name(&self, texts: &Texts) -> String {
        if let Some((t, _)) = self.cur.candidates(&self.world).first() {
            return self.level_name(texts, t);
        }
        self.level_name(texts, &self.cur.stem)
    }

    /// "Return to map" allowed (§7.3).
    pub fn can_return(&self) -> bool {
        let cr = &self.cur.meta.custom_return;
        if !self.world.is_base() && cr == "_NONE_" {
            return false;
        }
        self.tree.len() > 1
            || (self.world.is_base() && self.cur.stem == "200level")
            || (!self.world.is_base() && !cr.is_empty())
    }

    /// Loads a map with saved states; opens gates whose counters are met.
    /// Returns the current level and a gate animation, if any.
    pub fn load_map(
        env: &mut Env,
        world: &WorldInfo,
        slot: &mut Slot,
        stem: &str,
        focus: Option<String>,
    ) -> Result<(Current, Option<Anim>), String> {
        let info = world.load(stem)?.info;
        let mut layout = map_layout(stem, &info, slot, &world.id);
        let (display, anim) = gate_anim(&mut layout, slot, &world.id, stem);
        layout.write(slot, &world.id);
        let conv = slot.conversions(&world.id, stem);
        let cur = Current::load(env, world, stem, Some(display), conv, focus, None)?;
        Ok((cur, anim))
    }

    /// Shared save (mode 2): the save changed under this player (someone
    /// else won a level or opened a gate). On a map of that world the
    /// icon and path states are read again; opened icons and paths show at
    /// once, newly met gates open with their animation, and the cursor
    /// stays where it is. `None` while the instance is busy (animation,
    /// pending win): try again later. `Some(true)` if anything changed.
    pub fn refresh_map(&mut self, slot: &mut Slot) -> Option<bool> {
        if self.anim.is_some() || self.pending.is_some() {
            return None;
        }
        if !self.cur.is_map() {
            return Some(false);
        }
        let wid = self.world.id.clone();
        let stem = self.cur.stem.clone();
        let mut layout = map_layout(&stem, &self.cur.info, slot, &wid);
        let (display, anim) = gate_anim(&mut layout, slot, &wid, &stem);
        let conv = slot.conversions(&wid, &stem);
        if anim.is_none()
            && self.cur.layout.as_ref() == Some(&display)
            && conv == self.cur.conversions
        {
            return Some(false);
        }
        if anim.is_some() {
            layout.write(slot, &wid);
        }
        self.cur.cursor_at = self.cur.cursors().first().map(|u| (u.x, u.y));
        self.cur.layout = Some(display);
        self.cur.conversions = conv;
        self.cur.rebuild();
        self.anim = anim;
        self.log.push(format!("refresh {stem}"));
        Some(true)
    }

    /// Ends a running map animation at once (its final states), e.g. when
    /// a turn arrives from another machine before the local animation
    /// finished. The states after an animation never depend on its timing.
    pub fn finish_anim(&mut self) {
        if let Some(a) = self.anim.take() {
            self.cur.layout = Some(a.layout);
            self.cur.rebuild();
        }
    }

    /// Loads `stem` (map or level) at the top of the tree.
    pub fn load_top(&mut self, env: &mut Env, slot: &mut Slot) -> Result<(), String> {
        let stem = self.tree.current().unwrap_or("").to_string();
        let focus = nav(&mut self.own_nav, slot)
            .previous(&self.world.id)
            .map(str::to_string);
        let is_map = self.world.meta(&stem).is_some_and(|m| m.is_map);
        if is_map {
            let (cur, anim) = Self::load_map(env, &self.world, slot, &stem, focus)?;
            self.cur = cur;
            self.anim = anim;
        } else {
            let sur = nav(&mut self.own_nav, slot)
                .surrounds(&self.world.id)
                .map(|(dir, cells)| Surroundings { dir, cells });
            self.cur = Current::load(env, &self.world, &stem, None, Vec::new(), None, sur)?;
        }
        self.won = None;
        Ok(())
    }

    /// Starts a world in a slot (§7.5, Continue).
    pub fn start(
        env: &mut Env,
        world: WorldInfo,
        slot_index: usize,
        slot: &mut Slot,
    ) -> Result<Play, String> {
        Self::start_with(env, world, slot_index, slot, None)
    }

    /// [`Play::start`] with the player's own navigation state (mode 2).
    pub fn start_with(
        env: &mut Env,
        world: WorldInfo,
        slot_index: usize,
        slot: &mut Slot,
        mut own_nav: Option<Slot>,
    ) -> Result<Play, String> {
        let wid = world.id.clone();
        let tree = match nav(&mut own_nav, slot).level_tree(&wid) {
            Some((stems, ids)) if stems.iter().all(|s| world.exists(s)) => {
                LevelTree::from_parts(&stems, &ids)
            }
            _ => {
                let start = world.data.start.clone();
                let first = world.data.first_level.clone();
                let mut t = LevelTree::root(if start.is_empty() { &first } else { &start });
                if !first.is_empty()
                    && world.exists(&first)
                    && slot.level_state(&wid, &first) < 3
                    && first != start
                {
                    t.enter(&first, "0");
                }
                t
            }
        };
        if tree.is_empty() || !world.exists(tree.current().unwrap_or("")) {
            return Err(format!("world {wid} has no start level"));
        }
        slot.set_flag(&wid, "firsttime", true);
        let placeholder = Current::load(
            env,
            &world,
            tree.current().unwrap(),
            None,
            Vec::new(),
            None,
            None,
        )?;
        let mut p = Play {
            slot: slot_index,
            world,
            tree,
            cur: placeholder,
            anim: None,
            won: None,
            banner: None,
            title: None,
            frames: 0,
            log: Vec::new(),
            pending: None,
            own_nav,
            single: None,
        };
        p.load_top(env, slot)?;
        p.save_tree(slot);
        if !p.cur.is_map() {
            p.title = Some(TITLE_FRAMES);
            slot.set_entered(&p.world.id, &p.cur.stem.clone());
        }
        p.log.push(format!("start {} {}", p.world.id, p.cur.stem));
        Ok(p)
    }

    /// Plays one level of a collection on its own (custom levels,
    /// downloads; `worldpacks.md` §6): no map, no level tree beyond the
    /// level. Its wins and endings are recorded by [`Play::apply`] as
    /// single-level results (`level`).
    pub fn single(
        env: &mut Env,
        world: WorldInfo,
        slot_index: usize,
        stem: &str,
        level: SingleLevel,
    ) -> Result<Play, String> {
        if !world.exists(stem) {
            return Err(format!("no level {stem} in {}", world.dir.display()));
        }
        let cur = Current::load(env, &world, stem, None, Vec::new(), None, None)?;
        Ok(Play {
            slot: slot_index,
            tree: LevelTree::root(stem),
            world,
            cur,
            anim: None,
            won: None,
            banner: None,
            title: Some(TITLE_FRAMES),
            frames: 0,
            log: vec![format!("single {stem}")],
            pending: None,
            own_nav: None,
            single: Some(level),
        })
    }

    pub fn save_tree(&mut self, slot: &mut Slot) {
        let (stems, ids) = (self.tree.stems(), self.tree.ids());
        nav(&mut self.own_nav, slot).set_level_tree(&self.world.id, &stems, &ids);
    }

    /// Enters the icon `unit` targeting `target` (§7.1).
    pub fn enter(
        &mut self,
        env: &mut Env,
        slot: &mut Slot,
        texts: &Texts,
        target: &str,
        unit: i32,
    ) -> Result<(), String> {
        let wid = self.world.id.clone();
        let icon = self.cur.unit(unit).and_then(|u| u.icon.clone());
        let mapid = texts
            .map_id(&wid, target)
            .or_else(|| self.world.meta(target).and_then(|m| m.mapid));
        let id = icon
            .map(|i| level_id(mapid.as_deref(), i.style, i.number, "extra"))
            .unwrap_or_else(|| mapid.unwrap_or_else(|| "dummy".into()));
        let sur = self.cur.surroundings_of(unit);
        if let Some(s) = &sur {
            nav(&mut self.own_nav, slot).set_surrounds(&wid, s.dir, &s.cells);
        }
        self.tree.enter(target, &id);
        nav(&mut self.own_nav, slot).set_previous(&wid, target);
        slot.set_entered(&wid, target);
        self.save_tree(slot);
        let is_map = self.world.meta(target).is_some_and(|m| m.is_map);
        if is_map {
            let (cur, anim) = Self::load_map(env, &self.world, slot, target, None)?;
            self.cur = cur;
            self.anim = anim;
        } else {
            self.cur = Current::load(env, &self.world, target, None, Vec::new(), None, sur)?;
            self.title = Some(TITLE_FRAMES);
        }
        self.log.push(format!("enter {target}"));
        Ok(())
    }

    /// Jumps to `stem` directly under the root map (tests, debugging).
    pub fn goto(&mut self, env: &mut Env, slot: &mut Slot, stem: &str) -> Result<(), String> {
        if !self.world.exists(stem) {
            return Err(format!("no level {stem} in {}", self.world.id));
        }
        let root = self.tree.nodes[0].stem.clone();
        self.tree = LevelTree::root(&root);
        if stem != root {
            self.tree.enter(stem, "dummy");
        }
        self.save_tree(slot);
        self.load_top(env, slot)?;
        self.title = None;
        self.log.push(format!("goto {stem}"));
        Ok(())
    }

    /// Leaves without winning (pause → Return to map, §7.3).
    pub fn leave(&mut self, env: &mut Env, slot: &mut Slot) -> Result<(), String> {
        let wid = self.world.id.clone();
        if let Some(left) = self.tree.leave() {
            nav(&mut self.own_nav, slot).set_previous(&wid, &left.stem);
        } else if self.world.is_base() && self.cur.stem == "200level" {
            // ??? returns to the main map.
            let start = self.world.data.start.clone();
            self.tree = LevelTree::root(&start);
            nav(&mut self.own_nav, slot).set_previous(&wid, "200level");
        }
        self.save_tree(slot);
        self.load_top(env, slot)?;
        self.log.push(format!("leave to {}", self.cur.stem));
        Ok(())
    }

    /// Records the win of the current level in the save and moves up the
    /// tree (§5.1, §7.2, §7.4). The parent map is loaded by the caller
    /// (after the transition) with [`Play::return_after_win`].
    pub fn record_win(
        &mut self,
        slot: &mut Slot,
        bonus: bool,
        texts: &Texts,
    ) -> (AfterWin, Option<PendingReturn>) {
        let wid = self.world.id.clone();
        let stem = self.cur.stem.clone();
        let cp = self.cur.meta.custom_parent.clone();
        self.log.push(format!("win {stem}"));
        if cp == "<win>" {
            slot.set_level_state(&wid, &stem, 3);
            return (AfterWin::PackEnd, None);
        }
        let parent = self.tree.parent().map(str::to_string);
        let mut pending = None;
        if let Some(parent) = &parent {
            if let Some(w) = win_progress(&self.world, slot, &stem, parent, bonus)
                && (w.first || w.clear)
            {
                let banner = w.clear.then(|| texts.get("ingame_clear"));
                pending = Some(PendingReturn {
                    target: stem.clone(),
                    before: w.before,
                    after: w.after,
                    unlock: w.unlock,
                    clear: w.clear,
                    banner,
                });
            }
        } else {
            slot.set_level_state(&wid, &stem, 3);
        }
        // Up the tree, then customparent (§7.4).
        if self.tree.leave().is_none() && cp.is_empty() {
            return (AfterWin::PackEnd, None);
        }
        nav(&mut self.own_nav, slot).set_previous(&wid, &stem);
        if !cp.is_empty() && self.world.exists(&cp) {
            // Popped one step above; the top is replaced by the named
            // level or map.
            self.tree.change(&cp, "dummy");
            pending = None;
        }
        self.save_tree(slot);
        (AfterWin::Map, pending)
    }

    /// Loads the map after a win, with the unlock animation.
    pub fn return_after_win(
        &mut self,
        env: &mut Env,
        slot: &mut Slot,
        pending: Option<PendingReturn>,
    ) -> Result<(), String> {
        self.load_top(env, slot)?;
        if let Some(p) = pending
            && self.cur.is_map()
            && self.cur.stem == p.after.map
        {
            let mut display = p.before.clone();
            // Keep gates the load just opened.
            if let Some(l) = &self.cur.layout {
                for (d, cur) in display.paths.iter_mut().zip(&l.paths) {
                    if cur.gate != 0 {
                        d.state = cur.state;
                    }
                }
            }
            self.cur.layout = Some(display.clone());
            self.cur.rebuild();
            self.anim = Some(Anim::after_win(
                &p.target, &display, p.after, &p.unlock, p.clear, p.banner,
            ));
            let unlocks = self.cur.info.general.unlock_levels.clone();
            if p.clear && !unlocks.is_empty() {
                self.pending = Some(Pending::Showcase(Showcase {
                    queue: unlocks,
                    cleared: self.cur.stem.clone(),
                }));
            }
        }
        Ok(())
    }

    /// The next step of the map-clear showcase (a `Proceed`, submitted at
    /// full transition cover): shows the next listed map with the cleared
    /// map's icon completing, or, when none is left, returns to the cleared
    /// map. Returns whether the showcase is over.
    pub fn showcase_step(
        &mut self,
        env: &mut Env,
        slot: &mut Slot,
        sc: &mut Showcase,
    ) -> Result<bool, String> {
        if sc.queue.is_empty() {
            self.load_top(env, slot)?;
            return Ok(true);
        }
        let map = sc.queue.remove(0);
        let cleared = sc.cleared.clone();
        self.log.push(format!("showcase {map}"));
        let (mut cur, _) = Self::load_map(env, &self.world, slot, &map, None)?;
        let mut origin = None;
        if let Some(l) = &mut cur.layout {
            let after = l.clone();
            if let Some(i) = l.icons.iter_mut().find(|i| i.target == cleared) {
                origin = Some((i.x, i.y));
                i.state = i.state.min(2);
            }
            let anim = origin.map(|o| Anim::showcase(o, &cleared, after));
            cur.rebuild();
            self.anim = anim;
        }
        self.cur = cur;
        // Without an animation (nothing to show on this map) the presenter
        // goes on at once.
        Ok(false)
    }

    /// LEVEL IS X (§11.2): stores the conversion under the parent map and
    /// leaves.
    pub fn level_conversion(
        &mut self,
        env: &mut Env,
        slot: &mut Slot,
        objs: &[String],
    ) -> Result<(), String> {
        let wid = self.world.id.clone();
        let stem = self.cur.stem.clone();
        if let Some(parent) = self.tree.parent().map(str::to_string) {
            let list: Vec<String> = if objs.iter().any(|o| o == "revert") {
                Vec::new()
            } else {
                objs.to_vec()
            };
            slot.set_conversion(&wid, &parent, &stem, &list);
        }
        self.log.push(format!("convert {stem} {objs:?}"));
        self.leave(env, slot)
    }

    /// One turn.
    pub fn step(&mut self, input: Input) -> TurnOutcome {
        let out = self.cur.level.step(input);
        self.cur.fx.on_turn(&out, &self.cur.level);
        out
    }

    /// Advances a running animation by one frame; returns sounds to play.
    pub fn tick_anim(&mut self) -> Vec<(String, f32)> {
        let mut sounds = Vec::new();
        let Some(a) = &mut self.anim else {
            return sounds;
        };
        a.t += 1;
        let t = a.t;
        let mut changed = false;
        // Effects of the sequences (progression.md §5.2, §5.4).
        let fx = &mut self.cur.fx;
        if let Some(o) = a.origin {
            if (10..=80).contains(&t) && t % 2 == 0 {
                fx.burst("unlock", o, 1, (2, 4), true);
            }
            if t == 70 {
                fx.shake(15);
                fx.burst("smoke", o, 10, (0, 3), true);
            }
        }
        for (i, &g) in a.gates.iter().enumerate() {
            let (kind, req) = a.gate_kinds.get(i).copied().unwrap_or((1, 1));
            // The gate type's colour (`progression.md` §4.1).
            let col = match kind {
                2 => (4, 2),
                3 => (0, 3),
                _ => (2, 4),
            };
            if t == 10 {
                fx.gate_ring(g, kind, req);
            }
            if (10..130).contains(&t) && t % 20 == 10 {
                fx.burst("bonus", g, 2, col, true);
            }
            if t == 130 {
                fx.shake(20);
                fx.burst("bonus", g, 40, col, true);
                fx.burst("unlock", g, 20, (2, 4), true);
            }
        }
        for (at, changes, snd) in &a.steps {
            if *at == t {
                for c in changes {
                    if let Change::Icon { x, y, from, to, .. } | Change::Path { x, y, from, to, .. } =
                        c
                        && to > from
                    {
                        let kind = if *from == 0 { "glow" } else { "hot" };
                        self.cur.fx.burst(kind, (*x, *y), 5, (0, 3), true);
                    }
                }
                if let Some(l) = &mut self.cur.layout {
                    for c in changes {
                        l.apply(c);
                    }
                }
                changed |= !changes.is_empty();
                sounds.extend(snd.iter().cloned());
                if *at >= 70 && a.banner.is_some() && self.banner.is_none() {
                    self.banner = a.banner.clone().map(|b| (b, 120));
                }
            }
        }
        if t >= a.end {
            let a = self.anim.take().unwrap();
            self.cur.layout = Some(a.layout);
            changed = true;
        }
        if changed {
            self.cur.rebuild();
        }
        sounds
    }
}

/// What a win did to the parent map ([`win_progress`]).
pub struct WinProgress {
    /// The parent map's layout before and after the win.
    pub before: MapLayout,
    pub after: MapLayout,
    pub unlock: Unlock,
    /// First completion (a spore).
    pub first: bool,
    /// The win cleared the parent map (a flower).
    pub clear: bool,
}

/// Records the win of `stem` under the map `parent` in `slot` (§5.1): the
/// level completed, the parent map's unlocks, the spore of a first
/// completion, the orb, the map clear and completion. Idempotent: winning
/// a level again adds nothing. Also used to merge another player's win
/// into a shared save (online mode 2). `None` if the parent map does not
/// load.
pub fn win_progress(
    world: &WorldInfo,
    slot: &mut Slot,
    stem: &str,
    parent: &str,
    bonus: bool,
) -> Option<WinProgress> {
    let wid = world.id.as_str();
    let first = slot.level_state(wid, stem) < 3;
    let info = world.load(parent).map(|l| l.info).ok()?;
    let mut layout = map_layout(parent, &info, slot, wid);
    let before = layout.clone();
    let pos = layout
        .icons
        .iter()
        .find(|i| i.target == stem)
        .map(|i| (i.x, i.y));
    let unlock = match pos {
        Some((x, y)) => layout.complete(stem, x, y),
        None => Unlock::default(),
    };
    slot.set_level_state(wid, stem, 3);
    if first {
        slot.add_prize(wid, parent);
    }
    if bonus {
        slot.add_bonus(wid, stem);
    }
    layout.write(slot, wid);
    let pm = world.meta(parent).unwrap_or_default();
    let mut clear = false;
    if pm.unlock_count > 0
        && layout.completed_count() as i32 >= pm.unlock_count
        && !slot.cleared(wid, parent)
    {
        slot.add_clear(wid, parent);
        slot.set_level_state(wid, parent, 3);
        clear = true;
    }
    if !layout.icons.is_empty() && layout.icons.iter().all(|i| i.state >= 3) {
        slot.set_complete(wid, parent);
    }
    Some(WinProgress {
        before,
        after: layout,
        unlock,
        first,
        clear,
    })
}

/// Data carried from the win to the parent map's load.
pub struct PendingReturn {
    pub target: String,
    pub before: MapLayout,
    pub after: MapLayout,
    pub unlock: Unlock,
    pub clear: bool,
    pub banner: Option<String>,
}

/// Title card length in frames.
pub const TITLE_FRAMES: u32 = 200;

/// Palette colour helper for icons (`x,y` strings).
pub fn parse_colour(s: &str) -> Option<PalColour> {
    PalColour::parse(s)
}

/// Cosmetic seed of a level view (any value works; outcomes never depend
/// on it).
fn fx_seed(world: &str, stem: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in world.bytes().chain(stem.bytes()) {
        h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// Level flags that switch effects off (`presentation.md` §6.4, §7.1, §9.1).
fn fx_options(info: &LevelInfo) -> FxOptions {
    let flag = |k: &str| info.ini.get("general", k).is_some_and(|v| v.trim() == "1");
    FxOptions {
        particles: !flag("disableparticles"),
        shake: !flag("disableshake"),
        rule_effect: !flag("disableruleeffect"),
    }
}
