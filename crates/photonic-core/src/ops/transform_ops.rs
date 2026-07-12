use crate::{node::SceneNode, path::PathData, transform::Transform};
use kurbo::{Affine, BezPath, PathEl};

/// Apply a transform to a node's existing transform (concatenates).
pub fn apply_transform(node: &mut SceneNode, t: &Transform) {
    node.transform = node.transform.then(t);
    node.transform_user_space_gradients(t);
}

/// Set a node's transform to an absolute value.
pub fn set_transform(node: &mut SceneNode, t: Transform) {
    node.transform = t;
    node.transform_user_space_gradients(&t);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        color::Color,
        layer::LayerId,
        node::{PathNode, SceneNodeKind},
        style::{Fill, FillKind, Gradient, GradientStop, Stroke},
    };

    fn gradient() -> Gradient {
        Gradient::linear(
            10.0,
            20.0,
            30.0,
            20.0,
            vec![
                GradientStop::new(0.0, Color::BLACK),
                GradientStop::new(1.0, Color::WHITE),
            ],
        )
    }

    #[test]
    fn transforms_fill_and_stroke_user_space_gradients() {
        let layer = LayerId::new_v4();
        let mut stroke = Stroke::solid(Color::BLACK, 1.0);
        stroke.paint = Some(FillKind::Gradient(gradient()));
        let mut node = SceneNode::new(
            "gradient path",
            layer,
            SceneNodeKind::Path(
                PathNode::new(PathData::rect(10.0, 20.0, 20.0, 20.0))
                    .with_fill(Fill::gradient(gradient()))
                    .with_stroke(stroke),
            ),
        );

        translate(&mut node, 0.0, 700.0);
        let SceneNodeKind::Path(path) = node.kind else {
            unreachable!()
        };
        let FillKind::Gradient(fill) = path.fill.kind else {
            unreachable!()
        };
        let FillKind::Gradient(stroke) = path.stroke.paint.expect("gradient stroke") else {
            unreachable!()
        };
        assert_eq!(fill.coords, vec![10.0, 720.0, 30.0, 720.0]);
        assert_eq!(stroke.coords, fill.coords);
    }

    #[test]
    fn full_transform_maps_gradient_coordinates_with_geometry() {
        let layer = LayerId::new_v4();
        let mut node = SceneNode::new(
            "gradient path",
            layer,
            SceneNodeKind::Path(
                PathNode::new(PathData::rect(10.0, 20.0, 20.0, 20.0))
                    .with_fill(Fill::gradient(gradient())),
            ),
        );

        apply_transform(&mut node, &Transform::scale(2.0, 3.0));
        let SceneNodeKind::Path(path) = &node.kind else {
            unreachable!()
        };
        let FillKind::Gradient(fill) = &path.fill.kind else {
            unreachable!()
        };
        assert_eq!(fill.coords, vec![20.0, 60.0, 60.0, 60.0]);

        set_transform(&mut node, Transform::translate(5.0, 7.0));
        let SceneNodeKind::Path(path) = &node.kind else {
            unreachable!()
        };
        let FillKind::Gradient(fill) = &path.fill.kind else {
            unreachable!()
        };
        assert_eq!(fill.coords, vec![25.0, 67.0, 65.0, 67.0]);
    }
}
