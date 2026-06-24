use crate::{node::SceneNode, path::PathData, transform::Transform};
use kurbo::{Affine, BezPath, PathEl};

/// Apply a transform to a node's existing transform (concatenates).
pub fn apply_transform(node: &mut SceneNode, t: &Transform) {
    node.transform = node.transform.then(t);
}

/// Set a node's transform to an absolute value.
pub fn set_transform(node: &mut SceneNode, t: Transform) {
    node.transform = t;
}

/// Translate a node by (dx, dy).
pub fn translate(node: &mut SceneNode, dx: f64, dy: f64) {
    apply_transform(node, &Transform::translate(dx, dy));
}

/// Rotate a node around a point (cx, cy) by angle_degrees.
pub fn rotate(node: &mut SceneNode, angle_degrees: f64, cx: f64, cy: f64) {
    let radians = angle_degrees.to_radians();
    apply_transform(node, &Transform::rotate_around(radians, cx, cy));
}

/// Scale a node around a point (cx, cy) by (sx, sy).
pub fn scale(node: &mut SceneNode, sx: f64, sy: f64, cx: f64, cy: f64) {
    apply_transform(node, &Transform::scale_around(sx, sy, cx, cy));
}

/// Reflect a node horizontally around x = cx.
pub fn reflect_horizontal(node: &mut SceneNode, cx: f64) {
    apply_transform(node, &Transform::scale_around(-1.0, 1.0, cx, 0.0));
}

/// Reflect a node vertically around y = cy.
pub fn reflect_vertical(node: &mut SceneNode, cy: f64) {
    apply_transform(node, &Transform::scale_around(1.0, -1.0, 0.0, cy));
}

/// Shear a node around (cx, cy) by the given shear factors.
/// `shx` moves x proportionally to y; `shy` moves y proportionally to x.
pub fn shear(node: &mut SceneNode, shx: f64, shy: f64, cx: f64, cy: f64) {
    apply_transform(node, &Transform::shear_around(shx, shy, cx, cy));
}

/// Bake an affine transform into the path's coordinates, returning a new
/// `PathData` with all points mapped by `affine`.  The transform of the node
/// itself is NOT modified — only the path points are remapped.
pub fn apply_affine_to_path(path: &PathData, affine: Affine) -> PathData {
    let mut result = BezPath::new();
    for el in path.to_bez_path().elements() {
        let mapped = match *el {
            PathEl::MoveTo(p) => PathEl::MoveTo(affine * p),
            PathEl::LineTo(p) => PathEl::LineTo(affine * p),
            PathEl::CurveTo(c1, c2, p) => PathEl::CurveTo(affine * c1, affine * c2, affine * p),
            PathEl::QuadTo(c, p) => PathEl::QuadTo(affine * c, affine * p),
            PathEl::ClosePath => PathEl::ClosePath,
        };
        match mapped {
            PathEl::MoveTo(p) => result.move_to(p),
            PathEl::LineTo(p) => result.line_to(p),
            PathEl::CurveTo(c1, c2, p) => result.curve_to(c1, c2, p),
            PathEl::QuadTo(c, p) => result.quad_to(c, p),
            PathEl::ClosePath => result.close_path(),
        }
    }
    PathData::from_bez_path(&result)
}
