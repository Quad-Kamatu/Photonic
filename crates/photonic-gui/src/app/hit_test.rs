//! Hit-testing, node bounds/colour probes, and shape-drawing math helpers
//! extracted from app::mod. Pure functions over the document/nodes.
#![allow(clippy::too_many_arguments)]
use super::*;

pub(crate) fn gui_apply_affine_to_path(path: &PathData, affine: kurbo::Affine) -> PathData {
    use kurbo::PathEl;
    let mut result = BezPath::new();
    for el in path.to_bez_path().elements() {
        let t = match *el {
            PathEl::MoveTo(p) => PathEl::MoveTo(affine * p),
            PathEl::LineTo(p) => PathEl::LineTo(affine * p),
            PathEl::CurveTo(c1, c2, p) => PathEl::CurveTo(affine * c1, affine * c2, affine * p),
            PathEl::QuadTo(c, p) => PathEl::QuadTo(affine * c, affine * p),
            PathEl::ClosePath => PathEl::ClosePath,
        };
        result.push(t);
    }
    PathData::from_bez_path(&result)
}

/// Return the topmost node (reverse draw order) whose bounding box contains (cx, cy).
/// Tests whether the canvas-space point `(cx, cy)` lands on a path node's
/// actual geometry: inside its filled area (non-zero winding) or within a small
/// tolerance of its outline (so thin / open / stroke-only paths stay clickable).
///
/// Returns `None` for non-path nodes so the caller can fall back to a plain
/// bounding-box test (text, images and groups have no fillable outline here).
pub(crate) fn path_geometry_hit(node: &SceneNode, cx: f64, cy: f64) -> Option<bool> {
    use kurbo::{ParamCurveNearest, Shape};
    let SceneNodeKind::Path(pn) = &node.kind else {
        return None;
    };
    let bez = pn.path_data.to_bez_path();
    if bez.elements().is_empty() {
        return Some(false);
    }
    // Transform into canvas space so the click point and the tolerance share
    // units (the click is already in canvas coordinates).
    let canvas_bez = node.transform.to_kurbo() * bez;
    let pt = kurbo::Point::new(cx, cy);

    // Filled interior: only a *filled* shape should be selectable through its
    // body — an unfilled outline is clickable only on its edge, matching the
    // "click through transparent areas" behaviour requested in #3.
    if pn.fill.enabled && canvas_bez.contains(pt) {
        return Some(true);
    }

    // Outline proximity: clickable within half the stroke width plus a few
    // canvas units of slack so hairline strokes and open paths stay grabbable.
    let tol = (pn.stroke.width * 0.5) + 3.0;
    let tol_sq = tol * tol;
    let on_edge = canvas_bez
        .segments()
        .any(|seg| seg.nearest(pt, 0.1).distance_sq <= tol_sq);
    Some(on_edge)
}

/// Topmost path under `(cx, cy)` for the Direct Selection tool. Unlike
/// [`hit_test`], the interior of a path is clickable even when it has no fill —
/// in point-edit context the user is plainly pointing at the shape they want to
/// grab, so an empty (stroke-only) object should still select on a body click.
pub(crate) fn direct_select_hit(
    doc: &Document,
    cx: f64,
    cy: f64,
    renderer: &mut PhotonicRenderer,
) -> Option<NodeId> {
    use kurbo::{ParamCurveNearest, Shape};
    for node in doc.nodes_in_draw_order().into_iter().rev() {
        if node.locked {
            continue;
        }
        let Some((x0, y0, x1, y1)) = text_aware_canvas_bounds(node, renderer) else {
            continue;
        };
        if cx < x0 || cx > x1 || cy < y0 || cy > y1 {
            continue;
        }
        let SceneNodeKind::Path(pn) = &node.kind else {
            // Non-path nodes keep the bounding-box hit (matches hit_test).
            return Some(node.id);
        };
        let bez = pn.path_data.to_bez_path();
        if bez.elements().is_empty() {
            continue;
        }
        let canvas_bez = node.transform.to_kurbo() * bez;
        let pt = kurbo::Point::new(cx, cy);
        // Interior — selectable regardless of fill state.
        if canvas_bez.contains(pt) {
            return Some(node.id);
        }
        // Or near the outline.
        let tol = (pn.stroke.width * 0.5) + 4.0;
        let tol_sq = tol * tol;
        if canvas_bez
            .segments()
            .any(|seg| seg.nearest(pt, 0.1).distance_sq <= tol_sq)
        {
            return Some(node.id);
        }
    }
    None
}

pub(crate) fn hit_test(doc: &Document, cx: f64, cy: f64, renderer: &mut PhotonicRenderer) -> Option<NodeId> {
    for node in doc.nodes_in_draw_order().into_iter().rev() {
        if node.locked {
            continue;
        }
        // Cheap reject: the click must at least fall inside the bounding box.
        let Some((x0, y0, x1, y1)) = text_aware_canvas_bounds(node, renderer) else {
            continue;
        };
        if cx < x0 || cx > x1 || cy < y0 || cy > y1 {
            continue;
        }
        // Refine path nodes to their real geometry so clicks fall through the
        // transparent parts of a non-rectangular shape onto whatever is below;
        // other node kinds keep the bounding-box hit.
        match path_geometry_hit(node, cx, cy) {
            Some(true) | None => return Some(node.id),
            Some(false) => continue,
        }
    }
    None
}

/// Horizontal center of a path node's bounding box in local space.
pub(crate) fn gui_path_center_x(node: &SceneNode) -> f32 {
    if let SceneNodeKind::Path(p) = &node.kind {
        if let Some(bb) = p.path_data.bounding_box() {
            return ((bb.x0 + bb.x1) / 2.0) as f32;
        }
    }
    0.0
}

/// Vertical center of a path node's bounding box in local space.
pub(crate) fn gui_path_center_y(node: &SceneNode) -> f32 {
    if let SceneNodeKind::Path(p) = &node.kind {
        if let Some(bb) = p.path_data.bounding_box() {
            return ((bb.y0 + bb.y1) / 2.0) as f32;
        }
    }
    0.0
}

/// Extract the solid fill color from a node's path fill, or None if absent.
pub(crate) fn gui_solid_fill_color(node: &SceneNode) -> Option<photonic_core::color::Color> {
    use photonic_core::style::FillKind;
    if let SceneNodeKind::Path(pn) = &node.kind {
        if pn.fill.enabled {
            if let FillKind::Solid(c) = pn.fill.kind {
                return Some(c);
            }
        }
    }
    None
}

/// Euclidean distance between two RGBA colors in [0,1] space.
pub(crate) fn gui_color_dist(a: photonic_core::color::Color, b: photonic_core::color::Color) -> f32 {
    let dr = a.r - b.r;
    let dg = a.g - b.g;
    let db = a.b - b.b;
    let da = a.a - b.a;
    (dr * dr + dg * dg + db * db + da * da).sqrt()
}

/// Snap the line endpoint `(ex, ey)` from start `(sx, sy)` to the nearest 45° angle.
/// The distance from start to the snapped end is preserved.
pub(crate) fn snap_line_to_45(sx: f64, sy: f64, ex: f64, ey: f64) -> (f64, f64) {
    let dx = ex - sx;
    let dy = ey - sy;
    let len = dx.hypot(dy);
    if len < 1e-6 {
        return (ex, ey);
    }
    let angle = dy.atan2(dx);
    // Round to nearest multiple of 45° (π/4 radians).
    let snapped = (angle / (std::f64::consts::PI / 4.0)).round() * (std::f64::consts::PI / 4.0);
    (sx + len * snapped.cos(), sy + len * snapped.sin())
}

/// Lock a movement delta `(dx, dy)` to the nearest of 8 directions
/// (N/S/E/W + the four diagonals), preserving magnitude. Used for
/// Shift-constrained moves of selected objects.
pub(crate) fn axis_lock_8(dx: f64, dy: f64) -> (f64, f64) {
    let len = dx.hypot(dy);
    if len < 1e-9 {
        return (0.0, 0.0);
    }
    let step = std::f64::consts::FRAC_PI_4; // 45°
    let snapped = (dy.atan2(dx) / step).round() * step;
    (len * snapped.cos(), len * snapped.sin())
}

/// Treat `(sx, sy)` as the center of a shape and mirror the drag end through it,
/// returning the two opposite corners `((ax, ay), (bx, by))`. Used when Alt is
/// held while drawing so the shape grows symmetrically from the start point.
pub(crate) fn shape_corners_from_center(sx: f64, sy: f64, ex: f64, ey: f64) -> ((f64, f64), (f64, f64)) {
    let dx = ex - sx;
    let dy = ey - sy;
    ((sx - dx, sy - dy), (sx + dx, sy + dy))
}

/// Constrain a drag rectangle to a 1:1 square while keeping `(sx, sy)` as the
/// anchor corner. The larger of the two deltas wins, so the endpoint moves
/// along the drag direction's diagonal (square / circle / proportional shape).
pub(crate) fn constrain_to_square(sx: f64, sy: f64, ex: f64, ey: f64) -> (f64, f64) {
    let dx = ex - sx;
    let dy = ey - sy;
    let m = dx.abs().max(dy.abs());
    (sx + m.copysign(dx), sy + m.copysign(dy))
}

/// Extract the solid fill RGBA from a node (used by the Magic Wand tool).
pub(crate) fn magic_wand_solid_fill(node: &SceneNode) -> Option<Color> {
    use photonic_core::style::FillKind;
    if let SceneNodeKind::Path(pn) = &node.kind {
        if pn.fill.enabled {
            if let FillKind::Solid(c) = pn.fill.kind {
                return Some(c);
            }
        }
    }
    None
}

/// Euclidean distance between two RGBA colors in [0, 1] space (Magic Wand helper).
pub(crate) fn magic_wand_color_dist(a: Color, b: Color) -> f32 {
    let dr = a.r - b.r;
    let dg = a.g - b.g;
    let db = a.b - b.b;
    let da = a.a - b.a;
    (dr * dr + dg * dg + db * db + da * da).sqrt()
}

/// Shared logic for ConvertToSmooth / ConvertToCorner panel actions.
pub(crate) fn convert_anchor_points_gui(
    smooth: bool,
    node_ids: Vec<photonic_core::node::NodeId>,
    doc: &mut Document,
    history: &mut photonic_core::history::CommandHistory,
    doc_modified: &mut bool,
) {
    let mut cmds: Vec<Command> = Vec::new();
    for nid in node_ids {
        if let Some(node) = doc.nodes.get(&nid).cloned() {
            if let SceneNodeKind::Path(ref pn) = node.kind {
                let new_path = if smooth {
                    pn.path_data.convert_to_smooth()
                } else {
                    pn.path_data.convert_to_corner()
                };
                let mut new_node = node.clone();
                if let SceneNodeKind::Path(ref mut np) = new_node.kind {
                    np.path_data = new_path;
                }
                cmds.push(Command::UpdateNode {
                    old: node,
                    new: new_node,
                });
            }
        }
    }
    if !cmds.is_empty() {
        let cmd = if cmds.len() == 1 {
            cmds.remove(0)
        } else {
            Command::Batch(cmds)
        };
        history.execute(cmd, doc);
        *doc_modified = true;
    }
}

/// Compute the world-space AABB of a node as (x0, y0, x1, y1), or None if the node
/// has no computable bounding box (e.g. groups without children).
pub(crate) fn node_world_aabb_opt(node: &SceneNode) -> Option<(f64, f64, f64, f64)> {
    use photonic_core::node::SceneNodeKind;
    let local_rect = match &node.kind {
        SceneNodeKind::Path(pn) => pn.path_data.bounding_box()?,
        SceneNodeKind::Text(_) => return None,
        SceneNodeKind::Group(_) => return None,
        // raster nodes have no vector geometry
        SceneNodeKind::Raster(_) => return None,
    };
    let tf = node.transform.to_kurbo();
    let corners = [
        kurbo::Point::new(local_rect.x0, local_rect.y0),
        kurbo::Point::new(local_rect.x1, local_rect.y0),
        kurbo::Point::new(local_rect.x1, local_rect.y1),
        kurbo::Point::new(local_rect.x0, local_rect.y1),
    ];
    let world: Vec<kurbo::Point> = corners.iter().map(|p| tf * *p).collect();
    let wx0 = world.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
    let wy0 = world.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
    let wx1 = world.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
    let wy1 = world.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
    Some((wx0, wy0, wx1, wy1))
}

/// Ray-casting point-in-polygon test (Jordan curve theorem).
pub(crate) fn lasso_point_in_polygon(px: f64, py: f64, poly: &[[f64; 2]]) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let xi = poly[i][0];
        let yi = poly[i][1];
        let xj = poly[j][0];
        let yj = poly[j][1];
        if ((yi > py) != (yj > py)) && (px < (xj - xi) * (py - yi) / (yj - yi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}
