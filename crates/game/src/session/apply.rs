//! The one path through which an instance changes: [`Play::apply`] takes
//! a [`Turn`] and returns what the presenter should show or play
//! ([`Fx`]). Single player, local multiplayer and (M7) the online replica
//! all call it; presentation timing never changes the result.

use baba_progress::Slot;
use baba_sim::{DestroyStyle, Event as SimEvent, Input as SimInput, Transition, TurnOutcome};

use crate::credits::Kind as CreditsKind;
use crate::lang::Texts;
use crate::levelcodes::store;
use crate::packs;
use crate::play::{AfterWin, Env, PendingReturn, Play, Showcase};
use crate::session::Turn;

/// What [`Turn::Proceed`] does.
pub enum Pending {
    /// Back to the parent map (with the unlock animation if any).
    Return(Option<Box<PendingReturn>>),
    /// The pack is beaten (`customparent=<win>` or the root was won).
    PackEnd,
    /// LEVEL IS X (`progression.md` §11.2).
    Convert(Vec<String>),
    /// END / ALL IS DONE reached; the credits roll.
    Ending(CreditsKind),
    /// The map-clear showcase (`progression.md` §5.3): each `Proceed`
    /// shows the next map, the last one returns to the cleared map.
    Showcase(Showcase),
    /// A level played on its own (`Play::single`) was won, ended or
    /// converted (already recorded); the play ends. `won`: it was a win.
    LeaveSingle { won: bool },
}

/// What the presenter does after a turn.
#[derive(Clone, Debug)]
pub enum Fx {
    /// Turn sounds and the music state (`audio.md` §3, §4).
    Turn(TurnOutcome),
    /// The undo sound (a random variant).
    UndoSound,
    /// A banner text (already localised) for N frames.
    Banner(String, u32),
    /// The level was won: the victory banner, then (after the delay and a
    /// closing transition) [`Turn::Proceed`].
    Won,
    /// Close the screen and submit [`Turn::Proceed`] at full cover (level
    /// conversion, endings).
    ProceedAfterTransition,
    /// Roll these credits now.
    Credits(CreditsKind),
    /// A new level or map is loaded (music, title card).
    Loaded,
    /// ALL IS DONE in the base world unlocks the editor's advanced words
    /// (a setting, not part of the slot).
    EditorSpecial,
    /// A single level's play is over (`Pending::LeaveSingle`): back to
    /// the menus it was started from.
    LeaveSingle { won: bool },
}

/// Result of [`Play::apply`].
#[derive(Default, Debug)]
pub struct Applied {
    pub fx: Vec<Fx>,
    /// The slot changed and should be written.
    pub save: bool,
    /// Shared progress changed (a win, gates, conversions): other players
    /// on the same save re-read their maps (mode 2).
    pub progress: bool,
    /// Nothing happened (turn not valid in this state).
    pub ignored: bool,
}

/// FNV-1a 64 over bytes, continued from `h`.
pub fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// FNV-1a 64 offset basis.
pub const FNV0: u64 = 0xcbf2_9ce4_8422_2325;

impl Play {
    /// Applies one turn (see [`Turn`]). `slot` is the save the instance
    /// plays on (shared by every instance in mode 2).
    pub fn apply(&mut self, turn: &Turn, env: &mut Env, slot: &mut Slot, texts: &Texts) -> Applied {
        let mut a = Applied::default();
        // States after an animation never depend on its timing.
        self.finish_anim();
        self.title = None;
        let r = match turn {
            Turn::Move { dir, you } => {
                if self.pending.is_some() || *dir > 4 {
                    a.ignored = true;
                    return a;
                }
                let input = SimInput::Key {
                    dir: *dir,
                    player: (*you).clamp(1, 2),
                };
                self.sim_turn(input, slot, texts, &mut a);
                Ok(())
            }
            Turn::Auto => {
                if self.pending.is_some() || !self.cur.level.auto_active() {
                    a.ignored = true;
                    return a;
                }
                self.sim_turn(SimInput::Tick, slot, texts, &mut a);
                Ok(())
            }
            Turn::Undo => {
                if self.pending.is_some() {
                    a.ignored = true;
                    return a;
                }
                a.fx.push(Fx::UndoSound);
                self.sim_turn(SimInput::Undo, slot, texts, &mut a);
                Ok(())
            }
            Turn::Restart => {
                self.cur.level.restart();
                self.cur.fx.start(&self.cur.level);
                self.won = None;
                self.pending = None;
                a.save = true;
                Ok(())
            }
            Turn::Enter { target, unit } => {
                // Only what is under a cursor now: a turn made on another
                // machine (or before another player moved the cursor) may
                // arrive when the map has changed.
                if !self.cur.is_map()
                    || self.pending.is_some()
                    || !self.world.exists(target)
                    || !self
                        .cur
                        .candidates(&self.world)
                        .iter()
                        .any(|(t, u)| t == target && u == unit)
                {
                    a.ignored = true;
                    return a;
                }
                let r = self.enter(env, slot, texts, target, *unit);
                a.fx.push(Fx::Loaded);
                a.save = true;
                a.progress = true;
                r
            }
            Turn::Leave => {
                if !self.can_return() || self.pending.is_some() {
                    a.ignored = true;
                    return a;
                }
                let r = self.leave(env, slot);
                a.fx.push(Fx::Loaded);
                a.save = true;
                a.progress = true;
                r
            }
            Turn::Refresh => match self.refresh_map(slot) {
                Some(true) => {
                    a.fx.push(Fx::Loaded);
                    Ok(())
                }
                _ => {
                    a.ignored = true;
                    return a;
                }
            },
            Turn::Proceed => match self.pending.take() {
                None => {
                    a.ignored = true;
                    return a;
                }
                Some(Pending::Return(p)) => {
                    self.won = None;
                    let r = self.return_after_win(env, slot, p.map(|b| *b));
                    a.fx.push(Fx::Loaded);
                    a.save = true;
                    a.progress = true;
                    r
                }
                Some(Pending::Convert(objs)) => {
                    let r = self.level_conversion(env, slot, &objs);
                    a.fx.push(Fx::Loaded);
                    a.save = true;
                    a.progress = true;
                    r
                }
                Some(Pending::PackEnd) => {
                    let wid = self.world.id.clone();
                    slot.set_flag(&wid, "end", true);
                    self.log.push(format!("pack end {wid}"));
                    a.fx.push(Fx::Credits(CreditsKind::Pack));
                    a.save = true;
                    a.progress = true;
                    Ok(())
                }
                Some(Pending::Ending(kind)) => {
                    a.fx.push(Fx::Credits(kind));
                    Ok(())
                }
                Some(Pending::Showcase(mut sc)) => {
                    let r = self.showcase_step(env, slot, &mut sc);
                    if matches!(r, Ok(false)) {
                        self.pending = Some(Pending::Showcase(sc));
                    }
                    a.fx.push(Fx::Loaded);
                    a.save = true;
                    r.map(|_| ())
                }
                Some(Pending::LeaveSingle { won }) => {
                    self.won = None;
                    a.fx.push(Fx::LeaveSingle { won });
                    Ok(())
                }
            },
        };
        if let Err(e) = r {
            log::error!("{e}");
            self.log.push(format!("error: {e}"));
        }
        a
    }

    fn sim_turn(&mut self, input: SimInput, slot: &mut Slot, texts: &Texts, a: &mut Applied) {
        let out = self.step(input);
        for e in &out.events {
            if let SimEvent::LevelDestroyed(s) = e {
                let key = match s {
                    DestroyStyle::Infinity => Some("ingame_infiniteloop"),
                    DestroyStyle::TooComplex => Some("ingame_toocomplex"),
                    _ => None,
                };
                if let Some(k) = key {
                    a.fx.push(Fx::Banner(texts.get(k), 180));
                }
            }
        }
        a.fx.push(Fx::Turn(out.clone()));
        if self.single.is_some() {
            self.single_outcome(&out, slot, a);
            return;
        }
        let base = self.world.is_base();
        let wid = self.world.id.clone();
        if out.ended {
            slot.set_flag(&wid, "end", true);
            self.log.push(format!("ending end {}", self.cur.stem));
            let kind = if base {
                CreditsKind::End
            } else {
                CreditsKind::Pack
            };
            self.pending = Some(Pending::Ending(kind));
            a.fx.push(Fx::ProceedAfterTransition);
            a.save = true;
            a.progress = true;
            return;
        }
        if out.all_done {
            slot.set_flag(&wid, "done", true);
            if base {
                a.fx.push(Fx::EditorSpecial);
            }
            self.log.push(format!("ending done {}", self.cur.stem));
            let kind = if base {
                CreditsKind::Done
            } else {
                CreditsKind::Pack
            };
            self.pending = Some(Pending::Ending(kind));
            a.fx.push(Fx::ProceedAfterTransition);
            a.save = true;
            a.progress = true;
            return;
        }
        if let Some(Transition::LevelConversion(objs)) = out.transition {
            self.pending = Some(Pending::Convert(objs));
            a.fx.push(Fx::ProceedAfterTransition);
            return;
        }
        if out.won {
            let bonus = self.cur.level.bonus();
            let (after, pending) = self.record_win(slot, bonus, texts);
            self.pending = Some(match after {
                AfterWin::Map => Pending::Return(pending.map(Box::new)),
                AfterWin::PackEnd => Pending::PackEnd,
            });
            a.fx.push(Fx::Won);
            a.save = true;
            a.progress = true;
        }
    }

    /// Wins, endings and level conversions of a level played on its own
    /// (instead of the map flow; `worldpacks.md` §6, `levelcodes.md`):
    /// `[levels] <stem>=3`, `[levels_won] <code>`, and the `levels_*_single`
    /// keys; then the victory banner and [`Pending::LeaveSingle`].
    fn single_outcome(&mut self, out: &TurnOutcome, slot: &mut Slot, a: &mut Applied) {
        let Some(single) = &self.single else {
            return;
        };
        let conv = match &out.transition {
            Some(Transition::LevelConversion(o)) => Some(o.join(",")),
            _ => None,
        };
        if !(out.won || out.ended || out.all_done || conv.is_some()) {
            return;
        }
        let w = store::Won {
            won: out.won,
            bonus: out.won && self.cur.level.bonus(),
            end: out.ended,
            done: out.all_done,
            converted: conv.is_some(),
        };
        if let Some(code) = &single.code {
            store::record_won(slot, code, w);
        }
        let lv = packs::LEVELS;
        let stem = self.cur.stem.as_str();
        if !single.temp {
            if out.won {
                slot.set_level_state(lv, stem, 3);
            }
            if out.ended {
                slot.store.set(&format!("{lv}_end_single"), stem, 1);
            }
            if out.all_done {
                slot.store.set(&format!("{lv}_done_single"), stem, 1);
            }
            if let Some(c) = &conv {
                slot.store.set(&format!("{lv}_converts_single"), stem, c);
            }
        }
        self.log.push(format!("single win {stem} {w:?}"));
        self.pending = Some(Pending::LeaveSingle { won: out.won });
        a.fx.push(Fx::Won);
        a.save = true;
    }

    /// Hash of everything turns drive in this instance (for desync checks
    /// and tests): the level tree, the current level's sim state, what is
    /// pending.
    pub fn state_hash(&self) -> u64 {
        let mut h = fnv(FNV0, self.world.id.as_bytes());
        for s in self.tree.stems() {
            h = fnv(h, s.as_bytes());
            h = fnv(h, b"/");
        }
        h = fnv(h, self.cur.stem.as_bytes());
        h = fnv(h, &self.cur.level.state_hash().to_le_bytes());
        let p: u8 = match &self.pending {
            None => 0,
            Some(Pending::Return(_)) => 1,
            Some(Pending::PackEnd) => 2,
            Some(Pending::Convert(_)) => 3,
            Some(Pending::Ending(_)) => 4,
            Some(Pending::LeaveSingle { .. }) => 5,
            Some(Pending::Showcase(sc)) => {
                h = fnv(h, sc.queue.join("/").as_bytes());
                6
            }
        };
        fnv(h, &[p])
    }
}
