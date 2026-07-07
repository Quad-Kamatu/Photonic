//! One-off generator for the P1 golden-output fixture corpus
//! (`03-render-color-pipeline.md` §2.6, `tests/golden/`).
//!
//! Not run by CI or by `cargo test` — a checked-in developer tool in the same
//! spirit as `tools/gen-mcp-docs.py` (11-testing-phasing.md §2's convention:
//! "a checked-in script CI *consumes the output of* rather than *runs*").
//! Regenerate a fixture's `project.photon` (e.g. after a deliberate schema
//! change) with:
//!
//! ```sh
//! cargo run -p photonic-render --example gen_p1_golden_fixtures
//! ```
//!
//! This only (re)writes `project.photon` files. It never touches
//! `expected/reference.png` — that is the golden test harness's job
//! (`tests/golden_vector_equivalence.rs`, `PHOTONIC_BLESS_GOLDEN=1`).

use photonic_core::effects::{ColorOverlay, LayerEffect, StrokeEffect};
use photonic_core::node::{GroupNode, PathNode, TextNode};
use photonic_core::ops::boolean::BooleanOp;
use photonic_core::style::{LineCap, LineJoin};
use photonic_core::{
    save_photon, BlendMode, Color, Fill, Gradient, GradientStop, PathData, RasterImage,
    RasterNode, SceneNode, SceneNodeKind, Stroke, Transform,
};
use photonic_core::Document;

fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    for (name, doc) in fixtures() {
        let dir = root.join(&name);
        std::fs::create_dir_all(dir.join("expected")).expect("create case dir");
        let json = save_photon(&doc, None).expect("serialize fixture");
        std::fs::write(dir.join("project.photon"), json).expect("write project.photon");
        println!("wrote {}/project.photon", name);
    }
    println!(
        "\nNo expected/reference.png written (harness blesses those). Run:\n  \
         PHOTONIC_BLESS_GOLDEN=1 cargo test -p photonic-render --test golden_vector_equivalence -- --test-threads=1\n\
         then review `git diff --stat tests/golden/` before committing."
    );
}

fn fixtures() -> Vec<(&'static str, Document)> {
    vec![
        ("paths_fills_basic", paths_fills_basic()),
        ("strokes_basic", strokes_basic()),
        ("gradient_linear", gradient_linear()),
        ("gradient_radial", gradient_radial()),
        ("blend_separable", blend_separable()),
        ("blend_nonseparable", blend_nonseparable()),
        ("text_basic", text_basic()),
        ("raster_placement", raster_placement()),
        ("effect_stack_color_overlay_stroke", effect_stack()),
        ("boolean_groups", boolean_groups()),
    ]
}

/// Two solid-fill paths (rect + ellipse), the baseline "just tessellate and
/// fill" case every other fixture builds on.
fn paths_fills_basic() -> Document {
    let mut doc = Document::new("paths-fills-basic", 120.0, 120.0);
    let rect = SceneNode::new(
        "rect",
        doc.active_layer_id.unwrap(),
        SceneNodeKind::Path(
            PathNode::new(PathData::rect(10.0, 10.0, 40.0, 40.0))
                .with_fill(Fill::solid(Color::new(0.85, 0.15, 0.15, 1.0))),
        ),
    );
    doc.add_node(rect, None);
    let ellipse = SceneNode::new(
        "ellipse",
        doc.active_layer_id.unwrap(),
        SceneNodeKind::Path(
            PathNode::new(PathData::ellipse(80.0, 45.0, 25.0, 20.0))
                .with_fill(Fill::solid(Color::new(0.15, 0.35, 0.85, 0.7))),
        ),
    );
    doc.add_node(ellipse, None);
    doc
}

/// Stroke-only paths (no fill) exercising cap/join/dash.
fn strokes_basic() -> Document {
    let mut doc = Document::new("strokes-basic", 120.0, 120.0);
    let mut round_stroke = Stroke::solid(Color::new(0.1, 0.6, 0.3, 1.0), 6.0);
    round_stroke.line_cap = LineCap::Round;
    round_stroke.line_join = LineJoin::Round;
    let line_a = SceneNode::new(
        "line-round",
        doc.active_layer_id.unwrap(),
        SceneNodeKind::Path(
            PathNode::new(PathData::line(10.0, 20.0, 100.0, 20.0))
                .with_fill(Fill::solid(Color::TRANSPARENT))
                .with_stroke(round_stroke),
        ),
    );
    doc.add_node(line_a, None);

    let mut dashed_stroke = Stroke::solid(Color::new(0.6, 0.2, 0.7, 1.0), 4.0);
    dashed_stroke.line_cap = LineCap::Square;
    dashed_stroke.dash_array = vec![8.0, 4.0];
    let line_b = SceneNode::new(
        "line-dashed",
        doc.active_layer_id.unwrap(),
        SceneNodeKind::Path(
            PathNode::new(PathData::line(10.0, 60.0, 100.0, 100.0))
                .with_fill(Fill::solid(Color::TRANSPARENT))
                .with_stroke(dashed_stroke),
        ),
    );
    doc.add_node(line_b, None);
    doc
}

fn gradient_linear() -> Document {
    let mut doc = Document::new("gradient-linear", 120.0, 120.0);
    let stops = vec![
        GradientStop::new(0.0, Color::new(1.0, 0.9, 0.2, 1.0)),
        GradientStop::new(0.5, Color::new(0.9, 0.3, 0.1, 1.0)),
        GradientStop::new(1.0, Color::new(0.4, 0.0, 0.5, 1.0)),
    ];
    let rect = SceneNode::new(
        "gradient-rect",
        doc.active_layer_id.unwrap(),
        SceneNodeKind::Path(
            PathNode::new(PathData::rect(10.0, 10.0, 100.0, 100.0))
                .with_fill(Fill::gradient(Gradient::linear(10.0, 10.0, 110.0, 110.0, stops))),
        ),
    );
    doc.add_node(rect, None);
    doc
}

fn gradient_radial() -> Document {
    let mut doc = Document::new("gradient-radial", 120.0, 120.0);
    let stops = vec![
        GradientStop::new(0.0, Color::new(0.95, 0.95, 1.0, 1.0)),
        GradientStop::new(0.6, Color::new(0.2, 0.5, 0.9, 1.0)),
        GradientStop::new(1.0, Color::new(0.0, 0.05, 0.3, 1.0)),
    ];
    let ellipse = SceneNode::new(
        "gradient-ellipse",
        doc.active_layer_id.unwrap(),
        SceneNodeKind::Path(
            PathNode::new(PathData::ellipse(60.0, 60.0, 45.0, 45.0))
                .with_fill(Fill::gradient(Gradient::radial(60.0, 60.0, 45.0, stops))),
        ),
    );
    doc.add_node(ellipse, None);
    doc
}

/// Backdrop rect + four small rects, one per fixed-function `SEPARABLE_BLEND_MODES`
/// entry (`pipeline.rs` `[Multiply, Screen, Darken, Lighten]`) — the P1 refactor
/// must leave this pixel-identical since these modes never touch `COMPOSITE_SHADER`.
fn blend_separable() -> Document {
    blend_grid(
        "blend-separable",
        &[
            BlendMode::Multiply,
            BlendMode::Screen,
            BlendMode::Darken,
            BlendMode::Lighten,
        ],
    )
}

/// Same grid, but with non-separable / backdrop-read modes that `COMPOSITE_SHADER`
/// wiring (03 §2.4) will change the isolation path for — this case carries a
/// `tolerance_db.txt` (PSNR ≥ 45 dB) rather than requiring byte-exact match.
fn blend_nonseparable() -> Document {
    blend_grid(
        "blend-nonseparable",
        &[
            BlendMode::Hue,
            BlendMode::Saturation,
            BlendMode::Color,
            BlendMode::Luminosity,
        ],
    )
}

fn blend_grid(name: &str, modes: &[BlendMode; 4]) -> Document {
    let mut doc = Document::new(name, 120.0, 120.0);
    let backdrop = SceneNode::new(
        "backdrop",
        doc.active_layer_id.unwrap(),
        SceneNodeKind::Path(
            PathNode::new(PathData::rect(0.0, 0.0, 120.0, 120.0))
                .with_fill(Fill::solid(Color::new(0.8, 0.4, 0.2, 1.0))),
        ),
    );
    doc.add_node(backdrop, None);
    let positions = [(10.0, 10.0), (65.0, 10.0), (10.0, 65.0), (65.0, 65.0)];
    for (i, mode) in modes.iter().enumerate() {
        let (x, y) = positions[i];
        let mut node = SceneNode::new(
            format!("swatch-{i}"),
            doc.active_layer_id.unwrap(),
            SceneNodeKind::Path(
                PathNode::new(PathData::rect(x, y, 45.0, 45.0))
                    .with_fill(Fill::solid(Color::new(0.3, 0.6, 0.9, 1.0))),
            ),
        );
        node.blend_mode = *mode;
        doc.add_node(node, None);
    }
    doc
}

fn text_basic() -> Document {
    let mut doc = Document::new("text-basic", 160.0, 60.0);
    let mut text = TextNode::new("Photonic");
    text.font_size = 24.0;
    text.fill = Fill::solid(Color::new(0.1, 0.1, 0.1, 1.0));
    let mut node = SceneNode::new(
        "title",
        doc.active_layer_id.unwrap(),
        SceneNodeKind::Text(text),
    );
    node.transform = Transform::translate(10.0, 15.0);
    doc.add_node(node, None);
    doc
}

fn raster_placement() -> Document {
    let mut doc = Document::new("raster-placement", 120.0, 120.0);
    let image = RasterImage::filled(32, 32, [255, 128, 0, 255]);
    let mut node = SceneNode::new(
        "raster",
        doc.active_layer_id.unwrap(),
        SceneNodeKind::Raster(RasterNode::new(image)),
    );
    // Place + scale 2x so the placement transform is exercised, not just 1:1.
    node.transform = Transform::scale(2.0, 2.0).then(&Transform::translate(20.0, 20.0));
    doc.add_node(node, None);
    doc
}

/// A path styled entirely via the Layer-Styles effect stack (`node.effects`)
/// rather than its own `fill`/`stroke` fields — exercises `ColorOverlay` +
/// `StrokeEffect` composited in list order.
fn effect_stack() -> Document {
    let mut doc = Document::new("effect-stack", 120.0, 120.0);
    let mut node = SceneNode::new(
        "styled-shape",
        doc.active_layer_id.unwrap(),
        SceneNodeKind::Path(
            PathNode::new(PathData::ellipse(60.0, 60.0, 40.0, 40.0))
                .with_fill(Fill::solid(Color::TRANSPARENT)),
        ),
    );
    node.effects = vec![
        LayerEffect::ColorOverlay(ColorOverlay {
            color: Color::new(0.2, 0.7, 0.4, 1.0),
            ..ColorOverlay::default()
        }),
        LayerEffect::Stroke(StrokeEffect {
            width: 5.0,
            fill: Fill::solid(Color::new(0.05, 0.2, 0.1, 1.0)),
            ..StrokeEffect::default()
        }),
    ];
    doc.add_node(node, None);
    doc
}

/// Live (non-destructive) boolean union (#25) of two overlapping rects.
/// Operand paths are added, folded into a `GroupNode` with `live_boolean`
/// set, then removed from the layer's top-level `node_ids` (mirrors what the
/// `GroupNodes` command does) so only the resolved shape renders — matching
/// what a real group-and-boolean user action produces.
fn boolean_groups() -> Document {
    let mut doc = Document::new("boolean-groups", 120.0, 120.0);
    let lid = doc.active_layer_id.unwrap();
    let a = SceneNode::new(
        "operand-a",
        lid,
        SceneNodeKind::Path(
            PathNode::new(PathData::rect(20.0, 20.0, 50.0, 50.0))
                .with_fill(Fill::solid(Color::new(0.9, 0.5, 0.1, 1.0))),
        ),
    );
    let b = SceneNode::new(
        "operand-b",
        lid,
        SceneNodeKind::Path(
            PathNode::new(PathData::rect(50.0, 50.0, 50.0, 50.0))
                .with_fill(Fill::solid(Color::new(0.9, 0.5, 0.1, 1.0))),
        ),
    );
    let (aid, bid) = (a.id, b.id);
    doc.add_node(a, Some(lid));
    doc.add_node(b, Some(lid));

    let mut group = GroupNode::new();
    group.children = vec![aid, bid];
    group.live_boolean = Some(BooleanOp::Union);
    let gnode = SceneNode::new("union", lid, SceneNodeKind::Group(group));
    doc.add_node(gnode, Some(lid));

    // Operands are now represented only via the group's children list — drop
    // their top-level layer entries so they don't ALSO render standalone.
    if let Some(layer) = doc.layers.get_mut(&lid) {
        layer.node_ids.retain(|id| *id != aid && *id != bid);
    }
    doc
}
