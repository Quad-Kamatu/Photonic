//! `RasterImage` — the core 8-bit RGBA pixel buffer.
//!
//! Storage is straight (non-premultiplied) alpha, row-major, sRGB — matching
//! Photoshop's default 8-bit mode and the rest of Photonic's color model.
//! Adjustments and filters convert to `f32` internally per-op, so the public
//! representation stays compact while math stays precise.

use base64::Engine;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An 8-bit RGBA raster image with straight alpha.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasterImage {
    pub width: u32,
    pub height: u32,
    /// RGBA8 pixels, row-major. `len == width * height * 4`.
    pub pixels: Vec<u8>,
}

impl RasterImage {
    /// A fully transparent image of the given size.
    pub fn new(width: u32, height: u32) -> Self {
        let w = width.max(1);
        let h = height.max(1);
        Self {
            width: w,
            height: h,
            pixels: vec![0u8; (w as usize) * (h as usize) * 4],
        }
    }

    /// An image filled with a single RGBA color.
    pub fn filled(width: u32, height: u32, rgba: [u8; 4]) -> Self {
        let mut img = Self::new(width, height);
        for px in img.pixels.chunks_exact_mut(4) {
            px.copy_from_slice(&rgba);
        }
        img
    }

    /// Build from raw RGBA bytes. Errors if the length doesn't match `w*h*4`.
    pub fn from_rgba(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, String> {
        // A RasterImage is always at least 1×1: reject zero dimensions rather
        // than build a degenerate buffer whose stored size disagrees with its
        // length (which would panic on the next pixel access).
        if width == 0 || height == 0 {
            return Err("image dimensions must be non-zero".to_string());
        }
        let expected = (width as usize) * (height as usize) * 4;
        if pixels.len() != expected {
            return Err(format!(
                "pixel buffer length {} does not match {}x{}x4 = {}",
                pixels.len(),
                width,
                height,
                expected
            ));
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// Decode a PNG/JPEG/WebP/etc. encoded image into an RGBA8 buffer.
    pub fn from_encoded(bytes: &[u8]) -> Result<Self, String> {
        let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
        let rgba = img.to_rgba8();
        let (width, height) = rgba.dimensions();
        Ok(Self {
            width: width.max(1),
            height: height.max(1),
            pixels: rgba.into_raw(),
        })
    }

    /// Encode the image as PNG bytes.
    pub fn to_png(&self) -> Vec<u8> {
        let buf: image::RgbaImage =
            image::ImageBuffer::from_raw(self.width, self.height, self.pixels.clone())
                .unwrap_or_else(|| image::ImageBuffer::new(self.width, self.height));
        let mut out = Vec::new();
        let _ = image::DynamicImage::ImageRgba8(buf)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png);
        out
    }

    #[inline]
    pub fn in_bounds(&self, x: i64, y: i64) -> bool {
        x >= 0 && y >= 0 && (x as u32) < self.width && (y as u32) < self.height
    }

    #[inline]
    pub fn index(&self, x: u32, y: u32) -> usize {
        ((y as usize) * (self.width as usize) + (x as usize)) * 4
    }

    /// Read a pixel. Returns transparent black for out-of-bounds coordinates.
    #[inline]
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= self.width || y >= self.height {
            return [0, 0, 0, 0];
        }
        let i = self.index(x, y);
        [
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        ]
    }

    /// Sample a pixel with clamp-to-edge addressing (never out of bounds).
    #[inline]
    pub fn sample_clamped(&self, x: i64, y: i64) -> [u8; 4] {
        let cx = x.clamp(0, self.width as i64 - 1) as u32;
        let cy = y.clamp(0, self.height as i64 - 1) as u32;
        self.pixel(cx, cy)
    }

    /// Bilinear sample at a floating-point coordinate (clamp-to-edge).
    pub fn sample_bilinear(&self, fx: f32, fy: f32) -> [u8; 4] {
        let x0 = fx.floor();
        let y0 = fy.floor();
        let tx = fx - x0;
        let ty = fy - y0;
        let x0 = x0 as i64;
        let y0 = y0 as i64;
        let c00 = self.sample_clamped(x0, y0);
        let c10 = self.sample_clamped(x0 + 1, y0);
        let c01 = self.sample_clamped(x0, y0 + 1);
        let c11 = self.sample_clamped(x0 + 1, y0 + 1);
        let mut out = [0u8; 4];
        for c in 0..4 {
            let top = c00[c] as f32 * (1.0 - tx) + c10[c] as f32 * tx;
            let bot = c01[c] as f32 * (1.0 - tx) + c11[c] as f32 * tx;
            out[c] = (top * (1.0 - ty) + bot * ty).round().clamp(0.0, 255.0) as u8;
        }
        out
    }

    #[inline]
    pub fn set_pixel(&mut self, x: u32, y: u32, rgba: [u8; 4]) {
        if x >= self.width || y >= self.height {
            return;
        }
        let i = self.index(x, y);
        self.pixels[i..i + 4].copy_from_slice(&rgba);
    }

    /// Apply a per-pixel transform across the whole image.
    pub fn map_pixels(&mut self, mut f: impl FnMut([u8; 4]) -> [u8; 4]) {
        for px in self.pixels.chunks_exact_mut(4) {
            let out = f([px[0], px[1], px[2], px[3]]);
            px.copy_from_slice(&out);
        }
    }

    /// Apply a per-pixel RGB transform that leaves alpha untouched. `f` receives
    /// and returns linearless 0..1 RGB; alpha is preserved.
    pub fn map_rgb(&mut self, mut f: impl FnMut([f32; 3]) -> [f32; 3]) {
        for px in self.pixels.chunks_exact_mut(4) {
            let rgb = [
                px[0] as f32 / 255.0,
                px[1] as f32 / 255.0,
                px[2] as f32 / 255.0,
            ];
            let out = f(rgb);
            px[0] = (out[0] * 255.0).round().clamp(0.0, 255.0) as u8;
            px[1] = (out[1] * 255.0).round().clamp(0.0, 255.0) as u8;
            px[2] = (out[2] * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }

    /// Number of pixels (not bytes).
    pub fn len(&self) -> usize {
        (self.width as usize) * (self.height as usize)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ── sRGB luminance ─────────────────────────────────────────────────────────────

/// Perceptual luma (Rec. 601) of an sRGB pixel, 0..1.
#[inline]
pub fn luma(rgb: [f32; 3]) -> f32 {
    0.299 * rgb[0] + 0.587 * rgb[1] + 0.114 * rgb[2]
}

// ── Serialization (base64 PNG) ──────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct RasterImageRepr {
    width: u32,
    height: u32,
    /// base64-encoded PNG of the pixel data.
    png: String,
}

impl Serialize for RasterImage {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let png = self.to_png();
        let repr = RasterImageRepr {
            width: self.width,
            height: self.height,
            png: base64::engine::general_purpose::STANDARD.encode(png),
        };
        repr.serialize(s)
    }
}

impl<'de> Deserialize<'de> for RasterImage {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let repr = RasterImageRepr::deserialize(d)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(repr.png.as_bytes())
            .map_err(serde::de::Error::custom)?;
        let mut img = RasterImage::from_encoded(&bytes).map_err(serde::de::Error::custom)?;
        // Trust the stored dims if decode disagrees (defensive, should match).
        if img.width != repr.width || img.height != repr.height {
            img.width = repr.width.max(1);
            img.height = repr.height.max(1);
        }
        Ok(img)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_transparent() {
        let img = RasterImage::new(4, 3);
        assert_eq!(img.width, 4);
        assert_eq!(img.height, 3);
        assert_eq!(img.pixels.len(), 4 * 3 * 4);
        assert!(img.pixels.iter().all(|&b| b == 0));
    }

    #[test]
    fn filled_and_pixel() {
        let img = RasterImage::filled(2, 2, [10, 20, 30, 255]);
        assert_eq!(img.pixel(0, 0), [10, 20, 30, 255]);
        assert_eq!(img.pixel(1, 1), [10, 20, 30, 255]);
        assert_eq!(img.pixel(5, 5), [0, 0, 0, 0]); // oob
    }

    #[test]
    fn set_pixel_roundtrip() {
        let mut img = RasterImage::new(3, 3);
        img.set_pixel(1, 2, [1, 2, 3, 4]);
        assert_eq!(img.pixel(1, 2), [1, 2, 3, 4]);
    }

    #[test]
    fn from_rgba_validates_length() {
        assert!(RasterImage::from_rgba(2, 2, vec![0; 16]).is_ok());
        assert!(RasterImage::from_rgba(2, 2, vec![0; 15]).is_err());
    }

    #[test]
    fn png_roundtrip() {
        let mut img = RasterImage::filled(8, 6, [200, 100, 50, 255]);
        img.set_pixel(0, 0, [1, 2, 3, 255]);
        let png = img.to_png();
        let back = RasterImage::from_encoded(&png).unwrap();
        assert_eq!(back.width, 8);
        assert_eq!(back.height, 6);
        assert_eq!(back.pixel(0, 0), [1, 2, 3, 255]);
        assert_eq!(back.pixel(7, 5), [200, 100, 50, 255]);
    }

    #[test]
    fn serde_roundtrip_via_png() {
        let mut img = RasterImage::filled(5, 5, [0, 128, 255, 255]);
        img.set_pixel(2, 2, [255, 0, 0, 128]);
        let json = serde_json::to_string(&img).unwrap();
        assert!(json.contains("\"png\""));
        let back: RasterImage = serde_json::from_str(&json).unwrap();
        assert_eq!(img, back);
    }

    #[test]
    fn bilinear_midpoint_average() {
        let mut img = RasterImage::new(2, 1);
        img.set_pixel(0, 0, [0, 0, 0, 255]);
        img.set_pixel(1, 0, [100, 100, 100, 255]);
        let mid = img.sample_bilinear(0.5, 0.0);
        assert_eq!(mid[0], 50);
    }
}
