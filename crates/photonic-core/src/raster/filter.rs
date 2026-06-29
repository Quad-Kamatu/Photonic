//! Filter menu — Photoshop-style neighborhood and noise filters as pure CPU ops.
//!
//! Every operation here is deterministic and unit-tested, with no GPU or
//! windowing dependency. Each *neighborhood* filter reads from an immutable
//! snapshot of the source pixels, builds a fully-computed result
//! [`RasterImage`], and then hands it to [`blend_result`] so that an optional
//! selection [`Mask`] is honored uniformly across every filter.
//!
//! Implemented:
//! - Blur: [`gaussian_blur`], [`box_blur`], [`motion_blur`]
//! - Sharpen: [`sharpen`], [`unsharp_mask`]
//! - Noise / cleanup: [`add_noise`], [`median`]
//! - Stylize: [`emboss`], [`find_edges`], [`mosaic`], [`high_pass`]
//!
//! [`gaussian_blur_gray`] is a standalone separable Gaussian over a
//! single-channel 8-bit buffer; it backs the selection-feather code in
//! `mask.rs`, so its signature and behavior are load-bearing.

use crate::raster::{
    blend_result,
    image::{luma, RasterImage},
    mask::Mask,
};

// ── Input sanitizers (BUG 1: never panic / overflow on hostile inputs) ──────────

/// The largest radius/distance/window we will ever honor. Bounds every kernel
/// size so the `as usize` / `as i64` computations derived from it can never
/// overflow, regardless of the f32/u32 the caller passes in.
pub(crate) const MAX_RADIUS: u32 = 1024;

/// Sanitize a floating radius/sigma: non-finite (NaN/±inf) → `0.0` (no-op),
/// otherwise clamp into `0..=MAX_RADIUS` so derived kernel sizes stay bounded.
pub(crate) fn san_radius(r: f32) -> f32 {
    if r.is_finite() {
        r.clamp(0.0, MAX_RADIUS as f32)
    } else {
        0.0
    }
}

/// Sanitize a general scalar (amount/threshold/strength/angle): non-finite → `0.0`.
/// Callers apply their own range clamps afterwards.
pub(crate) fn san_amount(a: f32) -> f32 {
    if a.is_finite() {
        a
    } else {
        0.0
    }
}

// ── Premultiplied-alpha helpers (BUG 2: no dark fringing from straight blur) ─────

/// Premultiply an RGBA image into four `f32` planes: `(R·a, G·a, B·a, A)` with
/// `a = A/255`. Working in premultiplied space means transparent pixels (whose
/// stored RGB is usually black) contribute *nothing* to a neighborhood average,
/// so they can no longer darken opaque neighbors.
fn premultiply_planes(img: &RasterImage) -> [Vec<f32>; 4] {
    let n = img.len();
    let mut p = [vec![0f32; n], vec![0f32; n], vec![0f32; n], vec![0f32; n]];
    for i in 0..n {
        let base = i * 4;
        let a = img.pixels[base + 3] as f32;
        let af = a / 255.0;
        p[0][i] = img.pixels[base] as f32 * af;
        p[1][i] = img.pixels[base + 1] as f32 * af;
        p[2][i] = img.pixels[base + 2] as f32 * af;
        p[3][i] = a;
    }
    p
}

/// Un-premultiply four blurred `f32` planes back into an RGBA image. Pixels with
/// `A == 0` collapse to `[0, 0, 0, 0]`; otherwise `R = Rp · 255 / A` (clamped).
fn unpremultiply_planes(width: u32, height: u32, p: &[Vec<f32>; 4]) -> RasterImage {
    let mut out = RasterImage::new(width, height);
    let n = out.len();
    for i in 0..n {
        let base = i * 4;
        let a = p[3][i];
        if a <= 0.0 {
            out.pixels[base] = 0;
            out.pixels[base + 1] = 0;
            out.pixels[base + 2] = 0;
            out.pixels[base + 3] = 0;
        } else {
            let af = a / 255.0;
            out.pixels[base] = (p[0][i] / af).round().clamp(0.0, 255.0) as u8;
            out.pixels[base + 1] = (p[1][i] / af).round().clamp(0.0, 255.0) as u8;
            out.pixels[base + 2] = (p[2][i] / af).round().clamp(0.0, 255.0) as u8;
            out.pixels[base + 3] = a.round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// Separable convolution of a single `f32` plane with a symmetric 1-D `kernel`
/// (horizontal pass then vertical pass, clamp-to-edge at the borders). Shared by
/// the premultiplied Gaussian and box blurs.
fn separable_blur_f32(data: &[f32], w: usize, h: usize, kernel: &[f32]) -> Vec<f32> {
    if kernel.is_empty() || w == 0 || h == 0 || data.len() != w * h {
        return data.to_vec();
    }
    let r = (kernel.len() / 2) as i64;

    let mut tmp = vec![0f32; w * h];
    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            let mut acc = 0.0f32;
            for (ki, &kv) in kernel.iter().enumerate() {
                let sx = (x as i64 + ki as i64 - r).clamp(0, w as i64 - 1) as usize;
                acc += data[row + sx] * kv;
            }
            tmp[row + x] = acc;
        }
    }

    let mut out = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0f32;
            for (ki, &kv) in kernel.iter().enumerate() {
                let sy = (y as i64 + ki as i64 - r).clamp(0, h as i64 - 1) as usize;
                acc += tmp[sy * w + x] * kv;
            }
            out[y * w + x] = acc;
        }
    }
    out
}

// ── Gaussian ─────────────────────────────────────────────────────────────────

/// Build a normalized 1-D Gaussian kernel for the given sigma. Radius ≈ 3·sigma.
fn gaussian_kernel(sigma: f32) -> Vec<f32> {
    // Defensive: callers already sanitize, but bound here too so a kernel size
    // derived from sigma can never overflow or balloon out of memory.
    let sigma = if sigma.is_finite() {
        sigma.clamp(0.0, MAX_RADIUS as f32)
    } else {
        0.0
    };
    let radius = ((sigma * 3.0).ceil() as i64)
        .max(1)
        .min(4 * MAX_RADIUS as i64);
    let two_sigma2 = 2.0 * sigma * sigma;
    // Degenerate sigma (≈0) would divide by zero → NaN; fall back to a 1-tap
    // identity kernel so we never emit NaN weights.
    if two_sigma2 <= 0.0 {
        return vec![1.0];
    }
    let mut kernel = Vec::with_capacity((2 * radius + 1) as usize);
    let mut sum = 0.0f32;
    for i in -radius..=radius {
        let v = (-((i * i) as f32) / two_sigma2).exp();
        kernel.push(v);
        sum += v;
    }
    if sum > 0.0 {
        for v in kernel.iter_mut() {
            *v /= sum;
        }
    }
    kernel
}

/// Separable Gaussian blur of a single-channel 8-bit buffer. Returns a new
/// buffer. Horizontal pass then vertical pass, clamp-to-edge at the borders.
///
/// Used by the selection-feather code in `mask.rs`; keep this correct + public.
pub fn gaussian_blur_gray(data: &[u8], width: u32, height: u32, radius: f32) -> Vec<u8> {
    let radius = san_radius(radius);
    let w = width as usize;
    let h = height as usize;
    if radius <= 0.0 || w == 0 || h == 0 || data.len() != w * h {
        return data.to_vec();
    }
    let kernel = gaussian_kernel(radius);
    let r = (kernel.len() / 2) as i64;

    // Horizontal pass → f32 intermediate.
    let mut tmp = vec![0f32; w * h];
    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            let mut acc = 0.0f32;
            for (ki, &kv) in kernel.iter().enumerate() {
                let sx = (x as i64 + ki as i64 - r).clamp(0, w as i64 - 1) as usize;
                acc += data[row + sx] as f32 * kv;
            }
            tmp[row + x] = acc;
        }
    }

    // Vertical pass → u8 output.
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0f32;
            for (ki, &kv) in kernel.iter().enumerate() {
                let sy = (y as i64 + ki as i64 - r).clamp(0, h as i64 - 1) as usize;
                acc += tmp[sy * w + x] * kv;
            }
            out[y * w + x] = acc.round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// Blur an RGBA image with a separable Gaussian, operating in **premultiplied
/// alpha** so transparent (usually black) pixels do not darken opaque neighbors.
fn gaussian_blur_image(img: &RasterImage, radius: f32) -> RasterImage {
    let radius = san_radius(radius);
    if radius <= 0.0 {
        return img.clone();
    }
    let kernel = gaussian_kernel(radius);
    let (w, h) = (img.width as usize, img.height as usize);
    let p = premultiply_planes(img);
    let out = [
        separable_blur_f32(&p[0], w, h, &kernel),
        separable_blur_f32(&p[1], w, h, &kernel),
        separable_blur_f32(&p[2], w, h, &kernel),
        separable_blur_f32(&p[3], w, h, &kernel),
    ];
    unpremultiply_planes(img.width, img.height, &out)
}

/// Gaussian Blur — separable, premultiplied-alpha, applied to every RGBA channel.
pub fn gaussian_blur(img: &mut RasterImage, radius: f32, sel: Option<&Mask>) {
    let radius = san_radius(radius);
    if radius <= 0.0 {
        return;
    }
    let result = gaussian_blur_image(img, radius);
    blend_result(img, &result, sel);
}

// ── Box blur ─────────────────────────────────────────────────────────────────

/// Box (averaged) blur over a `(2·radius+1)²` window, in **premultiplied alpha**
/// (a uniform separable kernel). `radius` is clamped to `MAX_RADIUS`.
pub fn box_blur(img: &mut RasterImage, radius: u32, sel: Option<&Mask>) {
    let radius = radius.min(MAX_RADIUS);
    if radius == 0 {
        return;
    }
    let (w, h) = (img.width as usize, img.height as usize);
    let size = 2 * radius as usize + 1;
    let kernel = vec![1.0f32 / size as f32; size];
    let p = premultiply_planes(img);
    let out = [
        separable_blur_f32(&p[0], w, h, &kernel),
        separable_blur_f32(&p[1], w, h, &kernel),
        separable_blur_f32(&p[2], w, h, &kernel),
        separable_blur_f32(&p[3], w, h, &kernel),
    ];
    let result = unpremultiply_planes(img.width, img.height, &out);
    blend_result(img, &result, sel);
}

// ── Motion blur ───────────────────────────────────────────────────────────────

/// Motion Blur — average `distance` samples along a line at `angle_deg`, in
/// **premultiplied alpha**. `angle_deg` is sanitized (non-finite → 0) and
/// `distance` is clamped to `MAX_RADIUS`.
pub fn motion_blur(img: &mut RasterImage, angle_deg: f32, distance: u32, sel: Option<&Mask>) {
    let angle_deg = san_amount(angle_deg);
    let distance = distance.min(MAX_RADIUS);
    if distance == 0 {
        return;
    }
    let n = distance as i64; // guaranteed >= 1 here
    let theta = angle_deg.to_radians();
    let dx = theta.cos();
    let dy = theta.sin();
    let half = (n - 1) as f32 / 2.0;

    let mut result = RasterImage::new(img.width, img.height);
    for y in 0..img.height as i64 {
        for x in 0..img.width as i64 {
            let mut acc = [0f32; 4]; // premultiplied R,G,B + accumulated alpha
            for i in 0..n {
                let t = i as f32 - half;
                let sx = (x as f32 + dx * t).round() as i64;
                let sy = (y as f32 + dy * t).round() as i64;
                let p = img.sample_clamped(sx, sy);
                let a = p[3] as f32;
                let af = a / 255.0;
                acc[0] += p[0] as f32 * af;
                acc[1] += p[1] as f32 * af;
                acc[2] += p[2] as f32 * af;
                acc[3] += a;
            }
            let inv = 1.0 / n as f32;
            let ap = acc[3] * inv;
            let out = if ap <= 0.0 {
                [0, 0, 0, 0]
            } else {
                let afp = ap / 255.0;
                [
                    ((acc[0] * inv) / afp).round().clamp(0.0, 255.0) as u8,
                    ((acc[1] * inv) / afp).round().clamp(0.0, 255.0) as u8,
                    ((acc[2] * inv) / afp).round().clamp(0.0, 255.0) as u8,
                    ap.round().clamp(0.0, 255.0) as u8,
                ]
            };
            result.set_pixel(x as u32, y as u32, out);
        }
    }
    blend_result(img, &result, sel);
}

// ── Sharpen ─────────────────────────────────────────────────────────────────

/// Apply a 3×3 kernel to the RGB channels (alpha preserved from the source).
fn conv3x3_rgb(img: &RasterImage, kernel: [[f32; 3]; 3]) -> RasterImage {
    let mut out = img.clone();
    for y in 0..img.height as i64 {
        for x in 0..img.width as i64 {
            let mut acc = [0f32; 3];
            for ky in 0..3i64 {
                for kx in 0..3i64 {
                    let p = img.sample_clamped(x + kx - 1, y + ky - 1);
                    let kv = kernel[ky as usize][kx as usize];
                    for c in 0..3 {
                        acc[c] += p[c] as f32 * kv;
                    }
                }
            }
            let i = img.index(x as u32, y as u32);
            for c in 0..3 {
                out.pixels[i + c] = acc[c].round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// Sharpen — a 3×3 high-boost kernel whose strength scales with `amount`.
/// `amount == 0` is identity. Alpha is preserved.
pub fn sharpen(img: &mut RasterImage, amount: f32, sel: Option<&Mask>) {
    let amount = san_amount(amount);
    if amount <= 0.0 {
        return;
    }
    let a = amount;
    let kernel = [[0.0, -a, 0.0], [-a, 1.0 + 4.0 * a, -a], [0.0, -a, 0.0]];
    let result = conv3x3_rgb(img, kernel);
    blend_result(img, &result, sel);
}

// ── Unsharp mask ───────────────────────────────────────────────────────────────

/// Unsharp Mask — `img + amount·(img − gaussian(img))` where the per-channel
/// absolute difference exceeds `threshold`. Alpha is preserved.
pub fn unsharp_mask(
    img: &mut RasterImage,
    radius: f32,
    amount: f32,
    threshold: u8,
    sel: Option<&Mask>,
) {
    let radius = san_radius(radius);
    let amount = san_amount(amount);
    if amount == 0.0 || radius <= 0.0 {
        return;
    }
    let blurred = gaussian_blur_image(img, radius);
    let thr = threshold as f32;
    let mut result = img.clone();
    let n = img.len();
    for i in 0..n {
        let base = i * 4;
        for c in 0..3 {
            let orig = img.pixels[base + c] as f32;
            let diff = orig - blurred.pixels[base + c] as f32;
            if diff.abs() > thr {
                result.pixels[base + c] = (orig + amount * diff).round().clamp(0.0, 255.0) as u8;
            }
        }
        // alpha (base + 3) preserved by the clone
    }
    blend_result(img, &result, sel);
}

// ── Median ─────────────────────────────────────────────────────────────────

/// Median filter — per-channel median over a `(2·radius+1)²` window. Excellent
/// for removing salt-and-pepper noise / single-pixel outliers.
pub fn median(img: &mut RasterImage, radius: u32, sel: Option<&Mask>) {
    if radius == 0 {
        return;
    }
    // Clamp to MAX_RADIUS so the window-capacity `(2r+1)²` can never overflow,
    // and to the image extent (clamp-to-edge makes anything larger redundant)
    // so a huge radius on a tiny image stays cheap.
    let max_extent = (img.width.max(img.height)).max(1);
    let r = radius.min(MAX_RADIUS).min(max_extent) as i64;
    let mut result = RasterImage::new(img.width, img.height);
    let mut window: Vec<[u8; 4]> = Vec::with_capacity(((2 * r + 1) * (2 * r + 1)) as usize);
    for y in 0..img.height as i64 {
        for x in 0..img.width as i64 {
            window.clear();
            for wy in -r..=r {
                for wx in -r..=r {
                    window.push(img.sample_clamped(x + wx, y + wy));
                }
            }
            let mid = window.len() / 2;
            let mut out = [0u8; 4];
            for c in 0..4 {
                let mut vals: Vec<u8> = window.iter().map(|p| p[c]).collect();
                vals.sort_unstable();
                out[c] = vals[mid];
            }
            result.set_pixel(x as u32, y as u32, out);
        }
    }
    blend_result(img, &result, sel);
}

// ── Add noise ─────────────────────────────────────────────────────────────────

/// A tiny deterministic splitmix64 PRNG (no external crate).
struct SplitMix64(u64);

impl SplitMix64 {
    #[inline]
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform f32 in [0, 1).
    #[inline]
    fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Signed noise in [-1, 1).
    #[inline]
    fn next_signed(&mut self) -> f32 {
        self.next_f32() * 2.0 - 1.0
    }
}

/// Add Noise — uniform noise scaled by `amount` (0..1). When `monochrome`, the
/// same offset is applied to R, G, B; otherwise each channel is independent.
/// Deterministic for a fixed `seed` (same input + seed ⇒ same output). Alpha is
/// preserved.
pub fn add_noise(
    img: &mut RasterImage,
    amount: f32,
    monochrome: bool,
    seed: u64,
    sel: Option<&Mask>,
) {
    let amount = san_amount(amount).clamp(0.0, 1.0);
    if amount == 0.0 {
        return;
    }
    let scale = amount * 255.0;
    let mut rng = SplitMix64(seed);
    let mut result = img.clone();
    let n = img.len();
    for i in 0..n {
        let base = i * 4;
        if monochrome {
            let delta = rng.next_signed() * scale;
            for c in 0..3 {
                let v = img.pixels[base + c] as f32 + delta;
                result.pixels[base + c] = v.round().clamp(0.0, 255.0) as u8;
            }
        } else {
            for c in 0..3 {
                let delta = rng.next_signed() * scale;
                let v = img.pixels[base + c] as f32 + delta;
                result.pixels[base + c] = v.round().clamp(0.0, 255.0) as u8;
            }
        }
        // alpha preserved
    }
    blend_result(img, &result, sel);
}

// ── Emboss ─────────────────────────────────────────────────────────────────

/// Emboss — a directional gradient over luma, biased by 128 to mid-gray. A flat
/// region becomes neutral gray; edges become light/dark relief. Output is gray
/// (R = G = B); alpha is preserved.
pub fn emboss(img: &mut RasterImage, sel: Option<&Mask>) {
    // Sum-zero emboss kernel so flat regions land exactly on the 128 bias.
    const K: [[f32; 3]; 3] = [[-1.0, -1.0, 0.0], [-1.0, 0.0, 1.0], [0.0, 1.0, 1.0]];
    let mut result = img.clone();
    for y in 0..img.height as i64 {
        for x in 0..img.width as i64 {
            let mut acc = 0.0f32;
            for ky in 0..3i64 {
                for kx in 0..3i64 {
                    let p = img.sample_clamped(x + kx - 1, y + ky - 1);
                    let l = luma([p[0] as f32, p[1] as f32, p[2] as f32]);
                    acc += l * K[ky as usize][kx as usize];
                }
            }
            let g = (acc + 128.0).round().clamp(0.0, 255.0) as u8;
            let i = img.index(x as u32, y as u32);
            result.pixels[i] = g;
            result.pixels[i + 1] = g;
            result.pixels[i + 2] = g;
        }
    }
    blend_result(img, &result, sel);
}

// ── Find edges ───────────────────────────────────────────────────────────────

/// Find Edges — Sobel gradient magnitude over luma. Flat regions go black,
/// edges go bright. Output is gray (R = G = B); alpha is preserved.
pub fn find_edges(img: &mut RasterImage, sel: Option<&Mask>) {
    const GX: [[f32; 3]; 3] = [[-1.0, 0.0, 1.0], [-2.0, 0.0, 2.0], [-1.0, 0.0, 1.0]];
    const GY: [[f32; 3]; 3] = [[-1.0, -2.0, -1.0], [0.0, 0.0, 0.0], [1.0, 2.0, 1.0]];
    let mut result = img.clone();
    for y in 0..img.height as i64 {
        for x in 0..img.width as i64 {
            let mut gx = 0.0f32;
            let mut gy = 0.0f32;
            for ky in 0..3i64 {
                for kx in 0..3i64 {
                    let p = img.sample_clamped(x + kx - 1, y + ky - 1);
                    let l = luma([p[0] as f32, p[1] as f32, p[2] as f32]);
                    gx += l * GX[ky as usize][kx as usize];
                    gy += l * GY[ky as usize][kx as usize];
                }
            }
            let mag = (gx * gx + gy * gy).sqrt().round().clamp(0.0, 255.0) as u8;
            let i = img.index(x as u32, y as u32);
            result.pixels[i] = mag;
            result.pixels[i + 1] = mag;
            result.pixels[i + 2] = mag;
        }
    }
    blend_result(img, &result, sel);
}

// ── Mosaic ─────────────────────────────────────────────────────────────────

/// Mosaic (pixelate) — average each `block × block` cell into a single color.
pub fn mosaic(img: &mut RasterImage, block: u32, sel: Option<&Mask>) {
    if block <= 1 {
        return;
    }
    let mut result = RasterImage::new(img.width, img.height);
    // Block is bounded by the image extent anyway; clamp defensively.
    let b = (block.min(img.width.max(img.height).max(1))) as i64;
    let w = img.width as i64;
    let h = img.height as i64;
    let mut by = 0i64;
    while by < h {
        let mut bx = 0i64;
        while bx < w {
            let x1 = (bx + b).min(w);
            let y1 = (by + b).min(h);
            let count = ((x1 - bx) * (y1 - by)) as f32;
            // Average in premultiplied alpha so transparent cells don't darken
            // the block's color.
            let mut acc = [0f32; 4]; // premultiplied R,G,B + accumulated alpha
            for yy in by..y1 {
                for xx in bx..x1 {
                    let p = img.pixel(xx as u32, yy as u32);
                    let a = p[3] as f32;
                    let af = a / 255.0;
                    acc[0] += p[0] as f32 * af;
                    acc[1] += p[1] as f32 * af;
                    acc[2] += p[2] as f32 * af;
                    acc[3] += a;
                }
            }
            let ap = acc[3] / count;
            let avg = if ap <= 0.0 {
                [0, 0, 0, 0]
            } else {
                let afp = ap / 255.0;
                [
                    ((acc[0] / count) / afp).round().clamp(0.0, 255.0) as u8,
                    ((acc[1] / count) / afp).round().clamp(0.0, 255.0) as u8,
                    ((acc[2] / count) / afp).round().clamp(0.0, 255.0) as u8,
                    ap.round().clamp(0.0, 255.0) as u8,
                ]
            };
            for yy in by..y1 {
                for xx in bx..x1 {
                    result.set_pixel(xx as u32, yy as u32, avg);
                }
            }
            bx += b;
        }
        by += b;
    }
    blend_result(img, &result, sel);
}

// ── High pass ─────────────────────────────────────────────────────────────────

/// High Pass — `128 + (img − gaussian(img))` per RGB channel. Keeps fine
/// detail around mid-gray; large smooth areas converge to gray. Alpha preserved.
pub fn high_pass(img: &mut RasterImage, radius: f32, sel: Option<&Mask>) {
    let radius = san_radius(radius);
    let blurred = gaussian_blur_image(img, radius);
    let mut result = img.clone();
    let n = img.len();
    for i in 0..n {
        let base = i * 4;
        for c in 0..3 {
            let diff = img.pixels[base + c] as f32 - blurred.pixels[base + c] as f32;
            result.pixels[base + c] = (128.0 + diff).round().clamp(0.0, 255.0) as u8;
        }
        // alpha preserved
    }
    blend_result(img, &result, sel);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: u32, h: u32) -> RasterImage {
        let mut img = RasterImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = ((x + y) * 7 % 256) as u8;
                img.set_pixel(x, y, [v, v.wrapping_add(40), v.wrapping_add(80), 255]);
            }
        }
        img
    }

    // ── gaussian_blur_gray ──────────────────────────────────────────────────────

    #[test]
    fn gaussian_gray_preserves_constant() {
        let data = vec![123u8; 8 * 8];
        let out = gaussian_blur_gray(&data, 8, 8, 2.0);
        assert!(out.iter().all(|&v| v == 123));
    }

    #[test]
    fn gaussian_gray_radius_zero_is_identity() {
        let data: Vec<u8> = (0..16).map(|i| (i * 13) as u8).collect();
        let out = gaussian_blur_gray(&data, 4, 4, 0.0);
        assert_eq!(out, data);
    }

    #[test]
    fn gaussian_gray_bad_length_is_passthrough() {
        let data = vec![1u8, 2, 3];
        assert_eq!(gaussian_blur_gray(&data, 4, 4, 1.5), data);
    }

    // ── gaussian_blur ─────────────────────────────────────────────────────────

    #[test]
    fn gaussian_flat_unchanged() {
        let mut img = RasterImage::filled(10, 10, [50, 100, 150, 255]);
        let before = img.clone();
        gaussian_blur(&mut img, 3.0, None);
        assert_eq!(img, before);
    }

    #[test]
    fn gaussian_radius_zero_noop() {
        let mut img = gradient(6, 6);
        let before = img.clone();
        gaussian_blur(&mut img, 0.0, None);
        assert_eq!(img, before);
    }

    #[test]
    fn gaussian_respects_selection() {
        let mut img = RasterImage::filled(6, 1, [0, 0, 0, 255]);
        img.set_pixel(0, 0, [255, 255, 255, 255]);
        // Select only column 5 (far from the bright pixel) — it must stay unchanged.
        let mut sel = Mask::empty(6, 1);
        sel.set(5, 0, 255);
        let before = img.pixel(1, 0);
        gaussian_blur(&mut img, 2.0, Some(&sel));
        // Unselected pixel keeps original value despite the blur computation.
        assert_eq!(img.pixel(1, 0), before);
    }

    // ── box_blur ─────────────────────────────────────────────────────────────────

    #[test]
    fn box_blur_radius_zero_identity() {
        let mut img = gradient(5, 5);
        let before = img.clone();
        box_blur(&mut img, 0, None);
        assert_eq!(img, before);
    }

    #[test]
    fn box_blur_flat_unchanged() {
        let mut img = RasterImage::filled(7, 7, [80, 80, 80, 255]);
        let before = img.clone();
        box_blur(&mut img, 2, None);
        assert_eq!(img, before);
    }

    #[test]
    fn box_blur_averages_neighbors() {
        let mut img = RasterImage::filled(3, 1, [0, 0, 0, 255]);
        img.set_pixel(1, 0, [90, 90, 90, 255]);
        box_blur(&mut img, 1, None);
        // Center averages [0,90,0] -> 30; edges clamp-to-edge -> (0+0+90)/3 = 30.
        assert_eq!(img.pixel(0, 0)[0], 30);
        assert_eq!(img.pixel(1, 0)[0], 30);
    }

    // ── motion_blur ───────────────────────────────────────────────────────────

    #[test]
    fn motion_blur_distance_zero_noop() {
        let mut img = gradient(5, 5);
        let before = img.clone();
        motion_blur(&mut img, 45.0, 0, None);
        assert_eq!(img, before);
    }

    #[test]
    fn motion_blur_flat_unchanged() {
        let mut img = RasterImage::filled(8, 8, [33, 66, 99, 255]);
        let before = img.clone();
        motion_blur(&mut img, 30.0, 5, None);
        assert_eq!(img, before);
    }

    // ── sharpen ─────────────────────────────────────────────────────────────────

    #[test]
    fn sharpen_flat_unchanged() {
        let mut img = RasterImage::filled(6, 6, [120, 130, 140, 255]);
        let before = img.clone();
        sharpen(&mut img, 1.5, None);
        assert_eq!(img, before);
    }

    #[test]
    fn sharpen_zero_noop() {
        let mut img = gradient(5, 5);
        let before = img.clone();
        sharpen(&mut img, 0.0, None);
        assert_eq!(img, before);
    }

    #[test]
    fn sharpen_increases_contrast_at_edge() {
        let mut img = RasterImage::filled(3, 1, [100, 100, 100, 255]);
        img.set_pixel(1, 0, [150, 150, 150, 255]);
        sharpen(&mut img, 1.0, None);
        // The bright center should be pushed brighter than its original 150.
        assert!(img.pixel(1, 0)[0] > 150);
    }

    // ── unsharp_mask ───────────────────────────────────────────────────────────

    #[test]
    fn unsharp_flat_unchanged() {
        let mut img = RasterImage::filled(8, 8, [60, 60, 60, 255]);
        let before = img.clone();
        unsharp_mask(&mut img, 2.0, 1.0, 0, None);
        assert_eq!(img, before);
    }

    #[test]
    fn unsharp_threshold_suppresses_small_diffs() {
        let mut img = gradient(8, 8);
        let before = img.clone();
        // A huge threshold means no difference ever qualifies → identity.
        unsharp_mask(&mut img, 2.0, 2.0, 255, None);
        assert_eq!(img, before);
    }

    // ── median ─────────────────────────────────────────────────────────────────

    #[test]
    fn median_removes_single_outlier() {
        let mut img = RasterImage::filled(5, 5, [40, 40, 40, 255]);
        img.set_pixel(2, 2, [240, 240, 240, 255]); // salt outlier
        median(&mut img, 1, None);
        // The lone bright pixel is dominated by the surrounding 40s.
        assert_eq!(img.pixel(2, 2), [40, 40, 40, 255]);
    }

    #[test]
    fn median_radius_zero_identity() {
        let mut img = gradient(4, 4);
        let before = img.clone();
        median(&mut img, 0, None);
        assert_eq!(img, before);
    }

    // ── add_noise ───────────────────────────────────────────────────────────────

    #[test]
    fn add_noise_is_deterministic() {
        let base = gradient(12, 12);
        let mut a = base.clone();
        let mut b = base.clone();
        add_noise(&mut a, 0.3, false, 42, None);
        add_noise(&mut b, 0.3, false, 42, None);
        assert_eq!(a, b);
    }

    #[test]
    fn add_noise_different_seed_differs() {
        let base = gradient(12, 12);
        let mut a = base.clone();
        let mut b = base.clone();
        add_noise(&mut a, 0.5, false, 1, None);
        add_noise(&mut b, 0.5, false, 2, None);
        assert_ne!(a, b);
    }

    #[test]
    fn add_noise_zero_amount_noop() {
        let mut img = gradient(6, 6);
        let before = img.clone();
        add_noise(&mut img, 0.0, true, 7, None);
        assert_eq!(img, before);
    }

    #[test]
    fn add_noise_preserves_alpha() {
        let mut img = RasterImage::filled(5, 5, [100, 100, 100, 200]);
        add_noise(&mut img, 0.9, false, 99, None);
        for y in 0..5 {
            for x in 0..5 {
                assert_eq!(img.pixel(x, y)[3], 200);
            }
        }
    }

    #[test]
    fn add_noise_monochrome_equal_channels() {
        let mut img = RasterImage::filled(5, 5, [80, 80, 80, 255]);
        add_noise(&mut img, 0.4, true, 5, None);
        for y in 0..5 {
            for x in 0..5 {
                let p = img.pixel(x, y);
                assert_eq!(p[0], p[1]);
                assert_eq!(p[1], p[2]);
            }
        }
    }

    // ── emboss ─────────────────────────────────────────────────────────────────

    #[test]
    fn emboss_flat_is_neutral_gray() {
        let mut img = RasterImage::filled(6, 6, [70, 90, 110, 255]);
        emboss(&mut img, None);
        for y in 0..6 {
            for x in 0..6 {
                assert_eq!(img.pixel(x, y), [128, 128, 128, 255]);
            }
        }
    }

    // ── find_edges ───────────────────────────────────────────────────────────────

    #[test]
    fn find_edges_flat_is_black() {
        let mut img = RasterImage::filled(8, 8, [123, 45, 67, 255]);
        find_edges(&mut img, None);
        for y in 0..8 {
            for x in 0..8 {
                let p = img.pixel(x, y);
                assert_eq!([p[0], p[1], p[2]], [0, 0, 0]);
            }
        }
    }

    #[test]
    fn find_edges_detects_a_step() {
        let mut img = RasterImage::filled(5, 5, [0, 0, 0, 255]);
        for y in 0..5 {
            for x in 3..5 {
                img.set_pixel(x, y, [255, 255, 255, 255]);
            }
        }
        find_edges(&mut img, None);
        // The vertical step (between cols 2 and 3) should yield a bright edge.
        assert!(img.pixel(2, 2)[0] > 50 || img.pixel(3, 2)[0] > 50);
    }

    // ── mosaic ─────────────────────────────────────────────────────────────────

    #[test]
    fn mosaic_makes_blocks_uniform() {
        let mut img = gradient(8, 8);
        mosaic(&mut img, 4, None);
        // Each 4x4 block must be a single uniform color.
        for (bx, by) in [(0u32, 0u32), (4, 0), (0, 4), (4, 4)] {
            let c = img.pixel(bx, by);
            for dy in 0..4 {
                for dx in 0..4 {
                    assert_eq!(img.pixel(bx + dx, by + dy), c);
                }
            }
        }
    }

    #[test]
    fn mosaic_block_one_noop() {
        let mut img = gradient(5, 5);
        let before = img.clone();
        mosaic(&mut img, 1, None);
        assert_eq!(img, before);
    }

    #[test]
    fn mosaic_handles_partial_edge_blocks() {
        // 5x5 with block 4 → a 4x4 cell and 1px edge strips; must not panic.
        let mut img = gradient(5, 5);
        mosaic(&mut img, 4, None);
        assert_eq!(img.width, 5);
        assert_eq!(img.height, 5);
    }

    // ── high_pass ─────────────────────────────────────────────────────────────

    #[test]
    fn high_pass_flat_is_mid_gray() {
        let mut img = RasterImage::filled(8, 8, [200, 50, 90, 255]);
        high_pass(&mut img, 3.0, None);
        for y in 0..8 {
            for x in 0..8 {
                let p = img.pixel(x, y);
                assert_eq!([p[0], p[1], p[2]], [128, 128, 128]);
            }
        }
    }

    #[test]
    fn high_pass_preserves_alpha() {
        let mut img = RasterImage::filled(4, 4, [10, 20, 30, 111]);
        high_pass(&mut img, 2.0, None);
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(img.pixel(x, y)[3], 111);
            }
        }
    }

    // ── BUG 1: no public function panics on NaN / inf / huge inputs ──────────────

    #[test]
    fn no_panic_on_hostile_radius_and_amount() {
        let bad_f32 = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e30, -1e30, 0.0];
        let bad_u32 = [u32::MAX, u32::MAX - 1, 1_000_000, 0];

        // gaussian_blur_gray (standalone single-channel)
        for &r in &bad_f32 {
            let data = vec![123u8; 4 * 4];
            let out = gaussian_blur_gray(&data, 4, 4, r);
            assert_eq!(out.len(), data.len());
        }

        // Raster-image filters
        for &r in &bad_f32 {
            let mut img = gradient(4, 4);
            gaussian_blur(&mut img, r, None);
            let mut img = gradient(4, 4);
            sharpen(&mut img, r, None);
            let mut img = gradient(4, 4);
            unsharp_mask(&mut img, r, r, 0, None);
            let mut img = gradient(4, 4);
            high_pass(&mut img, r, None);
            let mut img = gradient(4, 4);
            add_noise(&mut img, r, false, 7, None);
            let mut img = gradient(4, 4);
            motion_blur(&mut img, r, 3, None);
        }

        for &n in &bad_u32 {
            let mut img = gradient(4, 4);
            box_blur(&mut img, n, None);
            let mut img = gradient(4, 4);
            motion_blur(&mut img, 45.0, n, None);
            let mut img = gradient(4, 4);
            median(&mut img, n, None);
            let mut img = gradient(4, 4);
            mosaic(&mut img, n, None);
        }

        // No-arg neighborhood filters never panic either.
        let mut img = gradient(4, 4);
        emboss(&mut img, None);
        let mut img = gradient(4, 4);
        find_edges(&mut img, None);
    }

    // ── BUG 2: premultiplied blur does not darken opaque pixels near transparency ─

    /// An opaque white region adjacent to a fully-transparent region must stay
    /// ~white after a Gaussian blur — straight-alpha averaging would pull the
    /// edge toward gray because transparent pixels carry black RGB.
    #[test]
    fn gaussian_blur_premultiplied_no_dark_fringe() {
        let (w, h) = (16u32, 4u32);
        let mut img = RasterImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                if x < w / 2 {
                    img.set_pixel(x, y, [255, 255, 255, 255]); // opaque white
                } else {
                    img.set_pixel(x, y, [0, 0, 0, 0]); // fully transparent
                }
            }
        }
        gaussian_blur(&mut img, 2.0, None);
        // Opaque pixel right at the boundary: must remain essentially white.
        let p = img.pixel(w / 2 - 1, h / 2);
        assert!(
            p[0] > 240 && p[1] > 240 && p[2] > 240,
            "opaque side darkened by straight-alpha blur: {p:?}"
        );
    }

    /// The same premultiplied invariant for the box blur.
    #[test]
    fn box_blur_premultiplied_no_dark_fringe() {
        let (w, h) = (16u32, 4u32);
        let mut img = RasterImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                if x < w / 2 {
                    img.set_pixel(x, y, [255, 255, 255, 255]);
                } else {
                    img.set_pixel(x, y, [0, 0, 0, 0]);
                }
            }
        }
        box_blur(&mut img, 2, None);
        let p = img.pixel(w / 2 - 1, h / 2);
        assert!(
            p[0] > 240 && p[1] > 240 && p[2] > 240,
            "box blur darkened opaque side: {p:?}"
        );
    }
}
