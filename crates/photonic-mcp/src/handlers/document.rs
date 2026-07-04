use crate::protocol::{
    ApplyGraphicStyleArgs,
    ApplyWidthProfileArgs,
    BranchCreateArgs,
    BranchDeleteArgs,
    BranchSwitchArgs,
    BreakLinkToSymbolArgs,
    CheckGrammarArgs,
    DefineActionArgs,
    DefineGrammarRuleArgs,
    DefineGraphicStyleArgs,
    DefineSymbolArgs,
    DefineVariableArgs,
    DefineWidthProfileArgs,
    DeleteActionArgs,
    DeleteGrammarRuleArgs,
    DeleteGraphicStyleArgs,
    DeleteSymbolArgs,
    DeleteVariableArgs,
    DeleteWidthProfileArgs,
    DeleteWorkspaceArgs,
    LoadSymbolLibraryArgs,
    LoadWorkspaceArgs,
    PlaceSymbolArgs,
    PlayActionArgs,
    RegisterEventTriggerArgs,
    RemoveEventTriggerArgs,
    SaveWorkspaceArgs,
    SetVariableValueArgs,
    SpraySymbolInstancesArgs,
    ToolResult,
};
use crate::server::AppState;
use serde_json::json;

pub use crate::handlers::doc_state::{
    get_document_state, get_document_info, undo, redo, list_checkpoints, restore_checkpoint, diff_checkpoints, list_history, jump_to_history, get_canvas_overview, resize_canvas, get_document_template, apply_document_template, set_document_bleed, get_document_bleed, set_artboard_margins, get_artboard_margins, add_construction_line, add_dimension, list_dimensions, remove_dimension, fit_to_margins,
};
pub use crate::handlers::doc_export::{
    export_svg, export_pdf, export_raster, preview_selection, export_selection_as_svg, export_icon_set, export_design_tokens, add_export_profile, list_export_profiles, remove_export_profile, run_export_profile, import_design_tokens, set_active_layer, delete_layer, reorder_layers, duplicate_layer,
};
pub use crate::handlers::doc_swatches::{
    add_color_swatch, list_color_swatches, apply_color_swatch, update_color_swatch, delete_color_swatch, load_swatch_library, save_gradient_swatch, list_gradient_swatches, apply_gradient_swatch, delete_gradient_swatch, define_spot_color, list_spot_colors, apply_spot_color, delete_spot_color, define_pattern, list_patterns, apply_pattern_fill, delete_pattern,
};
pub use crate::handlers::doc_analysis::{
    analyze_composition, detect_rhythms, measure_distances,
};

/// Define (or update) a named graphic style.
pub async fn define_graphic_style(state: &AppState, args: DefineGraphicStyleArgs) -> ToolResult {
    tracing::debug!("tool: define_graphic_style");
    use photonic_core::GraphicStyle;

    if args.name.trim().is_empty() {
        return ToolResult::error("Graphic style name must not be empty.");
    }

    let doc = state.document.lock().await;
    let (fill_json, stroke_json, opacity) = if let Some(ref nid) = args.node_id {
        // Capture from a node
        let node_id = uuid::Uuid::parse_str(nid)
            .ok()
            .or_else(|| doc.find_node_by_name(nid).map(|n| n.id));
        let node = node_id.and_then(|id| doc.nodes.get(&id)).cloned();
        drop(doc);
        match node {
            None => return ToolResult::error(format!("Node '{}' not found.", nid)),
            Some(n) => {
                use photonic_core::node::SceneNodeKind;
                let (fill, stroke) = match &n.kind {
                    SceneNodeKind::Path(pn) => (pn.fill.clone(), pn.stroke.clone()),
                    SceneNodeKind::Text(tn) => {
                        use photonic_core::style::Stroke;
                        (tn.fill.clone(), Stroke::none())
                    }
                    SceneNodeKind::Group(_) => {
                        use photonic_core::style::{Fill, Stroke};
                        (Fill::default(), Stroke::none())
                    }
                    // raster: no fill/stroke to capture
                    SceneNodeKind::Raster(_) => {
                        use photonic_core::style::{Fill, Stroke};
                        (Fill::default(), Stroke::none())
                    }
                };
                let fj = serde_json::to_string(&fill).unwrap_or_default();
                let sj = serde_json::to_string(&stroke).unwrap_or_default();
                (fj, sj, n.opacity)
            }
        }
    } else {
        drop(doc);
        // Build from explicit parameters
        use photonic_core::style::{Fill, Stroke};
        use photonic_core::Color;
        let fill = if let Some(ref hex) = args.fill_hex {
            Color::from_hex(hex).map(Fill::solid).unwrap_or_default()
        } else {
            Fill::default()
        };
        let stroke = if let (Some(ref hex), Some(w)) = (&args.stroke_hex, args.stroke_width) {
            Color::from_hex(hex)
                .map(|c| Stroke::solid(c, w))
                .unwrap_or_default()
        } else {
            Stroke::none()
        };
        let fj = serde_json::to_string(&fill).unwrap_or_default();
        let sj = serde_json::to_string(&stroke).unwrap_or_default();
        (fj, sj, args.opacity.unwrap_or(1.0))
    };

    let mut doc = state.document.lock().await;
    let name = args.name.trim().to_string();
    let style = GraphicStyle::new(&name, fill_json, stroke_json, opacity);
    if let Some(existing) = doc.graphic_styles.iter_mut().find(|s| s.name == name) {
        *existing = style;
        ToolResult::text(format!("Updated graphic style '{}'.", name))
    } else {
        doc.graphic_styles.push(style);
        ToolResult::text(format!("Defined graphic style '{}'.", name))
    }
}

/// List all named graphic styles in the document.
pub async fn list_graphic_styles(state: &AppState) -> ToolResult {
    tracing::debug!("tool: list_graphic_styles");
    let doc = state.document.lock().await;
    let styles: Vec<serde_json::Value> = doc
        .graphic_styles
        .iter()
        .map(|s| {
            serde_json::json!({
                "name": s.name,
                "opacity": s.opacity,
                "id": s.id.to_string(),
            })
        })
        .collect();
    ToolResult::text(format!("{} graphic style(s).", styles.len()))
        .with_data(serde_json::json!({ "styles": styles }))
}

/// Apply a named graphic style to one or more nodes.
pub async fn apply_graphic_style(state: &AppState, args: ApplyGraphicStyleArgs) -> ToolResult {
    tracing::debug!("tool: apply_graphic_style");
    use photonic_core::history::Command;
    use photonic_core::node::SceneNodeKind;
    use photonic_core::style::{Fill, Stroke};

    // Read style definition first (drop lock before re-acquiring with history)
    let style_data = {
        let doc = state.document.lock().await;
        doc.graphic_styles
            .iter()
            .find(|s| s.name == args.name)
            .cloned()
    };
    let style = match style_data {
        None => return ToolResult::error(format!("No graphic style named '{}'.", args.name)),
        Some(s) => s,
    };

    let fill: Fill = serde_json::from_str(&style.fill_json).unwrap_or_default();
    let stroke: Stroke = serde_json::from_str(&style.stroke_json).unwrap_or_default();
    let opacity = style.opacity;

    let doc = state.document.lock().await;
    let mut commands: Vec<Command> = Vec::new();

    for id_str in &args.node_ids {
        let node_id = uuid::Uuid::parse_str(id_str)
            .ok()
            .or_else(|| doc.find_node_by_name(id_str).map(|n| n.id));
        if let Some(nid) = node_id {
            if let Some(node) = doc.nodes.get(&nid).cloned() {
                let mut new_node = node.clone();
                new_node.opacity = opacity;
                match &mut new_node.kind {
                    SceneNodeKind::Path(pn) => {
                        pn.fill = fill.clone();
                        pn.stroke = stroke.clone();
                    }
                    SceneNodeKind::Text(tn) => {
                        tn.fill = fill.clone();
                    }
                    SceneNodeKind::Group(_) => {}
                    // raster: no fill/stroke to apply
                    SceneNodeKind::Raster(_) => {}
                }
                commands.push(Command::UpdateNode {
                    old: node,
                    new: new_node,
                });
            }
        }
    }

    if commands.is_empty() {
        return ToolResult::text("No matching nodes found.");
    }

    let count = commands.len();
    drop(doc);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    history.execute_discrete(Command::Batch(commands), &mut doc);
    drop(history);

    ToolResult::text(format!(
        "Applied graphic style '{}' to {} node(s).",
        args.name, count
    ))
}

/// Delete a named graphic style.
pub async fn delete_graphic_style(state: &AppState, args: DeleteGraphicStyleArgs) -> ToolResult {
    tracing::debug!("tool: delete_graphic_style");
    let mut doc = state.document.lock().await;
    let before = doc.graphic_styles.len();
    doc.graphic_styles.retain(|s| s.name != args.name);
    if doc.graphic_styles.len() < before {
        ToolResult::text(format!("Deleted graphic style '{}'.", args.name))
    } else {
        ToolResult::error(format!("No graphic style named '{}' found.", args.name))
    }
}

// ─── Variable Width Profiles ─────────────────────────────────────────────────

/// Define (or overwrite) a named variable-width stroke profile.
pub async fn define_width_profile(state: &AppState, args: DefineWidthProfileArgs) -> ToolResult {
    tracing::debug!("tool: define_width_profile");
    use photonic_core::WidthProfile;

    if args.name.trim().is_empty() {
        return ToolResult::error("Width profile name must not be empty.");
    }
    if args.widths.len() < 2 {
        return ToolResult::error("Width profile must have at least 2 width values.");
    }
    if args.widths.iter().any(|&w| w < 0.0) {
        return ToolResult::error("All width values must be non-negative.");
    }

    let name = args.name.trim().to_string();
    let profile = WidthProfile::new(&name, args.widths);
    let mut doc = state.document.lock().await;

    if let Some(existing) = doc.width_profiles.iter_mut().find(|p| p.name == name) {
        *existing = profile;
        ToolResult::text(format!("Updated width profile '{}'.", name))
    } else {
        doc.width_profiles.push(profile);
        ToolResult::text(format!("Defined width profile '{}'.", name))
    }
}

/// List all named variable-width stroke profiles in the document.
pub async fn list_width_profiles(state: &AppState) -> ToolResult {
    tracing::debug!("tool: list_width_profiles");
    let doc = state.document.lock().await;
    let profiles: Vec<serde_json::Value> = doc
        .width_profiles
        .iter()
        .map(|p| {
            serde_json::json!({
                "name": p.name,
                "widths": p.widths,
                "average_width": p.average_width(),
                "id": p.id.to_string(),
            })
        })
        .collect();
    ToolResult::text(format!("{} width profile(s).", profiles.len()))
        .with_data(serde_json::json!({ "profiles": profiles }))
}

/// Apply a named width profile to path nodes (sets stroke width to the profile average).
pub async fn apply_width_profile(state: &AppState, args: ApplyWidthProfileArgs) -> ToolResult {
    tracing::debug!("tool: apply_width_profile");
    use photonic_core::history::Command;
    use photonic_core::node::SceneNodeKind;

    // Read profile (drop lock before re-acquiring)
    let (profile_id, avg_width) = {
        let doc = state.document.lock().await;
        match doc.width_profiles.iter().find(|p| p.name == args.name) {
            None => return ToolResult::error(format!("No width profile named '{}'.", args.name)),
            Some(p) => (p.id, p.average_width()),
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
                if let SceneNodeKind::Path(ref pn) = node.kind {
                    let mut new_node = node.clone();
                    if let SceneNodeKind::Path(ref mut pn2) = new_node.kind {
                        // Legacy uniform fallback + the profile link that drives
                        // true variable-width rendering.
                        pn2.stroke.width = avg_width;
                        pn2.stroke.width_profile_id = Some(profile_id);
                    }
                    let _ = pn; // suppress warning
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
        "Applied width profile '{}' (avg {:.1}px) to {} path node(s).",
        args.name, avg_width, count
    ))
}

/// Delete a named width profile.
pub async fn delete_width_profile(state: &AppState, args: DeleteWidthProfileArgs) -> ToolResult {
    tracing::debug!("tool: delete_width_profile");
    let mut doc = state.document.lock().await;
    let before = doc.width_profiles.len();
    doc.width_profiles.retain(|p| p.name != args.name);
    if doc.width_profiles.len() < before {
        ToolResult::text(format!("Deleted width profile '{}'.", args.name))
    } else {
        ToolResult::error(format!("No width profile named '{}' found.", args.name))
    }
}

// ─── Patterns ──────────────────────────────────────────────────────────────────

/// Create a live property constraint binding a node property to an expression.
pub async fn set_constraint(
    state: &AppState,
    args: crate::protocol::SetConstraintArgs,
) -> ToolResult {
    tracing::debug!("tool: set_constraint");
    use photonic_core::document::PropertyConstraint;
    use photonic_core::ops::constraints::evaluate_constraints;

    const SETTABLE: [&str; 4] = ["x", "y", "opacity", "font_size"];
    if !SETTABLE.contains(&args.property.as_str()) {
        return ToolResult::error(format!(
            "Property '{}' is not a settable constraint target (use one of {SETTABLE:?}).",
            args.property
        ));
    }

    let mut doc = state.document.lock().await;
    let node_id = uuid::Uuid::parse_str(&args.node_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.node_id).map(|n| n.id));
    let node_id = match node_id {
        Some(id) if doc.nodes.contains_key(&id) => id,
        _ => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    let constraint =
        PropertyConstraint::new(node_id, args.property.clone(), args.expression.clone());
    let constraint_id = constraint.id;
    doc.constraints.push(constraint);

    // Validate by evaluating; roll back on failure (e.g. a cycle).
    if let Err(e) = evaluate_constraints(&mut doc) {
        doc.constraints.retain(|c| c.id != constraint_id);
        let _ = evaluate_constraints(&mut doc);
        return ToolResult::error(format!("Constraint rejected: {e}"));
    }

    let current = photonic_core::ops::constraints::get_property(&doc, node_id, &args.property);
    ToolResult::text(format!(
        "Constraint set: {}.{} = {}{}.",
        args.node_id,
        args.property,
        args.expression,
        current
            .map(|v| format!(" (now {v:.3})"))
            .unwrap_or_default()
    ))
    .with_data(serde_json::json!({ "constraint_id": constraint_id.to_string() }))
}

/// List all property constraints with their current evaluated values.
pub async fn list_constraints(state: &AppState) -> ToolResult {
    tracing::debug!("tool: list_constraints");
    let doc = state.document.lock().await;
    let items: Vec<serde_json::Value> = doc
        .constraints
        .iter()
        .map(|c| {
            serde_json::json!({
                "id": c.id.to_string(),
                "node_id": c.target_node_id.to_string(),
                "property": c.target_property,
                "expression": c.expression,
                "current_value": photonic_core::ops::constraints::get_property(
                    &doc, c.target_node_id, &c.target_property
                ),
            })
        })
        .collect();
    ToolResult::text(format!("{} constraint(s).", items.len()))
        .with_data(serde_json::json!({ "constraints": items }))
}

/// Remove a property constraint by id.
pub async fn remove_constraint(
    state: &AppState,
    args: crate::protocol::RemoveConstraintArgs,
) -> ToolResult {
    tracing::debug!("tool: remove_constraint");
    let id = match uuid::Uuid::parse_str(&args.constraint_id) {
        Ok(id) => id,
        Err(_) => return ToolResult::error("Invalid constraint id."),
    };
    let mut doc = state.document.lock().await;
    let before = doc.constraints.len();
    doc.constraints.retain(|c| c.id != id);
    if doc.constraints.len() < before {
        ToolResult::text(format!("Removed constraint {id}."))
    } else {
        ToolResult::error(format!("No constraint with id {id}."))
    }
}

// ─── Document Variables ───────────────────────────────────────────────────────

/// Define (or update) a named document variable.
pub async fn define_variable(state: &AppState, args: DefineVariableArgs) -> ToolResult {
    tracing::debug!("tool: define_variable");
    use photonic_core::DocumentVariable;

    if args.name.trim().is_empty() {
        return ToolResult::error("Variable name must not be empty.");
    }
    let mut doc = state.document.lock().await;
    let name = args.name.trim().to_string();

    let action = if let Some(var) = doc.variables.iter_mut().find(|v| v.name == name) {
        var.value = args.value.clone();
        "Updated"
    } else {
        doc.variables
            .push(DocumentVariable::new(&name, &args.value));
        "Defined"
    };

    ToolResult::text(format!("{action} variable '{name}' = '{}'.", args.value))
        .with_data(serde_json::json!({ "name": name, "value": args.value }))
}

/// List all document variables.
pub async fn list_variables(state: &AppState) -> ToolResult {
    tracing::debug!("tool: list_variables");
    let doc = state.document.lock().await;
    if doc.variables.is_empty() {
        return ToolResult::text("No variables defined.")
            .with_data(serde_json::json!({ "variables": [] }));
    }
    let vars: Vec<_> = doc
        .variables
        .iter()
        .map(|v| serde_json::json!({ "name": v.name, "value": v.value }))
        .collect();
    ToolResult::text(format!("{} variable(s).", vars.len()))
        .with_data(serde_json::json!({ "variables": vars }))
}

/// Set the value of an existing document variable.
pub async fn set_variable_value(state: &AppState, args: SetVariableValueArgs) -> ToolResult {
    tracing::debug!("tool: set_variable_value");
    let mut doc = state.document.lock().await;
    match doc.variables.iter_mut().find(|v| v.name == args.name) {
        Some(var) => {
            var.value = args.value.clone();
            ToolResult::text(format!("Variable '{}' set to '{}'.", args.name, args.value))
                .with_data(serde_json::json!({ "name": args.name, "value": args.value }))
        }
        None => ToolResult::error(format!("No variable named '{}' found.", args.name)),
    }
}

/// Delete a named document variable.
pub async fn delete_variable(state: &AppState, args: DeleteVariableArgs) -> ToolResult {
    tracing::debug!("tool: delete_variable");
    let mut doc = state.document.lock().await;
    let before = doc.variables.len();
    doc.variables.retain(|v| v.name != args.name);
    if doc.variables.len() < before {
        ToolResult::text(format!("Deleted variable '{}'.", args.name))
    } else {
        ToolResult::error(format!("No variable named '{}' found.", args.name))
    }
}

/// Apply all document variables — update content of all bound text nodes.
pub async fn apply_variables(state: &AppState) -> ToolResult {
    tracing::debug!("tool: apply_variables");
    use photonic_core::history::Command;

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let var_map: std::collections::HashMap<String, String> = doc
        .variables
        .iter()
        .map(|v| (v.name.clone(), v.value.clone()))
        .collect();

    let mut updated = 0usize;
    let mut commands = Vec::new();

    for node in doc.nodes.values() {
        if let photonic_core::node::SceneNodeKind::Text(ref tn) = node.kind {
            if let Some(ref binding) = tn.variable_binding {
                if let Some(value) = var_map.get(binding.as_str()) {
                    if tn.content != *value {
                        let mut new_node = node.clone();
                        if let photonic_core::node::SceneNodeKind::Text(ref mut new_tn) =
                            new_node.kind
                        {
                            new_tn.content = value.clone();
                        }
                        commands.push(Command::UpdateNode {
                            old: node.clone(),
                            new: new_node,
                        });
                        updated += 1;
                    }
                }
            }
        }
    }

    if !commands.is_empty() {
        history.execute_discrete(Command::Batch(commands), &mut doc);
    }
    drop(history);

    ToolResult::text(format!(
        "Applied variables — {} text node(s) updated.",
        updated
    ))
    .with_data(serde_json::json!({ "nodes_updated": updated }))
}

// ─── Symbols ──────────────────────────────────────────────────────────────────

/// Designate a node as a named symbol master.
pub async fn define_symbol(state: &AppState, args: DefineSymbolArgs) -> ToolResult {
    tracing::debug!("tool: define_symbol");
    use photonic_core::Symbol;

    if args.name.trim().is_empty() {
        return ToolResult::error("Symbol name must not be empty.");
    }
    let name = args.name.trim().to_string();
    let mut doc = state.document.lock().await;

    let node_id = uuid::Uuid::parse_str(&args.node_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.node_id).map(|n| n.id));
    let node_id = match node_id {
        Some(id) => id,
        None => return ToolResult::error(format!("Node '{}' not found.", args.node_id)),
    };

    if !doc.nodes.contains_key(&node_id) {
        return ToolResult::error(format!("Node '{}' not found.", args.node_id));
    }

    // Upsert the symbol.
    let action = if let Some(existing) = doc.symbols.iter_mut().find(|s| s.name == name) {
        existing.master_node_id = node_id;
        "Updated"
    } else {
        doc.symbols.push(Symbol::new(&name, node_id));
        "Defined"
    };

    ToolResult::text(format!(
        "{action} symbol '{name}' (master: {}).",
        args.node_id
    ))
    .with_data(serde_json::json!({ "symbol_name": name, "master_node_id": node_id }))
}

/// List all symbols defined in the document.
pub async fn list_symbols(state: &AppState) -> ToolResult {
    tracing::debug!("tool: list_symbols");
    let doc = state.document.lock().await;
    if doc.symbols.is_empty() {
        return ToolResult::text("No symbols defined.")
            .with_data(serde_json::json!({ "symbols": [] }));
    }
    let syms: Vec<_> = doc.symbols.iter().map(|s| serde_json::json!({
        "name": s.name,
        "id": s.id,
        "master_node_id": s.master_node_id,
        "master_name": doc.nodes.get(&s.master_node_id).map(|n| n.name.clone()).unwrap_or_default(),
    })).collect();
    ToolResult::text(format!("{} symbol(s).", syms.len()))
        .with_data(serde_json::json!({ "symbols": syms }))
}

/// Place an instance of a named symbol at the given position.
pub async fn place_symbol(state: &AppState, args: PlaceSymbolArgs) -> ToolResult {
    tracing::debug!("tool: place_symbol");
    use photonic_core::history::Command;
    use photonic_core::transform::Transform;

    let mut doc = state.document.lock().await;

    let symbol = match doc.symbols.iter().find(|s| s.name == args.symbol_name) {
        Some(s) => s.clone(),
        None => return ToolResult::error(format!("Symbol '{}' not found.", args.symbol_name)),
    };

    let master = match doc.nodes.get(&symbol.master_node_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error(format!("Symbol master node is missing from document.")),
    };

    // Clone the master to create an instance.
    let layer_id = match doc
        .active_layer_id
        .or_else(|| doc.layer_order.first().copied())
    {
        Some(id) => id,
        None => return ToolResult::error("No layer available."),
    };
    let instance_name = format!("{} (instance)", symbol.name);
    let mut instance = master.clone();
    instance.id = uuid::Uuid::new_v4();
    instance.name = instance_name;
    instance.layer_id = layer_id;
    instance.transform = Transform::translate(args.x, args.y);
    instance.symbol_ref = Some(symbol.id);

    let instance_id = instance.id;
    let mut history = state.history.lock().await;
    history.execute_discrete(
        Command::AddNode {
            node: instance,
            layer_id: Some(layer_id),
        },
        &mut doc,
    );
    drop(history);

    ToolResult::text(format!(
        "Placed instance of '{}' at ({:.1}, {:.1}).",
        args.symbol_name, args.x, args.y
    ))
    .with_data(serde_json::json!({ "instance_id": instance_id, "symbol_name": args.symbol_name }))
}

/// Break the link between an instance node and its symbol master.
pub async fn break_link_to_symbol(state: &AppState, args: BreakLinkToSymbolArgs) -> ToolResult {
    tracing::debug!("tool: break_link_to_symbol");
    use photonic_core::history::Command;

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

    if node.symbol_ref.is_none() {
        return ToolResult::error(format!("Node '{}' is not a symbol instance.", args.node_id));
    }

    let mut new_node = node.clone();
    // Bake the current master geometry/style (+ overrides) into the instance so
    // breaking the link preserves what's rendered rather than reverting to the
    // frozen copy captured at placement time.
    new_node.kind = doc.resolve_render_node(&node).kind.clone();
    new_node.symbol_ref = None;
    new_node.symbol_fill_override = None;
    new_node.symbol_stroke_override = None;

    let mut history = state.history.lock().await;
    history.execute_discrete(
        Command::UpdateNode {
            old: node,
            new: new_node,
        },
        &mut doc,
    );
    drop(history);

    ToolResult::text(format!("Broke symbol link on node '{}'.", args.node_id))
        .with_data(serde_json::json!({ "node_id": node_id }))
}

/// Delete a named symbol from the registry (instances become unlinked standalone nodes).
pub async fn delete_symbol(state: &AppState, args: DeleteSymbolArgs) -> ToolResult {
    tracing::debug!("tool: delete_symbol");
    let mut doc = state.document.lock().await;
    let before = doc.symbols.len();
    doc.symbols.retain(|s| s.name != args.name);
    if doc.symbols.len() < before {
        ToolResult::text(format!(
            "Deleted symbol '{}'. Existing instances remain as standalone nodes.",
            args.name
        ))
    } else {
        ToolResult::error(format!("No symbol named '{}' found.", args.name))
    }
}

/// Define (or update) a named document grammar rule.
pub async fn define_grammar_rule(state: &AppState, args: DefineGrammarRuleArgs) -> ToolResult {
    tracing::debug!("tool: define_grammar_rule");
    if args.name.trim().is_empty() {
        return ToolResult::error("Rule name must not be empty.");
    }
    let valid_types = [
        "palette_includes",
        "max_colors",
        "min_text_size",
        "required_layer",
        "max_node_count",
    ];
    if !valid_types.contains(&args.rule_type.as_str()) {
        return ToolResult::error(format!(
            "Unknown rule_type '{}'. Valid types: {}",
            args.rule_type,
            valid_types.join(", ")
        ));
    }
    let params_json = args.params.to_string();
    let mut doc = state.document.lock().await;
    // Overwrite if name already exists
    let existing_idx = doc.grammar_rules.iter().position(|r| r.name == args.name);
    let rule = photonic_core::GrammarRule::new(&args.name, &args.rule_type, &params_json);
    let name = rule.name.clone();
    let rule_type = rule.rule_type.clone();
    if let Some(idx) = existing_idx {
        doc.grammar_rules[idx] = rule;
    } else {
        doc.grammar_rules.push(rule);
    }
    ToolResult::text(format!(
        "Grammar rule '{}' (type: {}) defined.",
        name, rule_type
    ))
    .with_data(json!({ "name": name, "rule_type": rule_type }))
}

/// List all grammar rules defined in the document.
pub async fn list_grammar_rules(state: &AppState) -> ToolResult {
    tracing::debug!("tool: list_grammar_rules");
    let doc = state.document.lock().await;
    let rules: Vec<serde_json::Value> = doc
        .grammar_rules
        .iter()
        .map(|r| json!({ "name": r.name, "rule_type": r.rule_type, "params": r.params_json }))
        .collect();
    ToolResult::text(format!("{} grammar rule(s).", rules.len()))
        .with_data(json!({ "rules": rules }))
}

/// Delete a named grammar rule.
pub async fn delete_grammar_rule(state: &AppState, args: DeleteGrammarRuleArgs) -> ToolResult {
    tracing::debug!("tool: delete_grammar_rule");
    let mut doc = state.document.lock().await;
    let before = doc.grammar_rules.len();
    doc.grammar_rules.retain(|r| r.name != args.name);
    if doc.grammar_rules.len() == before {
        return ToolResult::error(format!("No grammar rule named '{}'.", args.name));
    }
    ToolResult::text(format!("Grammar rule '{}' deleted.", args.name))
        .with_data(json!({ "name": args.name }))
}

/// Check the document against its grammar rules and return pass/fail per rule.
pub async fn check_grammar(state: &AppState, args: CheckGrammarArgs) -> ToolResult {
    tracing::debug!("tool: check_grammar");
    let doc = state.document.lock().await;

    if doc.grammar_rules.is_empty() {
        return ToolResult::text("No grammar rules defined.").with_data(json!({ "results": [] }));
    }

    let rules: Vec<_> = if args.rule_names.is_empty() {
        doc.grammar_rules.iter().collect()
    } else {
        doc.grammar_rules
            .iter()
            .filter(|r| args.rule_names.contains(&r.name))
            .collect()
    };

    // Pre-collect document metrics once
    use photonic_core::node::SceneNodeKind;
    use photonic_core::style::FillKind;

    let mut unique_colors: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut min_text_size: f64 = f64::MAX;
    let mut total_nodes = 0usize;

    for node in doc.nodes_in_draw_order() {
        if !node.visible {
            continue;
        }
        total_nodes += 1;
        match &node.kind {
            SceneNodeKind::Path(pn) => {
                if let FillKind::Solid(c) = &pn.fill.kind {
                    unique_colors.insert(format!("{:.3},{:.3},{:.3}", c.r, c.g, c.b));
                }
            }
            SceneNodeKind::Text(tn) => {
                if let FillKind::Solid(c) = &tn.fill.kind {
                    unique_colors.insert(format!("{:.3},{:.3},{:.3}", c.r, c.g, c.b));
                }
                if tn.font_size < min_text_size {
                    min_text_size = tn.font_size;
                }
            }
            SceneNodeKind::Group(_) => {}
            // raster: no fill color / text size to sample
            SceneNodeKind::Raster(_) => {}
        }
    }
    let layer_names: Vec<String> = doc
        .layer_order
        .iter()
        .filter_map(|id| doc.layers.get(id))
        .map(|l| l.name.clone())
        .collect();

    let mut results: Vec<serde_json::Value> = Vec::new();

    for rule in rules {
        let params: serde_json::Value = serde_json::from_str(&rule.params_json)
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

        let (passed, message) = match rule.rule_type.as_str() {
            "palette_includes" => {
                let hex = params["color_hex"].as_str().unwrap_or("").to_lowercase();
                // Parse hex to approximate r,g,b and compare against collected colors
                let target = parse_hex_to_rgb(&hex);
                let found = if let Some((tr, tg, tb)) = target {
                    unique_colors.iter().any(|c| {
                        let parts: Vec<f32> = c.split(',').filter_map(|x| x.parse().ok()).collect();
                        if parts.len() == 3 {
                            ((parts[0] - tr).abs() < 0.02)
                                && ((parts[1] - tg).abs() < 0.02)
                                && ((parts[2] - tb).abs() < 0.02)
                        } else {
                            false
                        }
                    })
                } else {
                    false
                };
                if found {
                    (true, format!("Color {} is present in the document.", hex))
                } else {
                    (
                        false,
                        format!("Color {} was not found in any visible fill.", hex),
                    )
                }
            }
            "max_colors" => {
                let limit = params["count"].as_u64().unwrap_or(10) as usize;
                if unique_colors.len() <= limit {
                    (
                        true,
                        format!(
                            "{} unique color(s) — within limit of {}.",
                            unique_colors.len(),
                            limit
                        ),
                    )
                } else {
                    (
                        false,
                        format!(
                            "{} unique color(s) exceed limit of {}.",
                            unique_colors.len(),
                            limit
                        ),
                    )
                }
            }
            "min_text_size" => {
                let min_px = params["px"].as_f64().unwrap_or(12.0);
                if min_text_size == f64::MAX {
                    (
                        true,
                        "No text nodes — constraint vacuously satisfied.".to_string(),
                    )
                } else if min_text_size >= min_px {
                    (
                        true,
                        format!(
                            "Smallest text is {:.0}px — meets minimum of {:.0}px.",
                            min_text_size, min_px
                        ),
                    )
                } else {
                    (
                        false,
                        format!(
                            "Text as small as {:.0}px found — minimum is {:.0}px.",
                            min_text_size, min_px
                        ),
                    )
                }
            }
            "required_layer" => {
                let target_name = params["name"].as_str().unwrap_or("");
                let prefix = params["prefix"].as_str().unwrap_or("");
                let found = if !target_name.is_empty() {
                    layer_names.iter().any(|n| n == target_name)
                } else if !prefix.is_empty() {
                    layer_names.iter().any(|n| n.starts_with(prefix))
                } else {
                    false
                };
                if found {
                    (true, format!("Required layer is present."))
                } else {
                    let desc = if !target_name.is_empty() {
                        format!("'{}'", target_name)
                    } else {
                        format!("with prefix '{}'", prefix)
                    };
                    (
                        false,
                        format!(
                            "Required layer {} not found. Layers: {}.",
                            desc,
                            layer_names.join(", ")
                        ),
                    )
                }
            }
            "max_node_count" => {
                let limit = params["count"].as_u64().unwrap_or(500) as usize;
                if total_nodes <= limit {
                    (
                        true,
                        format!("{} node(s) — within limit of {}.", total_nodes, limit),
                    )
                } else {
                    (
                        false,
                        format!("{} node(s) exceed limit of {}.", total_nodes, limit),
                    )
                }
            }
            _ => (false, format!("Unknown rule type '{}'.", rule.rule_type)),
        };

        results.push(json!({
            "rule": rule.name,
            "rule_type": rule.rule_type,
            "passed": passed,
            "message": message,
        }));
    }

    let pass_count = results
        .iter()
        .filter(|r| r["passed"].as_bool().unwrap_or(false))
        .count();
    let fail_count = results.len() - pass_count;
    let summary = if fail_count == 0 {
        format!("All {} rule(s) passed.", results.len())
    } else {
        format!(
            "{}/{} rule(s) passed, {} failed.",
            pass_count,
            results.len(),
            fail_count
        )
    };

    ToolResult::text(summary).with_data(json!({
        "pass_count": pass_count,
        "fail_count": fail_count,
        "results": results,
    }))
}

/// Parse a CSS hex color string to (r, g, b) in [0,1] range.
fn parse_hex_to_rgb(hex: &str) -> Option<(f32, f32, f32)> {
    let hex = hex.trim_start_matches('#');
    if hex.len() == 6 {
        let r = u8::from_str_radix(&hex[0..2], 16).ok()? as f32 / 255.0;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()? as f32 / 255.0;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()? as f32 / 255.0;
        Some((r, g, b))
    } else if hex.len() == 3 {
        let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()? as f32 / 255.0;
        let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()? as f32 / 255.0;
        let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()? as f32 / 255.0;
        Some((r, g, b))
    } else {
        None
    }
}

// ─── Actions ─────────────────────────────────────────────────────────────────

/// Define (or overwrite) a named action set — a replayable sequence of MCP tool calls.
pub async fn define_action(state: &AppState, args: DefineActionArgs) -> ToolResult {
    tracing::debug!("tool: define_action");
    if args.name.trim().is_empty() {
        return ToolResult::error("Action name must not be empty.");
    }
    if args.steps.is_empty() {
        return ToolResult::error("Action must have at least one step.");
    }
    let name = args.name.trim().to_string();
    let steps_json = serde_json::to_string(&args.steps).unwrap_or_default();
    let action_set = photonic_core::ActionSet::new(&name, &steps_json);

    let mut doc = state.document.lock().await;
    if let Some(idx) = doc.action_sets.iter().position(|a| a.name == name) {
        doc.action_sets[idx] = action_set;
        ToolResult::text(format!(
            "Action '{}' updated ({} step(s)).",
            name,
            args.steps.len()
        ))
        .with_data(json!({ "name": name, "step_count": args.steps.len() }))
    } else {
        doc.action_sets.push(action_set);
        ToolResult::text(format!(
            "Action '{}' defined ({} step(s)).",
            name,
            args.steps.len()
        ))
        .with_data(json!({ "name": name, "step_count": args.steps.len() }))
    }
}

/// List all named action sets.
pub async fn list_actions(state: &AppState) -> ToolResult {
    tracing::debug!("tool: list_actions");
    let doc = state.document.lock().await;
    let actions: Vec<serde_json::Value> = doc
        .action_sets
        .iter()
        .map(|a| {
            let step_count = serde_json::from_str::<serde_json::Value>(&a.steps_json)
                .ok()
                .and_then(|v| v.as_array().map(|arr| arr.len()))
                .unwrap_or(0);
            json!({ "name": a.name, "step_count": step_count })
        })
        .collect();
    ToolResult::text(format!("{} action(s).", actions.len()))
        .with_data(json!({ "actions": actions }))
}

/// Delete a named action set.
pub async fn delete_action(state: &AppState, args: DeleteActionArgs) -> ToolResult {
    tracing::debug!("tool: delete_action");
    let mut doc = state.document.lock().await;
    let before = doc.action_sets.len();
    doc.action_sets.retain(|a| a.name != args.name);
    if doc.action_sets.len() == before {
        ToolResult::error(format!("No action named '{}'.", args.name))
    } else {
        ToolResult::text(format!("Action '{}' deleted.", args.name))
            .with_data(json!({ "name": args.name }))
    }
}

/// Play a named action set, with optional node ID substitutions.
pub fn play_action(
    state: &AppState,
    args: PlayActionArgs,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolResult> + Send + '_>> {
    Box::pin(play_action_inner(state, args))
}

async fn play_action_inner(state: &AppState, args: PlayActionArgs) -> ToolResult {
    tracing::debug!("tool: play_action '{}'", args.name);
    use crate::protocol::ActionStep;

    // Read the steps without holding the lock during dispatch
    let steps: Vec<ActionStep> = {
        let doc = state.document.lock().await;
        let action = doc.action_sets.iter().find(|a| a.name == args.name);
        match action {
            None => return ToolResult::error(format!("No action named '{}'.", args.name)),
            Some(a) => match serde_json::from_str::<Vec<ActionStep>>(&a.steps_json) {
                Ok(s) => s,
                Err(e) => return ToolResult::error(format!("Malformed action steps: {}", e)),
            },
        }
    }; // doc lock released here

    let mut completed = 0usize;
    let mut last_error: Option<String> = None;

    for step in &steps {
        // Apply node ID substitutions to args JSON
        let mut args_value = step.args.clone();
        if !args.substitutions.is_empty() {
            let mut args_str = args_value.to_string();
            for (from, to) in &args.substitutions {
                args_str = args_str.replace(from.as_str(), to.as_str());
            }
            args_value = serde_json::from_str(&args_str).unwrap_or(step.args.clone());
        }

        // Guard against recursive action playback
        if step.tool == "play_action" {
            last_error = Some(format!(
                "Step {}: play_action cannot be nested.",
                completed + 1
            ));
            break;
        }
        match crate::server::dispatch_tool_inner(state, &step.tool, args_value).await {
            Ok(output) if output.result.is_error != Some(true) => {
                completed += 1;
            }
            Ok(output) => {
                let msg = output
                    .result
                    .content
                    .first()
                    .and_then(|c| {
                        if let crate::protocol::ContentItem::Text { text } = c {
                            Some(text.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| "Unknown error".to_string());
                last_error = Some(format!("Step {} ({}): {}", completed + 1, step.tool, msg));
                break;
            }
            Err(e) => {
                last_error = Some(format!("Step {} ({}): {}", completed + 1, step.tool, e));
                break;
            }
        }
    }

    if let Some(err) = last_error {
        ToolResult::error(format!(
            "Action '{}' failed at step {}/{}: {}",
            args.name,
            completed + 1,
            steps.len(),
            err
        ))
    } else {
        ToolResult::text(format!(
            "Action '{}' completed ({}/{} steps).",
            args.name,
            completed,
            steps.len()
        ))
        .with_data(
            json!({ "name": args.name, "steps_completed": completed, "steps_total": steps.len() }),
        )
    }
}

/// Save current document state as a named branch.
pub async fn branch_create(state: &AppState, args: BranchCreateArgs) -> ToolResult {
    tracing::debug!("tool: branch_create");
    let doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    history.branch_create(args.name.clone(), &doc);
    ToolResult::text(format!("Branch '{}' saved.", args.name))
        .with_data(json!({ "name": args.name }))
}

/// List all named branches.
pub async fn branch_list(state: &AppState) -> ToolResult {
    tracing::debug!("tool: branch_list");
    let history = state.history.lock().await;
    let names = history.branch_list();
    ToolResult::text(format!("{} branch(es).", names.len())).with_data(json!({ "branches": names }))
}

/// Switch to a named branch — restores that document snapshot.
pub async fn branch_switch(state: &AppState, args: BranchSwitchArgs) -> ToolResult {
    tracing::debug!("tool: branch_switch");
    let mut history = state.history.lock().await;
    match history.branch_switch(&args.name) {
        Some(snapshot) => {
            let mut doc = state.document.lock().await;
            *doc = snapshot;
            ToolResult::text(format!("Switched to branch '{}'.", args.name))
                .with_data(json!({ "name": args.name }))
        }
        None => ToolResult::error(format!("No branch named '{}' found.", args.name)),
    }
}

/// Delete a named branch.
pub async fn branch_delete(state: &AppState, args: BranchDeleteArgs) -> ToolResult {
    tracing::debug!("tool: branch_delete");
    let mut history = state.history.lock().await;
    if history.branch_delete(&args.name) {
        ToolResult::text(format!("Deleted branch '{}'.", args.name))
    } else {
        ToolResult::error(format!("No branch named '{}' found.", args.name))
    }
}

/// Register a script event trigger — maps a document event to a named action.
pub async fn register_event_trigger(
    state: &AppState,
    args: RegisterEventTriggerArgs,
) -> ToolResult {
    tracing::debug!("tool: register_event_trigger");

    const VALID_EVENTS: &[&str] = &[
        "on_open",
        "on_save",
        "on_node_create",
        "on_selection_change",
    ];
    if !VALID_EVENTS.contains(&args.event.as_str()) {
        return ToolResult::error(format!(
            "Unknown event '{}'. Valid events: {}",
            args.event,
            VALID_EVENTS.join(", ")
        ));
    }

    let mut doc = state.document.lock().await;

    // Verify the action exists.
    if !doc.action_sets.iter().any(|a| a.name == args.action_name) {
        return ToolResult::error(format!(
            "No action named '{}' found. Define it first with `define_action`.",
            args.action_name
        ));
    }

    // Avoid duplicate registrations.
    let already = doc
        .event_triggers
        .iter()
        .any(|t| t.event == args.event && t.action_name == args.action_name);
    if already {
        return ToolResult::text(format!(
            "Trigger '{}' → '{}' is already registered.",
            args.event, args.action_name
        ));
    }

    doc.event_triggers.push(photonic_core::EventTrigger {
        event: args.event.clone(),
        action_name: args.action_name.clone(),
    });

    ToolResult::text(format!(
        "Registered trigger: {} → {}.",
        args.event, args.action_name
    ))
    .with_data(json!({ "event": args.event, "action_name": args.action_name }))
}

/// List all registered script event triggers.
pub async fn list_event_triggers(state: &AppState) -> ToolResult {
    tracing::debug!("tool: list_event_triggers");
    let doc = state.document.lock().await;
    let entries: Vec<serde_json::Value> = doc
        .event_triggers
        .iter()
        .map(|t| json!({ "event": t.event, "action_name": t.action_name }))
        .collect();
    ToolResult::text(format!("{} event trigger(s) registered.", entries.len()))
        .with_data(json!({ "count": entries.len(), "triggers": entries }))
}

/// Remove one or all event triggers for a given event.
pub async fn remove_event_trigger(state: &AppState, args: RemoveEventTriggerArgs) -> ToolResult {
    tracing::debug!("tool: remove_event_trigger");
    let mut doc = state.document.lock().await;
    let before = doc.event_triggers.len();
    if let Some(ref aname) = args.action_name {
        doc.event_triggers
            .retain(|t| !(t.event == args.event && &t.action_name == aname));
    } else {
        doc.event_triggers.retain(|t| t.event != args.event);
    }
    let removed = before - doc.event_triggers.len();
    if removed == 0 {
        ToolResult::error(format!(
            "No matching triggers found for event '{}'.",
            args.event
        ))
    } else {
        ToolResult::text(format!(
            "Removed {} trigger(s) for event '{}'.",
            removed, args.event
        ))
        .with_data(json!({ "removed": removed }))
    }
}

/// Save the current properties-panel search query as a named workspace preset.
pub async fn save_workspace(state: &AppState, args: SaveWorkspaceArgs) -> ToolResult {
    tracing::debug!("tool: save_workspace name={}", args.name);
    if args.name.is_empty() {
        return ToolResult::error("Workspace name must not be empty.");
    }
    let mut doc = state.document.lock().await;
    if let Some(ws) = doc.workspaces.iter_mut().find(|w| w.name == args.name) {
        ws.search_query = args.search_query.clone();
    } else {
        doc.workspaces.push(photonic_core::Workspace {
            name: args.name.clone(),
            search_query: args.search_query.clone(),
        });
    }
    ToolResult::text(format!(
        "Workspace '{}' saved (query: {:?}).",
        args.name, args.search_query
    ))
    .with_data(serde_json::json!({ "name": args.name, "search_query": args.search_query }))
}

/// Load a named workspace — returns the search query to apply.
pub async fn load_workspace(state: &AppState, args: LoadWorkspaceArgs) -> ToolResult {
    tracing::debug!("tool: load_workspace name={}", args.name);
    let doc = state.document.lock().await;
    match doc.workspaces.iter().find(|w| w.name == args.name) {
        Some(ws) => {
            let q = ws.search_query.clone();
            ToolResult::text(format!(
                "Workspace '{}' loaded. Apply search_query: {:?}.",
                args.name, q
            ))
            .with_data(serde_json::json!({ "name": args.name, "search_query": q }))
        }
        None => ToolResult::error(format!("Workspace '{}' not found.", args.name)),
    }
}

/// List all saved workspace presets.
pub async fn list_workspaces(state: &AppState) -> ToolResult {
    tracing::debug!("tool: list_workspaces");
    let doc = state.document.lock().await;
    let items: Vec<serde_json::Value> = doc
        .workspaces
        .iter()
        .map(|w| serde_json::json!({ "name": w.name, "search_query": w.search_query }))
        .collect();
    ToolResult::text(format!("{} workspace(s) defined.", items.len()))
        .with_data(serde_json::json!({ "workspaces": items }))
}

/// Delete a named workspace preset.
pub async fn delete_workspace(state: &AppState, args: DeleteWorkspaceArgs) -> ToolResult {
    tracing::debug!("tool: delete_workspace name={}", args.name);
    let mut doc = state.document.lock().await;
    let before = doc.workspaces.len();
    doc.workspaces.retain(|w| w.name != args.name);
    if doc.workspaces.len() < before {
        ToolResult::text(format!("Workspace '{}' deleted.", args.name))
            .with_data(serde_json::json!({ "name": args.name }))
    } else {
        ToolResult::error(format!("Workspace '{}' not found.", args.name))
    }
}

/// Spray multiple instances of a named symbol scattered around a center point.
/// Uses the golden-angle spiral distribution for even, natural-looking scatter.
pub async fn spray_symbol_instances(
    state: &AppState,
    args: SpraySymbolInstancesArgs,
) -> ToolResult {
    tracing::debug!(
        "tool: spray_symbol_instances name={} count={}",
        args.symbol_name,
        args.count
    );
    use photonic_core::history::Command;
    use photonic_core::transform::Transform;

    let count = args.count.max(1).min(200);
    let spread = if args.spread <= 0.0 {
        100.0
    } else {
        args.spread
    };

    let mut doc = state.document.lock().await;

    let symbol = match doc.symbols.iter().find(|s| s.name == args.symbol_name) {
        Some(s) => s.clone(),
        None => return ToolResult::error(format!("Symbol '{}' not found.", args.symbol_name)),
    };

    let master = match doc.nodes.get(&symbol.master_node_id) {
        Some(n) => n.clone(),
        None => return ToolResult::error("Symbol master node is missing from document."),
    };

    let layer_id = match doc
        .active_layer_id
        .or_else(|| doc.layer_order.first().copied())
    {
        Some(id) => id,
        None => return ToolResult::error("No layer available."),
    };

    // Golden-angle spiral: even distribution of N points within a disk.
    const GOLDEN_ANGLE: f64 = std::f64::consts::TAU * (1.0 - 1.0 / 1.6180339887498949);
    let mut instance_ids = Vec::with_capacity(count);
    let mut history = state.history.lock().await;

    for i in 0..count {
        let r = spread * ((i as f64 + 0.5) / count as f64).sqrt();
        let theta = i as f64 * GOLDEN_ANGLE;
        let ix = args.x + r * theta.cos();
        let iy = args.y + r * theta.sin();

        let instance_name = format!("{} (instance {})", symbol.name, i + 1);
        let mut instance = master.clone();
        instance.id = uuid::Uuid::new_v4();
        instance.name = instance_name;
        instance.layer_id = layer_id;
        instance.transform = Transform::translate(ix, iy);
        instance.symbol_ref = Some(symbol.id);
        instance_ids.push(instance.id);
        history.execute_discrete(
            Command::AddNode {
                node: instance,
                layer_id: Some(layer_id),
            },
            &mut doc,
        );
    }

    let ids: Vec<String> = instance_ids.iter().map(|id| id.to_string()).collect();
    ToolResult::text(format!(
        "Sprayed {} instance(s) of '{}' around ({:.1}, {:.1}) with spread={:.1}.",
        count, args.symbol_name, args.x, args.y, spread
    ))
    .with_data(serde_json::json!({
        "symbol_name": args.symbol_name,
        "count": count,
        "instance_ids": ids
    }))
}

/// Built-in symbol library definitions: (name, svg_path_d)
fn builtin_symbols(library: &str) -> Option<Vec<(&'static str, &'static str)>> {
    match library {
        "arrows" => Some(vec![
            (
                "arrow-right",
                "M10,45 L70,45 L70,30 L90,50 L70,70 L70,55 L10,55 Z",
            ),
            (
                "arrow-left",
                "M90,45 L30,45 L30,30 L10,50 L30,70 L30,55 L90,55 Z",
            ),
            (
                "arrow-up",
                "M45,90 L45,30 L30,30 L50,10 L70,30 L55,30 L55,90 Z",
            ),
            (
                "arrow-down",
                "M45,10 L45,70 L30,70 L50,90 L70,70 L55,70 L55,10 Z",
            ),
            (
                "double-arrow-h",
                "M10,50 L25,35 L25,43 L75,43 L75,35 L90,50 L75,65 L75,57 L25,57 L25,65 Z",
            ),
            (
                "arrow-ne",
                "M20,80 L70,30 L45,30 L45,20 L80,20 L80,55 L70,55 L70,30",
            ),
        ]),
        "shapes" => Some(vec![
            ("diamond", "M50,5 L95,50 L50,95 L5,50 Z"),
            ("hexagon", "M50,5 L91,27 L91,73 L50,95 L9,73 L9,27 Z"),
            ("pentagon", "M50,5 L95,34 L79,88 L21,88 L5,34 Z"),
            (
                "star-5pt",
                "M50,5 L61,35 L95,35 L68,57 L79,91 L50,70 L21,91 L32,57 L5,35 L39,35 Z",
            ),
            (
                "cross",
                "M35,5 L65,5 L65,35 L95,35 L95,65 L65,65 L65,95 L35,95 L35,65 L5,65 L5,35 L35,35 Z",
            ),
            ("checkmark", "M10,50 L35,75 L90,20"),
        ]),
        "ui" => Some(vec![
            (
                "checkbox-empty",
                "M10,10 L90,10 L90,90 L10,90 Z M15,15 L85,15 L85,85 L15,85 Z",
            ),
            (
                "checkbox-checked",
                "M10,10 L90,10 L90,90 L10,90 Z M20,50 L40,70 L80,25",
            ),
            (
                "radio-empty",
                "M50,5 A45,45 0 1 1 49.9,5 Z M50,15 A35,35 0 1 1 49.9,15 Z",
            ),
            ("close-x", "M15,15 L85,85 M85,15 L15,85"),
            ("menu-lines", "M10,25 L90,25 M10,50 L90,50 M10,75 L90,75"),
            ("plus-icon", "M50,10 L50,90 M10,50 L90,50"),
        ]),
        _ => None,
    }
}

/// Load a built-in symbol library, adding all symbols to the document.
pub async fn load_symbol_library(state: &AppState, args: LoadSymbolLibraryArgs) -> ToolResult {
    tracing::debug!("tool: load_symbol_library lib={}", args.library_name);
    use photonic_core::history::Command;
    use photonic_core::node::{PathNode, SceneNode};
    use photonic_core::path::PathData;
    use photonic_core::style::Stroke;
    use photonic_core::transform::Transform;
    use photonic_core::Symbol;

    let library = args.library_name.trim().to_lowercase();
    let entries = match builtin_symbols(&library) {
        Some(e) => e,
        None => {
            return ToolResult::error(format!(
                "Unknown library '{}'. Available: arrows, shapes, ui.",
                args.library_name
            ))
        }
    };

    let mut doc = state.document.lock().await;
    let layer_id = doc
        .active_layer_id
        .or_else(|| doc.layer_order.first().copied())
        .unwrap_or(uuid::Uuid::nil());

    let mut history = state.history.lock().await;
    let mut added = Vec::new();
    let mut skipped = Vec::new();

    // Off-canvas position so master nodes don't clutter the canvas.
    const OFF_X: f64 = -9999.0;

    for (i, (name, path_d)) in entries.iter().enumerate() {
        let sym_name = format!("{}/{}", library, name);

        // Skip if already defined.
        if doc.symbols.iter().any(|s| s.name == sym_name) {
            skipped.push(sym_name);
            continue;
        }

        let path_data = match PathData::from_svg(path_d) {
            Ok(pd) => pd,
            Err(_) => continue, // Skip malformed definitions (shouldn't happen)
        };

        // Build a black fill / no stroke path node for the master.
        let mut path_node = PathNode::new(path_data);
        path_node.stroke = Stroke::none();

        let mut master = SceneNode::new(
            sym_name.clone(),
            layer_id,
            photonic_core::node::SceneNodeKind::Path(path_node),
        );
        // Place master off-canvas, staggered so nodes don't overlap.
        master.transform = Transform::translate(OFF_X + i as f64 * 150.0, -9999.0);
        master.visible = false;

        let master_id = master.id;
        history.execute_discrete(
            Command::AddNode {
                node: master,
                layer_id: Some(layer_id),
            },
            &mut doc,
        );
        doc.symbols.push(Symbol::new(&sym_name, master_id));
        added.push(sym_name);
    }

    ToolResult::text(format!(
        "Loaded '{}' library: {} symbol(s) added, {} already present.",
        library,
        added.len(),
        skipped.len()
    ))
    .with_data(serde_json::json!({
        "library": library,
        "added": added,
        "skipped": skipped,
    }))
}

// ─── Fit to Margins ───────────────────────────────────────────────────────────
