//! Native QR-code generation.
//!
//! Encodes text/URL to a QR module matrix ([`qrcodegen`]), then emits it as a
//! single compound vector path (all dark modules as sub-paths of one
//! [`PathData`]), styled per the caller's options. The result is a
//! resolution-independent QR that lives in the document like any other vector
//! art — recolour it with any fill (solid or gradient), scale it, export it to
//! SVG/PDF, all losslessly.
//!
//! Coordinates: the artwork's top-left is (0, 0) and its side is `opts.size`
//! (quiet-zone inclusive). The caller positions it via the node transform.

use crate::path::PathData;
use kurbo::{BezPath, Circle, Point, RoundedRect, Shape};
use qrcodegen::{QrCode, QrCodeEcc};

/// Error-correction level — redundancy vs. density. Higher tolerates more
/// damage/occlusion (needed for centre logos) at the cost of a denser code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QrEcc {
    Low,
    Medium,
    Quartile,
    High,
}

impl QrEcc {
    fn to_qrcodegen(self) -> QrCodeEcc {
        match self {
            QrEcc::Low => QrCodeEcc::Low,
            QrEcc::Medium => QrCodeEcc::Medium,
            QrEcc::Quartile => QrCodeEcc::Quartile,
            QrEcc::High => QrCodeEcc::High,
        }
    }

    /// Parse `"l"|"low"|"m"|"medium"|"q"|"quartile"|"h"|"high"` (case-insensitive).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "l" | "low" => Some(QrEcc::Low),
            "m" | "medium" => Some(QrEcc::Medium),
            "q" | "quartile" => Some(QrEcc::Quartile),
            "h" | "high" => Some(QrEcc::High),
            _ => None,
        }
    }
}

/// How each dark module is shaped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QrModuleShape {
    /// Plain squares (classic).
    Square,
    /// Rounded square per module (isolated); `radius` fraction 0..=0.5.
    Rounded,
    /// Circle/dot per module.
    Dot,
    /// Neighbour-aware "blob" rounding: a corner rounds only when both of its
    /// adjacent edges are exposed, so shared edges stay square and abut
    /// seamlessly — isolated modules become dots/rounded-squares while runs
    /// become rounded-ended bars. `radius` fraction 0..=0.5.
    Connected,
}

impl QrModuleShape {
    /// Parse `"square"|"rounded"|"dot"|"connected"` (case-insensitive).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "square" => Some(QrModuleShape::Square),
            "rounded" => Some(QrModuleShape::Rounded),
            "dot" | "dots" | "circle" => Some(QrModuleShape::Dot),
            "connected" | "blob" | "liquid" => Some(QrModuleShape::Connected),
            _ => None,
        }
    }
}

/// Options controlling QR generation.
#[derive(Debug, Clone)]
pub struct QrOptions {
    /// Content to encode (URL or text).
    pub data: String,
    pub ecc: QrEcc,
    pub shape: QrModuleShape,
    /// Corner radius as a fraction of one module, 0..=0.5 (0.5 = fully round).
    /// Used by `Rounded` and `Connected`.
    pub radius: f64,
    /// Total artwork side in document units, INCLUDING the quiet-zone margin.
    pub size: f64,
    /// Quiet-zone margin in modules (ISO minimum is 4; keep ≥ 2 to stay scannable).
    pub quiet_zone: u32,
}

impl Default for QrOptions {
    fn default() -> Self {
        Self {
            data: String::new(),
            ecc: QrEcc::Medium,
            shape: QrModuleShape::Square,
            radius: 0.0,
            size: 200.0,
            quiet_zone: 4,
        }
    }
}

/// Generated QR geometry.
pub struct QrArtwork {
    /// One compound path of every dark module, top-left at (0, 0).
    pub modules: PathData,
    /// Artwork side (== `opts.size`), quiet-zone inclusive.
    pub size: f64,
    /// Modules per side (excluding the quiet zone).
    pub matrix_size: usize,
    /// One module's side in document units.
    pub module_size: f64,
}

/// `k` for a cubic-Bézier quarter-circle (≈ 0.5523).
const KAPPA: f64 = 0.552_284_749_830_793_4;

/// Build QR vector geometry. Returns `Err` if `data` is empty or too large to
/// encode at the chosen ECC level.
pub fn build_qr(opts: &QrOptions) -> Result<QrArtwork, String> {
    if opts.data.trim().is_empty() {
        return Err("QR data is empty".into());
    }
    if !(opts.size > 0.0) {
        return Err("QR size must be > 0".into());
    }
    let qr = QrCode::encode_text(&opts.data, opts.ecc.to_qrcodegen())
        .map_err(|e| format!("QR encode failed (data too long for this error-correction level?): {e}"))?;
    let n = qr.size().max(1) as usize; // modules per side
    let quiet = opts.quiet_zone as usize;
    let total = n + 2 * quiet;
    let m = opts.size / total as f64; // one module, in doc units
    let radius = opts.radius.clamp(0.0, 0.5);

    let dark = |x: i32, y: i32| -> bool {
        x >= 0 && y >= 0 && (x as usize) < n && (y as usize) < n && qr.get_module(x, y)
    };
    // The three finder patterns (corner "eyes") occupy 7×7 blocks at the TL, TR
    // and BL corners. They must stay recognizable or scanners can't lock on.
    let ni = n as i32;
    let in_finder = |x: i32, y: i32| -> bool {
        (x < 7 && y < 7) || (x >= ni - 7 && y < 7) || (x < 7 && y >= ni - 7)
    };

    let mut bez = BezPath::new();
    for y in 0..n as i32 {
        for x in 0..n as i32 {
            if !qr.get_module(x, y) {
                continue;
            }
            let px = (x as usize + quiet) as f64 * m;
            let py = (y as usize + quiet) as f64 * m;
            // For any stylised data shape, render finder modules with the
            // neighbour-aware Connected style so each eye reads as a solid rounded
            // square instead of a grid of dots / notched cells (which won't scan).
            let shape = if opts.shape != QrModuleShape::Square && in_finder(x, y) {
                QrModuleShape::Connected
            } else {
                opts.shape
            };
            match shape {
                QrModuleShape::Square => append_square(&mut bez, px, py, m),
                QrModuleShape::Rounded => {
                    let r = (radius * m).min(m * 0.5);
                    if r <= 1e-9 {
                        append_square(&mut bez, px, py, m);
                    } else {
                        bez.extend(RoundedRect::new(px, py, px + m, py + m, r).path_elements(0.05));
                    }
                }
                QrModuleShape::Dot => {
                    bez.extend(
                        Circle::new(Point::new(px + m * 0.5, py + m * 0.5), m * 0.5)
                            .path_elements(0.05),
                    );
                }
                QrModuleShape::Connected => {
                    append_connected(&mut bez, px, py, m, radius * m, x, y, &dark)
                }
            }
        }
    }

    Ok(QrArtwork {
        modules: PathData::from_bez_path(&bez),
        size: opts.size,
        matrix_size: n,
        module_size: m,
    })
}

fn append_square(bez: &mut BezPath, px: f64, py: f64, m: f64) {
    bez.move_to((px, py));
    bez.line_to((px + m, py));
    bez.line_to((px + m, py + m));
    bez.line_to((px, py + m));
    bez.close_path();
}

/// A quarter-circle cubic from `from` to `to`, bulging toward `corner`.
fn corner_arc(bez: &mut BezPath, from: Point, corner: Point, to: Point) {
    let c1 = Point::new(from.x + (corner.x - from.x) * KAPPA, from.y + (corner.y - from.y) * KAPPA);
    let c2 = Point::new(to.x + (corner.x - to.x) * KAPPA, to.y + (corner.y - to.y) * KAPPA);
    bez.curve_to(c1, c2, to);
}

/// Emit one module as a rectangle whose corners round only where BOTH adjacent
/// edges are exposed (no orthogonal neighbour). Shared edges stay square, so
/// adjacent modules abut seamlessly and connected runs read as one rounded blob.
#[allow(clippy::too_many_arguments)]
fn append_connected(
    bez: &mut BezPath,
    px: f64,
    py: f64,
    m: f64,
    r_in: f64,
    x: i32,
    y: i32,
    dark: &impl Fn(i32, i32) -> bool,
) {
    let r = r_in.clamp(0.0, m * 0.5);
    if r <= 1e-9 {
        append_square(bez, px, py, m);
        return;
    }
    let (up, down, left, right) = (dark(x, y - 1), dark(x, y + 1), dark(x - 1, y), dark(x + 1, y));
    // Round a corner only when both edges meeting there are exposed.
    let tl = !up && !left;
    let tr = !up && !right;
    let br = !down && !right;
    let bl = !down && !left;

    let (x0, y0, x1, y1) = (px, py, px + m, py + m);
    // Walk clockwise starting on the top edge just after the top-left corner.
    bez.move_to((x0 + if tl { r } else { 0.0 }, y0));
    // → top edge to top-right
    bez.line_to((x1 - if tr { r } else { 0.0 }, y0));
    if tr {
        corner_arc(bez, Point::new(x1 - r, y0), Point::new(x1, y0), Point::new(x1, y0 + r));
    }
    // ↓ right edge to bottom-right
    bez.line_to((x1, y1 - if br { r } else { 0.0 }));
    if br {
        corner_arc(bez, Point::new(x1, y1 - r), Point::new(x1, y1), Point::new(x1 - r, y1));
    }
    // ← bottom edge to bottom-left
    bez.line_to((x0 + if bl { r } else { 0.0 }, y1));
    if bl {
        corner_arc(bez, Point::new(x0 + r, y1), Point::new(x0, y1), Point::new(x0, y1 - r));
    }
    // ↑ left edge to top-left
    bez.line_to((x0, y0 + if tl { r } else { 0.0 }));
    if tl {
        corner_arc(bez, Point::new(x0, y0 + r), Point::new(x0, y0), Point::new(x0 + r, y0));
    }
    bez.close_path();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_data_errors() {
        let opts = QrOptions { data: "  ".into(), ..Default::default() };
        assert!(build_qr(&opts).is_err());
    }

    #[test]
    fn generates_nonempty_geometry_for_all_shapes() {
        for shape in [
            QrModuleShape::Square,
            QrModuleShape::Rounded,
            QrModuleShape::Dot,
            QrModuleShape::Connected,
        ] {
            let opts = QrOptions {
                data: "https://kamatu.studio".into(),
                shape,
                radius: 0.4,
                size: 210.0,
                ..Default::default()
            };
            let art = build_qr(&opts).expect("build");
            assert!(art.matrix_size >= 21, "smallest QR is 21×21");
            let bez = art.modules.to_bez_path();
            let n_moves = bez.elements().iter().filter(|e| matches!(e, kurbo::PathEl::MoveTo(_))).count();
            assert!(n_moves > 50, "{shape:?}: expected many module sub-paths, got {n_moves}");
            // Geometry must sit inside the artwork square.
            let bb = art.modules.bounding_box().expect("bbox");
            assert!(bb.x0 >= -0.01 && bb.y0 >= -0.01 && bb.x1 <= art.size + 0.01 && bb.y1 <= art.size + 0.01);
        }
    }

    #[test]
    fn qr_group_parents_children_and_deletes_cleanly() {
        use crate::document::Document;
        use crate::history::{Command, CommandHistory};
        use crate::node::{GroupNode, PathNode, SceneNode, SceneNodeKind};

        let mut doc = Document::new("t", 4000.0, 4000.0);
        let mut history = CommandHistory::new(200);
        let layer = doc.active_layer_id.unwrap();

        let art = build_qr(&QrOptions {
            data: "https://kamatu.studio".into(),
            size: 290.0,
            quiet_zone: 3,
            shape: QrModuleShape::Connected,
            ecc: QrEcc::High,
            radius: 0.4,
        })
        .unwrap();

        // Mirror the create_qr_code handler's command sequence.
        let bg = SceneNode::new(
            "QR Background",
            layer,
            SceneNodeKind::Path(PathNode::new(PathData::rect(0.0, 0.0, art.size, art.size))),
        );
        let modules =
            SceneNode::new("QR Modules", layer, SceneNodeKind::Path(PathNode::new(art.modules)));
        let (bg_id, mod_id) = (bg.id, modules.id);
        let child_ids = vec![bg_id, mod_id];
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
        let group_id = group.id;
        history.execute_discrete(
            Command::Batch(vec![
                Command::AddNode { node: bg, layer_id: Some(layer) },
                Command::AddNode { node: modules, layer_id: Some(layer) },
                Command::GroupNodes {
                    group,
                    layer_id: layer,
                    insert_index: usize::MAX,
                    children: child_ids,
                },
            ]),
            &mut doc,
        );

        // (1,2) Group is populated and the modules are NOT loose siblings.
        match &doc.nodes.get(&group_id).expect("group exists").kind {
            SceneNodeKind::Group(g) => {
                assert_eq!(g.children, vec![bg_id, mod_id], "group must list its children")
            }
            _ => panic!("not a group"),
        }
        let lids = &doc.layers[&layer].node_ids;
        assert_eq!(lids.last(), Some(&group_id), "group must be at the TOP of z-order");
        assert!(
            !lids.contains(&bg_id) && !lids.contains(&mod_id),
            "children must be in the group, not loose siblings"
        );

        // (4) Deleting the group must remove ALL QR geometry — zero orphans. This
        // is the RemoveSubtree that delete_nodes now issues for a group (a bare
        // RemoveNode would leave the children orphaned in doc.nodes).
        let subtree: Vec<SceneNode> = [group_id, bg_id, mod_id]
            .iter()
            .filter_map(|id| doc.nodes.get(id).cloned())
            .collect();
        history.execute_discrete(
            Command::RemoveSubtree { layer_id: layer, roots: vec![group_id], nodes: subtree },
            &mut doc,
        );
        assert!(!doc.nodes.contains_key(&group_id), "group removed");
        assert!(!doc.nodes.contains_key(&mod_id), "QR Modules orphaned after group delete");
        assert!(!doc.nodes.contains_key(&bg_id), "QR Background orphaned after group delete");
    }

    #[test]
    fn quiet_zone_insets_the_matrix() {
        let opts = QrOptions { data: "x".into(), size: 200.0, quiet_zone: 4, ..Default::default() };
        let art = build_qr(&opts).unwrap();
        let bb = art.modules.bounding_box().unwrap();
        // With a 4-module quiet zone, no dark module touches the outer edge.
        assert!(bb.x0 >= art.module_size * 3.5, "left quiet zone missing: {bb:?}");
    }
}
