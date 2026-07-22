//! Raster-kernel bridge into the video catalogue (K-B16 / 30 §4).
//!
//! Converts the video working buffer ([`super::ops::Image`] — linear
//! premultiplied f32) to/from [`RasterImage`] under the effect's
//! [`OperandSpace`](photonic_core::timeline::effect_manifest::OperandSpace),
//! then dispatches the existing `photonic_core::raster` CPU oracle.
//!
//! # Operand contract (30 §4.2)
//!
//! | Space | Conversion |
//! |---|---|
//! | `TransferStraight` | unpremult → sRGB encode → kernel → linear decode → premult |
//! | `LinearStraight`   | unpremult → *linear* 0..1 as u8 → kernel → back → premult |
//!
//! LinearStraight avoids the gamma encode that would halo spatial ops. The
//! photo kernels still see 8-bit quantised values; GPU twins operate in true
//! linear f32 and are the interactive path. Transfer ops (levels, curves,
//! posterize…) must run in the encoded domain — matching the photo editor.
//!
//! Bridged effects lower as `EffectKind::Unknown(tag)`. The GPU evaluator has
//! WGSL twins for the neighbourhood/point ops listed in [`BRIDGED_IDS`].

use photonic_core::raster::adjust::{
    black_and_white, channel_mixer, desaturate, hue_saturation, invert, levels, posterize,
    threshold, vibrance,
};
use photonic_core::raster::advanced::{chromatic_aberration, clarity, vignette};
use photonic_core::raster::filter::{
    add_noise, box_blur, emboss, find_edges, gaussian_blur, high_pass, median, mosaic, motion_blur,
    unsharp_mask,
};
use photonic_core::timeline::effect_manifest::{manifest, EffectId, OperandSpace};
use photonic_core::RasterImage;

use crate::contract::ResolvedParams;
use super::ops::Image;

/// Stable EffectId strings this bridge can evaluate on CPU.
pub const BRIDGED_IDS: &[&str] = &[
    // Neighbourhood / stylize (slice + Tier 1)
    "blur.box",
    "blur.gaussian",
    "blur.motion",
    "sharpen.unsharp_raster",
    "filter.high_pass",
    "filter.median",
    "stylize.emboss",
    "stylize.find_edges",
    "stylize.mosaic",
    "stylize.grain",
    "stylize.vignette",
    "stylize.chromatic_aberration",
    // Transfer color
    "color.levels",
    "color.posterize",
    "color.threshold",
    "color.channel_mixer",
    "color.hue_saturation",
    "color.vibrance",
    "color.desaturate",
    "color.black_and_white",
    "color.invert_raster",
    // Linear local-contrast / polish
    "color.clarity",
];

pub fn is_bridged(id: &str) -> bool {
    BRIDGED_IDS.contains(&id)
}

/// Operand space for a bridged id — from the manifest when present, else Linear.
pub fn operand_space(id: &str) -> OperandSpace {
    manifest(EffectId::new(id.to_string()))
        .map(|m| m.space)
        .unwrap_or(OperandSpace::LinearStraight)
}

/// Apply a bridged raster kernel. Returns `None` if `id` is not bridged.
pub fn apply(id: &str, input: &Image, params: &ResolvedParams) -> Option<Image> {
    if !is_bridged(id) {
        return None;
    }
    let space = operand_space(id);
    let mut raster = image_to_raster(input, space);
    dispatch(id, &mut raster, params)?;
    Some(raster_to_image(&raster, space))
}

/// Back-compat: radius/amount-only dispatch used by older call sites/tests.
pub fn apply_simple(id: &str, input: &Image, radius: f32, amount: f32) -> Option<Image> {
    let params = ResolvedParams {
        entries: vec![
            (
                photonic_core::timeline::PropPath::new("params.radius"),
                photonic_core::timeline::PropValue::Float(radius as f64),
            ),
            (
                photonic_core::timeline::PropPath::new("params.amount"),
                photonic_core::timeline::PropValue::Float(amount as f64),
            ),
        ],
    };
    apply(id, input, &params)
}

fn dispatch(id: &str, raster: &mut RasterImage, params: &ResolvedParams) -> Option<()> {
    let f = |path: &str, d: f32| params.f32_or(path, d);
    match id {
        "blur.box" => {
            let r = f("params.radius", 1.0).max(0.0).round() as u32;
            box_blur(raster, r.max(1), None);
        }
        "blur.gaussian" => {
            gaussian_blur(raster, f("params.radius", 1.0).max(0.0), None);
        }
        "blur.motion" => {
            let dist = f("params.distance", 8.0).max(0.0).round() as u32;
            motion_blur(raster, f("params.angle", 0.0), dist, None);
        }
        "sharpen.unsharp_raster" => {
            unsharp_mask(
                raster,
                f("params.radius", 1.0).max(0.0),
                f("params.amount", 1.0).max(0.0),
                0,
                None,
            );
        }
        "filter.high_pass" => high_pass(raster, f("params.radius", 2.0).max(0.1), None),
        "filter.median" => {
            let r = f("params.radius", 1.0).max(0.0).round() as u32;
            median(raster, r.max(1), None);
        }
        "stylize.emboss" => emboss(raster, None),
        "stylize.find_edges" => find_edges(raster, None),
        "stylize.mosaic" => {
            let b = f("params.block", 8.0).max(1.0).round() as u32;
            mosaic(raster, b, None);
        }
        "stylize.grain" => {
            let seed = f("params.seed", 1.0).max(0.0).round() as u64;
            add_noise(
                raster,
                f("params.amount", 0.1).clamp(0.0, 1.0),
                f("params.monochrome", 1.0) > 0.5,
                seed,
                None,
            );
        }
        "stylize.vignette" => {
            vignette(
                raster,
                f("params.amount", -0.5).clamp(-1.0, 1.0),
                f("params.feather", 0.5).clamp(0.0, 1.0),
                None,
            );
        }
        "stylize.chromatic_aberration" => {
            chromatic_aberration(raster, f("params.amount", 2.0), None);
        }
        "color.levels" => {
            levels(
                raster,
                f("params.in_black", 0.0),
                f("params.in_white", 1.0),
                f("params.gamma", 1.0),
                f("params.out_black", 0.0),
                f("params.out_white", 1.0),
                None,
            );
        }
        "color.posterize" => {
            let n = f("params.levels", 4.0).round().clamp(2.0, 255.0) as u32;
            posterize(raster, n, None);
        }
        "color.threshold" => threshold(raster, f("params.level", 0.5), None),
        "color.channel_mixer" => {
            // Identity matrix by default.
            let r = [
                f("params.rr", 1.0),
                f("params.rg", 0.0),
                f("params.rb", 0.0),
            ];
            let g = [
                f("params.gr", 0.0),
                f("params.gg", 1.0),
                f("params.gb", 0.0),
            ];
            let b = [
                f("params.br", 0.0),
                f("params.bg", 0.0),
                f("params.bb", 1.0),
            ];
            channel_mixer(raster, r, g, b, None);
        }
        "color.hue_saturation" => {
            hue_saturation(
                raster,
                f("params.hue", 0.0),
                f("params.saturation", 0.0),
                f("params.lightness", 0.0),
                None,
            );
        }
        "color.vibrance" => vibrance(raster, f("params.amount", 0.0), None),
        "color.desaturate" => desaturate(raster, None),
        "color.black_and_white" => {
            black_and_white(
                raster,
                [
                    f("params.wr", 0.299),
                    f("params.wg", 0.587),
                    f("params.wb", 0.114),
                ],
                None,
            );
        }
        "color.invert_raster" => invert(raster, None),
        "color.clarity" => clarity(raster, f("params.amount", 0.0), None),
        _ => return None,
    }
    Some(())
}

/// Working buffer → raster, honouring operand space.
fn image_to_raster(img: &Image, space: OperandSpace) -> RasterImage {
    let mut out = RasterImage::new(img.width, img.height);
    for (i, p) in img.pixels.iter().enumerate() {
        let a = p[3].clamp(0.0, 1.0);
        let (r, g, b) = if a > 1e-6 {
            (p[0] / a, p[1] / a, p[2] / a)
        } else {
            (0.0, 0.0, 0.0)
        };
        let o = i * 4;
        match space {
            OperandSpace::TransferStraight => {
                out.pixels[o] = linear_to_srgb_u8(r);
                out.pixels[o + 1] = linear_to_srgb_u8(g);
                out.pixels[o + 2] = linear_to_srgb_u8(b);
            }
            OperandSpace::LinearStraight => {
                // Linear 0..1 quantised to u8 — no OETF (avoids blur halos).
                out.pixels[o] = linear_to_u8(r);
                out.pixels[o + 1] = linear_to_u8(g);
                out.pixels[o + 2] = linear_to_u8(b);
            }
        }
        out.pixels[o + 3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    out
}

/// Raster → working buffer, inverse of [`image_to_raster`].
fn raster_to_image(img: &RasterImage, space: OperandSpace) -> Image {
    let mut out = Image::new(img.width, img.height);
    for (i, p) in out.pixels.iter_mut().enumerate() {
        let o = i * 4;
        let a = img.pixels[o + 3] as f32 / 255.0;
        let (r, g, b) = match space {
            OperandSpace::TransferStraight => (
                srgb_u8_to_linear(img.pixels[o]),
                srgb_u8_to_linear(img.pixels[o + 1]),
                srgb_u8_to_linear(img.pixels[o + 2]),
            ),
            OperandSpace::LinearStraight => (
                u8_to_linear(img.pixels[o]),
                u8_to_linear(img.pixels[o + 1]),
                u8_to_linear(img.pixels[o + 2]),
            ),
        };
        *p = [r * a, g * a, b * a, a];
    }
    out
}

fn linear_to_u8(c: f32) -> u8 {
    (c.clamp(0.0, 1.0) * 255.0).round().clamp(0.0, 255.0) as u8
}

fn u8_to_linear(c: u8) -> f32 {
    c as f32 / 255.0
}

fn linear_to_srgb_u8(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round().clamp(0.0, 255.0) as u8
}

fn srgb_u8_to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::ir::LinearColor;
    use crate::graph::ops::solid;

    #[test]
    fn bridged_ids_are_recognized() {
        assert!(is_bridged("stylize.emboss"));
        assert!(is_bridged("color.levels"));
        assert!(is_bridged("blur.motion"));
        assert!(!is_bridged("nope.fake"));
    }

    #[test]
    fn transfer_space_for_levels() {
        assert_eq!(operand_space("color.levels"), OperandSpace::TransferStraight);
        assert_eq!(operand_space("blur.box"), OperandSpace::LinearStraight);
    }

    #[test]
    fn emboss_changes_a_solid_edge_image() {
        let mut img = Image::new(8, 8);
        for y in 0..8 {
            for x in 0..8 {
                let v = if x < 4 { 0.0 } else { 1.0 };
                img.set(x, y, [v, v, v, 1.0]);
            }
        }
        let out = apply_simple("stylize.emboss", &img, 0.0, 0.0).expect("bridged");
        assert_ne!(out.pixels, img.pixels, "emboss must alter an edge image");
    }

    #[test]
    fn box_blur_softens_a_checker() {
        let mut img = Image::new(4, 1);
        img.set(0, 0, [1.0, 1.0, 1.0, 1.0]);
        img.set(1, 0, [0.0, 0.0, 0.0, 1.0]);
        img.set(2, 0, [1.0, 1.0, 1.0, 1.0]);
        img.set(3, 0, [0.0, 0.0, 0.0, 1.0]);
        let out = apply_simple("blur.box", &img, 1.0, 0.0).expect("bridged");
        assert!(out.pixel(0, 0)[0] < 1.0);
    }

    #[test]
    fn levels_identity_at_defaults() {
        let img = solid(
            4,
            4,
            LinearColor {
                r: 0.4,
                g: 0.5,
                b: 0.6,
                a: 1.0,
            },
        );
        let params = ResolvedParams {
            entries: vec![
                (
                    photonic_core::timeline::PropPath::new("params.in_black"),
                    photonic_core::timeline::PropValue::Float(0.0),
                ),
                (
                    photonic_core::timeline::PropPath::new("params.in_white"),
                    photonic_core::timeline::PropValue::Float(1.0),
                ),
                (
                    photonic_core::timeline::PropPath::new("params.gamma"),
                    photonic_core::timeline::PropValue::Float(1.0),
                ),
                (
                    photonic_core::timeline::PropPath::new("params.out_black"),
                    photonic_core::timeline::PropValue::Float(0.0),
                ),
                (
                    photonic_core::timeline::PropPath::new("params.out_white"),
                    photonic_core::timeline::PropValue::Float(1.0),
                ),
            ],
        };
        let out = apply("color.levels", &img, &params).expect("bridged");
        for (a, b) in img.pixels.iter().zip(out.pixels.iter()) {
            for c in 0..3 {
                assert!(
                    (a[c] - b[c]).abs() < 0.03,
                    "levels identity channel {c}: {a:?} vs {b:?}"
                );
            }
        }
    }

    #[test]
    fn posterize_reduces_unique_levels() {
        let mut img = Image::new(8, 1);
        for x in 0..8 {
            let v = x as f32 / 7.0;
            img.set(x, 0, [v, v, v, 1.0]);
        }
        let params = ResolvedParams {
            entries: vec![(
                photonic_core::timeline::PropPath::new("params.levels"),
                photonic_core::timeline::PropValue::Float(2.0),
            )],
        };
        let out = apply("color.posterize", &img, &params).expect("bridged");
        // 2 levels → only near-black and near-white.
        let mut lows = 0;
        let mut highs = 0;
        for p in &out.pixels {
            if p[0] < 0.25 {
                lows += 1;
            } else if p[0] > 0.75 {
                highs += 1;
            }
        }
        assert!(lows > 0 && highs > 0, "posterize 2 must produce both poles");
    }

    #[test]
    fn linear_roundtrip_near_identity() {
        let img = solid(
            4,
            4,
            LinearColor {
                r: 0.5,
                g: 0.25,
                b: 0.75,
                a: 1.0,
            },
        );
        let r = image_to_raster(&img, OperandSpace::LinearStraight);
        let back = raster_to_image(&r, OperandSpace::LinearStraight);
        for (a, b) in img.pixels.iter().zip(back.pixels.iter()) {
            for c in 0..4 {
                assert!(
                    (a[c] - b[c]).abs() < 0.01,
                    "linear channel {c}: {a:?} vs {b:?}"
                );
            }
        }
    }

    #[test]
    fn solid_roundtrip_is_near_identity_transfer() {
        let img = solid(
            4,
            4,
            LinearColor {
                r: 0.5,
                g: 0.25,
                b: 0.75,
                a: 1.0,
            },
        );
        let r = image_to_raster(&img, OperandSpace::TransferStraight);
        let back = raster_to_image(&r, OperandSpace::TransferStraight);
        for (a, b) in img.pixels.iter().zip(back.pixels.iter()) {
            for c in 0..4 {
                assert!(
                    (a[c] - b[c]).abs() < 0.02,
                    "channel {c}: {a:?} vs {b:?}"
                );
            }
        }
    }
}
