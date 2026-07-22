//! Raster-kernel bridge into the video catalogue (K-B16 / 30 §4).
//!
//! Photonic's `photonic_core::raster` library already owns ~dozen neighbourhood
//! filters with tested CPU oracles. This module converts the video working
//! buffer ([`super::ops::Image`] — linear premultiplied f32) to/from
//! [`RasterImage`] (sRGB8 straight) and dispatches named kernels by
//! [`EffectId`](photonic_core::timeline::EffectId) string.
//!
//! Bridged effects lower as `EffectKind::Unknown(tag)` so the IR stays stable.
//! The GPU evaluator has WGSL twins for all six bridged ids (box/unsharp via
//! blur/sharpen reuse; high-pass combine; emboss / find-edges / median
//! neighbourhood passes). The CPU path remains the oracle for raster parity.

use photonic_core::raster::filter::{
    box_blur, emboss, find_edges, high_pass, median, unsharp_mask,
};
use photonic_core::RasterImage;

use super::ops::Image;

/// Stable EffectId strings this bridge can evaluate on CPU.
pub const BRIDGED_IDS: &[&str] = &[
    "blur.box",
    "sharpen.unsharp_raster", // distinct from IR Sharpen; uses raster unsharp
    "stylize.emboss",
    "stylize.find_edges",
    "filter.high_pass",
    "filter.median",
];

pub fn is_bridged(id: &str) -> bool {
    BRIDGED_IDS.contains(&id)
}

/// Apply a bridged raster kernel to a video working buffer. `radius`/`amount`
/// come from resolved params (defaults match the raster filter neutrals).
/// Returns `None` if `id` is not bridged.
pub fn apply(id: &str, input: &Image, radius: f32, amount: f32) -> Option<Image> {
    if !is_bridged(id) {
        return None;
    }
    let mut raster = image_to_raster(input);
    match id {
        "blur.box" => {
            let r = radius.max(0.0).round() as u32;
            box_blur(&mut raster, r.max(1), None);
        }
        "sharpen.unsharp_raster" => {
            unsharp_mask(&mut raster, radius.max(0.0), amount.max(0.0), 0, None);
        }
        "stylize.emboss" => emboss(&mut raster, None),
        "stylize.find_edges" => find_edges(&mut raster, None),
        "filter.high_pass" => high_pass(&mut raster, radius.max(0.1), None),
        "filter.median" => {
            let r = radius.max(0.0).round() as u32;
            median(&mut raster, r.max(1), None);
        }
        _ => return None,
    }
    Some(raster_to_image(&raster))
}

/// Linear premultiplied f32 → sRGB8 straight (for raster kernels).
fn image_to_raster(img: &Image) -> RasterImage {
    let mut out = RasterImage::new(img.width, img.height);
    for (i, p) in img.pixels.iter().enumerate() {
        let a = p[3].clamp(0.0, 1.0);
        let (r, g, b) = if a > 1e-6 {
            (p[0] / a, p[1] / a, p[2] / a)
        } else {
            (0.0, 0.0, 0.0)
        };
        let o = i * 4;
        out.pixels[o] = linear_to_srgb_u8(r);
        out.pixels[o + 1] = linear_to_srgb_u8(g);
        out.pixels[o + 2] = linear_to_srgb_u8(b);
        out.pixels[o + 3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    out
}

/// sRGB8 straight → linear premultiplied f32.
fn raster_to_image(img: &RasterImage) -> Image {
    let mut out = Image::new(img.width, img.height);
    for (i, p) in out.pixels.iter_mut().enumerate() {
        let o = i * 4;
        let a = img.pixels[o + 3] as f32 / 255.0;
        let r = srgb_u8_to_linear(img.pixels[o]) * a;
        let g = srgb_u8_to_linear(img.pixels[o + 1]) * a;
        let b = srgb_u8_to_linear(img.pixels[o + 2]) * a;
        *p = [r, g, b, a];
    }
    out
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
        assert!(!is_bridged("blur.gaussian"));
    }

    #[test]
    fn emboss_changes_a_solid_edge_image() {
        // Build a left-black / right-white step — emboss reacts to edges.
        let mut img = Image::new(8, 8);
        for y in 0..8 {
            for x in 0..8 {
                let v = if x < 4 { 0.0 } else { 1.0 };
                img.set(x, y, [v, v, v, 1.0]);
            }
        }
        let out = apply("stylize.emboss", &img, 0.0, 0.0).expect("bridged");
        assert_ne!(out.pixels, img.pixels, "emboss must alter an edge image");
    }

    #[test]
    fn box_blur_softens_a_checker() {
        let mut img = Image::new(4, 1);
        img.set(0, 0, [1.0, 1.0, 1.0, 1.0]);
        img.set(1, 0, [0.0, 0.0, 0.0, 1.0]);
        img.set(2, 0, [1.0, 1.0, 1.0, 1.0]);
        img.set(3, 0, [0.0, 0.0, 0.0, 1.0]);
        let out = apply("blur.box", &img, 1.0, 0.0).expect("bridged");
        // Neighbours pull the pure white pixel down.
        assert!(out.pixel(0, 0)[0] < 1.0);
    }

    #[test]
    fn solid_roundtrip_is_near_identity() {
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
        let r = image_to_raster(&img);
        let back = raster_to_image(&r);
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
