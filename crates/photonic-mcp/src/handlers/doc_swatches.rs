use crate::protocol::{
    AddColorSwatchArgs, ApplyColorSwatchArgs, ApplyGradientSwatchArgs, ApplyPatternFillArgs,
    ApplySpotColorArgs, DefinePatternArgs, DefineSpotColorArgs, DeleteColorSwatchArgs,
    DeleteGradientSwatchArgs, DeletePatternArgs, DeleteSpotColorArgs, LoadSwatchLibraryArgs,
    SaveGradientSwatchArgs, ToolResult, UpdateColorSwatchArgs,
};
use crate::server::AppState;
use photonic_core::node::SceneNodeKind;
use photonic_core::style::{Fill, FillKind};
use serde_json::json;

/// Add (or replace) a named color swatch in the document.
pub async fn add_color_swatch(state: &AppState, args: AddColorSwatchArgs) -> ToolResult {
    tracing::debug!("tool: add_color_swatch");
    use photonic_core::ColorSwatch;

    if args.name.trim().is_empty() {
        return ToolResult::error("Swatch name must not be empty");
    }

    let hex = args.color_hex.trim_start_matches('#').to_uppercase();
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return ToolResult::error(format!(
            "Invalid hex color '{}'. Use 6-digit hex e.g. #FF5733.",
            args.color_hex
        ));
    }
    let hex_full = format!("#{hex}");

    let mut doc = state.document.lock().await;
    let name = args.name.trim().to_string();

    let action = if let Some(existing) = doc.color_swatches.iter_mut().find(|s| s.name == name) {
        existing.color_hex = hex_full.clone();
        "Updated"
    } else {
        doc.color_swatches.push(ColorSwatch::new(&name, &hex_full));
        "Added"
    };

    ToolResult::text(format!("{action} color swatch '{name}' ({hex_full})."))
        .with_data(serde_json::json!({ "name": name, "color_hex": hex_full }))
}

/// List all color swatches in the document.
pub async fn list_color_swatches(state: &AppState) -> ToolResult {
    tracing::debug!("tool: list_color_swatches");
    let doc = state.document.lock().await;
    if doc.color_swatches.is_empty() {
        return ToolResult::text("No color swatches defined.");
    }
    let swatches: Vec<_> = doc
        .color_swatches
        .iter()
        .map(|s| {
            serde_json::json!({
                "id": s.id.to_string(),
                "name": s.name,
                "color_hex": s.color_hex,
            })
        })
        .collect();
    ToolResult::text(format!("{} swatch(es).", swatches.len()))
        .with_data(serde_json::json!({ "color_swatches": swatches }))
}

/// Apply a swatch color to the fill and/or stroke of the specified nodes.
pub async fn apply_color_swatch(state: &AppState, args: ApplyColorSwatchArgs) -> ToolResult {
    tracing::debug!("tool: apply_color_swatch");
    use photonic_core::history::Command;

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let swatch = match doc
        .color_swatches
        .iter()
        .find(|s| s.name == args.swatch_name)
        .cloned()
    {
        Some(s) => s,
        None => return ToolResult::error(format!("No swatch named '{}'.", args.swatch_name)),
    };

    let color = match photonic_core::color::Color::from_hex(&swatch.color_hex) {
        Some(c) => c,
        None => {
            return ToolResult::error(format!(
                "Swatch has invalid color hex '{}'.",
                swatch.color_hex
            ))
        }
    };

    let target = args.target.as_deref().unwrap_or("fill");
    let do_fill = matches!(target, "fill" | "both");
    let do_stroke = matches!(target, "stroke" | "both");

    let ids: Vec<photonic_core::NodeId> = if args.node_ids.is_empty() {
        doc.selection.node_ids.iter().copied().collect()
    } else {
        args.node_ids
            .iter()
            .filter_map(|s| {
                uuid::Uuid::parse_str(s)
                    .ok()
                    .or_else(|| doc.nodes.values().find(|n| n.name == *s).map(|n| n.id))
            })
            .collect()
    };

    if ids.is_empty() {
        return ToolResult::error("No target nodes and no active selection.");
    }

    let mut commands = Vec::new();
    for nid in &ids {
        if let Some(node) = doc.nodes.get(nid).cloned() {
            let mut new_node = node.clone();
            match &mut new_node.kind {
                photonic_core::SceneNodeKind::Path(ref mut p) => {
                    if do_fill {
                        p.fill = photonic_core::style::Fill::solid(color);
                    }
                    if do_stroke {
                        p.stroke.color = color;
                        p.stroke.enabled = true;
                    }
                }
                photonic_core::SceneNodeKind::Text(ref mut t) => {
                    if do_fill {
                        t.fill = photonic_core::style::Fill::solid(color);
                    }
                    if do_stroke {
                        t.stroke.color = color;
                        t.stroke.enabled = true;
                    }
                }
                photonic_core::SceneNodeKind::Group(_) => {}
                // raster: no fill/stroke to recolor
                photonic_core::SceneNodeKind::Raster(_) => {}
            }
            commands.push(Command::UpdateNode {
                old: node,
                new: new_node,
            });
        }
    }

    if commands.is_empty() {
        return ToolResult::error("No eligible nodes found.");
    }

    let count = commands.len();
    let batch = if commands.len() == 1 {
        commands.remove(0)
    } else {
        Command::Batch(commands)
    };
    history.execute_discrete(batch, &mut doc);

    ToolResult::text(format!(
        "Applied swatch '{}' ({}) to {} node(s).",
        swatch.name, swatch.color_hex, count
    ))
    .with_data(serde_json::json!({
        "swatch_name": swatch.name,
        "color_hex": swatch.color_hex,
        "nodes_updated": count,
        "target": target,
    }))
}

/// Rename and/or recolor a swatch. When `propagate` is true (default), all
/// nodes whose fill color matches the old color are updated to the new color.
pub async fn update_color_swatch(state: &AppState, args: UpdateColorSwatchArgs) -> ToolResult {
    tracing::debug!("tool: update_color_swatch");
    use photonic_core::history::Command;
    use photonic_core::style::FillKind;

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let old_swatch = match doc
        .color_swatches
        .iter()
        .find(|s| s.name == args.name)
        .cloned()
    {
        Some(s) => s,
        None => return ToolResult::error(format!("No swatch named '{}'.", args.name)),
    };

    let new_hex = if let Some(h) = &args.new_color_hex {
        let hex = h.trim_start_matches('#').to_uppercase();
        if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return ToolResult::error(format!("Invalid hex color '{h}'."));
        }
        format!("#{hex}")
    } else {
        old_swatch.color_hex.clone()
    };

    let new_name = args.new_name.as_deref().unwrap_or(&args.name).to_string();

    // Update the swatch entry.
    if let Some(swatch) = doc.color_swatches.iter_mut().find(|s| s.name == args.name) {
        swatch.name = new_name.clone();
        swatch.color_hex = new_hex.clone();
    }

    let mut nodes_updated = 0usize;

    // Optionally propagate color change.
    if args.propagate && args.new_color_hex.is_some() {
        let old_color = match photonic_core::color::Color::from_hex(&old_swatch.color_hex) {
            Some(c) => c,
            None => {
                return ToolResult::text(format!(
                    "Swatch '{}' updated (old color invalid, no propagation).",
                    new_name
                ))
            }
        };
        let new_color = match photonic_core::color::Color::from_hex(&new_hex) {
            Some(c) => c,
            None => return ToolResult::error(format!("New color '{}' is invalid.", new_hex)),
        };

        let tol = 1.0_f32 / 255.0_f32; // exact match only

        let all_ids: Vec<photonic_core::NodeId> = doc.nodes.keys().copied().collect();
        let mut commands = Vec::new();

        for nid in &all_ids {
            if let Some(node) = doc.nodes.get(nid).cloned() {
                let fill_matches = match &node.kind {
                    photonic_core::SceneNodeKind::Path(p) => {
                        p.fill.enabled
                            && matches!(&p.fill.kind, FillKind::Solid(c)
                            if (c.r - old_color.r).abs() <= tol
                            && (c.g - old_color.g).abs() <= tol
                            && (c.b - old_color.b).abs() <= tol)
                    }
                    photonic_core::SceneNodeKind::Text(t) => {
                        t.fill.enabled
                            && matches!(&t.fill.kind, FillKind::Solid(c)
                            if (c.r - old_color.r).abs() <= tol
                            && (c.g - old_color.g).abs() <= tol
                            && (c.b - old_color.b).abs() <= tol)
                    }
                    _ => false,
                };
                if fill_matches {
                    let mut new_node = node.clone();
                    match &mut new_node.kind {
                        photonic_core::SceneNodeKind::Path(ref mut p) => {
                            p.fill = photonic_core::style::Fill::solid(new_color);
                        }
                        photonic_core::SceneNodeKind::Text(ref mut t) => {
                            t.fill = photonic_core::style::Fill::solid(new_color);
                        }
                        _ => {}
                    }
                    commands.push(Command::UpdateNode {
                        old: node,
                        new: new_node,
                    });
                    nodes_updated += 1;
                }
            }
        }

        if !commands.is_empty() {
            let batch = if commands.len() == 1 {
                commands.remove(0)
            } else {
                Command::Batch(commands)
            };
            history.execute_discrete(batch, &mut doc);
        }
    }

    ToolResult::text(format!(
        "Updated swatch '{}' → '{}' ({}); propagated to {} node(s).",
        args.name, new_name, new_hex, nodes_updated
    ))
    .with_data(serde_json::json!({
        "old_name": args.name,
        "new_name": new_name,
        "color_hex": new_hex,
        "nodes_updated": nodes_updated,
    }))
}

/// Delete a named color swatch.
pub async fn delete_color_swatch(state: &AppState, args: DeleteColorSwatchArgs) -> ToolResult {
    tracing::debug!("tool: delete_color_swatch");
    let mut doc = state.document.lock().await;
    let before = doc.color_swatches.len();
    doc.color_swatches.retain(|s| s.name != args.name);
    if doc.color_swatches.len() < before {
        ToolResult::text(format!("Deleted color swatch '{}'.", args.name))
    } else {
        ToolResult::error(format!("No swatch named '{}' found.", args.name))
    }
}

/// Load a predefined color swatch library into the document.
pub async fn load_swatch_library(state: &AppState, args: LoadSwatchLibraryArgs) -> ToolResult {
    tracing::debug!("tool: load_swatch_library");
    use photonic_core::ColorSwatch;

    let palette: &[(&str, &str)] = match args.library.as_str() {
        "web" => &[
            ("White", "#ffffff"), ("Silver", "#c0c0c0"), ("Gray", "#808080"), ("Black", "#000000"),
            ("Red", "#ff0000"), ("Maroon", "#800000"), ("Yellow", "#ffff00"), ("Olive", "#808000"),
            ("Lime", "#00ff00"), ("Green", "#008000"), ("Aqua", "#00ffff"), ("Teal", "#008080"),
            ("Blue", "#0000ff"), ("Navy", "#000080"), ("Fuchsia", "#ff00ff"), ("Purple", "#800080"),
        ],
        "material" => &[
            ("Red 500", "#f44336"), ("Pink 500", "#e91e63"), ("Purple 500", "#9c27b0"),
            ("Deep Purple 500", "#673ab7"), ("Indigo 500", "#3f51b5"), ("Blue 500", "#2196f3"),
            ("Cyan 500", "#00bcd4"), ("Teal 500", "#009688"), ("Green 500", "#4caf50"),
            ("Yellow 500", "#ffeb3b"), ("Orange 500", "#ff9800"), ("Deep Orange 500", "#ff5722"),
            ("Brown 500", "#795548"), ("Grey 500", "#9e9e9e"), ("Blue Grey 500", "#607d8b"),
            ("White", "#ffffff"),
        ],
        "pastels" => &[
            ("Pastel Pink", "#ffb3ba"), ("Pastel Peach", "#ffdfba"), ("Pastel Yellow", "#ffffba"),
            ("Pastel Green", "#baffc9"), ("Pastel Blue", "#bae1ff"), ("Pastel Lavender", "#d4baff"),
            ("Pastel Mint", "#b5ead7"), ("Pastel Lilac", "#c7ceea"), ("Pastel Coral", "#ffd7be"),
            ("Pastel Sky", "#aec6cf"), ("Pastel Lemon", "#fffacd"), ("Pastel Rose", "#f2c6c2"),
        ],
        "earth_tones" => &[
            ("Terracotta", "#c65d3c"), ("Rust", "#b7410e"), ("Burnt Sienna", "#e97451"),
            ("Sandy Brown", "#daa06d"), ("Khaki", "#c3a882"), ("Tan", "#d2b48c"),
            ("Warm Taupe", "#b09080"), ("Driftwood", "#9a7b4f"), ("Saddle Brown", "#8b4513"),
            ("Dark Chocolate", "#5c3317"), ("Forest Floor", "#4a3728"), ("Moss", "#8a9a5b"),
        ],
        "neon" => &[
            ("Neon Pink", "#ff006e"), ("Neon Orange", "#fb5607"), ("Neon Yellow", "#ffbe0b"),
            ("Neon Green", "#8338ec"), ("Neon Cyan", "#00f5d4"), ("Neon Blue", "#3a86ff"),
            ("Electric Lime", "#ccff00"), ("Hot Magenta", "#ff00ff"), ("Laser Lemon", "#ffff66"),
            ("Neon Red", "#ff073a"), ("Electric Blue", "#00b0ff"), ("UV Purple", "#9400d3"),
        ],
        "grayscale" => &[
            ("White", "#ffffff"), ("Gray 10", "#e6e6e6"), ("Gray 20", "#cccccc"),
            ("Gray 30", "#b3b3b3"), ("Gray 40", "#999999"), ("Gray 50", "#808080"),
            ("Gray 60", "#666666"), ("Gray 70", "#4d4d4d"), ("Gray 80", "#333333"),
            ("Gray 90", "#1a1a1a"), ("Black", "#000000"),
        ],
        other => return ToolResult::error(format!(
            "Unknown library '{}'. Valid options: web, material, pastels, earth_tones, neon, grayscale.", other
        )),
    };

    let mut doc = state.document.lock().await;
    if args.clear_existing {
        doc.color_swatches.clear();
    }

    let mut added = 0usize;
    for (name, hex) in palette {
        if !doc.color_swatches.iter().any(|s| s.name == *name) {
            doc.color_swatches.push(ColorSwatch::new(*name, *hex));
            added += 1;
        }
    }

    ToolResult::text(format!(
        "Loaded '{}' library: {} swatches added ({} already existed).",
        args.library,
        added,
        palette.len() - added
    ))
}

/// Define (or overwrite) a named tiled pattern in the document registry.
pub async fn define_pattern(state: &AppState, args: DefinePatternArgs) -> ToolResult {
    tracing::debug!("tool: define_pattern");
    use base64::Engine;
    use photonic_core::document::Pattern;
    use photonic_core::style::{PatternFill, PatternTileType};
    use photonic_core::RasterImage;

    if args.name.trim().is_empty() {
        return ToolResult::error("Pattern name must not be empty.");
    }

    // Resolve tile bytes from a file path or inline base64.
    let bytes = if let Some(path) = &args.path {
        match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => return ToolResult::error(format!("Failed to read '{}': {}", path, e)),
        }
    } else if let Some(b64) = &args.data_base64 {
        match base64::engine::general_purpose::STANDARD.decode(b64.as_bytes()) {
            Ok(b) => b,
            Err(e) => return ToolResult::error(format!("Invalid base64: {}", e)),
        }
    } else {
        return ToolResult::error("define_pattern requires `path` or `data_base64`.");
    };

    let tile = match RasterImage::from_encoded(&bytes) {
        Ok(t) => t,
        Err(e) => return ToolResult::error(format!("Failed to decode tile image: {}", e)),
    };

    let mut fill = PatternFill::new(tile);
    if let Some(t) = &args.tile_type {
        match PatternTileType::from_label(t) {
            Some(tt) => fill.tile_type = tt,
            None => return ToolResult::error(format!("Unknown tile_type: {}", t)),
        }
    }
    if let Some(s) = args.scale {
        fill.scale = s;
    }
    if let Some(r) = args.rotation_degrees {
        fill.rotation = r.to_radians();
    }
    if let Some(o) = args.offset {
        fill.offset = o;
    }
    if let Some(sp) = args.spacing {
        fill.spacing = sp;
    }

    let name = args.name.trim().to_string();
    let (tw, th) = (fill.tile.width, fill.tile.height);
    let mut doc = state.document.lock().await;

    let pattern_id = if let Some(existing) = doc.patterns.iter_mut().find(|p| p.name == name) {
        existing.fill = fill;
        existing.id
    } else {
        let pattern = Pattern::new(&name, fill);
        let id = pattern.id;
        doc.patterns.push(pattern);
        id
    };

    ToolResult::text(format!(
        "Defined pattern '{}' ({}×{}px tile).",
        name, tw, th
    ))
    .with_data(serde_json::json!({
        "name": name,
        "id": pattern_id.to_string(),
        "tile_width": tw,
        "tile_height": th,
    }))
}

/// List all named patterns in the document registry.
pub async fn list_patterns(state: &AppState) -> ToolResult {
    tracing::debug!("tool: list_patterns");
    let doc = state.document.lock().await;
    let patterns: Vec<serde_json::Value> = doc
        .patterns
        .iter()
        .map(|p| {
            serde_json::json!({
                "name": p.name,
                "id": p.id.to_string(),
                "tile_width": p.fill.tile.width,
                "tile_height": p.fill.tile.height,
                "tile_type": p.fill.tile_type.label(),
                "scale": p.fill.scale,
                "rotation_degrees": p.fill.rotation.to_degrees(),
                "offset": p.fill.offset,
                "spacing": p.fill.spacing,
            })
        })
        .collect();
    ToolResult::text(format!("{} pattern(s).", patterns.len()))
        .with_data(serde_json::json!({ "patterns": patterns }))
}

/// Apply a registry pattern as the fill of path nodes (undo-safe batch).
pub async fn apply_pattern_fill(state: &AppState, args: ApplyPatternFillArgs) -> ToolResult {
    tracing::debug!("tool: apply_pattern_fill");
    use photonic_core::history::Command;
    use photonic_core::node::SceneNodeKind;
    use photonic_core::style::{Fill, PatternTileType};

    // Resolve the pattern fill (by name or id), then drop the lock.
    let fill = {
        let doc = state.document.lock().await;
        let found = doc
            .patterns
            .iter()
            .find(|p| p.name == args.pattern || p.id.to_string() == args.pattern);
        match found {
            None => {
                return ToolResult::error(format!("No pattern named '{}'.", args.pattern));
            }
            Some(p) => {
                let mut f = p.fill.clone();
                if let Some(t) = &args.tile_type {
                    match PatternTileType::from_label(t) {
                        Some(tt) => f.tile_type = tt,
                        None => return ToolResult::error(format!("Unknown tile_type: {}", t)),
                    }
                }
                if let Some(s) = args.scale {
                    f.scale = s;
                }
                if let Some(r) = args.rotation_degrees {
                    f.rotation = r.to_radians();
                }
                if let Some(o) = args.offset {
                    f.offset = o;
                }
                if let Some(sp) = args.spacing {
                    f.spacing = sp;
                }
                f
            }
        }
    };

    let doc = state.document.lock().await;
    let mut commands: Vec<Command> = Vec::new();

    for id_str in &args.node_ids {
        let node_id = uuid::Uuid::parse_str(id_str)
            .ok()
            .or_else(|| doc.find_node_by_name(id_str).map(|n| n.id));
        if let Some(nid) = node_id {
            if let Some(node) = doc.nodes.get(&nid).cloned() {
                if let SceneNodeKind::Path(_) = node.kind {
                    let mut new_node = node.clone();
                    if let SceneNodeKind::Path(ref mut pn) = new_node.kind {
                        pn.fill = Fill::pattern(fill.clone());
                    }
                    commands.push(Command::UpdateNode {
                        old: node,
                        new: new_node,
                    });
                }
            }
        }
    }

    if commands.is_empty() {
        return ToolResult::text("No matching path nodes found.");
    }

    let count = commands.len();
    drop(doc);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    history.execute_discrete(Command::Batch(commands), &mut doc);
    drop(history);

    ToolResult::text(format!(
        "Applied pattern '{}' to {} path node(s).",
        args.pattern, count
    ))
}

/// Delete a named pattern from the registry. Does not affect nodes already filled.
pub async fn delete_pattern(state: &AppState, args: DeletePatternArgs) -> ToolResult {
    tracing::debug!("tool: delete_pattern");
    let mut doc = state.document.lock().await;
    let before = doc.patterns.len();
    doc.patterns.retain(|p| p.name != args.name);
    if doc.patterns.len() < before {
        ToolResult::text(format!("Deleted pattern '{}'.", args.name))
    } else {
        ToolResult::error(format!("No pattern named '{}' found.", args.name))
    }
}

// ─── Property Constraints ──────────────────────────────────────────────────────

/// Save the gradient fill of a node as a named gradient swatch.
pub async fn save_gradient_swatch(state: &AppState, args: SaveGradientSwatchArgs) -> ToolResult {
    tracing::debug!("tool: save_gradient_swatch");
    let mut doc = state.document.lock().await;

    let node_id = uuid::Uuid::parse_str(&args.node_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.node_id).map(|n| n.id));
    let node_id = match node_id {
        Some(id) => id,
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };
    let node = match doc.nodes.get(&node_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    // Extract fill from path or text node
    let fill = match &node.kind {
        SceneNodeKind::Path(pn) => pn.fill.clone(),
        SceneNodeKind::Text(tn) => tn.fill.clone(),
        SceneNodeKind::Group(_) => return ToolResult::error("Group nodes do not have a fill."),
        // raster: no vector fill
        SceneNodeKind::Raster(_) => return ToolResult::error("Raster nodes do not have a fill."),
    };

    // Ensure it's a gradient (not solid/none)
    match &fill.kind {
        FillKind::Gradient(_) | FillKind::FluidGradient(_) | FillKind::MeshGradient(_) => {}
        _ => {
            return ToolResult::error(format!(
                "Node '{}' does not have a gradient fill. Use add_color_swatch for solid fills.",
                args.node_id
            ))
        }
    }

    // Serialize the fill to JSON for storage
    let fill_json = match serde_json::to_string(&fill) {
        Ok(s) => s,
        Err(e) => return ToolResult::error(format!("Failed to serialize fill: {}", e)),
    };

    // Replace or add swatch
    let name = args.name.clone();
    if let Some(existing) = doc.gradient_swatches.iter_mut().find(|s| s.name == name) {
        existing.fill_json = fill_json;
        ToolResult::text(format!("Updated gradient swatch '{}'.", name))
            .with_data(json!({ "name": name, "action": "updated" }))
    } else {
        use photonic_core::GradientSwatch;
        doc.gradient_swatches
            .push(GradientSwatch::new(name.clone(), fill_json));
        ToolResult::text(format!("Saved gradient swatch '{}'.", name))
            .with_data(json!({ "name": name, "action": "created" }))
    }
}

/// List all named gradient swatches.
pub async fn list_gradient_swatches(state: &AppState) -> ToolResult {
    tracing::debug!("tool: list_gradient_swatches");
    let doc = state.document.lock().await;
    let swatches: Vec<_> = doc
        .gradient_swatches
        .iter()
        .map(|s| {
            json!({
                "name": s.name,
                "id": s.id,
            })
        })
        .collect();
    ToolResult::text(format!("{} gradient swatch(es).", swatches.len()))
        .with_data(json!({ "gradient_swatches": swatches }))
}

/// Apply a named gradient swatch to one or more path nodes.
pub async fn apply_gradient_swatch(state: &AppState, args: ApplyGradientSwatchArgs) -> ToolResult {
    tracing::debug!("tool: apply_gradient_swatch");
    use photonic_core::history::Command;

    let mut doc = state.document.lock().await;

    let swatch = match doc.gradient_swatches.iter().find(|s| s.name == args.name) {
        Some(s) => s.clone(),
        None => return ToolResult::error(format!("Gradient swatch '{}' not found.", args.name)),
    };
    let fill: Fill = match serde_json::from_str(&swatch.fill_json) {
        Ok(f) => f,
        Err(e) => return ToolResult::error(format!("Corrupt swatch '{}': {}", args.name, e)),
    };

    if args.node_ids.is_empty() {
        return ToolResult::error("node_ids must not be empty.");
    }

    let mut commands = Vec::new();
    let mut applied = 0usize;
    for id_str in &args.node_ids {
        let nid = uuid::Uuid::parse_str(id_str)
            .ok()
            .or_else(|| doc.find_node_by_name(id_str).map(|n| n.id));
        if let Some(nid) = nid {
            if let Some(node) = doc.nodes.get(&nid) {
                if matches!(node.kind, SceneNodeKind::Path(_)) {
                    let mut new_node = node.clone();
                    if let SceneNodeKind::Path(ref mut pn) = new_node.kind {
                        pn.fill = fill.clone();
                    }
                    commands.push(Command::UpdateNode {
                        old: node.clone(),
                        new: new_node,
                    });
                    applied += 1;
                }
            }
        }
    }

    if commands.is_empty() {
        return ToolResult::error("No valid path nodes found in node_ids.");
    }

    let mut history = state.history.lock().await;
    for cmd in commands {
        history.execute_discrete(cmd, &mut doc);
    }
    drop(history);

    ToolResult::text(format!(
        "Applied gradient swatch '{}' to {} node(s).",
        args.name, applied
    ))
    .with_data(json!({ "name": args.name, "applied_count": applied }))
}

/// Delete a named gradient swatch.
pub async fn delete_gradient_swatch(
    state: &AppState,
    args: DeleteGradientSwatchArgs,
) -> ToolResult {
    tracing::debug!("tool: delete_gradient_swatch");
    let mut doc = state.document.lock().await;
    let before = doc.gradient_swatches.len();
    doc.gradient_swatches.retain(|s| s.name != args.name);
    if doc.gradient_swatches.len() < before {
        ToolResult::text(format!("Deleted gradient swatch '{}'.", args.name))
    } else {
        ToolResult::error(format!("No gradient swatch named '{}' found.", args.name))
    }
}

/// Define (or update) a named spot color.
pub async fn define_spot_color(state: &AppState, args: DefineSpotColorArgs) -> ToolResult {
    tracing::debug!("tool: define_spot_color");
    let mut doc = state.document.lock().await;
    // Normalise hex — ensure it starts with #
    let hex = if args.hex.starts_with('#') {
        args.hex.clone()
    } else {
        format!("#{}", args.hex)
    };
    if let Some(existing) = doc.spot_colors.iter_mut().find(|s| s.name == args.name) {
        existing.hex = hex.clone();
        existing.overprint = args.overprint;
        ToolResult::text(format!("Updated spot color '{}'.", args.name))
            .with_data(json!({ "name": args.name, "hex": hex, "overprint": args.overprint }))
    } else {
        use photonic_core::SpotColor;
        doc.spot_colors.push(SpotColor::new(
            args.name.clone(),
            hex.clone(),
            args.overprint,
        ));
        ToolResult::text(format!("Defined spot color '{}'.", args.name))
            .with_data(json!({ "name": args.name, "hex": hex, "overprint": args.overprint }))
    }
}

/// List all named spot colors.
pub async fn list_spot_colors(state: &AppState) -> ToolResult {
    tracing::debug!("tool: list_spot_colors");
    let doc = state.document.lock().await;
    let swatches: Vec<_> = doc
        .spot_colors
        .iter()
        .map(|s| {
            json!({
                "name": s.name, "hex": s.hex, "overprint": s.overprint
            })
        })
        .collect();
    ToolResult::text(format!("{} spot color(s).", swatches.len()))
        .with_data(json!({ "spot_colors": swatches }))
}

/// Apply a spot color as a solid fill to one or more nodes.
pub async fn apply_spot_color(state: &AppState, args: ApplySpotColorArgs) -> ToolResult {
    tracing::debug!("tool: apply_spot_color");
    let doc = state.document.lock().await;

    // Find the spot color
    let (hex, _overprint) = match doc.spot_colors.iter().find(|s| s.name == args.name) {
        Some(s) => (s.hex.clone(), s.overprint),
        None => return ToolResult::error(format!("No spot color named '{}' found.", args.name)),
    };

    // Parse hex to Color
    let hex_clean = hex.trim_start_matches('#');
    let (r, g, b) = if hex_clean.len() == 6 {
        let r = u8::from_str_radix(&hex_clean[0..2], 16).unwrap_or(0);
        let g = u8::from_str_radix(&hex_clean[2..4], 16).unwrap_or(0);
        let b = u8::from_str_radix(&hex_clean[4..6], 16).unwrap_or(0);
        (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
    } else {
        return ToolResult::error(format!("Invalid hex color: '{}'.", hex));
    };
    use photonic_core::color::Color;
    use photonic_core::style::{Fill, FillKind};
    let color = Color { r, g, b, a: 1.0 };
    let fill = Fill {
        kind: FillKind::Solid(color),
        opacity: 1.0,
        enabled: true,
    };

    let mut applied = 0usize;
    let mut commands = Vec::new();
    for id_str in &args.node_ids {
        let node_id = uuid::Uuid::parse_str(id_str)
            .ok()
            .or_else(|| doc.find_node_by_name(id_str).map(|n| n.id));
        if let Some(nid) = node_id {
            if let Some(node) = doc.nodes.get(&nid) {
                let mut new_node = node.clone();
                match &mut new_node.kind {
                    SceneNodeKind::Path(pn) => {
                        pn.fill = fill.clone();
                    }
                    SceneNodeKind::Text(tn) => {
                        tn.fill = fill.clone();
                    }
                    SceneNodeKind::Group(_) => {
                        continue;
                    }
                    // raster: no fill to apply
                    SceneNodeKind::Raster(_) => {
                        continue;
                    }
                }
                commands.push(photonic_core::history::Command::UpdateNode {
                    old: node.clone(),
                    new: new_node,
                });
                applied += 1;
            }
        }
    }

    if commands.is_empty() {
        return ToolResult::error("No valid nodes found in node_ids.");
    }

    drop(doc);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    for cmd in commands {
        history.execute_discrete(cmd, &mut doc);
    }
    drop(history);

    ToolResult::text(format!(
        "Applied spot color '{}' to {} node(s).",
        args.name, applied
    ))
    .with_data(json!({ "name": args.name, "applied_count": applied }))
}

/// Delete a named spot color.
pub async fn delete_spot_color(state: &AppState, args: DeleteSpotColorArgs) -> ToolResult {
    tracing::debug!("tool: delete_spot_color");
    let mut doc = state.document.lock().await;
    let before = doc.spot_colors.len();
    doc.spot_colors.retain(|s| s.name != args.name);
    if doc.spot_colors.len() < before {
        ToolResult::text(format!("Deleted spot color '{}'.", args.name))
    } else {
        ToolResult::error(format!("No spot color named '{}' found.", args.name))
    }
}
