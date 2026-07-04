use crate::protocol::{FillArg, StrokeArg};

/// Apply optional fill and stroke arguments to a `PathNode`.
/// Returns `Err(message)` if either color fails to parse.
pub(crate) fn apply_style(
    path_node: &mut photonic_core::node::PathNode,
    fill: Option<FillArg>,
    stroke: Option<StrokeArg>,
) -> Result<(), String> {
    if let Some(fill_arg) = fill {
        path_node.fill = fill_arg.to_fill()?;
    }
    if let Some(stroke_arg) = stroke {
        path_node.stroke = stroke_arg.to_stroke()?;
    }
    Ok(())
}

/// Apply a `fill` paint onto a stroke slot. Solid → flat stroke color (clears any
/// gradient paint); gradient/pattern → `stroke.paint` (#201); `none` → disabled.
/// Preserves/initializes width and enables the stroke.
pub(crate) fn apply_stroke_paint(
    stroke: &mut photonic_core::style::Stroke,
    fill: &photonic_core::style::Fill,
) {
    use photonic_core::style::FillKind;
    match &fill.kind {
        FillKind::None => {
            stroke.enabled = false;
            stroke.paint = None;
        }
        FillKind::Solid(c) => {
            stroke.color = *c;
            stroke.paint = None;
            stroke.enabled = true;
            if stroke.width <= 0.0 {
                stroke.width = 1.0;
            }
        }
        other => {
            stroke.paint = Some(other.clone());
            stroke.enabled = true;
            if stroke.width <= 0.0 {
                stroke.width = 1.0;
            }
        }
    }
}

