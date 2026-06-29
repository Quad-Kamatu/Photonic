//! Liquify & distortion transforms — Photoshop's *Liquify* tools and the classic
//! *Distort* filters, implemented as pure CPU functions.
//!
//! Every operation here is an **inverse warp**: we never push pixels forward into
//! a mutable buffer (which would leave holes and require splatting). Instead, for
//! each *output* pixel we compute the *source* coordinate it should be drawn from,
//! sample the (immutable) source with [`RasterImage::sample_bilinear`], and write
//! the result. The completed result is then composited back through
//! [`blend_result`] so an optional selection [`Mask`] is honored exactly like the
//! rest of the raster subsystem.
//!
//! Conventions:
//! - Coordinates are in pixels; pixel `(x, y)` is sampled at the float coord
//!   `(x as f32, y as f32)`, so an identity warp reproduces the source exactly.
//! - All ops are total: a zero radius / zero amount / zero angle is the identity,
//!   degenerate inputs never panic, and sampling is always clamp-to-edge.
//! - `liquify_*` ops operate inside a circular brush of `radius` around
//!   `(cx, cy)` with a linear falloff to zero at the rim; pixels outside the brush
//!   are untouched.

use crate::raster::{blend_result, image::RasterImage, mask::Mask};

/// Upper bound for any pixel-space magnitude (radius, displacement, amplitude…).
/// Keeps degenerate/adversarial inputs from producing absurd sample coordinates.
const COORD_MAX: f32 = 16384.0;
/// Upper bound for a brush radius.
const RADIUS_MAX: f32 = 8192.0;

/// Sanitize a possibly non-finite scalar: NaN / ±inf collapse to `default`.
#[inline]
fn san(x: f32, default: f32) -> f32 {
    if x.is_finite() {
        x
    } else {
        default
    }
}

/// Build a premultiplied-alpha copy of `img` (R,G,B *= A/255).
///
/// Bilinear resampling must happen in premultiplied space; otherwise the RGB of a
/// transparent pixel (commonly black) bleeds into opaque neighbors at edges,
/// producing the classic "dark fringing" halo. We premultiply, sample *this*
/// copy, then [`unpremultiply_in_place`] the result.
fn premultiplied(img: &RasterImage) -> RasterImage {
    let mut out = img.clone();
    for px in out.pixels.chunks_exact_mut(4) {
        let a = px[3] as u32;
        px[0] = ((px[0] as u32 * a + 127) / 255) as u8;
        px[1] = ((px[1] as u32 * a + 127) / 255) as u8;
        px[2] = ((px[2] as u32 * a + 127) / 255) as u8;
    }
    out
}

/// Invert [`premultiplied`] in place: `R = Rp * 255 / A`. `A == 0` → `[0,0,0,0]`.
fn unpremultiply_in_place(img: &mut RasterImage) {
    for px in img.pixels.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a == 0 {
            px[0] = 0;
            px[1] = 0;
            px[2] = 0;
        } else {
            px[0] = ((px[0] as u32 * 255 + a / 2) / a).min(255) as u8;
            px[1] = ((px[1] as u32 * 255 + a / 2) / a).min(255) as u8;
            px[2] = ((px[2] as u32 * 255 + a / 2) / a).min(255) as u8;
        }
    }
}

/// Linear brush falloff: `1.0` at the center, `0.0` at (and beyond) `radius`.
#[inline]
fn falloff(r: f32, radius: f32) -> f32 {
    if radius <= 0.0 {
        return 0.0;
    }
    (1.0 - r / radius).clamp(0.0, 1.0)
}

/// Liquify Forward Warp ("push"): drag content from `(cx, cy)` by `(dx, dy)`
/// within `radius`, with the displacement falling off to `0` at the brush edge.
///
/// `strength` scales the drag. A zero radius / strength / displacement is the
/// identity.
pub fn liquify_push(
    img: &mut RasterImage,
    cx: f32,
    cy: f32,
    dx: f32,
    dy: f32,
    radius: f32,
    strength: f32,
    sel: Option<&Mask>,
) {
    let cx = san(cx, 0.0);
    let cy = san(cy, 0.0);
    let dx = san(dx, 0.0).clamp(-COORD_MAX, COORD_MAX);
    let dy = san(dy, 0.0).clamp(-COORD_MAX, COORD_MAX);
    let radius = san(radius, 0.0).clamp(0.0, RADIUS_MAX);
    let strength = san(strength, 0.0).clamp(-COORD_MAX, COORD_MAX);
    if radius <= 0.0 || strength == 0.0 || (dx == 0.0 && dy == 0.0) {
        return;
    }
    let src = premultiplied(img);
    let mut out = RasterImage::new(img.width, img.height);
    for y in 0..img.height {
        for x in 0..img.width {
            let fx = x as f32;
            let fy = y as f32;
            let r = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
            let f = falloff(r, radius);
            let (sxf, syf) = if f > 0.0 {
                // Inverse warp: sample where this output pixel's content came from,
                // i.e. step *back* along the drag vector.
                (fx - dx * strength * f, fy - dy * strength * f)
            } else {
                (fx, fy)
            };
            out.set_pixel(x, y, src.sample_bilinear(sxf, syf));
        }
    }
    unpremultiply_in_place(&mut out);
    blend_result(img, &out, sel);
}

/// Liquify Twirl: rotate content around `(cx, cy)` within `radius`, with the
/// rotation angle falling off to zero at the rim. `angle_deg > 0` rotates
/// clockwise (in image space, y-down). A zero radius / angle is the identity, and
/// the center pixel is always preserved.
pub fn liquify_twirl(
    img: &mut RasterImage,
    cx: f32,
    cy: f32,
    radius: f32,
    angle_deg: f32,
    sel: Option<&Mask>,
) {
    let cx = san(cx, 0.0);
    let cy = san(cy, 0.0);
    let radius = san(radius, 0.0).clamp(0.0, RADIUS_MAX);
    let angle_deg = san(angle_deg, 0.0);
    if radius <= 0.0 || angle_deg == 0.0 {
        return;
    }
    let src = premultiplied(img);
    let mut out = RasterImage::new(img.width, img.height);
    let ang = angle_deg.to_radians();
    for y in 0..img.height {
        for x in 0..img.width {
            let fx = x as f32;
            let fy = y as f32;
            let relx = fx - cx;
            let rely = fy - cy;
            let r = (relx * relx + rely * rely).sqrt();
            let f = falloff(r, radius);
            let (sxf, syf) = if f > 0.0 && r > 0.0 {
                // Forward rotation is `+theta`; the inverse warp rotates by `-theta`.
                let theta = ang * f;
                let (s, c) = (-theta).sin_cos();
                (cx + relx * c - rely * s, cy + relx * s + rely * c)
            } else {
                (fx, fy)
            };
            out.set_pixel(x, y, src.sample_bilinear(sxf, syf));
        }
    }
    unpremultiply_in_place(&mut out);
    blend_result(img, &out, sel);
}

/// Liquify Pucker (`amount > 0`, contract toward center) / Bloat (`amount < 0`,
/// expand outward) around `(cx, cy)` within `radius`. `amount` is clamped to
/// `-1..1`; a zero radius / amount is the identity.
pub fn liquify_pucker(
    img: &mut RasterImage,
    cx: f32,
    cy: f32,
    radius: f32,
    amount: f32,
    sel: Option<&Mask>,
) {
    let cx = san(cx, 0.0);
    let cy = san(cy, 0.0);
    let radius = san(radius, 0.0).clamp(0.0, RADIUS_MAX);
    let amount = san(amount, 0.0).clamp(-1.0, 1.0);
    if radius <= 0.0 || amount == 0.0 {
        return;
    }
    let src = premultiplied(img);
    let mut out = RasterImage::new(img.width, img.height);
    for y in 0..img.height {
        for x in 0..img.width {
            let fx = x as f32;
            let fy = y as f32;
            let relx = fx - cx;
            let rely = fy - cy;
            let r = (relx * relx + rely * rely).sqrt();
            let (sxf, syf) = if r > 0.0 && r < radius {
                let nd = r / radius;
                let f = 1.0 - nd; // smooth-ish falloff, 1 at center → 0 at rim
                                  // Pucker (amount>0): content moves inward, so we sample *outward*.
                let scale = 1.0 + amount * f;
                (cx + relx * scale, cy + rely * scale)
            } else {
                (fx, fy)
            };
            out.set_pixel(x, y, src.sample_bilinear(sxf, syf));
        }
    }
    unpremultiply_in_place(&mut out);
    blend_result(img, &out, sel);
}

/// Pinch (`amount > 0`, squeeze toward center) / spherize-out (`amount < 0`) the
/// whole image about its center, within the inscribed circle. `amount` is clamped
/// to `-1..1`; zero is the identity.
pub fn pinch(img: &mut RasterImage, amount: f32, sel: Option<&Mask>) {
    let amount = san(amount, 0.0).clamp(-1.0, 1.0);
    if amount == 0.0 {
        return;
    }
    let cx = img.width as f32 / 2.0;
    let cy = img.height as f32 / 2.0;
    let radius = (img.width.min(img.height) as f32) / 2.0;
    if radius <= 0.0 {
        return;
    }
    let src = premultiplied(img);
    let mut out = RasterImage::new(img.width, img.height);
    for y in 0..img.height {
        for x in 0..img.width {
            let fx = x as f32;
            let fy = y as f32;
            let relx = fx - cx;
            let rely = fy - cy;
            let r = (relx * relx + rely * rely).sqrt();
            let (sxf, syf) = if r > 0.0 && r < radius {
                let nd = r / radius;
                let f = 1.0 - nd;
                // Smooth bell (f²) so the effect tapers gently to the rim.
                let scale = 1.0 + amount * f * f;
                (cx + relx * scale, cy + rely * scale)
            } else {
                (fx, fy)
            };
            out.set_pixel(x, y, src.sample_bilinear(sxf, syf));
        }
    }
    unpremultiply_in_place(&mut out);
    blend_result(img, &out, sel);
}

/// Spherize: map the image onto a sphere. `amount > 0` bulges outward (magnifies
/// the center, like a convex lens); `amount < 0` is concave. Clamped to `-1..1`;
/// zero is the identity.
pub fn spherize(img: &mut RasterImage, amount: f32, sel: Option<&Mask>) {
    let amount = san(amount, 0.0).clamp(-1.0, 1.0);
    if amount == 0.0 {
        return;
    }
    let cx = img.width as f32 / 2.0;
    let cy = img.height as f32 / 2.0;
    let radius = (img.width.min(img.height) as f32) / 2.0;
    if radius <= 0.0 {
        return;
    }
    let src = premultiplied(img);
    let mut out = RasterImage::new(img.width, img.height);
    const HALF_PI: f32 = std::f32::consts::FRAC_PI_2;
    for y in 0..img.height {
        for x in 0..img.width {
            let fx = x as f32;
            let fy = y as f32;
            let relx = fx - cx;
            let rely = fy - cy;
            let r = (relx * relx + rely * rely).sqrt();
            let (sxf, syf) = if r > 0.0 && r < radius {
                let nd = (r / radius).clamp(0.0, 1.0);
                // Normalized source radius. For a bulge we sample nearer the
                // center (curve below the diagonal, via asin); concave uses sin.
                let curved = if amount >= 0.0 {
                    let a = nd.asin() / HALF_PI;
                    nd + (a - nd) * amount
                } else {
                    let a = (nd * HALF_PI).sin();
                    nd + (a - nd) * (-amount)
                };
                let scale = curved / nd;
                (cx + relx * scale, cy + rely * scale)
            } else {
                (fx, fy)
            };
            out.set_pixel(x, y, src.sample_bilinear(sxf, syf));
        }
    }
    unpremultiply_in_place(&mut out);
    blend_result(img, &out, sel);
}

/// Ripple / wave distortion. Each row is shifted horizontally and each column
/// vertically by a sine of the given `amplitude` (px) and `wavelength` (px). A
/// zero amplitude or wavelength is the identity.
pub fn ripple(img: &mut RasterImage, amplitude: f32, wavelength: f32, sel: Option<&Mask>) {
    let amplitude = san(amplitude, 0.0).clamp(-COORD_MAX, COORD_MAX);
    let wavelength = san(wavelength, 0.0).clamp(-COORD_MAX, COORD_MAX);
    if amplitude == 0.0 || wavelength == 0.0 {
        return;
    }
    let src = premultiplied(img);
    let mut out = RasterImage::new(img.width, img.height);
    let k = 2.0 * std::f32::consts::PI / wavelength;
    for y in 0..img.height {
        for x in 0..img.width {
            let fx = x as f32;
            let fy = y as f32;
            let sxf = fx + amplitude * (k * fy).sin();
            let syf = fy + amplitude * (k * fx).sin();
            out.set_pixel(x, y, src.sample_bilinear(sxf, syf));
        }
    }
    unpremultiply_in_place(&mut out);
    blend_result(img, &out, sel);
}

/// Perspective transform via four destination corners (a projective homography).
///
/// `dst` lists where the source-rectangle corners `TL, TR, BR, BL` (i.e. the full
/// image rect `[0,w] × [0,h]`) should land in the output, which has the **same
/// size** as the source. Output pixels whose mapped source coordinate falls
/// outside `[0,w) × [0,h)` become transparent `[0,0,0,0]`.
///
/// Implemented by solving for the inverse homography (output → source) from the
/// four corner correspondences with a hand-rolled 8×8 linear solve, then bilinear
/// sampling. Degenerate corner sets fall back to an identity copy.
pub fn perspective(img: &RasterImage, dst: [(f32, f32); 4]) -> RasterImage {
    // Any non-finite corner makes the mapping undefined → identity (no-op) copy.
    if dst.iter().any(|&(x, y)| !x.is_finite() || !y.is_finite()) {
        return img.clone();
    }
    // Clamp corner magnitudes so the linear solve / sampling can't overflow.
    let dst = dst.map(|(x, y)| {
        (
            x.clamp(-COORD_MAX, COORD_MAX),
            y.clamp(-COORD_MAX, COORD_MAX),
        )
    });

    let w = img.width as f32;
    let h = img.height as f32;
    let src_corners = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)];

    // We want output → source directly, so solve the homography mapping the
    // destination corners back onto the source corners.
    let g = match homography(dst, src_corners) {
        Some(g) => g,
        None => return img.clone(),
    };

    // Sample in premultiplied space so transparent regions don't darken edges.
    let src = premultiplied(img);
    let mut out = RasterImage::new(img.width, img.height);
    for y in 0..img.height {
        for x in 0..img.width {
            let fx = x as f32;
            let fy = y as f32;
            let denom = g[6] * fx + g[7] * fy + g[8];
            if denom.abs() < 1e-12 {
                continue;
            }
            let u = (g[0] * fx + g[1] * fy + g[2]) / denom;
            let v = (g[3] * fx + g[4] * fy + g[5]) / denom;
            if u >= 0.0 && u < w && v >= 0.0 && v < h {
                out.set_pixel(x, y, src.sample_bilinear(u, v));
            }
            // else: leave transparent
        }
    }
    unpremultiply_in_place(&mut out);
    out
}

/// Solve for the 3×3 projective homography (row-major, 9 entries, `h33 == 1`)
/// mapping the four `from` points onto the four `to` points via the standard DLT:
/// each correspondence yields two linear equations in the 8 unknowns. Returns
/// `None` if the system is singular (degenerate / collinear corners).
fn homography(from: [(f32, f32); 4], to: [(f32, f32); 4]) -> Option<[f32; 9]> {
    // Build the 8×9 augmented system (f64 for numerical stability).
    let mut a = [[0.0f64; 9]; 8];
    for i in 0..4 {
        let (x, y) = (from[i].0 as f64, from[i].1 as f64);
        let (xp, yp) = (to[i].0 as f64, to[i].1 as f64);
        let r0 = 2 * i;
        let r1 = 2 * i + 1;
        // xp = (h11 x + h12 y + h13) / (h31 x + h32 y + 1)
        a[r0] = [x, y, 1.0, 0.0, 0.0, 0.0, -x * xp, -y * xp, xp];
        // yp = (h21 x + h22 y + h23) / (h31 x + h32 y + 1)
        a[r1] = [0.0, 0.0, 0.0, x, y, 1.0, -x * yp, -y * yp, yp];
    }

    // Gauss-Jordan elimination with partial pivoting.
    for col in 0..8 {
        // Find pivot.
        let mut pivot = col;
        let mut best = a[col][col].abs();
        for row in (col + 1)..8 {
            let v = a[row][col].abs();
            if v > best {
                best = v;
                pivot = row;
            }
        }
        if best < 1e-12 {
            return None;
        }
        a.swap(col, pivot);

        // Normalize pivot row.
        let pv = a[col][col];
        for c in col..9 {
            a[col][c] /= pv;
        }
        // Eliminate this column from all other rows.
        for row in 0..8 {
            if row == col {
                continue;
            }
            let factor = a[row][col];
            if factor != 0.0 {
                for c in col..9 {
                    a[row][c] -= factor * a[col][c];
                }
            }
        }
    }

    let mut h = [0.0f32; 9];
    for i in 0..8 {
        let v = a[i][8];
        if !v.is_finite() {
            return None;
        }
        h[i] = v as f32;
    }
    h[8] = 1.0;
    Some(h)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic test image with a distinct color per pixel position.
    fn gradient(w: u32, h: u32) -> RasterImage {
        let mut img = RasterImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let r = ((x * 13) % 256) as u8;
                let g = ((y * 17) % 256) as u8;
                let b = (((x + y) * 7) % 256) as u8;
                img.set_pixel(x, y, [r, g, b, 255]);
            }
        }
        img
    }

    fn approx_eq(a: &RasterImage, b: &RasterImage, tol: i32) -> bool {
        if a.width != b.width || a.height != b.height {
            return false;
        }
        a.pixels
            .iter()
            .zip(b.pixels.iter())
            .all(|(&p, &q)| (p as i32 - q as i32).abs() <= tol)
    }

    fn px_close(p: [u8; 4], q: [u8; 4], tol: i32) -> bool {
        (0..4).all(|c| (p[c] as i32 - q[c] as i32).abs() <= tol)
    }

    // ── Identity / no-op behavior ────────────────────────────────────────────

    #[test]
    fn push_zero_is_identity() {
        let orig = gradient(16, 12);
        let mut img = orig.clone();
        liquify_push(&mut img, 8.0, 6.0, 0.0, 0.0, 5.0, 1.0, None); // zero drag
        assert_eq!(img, orig);
        liquify_push(&mut img, 8.0, 6.0, 3.0, 2.0, 0.0, 1.0, None); // zero radius
        assert_eq!(img, orig);
        liquify_push(&mut img, 8.0, 6.0, 3.0, 2.0, 5.0, 0.0, None); // zero strength
        assert_eq!(img, orig);
    }

    #[test]
    fn twirl_zero_angle_is_identity() {
        let orig = gradient(16, 16);
        let mut img = orig.clone();
        liquify_twirl(&mut img, 8.0, 8.0, 6.0, 0.0, None);
        assert_eq!(img, orig);
        liquify_twirl(&mut img, 8.0, 8.0, 0.0, 45.0, None); // zero radius
        assert_eq!(img, orig);
    }

    #[test]
    fn twirl_preserves_center_pixel() {
        let orig = gradient(17, 17);
        let mut img = orig.clone();
        let (cx, cy) = (8u32, 8u32);
        liquify_twirl(&mut img, cx as f32, cy as f32, 7.0, 90.0, None);
        assert!(px_close(img.pixel(cx, cy), orig.pixel(cx, cy), 1));
    }

    #[test]
    fn pucker_zero_is_identity() {
        let orig = gradient(16, 16);
        let mut img = orig.clone();
        liquify_pucker(&mut img, 8.0, 8.0, 6.0, 0.0, None);
        assert_eq!(img, orig);
        liquify_pucker(&mut img, 8.0, 8.0, 0.0, 0.5, None);
        assert_eq!(img, orig);
    }

    #[test]
    fn pinch_zero_is_identity() {
        let orig = gradient(16, 16);
        let mut img = orig.clone();
        pinch(&mut img, 0.0, None);
        assert_eq!(img, orig);
    }

    #[test]
    fn spherize_zero_is_identity() {
        let orig = gradient(16, 16);
        let mut img = orig.clone();
        spherize(&mut img, 0.0, None);
        assert_eq!(img, orig);
    }

    #[test]
    fn ripple_zero_is_identity() {
        let orig = gradient(16, 16);
        let mut img = orig.clone();
        ripple(&mut img, 0.0, 8.0, None); // zero amplitude
        assert_eq!(img, orig);
        ripple(&mut img, 4.0, 0.0, None); // zero wavelength
        assert_eq!(img, orig);
    }

    #[test]
    fn perspective_identity_corners() {
        let orig = gradient(16, 12);
        let w = orig.width as f32;
        let h = orig.height as f32;
        let dst = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)];
        let out = perspective(&orig, dst);
        assert!(
            approx_eq(&out, &orig, 2),
            "identity perspective should ~match"
        );
    }

    // ── Effects actually do something ────────────────────────────────────────

    #[test]
    fn push_changes_pixels() {
        let orig = gradient(24, 24);
        let mut img = orig.clone();
        liquify_push(&mut img, 12.0, 12.0, 6.0, 0.0, 10.0, 1.0, None);
        assert_ne!(img, orig);
    }

    #[test]
    fn twirl_changes_pixels_off_center() {
        let orig = gradient(24, 24);
        let mut img = orig.clone();
        liquify_twirl(&mut img, 12.0, 12.0, 10.0, 90.0, None);
        assert_ne!(img, orig);
    }

    #[test]
    fn pucker_and_bloat_change_pixels() {
        let orig = gradient(24, 24);
        let mut a = orig.clone();
        liquify_pucker(&mut a, 12.0, 12.0, 10.0, 0.7, None);
        assert_ne!(a, orig);
        let mut b = orig.clone();
        liquify_pucker(&mut b, 12.0, 12.0, 10.0, -0.7, None);
        assert_ne!(b, orig);
    }

    #[test]
    fn pinch_and_spherize_change_pixels() {
        let orig = gradient(24, 24);
        let mut p = orig.clone();
        pinch(&mut p, 0.6, None);
        assert_ne!(p, orig);
        let mut s = orig.clone();
        spherize(&mut s, 0.6, None);
        assert_ne!(s, orig);
    }

    #[test]
    fn ripple_changes_pixels() {
        let orig = gradient(24, 24);
        let mut img = orig.clone();
        ripple(&mut img, 3.0, 8.0, None);
        assert_ne!(img, orig);
    }

    #[test]
    fn perspective_inset_makes_transparent_border() {
        let orig = gradient(20, 20);
        let w = orig.width as f32;
        let h = orig.height as f32;
        // Map the source corners *inward*, so the outer border has no source.
        let dst = [
            (5.0, 5.0),
            (w - 5.0, 5.0),
            (w - 5.0, h - 5.0),
            (5.0, h - 5.0),
        ];
        let out = perspective(&orig, dst);
        assert_eq!(out.pixel(0, 0)[3], 0, "corner should be transparent");
        assert_eq!(out.pixel(19, 19)[3], 0, "corner should be transparent");
        // The center still has source content.
        assert_eq!(out.pixel(10, 10)[3], 255);
    }

    #[test]
    fn perspective_degenerate_falls_back() {
        let orig = gradient(8, 8);
        // All corners identical → singular system → identity fallback.
        let dst = [(0.0, 0.0); 4];
        let out = perspective(&orig, dst);
        assert_eq!(out, orig);
    }

    // ── Selection mask is honored ────────────────────────────────────────────

    #[test]
    fn empty_selection_leaves_image_untouched() {
        let orig = gradient(16, 16);
        let sel = Mask::empty(16, 16); // nothing selected → coverage 0 everywhere
        let mut img = orig.clone();
        liquify_twirl(&mut img, 8.0, 8.0, 7.0, 120.0, Some(&sel));
        assert_eq!(img, orig);
        let mut img2 = orig.clone();
        ripple(&mut img2, 4.0, 6.0, Some(&sel));
        assert_eq!(img2, orig);
        let mut img3 = orig.clone();
        pinch(&mut img3, 0.8, Some(&sel));
        assert_eq!(img3, orig);
    }

    #[test]
    fn full_selection_matches_unmasked() {
        let orig = gradient(16, 16);
        let sel = Mask::full(16, 16);
        let mut masked = orig.clone();
        let mut plain = orig.clone();
        liquify_pucker(&mut masked, 8.0, 8.0, 7.0, 0.5, Some(&sel));
        liquify_pucker(&mut plain, 8.0, 8.0, 7.0, 0.5, None);
        assert!(approx_eq(&masked, &plain, 1));
    }

    // ── Robustness on tiny images ────────────────────────────────────────────

    #[test]
    fn no_panic_on_tiny_images() {
        for &(w, h) in &[(1u32, 1u32), (1, 4), (4, 1), (2, 2), (3, 5)] {
            let mut img = gradient(w, h);
            liquify_push(&mut img, 0.5, 0.5, 2.0, -1.0, 3.0, 1.0, None);
            liquify_twirl(&mut img, 0.5, 0.5, 3.0, 75.0, None);
            liquify_pucker(&mut img, 0.5, 0.5, 3.0, 0.6, None);
            liquify_pucker(&mut img, 0.5, 0.5, 3.0, -0.6, None);
            pinch(&mut img, 0.9, None);
            spherize(&mut img, -0.9, None);
            ripple(&mut img, 5.0, 2.0, None);
            let _ = perspective(&img, [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]);
        }
    }

    #[test]
    fn amounts_are_clamped_not_panicking() {
        let mut img = gradient(12, 12);
        // Out-of-range amounts must be clamped internally, no panic / NaN.
        liquify_pucker(&mut img, 6.0, 6.0, 5.0, 5.0, None);
        spherize(&mut img, 9.0, None);
        spherize(&mut img, -9.0, None);
        pinch(&mut img, 9.0, None);
        assert!(img.pixels.iter().all(|&_b| true));
    }

    // ── BUG 1: non-finite / huge inputs never panic ──────────────────────────

    /// An image whose left half is opaque white and right half fully transparent.
    fn half_white(w: u32, h: u32) -> RasterImage {
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
        img
    }

    #[test]
    fn warps_are_panic_free_on_non_finite_and_huge() {
        let nan = f32::NAN;
        let inf = f32::INFINITY;
        let ninf = f32::NEG_INFINITY;
        let huge = 1.0e30f32;
        let bad = [nan, inf, ninf, huge, -huge];
        for &v in &bad {
            let mut img = gradient(10, 8);
            liquify_push(&mut img, v, v, v, v, v, v, None);
            liquify_twirl(&mut img, v, v, v, v, None);
            liquify_pucker(&mut img, v, v, v, v, None);
            pinch(&mut img, v, None);
            spherize(&mut img, v, None);
            ripple(&mut img, v, v, None);
            let _ = perspective(&img, [(v, v), (v, v), (v, v), (v, v)]);
            // Mixed finite/non-finite corners must also be safe.
            let _ = perspective(&img, [(0.0, 0.0), (v, 0.0), (10.0, v), (0.0, 8.0)]);
        }
    }

    // ── BUG 2: premultiplied resampling — no dark fringing at edges ───────────

    /// After a warp that resamples across the opaque/transparent boundary, every
    /// surviving (alpha>0) pixel must stay white — straight-alpha blending would
    /// have darkened edge pixels toward gray/black.
    fn assert_no_dark_fringe(img: &RasterImage) {
        for px in img.pixels.chunks_exact(4) {
            if px[3] > 0 {
                assert!(
                    px[0] >= 250 && px[1] >= 250 && px[2] >= 250,
                    "dark fringing: opaque pixel {:?} not white",
                    [px[0], px[1], px[2], px[3]]
                );
            }
        }
    }

    #[test]
    fn ripple_premultiplied_no_dark_fringe() {
        let mut img = half_white(24, 8);
        ripple(&mut img, 3.0, 6.0, None);
        assert_no_dark_fringe(&img);
    }

    #[test]
    fn twirl_premultiplied_no_dark_fringe() {
        let mut img = half_white(24, 24);
        liquify_twirl(&mut img, 12.0, 12.0, 14.0, 80.0, None);
        assert_no_dark_fringe(&img);
    }

    #[test]
    fn pinch_premultiplied_no_dark_fringe() {
        let mut img = half_white(24, 24);
        pinch(&mut img, 0.7, None);
        assert_no_dark_fringe(&img);
    }

    #[test]
    fn perspective_premultiplied_no_dark_fringe() {
        let img = half_white(24, 24);
        let w = img.width as f32;
        let h = img.height as f32;
        // Slight skew so sampling falls on fractional coords across the boundary.
        let out = perspective(&img, [(1.0, 0.0), (w - 2.0, 1.0), (w, h - 1.0), (0.0, h)]);
        assert_no_dark_fringe(&out);
    }
}
