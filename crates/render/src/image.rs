//! RGBA8 images: PNG load/save, crops, scaling, and the image comparison
//! used by tests and by the comparison against captures of the original.

use std::io::Cursor;
use std::path::Path;

/// A straight-alpha RGBA8 image, rows top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

#[derive(Debug)]
pub enum ImageError {
    Io(std::io::Error),
    Decode(String),
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageError::Io(e) => write!(f, "{e}"),
            ImageError::Decode(e) => write!(f, "PNG: {e}"),
        }
    }
}

impl std::error::Error for ImageError {}

impl From<std::io::Error> for ImageError {
    fn from(e: std::io::Error) -> Self {
        ImageError::Io(e)
    }
}

impl RgbaImage {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0; width as usize * height as usize * 4],
        }
    }

    pub fn filled(width: u32, height: u32, rgba: [u8; 4]) -> Self {
        let mut img = Self::new(width, height);
        for px in img.data.as_chunks_mut::<4>().0 {
            px.copy_from_slice(&rgba);
        }
        img
    }

    /// Decodes a PNG of any colour type to RGBA8. A `tRNS` colour key (the
    /// game's RGB sprites and palettes use black) becomes alpha 0.
    pub fn decode_png(bytes: &[u8]) -> Result<Self, ImageError> {
        let mut dec = png::Decoder::new(Cursor::new(bytes));
        dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = dec
            .read_info()
            .map_err(|e| ImageError::Decode(e.to_string()))?;
        let size = reader
            .output_buffer_size()
            .ok_or_else(|| ImageError::Decode("image too large".into()))?;
        let mut buf = vec![0; size];
        let info = reader
            .next_frame(&mut buf)
            .map_err(|e| ImageError::Decode(e.to_string()))?;
        let (w, h) = (info.width, info.height);
        let n = w as usize * h as usize;
        let src = &buf[..info.buffer_size()];
        let data = match info.color_type {
            png::ColorType::Rgba => src[..n * 4].to_vec(),
            png::ColorType::Rgb => src[..n * 3]
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect(),
            png::ColorType::GrayscaleAlpha => src[..n * 2]
                .as_chunks::<2>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[0], p[0], p[1]])
                .collect(),
            png::ColorType::Grayscale => src[..n].iter().flat_map(|&g| [g, g, g, 255]).collect(),
            png::ColorType::Indexed => {
                return Err(ImageError::Decode("palette not expanded".into()));
            }
        };
        Ok(Self {
            width: w,
            height: h,
            data,
        })
    }

    pub fn load_png(path: impl AsRef<Path>) -> Result<Self, ImageError> {
        Self::decode_png(&std::fs::read(path)?)
    }

    pub fn encode_png(&self) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, self.width, self.height);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().expect("PNG header to memory");
            w.write_image_data(&self.data).expect("PNG data to memory");
        }
        out
    }

    pub fn save_png(&self, path: impl AsRef<Path>) -> Result<(), ImageError> {
        std::fs::write(path, self.encode_png())?;
        Ok(())
    }

    #[inline]
    pub fn get(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        [
            self.data[i],
            self.data[i + 1],
            self.data[i + 2],
            self.data[i + 3],
        ]
    }

    #[inline]
    pub fn put(&mut self, x: u32, y: u32, px: [u8; 4]) {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        self.data[i..i + 4].copy_from_slice(&px);
    }

    /// Flips rows (GL read-back is bottom-up).
    pub fn flip_vertical(&mut self) {
        let row = self.width as usize * 4;
        let h = self.height as usize;
        for y in 0..h / 2 {
            let (a, b) = self.data.split_at_mut((h - 1 - y) * row);
            a[y * row..y * row + row].swap_with_slice(&mut b[..row]);
        }
    }

    /// The sub-image at (x, y) of size w x h (clipped to the image).
    pub fn crop(&self, x: u32, y: u32, w: u32, h: u32) -> Self {
        let w = w.min(self.width.saturating_sub(x));
        let h = h.min(self.height.saturating_sub(y));
        let mut out = Self::new(w, h);
        for row in 0..h {
            let s = ((y + row) as usize * self.width as usize + x as usize) * 4;
            let d = row as usize * w as usize * 4;
            out.data[d..d + w as usize * 4].copy_from_slice(&self.data[s..s + w as usize * 4]);
        }
        out
    }

    /// Box-filtered resize (area average); exact for integer down-scales and
    /// good enough for comparing captures at different resolutions.
    pub fn resize_area(&self, w: u32, h: u32) -> Self {
        let mut out = Self::new(w, h);
        let sx = self.width as f64 / w as f64;
        let sy = self.height as f64 / h as f64;
        for oy in 0..h {
            let y0 = oy as f64 * sy;
            let y1 = y0 + sy;
            for ox in 0..w {
                let x0 = ox as f64 * sx;
                let x1 = x0 + sx;
                let mut acc = [0.0f64; 4];
                let mut total = 0.0;
                let mut y = y0.floor() as u32;
                while (y as f64) < y1 && y < self.height {
                    let wy = (y1.min(y as f64 + 1.0) - y0.max(y as f64)).max(0.0);
                    let mut x = x0.floor() as u32;
                    while (x as f64) < x1 && x < self.width {
                        let wx = (x1.min(x as f64 + 1.0) - x0.max(x as f64)).max(0.0);
                        let wgt = wx * wy;
                        let p = self.get(x, y);
                        for c in 0..4 {
                            acc[c] += p[c] as f64 * wgt;
                        }
                        total += wgt;
                        x += 1;
                    }
                    y += 1;
                }
                let px = acc.map(|a| (a / total.max(1e-9)).round().clamp(0.0, 255.0) as u8);
                out.put(ox, oy, px);
            }
        }
        out
    }

    /// Nearest-neighbour integer up-scale.
    pub fn scale_nearest(&self, factor: u32) -> Self {
        let mut out = Self::new(self.width * factor, self.height * factor);
        for y in 0..out.height {
            for x in 0..out.width {
                out.put(x, y, self.get(x / factor, y / factor));
            }
        }
        out
    }
}

/// Result of comparing two images of the same size (RGB channels; alpha is
/// ignored because captures are opaque).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiffStats {
    pub width: u32,
    pub height: u32,
    /// Mean absolute error per channel, 0..255.
    pub mean_abs: f64,
    /// Largest per-channel error.
    pub max_abs: u8,
    /// Pixels where any channel differs by more than the threshold.
    pub differing: u64,
    /// `differing` / pixel count.
    pub differing_fraction: f64,
    /// Peak signal-to-noise ratio in dB (infinite when identical).
    pub psnr: f64,
}

impl std::fmt::Display for DiffStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}x{}: mean |d| {:.2}, max {}, {:.2}% px differ, PSNR {:.1} dB",
            self.width,
            self.height,
            self.mean_abs,
            self.max_abs,
            self.differing_fraction * 100.0,
            self.psnr
        )
    }
}

/// Compares `a` and `b` (same size). A pixel "differs" when any RGB channel
/// is off by more than `threshold`.
pub fn diff(a: &RgbaImage, b: &RgbaImage, threshold: u8) -> DiffStats {
    assert_eq!(
        (a.width, a.height),
        (b.width, b.height),
        "diff: image sizes differ"
    );
    let mut sum = 0u64;
    let mut sq = 0u64;
    let mut max = 0u8;
    let mut differing = 0u64;
    for (p, q) in a
        .data
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.data.as_chunks::<4>().0)
    {
        let mut px_max = 0u8;
        for c in 0..3 {
            let d = p[c].abs_diff(q[c]);
            sum += d as u64;
            sq += d as u64 * d as u64;
            px_max = px_max.max(d);
        }
        max = max.max(px_max);
        if px_max > threshold {
            differing += 1;
        }
    }
    let n = a.width as u64 * a.height as u64;
    let samples = (n * 3).max(1) as f64;
    let mse = sq as f64 / samples;
    DiffStats {
        width: a.width,
        height: a.height,
        mean_abs: sum as f64 / samples,
        max_abs: max,
        differing,
        differing_fraction: differing as f64 / n.max(1) as f64,
        psnr: if mse == 0.0 {
            f64::INFINITY
        } else {
            10.0 * (255.0f64 * 255.0 / mse).log10()
        },
    }
}

/// A visual diff: `a` dimmed to grey, differing pixels in red (brighter =
/// larger error).
pub fn diff_image(a: &RgbaImage, b: &RgbaImage, threshold: u8) -> RgbaImage {
    let mut out = RgbaImage::new(a.width, a.height);
    for (i, (p, q)) in a
        .data
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.data.as_chunks::<4>().0)
        .enumerate()
    {
        let d = (0..3).map(|c| p[c].abs_diff(q[c])).max().unwrap_or(0);
        let px = if d > threshold {
            [128u8.saturating_add(d / 2), 0, 0, 255]
        } else {
            let g = ((p[0] as u32 + p[1] as u32 + p[2] as u32) / 9) as u8;
            [g, g, g, 255]
        };
        out.data[i * 4..i * 4 + 4].copy_from_slice(&px);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_roundtrip_and_flip() {
        let mut img = RgbaImage::new(3, 2);
        img.put(0, 0, [1, 2, 3, 4]);
        img.put(2, 1, [9, 8, 7, 6]);
        let back = RgbaImage::decode_png(&img.encode_png()).unwrap();
        assert_eq!(back, img);
        let mut f = img.clone();
        f.flip_vertical();
        assert_eq!(f.get(0, 1), [1, 2, 3, 4]);
        assert_eq!(f.get(2, 0), [9, 8, 7, 6]);
    }

    #[test]
    fn diff_counts() {
        let a = RgbaImage::filled(4, 4, [10, 10, 10, 255]);
        let mut b = a.clone();
        assert!(diff(&a, &b, 0).psnr.is_infinite());
        b.put(1, 1, [20, 10, 10, 255]);
        let d = diff(&a, &b, 2);
        assert_eq!(d.differing, 1);
        assert_eq!(d.max_abs, 10);
        assert!((d.mean_abs - 10.0 / 48.0).abs() < 1e-9);
        assert_eq!(diff(&a, &b, 10).differing, 0);
    }

    #[test]
    fn resize_area_halves() {
        let mut a = RgbaImage::new(2, 2);
        a.put(0, 0, [100, 0, 0, 255]);
        a.put(1, 0, [0, 100, 0, 255]);
        a.put(0, 1, [0, 0, 100, 255]);
        a.put(1, 1, [100, 100, 100, 255]);
        let r = a.resize_area(1, 1);
        assert_eq!(r.get(0, 0), [50, 50, 50, 255]);
        assert_eq!(a.scale_nearest(2).resize_area(2, 2), a);
    }
}
