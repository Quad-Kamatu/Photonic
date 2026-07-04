use crate::protocol::{DefineVariableArgs, DeleteVariableArgs, SetVariableValueArgs, ToolResult};
use crate::server::AppState;

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
