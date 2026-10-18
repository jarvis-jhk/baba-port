//! `Assets.dat`: the packed asset file of the Chowdren builds (Linux,
//! Windows, macOS).
//!
//! It holds the engine-side images (UI, fonts as bitmaps, effects, the
//! Fusion-side sprites), the sound effects (Ogg Vorbis) and the shaders
//! (GLSL source on Linux/macOS, compiled Direct3D 9 bytecode on Windows;
//! the images and sounds are byte-identical in all three).
//! Assets have no names in the file; they are addressed by index. Layout
//! (details and evidence in `docs/formats/assets-dat.md`):
//!
//! ```text
//! header:  N x { u32 offset, u32 size }      one entry per asset
//!          K x   u32 region size             per asset kind, in kind order
//! data:    the assets, kind by kind (images, sounds, fonts, shaders, files, ...)
//! ```
//!
//! The header length is the offset of the first asset; `N` and `K` follow
//! from `8N + 4K = header length` and `sum(region sizes) = file length -
//! header length`.

use std::path::Path;

use crate::bytes::Reader;
use crate::error::{Error, Result, read_file};
use crate::image::RgbaImage;
use crate::lz4;

const FORMAT: &str = "Assets.dat";

/// Kind of an asset, from the region it lies in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AssetKind {
    /// Region 0: images.
    Image,
    /// Region 1: sounds.
    Sound,
    /// Region 2: fonts (empty in the shipped file).
    Font,
    /// Region 3: shaders.
    Shader,
    /// Region 4: other files (empty in the shipped file).
    File,
    /// Any further region.
    Other(u8),
}

impl AssetKind {
    fn from_region(i: usize) -> AssetKind {
        match i {
            0 => AssetKind::Image,
            1 => AssetKind::Sound,
            2 => AssetKind::Font,
            3 => AssetKind::Shader,
            4 => AssetKind::File,
            n => AssetKind::Other(n as u8),
        }
    }
}

/// One header entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AssetEntry {
    /// Global index in the header.
    pub index: usize,
    /// Kind (from the region).
    pub kind: AssetKind,
    /// Byte offset in the file.
    pub offset: u32,
    /// Byte size.
    pub size: u32,
}

/// Fixed fields of an image asset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageInfo {
    /// Width in pixels.
    pub width: u16,
    /// Height in pixels.
    pub height: u16,
    /// Hot spot (origin) in pixels.
    pub hotspot: (i16, i16),
    /// Action point in pixels.
    pub action_point: (i16, i16),
    /// Flag byte (0 normal, 1 seen on solid fills, 2 = no pixels).
    pub flags: u8,
}

/// A decoded image asset.
#[derive(Clone, Debug)]
pub struct AssetImage {
    /// Header fields.
    pub info: ImageInfo,
    /// Pixels (empty 0x0 image for assets without pixel data).
    pub image: RgbaImage,
}

/// A sound asset.
#[derive(Clone, Copy, Debug)]
pub struct AssetSound<'a> {
    /// Format code (2 = Ogg Vorbis, the only one present).
    pub format: u32,
    /// Encoded sound file bytes.
    pub data: &'a [u8],
}

/// What the shader entries hold; tells the builds' files apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderLanguage {
    /// GLSL source (`#version 120`): the Linux and macOS builds.
    Glsl,
    /// Compiled Direct3D 9 shader model 3 bytecode (`vs_3_0` / `ps_3_0`):
    /// the Windows build.
    D3d9,
}

impl ShaderLanguage {
    fn detect(vertex: &[u8]) -> ShaderLanguage {
        // D3D9 version token: minor, major, 0xFFFE (vertex shader).
        if vertex.len() >= 4 && vertex[2..4] == [0xfe, 0xff] {
            ShaderLanguage::D3d9
        } else {
            ShaderLanguage::Glsl
        }
    }
}

/// A shader asset: a vertex and a fragment (pixel) program.
#[derive(Clone, Debug)]
pub struct ShaderSource {
    /// GLSL source or D3D9 bytecode.
    pub language: ShaderLanguage,
    /// Vertex program bytes.
    pub vertex: Vec<u8>,
    /// Fragment (pixel) program bytes.
    pub fragment: Vec<u8>,
}

impl ShaderSource {
    /// The vertex shader as text (GLSL only).
    pub fn vertex_text(&self) -> Option<&str> {
        self.text(&self.vertex)
    }

    /// The fragment shader as text (GLSL only).
    pub fn fragment_text(&self) -> Option<&str> {
        self.text(&self.fragment)
    }

    fn text<'a>(&self, b: &'a [u8]) -> Option<&'a str> {
        (self.language == ShaderLanguage::Glsl)
            .then(|| std::str::from_utf8(b).ok())
            .flatten()
    }
}

/// An opened `Assets.dat`.
pub struct AssetsDat {
    data: Vec<u8>,
    /// All header entries in order.
    pub entries: Vec<AssetEntry>,
    /// Region sizes per kind, in kind order.
    pub region_sizes: Vec<u32>,
}

impl std::fmt::Debug for AssetsDat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssetsDat")
            .field("len", &self.data.len())
            .field("entries", &self.entries.len())
            .field("region_sizes", &self.region_sizes)
            .finish()
    }
}

impl AssetsDat {
    /// Reads and indexes the file (it is kept in memory, ~21 MB).
    pub fn load(path: &Path) -> Result<AssetsDat> {
        AssetsDat::parse(read_file(path)?).map_err(|e| e.at(path))
    }

    /// Indexes file bytes.
    pub fn parse(data: Vec<u8>) -> Result<AssetsDat> {
        let len = data.len();
        if len < 4 {
            return Err(Error::malformed(FORMAT, Some(0), "file too short"));
        }
        let header = u32::from_le_bytes(data[0..4].try_into().expect("4 bytes")) as usize;
        if header < 12 || header > len {
            return Err(Error::malformed(FORMAT, Some(0), "bad header length"));
        }
        let u32_at = |o: usize| u32::from_le_bytes(data[o..o + 4].try_into().expect("4 bytes"));
        // Find K (number of kind regions) so that the trailing K sizes add up.
        let mut found = None;
        for k in 1..=16usize {
            if 4 * k >= header || !(header - 4 * k).is_multiple_of(8) {
                continue;
            }
            let sum: u64 = (0..k).map(|i| u32_at(header - 4 * k + 4 * i) as u64).sum();
            if sum == (len - header) as u64 {
                found = Some(k);
                break;
            }
        }
        let k = found.ok_or_else(|| {
            Error::malformed(FORMAT, Some(0), "region sizes do not match the file length")
        })?;
        let n = (header - 4 * k) / 8;
        let region_sizes: Vec<u32> = (0..k).map(|i| u32_at(header - 4 * k + 4 * i)).collect();
        let mut bounds = Vec::with_capacity(k);
        let mut start = header as u64;
        for &s in &region_sizes {
            bounds.push((start, start + s as u64));
            start += s as u64;
        }
        let mut entries = Vec::with_capacity(n);
        for i in 0..n {
            let offset = u32_at(i * 8);
            let size = u32_at(i * 8 + 4);
            let (o, e) = (offset as u64, offset as u64 + size as u64);
            let region = bounds
                .iter()
                .position(|&(a, b)| o >= a && e <= b && o < b)
                .ok_or_else(|| {
                    Error::malformed(
                        FORMAT,
                        Some(i * 8),
                        format!("entry {i} ({offset:#x}+{size}) outside every region"),
                    )
                })?;
            entries.push(AssetEntry {
                index: i,
                kind: AssetKind::from_region(region),
                offset,
                size,
            });
        }
        Ok(AssetsDat {
            data,
            entries,
            region_sizes,
        })
    }

    /// Raw bytes of an entry.
    pub fn raw(&self, entry: &AssetEntry) -> &[u8] {
        &self.data[entry.offset as usize..(entry.offset + entry.size) as usize]
    }

    /// Entries of one kind, in order. The position in this list is the
    /// per-kind index the engine uses.
    pub fn of_kind(&self, kind: AssetKind) -> impl Iterator<Item = &AssetEntry> {
        self.entries.iter().filter(move |e| e.kind == kind)
    }

    /// Header of an image asset.
    pub fn image_info(&self, entry: &AssetEntry) -> Result<ImageInfo> {
        let read = || -> Result<ImageInfo> {
            let mut r = Reader::new(self.raw(entry), FORMAT);
            Ok(ImageInfo {
                width: r.u16()?,
                height: r.u16()?,
                hotspot: (r.i16()?, r.i16()?),
                action_point: (r.i16()?, r.i16()?),
                flags: r.u8()?,
            })
        };
        read().map_err(|e| with_entry(e, entry))
    }

    /// Decodes an image asset to RGBA.
    pub fn image(&self, entry: &AssetEntry) -> Result<AssetImage> {
        self.image_inner(entry).map_err(|e| with_entry(e, entry))
    }

    fn image_inner(&self, entry: &AssetEntry) -> Result<AssetImage> {
        if entry.kind != AssetKind::Image {
            return Err(Error::malformed(FORMAT, None, "not an image"));
        }
        let info = self.image_info(entry)?;
        let mut r = Reader::new(self.raw(entry), FORMAT);
        r.bytes(13)?;
        let total = r.u32()? as usize;
        if total != r.remaining() {
            return Err(r.err(format!(
                "pixel stream is {total} bytes, entry has {}",
                r.remaining()
            )));
        }
        let expected = info.width as usize * info.height as usize * 4;
        if total == 0 {
            return Ok(AssetImage {
                info,
                image: RgbaImage::new(0, 0),
            });
        }
        let mut pixels = Vec::with_capacity(expected);
        while !r.is_empty() {
            let csize = r.u32()? as usize;
            let block = r.bytes(csize)?;
            let room = expected.saturating_sub(pixels.len()).min(65536);
            lz4::decompress_block(block, &mut pixels, room)?;
        }
        if pixels.len() != expected {
            return Err(Error::decode(
                FORMAT,
                format!(
                    "image decoded to {} bytes, expected {expected}",
                    pixels.len()
                ),
            ));
        }
        Ok(AssetImage {
            info,
            image: RgbaImage {
                width: info.width as u32,
                height: info.height as u32,
                pixels,
            },
        })
    }

    /// A sound asset.
    pub fn sound(&self, entry: &AssetEntry) -> Result<AssetSound<'_>> {
        let raw = self.raw(entry);
        let mut r = Reader::new(raw, FORMAT);
        let format = r.u32()?;
        let n = r.u32()? as usize;
        let data = r.bytes(n).map_err(|e| with_entry(e, entry))?;
        Ok(AssetSound { format, data })
    }

    /// A shader asset: `u32 length, vertex program, u32 length, fragment
    /// program` (a fragment part without its length prefix is accepted too).
    pub fn shader(&self, entry: &AssetEntry) -> Result<ShaderSource> {
        let read = || -> Result<ShaderSource> {
            let mut r = Reader::new(self.raw(entry), FORMAT);
            let n = r.u32()? as usize;
            let vertex = r.bytes(n)?.to_vec();
            let rest = r.bytes(r.remaining())?;
            let fragment = match rest.get(..4) {
                Some(len)
                    if u32::from_le_bytes(len.try_into().expect("4")) as usize + 4
                        == rest.len() =>
                {
                    &rest[4..]
                }
                _ => rest,
            };
            Ok(ShaderSource {
                language: ShaderLanguage::detect(&vertex),
                vertex,
                fragment: fragment.to_vec(),
            })
        };
        read().map_err(|e| with_entry(e, entry))
    }

    /// The shader language of the file (`None` without shaders): GLSL in
    /// the Linux and macOS builds, D3D9 bytecode in the Windows build.
    pub fn shader_language(&self) -> Option<ShaderLanguage> {
        let e = self.of_kind(AssetKind::Shader).next()?;
        self.shader(e).ok().map(|s| s.language)
    }
}

fn with_entry(mut e: Error, entry: &AssetEntry) -> Error {
    if let crate::ErrorKind::Malformed { message, .. } | crate::ErrorKind::Decode { message, .. } =
        &mut e.kind
    {
        *message = format!("asset {}: {message}", entry.index);
    }
    e
}

/// Builds an `Assets.dat` image entry (used by tests; LZ4 "literals only").
#[cfg(test)]
pub(crate) fn encode_test_image(w: u16, h: u16, pixels: &[u8]) -> Vec<u8> {
    let mut e = Vec::new();
    for v in [w as i16, h as i16, 1, 2, 3, 4] {
        e.extend_from_slice(&v.to_le_bytes());
    }
    e.push(0);
    let mut stream = Vec::new();
    for chunk in pixels.chunks(65536) {
        // one literal-only LZ4 sequence
        let mut block = Vec::new();
        let n = chunk.len();
        if n >= 15 {
            block.push(0xF0);
            let mut rest = n - 15;
            while rest >= 255 {
                block.push(255);
                rest -= 255;
            }
            block.push(rest as u8);
        } else {
            block.push((n as u8) << 4);
        }
        block.extend_from_slice(chunk);
        stream.extend_from_slice(&(block.len() as u32).to_le_bytes());
        stream.extend_from_slice(&block);
    }
    e.extend_from_slice(&(stream.len() as u32).to_le_bytes());
    e.extend_from_slice(&stream);
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(entries: &[(usize, Vec<u8>)], regions: usize) -> Vec<u8> {
        let n = entries.len();
        let header = 8 * n + 4 * regions;
        let mut body = Vec::new();
        let mut table = Vec::new();
        let mut sizes = vec![0u32; regions];
        for (region, data) in entries {
            table.push(((header + body.len()) as u32, data.len() as u32));
            sizes[*region] += data.len() as u32;
            body.extend_from_slice(data);
        }
        let mut out = Vec::new();
        for (o, s) in table {
            out.extend_from_slice(&o.to_le_bytes());
            out.extend_from_slice(&s.to_le_bytes());
        }
        for s in sizes {
            out.extend_from_slice(&s.to_le_bytes());
        }
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn synthetic_file() {
        let px: Vec<u8> = (0..(300 * 300 * 4)).map(|i| (i % 251) as u8).collect();
        let img = encode_test_image(300, 300, &px);
        let mut snd = 2u32.to_le_bytes().to_vec();
        snd.extend_from_slice(&4u32.to_le_bytes());
        snd.extend_from_slice(b"OggS");
        let mut sh = 3u32.to_le_bytes().to_vec();
        sh.extend_from_slice(b"vvv");
        sh.extend_from_slice(&3u32.to_le_bytes());
        sh.extend_from_slice(b"fff");
        let mut d3d = 4u32.to_le_bytes().to_vec();
        d3d.extend_from_slice(&[0, 3, 0xfe, 0xff]);
        d3d.extend_from_slice(b"ps");
        let file = build(&[(0, img), (1, snd), (3, sh), (3, d3d)], 6);
        let a = AssetsDat::parse(file).unwrap();
        assert_eq!(a.entries.len(), 4);
        assert_eq!(a.region_sizes.len(), 6);
        let e = a.entries[0];
        assert_eq!(e.kind, AssetKind::Image);
        let im = a.image(&e).unwrap();
        assert_eq!(im.info.hotspot, (1, 2));
        assert_eq!(im.image.pixels, px);
        let s = a.sound(&a.entries[1]).unwrap();
        assert_eq!((s.format, s.data), (2, b"OggS".as_slice()));
        let sh = a.shader(&a.entries[2]).unwrap();
        assert_eq!(sh.language, ShaderLanguage::Glsl);
        assert_eq!(
            (sh.vertex_text(), sh.fragment_text()),
            (Some("vvv"), Some("fff"))
        );
        assert_eq!(a.shader_language(), Some(ShaderLanguage::Glsl));
        let d = a.shader(&a.entries[3]).unwrap();
        assert_eq!(d.language, ShaderLanguage::D3d9);
        assert_eq!(
            (d.fragment.as_slice(), d.vertex_text()),
            (b"ps".as_slice(), None)
        );
        assert!(a.image(&a.entries[1]).is_err());
    }

    #[test]
    fn rejects_garbage() {
        assert!(AssetsDat::parse(vec![1, 2, 3]).is_err());
        assert!(AssetsDat::parse(vec![0xff; 64]).is_err());
    }
}
