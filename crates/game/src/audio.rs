//! Audio rules (`docs/spec/audio.md`): which track plays (level/map music,
//! menu, ending; continuity when the track does not change), the music
//! state when no player exists, per-turn sounds chosen from the turn's
//! events, effect sounds by name, and the volume settings.
//!
//! The director only queues commands; [`AudioDirector::flush`] sends them
//! to whatever `Audio` the platform has (a device, an offline mixer, or
//! nothing). Every request is also logged for the headless flow test.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use baba_platform::Audio;
use baba_platform::audio::{Bus, PlayParams, VoiceId};
use baba_sim::{CosmeticRng, Event as SimEvent, Sound, TurnKind, TurnOutcome};

use crate::sounds::SoundBank;

enum Cmd {
    Music(Option<PathBuf>),
    Sound(String, f32),
    Volumes(f32, f32),
    MusicGain(f32),
}

/// Most sounds per tick while mixing several instances.
pub const MIX_CAP: usize = 6;

/// Music and sounds.
pub struct AudioDirector {
    game_dir: PathBuf,
    pub bank: SoundBank,
    queue: Vec<Cmd>,
    current: Option<PathBuf>,
    voice: Option<VoiceId>,
    /// 0 = player present, 1 = no player (music faded out).
    pub state: u8,
    cosmetic: CosmeticRng,
    rng: u64,
    /// Multiplayer mixing: sound families played this tick.
    mix: Option<Vec<String>>,
    /// Everything requested (for scripted tests): `music:<name>`,
    /// `sound:<name>`.
    pub log: Vec<String>,
}

impl AudioDirector {
    pub fn new(game_dir: &Path, bank: SoundBank) -> Self {
        Self {
            game_dir: game_dir.to_path_buf(),
            bank,
            queue: Vec::new(),
            current: None,
            voice: None,
            state: 0,
            cosmetic: CosmeticRng::new(0x5EED_A0D1),
            rng: 0x1234_5678,
            mix: None,
            log: Vec::new(),
        }
    }

    fn rand(&mut self, n: u32) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng % n.max(1) as u64) as u32
    }

    /// Volume settings 0–100 (linear, `audio.md` §5).
    pub fn set_volumes(&mut self, music: i64, sound: i64) {
        self.queue
            .push(Cmd::Volumes(music as f32 / 100.0, sound as f32 / 100.0));
    }

    /// Music of a level (`audio.md` §2.1): `music`, from the world's
    /// `Music/` when `localmusic=1`; empty = `baba`.
    pub fn level_music(&mut self, world: &str, music: &str, local: bool) {
        let name = if music.is_empty() { "baba" } else { music };
        let path = if local {
            self.game_dir
                .join("Data/Worlds")
                .join(world)
                .join("Music")
                .join(format!("{name}.ogg"))
        } else {
            self.game_dir.join("Data/Music").join(format!("{name}.ogg"))
        };
        self.music_path(path);
    }

    /// A global track (`menu`, `ending`, …).
    pub fn music(&mut self, name: &str) {
        let p = self.game_dir.join("Data/Music").join(format!("{name}.ogg"));
        self.music_path(p);
    }

    pub fn stop_music(&mut self) {
        if self.current.is_some() {
            self.current = None;
            self.log.push("music:-".into());
            self.queue.push(Cmd::Music(None));
        }
    }

    fn music_path(&mut self, p: PathBuf) {
        // Same track: keep playing (continuity, audio.md §2.2).
        if self.current.as_ref() == Some(&p) {
            return;
        }
        let name = p
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.log.push(format!("music:{name}"));
        self.current = Some(p.clone());
        self.state = 0;
        self.queue.push(Cmd::Music(Some(p)));
    }

    /// Music state after a turn (`audio.md` §3).
    pub fn player_state(&mut self, no_player: bool) {
        let s = no_player as u8;
        if s != self.state {
            self.state = s;
            self.log.push(format!("musicstate:{s}"));
            self.queue
                .push(Cmd::MusicGain(if no_player { 0.0 } else { 1.0 }));
        }
    }

    /// Plays an effect by name at a playback rate (1 = normal).
    pub fn sound(&mut self, name: &str) {
        self.sound_pitch(name, 1.0);
    }

    /// Multiplayer mixing (many instances, one mixer): per tick every
    /// sound family (name without variant digits / `_short`) plays once
    /// and at most [`MIX_CAP`] sounds play; the rest are dropped.
    pub fn set_mixing(&mut self, on: bool) {
        self.mix = on.then(Vec::new);
    }

    /// Starts a mixing tick.
    pub fn begin_tick(&mut self) {
        if let Some(m) = &mut self.mix {
            m.clear();
        }
    }

    pub fn sound_pitch(&mut self, name: &str, pitch: f32) {
        if let Some(m) = &mut self.mix {
            let fam = name
                .trim_end_matches("_short")
                .trim_end_matches(|c: char| c.is_ascii_digit())
                .to_string();
            if m.contains(&fam) || m.len() >= MIX_CAP {
                self.log.push(format!("sound-skip:{name}"));
                return;
            }
            m.push(fam);
        }
        self.log.push(format!("sound:{name}"));
        self.queue.push(Cmd::Sound(name.to_string(), pitch));
    }

    /// A random variant `name1..nameN`.
    pub fn sound_variant(&mut self, base: &str, n: u32) {
        let v = self.rand(n) + 1;
        self.sound(&format!("{base}{v}"));
    }

    /// The sounds of one turn (`audio.md` §4.1–§4.3): the sim's two slots
    /// (variants rolled here, `_short` when the sim asks for it and the
    /// install has it), sounds played directly, rule sounds and PLAY notes.
    pub fn turn(&mut self, out: &TurnOutcome) {
        if out.kind == TurnKind::Stored {
            return;
        }
        for s in [out.turn_sound, out.removal_sound].into_iter().flatten() {
            self.play_sim_sound(s);
        }
        for e in &out.events {
            match e {
                SimEvent::Sound(s) => self.play_sim_sound(*s),
                SimEvent::RuleFormed { sound: Some(s), .. } => self.play_sim_sound(*s),
                SimEvent::Play { name, note, .. } => {
                    let freq = baba_sim::events::note_freq(note) as f32;
                    // The original plays samples at 44 kHz by default.
                    self.sound_pitch(baba_sim::events::play_sample(name), freq / 44000.0);
                }
                _ => {}
            }
        }
    }

    fn play_sim_sound(&mut self, s: Sound) {
        if s.is_silent() || s.family == "silent" || s.family == "turn" {
            return;
        }
        let name = s.name(&mut self.cosmetic);
        if s.short && !self.bank.has(&name) {
            let long = name.trim_end_matches("_short").to_string();
            self.sound(&long);
        } else {
            self.sound(&name);
        }
    }

    /// Sends queued commands to the platform's audio (if any).
    pub fn flush(&mut self, audio: Option<&mut Audio>) {
        let Some(a) = audio else {
            self.queue.clear();
            return;
        };
        for cmd in std::mem::take(&mut self.queue) {
            match cmd {
                Cmd::Volumes(m, s) => {
                    a.set_bus_volume(Bus::Music, m);
                    a.set_bus_volume(Bus::Sfx, s);
                    a.set_bus_volume(Bus::Ui, s);
                }
                Cmd::Music(p) => {
                    if let Some(v) = self.voice.take() {
                        a.stop(v);
                    }
                    if let Some(p) = p {
                        match std::fs::read(&p) {
                            Ok(bytes) => match a.play_music(Arc::from(bytes), true, 1.0) {
                                Ok(v) => self.voice = Some(v),
                                Err(e) => log::warn!("{}: {e}", p.display()),
                            },
                            Err(e) => log::warn!("{}: {e}", p.display()),
                        }
                    }
                }
                Cmd::MusicGain(g) => {
                    if let Some(v) = self.voice {
                        a.set_volume(v, g);
                    }
                }
                Cmd::Sound(name, pitch) => {
                    if let Some(s) = self.bank.get(&name) {
                        a.play(
                            &s,
                            PlayParams {
                                pitch,
                                ..Default::default()
                            },
                        );
                    }
                }
            }
        }
    }
}
