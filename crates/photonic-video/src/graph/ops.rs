//! CPU pixel kernels for the reference evaluator (`eval_cpu`), one per
//! `GraphOp` family (02 §1 module map's `graph/ops/`).
//!
//! Everything here operates on [`Image`] — an f32, premultiplied, linear-light
//! Rec.709 RGBA buffer, the CPU twin of the GPU `Rgba16Float` working texture
//! (D-09). Determinism is the whole point: same inputs ⇒ byte-identical output
//! (02 §2, 03 §4.4 rule 2 — the operation order matches the shader path). Blend
//! math reuses `photonic_core::raster::blend` so `Merge` agrees with the CPU
//! compositor where they overlap (03 §4.4 rule 4).

use glam::{Mat3, Vec2};
use photonic_core::layer::BlendMode;
use photonic_core::raster::blend::blend_rgb;

use crate::graph::ir::{FitMode, LinearColor, Sampling};

/// An f32 premultiplied linear-Rec.709 RGBA image — the CPU reference working
/// buffer. Row-major, `pixels.len() == width * height`.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// `[r, g, b, a]` premultiplied linear, row-major.
    pub pixels: Vec<[f32; 4]>,
}

impl Image {
    /// A transparent-black image (all zero, premultiplied).
    pub fn new(width: u32, height: u32) -> Self {
        let (width, height) = (width.max(1), height.max(1));
        Image {
            width,
            height,
            pixels: vec![[0.0; 4]; (width * height) as usize],
        }
    }

    /// A uniform image of `color` (already premultiplied linear).
    pub fn filled(width: u32, height: u32, color: LinearColor) -> Self {
        let (width, height) = (width.max(1), height.max(1));
        Image {
            width,
            height,
            pixels: vec![[color.r, color.g, color.b, color.a]; (width * height) as usize],
        }
    }

    #[inline]
    pub fn pixel(&self, x: u32, y: u32) -> [f32; 4] {
        let x = x.min(self.width - 1);
        let y = y.min(self.height - 1);
        self.pixels[(y * self.width + x) as usize]
    }

    #[inline]
    fn set(&mut self, x: u32, y: u32, v: [f32; 4]) {
        self.pixels[(y * self.width + x) as usize] = v;
    }

    /// Bilinear sample at pixel coordinates `(px, py)` (clamped at the edges).
    /// Operates directly on premultiplied values — correct because premultiplied
    /// linear is closed under linear interpolation (no fringing).
    pub fn sample_bilinear(&self, px: f32, py: f32) -> [f32; 4] {
        let w = self.width as i32;
        let h = self.height as i32;
        let x = px - 0.5;
        let y = py - 0.5;
        let x0 = x.floor() as i32;
        let y0 = y.floor() as i32;
        let fx = x - x0 as f32;
        let fy = y - y0 as f32;
        let at = |ix: i32, iy: i32| -> [f32; 4] {
            let cx = ix.clamp(0, w - 1) as u32;
            let cy = iy.clamp(0, h - 1) as u32;
            self.pixel(cx, cy)
        };
        let p00 = at(x0, y0);
        let p10 = at(x0 + 1, y0);
        let p01 = at(x0, y0 + 1);
        let p11 = at(x0 + 1, y0 + 1);
        let mut out = [0.0f32; 4];
        for c in 0..4 {
            let top = p00[c] * (1.0 - fx) + p10[c] * fx;
            let bot = p01[c] * (1.0 - fx) + p11[c] * fx;
            out[c] = top * (1.0 - fy) + bot * fy;
        }
        out
    }
}

/// `SolidColor`: a full-frame constant (premultiplied linear).
pub fn solid(width: u32, height: u32, color: LinearColor) -> Image {
    Image::filled(width, height, color)
}

/// `Transform2D`: resample `input` under the affine `mat` (dest ← src via the
/// inverse), producing a same-sized image. Identity is an exact passthrough.
pub fn transform2d(input: &Image, mat: Mat3, sampling: Sampling) -> Image {
    if !transform_matrix_is_valid(mat) {
        return Image::new(input.width, input.height);
    }
    if mat == Mat3::IDENTITY {
        return input.clone();
    }
    let inv = mat.inverse();
    let mut out = Image::new(input.width, input.height);
    for y in 0..out.height {
        for x in 0..out.width {
            let dst = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let src = inv.transform_point2(dst);
            let v = match sampling {
                Sampling::Bilinear => input.sample_bilinear(src.x, src.y),
                Sampling::Nearest => {
                    let sx = (src.x.floor() as i32).clamp(0, input.width as i32 - 1) as u32;
                    let sy = (src.y.floor() as i32).clamp(0, input.height as i32 - 1) as u32;
                    input.pixel(sx, sy)
                }
            };
            out.set(x, y, v);
        }
    }
    out
}

/// Inversion policy shared by the CPU and GPU transform paths. Near-singular
/// or non-finite matrices render transparent rather than sampling undefined
/// coordinates.
pub(crate) fn transform_matrix_is_valid(mat: Mat3) -> bool {
    let determinant = mat.determinant();
    mat.to_cols_array().into_iter().all(f32::is_finite)
        && determinant.is_finite()
        && determinant.abs() > 1e-8
}

/// `Resize`: scale `input` to `(w, h)` under `fit` (bilinear). `Stretch` maps the
/// whole frame; `Fit`/`Fill` apply a uniform scale (letterbox / crop).
pub fn resize(input: &Image, w: u32, h: u32, fit: FitMode) -> Image {
    let (w, h) = (w.max(1), h.max(1));
    let mut out = Image::new(w, h);
    let (iw, ih) = (input.width as f32, input.height as f32);
    let (ow, oh) = (w as f32, h as f32);
    let (sx, sy, ox, oy) = match fit {
        FitMode::Stretch => (iw / ow, ih / oh, 0.0, 0.0),
        FitMode::Fit => {
            let s = (iw / ow).max(ih / oh);
            (s, s, (iw - ow * s) * 0.5, (ih - oh * s) * 0.5)
        }
        FitMode::Fill => {
            let s = (iw / ow).min(ih / oh);
            (s, s, (iw - ow * s) * 0.5, (ih - oh * s) * 0.5)
        }
    };
    for y in 0..h {
        for x in 0..w {
            let sxp = (x as f32 + 0.5) * sx + ox;
            let syp = (y as f32 + 0.5) * sy + oy;
            out.set(x, y, input.sample_bilinear(sxp, syp));
        }
    }
    out
}

/// `Crop`: P3 passthrough (the region model finalizes with the node inspector /
/// timeline crop UI); kept as a named op so the chain shape is right now.
pub fn crop(input: &Image) -> Image {
    input.clone()
}

/// `Effect{Invert}` (08 §3 / §2 `Invert` row): invert the straight (unpremult)
/// linear color, preserving alpha, then re-premultiply — the CPU twin of the GPU
/// invert pass (`eval::Passes::invert`). Straight color is clamped to `[0,1]`
/// before inversion so the op is well-defined for out-of-range working values,
/// matching the shader's `1 - clamp(c, 0, 1)`. For an opaque pixel this reduces
/// to `1 - rgb`.
pub fn invert(input: &Image) -> Image {
    let mut out = Image::new(input.width, input.height);
    for (o, p) in out.pixels.iter_mut().zip(input.pixels.iter()) {
        let a = p[3];
        let straight = if a > 1e-6 {
            [
                (p[0] / a).clamp(0.0, 1.0),
                (p[1] / a).clamp(0.0, 1.0),
                (p[2] / a).clamp(0.0, 1.0),
            ]
        } else {
            [0.0, 0.0, 0.0]
        };
        *o = [
            (1.0 - straight[0]) * a,
            (1.0 - straight[1]) * a,
            (1.0 - straight[2]) * a,
            a,
        ];
    }
    out
}

/// `Merge`: composite `top` over `bottom` with global `opacity`, under `mode`
/// (02 §2 `Merge`). Premultiplied linear source-over with a blend function
/// (W3C compositing model); `Normal` reduces to plain `over`. Blend math reuses
/// [`blend_rgb`] (03 §4.4 rule 4). Output is the size of `bottom` (or `top` when
/// bottom is smaller); mismatched sizes sample `top` at matching pixels.
pub fn merge(top: &Image, bottom: &Image, mode: BlendMode, opacity: f32) -> Image {
    let opacity = opacity.clamp(0.0, 1.0);
    let w = top.width.max(bottom.width);
    let h = top.height.max(bottom.height);
    let mut out = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let tp = sample_clamped(top, x, y);
            let bp = sample_clamped(bottom, x, y);
            out.set(x, y, merge_pixel(tp, bp, mode, opacity));
        }
    }
    out
}

#[inline]
fn sample_clamped(img: &Image, x: u32, y: u32) -> [f32; 4] {
    let cx = x.min(img.width - 1);
    let cy = y.min(img.height - 1);
    img.pixel(cx, cy)
}

/// One premultiplied `over`-with-blend pixel (W3C compositing). `tp`/`bp` are
/// premultiplied linear; the result is premultiplied linear.
#[inline]
fn merge_pixel(tp: [f32; 4], bp: [f32; 4], mode: BlendMode, opacity: f32) -> [f32; 4] {
    let a_s = tp[3] * opacity; // effective source alpha
    let a_b = bp[3];
    let cs = unpremultiply(tp);
    let cb = unpremultiply(bp);
    // Backdrop-blended source color: Cs' = (1-αb)·Cs + αb·B(Cb, Cs).
    let blended = if mode == BlendMode::Normal {
        cs
    } else {
        blend_rgb(mode, cb, cs)
    };
    let mut out = [0.0f32; 4];
    for c in 0..3 {
        // Backdrop-blended straight source color, then premultiplied source-over:
        // Cs' = (1-αb)·Cs + αb·B(Cb, Cs);  co = αs·Cs' + (1-αs)·(backdrop premult).
        let cs_prime = (1.0 - a_b) * cs[c] + a_b * blended[c];
        out[c] = a_s * cs_prime + (1.0 - a_s) * bp[c];
    }
    out[3] = a_s + a_b * (1.0 - a_s);
    out
}

/// Straight (unpremultiplied) linear RGB from a premultiplied pixel.
#[inline]
fn unpremultiply(p: [f32; 4]) -> [f32; 3] {
    let a = p[3];
    if a > 1e-6 {
        [p[0] / a, p[1] / a, p[2] / a]
    } else {
        [0.0, 0.0, 0.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn premult(r: f32, g: f32, b: f32, a: f32) -> [f32; 4] {
        [r * a, g * a, b * a, a]
    }

    #[test]
    fn solid_is_uniform() {
        let img = solid(
            4,
            4,
            LinearColor {
                r: 0.25,
                g: 0.5,
                b: 0.75,
                a: 1.0,
            },
        );
        assert_eq!(img.pixels.len(), 16);
        assert!(img.pixels.iter().all(|p| *p == [0.25, 0.5, 0.75, 1.0]));
    }

    #[test]
    fn identity_transform_is_passthrough() {
        let img = solid(3, 3, LinearColor { r: 0.1, g: 0.2, b: 0.3, a: 1.0 });
        let out = transform2d(&img, Mat3::IDENTITY, Sampling::Bilinear);
        assert_eq!(out, img);
    }

    #[test]
    fn singular_transform_is_transparent() {
        let img = Image {
            width: 2,
            height: 2,
            pixels: vec![premult(1.0, 0.0, 0.0, 1.0); 4],
        };
        let out = transform2d(
            &img,
            Mat3::from_scale(Vec2::new(0.0, 1.0)),
            Sampling::Nearest,
        );
        assert_eq!(out, Image::new(2, 2));
    }

    #[test]
    fn merge_opaque_over_replaces_backdrop() {
        // Opaque red over opaque blue with Normal, opacity 1 → red.
        let top = solid(2, 2, LinearColor { r: 1.0, g: 0.0, b: 0.0, a: 1.0 });
        let bottom = solid(2, 2, LinearColor { r: 0.0, g: 0.0, b: 1.0, a: 1.0 });
        let out = merge(&top, &bottom, BlendMode::Normal, 1.0);
        for p in &out.pixels {
            assert!((p[0] - 1.0).abs() < 1e-6);
            assert!(p[1].abs() < 1e-6);
            assert!(p[2].abs() < 1e-6);
            assert!((p[3] - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn merge_half_opacity_is_linear_blend() {
        // Opaque white over opaque black at opacity 0.5 → premultiplied 0.5 grey.
        let top = solid(1, 1, LinearColor { r: 1.0, g: 1.0, b: 1.0, a: 1.0 });
        let bottom = solid(1, 1, LinearColor { r: 0.0, g: 0.0, b: 0.0, a: 1.0 });
        let out = merge(&top, &bottom, BlendMode::Normal, 0.5);
        let p = out.pixels[0];
        #[allow(clippy::needless_range_loop)]
        for c in 0..3 {
            assert!((p[c] - 0.5).abs() < 1e-6, "channel {c} = {}", p[c]);
        }
        assert!((p[3] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn merge_transparent_top_keeps_backdrop() {
        let top = solid(1, 1, LinearColor { r: 0.0, g: 0.0, b: 0.0, a: 0.0 });
        let bottom = solid(1, 1, LinearColor { r: 0.3, g: 0.4, b: 0.5, a: 1.0 });
        let out = merge(&top, &bottom, BlendMode::Normal, 1.0);
        let p = out.pixels[0];
        assert!((p[0] - 0.3).abs() < 1e-6);
        assert!((p[1] - 0.4).abs() < 1e-6);
        assert!((p[2] - 0.5).abs() < 1e-6);
        assert!((p[3] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn merge_over_transparent_backdrop_premultiplies_source() {
        // Opaque red over transparent, opacity 1 → premultiplied red, alpha 1.
        let top = solid(1, 1, LinearColor { r: 1.0, g: 0.0, b: 0.0, a: 1.0 });
        let bottom = Image::new(1, 1);
        let out = merge(&top, &bottom, BlendMode::Normal, 1.0);
        assert_eq!(out.pixels[0], premult(1.0, 0.0, 0.0, 1.0));
    }

    #[test]
    fn invert_opaque_is_one_minus_rgb() {
        let img = solid(2, 2, LinearColor { r: 0.2, g: 0.6, b: 0.9, a: 1.0 });
        let out = invert(&img);
        for p in &out.pixels {
            assert!((p[0] - 0.8).abs() < 1e-6, "r={}", p[0]);
            assert!((p[1] - 0.4).abs() < 1e-6, "g={}", p[1]);
            assert!((p[2] - 0.1).abs() < 1e-6, "b={}", p[2]);
            assert!((p[3] - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn invert_preserves_premultiplied_alpha() {
        // A 50%-transparent white premult pixel [0.5,0.5,0.5,0.5] → straight white
        // → inverted straight black → premult [0,0,0,0.5], alpha unchanged.
        let mut img = Image::new(1, 1);
        img.pixels[0] = [0.5, 0.5, 0.5, 0.5];
        let out = invert(&img);
        let p = out.pixels[0];
        for (c, &v) in p[..3].iter().enumerate() {
            assert!(v.abs() < 1e-6, "channel {c} = {v}");
        }
        assert!((p[3] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn bilinear_sample_midpoint_averages() {
        // A 2×1 image: black left, white right. Sampling the seam averages.
        let mut img = Image::new(2, 1);
        img.pixels[0] = [0.0, 0.0, 0.0, 1.0];
        img.pixels[1] = [1.0, 1.0, 1.0, 1.0];
        let mid = img.sample_bilinear(1.0, 0.5); // pixel boundary between the two
        #[allow(clippy::needless_range_loop)]
        for c in 0..3 {
            assert!((mid[c] - 0.5).abs() < 1e-6, "channel {c} = {}", mid[c]);
        }
    }
}
