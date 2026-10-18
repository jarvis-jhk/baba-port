//! Ogg Vorbis decoding (pure Rust, `lewton`): whole files for sounds,
//! on-the-fly streams for music.

use std::io::{Cursor, Read, Seek};
use std::sync::Arc;

use lewton::inside_ogg::OggStreamReader;

use super::mixer::{FrameStream, SoundData};

fn err(e: impl std::fmt::Display) -> String {
    format!("Ogg Vorbis: {e}")
}

/// Decodes a whole Ogg Vorbis stream to interleaved stereo `f32`
/// (mono is duplicated, channels beyond two are dropped).
pub fn decode_ogg<R: Read + Seek>(reader: R) -> Result<SoundData, String> {
    let mut ogg = OggStreamReader::new(reader).map_err(err)?;
    let channels = ogg.ident_hdr.audio_channels as usize;
    let sample_rate = ogg.ident_hdr.audio_sample_rate;
    if channels == 0 {
        return Err(err("no channels"));
    }
    let mut samples = Vec::new();
    while let Some(packet) = ogg.read_dec_packet_itl().map_err(err)? {
        push_stereo(&mut samples, &packet, channels);
    }
    Ok(SoundData {
        sample_rate,
        samples,
    })
}

fn push_stereo(out: &mut Vec<f32>, packet: &[i16], channels: usize) {
    for frame in packet.chunks_exact(channels) {
        let l = frame[0] as f32 / 32768.0;
        let r = if channels > 1 {
            frame[1] as f32 / 32768.0
        } else {
            l
        };
        out.extend_from_slice(&[l, r]);
    }
}

/// Music: an Ogg file in memory, decoded packet by packet as it plays
/// (a few KB of decoded audio at a time instead of tens of MB per song).
pub struct OggStream {
    data: Arc<[u8]>,
    reader: OggStreamReader<Cursor<Arc<[u8]>>>,
    channels: usize,
    sample_rate: u32,
    buf: Vec<f32>,
    pos: usize,
}

impl OggStream {
    pub fn new(data: Arc<[u8]>) -> Result<Self, String> {
        let reader = OggStreamReader::new(Cursor::new(data.clone())).map_err(err)?;
        let channels = reader.ident_hdr.audio_channels as usize;
        if channels == 0 {
            return Err(err("no channels"));
        }
        Ok(Self {
            sample_rate: reader.ident_hdr.audio_sample_rate,
            data,
            reader,
            channels,
            buf: Vec::new(),
            pos: 0,
        })
    }
}

impl FrameStream for OggStream {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn next_frame(&mut self) -> Option<[f32; 2]> {
        while self.pos + 2 > self.buf.len() {
            self.buf.clear();
            self.pos = 0;
            match self.reader.read_dec_packet_itl() {
                Ok(Some(p)) => push_stereo(&mut self.buf, &p, self.channels),
                Ok(None) => return None,
                Err(e) => {
                    log::warn!("music stream: {e}");
                    return None;
                }
            }
        }
        let f = [self.buf[self.pos], self.buf[self.pos + 1]];
        self.pos += 2;
        Some(f)
    }

    fn rewind(&mut self) -> bool {
        match OggStreamReader::new(Cursor::new(self.data.clone())) {
            Ok(r) => {
                self.reader = r;
                self.buf.clear();
                self.pos = 0;
                true
            }
            Err(_) => false,
        }
    }
}
