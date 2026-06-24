use crate::protocol::{ColorGuideArgs, ToolResult};
use crate::server::AppState;
use photonic_core::{color::Color, node::SceneNodeKind, style::FillKind};

/// Generate a color harmony palette from a base color (or the first selected node's fill).
pub async fn color_guide(state: &AppState, args: ColorGuideArgs) -> ToolResult {
    let rule = args.rule.as_deref().unwrap_or("complementary");

    // Resolve base color: explicit hex argument → selected node fill → error.
    let base: Color = if let Some(hex) = &args.base_color {
        match Color::from_hex(hex) {
            Some(c) => c,
            None => return ToolResult::error(format!("Invalid hex color: '{hex}'")),
        }
    } else {
        // Try to extract from the first selected node's solid fill.
        let doc = state.document.lock().await;
        let selected: Vec<_> = doc.selection.ids().copied().collect();
        if selected.is_empty() {
            return ToolResult::error(
                "No base_color provided and no nodes are selected. \
                 Pass base_color as a hex string (e.g. \"#FF5500\") or select a node first.",
            );
        }
        let mut found: Option<Color> = None;
        for id in &selected {
            if let Some(node) = doc.nodes.get(id) {
                let fill_opt = match &node.kind {
                    SceneNodeKind::Path(p) => Some(&p.fill),
                    SceneNodeKind::Text(t) => Some(&t.fill),
                    SceneNodeKind::Group(_) => None,
                };
                if let Some(fill) = fill_opt {
                    if fill.enabled {
                        if let FillKind::Solid(c) = &fill.kind {
                            found = Some(*c);
                            break;
                        }
                    }
                }
            }
        }
        match found {
            Some(c) => c,
            None => {
                return ToolResult::error(
                    "Selected node(s) have no solid fill. Pass base_color explicitly.",
                )
            }
        }
    };

    let palette = base.harmony(rule);

    if palette.len() == 1 && rule != "complementary" {
        // harmony() returns just base for unknown rules
        return ToolResult::error(format!(
            "Unknown harmony rule '{}'. Supported: complementary, analogous, triadic, \
             split_complementary, tetradic, monochromatic.",
            rule
        ));
    }

    let hex_palette: Vec<String> = palette.iter().map(|c| c.to_hex()).collect();
    let swatches: Vec<serde_json::Value> = palette
        .iter()
        .enumerate()
        .map(|(i, c)| {
            serde_json::json!({
                "index": i,
                "hex": c.to_hex(),
                "r": c.r,
                "g": c.g,
                "b": c.b,
                "a": c.a,
            })
        })
        .collect();

    ToolResult::text(format!(
        "Color Guide ({rule}): {} colors — {}",
        palette.len(),
        hex_palette.join(", ")
    ))
    .with_data(serde_json::json!({
        "rule": rule,
        "base_color": base.to_hex(),
        "palette": swatches,
        "hex_palette": hex_palette,
    }))
}
