//! A realistic Photoshop-style editing session exercised end-to-end on the CPU
//! engine — placement, adjustments, filters, brush, selection-confined edits,
//! retouching, liquify, blend-mode compositing, and PNG round-trip. Serves as
//! both regression coverage and evidence of breadth.

use photonic_core::layer::BlendMode;
use photonic_core::raster::{
    adjust, advanced, blend, brush, filter, geometry, image::RasterImage, mask::Mask, repair, warp,
};

/// A 64×64 synthetic "photograph": a diagonal color gradient.
fn synth_photo() -> RasterImage {
    let mut img = RasterImage::new(64, 64);
    for y in 0..64u32 {
        for x in 0..64u32 {
            img.set_pixel(
                x,
                y,
                [(x * 4) as u8, (y * 4) as u8, ((x + y) * 2) as u8, 255],
            );
        }
    }
    img
}

fn differs(a: &RasterImage, b: &RasterImage) -> bool {
    a.pixels != b.pixels
}

#[test]
fn full_editing_session_runs_and_changes_pixels() {
    let original = synth_photo();
    let mut img = original.clone();

    // ── Tonal adjustments ────────────────────────────────────────────────────
    adjust::levels(&mut img, 0.05, 0.95, 1.1, 0.0, 1.0, None);
    adjust::curves(
        &mut img,
        &[(0.0, 0.0), (0.5, 0.6), (1.0, 1.0)],
        &[],
        &[],
        &[],
        None,
    );
    adjust::hue_saturation(&mut img, 20.0, 0.15, 0.0, None);
    adjust::vibrance(&mut img, 0.3, None);
    assert_eq!((img.width, img.height), (64, 64));
    assert!(differs(&img, &original));
    // alpha preserved
    assert!(img.pixels.chunks_exact(4).all(|p| p[3] == 255));

    // ── Selection-confined edit ──────────────────────────────────────────────
    let sel = Mask::ellipse(64, 64, 16.0, 16.0, 32.0, 32.0);
    let before = img.clone();
    adjust::desaturate(&mut img, Some(&sel));
    // center changed, far corner (outside ellipse) unchanged
    assert_ne!(img.pixel(32, 32), before.pixel(32, 32));
    assert_eq!(img.pixel(0, 0), before.pixel(0, 0));

    // ── Filters ──────────────────────────────────────────────────────────────
    let pre_blur = img.clone();
    filter::gaussian_blur(&mut img, 2.0, None);
    assert!(differs(&img, &pre_blur));
    filter::unsharp_mask(&mut img, 2.0, 0.8, 2, None);
    advanced::surface_blur(&mut img, 3, 0.1, None);
    advanced::clarity(&mut img, 0.4, None);
    advanced::vignette(&mut img, -0.5, 0.6, None);
    assert_eq!((img.width, img.height), (64, 64));

    // ── Painting ─────────────────────────────────────────────────────────────
    let mut b = brush::Brush::new(5.0, [255, 0, 0, 255]);
    b.hardness = 0.9;
    let pre_paint = img.clone();
    brush::stroke(&mut img, &[(10.0, 10.0), (50.0, 50.0)], &b, None);
    assert!(differs(&img, &pre_paint));
    // a point on the stroke became reddish
    assert!(img.pixel(30, 30)[0] > 120);

    // ── Retouch ──────────────────────────────────────────────────────────────
    repair::spot_healing(&mut img, 40.0, 20.0, 4.0);
    repair::dust_and_scratches(&mut img, 1, 16, None);

    // ── Liquify ──────────────────────────────────────────────────────────────
    warp::liquify_twirl(&mut img, 32.0, 32.0, 20.0, 30.0, None);
    warp::ripple(&mut img, 3.0, 16.0, None);
    assert_eq!((img.width, img.height), (64, 64));

    // ── Geometry ─────────────────────────────────────────────────────────────
    let cropped = geometry::crop(&img, 8, 8, 48, 48);
    assert_eq!((cropped.width, cropped.height), (48, 48));
    let scaled = geometry::resize(&cropped, 96, 96, geometry::Resample::Lanczos3);
    assert_eq!((scaled.width, scaled.height), (96, 96));
    let rotated = geometry::rotate90(&scaled);
    assert_eq!((rotated.width, rotated.height), (96, 96));

    // ── PNG round-trip ───────────────────────────────────────────────────────
    let png = rotated.to_png();
    assert!(png.len() > 8 && &png[1..4] == b"PNG");
    let decoded = RasterImage::from_encoded(&png).unwrap();
    assert_eq!(decoded, rotated);
}

#[test]
fn tiny_and_degenerate_images_never_panic() {
    // 1×1 and 0-length buffers must not panic across the operation surface
    // (an adversarial probe flagged adjustments on an empty image).
    let mut one = RasterImage::filled(1, 1, [128, 64, 200, 255]);
    adjust::levels(&mut one, 0.1, 0.9, 1.2, 0.0, 1.0, None);
    adjust::curves(&mut one, &[(0.0, 0.0), (1.0, 1.0)], &[], &[], &[], None);
    adjust::auto_contrast(&mut one, None);
    adjust::auto_levels(&mut one, None);
    adjust::hue_saturation(&mut one, 30.0, 0.5, 0.0, None);
    filter::gaussian_blur(&mut one, 3.0, None);
    filter::median(&mut one, 2, None);
    advanced::surface_blur(&mut one, 4, 0.2, None);
    warp::liquify_twirl(&mut one, 0.5, 0.5, 5.0, 30.0, None);
    repair::spot_healing(&mut one, 0.5, 0.5, 2.0);
    assert_eq!((one.width, one.height), (1, 1));

    // A zero-area buffer built directly (len 0) — ops must tolerate it.
    if let Ok(mut empty) = RasterImage::from_rgba(0, 0, vec![]) {
        adjust::invert(&mut empty, None);
        adjust::auto_contrast(&mut empty, None);
        filter::gaussian_blur(&mut empty, 2.0, None);
    }
}

#[test]
fn two_layer_blend_composite() {
    // Bottom: solid blue. Top: solid red at 50% via Multiply through a half mask.
    let mut base = RasterImage::filled(16, 16, [0, 0, 255, 255]);
    let top = RasterImage::filled(16, 16, [255, 0, 0, 255]);
    let mut mask = Mask::empty(16, 16);
    for y in 0..16 {
        for x in 0..8 {
            mask.set(x, y, 255); // left half only
        }
    }
    blend::composite(&mut base, &top, 0, 0, 1.0, BlendMode::Multiply, Some(&mask));
    // left half: red×blue multiply → black-ish; right half: unchanged blue
    assert!(base.pixel(0, 0)[2] < 30); // blue channel killed by multiply with red
    assert_eq!(base.pixel(12, 0), [0, 0, 255, 255]);
}

#[test]
fn content_aware_fill_removes_a_blemish() {
    // Flat gray with a bright square; select the square and fill from surroundings.
    let mut img = RasterImage::filled(32, 32, [128, 128, 128, 255]);
    for y in 12..20 {
        for x in 12..20 {
            img.set_pixel(x, y, [255, 255, 0, 255]);
        }
    }
    let sel = Mask::rect(32, 32, 11, 11, 10, 10);
    repair::content_aware_fill(&mut img, &sel);
    // the formerly-yellow center should now be close to the surrounding gray
    // (in particular blue must rise from 0 toward 128, killing the yellow).
    let c = img.pixel(15, 15);
    assert!((c[0] as i32 - 128).abs() < 50, "r={}", c[0]);
    assert!((c[1] as i32 - 128).abs() < 50, "g={}", c[1]);
    assert!((c[2] as i32 - 128).abs() < 50, "b={} (was 0/yellow)", c[2]);
}
