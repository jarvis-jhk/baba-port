//! Level grids: the binary `NNNlevel.l` files.
//!
//! Layout (all little endian; full description in `docs/formats/levels.md`):
//!
//! ```text
//! "ACHTUNG!"  u16 version (0x0105)
//! chunk*:     4-byte tag, u32 size, payload
//!   "MAP "    2 bytes (always 0)
//!   "LAYR"    u16 layer count, then per layer:
//!               u32 width, u32 height (cells, border included)
//!               32 bytes layer parameters (constant in shipped files)
//!               u8  number of data blocks that follow (2, or 1 = no DATA)
//!               "MAIN" u32 size, zlib(width*height u16 tile ids)
//!               "DATA" u32 count, count x { u8 kind, u32 size, zlib(width*height u8) }
//! ```
//!
//! Data kind 0 holds each cell's facing [`Direction`].

use std::path::Path;

use crate::bytes::{Reader, zlib_compress, zlib_decompress};
use crate::error::{Error, Result, read_file};
use crate::objects::TileId;

const MAGIC: &[u8; 8] = b"ACHTUNG!";
const FORMAT: &str = "level";

/// The only level file version seen (and written by the game's editor).
pub const LEVEL_VERSION: u16 = 0x0105;

/// The 32 parameter bytes every layer in the shipped levels carries.
pub const DEFAULT_LAYER_PARAMS: [u8; 32] = [
    0x0c, 0x00, 0x0c, 0x00, 0x00, 0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x80, 0x3f, 0x00, 0x00, 0x80, 0x3f, 0x00, 0x00, 0x01, 0x00, 0x00, 0x80, 0x3f, 0xff, 0xff, 0xff,
];

/// Facing direction stored per cell (and used for sprite variants).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Direction {
    /// 0
    Right = 0,
    /// 1
    Up = 1,
    /// 2
    Left = 2,
    /// 3
    Down = 3,
}

impl Direction {
    /// From the stored byte (values above 3 wrap modulo 4).
    pub fn from_u8(v: u8) -> Direction {
        match v & 3 {
            0 => Direction::Right,
            1 => Direction::Up,
            2 => Direction::Left,
            _ => Direction::Down,
        }
    }
    /// Grid step `(dx, dy)`, y growing downwards.
    pub fn delta(self) -> (i32, i32) {
        match self {
            Direction::Right => (1, 0),
            Direction::Up => (0, -1),
            Direction::Left => (-1, 0),
            Direction::Down => (0, 1),
        }
    }
}

/// One tile layer of a level.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    /// Width in cells, including the one-cell border.
    pub width: u32,
    /// Height in cells, including the one-cell border.
    pub height: u32,
    /// Layer parameter bytes (see [`DEFAULT_LAYER_PARAMS`]); kept verbatim.
    pub params: [u8; 32],
    /// Row-major tile ids (`width * height`).
    pub tiles: Vec<TileId>,
    /// Row-major direction bytes (data kind 0), if the layer has a DATA block.
    pub dirs: Option<Vec<u8>>,
    /// Data blocks of other kinds `(kind, bytes)` (none in shipped files).
    pub extra_data: Vec<(u8, Vec<u8>)>,
}

impl Layer {
    /// An empty layer.
    pub fn new(width: u32, height: u32) -> Layer {
        let n = (width * height) as usize;
        Layer {
            width,
            height,
            params: DEFAULT_LAYER_PARAMS,
            tiles: vec![TileId::EMPTY; n],
            dirs: Some(vec![0; n]),
            extra_data: Vec::new(),
        }
    }

    /// Tile at (x, y), `None` outside the layer.
    pub fn tile(&self, x: u32, y: u32) -> Option<TileId> {
        (x < self.width && y < self.height).then(|| self.tiles[(y * self.width + x) as usize])
    }

    /// Facing at (x, y) (Right if the layer has no direction data).
    pub fn dir(&self, x: u32, y: u32) -> Direction {
        let i = (y * self.width + x) as usize;
        Direction::from_u8(
            self.dirs
                .as_ref()
                .and_then(|d| d.get(i).copied())
                .unwrap_or(0),
        )
    }

    /// Non-empty cells as `(x, y, tile, dir)`, row-major.
    pub fn objects(&self) -> impl Iterator<Item = (u32, u32, TileId, Direction)> + '_ {
        self.tiles.iter().enumerate().filter_map(move |(i, &t)| {
            if t.is_empty() {
                return None;
            }
            let (x, y) = (i as u32 % self.width, i as u32 / self.width);
            Some((x, y, t, self.dir(x, y)))
        })
    }
}

/// A parsed `.l` file.
#[derive(Clone, Debug, PartialEq)]
pub struct LevelMap {
    /// File version (`0x0105`).
    pub version: u16,
    /// Payload of the `MAP ` chunk (2 zero bytes).
    pub map: Vec<u8>,
    /// Tile layers, bottom first (up to 3).
    pub layers: Vec<Layer>,
    /// Unknown chunks `(tag, payload)`, kept for round trips.
    pub other_chunks: Vec<([u8; 4], Vec<u8>)>,
}

impl LevelMap {
    /// An empty level with one layer.
    pub fn new(width: u32, height: u32) -> LevelMap {
        LevelMap {
            version: LEVEL_VERSION,
            map: vec![0, 0],
            layers: vec![Layer::new(width, height)],
            other_chunks: Vec::new(),
        }
    }

    /// Width of the first layer.
    pub fn width(&self) -> u32 {
        self.layers.first().map_or(0, |l| l.width)
    }

    /// Height of the first layer.
    pub fn height(&self) -> u32 {
        self.layers.first().map_or(0, |l| l.height)
    }

    /// Reads a `.l` file.
    pub fn load(path: &Path) -> Result<LevelMap> {
        LevelMap::parse(&read_file(path)?).map_err(|e| e.at(path))
    }

    /// Parses `.l` bytes.
    pub fn parse(bytes: &[u8]) -> Result<LevelMap> {
        let mut r = Reader::new(bytes, FORMAT);
        if r.bytes(8).ok() != Some(MAGIC.as_slice()) {
            return Err(Error::malformed(
                FORMAT,
                Some(0),
                "missing ACHTUNG! signature",
            ));
        }
        let version = r.u16()?;
        let mut level = LevelMap {
            version,
            map: Vec::new(),
            layers: Vec::new(),
            other_chunks: Vec::new(),
        };
        let mut saw_layers = false;
        while !r.is_empty() {
            let tag: [u8; 4] = r.array()?;
            let size = r.u32()? as usize;
            let start = r.pos();
            let payload = r.bytes(size)?;
            match &tag {
                b"MAP " => level.map = payload.to_vec(),
                b"LAYR" => {
                    level.layers = parse_layers(payload).map_err(|mut e| {
                        if let crate::ErrorKind::Malformed {
                            offset: Some(o), ..
                        } = &mut e.kind
                        {
                            *o += start;
                        }
                        e
                    })?;
                    saw_layers = true;
                }
                _ => level.other_chunks.push((tag, payload.to_vec())),
            }
        }
        if !saw_layers {
            return Err(Error::malformed(FORMAT, None, "no LAYR chunk"));
        }
        Ok(level)
    }

    /// Serialises to `.l` bytes (zlib streams are re-compressed, so the
    /// bytes differ from the original file but parse identically).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.version.to_le_bytes());
        push_chunk(&mut out, b"MAP ", &self.map);
        let mut l = Vec::new();
        l.extend_from_slice(&(self.layers.len() as u16).to_le_bytes());
        for layer in &self.layers {
            l.extend_from_slice(&layer.width.to_le_bytes());
            l.extend_from_slice(&layer.height.to_le_bytes());
            l.extend_from_slice(&layer.params);
            let has_data = layer.dirs.is_some() || !layer.extra_data.is_empty();
            l.push(if has_data { 2 } else { 1 });
            let raw: Vec<u8> = layer.tiles.iter().flat_map(|t| t.0.to_le_bytes()).collect();
            let z = zlib_compress(&raw);
            l.extend_from_slice(b"MAIN");
            l.extend_from_slice(&(z.len() as u32).to_le_bytes());
            l.extend_from_slice(&z);
            if has_data {
                let mut blocks: Vec<(u8, &[u8])> = Vec::new();
                if let Some(d) = &layer.dirs {
                    blocks.push((0, d));
                }
                for (k, d) in &layer.extra_data {
                    blocks.push((*k, d));
                }
                l.extend_from_slice(b"DATA");
                l.extend_from_slice(&(blocks.len() as u32).to_le_bytes());
                for (k, d) in blocks {
                    let z = zlib_compress(d);
                    l.push(k);
                    l.extend_from_slice(&(z.len() as u32).to_le_bytes());
                    l.extend_from_slice(&z);
                }
            }
        }
        push_chunk(&mut out, b"LAYR", &l);
        for (tag, p) in &self.other_chunks {
            push_chunk(&mut out, tag, p);
        }
        out
    }
}

fn push_chunk(out: &mut Vec<u8>, tag: &[u8; 4], payload: &[u8]) {
    out.extend_from_slice(tag);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
}

fn parse_layers(p: &[u8]) -> Result<Vec<Layer>> {
    let mut r = Reader::new(p, FORMAT);
    let count = r.u16()?;
    let mut layers = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let width = r.u32()?;
        let height = r.u32()?;
        if width == 0 || height == 0 || width > 4096 || height > 4096 {
            return Err(r.err(format!("implausible layer size {width}x{height}")));
        }
        let cells = (width * height) as usize;
        let params: [u8; 32] = r.array()?;
        let blocks = r.u8()?;
        if r.array::<4>()? != *b"MAIN" {
            return Err(r.err("expected MAIN block"));
        }
        let size = r.u32()? as usize;
        let raw = zlib_decompress(r.bytes(size)?, "level MAIN block")?;
        if raw.len() != cells * 2 {
            return Err(r.err(format!(
                "MAIN holds {} bytes, expected {}",
                raw.len(),
                cells * 2
            )));
        }
        let tiles = raw
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| TileId(u16::from_le_bytes([c[0], c[1]])))
            .collect();
        let mut layer = Layer {
            width,
            height,
            params,
            tiles,
            dirs: None,
            extra_data: Vec::new(),
        };
        if blocks >= 2 || r.peek(4) == Some(b"DATA") {
            if r.array::<4>()? != *b"DATA" {
                return Err(r.err("expected DATA block"));
            }
            let n = r.u32()?;
            if n > 64 {
                return Err(r.err(format!("implausible DATA count {n}")));
            }
            for _ in 0..n {
                let kind = r.u8()?;
                let size = r.u32()? as usize;
                let d = zlib_decompress(r.bytes(size)?, "level DATA block")?;
                if kind == 0 && layer.dirs.is_none() {
                    if d.len() != cells {
                        return Err(r.err(format!(
                            "direction data holds {} bytes, expected {cells}",
                            d.len()
                        )));
                    }
                    layer.dirs = Some(d);
                } else {
                    layer.extra_data.push((kind, d));
                }
            }
        }
        layers.push(layer);
    }
    if !r.is_empty() {
        return Err(r.err(format!("{} trailing bytes after layers", r.remaining())));
    }
    Ok(layers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut m = LevelMap::new(5, 4);
        m.layers[0].tiles[6] = TileId::from_xy(1, 0);
        m.layers[0].dirs.as_mut().unwrap()[6] = 3;
        let mut l2 = Layer::new(5, 4);
        l2.dirs = None;
        l2.tiles[0] = TileId::EDGE;
        m.layers.push(l2);
        let bytes = m.to_bytes();
        let back = LevelMap::parse(&bytes).unwrap();
        assert_eq!(back, m);
        let objs: Vec<_> = back.layers[0].objects().collect();
        assert_eq!(objs, vec![(1, 1, TileId(1), Direction::Down)]);
        assert_eq!(back.layers[1].dir(0, 0), Direction::Right);
    }

    #[test]
    fn errors() {
        assert!(LevelMap::parse(b"NOTALEVEL").is_err());
        let m = LevelMap::new(3, 3);
        let mut b = m.to_bytes();
        b.truncate(b.len() - 3);
        let e = LevelMap::parse(&b).unwrap_err();
        assert!(e.to_string().contains("malformed level"), "{e}");
    }
}
