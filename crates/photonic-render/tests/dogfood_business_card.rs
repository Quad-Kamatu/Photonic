//! Dogfood: lay out a real 3.5×2 in business card with Photonic's own document
//! model, outline its text, and export a print-ready PDF/X-1a file natively.
//!
//! The heavy end-to-end run writes an actual PDF and is gated behind `DOGFOOD=1`
//! (it needs system fonts and writes a file); the DPI page-size check always runs.
//!
//!   DOGFOOD=1 DOGFOOD_OUT=/path/card.pdf \
//!     cargo test -p photonic-render --test dogfood_business_card -- --nocapture

use photonic_core::color::Color;
use photonic_core::document::{ColorMode, Document, PrintPreset};
use photonic_core::export::{export_pdf, PdfExportOptions};
use photonic_core::node::{PathNode, SceneNode, SceneNodeKind, TextNode};
use photonic_core::path::PathData;
use photonic_core::style::Fill;
use photonic_core::transform::Transform;
use photonic_render::outline_document_text;

fn hex(s: &str) -> Color {
    Color::from_hex(s).unwrap_or(Color::WHITE)
}

/// Add a filled rectangle to the active layer.
fn add_rect(doc: &mut Document, x: f64, y: f64, w: f64, h: f64, fill: Color) {
    let layer = doc.active_layer_id.unwrap();
    let node = SceneNode::new(
        "rect",
        layer,
        SceneNodeKind::Path(PathNode::new(PathData::rect(x, y, w, h)).with_fill(Fill::solid(fill))),
    );
    doc.add_node(node, None);
}

/// Add a text run positioned by a translate transform (top-left-ish anchor).
fn add_text(doc: &mut Document, content: &str, x: f64, y: f64, size: f64, fill: Color) {
    let layer = doc.active_layer_id.unwrap();
    let mut t = TextNode::new(content);
    t.font_family = "DejaVu Sans".to_string();
    t.font_size = size;
    t.fill = Fill::solid(fill);
    let mut node = SceneNode::new("text", layer, SceneNodeKind::Text(t));
    node.transform = Transform::new(1.0, 0.0, 0.0, 1.0, x, y);
    doc.add_node(node, None);
}

/// Build the Kamatu Studio business card (3.5×2 in @ 72 dpi ⇒ 252×144 pt),
/// 3 mm bleed, CMYK. Brand tokens are placeholders — swap for the real palette.
fn build_card() -> (Document, PdfExportOptions) {
    let navy = hex("#0B1F3A");
    let gold = hex("#C8A24B");
    let white = hex("#F5F3EC");

    let mut doc = Document::from_preset(PrintPreset::BUSINESS_CARD_US, 72.0);
    doc.bleed_mm = 3.0;
    doc.color_mode = ColorMode::Cmyk;

    // Gold accent bar + a small mark block.
    add_rect(&mut doc, 18.0, 30.0, 60.0, 4.0, gold);
    add_rect(&mut doc, 18.0, 40.0, 14.0, 14.0, gold);

    // Studio name, tagline, contact.
    add_text(&mut doc, "KAMATU STUDIO", 18.0, 78.0, 15.0, gold);
    add_text(&mut doc, "Design & Brand Systems", 18.0, 96.0, 8.0, white);
    add_text(&mut doc, "hello@kamatu.studio  ·  kamatu.studio", 18.0, 120.0, 7.0, white);

    let opts = PdfExportOptions {
        background: Some(navy),
        outline_text: true,
        marks: true,
        color_mode: ColorMode::Cmyk,
        icc_profile: None, // bundled FOGRA39
    };
    (doc, opts)
}

/// Physical-size model: a 3.5×2 in card authored at 300 dpi must still export at
/// exactly 252×144 pt (3.5×2 in), proving px→pt scaling by 72/dpi.
#[test]
fn business_card_page_size_is_physical_at_any_dpi() {
    for dpi in [72.0, 150.0, 300.0] {
        let doc = Document::from_preset(PrintPreset::BUSINESS_CARD_US, dpi);
        let opts = PdfExportOptions::default(); // rgb, no bleed/marks
        let boxes = photonic_core::export::compute_page_boxes(&doc, &opts);
        let approx = |a: f32, b: f32| (a - b).abs() < 0.05;
        assert!(
            approx(boxes.media[2], 252.0) && approx(boxes.media[3], 144.0),
            "at {dpi} dpi the media box must be 252×144 pt, got {:?}",
            boxes.media
        );
    }
}

#[test]
fn dogfood_export_business_card_pdfx() {
    if std::env::var("DOGFOOD").as_deref() != Ok("1") {
        return;
    }
    let (doc, opts) = build_card();

    let mut fs = glyphon::FontSystem::new();
    let outlined = outline_document_text(&doc, &mut fs);
    let bytes = export_pdf(&outlined, &opts);

    assert!(bytes.starts_with(b"%PDF-1.3"), "must be a PDF 1.3 file");
    assert!(bytes.len() > 1000, "PDF should be non-trivial");

    let out = std::env::var("DOGFOOD_OUT")
        .unwrap_or_else(|_| "/tmp/kamatu-business-card.pdf".to_string());
    std::fs::write(&out, &bytes).unwrap();
    println!("DOGFOOD wrote {} ({} bytes)", out, bytes.len());
}
