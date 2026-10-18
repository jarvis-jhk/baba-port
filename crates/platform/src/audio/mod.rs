//! Audio: our own [`Mixer`] driven by a platform audio callback (commands
//! cross over a channel, so the callback never blocks on the game), or
//! rendered offline into a WAV file (headless runs, tests).

mod decode;
mod mixer;
mod wav;

use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

pub use decode::{OggStream, decode_ogg};
pub use mixer::{Bus, Command, FrameStream, Mixer, PlayParams, SoundData, VoiceId};
pub use wav::{WavWriter, raw_levels};

/// Output rate requested from devices and used offline. The game's music
/// and sounds are 44.1 kHz, so this avoids resampling in the common case.
pub const SAMPLE_RATE: u32 = 44100;

/// The device side of an [`Audio`] channel: owned by the backend's audio
/// callback, which calls [`DeviceMixer::render`].
pub struct DeviceMixer {
    mixer: Mixer,
    rx: Receiver<Command>,
}

impl DeviceMixer {
    /// Applies pending commands and mixes one buffer (interleaved stereo).
    pub fn render(&mut self, out: &mut [f32]) {
        while let Ok(cmd) = self.rx.try_recv() {
            self.mixer.apply(cmd);
        }
        self.mixer.mix(out);
    }
}

enum Output {
    Device(Sender<Command>),
    Offline {
        mixer: Mixer,
        wav: Option<WavWriter>,
        carry: f64,
        buf: Vec<f32>,
    },
}

/// The game's audio front end (same API for a device and offline).
pub struct Audio {
    out: Output,
    next_id: u64,
    sample_rate: u32,
    description: String,
}

impl Audio {
    /// A front end plus the mixer a backend runs in its audio callback.
    pub fn device(sample_rate: u32, description: String) -> (Self, DeviceMixer) {
        let (tx, rx) = mpsc::channel();
        (
            Self {
                out: Output::Device(tx),
                next_id: 1,
                sample_rate,
                description,
            },
            DeviceMixer {
                mixer: Mixer::new(sample_rate),
                rx,
            },
        )
    }

    /// An offline mixer; with a path, everything mixed goes to a WAV file.
    pub fn offline(wav_path: Option<&Path>) -> std::io::Result<Self> {
        let wav = wav_path
            .map(|p| WavWriter::create(p, SAMPLE_RATE))
            .transpose()?;
        let description = match wav_path {
            Some(p) => format!("offline mixer -> {}", p.display()),
            None => "offline mixer (discarded)".into(),
        };
        Ok(Self {
            out: Output::Offline {
                mixer: Mixer::new(SAMPLE_RATE),
                wav,
                carry: 0.0,
                buf: Vec::new(),
            },
            next_id: 1,
            sample_rate: SAMPLE_RATE,
            description,
        })
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    /// Backends fill this in once the device is open.
    pub fn set_description(&mut self, description: String) {
        self.description = description;
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn send(&mut self, cmd: Command) {
        match &mut self.out {
            Output::Device(tx) => {
                let _ = tx.send(cmd);
            }
            Output::Offline { mixer, .. } => mixer.apply(cmd),
        }
    }

    fn id(&mut self) -> VoiceId {
        self.next_id += 1;
        VoiceId(self.next_id - 1)
    }

    /// Plays a decoded sound.
    pub fn play(&mut self, sound: &Arc<SoundData>, params: PlayParams) -> VoiceId {
        let id = self.id();
        self.send(Command::Play {
            id,
            sound: sound.clone(),
            params,
        });
        id
    }

    /// Plays a stream (music), decoded as it plays.
    pub fn play_stream(&mut self, stream: Box<dyn FrameStream>, params: PlayParams) -> VoiceId {
        let id = self.id();
        self.send(Command::PlayStream { id, stream, params });
        id
    }

    /// Streams an Ogg Vorbis file held in memory on the music bus.
    pub fn play_music(
        &mut self,
        ogg: Arc<[u8]>,
        looping: bool,
        volume: f32,
    ) -> Result<VoiceId, String> {
        let stream = OggStream::new(ogg)?;
        Ok(self.play_stream(
            Box::new(stream),
            PlayParams {
                looping,
                volume,
                bus: Bus::Music,
                ..Default::default()
            },
        ))
    }

    pub fn stop(&mut self, id: VoiceId) {
        self.send(Command::Stop(id));
    }

    pub fn set_volume(&mut self, id: VoiceId, volume: f32) {
        self.send(Command::SetVolume(id, volume));
    }

    pub fn set_pitch(&mut self, id: VoiceId, pitch: f32) {
        self.send(Command::SetPitch(id, pitch));
    }

    pub fn set_pan(&mut self, id: VoiceId, pan: f32) {
        self.send(Command::SetPan(id, pan));
    }

    pub fn set_bus_volume(&mut self, bus: Bus, volume: f32) {
        self.send(Command::SetBusVolume(bus, volume));
    }

    pub fn set_master_volume(&mut self, volume: f32) {
        self.send(Command::SetMasterVolume(volume));
    }

    pub fn stop_all(&mut self) {
        self.send(Command::StopAll);
    }

    /// Offline only: mixes `seconds` more audio (appending to the WAV).
    /// No-op on a device, which pulls audio itself.
    pub fn advance_offline(&mut self, seconds: f64) -> std::io::Result<()> {
        if let Output::Offline {
            mixer,
            wav,
            carry,
            buf,
        } = &mut self.out
        {
            let exact = seconds * mixer.sample_rate() as f64 + *carry;
            let frames = exact.floor();
            *carry = exact - frames;
            buf.resize(frames as usize * 2, 0.0);
            mixer.mix(buf);
            if let Some(w) = wav {
                w.write(buf)?;
            }
        }
        Ok(())
    }

    /// Flushes the WAV file, if any.
    pub fn finish(self) -> std::io::Result<()> {
        if let Output::Offline { wav: Some(w), .. } = self.out {
            w.finish()?;
        }
        Ok(())
    }
}
