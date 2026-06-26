use crate::protocol::{
    AddColorSwatchArgs, AddConstructionLineArgs, AddDimensionArgs, AddExportProfileArgs,
    AnalyzeCompositionArgs, ApplyColorSwatchArgs, ApplyDocumentTemplateArgs,
    ApplyGradientSwatchArgs, ApplyGraphicStyleArgs, ApplySpotColorArgs, ApplyWidthProfileArgs,
    BranchCreateArgs, BranchDeleteArgs, BranchSwitchArgs, BreakLinkToSymbolArgs, CheckGrammarArgs,
    DefineActionArgs, DefineGrammarRuleArgs, DefineGraphicStyleArgs, DefineSpotColorArgs,
    DefineSymbolArgs, DefineVariableArgs, DefineWidthProfileArgs, DeleteActionArgs,
    DeleteColorSwatchArgs, DeleteGradientSwatchArgs, DeleteGrammarRuleArgs, DeleteGraphicStyleArgs,
    DeleteLayerArgs, DeleteSpotColorArgs, DeleteSymbolArgs, DeleteVariableArgs,
    DeleteWidthProfileArgs, DeleteWorkspaceArgs, DetectRhythmsArgs, DiffCheckpointsArgs,
    DuplicateLayerArgs, ExportDesignTokensArgs, ExportRasterArgs, ExportSelectionArgs,
    ExportSvgArgs, FitToMarginsArgs, GetCanvasOverviewArgs, GetDocumentStateArgs,
    JumpToHistoryArgs, ListHistoryArgs, LoadSwatchLibraryArgs, LoadSymbolLibraryArgs,
    LoadWorkspaceArgs, MeasureDistancesArgs, PlaceSymbolArgs, PlayActionArgs,
    RegisterEventTriggerArgs, RemoveDimensionArgs, RemoveEventTriggerArgs, RemoveExportProfileArgs,
    ReorderLayersArgs, ResizeCanvasArgs, RestoreCheckpointArgs, RunExportProfileArgs,
    SaveGradientSwatchArgs, SaveWorkspaceArgs, SetActiveLayerArgs, SetArtboardMarginsArgs,
    SetDocumentBleedArgs, SetVariableValueArgs, SpraySymbolInstancesArgs, ToolResult, UndoRedoArgs,
    UpdateColorSwatchArgs,
};
use crate::server::AppState;
use photonic_core::node::SceneNodeKind;
use photonic_core::style::{Fill, FillKind};
use serde_json::json;
use std::collections::BTreeSet;

pub async fn get_document_state(state: &AppState, args: GetDocumentStateArgs) -> ToolResult {
    tracing::debug!("tool: get_document_state");
    let doc = state.document.lock().await;

    let layers: Vec<_> = doc
        .layer_order
        .iter()
        .filter(|id| {
            args.layer_id
                .map(|filter_id| filter_id == **id)
                .unwrap_or(true)
        })
        .filter_map(|id| doc.layers.get(id))
        .map(|layer| {
            let nodes: Vec<_> = layer
                .node_ids
                .iter()
                .enumerate()
                .filter_map(|(z_index, nid)| doc.nodes.get(nid).map(|n| (z_index, nid, n)))
                .map(|(z_index, _nid, node)| {
                    if args.summary_only {
                        // Compact: only id, name, kind type, z_index
                        let kind_type = match &node.kind {
                            SceneNodeKind::Path(_) => "path",
                            SceneNodeKind::Group(_) => "group",
                            SceneNodeKind::Text(_) => "text",
                        };
                        return json!({
                            "id": node.id,
                            "name": node.name,
                            "kind": kind_type,
                            "z_index": z_index,
                        });
                    }

                    let mut v = serde_json::to_value(node).unwrap_or_default();
                    // Strip verbose path data unless requested
                    if !args.include_path_data {
                        if let Some(kind) = v.get_mut("kind") {
                            if let Some(path_data) = kind.get_mut("path_data") {
                                *path_data = json!("<omitted>");
                            }
                        }
                    }
                    if let Some(obj) = v.as_object_mut() {
                        // layer_id is redundant — it's already the enclosing layer
                        obj.remove("layer_id");
                        // Inject z_index so Claude can reason about stacking order
                        obj.insert("z_index".to_string(), json!(z_index));
                        // For groups, also surface the children array at the top level
                        if let SceneNodeKind::Group(g) = &node.kind {
                            obj.insert("children".to_string(), json!(g.children));
                        }
                    }
                    v
                })
                .collect();

            json!({
                "id": layer.id,
                "name": layer.name,
                "visible": layer.visible,
                "locked": layer.locked,
                "opacity": layer.opacity,
                "node_count": nodes.len(),
                "nodes": nodes,
            })
        })
        .collect();

    let state_value = json!({
        "id": doc.id,
        "name": doc.name,
        "width": doc.width,
        "height": doc.height,
        "node_count": doc.node_count(),
        "layer_count": doc.layers.len(),
        "active_layer_id": doc.active_layer_id,
        "selection": doc.selection.ids().collect::<Vec<_>>(),
        "layers": layers,
    });

    ToolResult::text(format!(
        "Document '{}' — {} node(s) across {} layer(s)",
        doc.name,
        doc.node_count(),
        doc.layers.len()
    ))
    .with_data(state_value)
}

pub async fn get_document_info(state: &AppState) -> ToolResult {
    tracing::debug!("tool: get_document_info");
    let doc = state.document.lock().await;

    // Count nodes by kind
    let mut path_count = 0usize;
    let mut text_count = 0usize;
    let mut group_count = 0usize;
    let mut font_names: BTreeSet<String> = BTreeSet::new();
    let mut fill_hex: BTreeSet<String> = BTreeSet::new();

    for node in doc.nodes.values() {
        match &node.kind {
            SceneNodeKind::Path(p) => {
                path_count += 1;
                if p.fill.enabled {
                    if let FillKind::Solid(c) = &p.fill.kind {
                        fill_hex.insert(c.to_hex());
                    }
                }
            }
            SceneNodeKind::Text(t) => {
                text_count += 1;
                if !t.font_family.is_empty() {
                    font_names.insert(t.font_family.clone());
                }
                if t.fill.enabled {
                    if let FillKind::Solid(c) = &t.fill.kind {
                        fill_hex.insert(c.to_hex());
                    }
                }
            }
            SceneNodeKind::Group(_) => {
                group_count += 1;
            }
        }
    }

    let layer_summaries: Vec<serde_json::Value> = doc
        .layer_order
        .iter()
        .filter_map(|id| doc.layers.get(id))
        .map(|l| {
            json!({
                "id": l.id,
                "name": l.name,
                "visible": l.visible,
                "locked": l.locked,
                "is_template": l.is_template,
                "node_count": l.node_ids.len(),
            })
        })
        .collect();

    let total = path_count + text_count + group_count;

    ToolResult::text(format!(
        "Document '{}': {}×{} canvas, {} node(s) in {} layer(s) — {} path(s), {} text(s), {} group(s)",
        doc.name, doc.width as u32, doc.height as u32,
        total, layer_summaries.len(),
        path_count, text_count, group_count,
    ))
    .with_data(json!({
        "name": doc.name,
        "canvas": { "width": doc.width, "height": doc.height },
        "layer_count": layer_summaries.len(),
        "layers": layer_summaries,
        "nodes": {
            "total": total,
            "path": path_count,
            "text": text_count,
            "group": group_count,
        },
        "font_names": font_names.iter().take(20).collect::<Vec<_>>(),
        "fill_colors": fill_hex.iter().take(20).collect::<Vec<_>>(),
    }))
}

pub async fn undo(state: &AppState, args: UndoRedoArgs) -> ToolResult {
    tracing::debug!("tool: undo");
    let steps = args.steps.unwrap_or(1);
    // Acquire both locks once so the render thread is only blocked for one
    // short critical section rather than for N separate lock acquisitions.
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let mut count = 0;
    for _ in 0..steps {
        if history.undo(&mut doc) {
            count += 1;
        } else {
            break;
        }
    }
    if count > 0 {
        ToolResult::text(format!("Undid {} step(s)", count))
    } else {
        ToolResult::text("Nothing to undo")
    }
}

pub async fn redo(state: &AppState, args: UndoRedoArgs) -> ToolResult {
    tracing::debug!("tool: redo");
    let steps = args.steps.unwrap_or(1);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let mut count = 0;
    for _ in 0..steps {
        if history.redo(&mut doc) {
            count += 1;
        } else {
            break;
        }
    }
    if count > 0 {
        ToolResult::text(format!("Redid {} step(s)", count))
    } else {
        ToolResult::text("Nothing to redo")
    }
}

/// Export the document as SVG text.
pub async fn set_active_layer(state: &AppState, args: SetActiveLayerArgs) -> ToolResult {
    tracing::debug!("tool: set_active_layer");
    use photonic_core::history::Command;

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let lid = if let Ok(uuid) = uuid::Uuid::parse_str(&args.layer_id) {
        uuid
    } else {
        match doc.layers.values().find(|l| l.name == args.layer_id) {
            Some(l) => l.id,
            None => return ToolResult::error(format!("Layer not found: {}", args.layer_id)),
        }
    };

    if !doc.layers.contains_key(&lid) {
        return ToolResult::error("Layer not found");
    }

    let old_id = doc.active_layer_id;
    history.execute(
        Command::SetActiveLayer {
            old_id,
            new_id: Some(lid),
        },
        &mut doc,
    );

    let name = doc
        .layers
        .get(&lid)
        .map(|l| l.name.clone())
        .unwrap_or_default();
    ToolResult::text(format!("Active layer set to '{name}'"))
        .with_data(serde_json::json!({ "layer_id": lid, "name": name }))
}

pub async fn delete_layer(state: &AppState, args: DeleteLayerArgs) -> ToolResult {
    tracing::debug!("tool: delete_layer");
    use photonic_core::history::Command;

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    if doc.layer_order.len() <= 1 {
        return ToolResult::error("Cannot delete the last remaining layer");
    }

    let lid = if let Ok(uuid) = uuid::Uuid::parse_str(&args.layer_id) {
        uuid
    } else {
        match doc.layers.values().find(|l| l.name == args.layer_id) {
            Some(l) => l.id,
            None => return ToolResult::error(format!("Layer not found: {}", args.layer_id)),
        }
    };

    let layer = match doc.layers.get(&lid) {
        Some(l) => l.clone(),
        None => return ToolResult::error("Layer not found"),
    };

    let node_count = layer.node_ids.len();

    if args.delete_nodes {
        // Delete all nodes on the layer first.
        let mut cmds = Vec::new();
        for nid in &layer.node_ids {
            cmds.push(Command::RemoveNode { node_id: *nid });
        }
        cmds.push(Command::RemoveLayerFull {
            layer: layer.clone(),
        });
        history.execute(Command::Batch(cmds), &mut doc);
    } else {
        // Move nodes to first remaining layer, then delete the empty layer.
        let target_lid = doc
            .layer_order
            .iter()
            .find(|&&id| id != lid)
            .copied()
            .unwrap();

        let mut cmds = Vec::new();
        for (i, nid) in layer.node_ids.iter().enumerate() {
            let target_len = doc
                .layers
                .get(&target_lid)
                .map(|l| l.node_ids.len())
                .unwrap_or(0);
            cmds.push(Command::MoveNodeToLayer {
                node_id: *nid,
                old_layer_id: lid,
                new_layer_id: target_lid,
                old_index: 0, // After each move, index shifts, but we always take from front.
                new_index: target_len + i,
            });
        }
        cmds.push(Command::RemoveLayerFull {
            layer: layer.clone(),
        });
        history.execute(Command::Batch(cmds), &mut doc);
    }

    let action = if args.delete_nodes {
        "deleted with"
    } else {
        "deleted, moved"
    };
    ToolResult::text(format!(
        "Layer '{}' {} {node_count} node(s)",
        layer.name, action
    ))
    .with_data(serde_json::json!({ "layer_id": lid, "nodes_affected": node_count }))
}

pub async fn reorder_layers(state: &AppState, args: ReorderLayersArgs) -> ToolResult {
    tracing::debug!("tool: reorder_layers");
    use photonic_core::history::Command;

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let new_order: Vec<uuid::Uuid> = args
        .layer_order
        .iter()
        .filter_map(|s| uuid::Uuid::parse_str(s).ok())
        .collect();

    if new_order.len() != doc.layer_order.len() {
        return ToolResult::error(format!(
            "Layer count mismatch: provided {} but document has {}. All layers must be included.",
            new_order.len(),
            doc.layer_order.len()
        ));
    }

    // Verify all IDs are valid layers.
    for lid in &new_order {
        if !doc.layers.contains_key(lid) {
            return ToolResult::error(format!("Layer not found: {lid}"));
        }
    }

    let old_order = doc.layer_order.clone();
    history.execute(
        Command::ReorderLayers {
            old_order,
            new_order: new_order.clone(),
        },
        &mut doc,
    );

    ToolResult::text(format!("Reordered {} layers", new_order.len()))
        .with_data(serde_json::json!({ "layer_order": new_order }))
}

pub async fn duplicate_layer(state: &AppState, args: DuplicateLayerArgs) -> ToolResult {
    tracing::debug!("tool: duplicate_layer");
    use photonic_core::history::Command;
    use photonic_core::layer::Layer;

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    // Resolve layer.
    let src_layer_id = if let Ok(uuid) = uuid::Uuid::parse_str(&args.layer_id) {
        uuid
    } else {
        match doc.layers.values().find(|l| l.name == args.layer_id) {
            Some(l) => l.id,
            None => return ToolResult::error(format!("Layer not found: {}", args.layer_id)),
        }
    };

    let src_layer = match doc.layers.get(&src_layer_id) {
        Some(l) => l.clone(),
        None => return ToolResult::error("Layer not found"),
    };

    // Create new layer.
    let new_layer_name = args
        .name
        .unwrap_or_else(|| format!("{} Copy", src_layer.name));
    let mut new_layer = Layer::new(&new_layer_name);
    new_layer.visible = src_layer.visible;
    new_layer.opacity = src_layer.opacity;
    new_layer.blend_mode = src_layer.blend_mode;
    new_layer.color = src_layer.color;
    new_layer.is_template = src_layer.is_template;
    let new_layer_id = new_layer.id;

    // Deep-clone all nodes, assigning new IDs.
    let mut commands = Vec::new();
    commands.push(Command::AddLayer { layer: new_layer });

    for &nid in &src_layer.node_ids {
        if let Some(node) = doc.nodes.get(&nid) {
            let mut cloned = node.clone();
            cloned.id = uuid::Uuid::new_v4();
            cloned.name = format!("{} (copy)", node.name);
            cloned.layer_id = new_layer_id;
            commands.push(Command::AddNode {
                node: cloned,
                layer_id: Some(new_layer_id),
            });
        }
    }

    let node_count = src_layer.node_ids.len();
    history.execute(Command::Batch(commands), &mut doc);

    ToolResult::text(format!(
        "Duplicated layer '{}' → '{}' ({node_count} nodes)",
        src_layer.name, new_layer_name
    ))
    .with_data(serde_json::json!({
        "new_layer_id": new_layer_id,
        "node_count": node_count,
    }))
}

pub async fn resize_canvas(state: &AppState, args: ResizeCanvasArgs) -> ToolResult {
    tracing::debug!("tool: resize_canvas");
    use photonic_core::history::Command;

    if args.width <= 0.0 || args.height <= 0.0 {
        return ToolResult::error("Width and height must be positive");
    }

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let old_w = doc.width;
    let old_h = doc.height;

    history.execute(
        Command::ResizeCanvas {
            old_width: old_w,
            old_height: old_h,
            new_width: args.width,
            new_height: args.height,
        },
        &mut doc,
    );

    ToolResult::text(format!(
        "Resized canvas: {old_w}×{old_h} → {}×{}",
        args.width, args.height
    ))
    .with_data(serde_json::json!({
        "old_width": old_w, "old_height": old_h,
        "new_width": args.width, "new_height": args.height,
    }))
}

pub async fn export_svg(state: &AppState, args: ExportSvgArgs) -> ToolResult {
    tracing::debug!("tool: export_svg");
    let doc = state.document.lock().await;
    let opts = photonic_core::export::SvgExportOptions {
        semantic_ids: args.semantic_ids.unwrap_or(true),
        precision: args.precision.unwrap_or(4).clamp(1, 6),
        ..Default::default()
    };
    let svg = photonic_core::export::export_svg(&doc, &opts);

    let output = if args.inner_only {
        // Strip the outer <svg ...>...</svg> wrapper, returning only the body.
        svg.find('>')
            .and_then(|start| svg.rfind("</svg>").map(|end| &svg[start + 1..end]))
            .unwrap_or(&svg)
            .trim()
            .to_string()
    } else {
        svg.clone()
    };

    let byte_count = output.len();
    ToolResult::text(format!(
        "SVG export — {} bytes, {}×{} canvas",
        byte_count, doc.width, doc.height
    ))
    .with_data(serde_json::json!({ "svg": output, "bytes": byte_count }))
}

pub async fn export_raster(state: &AppState, args: ExportRasterArgs) -> ToolResult {
    tracing::debug!("tool: export_raster");

    let format = args.format.as_deref().unwrap_or("png");
    let is_jpeg = matches!(format, "jpeg" | "jpg");
    let is_webp = format == "webp";
    let is_gif = format == "gif";
    let is_tiff = matches!(format, "tiff" | "tif");
    if !matches!(
        format,
        "png" | "jpeg" | "jpg" | "webp" | "gif" | "tiff" | "tif"
    ) {
        return ToolResult::error(format!(
            "Unsupported format: '{format}'. Use 'png', 'jpeg', 'webp', 'gif', or 'tiff'."
        ));
    }

    // Capture a screenshot from the render thread (PNG bytes).
    let (tx, rx) = tokio::sync::oneshot::channel::<Vec<u8>>();
    let sent = state
        .capture_tx
        .lock()
        .map(|tx_guard| tx_guard.send(tx).is_ok())
        .unwrap_or(false);

    if !sent {
        return ToolResult::error("Export unavailable — render thread not running");
    }

    let png_bytes = match rx.await {
        Ok(b) if !b.is_empty() => b,
        _ => return ToolResult::error("Render thread did not return image data"),
    };

    // Optionally resize.
    let png_bytes = match (args.width, args.height) {
        (Some(w), Some(h)) => resize_png(&png_bytes, w, h).unwrap_or(png_bytes),
        _ => png_bytes,
    };

    // Convert format if needed.
    let (final_bytes, mime) = if is_jpeg {
        let quality = args.quality.unwrap_or(90).clamp(1, 100);
        match png_to_jpeg(&png_bytes, quality) {
            Some(jpeg) => (jpeg, "image/jpeg"),
            None => return ToolResult::error("Failed to encode JPEG"),
        }
    } else if is_webp {
        let quality = args.quality.unwrap_or(80).clamp(1, 100);
        match png_to_webp(&png_bytes, quality) {
            Some(webp) => (webp, "image/webp"),
            None => return ToolResult::error("Failed to encode WebP"),
        }
    } else if is_gif {
        match png_to_gif(&png_bytes) {
            Some(gif) => (gif, "image/gif"),
            None => return ToolResult::error("Failed to encode GIF"),
        }
    } else if is_tiff {
        match png_to_tiff(&png_bytes) {
            Some(tiff) => (tiff, "image/tiff"),
            None => return ToolResult::error("Failed to encode TIFF"),
        }
    } else {
        (png_bytes, "image/png")
    };

    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&final_bytes);
    let byte_count = final_bytes.len();
    let fmt_label = if is_jpeg {
        "JPEG"
    } else if is_webp {
        "WebP"
    } else if is_gif {
        "GIF"
    } else if is_tiff {
        "TIFF"
    } else {
        "PNG"
    };

    ToolResult::text(format!("{fmt_label} export — {byte_count} bytes")).with_data(
        serde_json::json!({
            "format": fmt_label.to_lowercase(),
            "bytes": byte_count,
            "mime": mime,
            "data_base64": b64,
        }),
    )
}

fn resize_png(png_bytes: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    use image::{imageops::FilterType, ImageFormat};
    let img = image::load_from_memory_with_format(png_bytes, ImageFormat::Png).ok()?;
    let resized = img.resize_exact(w.max(1), h.max(1), FilterType::Triangle);
    let mut out = Vec::new();
    resized
        .write_to(&mut std::io::Cursor::new(&mut out), ImageFormat::Png)
        .ok()?;
    Some(out)
}

fn png_to_jpeg(png_bytes: &[u8], quality: u8) -> Option<Vec<u8>> {
    let img = image::load_from_memory_with_format(png_bytes, image::ImageFormat::Png).ok()?;
    // Composite alpha onto white (to_rgb8 composites onto black).
    let rgba = img.to_rgba8();
    let mut rgb = image::RgbImage::new(rgba.width(), rgba.height());
    for (src, dst) in rgba.pixels().zip(rgb.pixels_mut()) {
        let a = src[3] as f32 / 255.0;
        dst[0] = (src[0] as f32 * a + 255.0 * (1.0 - a)) as u8;
        dst[1] = (src[1] as f32 * a + 255.0 * (1.0 - a)) as u8;
        dst[2] = (src[2] as f32 * a + 255.0 * (1.0 - a)) as u8;
    }
    let mut buf = Vec::new();
    let encoder =
        image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::Cursor::new(&mut buf), quality);
    image::DynamicImage::ImageRgb8(rgb)
        .write_with_encoder(encoder)
        .ok()?;
    Some(buf)
}

fn png_to_gif(png_bytes: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory_with_format(png_bytes, image::ImageFormat::Png).ok()?;
    let mut buf = Vec::new();
    let encoder = image::codecs::gif::GifEncoder::new(std::io::Cursor::new(&mut buf));
    img.write_with_encoder(encoder).ok()?;
    Some(buf)
}

fn png_to_tiff(png_bytes: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory_with_format(png_bytes, image::ImageFormat::Png).ok()?;
    let mut buf = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut buf),
        image::ImageFormat::Tiff,
    )
    .ok()?;
    Some(buf)
}

fn png_to_webp(png_bytes: &[u8], _quality: u8) -> Option<Vec<u8>> {
    let img = image::load_from_memory_with_format(png_bytes, image::ImageFormat::Png).ok()?;
    let mut buf = Vec::new();
    let encoder = image::codecs::webp::WebPEncoder::new_lossless(std::io::Cursor::new(&mut buf));
    img.write_with_encoder(encoder).ok()?;
    Some(buf)
}

/// List all saved checkpoints.
pub async fn list_checkpoints(state: &AppState) -> ToolResult {
    let infos = state.history.lock().await.list_checkpoints();
    let list: Vec<_> = infos
        .iter()
        .map(|c| json!({ "id": c.id.to_string(), "name": c.name, "created_at": c.created_at }))
        .collect();
    ToolResult::text(format!("{} checkpoint(s)", list.len()))
        .with_data(json!({ "checkpoints": list }))
}

/// Restore the document to a saved checkpoint, clearing undo/redo history.
pub async fn restore_checkpoint(state: &AppState, args: RestoreCheckpointArgs) -> ToolResult {
    let id = match uuid::Uuid::parse_str(&args.checkpoint_id) {
        Ok(id) => id,
        Err(_) => return ToolResult::error("Invalid checkpoint ID"),
    };
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    match history.restore_checkpoint(id) {
        Some(snapshot) => {
            *doc = snapshot;
            ToolResult::text(format!("Restored to checkpoint '{}'", args.checkpoint_id))
        }
        None => ToolResult::error(format!("Checkpoint '{}' not found", args.checkpoint_id)),
    }
}

/// Export a selection of nodes as a clean, minimal SVG with a tight viewBox.
pub async fn export_selection_as_svg(state: &AppState, args: ExportSelectionArgs) -> ToolResult {
    tracing::debug!("tool: export_selection_as_svg");
    let doc = state.document.lock().await;

    // Resolve node IDs: explicit list → current selection → error.
    let ids: Vec<photonic_core::node::NodeId> = match &args.node_ids {
        Some(raw) if !raw.is_empty() => raw
            .iter()
            .filter_map(|s| uuid::Uuid::parse_str(s).ok())
            .collect(),
        _ => doc.selection.ids().copied().collect(),
    };

    if ids.is_empty() {
        return ToolResult::error("No nodes specified and no active selection");
    }

    let svg = photonic_core::export::export_nodes_as_svg(&doc, &ids);

    let output = if args.as_react_component {
        let name = args.component_name.as_deref().unwrap_or("SvgIcon");
        let indented = svg
            .lines()
            .map(|l| format!("    {}", l))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "import React from 'react';\n\n\
             export function {}(props: React.SVGProps<SVGSVGElement>) {{\n  return (\n{}\n  );\n}}\n",
            name, indented
        )
    } else {
        svg
    };

    let byte_count = output.len();
    ToolResult::text(format!(
        "Selection SVG — {} node(s), {} bytes",
        ids.len(),
        byte_count
    ))
    .with_data(serde_json::json!({
        "svg": output,
        "bytes": byte_count,
        "node_count": ids.len()
    }))
}

// ─── Design Token Export ──────────────────────────────────────────────────────

/// Extract the document's design vocabulary as structured design tokens.
pub async fn export_design_tokens(state: &AppState, args: ExportDesignTokensArgs) -> ToolResult {
    tracing::debug!("tool: export_design_tokens");
    let doc = state.document.lock().await;

    let mut colors: BTreeSet<String> = BTreeSet::new();
    let mut font_families: BTreeSet<String> = BTreeSet::new();
    let mut font_sizes: Vec<f64> = Vec::new();
    let mut stroke_widths: Vec<f64> = Vec::new();

    for node in doc.nodes.values() {
        match &node.kind {
            SceneNodeKind::Path(p) => {
                collect_fill_colors(&p.fill, &mut colors);
                if p.stroke.enabled {
                    colors.insert(p.stroke.color.to_hex());
                    push_unique_f64(&mut stroke_widths, p.stroke.width);
                }
            }
            SceneNodeKind::Text(t) => {
                collect_fill_colors(&t.fill, &mut colors);
                if t.stroke.enabled {
                    colors.insert(t.stroke.color.to_hex());
                    push_unique_f64(&mut stroke_widths, t.stroke.width);
                }
                font_families.insert(t.font_family.clone());
                push_unique_f64(&mut font_sizes, t.font_size);
            }
            SceneNodeKind::Group(_) => {}
        }
    }

    font_sizes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    stroke_widths.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let format = args.format.as_deref().unwrap_or("json");
    let output = match format {
        "css" => format_tokens_css(&colors, &font_families, &font_sizes, &stroke_widths),
        "tailwind" => format_tokens_tailwind(&colors, &font_families, &font_sizes, &stroke_widths),
        "style-dictionary" => {
            format_tokens_style_dictionary(&colors, &font_families, &font_sizes, &stroke_widths)
        }
        _ => format_tokens_json(&colors, &font_families, &font_sizes, &stroke_widths),
    };

    ToolResult::text(format!(
        "Design tokens: {} color(s), {} font family/ies, {} font size(s), {} stroke width(s)",
        colors.len(),
        font_families.len(),
        font_sizes.len(),
        stroke_widths.len()
    ))
    .with_data(serde_json::json!({ "format": format, "tokens": output }))
}

fn collect_fill_colors(fill: &Fill, set: &mut BTreeSet<String>) {
    if !fill.enabled {
        return;
    }
    if let FillKind::Solid(color) = &fill.kind {
        set.insert(color.to_hex());
    }
    // Gradients are not exported as single-value tokens.
}

fn push_unique_f64(vec: &mut Vec<f64>, val: f64) {
    let already = vec.iter().any(|&v| (v - val).abs() < 0.01);
    if !already {
        vec.push(val);
    }
}

fn format_tokens_json(
    colors: &BTreeSet<String>,
    font_families: &BTreeSet<String>,
    font_sizes: &[f64],
    stroke_widths: &[f64],
) -> String {
    let colors_obj: serde_json::Map<String, serde_json::Value> = colors
        .iter()
        .enumerate()
        .map(|(i, hex)| (format!("color-{}", i + 1), serde_json::json!(hex)))
        .collect();

    let families: Vec<_> = font_families.iter().collect();
    let sizes: Vec<_> = font_sizes.iter().map(|v| serde_json::json!(v)).collect();
    let widths: Vec<_> = stroke_widths.iter().map(|v| serde_json::json!(v)).collect();

    serde_json::to_string_pretty(&serde_json::json!({
        "colors": colors_obj,
        "font_families": families,
        "font_sizes": sizes,
        "stroke_widths": widths,
    }))
    .unwrap_or_default()
}

fn format_tokens_css(
    colors: &BTreeSet<String>,
    font_families: &BTreeSet<String>,
    font_sizes: &[f64],
    stroke_widths: &[f64],
) -> String {
    let mut lines = vec![":root {".to_string()];

    for (i, hex) in colors.iter().enumerate() {
        lines.push(format!("  --color-{}: {};", i + 1, hex));
    }
    for (i, family) in font_families.iter().enumerate() {
        lines.push(format!("  --font-family-{}: {};", i + 1, family));
    }
    for (i, size) in font_sizes.iter().enumerate() {
        lines.push(format!("  --font-size-{}: {}px;", i + 1, size));
    }
    for (i, width) in stroke_widths.iter().enumerate() {
        lines.push(format!("  --stroke-width-{}: {}px;", i + 1, width));
    }

    lines.push("}".to_string());
    lines.join("\n")
}

fn format_tokens_tailwind(
    colors: &BTreeSet<String>,
    font_families: &BTreeSet<String>,
    font_sizes: &[f64],
    stroke_widths: &[f64],
) -> String {
    let colors_obj: serde_json::Map<String, serde_json::Value> = colors
        .iter()
        .enumerate()
        .map(|(i, hex)| (format!("color-{}", i + 1), serde_json::json!(hex)))
        .collect();

    let families_obj: serde_json::Map<String, serde_json::Value> = font_families
        .iter()
        .enumerate()
        .map(|(i, f)| (format!("family-{}", i + 1), serde_json::json!([f])))
        .collect();

    let sizes_obj: serde_json::Map<String, serde_json::Value> = font_sizes
        .iter()
        .enumerate()
        .map(|(i, v)| {
            (
                format!("size-{}", i + 1),
                serde_json::json!(format!("{}px", v)),
            )
        })
        .collect();

    let widths_obj: serde_json::Map<String, serde_json::Value> = stroke_widths
        .iter()
        .enumerate()
        .map(|(i, v)| {
            (
                format!("width-{}", i + 1),
                serde_json::json!(format!("{}px", v)),
            )
        })
        .collect();

    serde_json::to_string_pretty(&serde_json::json!({
        "theme": {
            "extend": {
                "colors": colors_obj,
                "fontFamily": families_obj,
                "fontSize": sizes_obj,
                "borderWidth": widths_obj,
            }
        }
    }))
    .unwrap_or_default()
}

fn format_tokens_style_dictionary(
    colors: &BTreeSet<String>,
    font_families: &BTreeSet<String>,
    font_sizes: &[f64],
    stroke_widths: &[f64],
) -> String {
    let mut root = serde_json::Map::new();

    if !colors.is_empty() {
        let color_tokens: serde_json::Map<String, serde_json::Value> = colors
            .iter()
            .enumerate()
            .map(|(i, hex)| {
                (
                    format!("color-{}", i + 1),
                    serde_json::json!({ "value": hex, "$type": "color" }),
                )
            })
            .collect();
        root.insert("color".to_string(), serde_json::Value::Object(color_tokens));
    }

    if !font_families.is_empty() {
        let family_tokens: serde_json::Map<String, serde_json::Value> = font_families
            .iter()
            .enumerate()
            .map(|(i, f)| {
                (
                    format!("family-{}", i + 1),
                    serde_json::json!({ "value": f, "$type": "fontFamily" }),
                )
            })
            .collect();
        root.insert(
            "fontFamily".to_string(),
            serde_json::Value::Object(family_tokens),
        );
    }

    if !font_sizes.is_empty() {
        let size_tokens: serde_json::Map<String, serde_json::Value> = font_sizes
            .iter()
            .enumerate()
            .map(|(i, v)| {
                (
                    format!("size-{}", i + 1),
                    serde_json::json!({ "value": format!("{}px", v), "$type": "dimension" }),
                )
            })
            .collect();
        root.insert(
            "fontSize".to_string(),
            serde_json::Value::Object(size_tokens),
        );
    }

    if !stroke_widths.is_empty() {
        let width_tokens: serde_json::Map<String, serde_json::Value> = stroke_widths
            .iter()
            .enumerate()
            .map(|(i, v)| {
                (
                    format!("width-{}", i + 1),
                    serde_json::json!({ "value": format!("{}px", v), "$type": "dimension" }),
                )
            })
            .collect();
        root.insert(
            "strokeWidth".to_string(),
            serde_json::Value::Object(width_tokens),
        );
    }

    serde_json::to_string_pretty(&serde_json::Value::Object(root)).unwrap_or_default()
}

// ─── Checkpoint Diff ─────────────────────────────────────────────────────────

/// Compare two checkpoint snapshots and return a structured diff of
/// added/removed/modified nodes and layers.
pub async fn diff_checkpoints(state: &AppState, args: DiffCheckpointsArgs) -> ToolResult {
    tracing::debug!("tool: diff_checkpoints");

    let from_uuid = match uuid::Uuid::parse_str(&args.from_id) {
        Ok(id) => id,
        Err(_) => return ToolResult::error(format!("Invalid from_id: '{}'", args.from_id)),
    };
    let to_uuid = match uuid::Uuid::parse_str(&args.to_id) {
        Ok(id) => id,
        Err(_) => return ToolResult::error(format!("Invalid to_id: '{}'", args.to_id)),
    };

    let history = state.history.lock().await;

    let from_info = history
        .list_checkpoints()
        .into_iter()
        .find(|c| c.id == from_uuid);
    let to_info = history
        .list_checkpoints()
        .into_iter()
        .find(|c| c.id == to_uuid);

    let from_doc = match history.get_checkpoint_snapshot(from_uuid) {
        Some(d) => d,
        None => return ToolResult::error(format!("Checkpoint '{}' not found", args.from_id)),
    };
    let to_doc = match history.get_checkpoint_snapshot(to_uuid) {
        Some(d) => d,
        None => return ToolResult::error(format!("Checkpoint '{}' not found", args.to_id)),
    };

    // Drop the history lock before doing the (potentially heavy) diff.
    drop(history);

    // ── Node diff ────────────────────────────────────────────────────────────
    let mut added_nodes = Vec::new();
    let mut removed_nodes = Vec::new();
    let mut modified_nodes = Vec::new();

    for (id, node) in &to_doc.nodes {
        let kind_str = match &node.kind {
            SceneNodeKind::Path(_) => "path",
            SceneNodeKind::Group(_) => "group",
            SceneNodeKind::Text(_) => "text",
        };
        if !from_doc.nodes.contains_key(id) {
            added_nodes.push(json!({ "id": id.to_string(), "name": node.name, "kind": kind_str }));
        } else if let Some(old) = from_doc.nodes.get(id) {
            let from_val = serde_json::to_value(old).unwrap_or_default();
            let to_val = serde_json::to_value(node).unwrap_or_default();
            if from_val != to_val {
                let changed: Vec<String> =
                    if let (Some(fo), Some(to)) = (from_val.as_object(), to_val.as_object()) {
                        fo.keys()
                            .filter(|k| fo.get(*k) != to.get(*k))
                            .cloned()
                            .collect()
                    } else {
                        vec![]
                    };
                modified_nodes.push(json!({
                    "id": id.to_string(),
                    "name": node.name,
                    "kind": kind_str,
                    "changed_fields": changed,
                }));
            }
        }
    }
    for (id, node) in &from_doc.nodes {
        if !to_doc.nodes.contains_key(id) {
            let kind_str = match &node.kind {
                SceneNodeKind::Path(_) => "path",
                SceneNodeKind::Group(_) => "group",
                SceneNodeKind::Text(_) => "text",
            };
            removed_nodes
                .push(json!({ "id": id.to_string(), "name": node.name, "kind": kind_str }));
        }
    }

    // ── Layer diff ────────────────────────────────────────────────────────────
    let mut added_layers = Vec::new();
    let mut removed_layers = Vec::new();
    let mut modified_layers = Vec::new();

    for (id, layer) in &to_doc.layers {
        if !from_doc.layers.contains_key(id) {
            added_layers.push(json!({ "id": id.to_string(), "name": layer.name }));
        } else if let Some(old) = from_doc.layers.get(id) {
            let from_val = serde_json::to_value(old).unwrap_or_default();
            let to_val = serde_json::to_value(layer).unwrap_or_default();
            if from_val != to_val {
                let changed: Vec<String> =
                    if let (Some(fo), Some(to)) = (from_val.as_object(), to_val.as_object()) {
                        fo.keys()
                            .filter(|k| fo.get(*k) != to.get(*k))
                            .cloned()
                            .collect()
                    } else {
                        vec![]
                    };
                modified_layers.push(json!({
                    "id": id.to_string(),
                    "name": layer.name,
                    "changed_fields": changed,
                }));
            }
        }
    }
    for (id, layer) in &from_doc.layers {
        if !to_doc.layers.contains_key(id) {
            removed_layers.push(json!({ "id": id.to_string(), "name": layer.name }));
        }
    }

    let total_changes = added_nodes.len()
        + removed_nodes.len()
        + modified_nodes.len()
        + added_layers.len()
        + removed_layers.len()
        + modified_layers.len();

    let from_name = from_info
        .as_ref()
        .map(|c| c.name.as_str())
        .unwrap_or("<unknown>");
    let to_name = to_info
        .as_ref()
        .map(|c| c.name.as_str())
        .unwrap_or("<unknown>");

    ToolResult::text(format!(
        "Diff '{}' → '{}': {} node change(s) ({} added, {} removed, {} modified), {} layer change(s)",
        from_name, to_name,
        added_nodes.len() + removed_nodes.len() + modified_nodes.len(),
        added_nodes.len(), removed_nodes.len(), modified_nodes.len(),
        added_layers.len() + removed_layers.len() + modified_layers.len(),
    ))
    .with_data(json!({
        "from_checkpoint": {
            "id": args.from_id,
            "name": from_info.as_ref().map(|c| c.name.clone()).unwrap_or_default(),
            "created_at": from_info.as_ref().map(|c| c.created_at).unwrap_or(0),
        },
        "to_checkpoint": {
            "id": args.to_id,
            "name": to_info.as_ref().map(|c| c.name.clone()).unwrap_or_default(),
            "created_at": to_info.as_ref().map(|c| c.created_at).unwrap_or(0),
        },
        "total_changes": total_changes,
        "nodes": {
            "added":    added_nodes,
            "removed":  removed_nodes,
            "modified": modified_nodes,
        },
        "layers": {
            "added":    added_layers,
            "removed":  removed_layers,
            "modified": modified_layers,
        },
    }))
}

// ─── export profiles ─────────────────────────────────────────────────────────

pub async fn add_export_profile(state: &AppState, args: AddExportProfileArgs) -> ToolResult {
    tracing::debug!("tool: add_export_profile");

    let format = args.format.to_lowercase();
    if !matches!(format.as_str(), "svg" | "png" | "jpeg" | "jpg" | "webp") {
        return ToolResult::error(format!(
            "Unsupported format '{}'. Use svg, png, jpeg, or webp.",
            args.format
        ));
    }
    if args.name.trim().is_empty() {
        return ToolResult::error("Profile name must not be empty");
    }

    use photonic_core::ExportProfile;
    let profile = ExportProfile {
        name: args.name.trim().to_string(),
        format: format.clone(),
        width: args.width,
        height: args.height,
        semantic_ids: args.semantic_ids,
        precision: args.precision,
    };

    let mut doc = state.document.lock().await;
    // Replace existing or append.
    if let Some(existing) = doc
        .export_profiles
        .iter_mut()
        .find(|p| p.name == profile.name)
    {
        *existing = profile.clone();
        ToolResult::text(format!("Updated export profile '{}'.", profile.name))
    } else {
        doc.export_profiles.push(profile.clone());
        ToolResult::text(format!(
            "Added export profile '{}' ({}).",
            profile.name, format
        ))
    }
    .with_data(serde_json::json!({ "name": profile.name, "format": format }))
}

pub async fn list_export_profiles(state: &AppState) -> ToolResult {
    tracing::debug!("tool: list_export_profiles");
    let doc = state.document.lock().await;
    if doc.export_profiles.is_empty() {
        return ToolResult::text("No export profiles defined.");
    }
    let profiles: Vec<_> = doc
        .export_profiles
        .iter()
        .map(|p| {
            serde_json::json!({
                "name": p.name,
                "format": p.format,
                "width": p.width,
                "height": p.height,
                "semantic_ids": p.semantic_ids,
                "precision": p.precision,
            })
        })
        .collect();
    ToolResult::text(format!("{} export profile(s) defined.", profiles.len()))
        .with_data(serde_json::json!({ "profiles": profiles }))
}

pub async fn remove_export_profile(state: &AppState, args: RemoveExportProfileArgs) -> ToolResult {
    tracing::debug!("tool: remove_export_profile");
    let mut doc = state.document.lock().await;
    let before = doc.export_profiles.len();
    doc.export_profiles.retain(|p| p.name != args.name);
    if doc.export_profiles.len() < before {
        ToolResult::text(format!("Removed export profile '{}'.", args.name))
    } else {
        ToolResult::error(format!("No profile named '{}' found.", args.name))
    }
}

pub async fn run_export_profile(state: &AppState, args: RunExportProfileArgs) -> ToolResult {
    tracing::debug!("tool: run_export_profile");

    let profile = {
        let doc = state.document.lock().await;
        doc.export_profiles
            .iter()
            .find(|p| p.name == args.name)
            .cloned()
    };

    let profile = match profile {
        Some(p) => p,
        None => return ToolResult::error(format!("No export profile named '{}'.", args.name)),
    };

    match profile.format.as_str() {
        "svg" => {
            let svg_args = crate::protocol::ExportSvgArgs {
                semantic_ids: profile.semantic_ids,
                precision: profile.precision.map(|p| p as u8),
                inner_only: false,
            };
            export_svg(state, svg_args).await
        }
        "png" | "jpeg" | "jpg" | "webp" => {
            let raster_args = crate::protocol::ExportRasterArgs {
                format: Some(profile.format.clone()),
                width: profile.width,
                height: profile.height,
                quality: None,
            };
            export_raster(state, raster_args).await
        }
        other => ToolResult::error(format!("Unknown format '{}' in profile.", other)),
    }
}

// ─── Document Templates ───────────────────────────────────────────────────────

/// Return the current document as a reusable template: canvas size, layers,
/// guides, and export profiles are preserved; all node content is stripped.
pub async fn get_document_template(state: &AppState) -> ToolResult {
    tracing::debug!("tool: get_document_template");
    let doc = state.document.lock().await;

    // Clone and strip all node content so the template carries structure only.
    let mut template = doc.clone();
    template.nodes.clear();
    template.selection = Default::default();
    for layer in template.layers.values_mut() {
        layer.node_ids.clear();
    }

    match template.to_json() {
        Ok(json_str) => {
            let bytes = json_str.len();
            ToolResult::text(format!(
                "Document template captured — {} layer(s), {} guide(s), {} export profile(s) ({bytes} bytes)",
                template.layers.len(),
                template.guides.len(),
                template.export_profiles.len(),
            ))
            .with_data(serde_json::json!({
                "template_json": json_str,
                "layer_count": template.layers.len(),
                "guide_count": template.guides.len(),
                "export_profile_count": template.export_profiles.len(),
                "canvas": { "width": template.width, "height": template.height },
            }))
        }
        Err(e) => ToolResult::error(format!("Failed to serialize template: {e}")),
    }
}

/// Apply a template (from `get_document_template`) to the current document.
/// Canvas size, guides, and export profiles from the template are merged in;
/// existing nodes are preserved. New layers from the template are added only
/// if no layer with the same name already exists.
pub async fn apply_document_template(
    state: &AppState,
    args: ApplyDocumentTemplateArgs,
) -> ToolResult {
    tracing::debug!("tool: apply_document_template");
    use photonic_core::history::Command;

    let template = match photonic_core::document::Document::from_json(&args.template_json) {
        Ok(t) => t,
        Err(e) => return ToolResult::error(format!("Invalid template JSON: {e}")),
    };

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    let mut commands: Vec<Command> = Vec::new();

    // 1. Canvas size.
    if template.width > 0.0
        && template.height > 0.0
        && (template.width != doc.width || template.height != doc.height)
    {
        commands.push(Command::ResizeCanvas {
            old_width: doc.width,
            old_height: doc.height,
            new_width: template.width,
            new_height: template.height,
        });
    }

    // Execute canvas resize early so subsequent operations see correct size.
    if !commands.is_empty() {
        history.execute(Command::Batch(commands.clone()), &mut doc);
        commands.clear();
    }

    // 2. Guides — add only those not already present (deduplicate by axis+position).
    use photonic_core::document::Guide;
    let mut guides_added = 0usize;
    for tg in &template.guides {
        let already = doc
            .guides
            .iter()
            .any(|g| g.orientation == tg.orientation && (g.position - tg.position).abs() < 0.5);
        if !already {
            doc.guides.push(Guide::new(tg.orientation, tg.position));
            guides_added += 1;
        }
    }

    // 3. Export profiles — replace same-name or append.
    let mut profiles_added = 0usize;
    let mut profiles_updated = 0usize;
    for tp in &template.export_profiles {
        if let Some(existing) = doc.export_profiles.iter_mut().find(|p| p.name == tp.name) {
            *existing = tp.clone();
            profiles_updated += 1;
        } else {
            doc.export_profiles.push(tp.clone());
            profiles_added += 1;
        }
    }

    // 4. Layers — add template layers whose name doesn't exist in current doc.
    let mut layers_added = 0usize;
    for tlid in &template.layer_order {
        if let Some(tlayer) = template.layers.get(tlid) {
            let name_exists = doc.layers.values().any(|l| l.name == tlayer.name);
            if !name_exists {
                let mut new_layer = tlayer.clone();
                new_layer.node_ids.clear(); // template layers have no nodes
                commands.push(Command::AddLayer { layer: new_layer });
                layers_added += 1;
            }
        }
    }
    if !commands.is_empty() {
        history.execute(Command::Batch(commands), &mut doc);
    }

    ToolResult::text(format!(
        "Template applied — {} layer(s) added, {} guide(s) added, {} export profile(s) added/updated",
        layers_added, guides_added, profiles_added + profiles_updated,
    ))
    .with_data(serde_json::json!({
        "layers_added": layers_added,
        "guides_added": guides_added,
        "export_profiles_added": profiles_added,
        "export_profiles_updated": profiles_updated,
        "canvas": { "width": doc.width, "height": doc.height },
    }))
}

// ─── Color Swatches ───────────────────────────────────────────────────────────

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
    history.execute(batch, &mut doc);

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
            history.execute(batch, &mut doc);
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

// ─── Graphic Styles ───────────────────────────────────────────────────────────

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
    history.execute(Command::Batch(commands), &mut doc);
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
    history.execute(Command::Batch(commands), &mut doc);
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
        history.execute(Command::Batch(commands), &mut doc);
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
    history.execute(
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
    history.execute(
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

/// Return a compact spatial overview of all visible nodes: bounding boxes and fill colors.
/// Useful for AI agents to understand document layout without loading the full document state.
pub async fn get_canvas_overview(state: &AppState, args: GetCanvasOverviewArgs) -> ToolResult {
    tracing::debug!("tool: get_canvas_overview");
    let doc = state.document.lock().await;

    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;

    let mut node_entries: Vec<serde_json::Value> = Vec::new();

    for node in doc.nodes_in_draw_order() {
        if !node.visible && !args.include_hidden {
            continue;
        }
        // World-space position origin
        let (wx, wy) = node.transform.apply(0.0, 0.0);

        // Approximate bounds using local_bounds() transformed by the node transform
        let (bx, by, bw, bh) = if let Some(lb) = node.local_bounds() {
            let (x0, y0) = node.transform.apply(lb.x0, lb.y0);
            let (x1, y1) = node.transform.apply(lb.x1, lb.y1);
            let nx = x0.min(x1);
            let ny = y0.min(y1);
            let nw = (x1 - x0).abs().max(1.0);
            let nh = (y1 - y0).abs().max(1.0);
            (nx, ny, nw, nh)
        } else {
            (wx, wy, 1.0, 1.0)
        };

        // Expand canvas bounds
        if bx < min_x {
            min_x = bx;
        }
        if by < min_y {
            min_y = by;
        }
        if bx + bw > max_x {
            max_x = bx + bw;
        }
        if by + bh > max_y {
            max_y = by + bh;
        }

        // Extract fill color as hex
        let fill_hex = match &node.kind {
            SceneNodeKind::Path(pn) => match &pn.fill.kind {
                FillKind::Solid(c) => format!(
                    "#{:02X}{:02X}{:02X}",
                    (c.r * 255.0) as u8,
                    (c.g * 255.0) as u8,
                    (c.b * 255.0) as u8
                ),
                FillKind::Gradient(_) => "#gradient".to_string(),
                FillKind::FluidGradient(_) => "#fluid".to_string(),
                FillKind::MeshGradient(_) => "#mesh".to_string(),
                FillKind::None => "#none".to_string(),
            },
            SceneNodeKind::Text(tn) => match &tn.fill.kind {
                FillKind::Solid(c) => format!(
                    "#{:02X}{:02X}{:02X}",
                    (c.r * 255.0) as u8,
                    (c.g * 255.0) as u8,
                    (c.b * 255.0) as u8
                ),
                _ => "#000000".to_string(),
            },
            SceneNodeKind::Group(_) => "#group".to_string(),
        };

        let layer_name = doc
            .layers
            .get(&node.layer_id)
            .map(|l| l.name.as_str())
            .unwrap_or("?");

        node_entries.push(json!({
            "id": node.id,
            "name": node.name,
            "layer": layer_name,
            "visible": node.visible,
            "kind": match &node.kind {
                SceneNodeKind::Path(_) => "path",
                SceneNodeKind::Text(_) => "text",
                SceneNodeKind::Group(_) => "group",
            },
            "bounds": { "x": bx, "y": by, "w": bw, "h": bh },
            "fill_hex": fill_hex,
        }));
    }

    // If no nodes, use defaults
    if min_x == f64::MAX {
        min_x = 0.0;
        min_y = 0.0;
        max_x = 800.0;
        max_y = 600.0;
    }

    ToolResult::text(format!(
        "{} node(s) in canvas overview.",
        node_entries.len()
    ))
    .with_data(json!({
        "node_count": node_entries.len(),
        "canvas_bounds": {
            "x": min_x, "y": min_y,
            "w": (max_x - min_x).max(1.0),
            "h": (max_y - min_y).max(1.0)
        },
        "nodes": node_entries,
    }))
}

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
        history.execute(cmd, &mut doc);
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

/// Analyze the composition of the current document and return advisory findings.
pub async fn analyze_composition(state: &AppState, args: AnalyzeCompositionArgs) -> ToolResult {
    tracing::debug!("tool: analyze_composition");
    let doc = state.document.lock().await;

    // Collect node bounds in world space
    struct NodeInfo {
        cx: f64,
        cy: f64,
        bx: f64,
        by: f64,
        bw: f64,
        bh: f64,
        fill_r: f32,
        fill_g: f32,
        fill_b: f32,
        has_solid_fill: bool,
    }

    let filter_ids: Option<std::collections::HashSet<uuid::Uuid>> = if args.node_ids.is_empty() {
        None
    } else {
        Some(
            args.node_ids
                .iter()
                .filter_map(|id| {
                    uuid::Uuid::parse_str(id)
                        .ok()
                        .or_else(|| doc.find_node_by_name(id).map(|n| n.id))
                })
                .collect(),
        )
    };

    let mut infos: Vec<NodeInfo> = Vec::new();
    let canvas_w = doc.width as f64;
    let canvas_h = doc.height as f64;

    for node in doc.nodes_in_draw_order() {
        if !node.visible {
            continue;
        }
        if let Some(ref ids) = filter_ids {
            if !ids.contains(&node.id) {
                continue;
            }
        }
        let (wx, wy) = node.transform.apply(0.0, 0.0);
        let (bx, by, bw, bh) = if let Some(lb) = node.local_bounds() {
            let (x0, y0) = node.transform.apply(lb.x0, lb.y0);
            let (x1, y1) = node.transform.apply(lb.x1, lb.y1);
            let nx = x0.min(x1);
            let ny = y0.min(y1);
            let nw = (x1 - x0).abs().max(1.0);
            let nh = (y1 - y0).abs().max(1.0);
            (nx, ny, nw, nh)
        } else {
            (wx, wy, 1.0, 1.0)
        };
        let (fill_r, fill_g, fill_b, has_solid) = match &node.kind {
            SceneNodeKind::Path(pn) => match &pn.fill.kind {
                FillKind::Solid(c) => (c.r, c.g, c.b, true),
                _ => (0.5, 0.5, 0.5, false),
            },
            SceneNodeKind::Text(tn) => match &tn.fill.kind {
                FillKind::Solid(c) => (c.r, c.g, c.b, true),
                _ => (0.0, 0.0, 0.0, true),
            },
            SceneNodeKind::Group(_) => (0.5, 0.5, 0.5, false),
        };
        infos.push(NodeInfo {
            cx: bx + bw / 2.0,
            cy: by + bh / 2.0,
            bx,
            by,
            bw,
            bh,
            fill_r,
            fill_g,
            fill_b,
            has_solid_fill: has_solid,
        });
    }

    let mut findings: Vec<serde_json::Value> = Vec::new();

    if infos.is_empty() {
        return ToolResult::text("No visible nodes to analyze.")
            .with_data(json!({ "node_count": 0, "findings": [] }));
    }

    let node_count = infos.len();

    // ── Balance: quadrant distribution ──────────────────────────────────────
    let mid_x = canvas_w / 2.0;
    let mid_y = canvas_h / 2.0;
    let (mut q_tl, mut q_tr, mut q_bl, mut q_br) = (0usize, 0usize, 0usize, 0usize);
    for n in &infos {
        match (n.cx < mid_x, n.cy < mid_y) {
            (true, true) => q_tl += 1,
            (false, true) => q_tr += 1,
            (true, false) => q_bl += 1,
            (false, false) => q_br += 1,
        }
    }
    let left = q_tl + q_bl;
    let right = q_tr + q_br;
    let top = q_tl + q_tr;
    let bottom = q_bl + q_br;
    let h_imbalance = if left + right > 0 {
        ((left as f64 - right as f64).abs() / (left + right) as f64 * 100.0) as u32
    } else {
        0
    };
    let v_imbalance = if top + bottom > 0 {
        ((top as f64 - bottom as f64).abs() / (top + bottom) as f64 * 100.0) as u32
    } else {
        0
    };
    if h_imbalance > 40 {
        let side = if left > right { "left" } else { "right" };
        findings.push(json!({
            "severity": "warning",
            "category": "balance",
            "description": format!(
                "Horizontal imbalance: {}% more objects on the {} side ({} left, {} right). Consider redistributing elements or adding counterweight.",
                h_imbalance, side, left, right
            )
        }));
    }
    if v_imbalance > 40 {
        let side = if top > bottom { "top" } else { "bottom" };
        findings.push(json!({
            "severity": "info",
            "category": "balance",
            "description": format!(
                "Vertical imbalance: {}% more objects near the {} ({} top half, {} bottom half).",
                v_imbalance, side, top, bottom
            )
        }));
    }
    if h_imbalance <= 20 && v_imbalance <= 20 {
        findings.push(json!({
            "severity": "ok",
            "category": "balance",
            "description": "Visual balance is good — objects are distributed evenly across quadrants."
        }));
    }

    // ── Density: canvas utilization ──────────────────────────────────────────
    let total_area: f64 = infos.iter().map(|n| n.bw * n.bh).sum();
    let canvas_area = (canvas_w * canvas_h).max(1.0);
    let density_pct = (total_area / canvas_area * 100.0).min(200.0);
    if density_pct < 5.0 {
        findings.push(json!({
            "severity": "info",
            "category": "density",
            "description": format!(
                "Canvas is very sparse ({:.1}% coverage). Objects occupy less than 5% of the canvas area.",
                density_pct
            )
        }));
    } else if density_pct > 120.0 {
        findings.push(json!({
            "severity": "warning",
            "category": "density",
            "description": format!(
                "Canvas may be overcrowded ({:.1}% combined bounding-box coverage). Some objects likely overlap significantly.",
                density_pct
            )
        }));
    }

    // ── Overlap detection ────────────────────────────────────────────────────
    let mut overlap_count = 0usize;
    for i in 0..infos.len() {
        for j in (i + 1)..infos.len() {
            let a = &infos[i];
            let b = &infos[j];
            let overlap = a.bx < b.bx + b.bw
                && a.bx + a.bw > b.bx
                && a.by < b.by + b.bh
                && a.by + a.bh > b.by;
            if overlap {
                overlap_count += 1;
            }
            if overlap_count >= 10 {
                break;
            }
        }
        if overlap_count >= 10 {
            break;
        }
    }
    if overlap_count > 0 {
        findings.push(json!({
            "severity": "info",
            "category": "overlap",
            "description": format!(
                "At least {} overlapping object pair(s) detected. This may be intentional (layering) or accidental — use distribute_no_overlap if unintended.",
                overlap_count
            )
        }));
    }

    // ── Color contrast ───────────────────────────────────────────────────────
    // Check pairs of solid-filled nodes for very similar colors
    let solid_nodes: Vec<_> = infos.iter().filter(|n| n.has_solid_fill).collect();
    let mut low_contrast_pairs = 0usize;
    'outer: for i in 0..solid_nodes.len() {
        for j in (i + 1)..solid_nodes.len() {
            let a = solid_nodes[i];
            let b = solid_nodes[j];
            let dr = (a.fill_r - b.fill_r).abs();
            let dg = (a.fill_g - b.fill_g).abs();
            let db = (a.fill_b - b.fill_b).abs();
            let delta = (dr * dr + dg * dg + db * db).sqrt();
            if delta < 0.1 {
                low_contrast_pairs += 1;
                if low_contrast_pairs >= 5 {
                    break 'outer;
                }
            }
        }
    }
    if low_contrast_pairs > 0 {
        findings.push(json!({
            "severity": "info",
            "category": "color_contrast",
            "description": format!(
                "{} pair(s) of objects with nearly identical fill colors detected. Objects may be hard to distinguish visually.",
                low_contrast_pairs
            )
        }));
    }

    // ── Unique colors (palette complexity) ──────────────────────────────────
    let unique_colors: std::collections::HashSet<(u8, u8, u8)> = solid_nodes
        .iter()
        .map(|n| {
            (
                (n.fill_r * 255.0) as u8,
                (n.fill_g * 255.0) as u8,
                (n.fill_b * 255.0) as u8,
            )
        })
        .collect();
    if unique_colors.len() > 12 {
        findings.push(json!({
            "severity": "info",
            "category": "color_palette",
            "description": format!(
                "{} unique fill colors in use. Consider reducing to a tighter palette (typically ≤ 5–7 colors) for visual cohesion.",
                unique_colors.len()
            )
        }));
    }

    // ── Off-canvas objects ───────────────────────────────────────────────────
    let off_canvas = infos
        .iter()
        .filter(|n| n.bx + n.bw < 0.0 || n.by + n.bh < 0.0 || n.bx > canvas_w || n.by > canvas_h)
        .count();
    if off_canvas > 0 {
        findings.push(json!({
            "severity": "warning",
            "category": "off_canvas",
            "description": format!(
                "{} object(s) are fully outside the canvas bounds and will not appear in exports.",
                off_canvas
            )
        }));
    }

    let summary = if findings.iter().any(|f| f["severity"] == "warning") {
        format!(
            "Analyzed {} node(s) — {} finding(s), some need attention.",
            node_count,
            findings.len()
        )
    } else {
        format!(
            "Analyzed {} node(s) — {} finding(s).",
            node_count,
            findings.len()
        )
    };

    ToolResult::text(summary).with_data(json!({
        "node_count": node_count,
        "quadrant_distribution": { "top_left": q_tl, "top_right": q_tr, "bottom_left": q_bl, "bottom_right": q_br },
        "canvas_coverage_pct": (density_pct * 10.0).round() / 10.0,
        "unique_fill_colors": unique_colors.len(),
        "findings": findings,
    }))
}

/// Detect visual rhythms (spacing, size, rotation patterns) in the document.
pub async fn detect_rhythms(state: &AppState, args: DetectRhythmsArgs) -> ToolResult {
    tracing::debug!("tool: detect_rhythms");
    let doc = state.document.lock().await;
    let min_count = args.min_count.unwrap_or(3).max(2);

    let filter_ids: Option<std::collections::HashSet<uuid::Uuid>> = if args.node_ids.is_empty() {
        None
    } else {
        Some(
            args.node_ids
                .iter()
                .filter_map(|id| {
                    uuid::Uuid::parse_str(id)
                        .ok()
                        .or_else(|| doc.find_node_by_name(id).map(|n| n.id))
                })
                .collect(),
        )
    };

    struct NodeMetrics {
        cx: f64,
        cy: f64,
        w: f64,
        area: f64,
        rotation_deg: f64,
    }

    let mut metrics: Vec<NodeMetrics> = Vec::new();

    for node in doc.nodes_in_draw_order() {
        if !node.visible {
            continue;
        }
        if let Some(ref ids) = filter_ids {
            if !ids.contains(&node.id) {
                continue;
            }
        }
        // Skip groups for cleaner analysis
        if matches!(node.kind, SceneNodeKind::Group(_)) {
            continue;
        }

        let (bx, by, bw, bh) = if let Some(lb) = node.local_bounds() {
            let (x0, y0) = node.transform.apply(lb.x0, lb.y0);
            let (x1, y1) = node.transform.apply(lb.x1, lb.y1);
            let nx = x0.min(x1);
            let ny = y0.min(y1);
            let nw = (x1 - x0).abs().max(0.001);
            let nh = (y1 - y0).abs().max(0.001);
            (nx, ny, nw, nh)
        } else {
            let (wx, wy) = node.transform.apply(0.0, 0.0);
            (wx, wy, 1.0, 1.0)
        };

        // Extract rotation from affine matrix [a, b, c, d, tx, ty]: angle = atan2(b, a)
        let rotation_deg = {
            let r = node.transform.matrix[1]
                .atan2(node.transform.matrix[0])
                .to_degrees()
                % 360.0;
            if r < 0.0 {
                r + 360.0
            } else {
                r
            }
        };

        metrics.push(NodeMetrics {
            cx: bx + bw / 2.0,
            cy: by + bh / 2.0,
            w: bw,
            area: bw * bh,
            rotation_deg,
        });
    }

    if metrics.len() < min_count {
        return ToolResult::text(format!(
            "Only {} visible leaf node(s) found — need at least {} to detect rhythms.",
            metrics.len(),
            min_count
        ))
        .with_data(json!({ "node_count": metrics.len(), "patterns": [] }));
    }

    let mut patterns: Vec<serde_json::Value> = Vec::new();
    let tolerance = 4.0_f64; // px tolerance for spacing/size grouping

    // ── Horizontal spacing rhythm ─────────────────────────────────────────────
    {
        let mut xs: Vec<f64> = metrics.iter().map(|m| m.cx).collect();
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mut gaps: Vec<f64> = xs.windows(2).map(|w| w[1] - w[0]).collect();
        gaps.sort_by(|a, b| a.partial_cmp(b).unwrap());

        // Find dominant gap interval
        let mut best_interval = 0.0_f64;
        let mut best_count = 0usize;
        for &gap in &gaps {
            if gap < 1.0 {
                continue;
            }
            let count = gaps
                .iter()
                .filter(|&&g| (g - gap).abs() < tolerance)
                .count();
            if count > best_count {
                best_count = count;
                best_interval = gap;
            }
        }
        if best_count >= min_count - 1 {
            patterns.push(json!({
                "type": "horizontal_spacing",
                "interval_px": (best_interval * 10.0).round() / 10.0,
                "count": best_count + 1,
                "description": format!(
                    "{} objects are spaced ~{:.0}px apart horizontally. Extend the pattern or enforce uniform spacing.",
                    best_count + 1, best_interval
                )
            }));
        }
    }

    // ── Vertical spacing rhythm ───────────────────────────────────────────────
    {
        let mut ys: Vec<f64> = metrics.iter().map(|m| m.cy).collect();
        ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mut gaps: Vec<f64> = ys.windows(2).map(|w| w[1] - w[0]).collect();
        gaps.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let mut best_interval = 0.0_f64;
        let mut best_count = 0usize;
        for &gap in &gaps {
            if gap < 1.0 {
                continue;
            }
            let count = gaps
                .iter()
                .filter(|&&g| (g - gap).abs() < tolerance)
                .count();
            if count > best_count {
                best_count = count;
                best_interval = gap;
            }
        }
        if best_count >= min_count - 1 {
            patterns.push(json!({
                "type": "vertical_spacing",
                "interval_px": (best_interval * 10.0).round() / 10.0,
                "count": best_count + 1,
                "description": format!(
                    "{} objects are spaced ~{:.0}px apart vertically. Extend the pattern or enforce uniform spacing.",
                    best_count + 1, best_interval
                )
            }));
        }
    }

    // ── Width rhythm ─────────────────────────────────────────────────────────
    {
        let mut widths: Vec<f64> = metrics.iter().map(|m| m.w).collect();
        widths.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let mut best_w = 0.0_f64;
        let mut best_count = 0usize;
        for &w in &widths {
            if w < 1.0 {
                continue;
            }
            let count = widths
                .iter()
                .filter(|&&x| (x - w).abs() < tolerance)
                .count();
            if count > best_count {
                best_count = count;
                best_w = w;
            }
        }
        if best_count >= min_count {
            patterns.push(json!({
                "type": "uniform_width",
                "width_px": (best_w * 10.0).round() / 10.0,
                "count": best_count,
                "description": format!(
                    "{} objects share a width of ~{:.0}px. Consider whether the remaining objects should match.",
                    best_count, best_w
                )
            }));
        }
    }

    // ── Size scaling rhythm (geometric progression) ───────────────────────────
    {
        let mut areas: Vec<f64> = metrics.iter().map(|m| m.area).collect();
        areas.sort_by(|a, b| a.partial_cmp(b).unwrap());

        if areas.len() >= min_count {
            // Look for geometric ratio between consecutive areas
            let ratios: Vec<f64> = areas
                .windows(2)
                .filter(|w| w[0] > 0.0)
                .map(|w| w[1] / w[0])
                .collect();

            let mut best_ratio = 1.0_f64;
            let mut best_count = 0usize;
            let ratio_tol = 0.15;
            for &r in &ratios {
                if (r - 1.0).abs() < 0.05 {
                    continue;
                } // skip near-equal
                let count = ratios
                    .iter()
                    .filter(|&&x| (x - r).abs() < ratio_tol)
                    .count();
                if count > best_count {
                    best_count = count;
                    best_ratio = r;
                }
            }
            if best_count >= min_count - 1 && (best_ratio - 1.0).abs() > 0.1 {
                let trend = if best_ratio > 1.0 {
                    "increasing"
                } else {
                    "decreasing"
                };
                patterns.push(json!({
                    "type": "size_progression",
                    "ratio": (best_ratio * 100.0).round() / 100.0,
                    "count": best_count + 1,
                    "description": format!(
                        "{} objects have {} sizes with a ~{:.0}% scale factor per step. Extend or enforce this progression.",
                        best_count + 1, trend, ((best_ratio - 1.0).abs() * 100.0).round()
                    )
                }));
            }
        }
    }

    // ── Rotation rhythm ───────────────────────────────────────────────────────
    {
        let mut rots: Vec<f64> = metrics.iter().map(|m| m.rotation_deg).collect();
        rots.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let rot_tol = 3.0_f64;

        let mut rot_gaps: Vec<f64> = rots.windows(2).map(|w| w[1] - w[0]).collect();
        rot_gaps.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let mut best_interval = 0.0_f64;
        let mut best_count = 0usize;
        for &gap in &rot_gaps {
            if gap < 1.0 {
                continue;
            }
            let count = rot_gaps
                .iter()
                .filter(|&&g| (g - gap).abs() < rot_tol)
                .count();
            if count > best_count {
                best_count = count;
                best_interval = gap;
            }
        }
        if best_count >= min_count - 1 && best_interval >= 5.0 {
            let symmetry_n = (360.0 / best_interval).round() as u32;
            let sym_note = if symmetry_n >= 2 && symmetry_n <= 12 {
                format!(" ({}× rotational symmetry)", symmetry_n)
            } else {
                String::new()
            };
            patterns.push(json!({
                "type": "rotation_rhythm",
                "interval_deg": (best_interval * 10.0).round() / 10.0,
                "count": best_count + 1,
                "description": format!(
                    "{} objects are rotated ~{:.0}° apart{sym_note}. Add missing rotations or flatten to a full symmetry group.",
                    best_count + 1, best_interval
                )
            }));
        }
    }

    let summary = if patterns.is_empty() {
        format!(
            "Analyzed {} node(s) — no repeating rhythms detected.",
            metrics.len()
        )
    } else {
        format!(
            "Analyzed {} node(s) — {} rhythm pattern(s) detected.",
            metrics.len(),
            patterns.len()
        )
    };

    ToolResult::text(summary).with_data(json!({
        "node_count": metrics.len(),
        "patterns": patterns,
    }))
}

/// Measure edge-to-edge gaps, center-to-center distances, and alignment between nodes.
pub async fn measure_distances(state: &AppState, args: MeasureDistancesArgs) -> ToolResult {
    tracing::debug!("tool: measure_distances");
    if args.node_ids.len() < 2 {
        return ToolResult::error("At least 2 node_ids are required for distance measurement.");
    }

    let doc = state.document.lock().await;

    struct NodeBox {
        name: String,
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
    }

    let mut boxes: Vec<NodeBox> = Vec::new();
    for id_str in &args.node_ids {
        let uid = uuid::Uuid::parse_str(id_str)
            .ok()
            .or_else(|| doc.find_node_by_name(id_str).map(|n| n.id));
        let node = uid.and_then(|uid| doc.nodes.get(&uid));
        if let Some(node) = node {
            let (bx, by, bw, bh) = if let Some(lb) = node.local_bounds() {
                let (x0, y0) = node.transform.apply(lb.x0, lb.y0);
                let (x1, y1) = node.transform.apply(lb.x1, lb.y1);
                let nx: f64 = x0.min(x1);
                let ny: f64 = y0.min(y1);
                let nw = (x1 - x0).abs().max(0.0);
                let nh = (y1 - y0).abs().max(0.0);
                (nx, ny, nw, nh)
            } else {
                let (wx, wy) = node.transform.apply(0.0, 0.0);
                (wx, wy, 0.0_f64, 0.0_f64)
            };
            boxes.push(NodeBox {
                name: if node.name.is_empty() {
                    id_str.clone()
                } else {
                    node.name.clone()
                },
                x0: bx,
                y0: by,
                x1: bx + bw,
                y1: by + bh,
            });
        } else {
            return ToolResult::error(format!("Node '{}' not found.", id_str));
        }
    }

    let mut measurements: Vec<serde_json::Value> = Vec::new();

    // Measure every pair (i, i+1) in the provided order, plus all combinations if ≤ 6 nodes
    let n = boxes.len();
    let pairs: Vec<(usize, usize)> = if n <= 6 {
        let mut p = Vec::new();
        for i in 0..n {
            for j in (i + 1)..n {
                p.push((i, j));
            }
        }
        p
    } else {
        (0..n - 1).map(|i| (i, i + 1)).collect()
    };

    for (i, j) in pairs {
        let a = &boxes[i];
        let b = &boxes[j];

        // Center-to-center
        let acx = (a.x0 + a.x1) / 2.0;
        let acy = (a.y0 + a.y1) / 2.0;
        let bcx = (b.x0 + b.x1) / 2.0;
        let bcy = (b.y0 + b.y1) / 2.0;
        let center_dist = ((bcx - acx).powi(2) + (bcy - acy).powi(2)).sqrt();

        // Edge-to-edge horizontal gap
        let h_gap = if a.x1 <= b.x0 {
            b.x0 - a.x1 // a is left of b
        } else if b.x1 <= a.x0 {
            b.x1 - a.x0 // b is left of a (negative means overlap)
        } else {
            // Overlapping horizontally
            let overlap = a.x1.min(b.x1) - a.x0.max(b.x0);
            -overlap
        };

        // Edge-to-edge vertical gap
        let v_gap = if a.y1 <= b.y0 {
            b.y0 - a.y1
        } else if b.y1 <= a.y0 {
            b.y1 - a.y0
        } else {
            let overlap = a.y1.min(b.y1) - a.y0.max(b.y0);
            -overlap
        };

        // Alignment offsets
        let h_align_offset = (acy - bcy).abs(); // how misaligned vertically (for horizontal layout)
        let v_align_offset = (acx - bcx).abs(); // how misaligned horizontally (for vertical layout)

        measurements.push(json!({
            "from": a.name,
            "to": b.name,
            "center_to_center_px": (center_dist * 10.0).round() / 10.0,
            "horizontal_gap_px": (h_gap * 10.0).round() / 10.0,
            "vertical_gap_px": (v_gap * 10.0).round() / 10.0,
            "horizontal_alignment_offset_px": (h_align_offset * 10.0).round() / 10.0,
            "vertical_alignment_offset_px": (v_align_offset * 10.0).round() / 10.0,
            "overlapping": h_gap < 0.0 && v_gap < 0.0,
        }));
    }

    ToolResult::text(format!("Measured {} pair(s).", measurements.len()))
        .with_data(json!({ "measurements": measurements }))
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
        history.execute(cmd, &mut doc);
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

/// Add an angled construction line (infinite guide) through a point at any angle.
pub async fn add_construction_line(state: &AppState, args: AddConstructionLineArgs) -> ToolResult {
    tracing::debug!("tool: add_construction_line");
    use photonic_core::document::{Guide, GuideOrientation};

    let color = if let Some(hex) = &args.color {
        let h = hex.trim_start_matches('#');
        if h.len() >= 6 {
            let r = u8::from_str_radix(&h[0..2], 16).unwrap_or(255) as f32 / 255.0;
            let g = u8::from_str_radix(&h[2..4], 16).unwrap_or(128) as f32 / 255.0;
            let b = u8::from_str_radix(&h[4..6], 16).unwrap_or(0) as f32 / 255.0;
            let a = if h.len() >= 8 {
                u8::from_str_radix(&h[6..8], 16).unwrap_or(255) as f32 / 255.0
            } else {
                0.85
            };
            Some([r, g, b, a])
        } else {
            Some([1.0, 0.5, 0.0, 0.85])
        } // orange default
    } else {
        Some([1.0, 0.5, 0.0, 0.85])
    };

    let mut guide = Guide::new(GuideOrientation::Horizontal, 0.0);
    guide.color = color;
    guide.angle_degrees = Some(args.angle_degrees);
    guide.position_x = args.x;
    guide.position_y = args.y;

    let id = guide.id;
    let mut doc = state.document.lock().await;
    doc.guides.push(guide);

    ToolResult::text(format!(
        "Added construction line at ({:.1}, {:.1}) angle={:.1}°.",
        args.x, args.y, args.angle_degrees
    ))
    .with_data(json!({ "id": id.to_string(), "x": args.x, "y": args.y, "angle_degrees": args.angle_degrees }))
}

/// Set the document bleed and/or slug margins for print production.
pub async fn set_document_bleed(state: &AppState, args: SetDocumentBleedArgs) -> ToolResult {
    tracing::debug!("tool: set_document_bleed");
    let mut doc = state.document.lock().await;

    if let Some(b) = args.bleed_mm {
        if b < 0.0 {
            return ToolResult::error("bleed_mm must be >= 0.");
        }
        doc.bleed_mm = b;
    }
    if let Some(s) = args.slug_mm {
        if s < 0.0 {
            return ToolResult::error("slug_mm must be >= 0.");
        }
        doc.slug_mm = s;
    }

    ToolResult::text(format!(
        "Document print settings: bleed={:.3} mm, slug={:.3} mm.",
        doc.bleed_mm, doc.slug_mm
    ))
    .with_data(json!({ "bleed_mm": doc.bleed_mm, "slug_mm": doc.slug_mm }))
}

/// Return the current document bleed and slug values.
pub async fn get_document_bleed(state: &AppState) -> ToolResult {
    tracing::debug!("tool: get_document_bleed");
    let doc = state.document.lock().await;
    ToolResult::text(format!(
        "Bleed: {:.3} mm, Slug: {:.3} mm.",
        doc.bleed_mm, doc.slug_mm
    ))
    .with_data(json!({ "bleed_mm": doc.bleed_mm, "slug_mm": doc.slug_mm }))
}

/// Set the artboard safe-area margins (top/right/bottom/left in document units).
pub async fn set_artboard_margins(state: &AppState, args: SetArtboardMarginsArgs) -> ToolResult {
    tracing::debug!("tool: set_artboard_margins");
    let mut doc = state.document.lock().await;

    if let Some(v) = args.top {
        if v < 0.0 {
            return ToolResult::error("top margin must be >= 0");
        }
        doc.margin_top = v;
    }
    if let Some(v) = args.right {
        if v < 0.0 {
            return ToolResult::error("right margin must be >= 0");
        }
        doc.margin_right = v;
    }
    if let Some(v) = args.bottom {
        if v < 0.0 {
            return ToolResult::error("bottom margin must be >= 0");
        }
        doc.margin_bottom = v;
    }
    if let Some(v) = args.left {
        if v < 0.0 {
            return ToolResult::error("left margin must be >= 0");
        }
        doc.margin_left = v;
    }

    ToolResult::text(format!(
        "Artboard margins set — top: {:.1}, right: {:.1}, bottom: {:.1}, left: {:.1}.",
        doc.margin_top, doc.margin_right, doc.margin_bottom, doc.margin_left
    ))
    .with_data(json!({
        "top": doc.margin_top, "right": doc.margin_right,
        "bottom": doc.margin_bottom, "left": doc.margin_left
    }))
}

/// Return the current artboard safe-area margin values.
pub async fn get_artboard_margins(state: &AppState) -> ToolResult {
    tracing::debug!("tool: get_artboard_margins");
    let doc = state.document.lock().await;
    ToolResult::text(format!(
        "Artboard margins — top: {:.1}, right: {:.1}, bottom: {:.1}, left: {:.1}.",
        doc.margin_top, doc.margin_right, doc.margin_bottom, doc.margin_left
    ))
    .with_data(json!({
        "top": doc.margin_top, "right": doc.margin_right,
        "bottom": doc.margin_bottom, "left": doc.margin_left
    }))
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

/// Return the most recent edit history entries from the undo stack.
pub async fn list_history(state: &AppState, args: ListHistoryArgs) -> ToolResult {
    tracing::debug!("tool: list_history");
    let limit = args.limit.unwrap_or(20).min(200);
    let history = state.history.lock().await;
    let entries = history.history_entries(limit);
    let total = history.undo_depth();
    drop(history);

    let items: Vec<serde_json::Value> = entries
        .iter()
        .map(|(step, desc)| json!({ "step": step, "description": desc }))
        .collect();

    let summary = if items.is_empty() {
        "No edit history — document hasn't been modified yet.".to_string()
    } else {
        format!("Last {} of {} total edit(s):", items.len(), total)
    };

    ToolResult::text(summary)
        .with_data(json!({ "total": total, "returned": items.len(), "entries": items }))
}

/// Jump to a specific position in the undo/redo history.
/// index=0 is the empty-document state; index=undo_depth() is the current state.
pub async fn jump_to_history(state: &AppState, args: JumpToHistoryArgs) -> ToolResult {
    tracing::debug!("tool: jump_to_history index={}", args.index);
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    let current = history.undo_depth();
    let max_index = current + history.redo_depth();
    let target = args.index.min(max_index);

    if target == current {
        return ToolResult::text(format!("Already at history index {} (no change).", current))
            .with_data(serde_json::json!({ "index": current, "total": max_index, "moved": 0 }));
    }

    let mut moved: isize = 0;
    if target < current {
        // Undo (current - target) times
        let steps = current - target;
        for _ in 0..steps {
            if !history.undo(&mut doc) {
                break;
            }
            moved -= 1;
        }
    } else {
        // Redo (target - current) times
        let steps = target - current;
        for _ in 0..steps {
            if !history.redo(&mut doc) {
                break;
            }
            moved += 1;
        }
    }

    let new_depth = history.undo_depth();
    ToolResult::text(format!(
        "Jumped from index {} to {} ({:+} step(s)).",
        current, new_depth, moved
    ))
    .with_data(serde_json::json!({
        "from": current,
        "to": new_depth,
        "moved": moved,
        "total": max_index,
    }))
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
        history.execute(
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
        history.execute(
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

/// Scale and position nodes to fill the artboard safe area (artboard minus margins).
pub async fn fit_to_margins(state: &AppState, args: FitToMarginsArgs) -> ToolResult {
    use photonic_core::history::Command;
    tracing::debug!("tool: fit_to_margins");

    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;

    // Compute the safe area
    let safe_x = doc.margin_left + args.padding;
    let safe_y = doc.margin_top + args.padding;
    let safe_w = doc.width - doc.margin_left - doc.margin_right - args.padding * 2.0;
    let safe_h = doc.height - doc.margin_top - doc.margin_bottom - args.padding * 2.0;

    if safe_w <= 0.0 || safe_h <= 0.0 {
        return ToolResult::error("Margins + padding exceed artboard size; safe area is empty.");
    }

    // Collect target node IDs
    let target_ids: Vec<photonic_core::node::NodeId> = if args.node_ids.is_empty() {
        doc.nodes.keys().copied().collect()
    } else {
        args.node_ids
            .iter()
            .filter_map(|id_str| {
                uuid::Uuid::parse_str(id_str)
                    .ok()
                    .or_else(|| doc.find_node_by_name(id_str).map(|n| n.id))
            })
            .collect()
    };

    if target_ids.is_empty() {
        return ToolResult::error("No target nodes found.");
    }

    // Compute the union bounding box of all targets
    let mut union_x0 = f64::MAX;
    let mut union_y0 = f64::MAX;
    let mut union_x1 = f64::MIN;
    let mut union_y1 = f64::MIN;
    let mut valid_ids: Vec<photonic_core::node::NodeId> = Vec::new();

    for nid in &target_ids {
        if let Some(node) = doc.nodes.get(nid) {
            if let Some(lb) = node.local_bounds() {
                let (x0, y0) = node.transform.apply(lb.x0, lb.y0);
                let (x1, y1) = node.transform.apply(lb.x1, lb.y1);
                let (nx0, ny0, nx1, ny1) = (x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1));
                union_x0 = union_x0.min(nx0);
                union_y0 = union_y0.min(ny0);
                union_x1 = union_x1.max(nx1);
                union_y1 = union_y1.max(ny1);
                valid_ids.push(*nid);
            }
        }
    }

    if valid_ids.is_empty() || union_x0 >= union_x1 || union_y0 >= union_y1 {
        return ToolResult::error("No nodes with valid bounds found.");
    }

    let content_w = union_x1 - union_x0;
    let content_h = union_y1 - union_y0;

    // Compute scale factor
    let scale = if args.uniform {
        (safe_w / content_w).min(safe_h / content_h)
    } else {
        1.0 // non-uniform handled per-axis below
    };

    let scale_x = if args.uniform {
        scale
    } else {
        safe_w / content_w
    };
    let scale_y = if args.uniform {
        scale
    } else {
        safe_h / content_h
    };

    // Center the scaled content in the safe area
    let target_cx = safe_x + safe_w / 2.0;
    let target_cy = safe_y + safe_h / 2.0;

    let content_cx = (union_x0 + union_x1) / 2.0;
    let content_cy = (union_y0 + union_y1) / 2.0;

    let mut cmds: Vec<Command> = Vec::new();
    for nid in &valid_ids {
        if let Some(node) = doc.nodes.get(nid) {
            let tx = node.transform.matrix[4];
            let ty = node.transform.matrix[5];
            // New position: shift from content center to target center, apply scale
            let new_tx = target_cx + (tx - content_cx) * scale_x;
            let new_ty = target_cy + (ty - content_cy) * scale_y;
            let mut new_node = node.clone();
            new_node.transform.matrix[4] = new_tx;
            new_node.transform.matrix[5] = new_ty;
            // Scale the node (adjust the scale component of the transform matrix)
            new_node.transform.matrix[0] *= scale_x;
            new_node.transform.matrix[3] *= scale_y;
            cmds.push(Command::UpdateNode {
                old: node.clone(),
                new: new_node,
            });
        }
    }

    if cmds.is_empty() {
        return ToolResult::error("No changes to apply.");
    }

    let moved = cmds.len();
    history.execute(Command::Batch(cmds), &mut doc);

    ToolResult::text(format!(
        "Fitted {} node(s) to safe area ({:.1}×{:.1}) with scale ×{:.3}.",
        moved, safe_w, safe_h, scale_x
    ))
    .with_data(serde_json::json!({
        "nodes_fitted": moved,
        "safe_area": { "x": safe_x, "y": safe_y, "w": safe_w, "h": safe_h },
        "scale_x": (scale_x * 1000.0).round() / 1000.0,
        "scale_y": (scale_y * 1000.0).round() / 1000.0,
    }))
}

// ─── Dimension Annotations ────────────────────────────────────────────────────

fn node_center(doc: &photonic_core::Document, id_str: &str) -> Option<(f64, f64)> {
    let uid = uuid::Uuid::parse_str(id_str)
        .ok()
        .or_else(|| doc.find_node_by_name(id_str).map(|n| n.id))?;
    let node = doc.nodes.get(&uid)?;
    if let Some(lb) = node.local_bounds() {
        let (x0, y0) = node.transform.apply(lb.x0, lb.y0);
        let (x1, y1) = node.transform.apply(lb.x1, lb.y1);
        Some(((x0 + x1) / 2.0, (y0 + y1) / 2.0))
    } else {
        let (wx, wy) = node.transform.apply(0.0, 0.0);
        Some((wx, wy))
    }
}

/// Add a dimension annotation showing the distance between two nodes.
pub async fn add_dimension(state: &AppState, args: AddDimensionArgs) -> ToolResult {
    tracing::debug!("tool: add_dimension");

    let mut doc = state.document.lock().await;

    let (from_x, from_y) = match node_center(&doc, &args.from_node_id) {
        Some(c) => c,
        None => return ToolResult::error(format!("Node '{}' not found.", args.from_node_id)),
    };
    let (to_x, to_y) = match node_center(&doc, &args.to_node_id) {
        Some(c) => c,
        None => return ToolResult::error(format!("Node '{}' not found.", args.to_node_id)),
    };

    let from_uid = uuid::Uuid::parse_str(&args.from_node_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.from_node_id).map(|n| n.id))
        .unwrap();
    let to_uid = uuid::Uuid::parse_str(&args.to_node_id)
        .ok()
        .or_else(|| doc.find_node_by_name(&args.to_node_id).map(|n| n.id))
        .unwrap();

    let axis = args.axis.unwrap_or_else(|| "diagonal".to_string());
    let label_offset = args.label_offset.unwrap_or(20.0);

    let dim = photonic_core::DimensionAnnotation::new(
        from_uid,
        to_uid,
        axis.clone(),
        label_offset,
        from_x,
        from_y,
        to_x,
        to_y,
    );
    let distance = dim.distance();
    let dim_id = dim.id;
    doc.dimensions.push(dim);

    ToolResult::text(format!(
        "Added {} dimension: {:.1} units between nodes.",
        axis, distance
    ))
    .with_data(serde_json::json!({
        "id": dim_id.to_string(),
        "axis": axis,
        "distance": (distance * 10.0).round() / 10.0,
        "from": [from_x, from_y],
        "to": [to_x, to_y],
    }))
}

/// List all dimension annotations in the document.
pub async fn list_dimensions(state: &AppState) -> ToolResult {
    tracing::debug!("tool: list_dimensions");
    let doc = state.document.lock().await;

    let items: Vec<serde_json::Value> = doc
        .dimensions
        .iter()
        .map(|d| {
            serde_json::json!({
                "id": d.id.to_string(),
                "from_node": d.from_node.to_string(),
                "to_node": d.to_node.to_string(),
                "axis": d.axis,
                "distance": (d.distance() * 10.0).round() / 10.0,
                "label_offset": d.label_offset,
                "from": [d.from_x, d.from_y],
                "to": [d.to_x, d.to_y],
            })
        })
        .collect();

    let count = items.len();
    ToolResult::text(format!("{} dimension annotation(s).", count))
        .with_data(serde_json::json!({ "dimensions": items, "count": count }))
}

/// Remove a dimension annotation by ID.
pub async fn remove_dimension(state: &AppState, args: RemoveDimensionArgs) -> ToolResult {
    tracing::debug!("tool: remove_dimension id={}", args.id);
    let id = match uuid::Uuid::parse_str(&args.id) {
        Ok(id) => id,
        Err(_) => return ToolResult::error(format!("Invalid dimension ID: '{}'", args.id)),
    };
    let mut doc = state.document.lock().await;
    let before = doc.dimensions.len();
    doc.dimensions.retain(|d| d.id != id);
    let removed = before - doc.dimensions.len();
    if removed == 0 {
        ToolResult::error(format!("Dimension '{}' not found.", args.id))
    } else {
        ToolResult::text(format!("Removed dimension '{}'.", args.id))
    }
}
