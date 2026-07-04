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
pub use crate::handlers::doc_automation::{
    define_grammar_rule, list_grammar_rules, delete_grammar_rule, check_grammar, define_action, list_actions, delete_action, branch_create, branch_list, branch_switch, branch_delete, register_event_trigger, list_event_triggers, remove_event_trigger, save_workspace, load_workspace, list_workspaces, delete_workspace,
};
pub use crate::handlers::doc_data::{
    set_constraint, list_constraints, remove_constraint, define_variable, list_variables, set_variable_value, delete_variable, apply_variables,
};
pub use crate::handlers::doc_styles_symbols::{
    define_graphic_style, list_graphic_styles, apply_graphic_style, delete_graphic_style, define_width_profile, list_width_profiles, apply_width_profile, delete_width_profile, define_symbol, list_symbols, place_symbol, break_link_to_symbol, delete_symbol, spray_symbol_instances, load_symbol_library,
};





// ─── Variable Width Profiles ─────────────────────────────────────────────────





// ─── Patterns ──────────────────────────────────────────────────────────────────




// ─── Document Variables ───────────────────────────────────────────────────────






// ─── Symbols ──────────────────────────────────────────────────────────────────











// ─── Actions ─────────────────────────────────────────────────────────────────




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















// ─── Fit to Margins ───────────────────────────────────────────────────────────
