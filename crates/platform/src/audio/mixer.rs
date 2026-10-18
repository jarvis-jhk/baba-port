//! Software mixer: many simultaneous voices with volume, pitch, pan,
//! looping and buses; voices play decoded samples (sounds) or pull frames
//! from a stream (music, decoded on the fly). Pure Rust, no I/O; driven by
//! the platform's audio callback or by the offline WAV renderer.

use std::sync::Arc;

/// Decoded audio, interleaved stereo `f32` at its own sample rate.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundData {
    pub sample_rate: u32,
    /// Interleaved L/R frames.
    pub samples: Vec<f32>,
}

impl SoundData {
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }

    pub fn duration(&self) -> f64 {
        self.frames() as f64 / self.sample_rate as f64
    }

    /// A sine tone with short fades (test signal).
    pub fn sine(freq: f32, seconds: f32, sample_rate: u32, amplitude: f32) -> Self {
        let n = (seconds * sample_rate as f32) as usize;
        let fade = (0.005 * sample_rate as f32) as usize;
        let mut samples = Vec::with_capacity(n * 2);
        for i in 0..n {
            let env = (i.min(n - 1 - i) as f32 / fade.max(1) as f32).min(1.0);
            let s = (i as f32 / sample_rate as f32 * freq * std::f32::consts::TAU).sin()
                * amplitude
                * env;
            samples.extend_from_slice(&[s, s]);
        }
        Self {
            sample_rate,
            samples,
        }
    }
}

/// Mixer buses; each has its own volume under the master volume.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bus {
    Music = 0,
    Sfx = 1,
    Ui = 2,
}

pub const BUS_COUNT: usize = 3;

/// How a voice plays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayParams {
    pub volume: f32,
    /// Playback rate multiplier (1 = original pitch).
    pub pitch: f32,
    /// -1 left .. 0 centre .. 1 right (constant-power).
    pub pan: f32,
    pub looping: bool,
    pub bus: Bus,
    /// Silence before the voice starts, in seconds (schedules a song's
    /// loop part to start exactly when its intro ends).
    pub delay: f64,
}

impl Default for PlayParams {
    fn default() -> Self {
        Self {
            volume: 1.0,
            pitch: 1.0,
            pan: 0.0,
            looping: false,
            bus: Bus::Sfx,
            delay: 0.0,
        }
    }
}

/// A source decoded while it plays (music): yields stereo frames.
pub trait FrameStream: Send {
    fn sample_rate(&self) -> u32;
    /// The next frame, or `None` at the end.
    fn next_frame(&mut self) -> Option<[f32; 2]>;
    /// Rewinds to the start (for looping); false if impossible.
    fn rewind(&mut self) -> bool;
}

/// Identifies a playing voice (allocated by the caller, never reused).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VoiceId(pub u64);

/// Commands from the game thread to the mixer.
pub enum Command {
    Play {
        id: VoiceId,
        sound: Arc<SoundData>,
        params: PlayParams,
    },
    PlayStream {
        id: VoiceId,
        stream: Box<dyn FrameStream>,
        params: PlayParams,
    },
    Stop(VoiceId),
    SetVolume(VoiceId, f32),
    SetPitch(VoiceId, f32),
    SetPan(VoiceId, f32),
    SetBusVolume(Bus, f32),
    SetMasterVolume(f32),
    StopAll,
}

enum Source {
    Sample {
        sound: Arc<SoundData>,
        /// Position in source frames (fractional).
        pos: f64,
    },
    Stream {
        stream: Box<dyn FrameStream>,
        /// The two frames being interpolated and the position between them.
        cur: [f32; 2],
        next: [f32; 2],
        frac: f64,
        primed: bool,
    },
}

struct Voice {
    id: VoiceId,
    source: Source,
    params: PlayParams,
    /// Output frames of silence left before the voice starts.
    wait: usize,
    /// Gains applied at the end of the previous buffer, for click-free
    /// ramps when volume/pan change.
    last_gain: [f32; 2],
    stopping: bool,
}

/// Mixes voices into interleaved stereo `f32`.
pub struct Mixer {
    sample_rate: u32,
    voices: Vec<Voice>,
    bus_volume: [f32; BUS_COUNT],
    master: f32,
    max_voices: usize,
}

fn pan_gains(pan: f32) -> [f32; 2] {
    let a = (pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    // Normalised so centre is unity gain on both channels.
    [
        a.cos() * std::f32::consts::SQRT_2,
        a.sin() * std::f32::consts::SQRT_2,
    ]
}

impl Mixer {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            voices: Vec::new(),
            bus_volume: [1.0; BUS_COUNT],
            master: 1.0,
            max_voices: 256,
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn active_voices(&self) -> usize {
        self.voices.len()
    }

    pub fn apply(&mut self, cmd: Command) {
        match cmd {
            Command::Play { id, sound, params } => {
                self.start(id, Source::Sample { sound, pos: 0.0 }, params)
            }
            Command::PlayStream { id, stream, params } => self.start(
                id,
                Source::Stream {
                    stream,
                    cur: [0.0; 2],
                    next: [0.0; 2],
                    frac: 0.0,
                    primed: false,
                },
                params,
            ),
            Command::Stop(id) => self.with_voice(id, |v| v.stopping = true),
            Command::SetVolume(id, x) => self.with_voice(id, |v| v.params.volume = x),
            Command::SetPitch(id, x) => self.with_voice(id, |v| v.params.pitch = x),
            Command::SetPan(id, x) => self.with_voice(id, |v| v.params.pan = x),
            Command::SetBusVolume(bus, x) => self.bus_volume[bus as usize] = x,
            Command::SetMasterVolume(x) => self.master = x,
            Command::StopAll => self.voices.iter_mut().for_each(|v| v.stopping = true),
        }
    }

    fn start(&mut self, id: VoiceId, source: Source, params: PlayParams) {
        if self.voices.len() >= self.max_voices {
            // Steal the oldest non-looping voice (else the oldest).
            let i = self
                .voices
                .iter()
                .position(|v| !v.params.looping)
                .unwrap_or(0);
            self.voices.remove(i);
        }
        let g = self.gains(&params);
        let wait = (params.delay.max(0.0) * self.sample_rate as f64).round() as usize;
        self.voices.push(Voice {
            id,
            source,
            params,
            wait,
            last_gain: g,
            stopping: false,
        });
    }

    fn with_voice(&mut self, id: VoiceId, f: impl FnOnce(&mut Voice)) {
        if let Some(v) = self.voices.iter_mut().find(|v| v.id == id) {
            f(v);
        }
    }

    fn gains(&self, p: &PlayParams) -> [f32; 2] {
        let k = p.volume * self.bus_volume[p.bus as usize] * self.master;
        let [l, r] = pan_gains(p.pan);
        [l * k, r * k]
    }

    /// Mixes into `out` (interleaved stereo), overwriting it.
    pub fn mix(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        let frames = out.len() / 2;
        if frames == 0 {
            return;
        }
        let rate = self.sample_rate as f64;
        let mut i = 0;
        while i < self.voices.len() {
            let target = if self.voices[i].stopping {
                [0.0, 0.0]
            } else {
                self.gains(&self.voices[i].params)
            };
            let v = &mut self.voices[i];
            let g0 = v.last_gain;
            let inv = 1.0 / frames as f32;
            let skip = v.wait.min(frames);
            v.wait -= skip;
            let pitch = v.params.pitch.max(0.0) as f64;
            let looping = v.params.looping;
            let mut finished = false;
            match &mut v.source {
                Source::Sample { sound, pos } => {
                    let step = sound.sample_rate as f64 / rate * pitch;
                    let src = &sound.samples;
                    let len = sound.frames();
                    finished = len == 0;
                    for f in skip..frames {
                        if finished {
                            break;
                        }
                        let t = (f + 1) as f32 * inv;
                        let gl = g0[0] + (target[0] - g0[0]) * t;
                        let gr = g0[1] + (target[1] - g0[1]) * t;
                        let p = *pos as usize;
                        let frac = (*pos - p as f64) as f32;
                        let next = if p + 1 < len {
                            p + 1
                        } else if looping {
                            0
                        } else {
                            p
                        };
                        let l = src[p * 2] + (src[next * 2] - src[p * 2]) * frac;
                        let r = src[p * 2 + 1] + (src[next * 2 + 1] - src[p * 2 + 1]) * frac;
                        out[f * 2] += l * gl;
                        out[f * 2 + 1] += r * gr;
                        *pos += step;
                        if *pos >= len as f64 {
                            if looping {
                                *pos %= len as f64;
                            } else {
                                finished = true;
                            }
                        }
                    }
                }
                Source::Stream {
                    stream,
                    cur,
                    next,
                    frac,
                    primed,
                } => {
                    let step = stream.sample_rate() as f64 / rate * pitch;
                    let pull = |s: &mut Box<dyn FrameStream>| -> Option<[f32; 2]> {
                        s.next_frame()
                            .or_else(|| (looping && s.rewind()).then(|| s.next_frame()).flatten())
                    };
                    if !*primed {
                        match (pull(stream), pull(stream)) {
                            (Some(a), Some(b)) => (*cur, *next) = (a, b),
                            (Some(a), None) => (*cur, *next) = (a, a),
                            _ => finished = true,
                        }
                        *primed = true;
                    }
                    for f in skip..frames {
                        if finished {
                            break;
                        }
                        let t = (f + 1) as f32 * inv;
                        let gl = g0[0] + (target[0] - g0[0]) * t;
                        let gr = g0[1] + (target[1] - g0[1]) * t;
                        let k = *frac as f32;
                        out[f * 2] += (cur[0] + (next[0] - cur[0]) * k) * gl;
                        out[f * 2 + 1] += (cur[1] + (next[1] - cur[1]) * k) * gr;
                        *frac += step;
                        while *frac >= 1.0 {
                            *frac -= 1.0;
                            *cur = *next;
                            match pull(stream) {
                                Some(n) => *next = n,
                                None => {
                                    finished = true;
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            v.last_gain = target;
            if finished || v.stopping {
                self.voices.swap_remove(i);
            } else {
                i += 1;
            }
        }
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play(m: &mut Mixer, id: u64, sound: &Arc<SoundData>, params: PlayParams) {
        m.apply(Command::Play {
            id: VoiceId(id),
            sound: sound.clone(),
            params,
        });
    }

    fn dc(frames: usize, rate: u32, v: f32) -> Arc<SoundData> {
        Arc::new(SoundData {
            sample_rate: rate,
            samples: vec![v; frames * 2],
        })
    }

    #[test]
    fn voices_sum_and_finish() {
        let mut m = Mixer::new(48000);
        let s = dc(100, 48000, 0.25);
        play(&mut m, 1, &s, PlayParams::default());
        play(&mut m, 2, &s, PlayParams::default());
        let mut out = vec![0.0; 2 * 64];
        m.mix(&mut out);
        assert!((out[10] - 0.5).abs() < 1e-6);
        assert_eq!(m.active_voices(), 2);
        m.mix(&mut out);
        // 100 frames consumed after 128 output frames.
        assert_eq!(m.active_voices(), 0);
        assert!((out[2 * 35] - 0.5).abs() < 1e-6);
        assert_eq!(out[2 * 40], 0.0);
    }

    #[test]
    fn pitch_and_resampling() {
        // A 24 kHz source at pitch 1 lasts twice as many 48 kHz frames.
        let mut m = Mixer::new(48000);
        play(&mut m, 1, &dc(100, 24000, 0.1), PlayParams::default());
        let mut out = vec![0.0; 2 * 199];
        m.mix(&mut out);
        assert_eq!(m.active_voices(), 1);
        m.mix(&mut out);
        assert_eq!(m.active_voices(), 0);
        // Pitch 2 on the same source matches the output rate.
        play(
            &mut m,
            2,
            &dc(100, 24000, 0.1),
            PlayParams {
                pitch: 2.0,
                ..Default::default()
            },
        );
        let mut out = vec![0.0; 2 * 100];
        m.mix(&mut out);
        assert_eq!(m.active_voices(), 0);
    }

    #[test]
    fn pan_buses_and_loop() {
        let mut m = Mixer::new(48000);
        let s = dc(10, 48000, 0.5);
        let p = PlayParams {
            pan: 1.0,
            looping: true,
            bus: Bus::Music,
            ..Default::default()
        };
        play(&mut m, 7, &s, p);
        let mut out = vec![0.0; 2 * 64];
        m.mix(&mut out);
        let (l, r) = (out[2 * 63], out[2 * 63 + 1]);
        assert!(l.abs() < 1e-6 && r > 0.6, "{l} {r}");
        assert_eq!(m.active_voices(), 1, "looping voice survives");
        m.apply(Command::SetBusVolume(Bus::Music, 0.0));
        m.mix(&mut out);
        m.mix(&mut out);
        assert!(out.iter().all(|s| s.abs() < 1e-6));
        m.apply(Command::Stop(VoiceId(7)));
        m.mix(&mut out);
        assert_eq!(m.active_voices(), 0);
    }

    #[test]
    fn delayed_start() {
        let mut m = Mixer::new(48000);
        let s = dc(1000, 48000, 0.5);
        play(
            &mut m,
            1,
            &s,
            PlayParams {
                delay: 100.0 / 48000.0,
                ..Default::default()
            },
        );
        let mut out = vec![0.0; 2 * 64];
        m.mix(&mut out);
        assert!(out.iter().all(|s| *s == 0.0), "still waiting");
        m.mix(&mut out);
        assert_eq!(out[2 * 35], 0.0);
        assert!((out[2 * 40] - 0.5).abs() < 1e-6, "started at frame 100");
    }

    #[test]
    fn many_voices() {
        let mut m = Mixer::new(48000);
        let s = dc(1000, 48000, 0.001);
        for i in 0..300 {
            play(&mut m, i, &s, PlayParams::default());
        }
        assert_eq!(m.active_voices(), 256);
        let mut out = vec![0.0; 2 * 256];
        m.mix(&mut out);
        assert!((out[100] - 0.256).abs() < 1e-3);
    }

    struct Dc {
        left: usize,
        len: usize,
        rate: u32,
    }

    impl FrameStream for Dc {
        fn sample_rate(&self) -> u32 {
            self.rate
        }
        fn next_frame(&mut self) -> Option<[f32; 2]> {
            (self.left > 0).then(|| {
                self.left -= 1;
                [0.25, -0.25]
            })
        }
        fn rewind(&mut self) -> bool {
            self.left = self.len;
            true
        }
    }

    #[test]
    fn streams_play_resample_and_loop() {
        let mut m = Mixer::new(48000);
        let stream = |len, rate| {
            Box::new(Dc {
                left: len,
                len,
                rate,
            })
        };
        m.apply(Command::PlayStream {
            id: VoiceId(1),
            stream: stream(100, 24000),
            params: PlayParams::default(),
        });
        let mut out = vec![0.0; 2 * 150];
        m.mix(&mut out);
        assert!((out[0] - 0.25).abs() < 1e-6 && (out[1] + 0.25).abs() < 1e-6);
        assert_eq!(m.active_voices(), 1, "100 frames at half rate last ~200");
        m.mix(&mut out);
        assert_eq!(m.active_voices(), 0);
        m.apply(Command::PlayStream {
            id: VoiceId(2),
            stream: stream(10, 48000),
            params: PlayParams {
                looping: true,
                ..Default::default()
            },
        });
        for _ in 0..5 {
            m.mix(&mut out);
        }
        assert_eq!(m.active_voices(), 1);
        assert!(out.iter().step_by(2).all(|s| (s - 0.25).abs() < 1e-6));
    }
}
