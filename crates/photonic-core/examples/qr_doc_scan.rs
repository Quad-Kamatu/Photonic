//! Build a document containing a QR **group** (background + module compound path,
//! exactly as create_qr_code assembles it) and export it — proving the group
//! renders through the real export path. Writes a CMYK PDF to argv[1].
//!   cargo run -p photonic-core --example qr_doc_scan -- <out.pdf> <data> <shape>

use photonic_core::color::Color;
use photonic_core::document::{ColorMode, Document};
use photonic_core::export::{export_pdf, PdfExportOptions};
use photonic_core::history::{Command, CommandHistory};
use photonic_core::node::{GroupNode, PathNode, SceneNode, SceneNodeKind};
use photonic_core::ops::qr::{build_qr, QrEcc, QrModuleShape, QrOptions};
use photonic_core::path::PathData;
use photonic_core::style::Fill;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let out = a
        .get(1)
        .cloned()
        .unwrap_or_else(|| "/tmp/qr_doc.pdf".into());
    let data = a
        .get(2)
        .cloned()
        .unwrap_or_else(|| "https://kamatu.studio".into());
    let shape = QrModuleShape::parse(a.get(3).map(|s| s.as_str()).unwrap_or("connected")).unwrap();

    let size = 300.0;
    let mut doc = Document::new("qr", size, size);
    doc.color_mode = ColorMode::Cmyk;
    let mut history = CommandHistory::new(200);
    let layer = doc.active_layer_id.unwrap();

    let art = build_qr(&QrOptions {
        data: data.clone(),
        size,
        quiet_zone: 4,
        shape,
        ecc: QrEcc::High,
        radius: 0.45,
    })
    .unwrap();

    // White background + black modules, grouped — mirrors create_qr_code.
    let mut bg_pn = PathNode::new(PathData::rect(0.0, 0.0, size, size));
    bg_pn.fill = Fill::solid(Color::WHITE);
    let bg = SceneNode::new("QR Background", layer, SceneNodeKind::Path(bg_pn));
    let mut mod_pn = PathNode::new(art.modules);
    mod_pn.fill = Fill::solid(Color::BLACK);
    let modules = SceneNode::new("QR Modules", layer, SceneNodeKind::Path(mod_pn));
    let child_ids = vec![bg.id, modules.id];
    let group = SceneNode::new(
        "QR Code",
        layer,
        SceneNodeKind::Group(GroupNode {
            children: child_ids.clone(),
            clip_children: false,
            clip_node_id: None,
            blend_spine_id: None,
            live_boolean: None,
        }),
    );
    history.execute_discrete(
        Command::Batch(vec![
            Command::AddNode {
                node: bg,
                layer_id: Some(layer),
            },
            Command::AddNode {
                node: modules,
                layer_id: Some(layer),
            },
            Command::GroupNodes {
                group,
                layer_id: layer,
                insert_index: usize::MAX,
                children: child_ids,
            },
        ]),
        &mut doc,
    );

    let bytes = export_pdf(
        &doc,
        &PdfExportOptions {
            color_mode: ColorMode::Cmyk,
            ..Default::default()
        },
    );
    std::fs::write(&out, &bytes).unwrap();
    println!("wrote {out} ({} bytes, {shape:?})", bytes.len());
}
