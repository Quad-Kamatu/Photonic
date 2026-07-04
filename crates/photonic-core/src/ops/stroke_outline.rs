/// Stroke outline: compute a filled closed path that traces the stroke outline
/// of a given path. Uses kurbo's built-in stroke expansion (center alignment only).
use kurbo::{BezPath, Cap, Join, Stroke as KurboStroke};

use crate::path::PathData;
use crate::style::{LineCap, LineJoin, Stroke};

/// Convert a path + stroke style into a filled outline path.
///
/// The returned [`PathData`] describes a closed shape whose filled area
/// matches exactly what the stroke would paint on the original path,
/// honouring width, caps, joins, and any dash pattern (a dashed stroke
/// outlines to one filled shape per dash, like Illustrator).
///
/// This variant assumes the path is drawn with an identity transform. When the
/// node carries a scaling transform, use [`outline_stroke_with_scale`] so the
/// outline matches Photonic's non-scaling stroke rendering.
///
/// Returns `Err` if the stroke is disabled or has zero width.
pub fn outline_stroke(path: &PathData, stroke: &Stroke) -> Result<PathData, String> {
    outline_stroke_with_scale(path, stroke, 1.0)
}

/// The uniform scale a node transform applies, `sqrt(|det|)` — matching the
/// factor the renderer uses for non-scaling strokes. Clamped away from zero.
pub fn transform_uniform_scale(matrix: &[f64; 6]) -> f64 {
    (matrix[0] * matrix[3] - matrix[1] * matrix[2])
        .abs()
        .sqrt()
        .max(1e-6)
}

/// Like [`outline_stroke`], but for a node whose transform applies `obj_scale`
/// uniform scale. Photonic renders strokes **non-scaling** — a stroke keeps a
/// constant on-canvas width regardless of object size — so the outline, which
/// is produced in the path's *local* space and then drawn under the node
/// transform, must use a local width of `stroke.width / obj_scale` (and dash
/// lengths likewise). With `obj_scale == 1.0` this is a plain stroke outline.
///
/// Returns `Err` if the stroke is disabled or has zero width.
pub fn outline_stroke_with_scale(
    path: &PathData,
    stroke: &Stroke,
    obj_scale: f64,
) -> Result<PathData, String> {
    if !stroke.enabled {
        return Err("Node has no enabled stroke to outline".into());
    }
    if stroke.width <= 0.0 {
        return Err("Stroke width must be > 0 to outline".into());
    }

    let scale = obj_scale.abs().max(1e-6);
    let bez = path.to_bez_path();

    let mut kurbo_style = KurboStroke {
        width: stroke.width / scale,
        join: match stroke.line_join {
            LineJoin::Miter => Join::Miter,
            LineJoin::Round => Join::Round,
            LineJoin::Bevel => Join::Bevel,
        },
        miter_limit: stroke.miter_limit,
        start_cap: map_cap(stroke.line_cap),
        end_cap: map_cap(stroke.line_cap),
        dash_pattern: Default::default(),
        dash_offset: 0.0,
    };
    // Trace the dashes when a real pattern is set (at least one positive dash);
    // an all-zero pattern would make kurbo emit nothing. Dash lengths are
    // non-scaling too, so divide them by the same factor as the width.
    if stroke.dash_array.iter().any(|d| *d > 0.0) {
        kurbo_style.dash_pattern = stroke.dash_array.iter().map(|d| d / scale).collect();
        kurbo_style.dash_offset = stroke.dash_offset / scale;
    }

    let outline: BezPath = kurbo::stroke(&bez, &kurbo_style, &Default::default(), 0.1);
    Ok(PathData::from_bez_path(&outline))
}

fn map_cap(cap: LineCap) -> Cap {
    match cap {
        LineCap::Butt => Cap::Butt,
        LineCap::Round => Cap::Round,
        LineCap::Square => Cap::Square,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;

    #[test]
    fn outline_of_line_is_a_centered_bar() {
        // A horizontal 100-long line stroked at width 10 → a 100×10 filled bar
        // centered on y = 0.
        let line = PathData::line(0.0, 0.0, 100.0, 0.0);
        let stroke = Stroke::solid(Color::BLACK, 10.0);
        let outline = outline_stroke(&line, &stroke).expect("outline");
        let bb = outline.bounding_box().expect("bbox");
        assert!((bb.width() - 100.0).abs() < 1.0, "width={}", bb.width());
        assert!((bb.height() - 10.0).abs() < 1.0, "height={}", bb.height());
        assert!(bb.y0 < -4.0 && bb.y1 > 4.0, "y {}..{}", bb.y0, bb.y1);
    }

    #[test]
    fn scaled_object_shrinks_local_outline_width() {
        // A width-10 stroke on an object scaled 2× must produce a local outline
        // 5 wide, so that after the ×2 transform it renders at the intended 10
        // (non-scaling stroke).
        let line = PathData::line(0.0, 0.0, 100.0, 0.0);
        let stroke = Stroke::solid(Color::BLACK, 10.0);
        let out = outline_stroke_with_scale(&line, &stroke, 2.0).expect("outline");
        let bb = out.bounding_box().expect("bbox");
        assert!((bb.height() - 5.0).abs() < 0.5, "local height={}", bb.height());
    }

    #[test]
    fn disabled_or_zero_width_errors() {
        let line = PathData::line(0.0, 0.0, 100.0, 0.0);
        assert!(outline_stroke(&line, &Stroke::none()).is_err());
        let mut zero = Stroke::solid(Color::BLACK, 0.0);
        zero.enabled = true;
        assert!(outline_stroke(&line, &zero).is_err());
    }

    #[test]
    fn dashed_stroke_outlines_multiple_pieces() {
        // A dashed stroke should trace each dash separately, so the outline has
        // more subpaths (Move commands) than the single-piece solid outline.
        let line = PathData::line(0.0, 0.0, 100.0, 0.0);
        let mut dashed = Stroke::solid(Color::BLACK, 10.0);
        dashed.dash_array = vec![10.0, 10.0];
        let solid = Stroke::solid(Color::BLACK, 10.0);
        let n_moves = |p: &PathData| {
            p.to_bez_path()
                .elements()
                .iter()
                .filter(|e| matches!(e, kurbo::PathEl::MoveTo(_)))
                .count()
        };
        let dashed_out = outline_stroke(&line, &dashed).expect("dashed outline");
        let solid_out = outline_stroke(&line, &solid).expect("solid outline");
        assert_eq!(n_moves(&solid_out), 1, "solid outline is one piece");
        assert!(
            n_moves(&dashed_out) > 1,
            "dashed outline should have multiple pieces, got {}",
            n_moves(&dashed_out)
        );
    }
}
