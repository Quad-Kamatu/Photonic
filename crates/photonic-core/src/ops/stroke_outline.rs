/// Stroke outline: compute a filled closed path that traces the stroke outline
/// of a given path. Uses kurbo's built-in stroke expansion (center alignment only).
use kurbo::{BezPath, Cap, Join, Stroke as KurboStroke};

use crate::path::PathData;
use crate::style::{LineCap, LineJoin, Stroke};

/// Convert a path + stroke style into a filled outline path.
///
/// The returned [`PathData`] describes a closed shape whose filled area
/// matches exactly what the stroke would paint on the original path.
/// Dash patterns are intentionally ignored — the solid stroke outline is
/// computed regardless of any `dash_array` setting.
///
/// Returns `Err` if the stroke is disabled or has zero width.
pub fn outline_stroke(path: &PathData, stroke: &Stroke) -> Result<PathData, String> {
    if !stroke.enabled {
        return Err("Node has no enabled stroke to outline".into());
    }
    if stroke.width <= 0.0 {
        return Err("Stroke width must be > 0 to outline".into());
    }

    let bez = path.to_bez_path();

    let kurbo_style = KurboStroke {
        width: stroke.width,
        join: match stroke.line_join {
            LineJoin::Miter => Join::Miter,
            LineJoin::Round => Join::Round,
            LineJoin::Bevel => Join::Bevel,
        },
        miter_limit: stroke.miter_limit,
        start_cap: map_cap(stroke.line_cap),
        end_cap: map_cap(stroke.line_cap),
        // Ignore dash pattern — we outline the solid stroke shape.
        dash_pattern: Default::default(),
        dash_offset: 0.0,
    };

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
