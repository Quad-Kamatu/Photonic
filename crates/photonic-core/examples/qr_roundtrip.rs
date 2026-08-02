//! Build a QR and write it as a black-on-white SVG for round-trip scan testing:
//!   cargo run -p photonic-core --example qr_roundtrip -- <out.svg> <data> <shape> [ecc]
//! Then rasterize (rsvg-convert) and decode (zbarimg) to prove it scans.

use photonic_core::ops::qr::{build_qr, QrEcc, QrModuleShape, QrOptions};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let out = a.get(1).cloned().unwrap_or_else(|| "/tmp/qr.svg".into());
    let data = a
        .get(2)
        .cloned()
        .unwrap_or_else(|| "https://kamatu.studio".into());
    let shape = a
        .get(3)
        .map(|s| QrModuleShape::parse(s).expect("shape"))
        .unwrap_or(QrModuleShape::Square);
    let ecc = a
        .get(4)
        .and_then(|s| QrEcc::parse(s))
        .unwrap_or(QrEcc::Medium);

    let opts = QrOptions {
        data,
        ecc,
        shape,
        radius: 0.45,
        size: 300.0,
        quiet_zone: 4,
    };
    let art = build_qr(&opts).expect("build qr");
    let d = art.modules.to_bez_path().to_svg();
    let w = art.size;
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{w}\" viewBox=\"0 0 {w} {w}\">\
         <rect width=\"{w}\" height=\"{w}\" fill=\"#ffffff\"/>\
         <path d=\"{d}\" fill=\"#000000\" fill-rule=\"nonzero\"/></svg>"
    );
    std::fs::write(&out, svg).expect("write svg");
    println!(
        "wrote {out}  ({}×{} modules, {:.2} u/module)",
        art.matrix_size, art.matrix_size, art.module_size
    );
}
