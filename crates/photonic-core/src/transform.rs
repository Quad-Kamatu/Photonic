use kurbo::Affine;
use serde::{Deserialize, Serialize};

/// A 2D affine transform stored as a 6-element array [a, b, c, d, e, f].
/// Equivalent to the matrix:
///   | a  c  e |
///   | b  d  f |
///   | 0  0  1 |
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    /// [a, b, c, d, e, f] — affine matrix coefficients
    pub matrix: [f64; 6],
}

impl Transform {
    pub const IDENTITY: Self = Self {
        matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
    };

    pub fn new(a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) -> Self {
        Self {
            matrix: [a, b, c, d, e, f],
        }
    }

    pub fn translate(tx: f64, ty: f64) -> Self {
        Self {
            matrix: [1.0, 0.0, 0.0, 1.0, tx, ty],
        }
    }

    pub fn scale(sx: f64, sy: f64) -> Self {
        Self {
            matrix: [sx, 0.0, 0.0, sy, 0.0, 0.0],
        }
    }

    pub fn scale_around(sx: f64, sy: f64, cx: f64, cy: f64) -> Self {
        // Translate to origin, scale, translate back
        Self::translate(cx, cy)
            .then(&Self::scale(sx, sy))
            .then(&Self::translate(-cx, -cy))
    }

    /// Rotation by `angle_radians` counter-clockwise
    pub fn rotate(angle_radians: f64) -> Self {
        let cos = angle_radians.cos();
        let sin = angle_radians.sin();
        Self {
            matrix: [cos, sin, -sin, cos, 0.0, 0.0],
        }
    }

    pub fn rotate_around(angle_radians: f64, cx: f64, cy: f64) -> Self {
        Self::translate(cx, cy)
            .then(&Self::rotate(angle_radians))
            .then(&Self::translate(-cx, -cy))
    }

    /// Shear by `shx` (x-axis shear: x' = x + shx*y) and `shy` (y-axis shear: y' = shy*x + y).
    pub fn shear(shx: f64, shy: f64) -> Self {
        // Matrix: | 1   shx  0 |
        //         | shy  1   0 |
        //         | 0    0   1 |
        // Stored as [a, b, c, d, e, f] = [1, shy, shx, 1, 0, 0]
        Self {
            matrix: [1.0, shy, shx, 1.0, 0.0, 0.0],
        }
    }

    /// Shear around a point (cx, cy).
    pub fn shear_around(shx: f64, shy: f64, cx: f64, cy: f64) -> Self {
        Self::translate(cx, cy)
            .then(&Self::shear(shx, shy))
            .then(&Self::translate(-cx, -cy))
    }

    /// Compose: apply `self` first, then `other`
    pub fn then(&self, other: &Self) -> Self {
        let [a1, b1, c1, d1, e1, f1] = self.matrix;
        let [a2, b2, c2, d2, e2, f2] = other.matrix;
        Self {
            matrix: [
                a1 * a2 + c1 * b2,
                b1 * a2 + d1 * b2,
                a1 * c2 + c1 * d2,
                b1 * c2 + d1 * d2,
                a1 * e2 + c1 * f2 + e1,
                b1 * e2 + d1 * f2 + f1,
            ],
        }
    }

    /// Apply transform to a point (x, y)
    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        let [a, b, c, d, e, f] = self.matrix;
        (a * x + c * y + e, b * x + d * y + f)
    }

    pub fn to_kurbo(&self) -> Affine {
        Affine::new(self.matrix)
    }

    pub fn from_kurbo(affine: Affine) -> Self {
        Self {
            matrix: affine.as_coeffs(),
        }
    }

    pub fn is_identity(&self) -> bool {
        self.matrix == [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression for the multi-select flip drift (#2 follow-up): a flip applied
    /// to a *moved* node (non-identity transform) about a fixed world pivot must
    /// be involutive — flipping back and forth keeps it in place. The transform
    /// is applied in WORLD space via `m.then(&node)` (node first, then mirror).
    #[test]
    fn world_space_mirror_is_involutive_for_a_moved_node() {
        let node = Transform::translate(100.0, 50.0); // a node that has been moved
        let (cx, cy) = (130.0, 70.0); // arbitrary shared world pivot
        let m = Transform::scale_around(-1.0, 1.0, cx, cy); // horizontal flip

        let once = m.then(&node);
        let twice = m.then(&once);

        // Flipping twice about the same pivot returns to the original mapping.
        for (x, y) in [(0.0, 0.0), (10.0, 5.0), (-3.0, 8.0)] {
            let (ox, oy) = node.apply(x, y);
            let (tx, ty) = twice.apply(x, y);
            assert!(
                (ox - tx).abs() < 1e-9 && (oy - ty).abs() < 1e-9,
                "flip-twice not involutive: ({ox},{oy}) != ({tx},{ty})"
            );
        }

        // A single flip mirrors world-x about cx and leaves y unchanged.
        let (wx, wy) = node.apply(0.0, 0.0);
        let (fx, fy) = once.apply(0.0, 0.0);
        assert!((fx - (2.0 * cx - wx)).abs() < 1e-9);
        assert!((fy - wy).abs() < 1e-9);
    }
}
