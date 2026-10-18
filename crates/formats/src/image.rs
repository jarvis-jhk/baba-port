//! Plain RGBA8 images and PNG decoding/encoding.

use std::io::Cursor;
use std::path::Path;

use crate::error::{Error, Result, read_file};

/// A straight-alpha RGBA8 image, rows top to bottom.
#[derive(Clone, PartialEq, Eq)]
pub struct RgbaImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes, RGBA.
    pub pixels: Vec<u8>,
}

impl std::fmt::Debug for RgbaImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RgbaImage({}x{})", self.width, self.height)
    }
}

impl RgbaImage {
    /// A transparent image.
    pub fn new(width: u32, height: u32) -> Self {
        RgbaImage {
            width,
            height,
            pixels: vec![0; width as usize * height as usize * 4],
        }
    }

    /// Pixel at (x, y); panics when out of bounds.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        [
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        ]
    }

    /// Sets the pixel at (x, y); ignores out-of-bounds coordinates.
    pub fn set_pixel(&mut self, x: u32, y: u32, p: [u8; 4]) {
        if x < self.width && y < self.height {
            let i = (y as usize * self.width as usize + x as usize) * 4;
            self.pixels[i..i + 4].copy_from_slice(&p);
        }
    }

    /// Decodes a PNG (any colour type; `tRNS` becomes alpha, 16-bit is
    /// reduced to 8-bit).
    pub fn decode_png(bytes: &[u8]) -> Result<RgbaImage> {
        let err = |e: png::DecodingError| Error::decode("PNG", e.to_string());
        let mut dec = png::Decoder::new(Cursor::new(bytes));
        dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = dec.read_info().map_err(err)?;
        let size = reader
            .output_buffer_size()
            .ok_or_else(|| Error::decode("PNG", "image too large"))?;
        let mut buf = vec![0; size];
        let info = reader.next_frame(&mut buf).map_err(err)?;
        buf.truncate(info.buffer_size());
        let (w, h) = (info.width, info.height);
        let n = w as usize * h as usize;
        let pixels = match info.color_type {
            png::ColorType::Rgba => buf,
            png::ColorType::Rgb => {
                let mut out = Vec::with_capacity(n * 4);
                for p in buf.as_chunks::<3>().0 {
                    out.extend_from_slice(&[p[0], p[1], p[2], 255]);
                }
                out
            }
            png::ColorType::GrayscaleAlpha => {
                let mut out = Vec::with_capacity(n * 4);
                for p in buf.as_chunks::<2>().0 {
                    out.extend_from_slice(&[p[0], p[0], p[0], p[1]]);
                }
                out
            }
            png::ColorType::Grayscale => {
                let mut out = Vec::with_capacity(n * 4);
                for &g in &buf {
                    out.extend_from_slice(&[g, g, g, 255]);
                }
                out
            }
            png::ColorType::Indexed => {
                return Err(Error::decode("PNG", "palette not expanded"));
            }
        };
        if pixels.len() != n * 4 {
            return Err(Error::decode("PNG", "unexpected buffer size"));
        }
        Ok(RgbaImage {
            width: w,
            height: h,
            pixels,
        })
    }

    /// Reads and decodes a PNG file.
    pub fn load_png(path: &Path) -> Result<RgbaImage> {
        RgbaImage::decode_png(&read_file(path)?).map_err(|e| e.at(path))
    }

    /// Encodes as an RGBA PNG.
    pub fn encode_png(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, self.width, self.height);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_compression(png::Compression::Fast);
            let err = |e: png::EncodingError| Error::decode("PNG (encode)", e.to_string());
            let mut w = enc.write_header().map_err(err)?;
            w.write_image_data(&self.pixels).map_err(err)?;
        }
        Ok(out)
    }

    /// Encodes and writes a PNG file.
    pub fn save_png(&self, path: &Path) -> Result<()> {
        let bytes = self.encode_png()?;
        std::fs::write(path, bytes).map_err(|e| Error::io(path, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_roundtrip() {
        let mut img = RgbaImage::new(3, 2);
        img.set_pixel(1, 1, [10, 20, 30, 40]);
        let bytes = img.encode_png().unwrap();
        let back = RgbaImage::decode_png(&bytes).unwrap();
        assert_eq!(back, img);
        assert!(RgbaImage::decode_png(b"not a png").is_err());
    }
}
