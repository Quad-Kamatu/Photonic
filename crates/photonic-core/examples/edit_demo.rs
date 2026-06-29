//! A runnable demonstration of Photonic's raster engine — a real, multi-step
//! "Photoshop session" on the CPU that writes before/after PNGs to disk.
//!
//! Run with:
//!   cargo run -p photonic-core --example edit_demo [OUTPUT_DIR]
//!
//! It synthesizes a test photograph, then applies a chain of adjustments,
//! filters, painting, retouching, liquify, and a two-layer blend, saving a PNG
//! after each major stage so the results can be inspected visually.

use photonic_core::layer::BlendMode;
use photonic_core::raster::{
    adjust, advanced, blend, brush, filter, geometry, image::RasterImage, mask::Mask, repair, warp,
};
use std::path::PathBuf;

fn save(img: &RasterImage, dir: &str, name: &str) {
    let mut path = PathBuf::from(dir);
    path.push(name);
    std::fs::write(&path, img.to_png()).expect("write png");
    println!("  wrote {}", path.display());
}

/// A 256×256 synthetic scene: sky gradient + a "sun" disc + ground.
fn synth_scene() -> RasterImage {
    let w = 256u32;
    let h = 256u32;
    let mut img = RasterImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let ty = y as f32 / h as f32;
            // sky: blue → warm near horizon
            let mut r = (90.0 + 120.0 * ty) as i32;
            let mut g = (140.0 + 60.0 * ty) as i32;
            let mut b = (220.0 - 120.0 * ty) as i32;
            // ground in the lower third
            if ty > 0.66 {
                r = 70 + (x % 32) as i32;
                g = 110;
                b = 60;
            }
            // sun disc
            let dx = x as f32 - 190.0;
            let dy = y as f32 - 70.0;
            if (dx * dx + dy * dy).sqrt() < 28.0 {
                r = 255;
                g = 240;
                b = 180;
            }
            img.set_pixel(
                x,
                y,
                [
                    r.clamp(0, 255) as u8,
                    g.clamp(0, 255) as u8,
                    b.clamp(0, 255) as u8,
                    255,
                ],
            );
        }
    }
    img
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());
    std::fs::create_dir_all(&dir).ok();
    println!("Photonic raster edit demo → {}", dir);

    let original = synth_scene();
    save(&original, &dir, "00_original.png");

    // 1. Tonal grade: lift contrast, warm it, boost vibrance.
    let mut img = original.clone();
    adjust::brightness_contrast(&mut img, 0.05, 0.2, None);
    adjust::curves(
        &mut img,
        &[(0.0, 0.02), (0.5, 0.55), (1.0, 0.98)],
        &[],
        &[],
        &[],
        None,
    );
    adjust::color_balance(
        &mut img,
        [0.1, 0.0, -0.1],
        [0.05, 0.0, -0.05],
        [0.1, 0.05, -0.1],
        true,
        None,
    );
    adjust::vibrance(&mut img, 0.4, None);
    save(&img, &dir, "01_graded.png");

    // 2. Selective edit: desaturate everything except an elliptical subject.
    let mut subject = Mask::ellipse(256, 256, 150.0, 30.0, 80.0, 80.0);
    subject.feather(6.0);
    subject.invert(); // select the surroundings
    adjust::hue_saturation(&mut img, 0.0, -0.7, 0.0, Some(&subject));
    save(&img, &dir, "02_selective_desaturate.png");

    // 3. Filters: sharpen the subject, add a vignette and gentle bloom.
    advanced::smart_sharpen(&mut img, 1.2, 2.0, 3, None);
    advanced::vignette(&mut img, -0.5, 0.6, None);
    let mut bloom = img.clone();
    filter::gaussian_blur(&mut bloom, 6.0, None);
    blend::composite(&mut img, &bloom, 0, 0, 0.25, BlendMode::Screen, None);
    save(&img, &dir, "03_filtered.png");

    // 4. Paint: a soft brush highlight + a hard pencil signature line.
    let mut soft = brush::Brush::new(18.0, [255, 250, 230, 120]);
    soft.hardness = 0.2;
    brush::stroke(
        &mut img,
        &[(60.0, 200.0), (90.0, 190.0), (120.0, 205.0)],
        &soft,
        None,
    );
    let pencil = brush::Brush::pencil(2.0, [20, 20, 30, 255]);
    brush::stroke(
        &mut img,
        &[(20.0, 240.0), (40.0, 235.0), (60.0, 240.0), (80.0, 236.0)],
        &pencil,
        None,
    );
    save(&img, &dir, "04_painted.png");

    // 5. Retouch: heal a spot, content-aware remove a rectangle.
    repair::spot_healing(&mut img, 130.0, 150.0, 8.0);
    let mut hole = Mask::rect(256, 256, 30, 30, 24, 24);
    hole.feather(2.0);
    repair::content_aware_fill(&mut img, &hole);
    save(&img, &dir, "05_retouched.png");

    // 6. Liquify: twirl the sun, ripple the surface.
    warp::liquify_twirl(&mut img, 190.0, 70.0, 40.0, 60.0, None);
    warp::ripple(&mut img, 2.5, 40.0, None);
    save(&img, &dir, "06_liquified.png");

    // 7. Non-destructive-style adjustment applied as data (AdjustmentSpec).
    use photonic_core::AdjustmentSpec;
    let mut final_img = img.clone();
    AdjustmentSpec::PhotoFilter {
        color: [1.0, 0.7, 0.4],
        density: 0.2,
        preserve_luminosity: true,
    }
    .apply(&mut final_img, None);
    save(&final_img, &dir, "07_final.png");

    // 8. Geometry: produce a cropped, upscaled thumbnail.
    let thumb = geometry::resize(
        &geometry::crop(&final_img, 110, 10, 120, 120),
        240,
        240,
        geometry::Resample::Lanczos3,
    );
    save(&thumb, &dir, "08_thumb.png");

    println!("Done. 9 PNGs written to {}", dir);
}
