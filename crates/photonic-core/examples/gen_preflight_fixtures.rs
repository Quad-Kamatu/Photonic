//! Generate the core-producible PDF fixtures for the preflight self-test.
//!
//! Writes into the directory given as argv[1]. The font/encrypted fixtures are
//! produced by `scripts/gen-preflight-fixtures.sh` (which calls this first, then
//! Ghostscript + qpdf). Committed fixtures live in `scripts/preflight-fixtures/`.
//!
//!   cargo run -p photonic-core --example gen_preflight_fixtures -- <out-dir>

use photonic_core::color::Color;
use photonic_core::document::{ColorMode, Document};
use photonic_core::export::{export_pdf, PdfExportOptions};
use photonic_core::node::{PathNode, SceneNode, SceneNodeKind};
use photonic_core::path::PathData;
use photonic_core::style::Fill;

/// A minimal document: one solid rectangle on the default layer.
fn red_rect_doc() -> Document {
    let mut doc = Document::new("fixture", 200.0, 120.0);
    let layer = doc.active_layer_id.unwrap();
    let mut r = PathNode::new(PathData::rect(10.0, 10.0, 180.0, 100.0));
    r.fill = Fill::solid(Color::new(0.85, 0.12, 0.16, 1.0));
    doc.add_node(SceneNode::new("r", layer, SceneNodeKind::Path(r)), None);
    doc
}

fn main() {
    let dir = std::env::args()
        .nth(1)
        .expect("usage: gen_preflight_fixtures <out-dir>");
    let write = |name: &str, bytes: &[u8]| {
        let p = format!("{dir}/{name}");
        std::fs::write(&p, bytes).unwrap_or_else(|e| panic!("write {p}: {e}"));
        println!("wrote {p} ({} bytes)", bytes.len());
    };

    // GOOD — CMYK export ⇒ PDF/X-1a: PDF 1.3, DeviceCMYK, embedded GTS_PDFX
    // OutputIntent, no RGB, no transparency, no fonts. Must PASS all invariants.
    let good = export_pdf(
        &red_rect_doc(),
        &PdfExportOptions {
            color_mode: ColorMode::Cmyk,
            ..Default::default()
        },
    );
    write("good_x1a.pdf", &good);

    // BAD (RGB) — default export ⇒ `rg` operators, no OutputIntent, PDF 1.7.
    // Exercises the RGB, OutputIntent, and version invariants.
    let bad_rgb = export_pdf(&red_rect_doc(), &PdfExportOptions::default());
    write("bad_rgb.pdf", &bad_rgb);

    // BAD (transparency) — CMYK export with a 50%-opacity layer ⇒ an ExtGState
    // with /ca 0.5 (live transparency). Otherwise a valid X-1a file, so this
    // isolates the transparency invariant.
    let mut doc = red_rect_doc();
    let layer = doc.active_layer_id.unwrap();
    doc.layers.get_mut(&layer).unwrap().opacity = 0.5;
    let bad_tr = export_pdf(
        &doc,
        &PdfExportOptions {
            color_mode: ColorMode::Cmyk,
            ..Default::default()
        },
    );
    write("bad_transparency.pdf", &bad_tr);
}
