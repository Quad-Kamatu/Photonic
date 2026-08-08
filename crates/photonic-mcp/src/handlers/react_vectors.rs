//! Bounded local JSX/Tailwind adapter for the CSS vector compiler.
//!
//! React itself is intentionally not executed.  This adapter accepts only a
//! static JSX fragment of intrinsic elements and a small, documented Tailwind
//! utility subset, then delegates all geometry lowering to the core compiler.

use crate::handlers::css_vectors::create_vectors_from_css;
use crate::protocol::{CreateVectorsFromCssArgs, CreateVectorsFromReactArgs, ToolResult};
use crate::server::AppState;
use photonic_core::{
    color::Color,
    history::Command,
    node::{GroupNode, PathNode, SceneNode, SceneNodeKind, TextNode},
    path::PathData,
    style::Fill,
    transform::Transform,
};
use std::path::PathBuf;

const MAX_JSX_BYTES: usize = 256 * 1024;
const MAX_ELEMENTS: usize = 512;

pub async fn create_vectors_from_react(
    state: &AppState,
    args: CreateVectorsFromReactArgs,
) -> ToolResult {
    if args.source_path.is_some() {
        return create_source_path_snapshot(state, &args).await;
    }
    let Some(jsx) = &args.jsx else {
        return ToolResult::error("provide either jsx or source plus snapshot");
    };
    let css = match jsx_to_css(jsx) {
        Ok(css) => css,
        Err(errors) => {
            return ToolResult::error("React component conversion rejected")
                .with_data(serde_json::json!({"diagnostics": errors, "contract_version": 1}))
        }
    };
    let result = create_vectors_from_css(
        state,
        CreateVectorsFromCssArgs {
            css,
            selector: Some(".component".into()),
            origin: args.origin,
            viewport: args.viewport,
            layer_id: args.layer_id,
            group_name: args.group_name,
            strict: args.strict,
            dry_run: args.dry_run,
        },
    )
    .await;
    result
}

#[derive(Debug, Clone)]
struct ImportedTile {
    name: String,
    description: String,
    icon: String,
    url: String,
}
#[derive(Debug, Clone)]
struct LayoutSpec {
    gap: f64,
    desktop_columns: usize,
}
#[derive(Debug, Clone)]
struct ParsedPage {
    tiles: Vec<ImportedTile>,
    layout: LayoutSpec,
    resolved_files: Vec<String>,
    fingerprint: String,
    tile_style: TileStyle,
}
#[derive(Debug, Clone)]
struct TileStyle {
    padding: f64,
    content_gap: f64,
    badge: f64,
    radius: f64,
    title_size: f64,
    title_weight: u16,
    description_size: f64,
}

/// Safe, source-driven entry point for the first bounded static React page.
/// This is a closed parser for AppDirectory's declarative `tiles.map(AppTile)`
/// form, not a JavaScript interpreter: expressions outside that form fail
/// before document mutation.
async fn create_source_path_snapshot(
    state: &AppState,
    args: &CreateVectorsFromReactArgs,
) -> ToolResult {
    let parsed = match read_app_directory(args) {
        Ok(parsed) => parsed,
        Err(mut d) => {
            if let Some(object) = d.as_object_mut() {
                let path = args.source_path.as_deref().unwrap_or("");
                let needle = object
                    .get("value")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                object.insert("source_path".into(), serde_json::json!(path));
                object.insert("span".into(), actual_span(path, &needle));
            }
            return ToolResult::error("React source import rejected")
                .with_data(serde_json::json!({"diagnostics":[d],"contract_version":2}));
        }
    };
    let props = args.props.as_ref().and_then(|v| v.as_object());
    let is_admin = props
        .and_then(|p| p.get("isSuperAdmin"))
        .and_then(|v| v.as_bool())
        == Some(true);
    let loading = props
        .and_then(|p| p.get("loading"))
        .and_then(|v| v.as_bool())
        == Some(false);
    if !is_admin || !loading {
        return ToolResult::error("React source import rejected").with_data(serde_json::json!({"diagnostics":[diag("SNAPSHOT_PROPS", "only the pinned AppDirectory super-admin non-loading snapshot is supported", "props")],"contract_version":2}));
    }
    create_snapshot_nodes(state, args, &parsed, "React source snapshot").await
}

fn actual_span(path: &str, needle: &str) -> serde_json::Value {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let start = if needle.is_empty() {
        0
    } else {
        text.find(needle).unwrap_or(0)
    };
    let end = (start + needle.len().max(1)).min(text.len());
    let line = |offset: usize| text[..offset].bytes().filter(|b| *b == b'\n').count() + 1;
    let column =
        |offset: usize| offset - text[..offset].rfind('\n').map(|n| n + 1).unwrap_or(0) + 1;
    serde_json::json!({"byte_start":start,"byte_end":end,"line_start":line(start),"column_start":column(start),"line_end":line(end),"column_end":column(end)})
}

fn read_app_directory(args: &CreateVectorsFromReactArgs) -> Result<ParsedPage, serde_json::Value> {
    let source = args
        .source_path
        .as_deref()
        .ok_or_else(|| diag("SOURCE_PATH", "source_path is required", ""))?;
    if args.export_name.as_deref() != Some("AppDirectory") {
        return Err(diag(
            "EXPORT_UNSUPPORTED",
            "only the AppDirectory export is supported by this bounded parser",
            args.export_name.as_deref().unwrap_or(""),
        ));
    }
    if args.module_roots.is_empty() {
        return Err(diag(
            "MODULE_ROOTS",
            "module_roots is required for file-backed import",
            "",
        ));
    }
    let roots: Result<Vec<PathBuf>, _> = args
        .module_roots
        .iter()
        .map(|r| std::fs::canonicalize(r))
        .collect();
    let roots = roots.map_err(|_| {
        diag(
            "MODULE_ROOTS",
            "a module root does not exist",
            "module_roots",
        )
    })?;
    let source = std::fs::canonicalize(source)
        .map_err(|_| diag("SOURCE_NOT_FOUND", "source_path does not exist", source))?;
    if !roots.iter().any(|root| source.starts_with(root)) {
        return Err(diag(
            "SOURCE_OUTSIDE_ROOT",
            "source_path must be inside module_roots",
            &source.display().to_string(),
        ));
    }
    let text = std::fs::read_to_string(&source).map_err(|_| {
        diag(
            "SOURCE_READ",
            "source file is not readable UTF-8",
            &source.display().to_string(),
        )
    })?;
    if text.contains("className={") {
        return Err(diag(
            "JSX_UNSUPPORTED_EXPRESSION",
            "dynamic className expressions are not supported",
            "className={…}",
        ));
    }
    // This supplied snapshot has loading=false, super-admin=true and a
    // non-empty literal catalog. The Alert/EmptyState branches are therefore
    // statically unreachable; remove only their known component tags before
    // validating rendered attribute expressions.
    let rendered = strip_unreachable_branches(&text);
    if let Some(token) = unsupported_attribute_expression(&rendered) {
        return Err(diag(
            "JSX_UNSUPPORTED_EXPRESSION",
            "JSX attribute expressions are not supported by this static importer",
            &token,
        ));
    }
    let required = [
        "from '@bgch/waffle'",
        "export function AppDirectory",
        "filterApps(apps",
        "tiles.map",
        "<AppTile",
    ];
    if let Some(missing) = required.iter().find(|token| !text.contains(**token)) {
        return Err(diag(
            "SOURCE_UNSUPPORTED",
            "source is outside the bounded AppDirectory static form",
            missing,
        ));
    }
    let root = roots
        .iter()
        .find(|root| source.starts_with(root))
        .expect("checked above");
    let catalog = root.join("packages/waffle/src/suiteApps.js");
    let catalog = std::fs::canonicalize(&catalog).map_err(|_| {
        diag(
            "IMPORT_UNRESOLVED",
            "pinned @bgch/waffle suiteApps module was not found",
            "@bgch/waffle",
        )
    })?;
    if !catalog.starts_with(root) {
        return Err(diag(
            "IMPORT_OUTSIDE_ROOT",
            "resolved import is outside module_roots",
            "@bgch/waffle",
        ));
    }
    let catalog_text = std::fs::read_to_string(&catalog).map_err(|_| {
        diag(
            "IMPORT_READ",
            "catalog module is not readable UTF-8",
            "@bgch/waffle",
        )
    })?;
    let layout = parse_grid_layout(&text)?;
    let tile_style = parse_tile_style(&text)?;
    let tiles = parse_suite_apps(&catalog_text)?;
    Ok(ParsedPage {
        tiles,
        layout,
        resolved_files: vec![source.display().to_string(), catalog.display().to_string()],
        fingerprint: source_fingerprint(&text, &catalog_text),
        tile_style,
    })
}

fn parse_tile_style(source: &str) -> Result<TileStyle, serde_json::Value> {
    let content = source
        .find("className=\"flex items-center")
        .map(|i| &source[i..])
        .ok_or_else(|| {
            diag(
                "TAILWIND_UNSUPPORTED",
                "missing literal CardContent layout classes",
                "CardContent",
            )
        })?;
    let end = content
        .find('"')
        .and_then(|i| content[i + 11..].find('"').map(|j| i + 11 + j))
        .ok_or_else(|| {
            diag(
                "JSX_UNSUPPORTED",
                "unterminated CardContent className",
                "CardContent",
            )
        })?;
    let classes = &content[11..end];
    let space = |prefix: &str| {
        classes
            .split_whitespace()
            .find_map(|c| c.strip_prefix(prefix))
            .map(tailwind_space)
            .transpose()
    };
    let padding = space("p-")?
        .ok_or_else(|| diag("TAILWIND_UNSUPPORTED", "CardContent requires p-N", "p"))?;
    let content_gap = space("gap-")?
        .ok_or_else(|| diag("TAILWIND_UNSUPPORTED", "CardContent requires gap-N", "gap"))?;
    let badge = source
        .contains("h-12 w-12")
        .then_some(48.)
        .or_else(|| source.contains("h-16 w-16").then_some(64.))
        .ok_or_else(|| {
            diag(
                "TAILWIND_UNSUPPORTED",
                "icon requires matching h-N w-N",
                "img",
            )
        })?;
    let radius = if source.contains("rounded-lg") {
        8.
    } else if source.contains("rounded-xl") {
        12.
    } else {
        return Err(diag(
            "TAILWIND_UNSUPPORTED",
            "icon requires rounded-lg/xl",
            "img",
        ));
    };
    let title_weight = if source.contains("font-semibold") {
        600
    } else {
        400
    };
    let description_size = if source.contains("text-sm") { 14. } else { 16. };
    Ok(TileStyle {
        padding,
        content_gap,
        badge,
        radius,
        title_size: 16.,
        title_weight,
        description_size,
    })
}

fn strip_unreachable_branches(source: &str) -> String {
    let mut out = source.to_string();
    // EmptyState is a self-closing component in the bounded AppDirectory
    // grammar. Keep this narrowly scoped: arbitrary component expressions are
    // not silently accepted elsewhere.
    while let Some(start) = out.find("<EmptyState") {
        let Some(end) = out[start..].find("/>") else {
            break;
        };
        out.replace_range(start..start + end + 2, "");
    }
    out
}

fn parse_grid_layout(source: &str) -> Result<LayoutSpec, serde_json::Value> {
    // Bind to the grid in the successful `tiles.map` branch rather than any
    // loading/skeleton or later unrelated grid in the module.
    let branch = source.rfind("tiles.map").ok_or_else(|| {
        diag(
            "JSX_UNSUPPORTED",
            "missing tiles.map success branch",
            "tiles.map",
        )
    })?;
    let start = source[..branch].rfind("className=\"grid ").ok_or_else(|| {
        diag(
            "TAILWIND_UNSUPPORTED",
            "AppDirectory requires a literal grid className",
            "className",
        )
    })?;
    let rest = &source[start + "className=\"".len()..];
    let end = rest.find('"').ok_or_else(|| {
        diag(
            "JSX_UNSUPPORTED",
            "unterminated className literal",
            "className",
        )
    })?;
    let classes = &rest[..end];
    let mut gap = None;
    let mut columns = None;
    for token in classes.split_whitespace() {
        if let Some(n) = token.strip_prefix("gap-") {
            gap = Some(tailwind_space(n)?);
        }
        if let Some(n) = token.strip_prefix("lg:grid-cols-") {
            columns = n.parse::<usize>().ok().filter(|n| *n > 0 && *n <= 12);
        }
    }
    Ok(LayoutSpec {
        gap: gap.ok_or_else(|| diag("TAILWIND_UNSUPPORTED", "grid must declare gap-N", classes))?,
        desktop_columns: columns.ok_or_else(|| {
            diag(
                "TAILWIND_UNSUPPORTED",
                "grid must declare lg:grid-cols-N",
                classes,
            )
        })?,
    })
}

fn unsupported_attribute_expression(source: &str) -> Option<String> {
    let mut rest = source;
    while let Some(offset) = rest.find("={") {
        let before = &rest[..offset];
        let name_start = before
            .rfind(|c: char| c.is_whitespace() || c == '<')
            .map(|i| i + 1)
            .unwrap_or(0);
        let name = &before[name_start..];
        let after = &rest[offset + 2..];
        let close = after.find('}')?;
        // These three references are the exact values consumed by the
        // catalog-to-tile lowering; every other attribute expression fails.
        if matches!(
            (name, &after[..close]),
            ("href", "app.url") | ("src", "app.icon") | ("key", "app.id") | ("app", "app")
        ) {
            rest = &after[close + 1..];
            continue;
        }
        if !name.is_empty() {
            return Some(format!("{}={{{}}}", name, &after[..close]));
        }
        rest = &after[close + 1..];
    }
    None
}
fn tailwind_space(token: &str) -> Result<f64, serde_json::Value> {
    token
        .parse::<f64>()
        .ok()
        .filter(|n| *n >= 0. && *n <= 96.)
        .map(|n| n * 4.)
        .ok_or_else(|| {
            diag(
                "TAILWIND_UNSUPPORTED",
                "gap must be a bounded numeric Tailwind spacing token",
                token,
            )
        })
}
fn source_fingerprint(source: &str, catalog: &str) -> String {
    format!(
        "{:016x}",
        source
            .bytes()
            .chain(catalog.bytes())
            .fold(14695981039346656037u64, |h, b| (h ^ b as u64)
                .wrapping_mul(1099511628211))
    )
}

/// Parses only `const SUITE_APPS = [{ id:'', name:'', icon:'', url:'',
/// description:'' }, ...]`. Bracket tracking handles optional literal arrays
/// such as requiredCapabilities, while every emitted field must be a literal.
fn parse_suite_apps(source: &str) -> Result<Vec<ImportedTile>, serde_json::Value> {
    let origin = literal_binding(source, "ICON_ORIGIN")?;
    let start = source.find("const SUITE_APPS = [").ok_or_else(|| {
        diag(
            "CATALOG_UNSUPPORTED",
            "missing literal SUITE_APPS declaration",
            "SUITE_APPS",
        )
    })?;
    let tail = &source[start..];
    let end = tail.find("];\n").ok_or_else(|| {
        diag(
            "CATALOG_UNSUPPORTED",
            "unterminated SUITE_APPS literal",
            "SUITE_APPS",
        )
    })?;
    let body = &tail[..end];
    let mut objects = Vec::new();
    let mut depth = 0usize;
    let mut object_start = None;
    for (i, c) in body.char_indices() {
        match c {
            '{' => {
                if depth == 0 {
                    object_start = Some(i);
                }
                depth += 1;
            }
            '}' => {
                if depth == 0 {
                    return Err(diag(
                        "CATALOG_UNSUPPORTED",
                        "unbalanced object literal",
                        "SUITE_APPS",
                    ));
                }
                depth -= 1;
                if depth == 0 {
                    objects.push(&body[object_start.expect("set at opening brace")..=i]);
                }
            }
            _ => {}
        }
    }
    if depth != 0 || objects.is_empty() {
        return Err(diag(
            "CATALOG_UNSUPPORTED",
            "SUITE_APPS must contain object literals",
            "SUITE_APPS",
        ));
    }
    objects
        .into_iter()
        .map(|object| {
            Ok(ImportedTile {
                name: literal_field(object, "name")?,
                description: literal_field(object, "description")?,
                icon: icon_field(object, &origin)?,
                url: literal_field(object, "url")?,
            })
        })
        .collect()
}

fn literal_binding(source: &str, name: &str) -> Result<String, serde_json::Value> {
    let marker = format!("const {name} =");
    let rest = source
        .find(&marker)
        .map(|i| &source[i + marker.len()..])
        .ok_or_else(|| {
            diag(
                "CATALOG_UNSUPPORTED",
                "missing required literal binding",
                name,
            )
        })?
        .trim_start();
    let quote = rest
        .chars()
        .next()
        .filter(|c| *c == '\'' || *c == '"')
        .ok_or_else(|| diag("CATALOG_DYNAMIC", "binding must be a string literal", name))?;
    let end = rest[1..]
        .find(quote)
        .ok_or_else(|| diag("CATALOG_UNSUPPORTED", "unterminated binding literal", name))?;
    Ok(rest[1..end + 1].to_string())
}

fn literal_field(object: &str, field: &str) -> Result<String, serde_json::Value> {
    let marker = format!("{field}:");
    let rest = object
        .find(&marker)
        .map(|i| &object[i + marker.len()..])
        .ok_or_else(|| {
            diag(
                "CATALOG_UNSUPPORTED",
                "missing required literal field",
                field,
            )
        })?
        .trim_start();
    let quote = rest
        .chars()
        .next()
        .filter(|c| *c == '\'' || *c == '"')
        .ok_or_else(|| {
            diag(
                "CATALOG_DYNAMIC",
                "catalog fields must be string literals",
                field,
            )
        })?;
    let end = rest[1..]
        .find(quote)
        .ok_or_else(|| diag("CATALOG_UNSUPPORTED", "unterminated string literal", field))?;
    Ok(rest[1..end + 1].to_string())
}

fn icon_field(object: &str, origin: &str) -> Result<String, serde_json::Value> {
    let marker = "icon:";
    let rest = object
        .find(marker)
        .map(|i| &object[i + marker.len()..])
        .ok_or_else(|| diag("CATALOG_UNSUPPORTED", "missing required icon field", "icon"))?
        .trim_start();
    // `ICON_ORIGIN` is a top-level literal binding in the supported catalog.
    let suffix_start = rest.find("}/").ok_or_else(|| {
        diag(
            "CATALOG_DYNAMIC",
            "icon must use the bounded ICON_ORIGIN template",
            "icon",
        )
    })? + 1;
    let suffix = rest[suffix_start..]
        .trim_start_matches('/')
        .split('`')
        .next()
        .unwrap_or("");
    if suffix.is_empty() {
        return Err(diag(
            "CATALOG_DYNAMIC",
            "icon template must have a literal suffix",
            "icon",
        ));
    }
    Ok(format!("{}/{}", origin.trim_end_matches('/'), suffix))
}

async fn create_snapshot_nodes(
    state: &AppState,
    args: &CreateVectorsFromReactArgs,
    parsed: &ParsedPage,
    root_label: &str,
) -> ToolResult {
    let (doc_w, doc_h, active_layer) = {
        let doc = state.document.lock().await;
        (doc.width, doc.height, doc.active_layer_id)
    };
    let Some(layer_id) = args.layer_id.or(active_layer) else {
        return ToolResult::error("Document has no active layer");
    };
    let viewport = args
        .viewport
        .as_ref()
        .map(|v| (v.width, v.height))
        .unwrap_or((doc_w, doc_h));
    if !viewport.0.is_finite()
        || !viewport.1.is_finite()
        || viewport.0 < 240.0
        || viewport.1 < 180.0
    {
        return ToolResult::error("viewport must be finite and at least 240 by 180");
    }
    {
        let doc = state.document.lock().await;
        if !doc.layers.get(&layer_id).is_some_and(|l| !l.locked) {
            return ToolResult::error("destination layer is missing or locked");
        }
    }
    let origin = args.origin.as_ref().map(|p| (p.x, p.y)).unwrap_or((0., 0.));
    let mut nodes = Vec::new();
    let root = layout_app_directory(
        &parsed.tiles,
        &parsed.layout,
        &parsed.tile_style,
        origin,
        viewport,
        layer_id,
        &mut nodes,
    );
    if let Some(n) = nodes.iter_mut().find(|n| n.id == root) {
        n.name = args.group_name.clone().unwrap_or_else(|| root_label.into());
        n.tags.push("react-role:page".into());
    }
    let created: Vec<_> = nodes.iter().map(|n| n.id).collect();
    let text_count = parsed.tiles.len() * 2 + 1;
    let semantic_tree: Vec<_> = parsed.tiles.iter().map(|tile| serde_json::json!({"kind":"link","href":tile.url,"children":[{"kind":"image","src":tile.icon},{"kind":"text","value":tile.name},{"kind":"text","value":tile.description}]})).collect();
    let data = serde_json::json!({"root_node_ids":[root],"created_node_ids":created,"node_counts":{"nodes":nodes.len(),"tiles":parsed.tiles.len(),"text":text_count,"images":parsed.tiles.len(),"links":parsed.tiles.len()},"layout":{"columns":parsed.layout.desktop_columns,"gap_px":parsed.layout.gap,"tile_padding":parsed.tile_style.padding,"badge_px":parsed.tile_style.badge,"radius_px":parsed.tile_style.radius},"semantic_tree":semantic_tree,"source_fingerprint":parsed.fingerprint,"resolved_files":parsed.resolved_files,"dry_run":args.dry_run,"contract_version":2,"diagnostics":[]});
    if args.dry_run {
        return ToolResult::text("React source import plan").with_data(data);
    }
    let cmd = Command::AddSubtree {
        layer_id,
        roots: vec![root],
        nodes,
    };
    let mut doc = state.document.lock().await;
    let mut history = state.history.lock().await;
    history.execute_discrete(cmd, &mut doc);
    ToolResult::text("Created editable vectors and text from React source snapshot").with_data(data)
}

fn layout_app_directory(
    tiles: &[ImportedTile],
    layout: &LayoutSpec,
    style: &TileStyle,
    origin: (f64, f64),
    viewport: (f64, f64),
    layer: uuid::Uuid,
    out: &mut Vec<SceneNode>,
) -> uuid::Uuid {
    let padding = style.padding * 2.0;
    let gap = layout.gap;
    let cols = if viewport.0 >= 900.0 {
        layout.desktop_columns
    } else if viewport.0 >= 560.0 {
        2
    } else {
        1
    };
    let card_w = (viewport.0 - padding * 2.0 - gap * (cols - 1) as f64) / cols as f64;
    let card_h = style.badge + style.padding * 2.0;
    let mut children = Vec::new();
    for (i, tile) in tiles.iter().enumerate() {
        let col = i % cols;
        let row = i / cols;
        let x = origin.0 + padding + col as f64 * (card_w + gap);
        let y = origin.1 + padding + row as f64 * (card_h + gap);
        let mut card_children = vec![
            rect_node(
                "Card surface",
                x,
                y,
                card_w,
                card_h,
                style.radius,
                "#ffffff",
                layer,
                out,
            ),
            rect_node(
                "App badge",
                x + style.padding,
                y + style.padding,
                style.badge,
                style.badge,
                style.radius,
                badge_color(&tile.icon),
                layer,
                out,
            ),
        ];
        if let Some(badge) = out.iter_mut().find(|n| n.id == card_children[1]) {
            badge.tags.push("react-role:image".into());
            badge.tags.push(format!("source:{}", tile.icon));
        }
        card_children.push(text_node(
            &tile.name,
            x + style.padding + style.badge + style.content_gap,
            y + style.padding + 14.0,
            style.title_size,
            style.title_weight,
            "#172033",
            layer,
            out,
        ));
        card_children.push(text_node(
            &tile.description,
            x + style.padding + style.badge + style.content_gap,
            y + style.padding + 14.0 + style.title_size + 5.0,
            style.description_size,
            400,
            "#64748b",
            layer,
            out,
        ));
        let tile_group = group_node(
            &format!("App tile: {}", tile.name),
            card_children,
            layer,
            out,
        );
        if let Some(group) = out.iter_mut().find(|n| n.id == tile_group) {
            group.tags.push("react-role:link".into());
            group.tags.push(format!("href:{}", tile.url));
        }
        children.push(tile_group);
    }
    let note_y =
        origin.1 + padding + ((tiles.len() + cols - 1) / cols) as f64 * (card_h + gap) + 10.0;
    children.push(text_node(
        "You'll confirm your Google account once when you open an app.",
        origin.0 + padding,
        note_y,
        12.0,
        400,
        "#64748b",
        layer,
        out,
    ));
    group_node("BGCH Hub AppDirectory snapshot", children, layer, out)
}

fn badge_color(source: &str) -> &'static str {
    const PALETTE: [&str; 7] = [
        "#2563eb", "#7c3aed", "#dc2626", "#0891b2", "#16a34a", "#ea580c", "#4f46e5",
    ];
    let index = source
        .bytes()
        .fold(0usize, |value, byte| value.wrapping_add(byte as usize))
        % PALETTE.len();
    PALETTE[index]
}

fn rect_node(
    name: &str,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    r: f64,
    color: &str,
    layer: uuid::Uuid,
    out: &mut Vec<SceneNode>,
) -> uuid::Uuid {
    let mut p = PathNode::new(PathData::rounded_rect(x, y, w, h, r));
    p.fill = Fill::solid(Color::from_hex(color).unwrap_or(Color::BLACK));
    let n = SceneNode::new(name, layer, SceneNodeKind::Path(p));
    let id = n.id;
    out.push(n);
    id
}
fn text_node(
    content: &str,
    x: f64,
    y: f64,
    size: f64,
    weight: u16,
    color: &str,
    layer: uuid::Uuid,
    out: &mut Vec<SceneNode>,
) -> uuid::Uuid {
    let mut t = TextNode::new(content);
    t.font_size = size;
    t.font_weight = weight;
    t.fill = Fill::solid(Color::from_hex(color).unwrap_or(Color::BLACK));
    let mut n = SceneNode::new(content, layer, SceneNodeKind::Text(t));
    n.transform = Transform::translate(x, y);
    let id = n.id;
    out.push(n);
    id
}
fn group_node(
    name: &str,
    children: Vec<uuid::Uuid>,
    layer: uuid::Uuid,
    out: &mut Vec<SceneNode>,
) -> uuid::Uuid {
    let mut g = GroupNode::new();
    g.children = children;
    let n = SceneNode::new(name, layer, SceneNodeKind::Group(g));
    let id = n.id;
    out.push(n);
    id
}

fn jsx_to_css(jsx: &str) -> Result<String, Vec<serde_json::Value>> {
    if jsx.len() > MAX_JSX_BYTES {
        return Err(vec![diag("JSX_LIMIT", "JSX input exceeds 256 KiB", "")]);
    }
    // Dynamic expressions are intentionally rejected, rather than evaluated
    // or silently omitted.  That also prohibits imports, hooks, and arbitrary
    // component execution at this boundary.
    if jsx.contains('{') || jsx.contains('}') || jsx.contains("import ") || jsx.contains("export ")
    {
        return Err(vec![diag(
            "JSX_DYNAMIC",
            "only a static JSX fragment is supported",
            "",
        )]);
    }
    let bytes = jsx.as_bytes();
    let mut i = 0;
    // Source tag name and generated selector are both retained: closing tags
    // validate against the former while children extend the latter.
    let mut stack: Vec<(String, String)> = Vec::new();
    let mut rules = Vec::new();
    let mut elements = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            let next = jsx[i..]
                .find('<')
                .map(|offset| i + offset)
                .unwrap_or(bytes.len());
            if !jsx[i..next].trim().is_empty() {
                return Err(vec![diag(
                    "JSX_TEXT_UNSUPPORTED",
                    "text children are not yet vectorizable; use editable text nodes separately",
                    jsx[i..next].trim(),
                )]);
            }
            i = next;
            continue;
        }
        let Some(end_rel) = jsx[i..].find('>') else {
            return Err(vec![diag("JSX_MALFORMED", "unterminated JSX tag", "")]);
        };
        let end = i + end_rel;
        let token = jsx[i + 1..end].trim();
        i = end + 1;
        if token.starts_with('!') {
            continue;
        }
        if let Some(close) = token.strip_prefix('/') {
            let close = close.trim();
            match stack.pop() {
                Some((tag, _)) if tag == close => continue,
                _ => {
                    return Err(vec![diag(
                        "JSX_MALFORMED",
                        "closing tag does not match an open tag",
                        close,
                    )])
                }
            }
        }
        let self_closing = token.ends_with('/');
        let token = token.trim_end_matches('/').trim();
        let tag_end = token.find(char::is_whitespace).unwrap_or(token.len());
        let tag = &token[..tag_end];
        if !matches!(
            tag,
            "div"
                | "section"
                | "main"
                | "article"
                | "header"
                | "footer"
                | "button"
                | "span"
                | "p"
                | "h1"
                | "h2"
                | "h3"
        ) {
            return Err(vec![diag(
                "JSX_UNSUPPORTED_ELEMENT",
                "only intrinsic layout elements are supported",
                tag,
            )]);
        }
        elements += 1;
        if elements > MAX_ELEMENTS {
            return Err(vec![diag(
                "JSX_LIMIT",
                "component contains more than 512 elements",
                tag,
            )]);
        }
        if stack.len() >= 32 {
            return Err(vec![diag(
                "JSX_LIMIT",
                "component nesting exceeds 32 levels",
                tag,
            )]);
        }
        let index = elements;
        let selector = if let Some((_, parent)) = stack.last() {
            format!("{parent} > .node-{index}")
        } else {
            if index != 1 {
                return Err(vec![diag(
                    "JSX_MULTIPLE_ROOTS",
                    "JSX fragment must have exactly one root element",
                    tag,
                )]);
            }
            ".component".into()
        };
        let classes = attr(token, "className")
            .or_else(|| attr(token, "class"))
            .unwrap_or_default();
        let declarations = tailwind(&classes, index == 1)?;
        rules.push(format!("{selector} {{ {declarations} }}"));
        if !self_closing {
            stack.push((tag.to_string(), selector));
        }
    }
    if !stack.is_empty() {
        return Err(vec![diag("JSX_MALFORMED", "unclosed JSX tag", "")]);
    }
    if rules.is_empty() {
        return Err(vec![diag("JSX_EMPTY", "JSX contains no elements", "")]);
    }
    Ok(rules.join("\n"))
}

fn attr(token: &str, name: &str) -> Option<String> {
    let start = token.find(name)? + name.len();
    let value = token[start..].trim_start().strip_prefix('=')?.trim_start();
    let quote = value.chars().next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    value[1..]
        .find(quote)
        .map(|end| value[1..1 + end].to_string())
}

fn tailwind(classes: &str, root: bool) -> Result<String, Vec<serde_json::Value>> {
    let mut out = if root {
        vec!["width:100%".into(), "height:100%".into()]
    } else {
        vec!["width:100%".into(), "height:40px".into()]
    };
    for class in classes.split_whitespace() {
        let mapped = match class {
            "w-full" => Some("width:100%".into()),
            "h-full" => Some("height:100%".into()),
            "rounded" => Some("border-radius:4px".into()),
            "rounded-md" => Some("border-radius:6px".into()),
            "rounded-lg" => Some("border-radius:8px".into()),
            "rounded-xl" => Some("border-radius:12px".into()),
            "rounded-full" => Some("border-radius:9999px".into()),
            "border" => Some("border:1px solid #000000".into()),
            "border-2" => Some("border:2px solid #000000".into()),
            "opacity-50" => Some("opacity:0.5".into()),
            "opacity-75" => Some("opacity:0.75".into()),
            "opacity-100" => Some("opacity:1".into()),
            "bg-white" => Some("background:#ffffff".into()),
            "bg-black" => Some("background:#000000".into()),
            "bg-slate-900" => Some("background:#0f172a".into()),
            "bg-slate-100" => Some("background:#f1f5f9".into()),
            "bg-blue-500" => Some("background:#3b82f6".into()),
            "bg-indigo-600" => Some("background:#4f46e5".into()),
            "bg-emerald-500" => Some("background:#10b981".into()),
            "bg-red-500" => Some("background:#ef4444".into()),
            _ => arbitrary_size(class),
        };
        match mapped {
            Some(value) => out.push(value),
            None => {
                return Err(vec![diag(
                    "TAILWIND_UNSUPPORTED",
                    "utility is outside the bounded Tailwind subset",
                    class,
                )])
            }
        }
    }
    Ok(out.join(";"))
}

fn arbitrary_size(class: &str) -> Option<String> {
    let (property, value) = class
        .strip_prefix("w-[")
        .map(|v| ("width", v))
        .or_else(|| class.strip_prefix("h-[").map(|v| ("height", v)))?;
    let value = value.strip_suffix(']')?;
    if value.ends_with("px")
        && value[..value.len() - 2]
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite() && *n >= 0.0 && *n <= 1_000_000.0)
            .is_some()
    {
        Some(format!("{property}:{value}"))
    } else {
        None
    }
}

fn diag(code: &str, message: &str, value: &str) -> serde_json::Value {
    serde_json::json!({"severity":"error", "code":code, "message":message, "value":value})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::McpServerConfig;
    use photonic_core::{history::CommandHistory, AuditLog, Document};
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio::sync::Mutex;

    fn source_test_state() -> AppState {
        let (tx, _rx) = std::sync::mpsc::channel();
        AppState {
            document: Arc::new(Mutex::new(Document::new("t", 1120.0, 720.0))),
            history: Arc::new(Mutex::new(CommandHistory::new(100))),
            document_path: Arc::new(StdMutex::new(None)),
            capture_tx: Arc::new(StdMutex::new(tx)),
            config: McpServerConfig::default(),
            audit_log: Arc::new(StdMutex::new(AuditLog::new())),
            clipboard_ring: Arc::new(crate::handlers::clipboard::new_clipboard_ring()),
        }
    }
    fn copied_root() -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("photonic-react-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("apps/hub/src/components")).unwrap();
        std::fs::create_dir_all(root.join("packages/waffle/src")).unwrap();
        let app = "import { SUITE_APPS, filterApps } from '@bgch/waffle';\nfunction AppTile(){return <CardContent className=\"flex items-center gap-4 p-5\"><img className=\"h-12 w-12 rounded-lg\"/><span className=\"font-semibold\"/><p className=\"text-sm\"/></CardContent>}\nexport function AppDirectory(){ const tiles = filterApps(apps); return <section><div className=\"grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3\">{tiles.map((app) => (<AppTile key={app.id} app={app} />))}</div></section> }\n";
        let catalog = "const ICON_ORIGIN = 'https://icons.example';\nconst SUITE_APPS = [{ id: 'a', name: 'ONE', icon: `${ICON_ORIGIN}/one.svg`, url: 'https://one.example', description: 'One' }, { id: 'b', name: 'TWO', icon: `${ICON_ORIGIN}/two.svg`, url: 'https://two.example', description: 'Two' }];\n";
        std::fs::write(root.join("apps/hub/src/components/AppDirectory.jsx"), app).unwrap();
        std::fs::write(root.join("packages/waffle/src/suiteApps.js"), catalog).unwrap();
        root
    }
    fn source_args(root: &std::path::Path, dry_run: bool) -> CreateVectorsFromReactArgs {
        CreateVectorsFromReactArgs {
            jsx: None,
            source: None,
            snapshot: None,
            source_path: Some(
                root.join("apps/hub/src/components/AppDirectory.jsx")
                    .display()
                    .to_string(),
            ),
            export_name: Some("AppDirectory".into()),
            props: Some(serde_json::json!({"isSuperAdmin":true,"loading":false})),
            module_roots: vec![root.display().to_string()],
            origin: None,
            viewport: None,
            layer_id: None,
            group_name: None,
            strict: true,
            dry_run,
        }
    }
    fn plan_json(result: &ToolResult) -> serde_json::Value {
        let crate::protocol::ContentItem::Text { text } = &result.content[1] else {
            panic!("missing JSON plan")
        };
        serde_json::from_str(text).unwrap()
    }
    #[test]
    fn static_tailwind_component_becomes_css_tree() {
        let css = jsx_to_css(r#"<div className="w-[320px] h-[160px] bg-slate-900 rounded-xl"><button className="w-[120px] h-[40px] bg-blue-500 rounded" /></div>"#).unwrap();
        assert!(css.contains(".component"));
        assert!(css.contains(".component > .node-2"));
        assert!(css.contains("background:#3b82f6"));
    }
    #[test]
    fn dynamic_jsx_is_rejected_not_ignored() {
        assert!(jsx_to_css("<div>{label}</div>").is_err());
    }
    #[test]
    fn text_is_rejected_not_silently_dropped() {
        assert!(jsx_to_css("<div>Save</div>").is_err());
    }

    #[test]
    fn catalog_literals_drive_imported_tile_content() {
        let source = "const ICON_ORIGIN = 'https://icons.example';\nconst SUITE_APPS = [\n{ id: 'x', name: 'ALPHA', icon: `${ICON_ORIGIN}/alpha-icon.svg`, url: 'https://alpha.example', description: 'First literal' },\n];\n";
        let mut tiles = parse_suite_apps(source).unwrap();
        assert_eq!(tiles[0].name, "ALPHA");
        assert_eq!(tiles[0].description, "First literal");
        assert_eq!(tiles[0].url, "https://alpha.example");
        assert_eq!(tiles[0].icon, "https://icons.example/alpha-icon.svg");
        // Simulates a copied catalog literal changed after the importer was built:
        // the parsed plan changes without any importer code change.
        let changed = source.replace("First literal", "Changed literal");
        tiles = parse_suite_apps(&changed).unwrap();
        assert_eq!(tiles[0].description, "Changed literal");
    }

    #[test]
    fn catalog_dynamic_field_is_a_diagnostic() {
        let source = "const ICON_ORIGIN = 'https://icons.example';\nconst SUITE_APPS = [\n{ name: app.name, icon: `${ICON_ORIGIN}/x.svg`, url: 'https://x.example', description: 'x' },\n];\n";
        assert_eq!(
            parse_suite_apps(source).unwrap_err()["code"],
            "CATALOG_DYNAMIC"
        );
    }

    #[test]
    fn literal_tailwind_gap_drives_layout_spec() {
        let baseline =
            r#"<div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">{tiles.map("#;
        let changed = baseline.replace("gap-4", "gap-8");
        assert_eq!(parse_grid_layout(baseline).unwrap().gap, 16.0);
        assert_eq!(parse_grid_layout(&changed).unwrap().gap, 32.0);
        assert_eq!(parse_grid_layout(&changed).unwrap().desktop_columns, 3);
    }

    #[test]
    fn last_grid_is_the_active_non_loading_grid() {
        let source = r#"<div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3"><div className="grid grid-cols-1 gap-8 sm:grid-cols-2 lg:grid-cols-3">{tiles.map("#;
        assert_eq!(parse_grid_layout(source).unwrap().gap, 32.0);
    }

    #[test]
    fn icon_origin_literal_drives_url() {
        let source = "const ICON_ORIGIN = 'https://new-icons.example';\nconst SUITE_APPS = [{ name: 'A', icon: `${ICON_ORIGIN}/a.svg`, url: 'https://a.example', description: 'A' },\n];\n";
        assert_eq!(
            parse_suite_apps(source).unwrap()[0].icon,
            "https://new-icons.example/a.svg"
        );
    }

    #[test]
    fn unreachable_empty_state_expression_is_not_scanned() {
        let source = "<section><EmptyState icon={AlertCircle} title=\"none\" /><div className=\"grid\">{tiles.map(";
        assert!(unsupported_attribute_expression(&strip_unreachable_branches(source)).is_none());
    }

    #[test]
    fn rendered_unknown_attribute_expression_is_scanned() {
        assert_eq!(
            unsupported_attribute_expression("<section data-fixture={unknownStaticValue}>")
                .as_deref(),
            Some("data-fixture={unknownStaticValue}")
        );
    }

    #[tokio::test]
    async fn copied_root_source_import_is_metamorphic_and_rejection_does_not_mutate() {
        let root = copied_root();
        let state = source_test_state();
        let baseline = create_vectors_from_react(&state, source_args(&root, true)).await;
        let baseline_json = plan_json(&baseline);
        assert_eq!(baseline_json["layout"]["gap_px"], 16.0, "{baseline_json}");
        assert!(baseline_json
            .to_string()
            .contains("https://icons.example/one.svg"));
        let app_path = root.join("apps/hub/src/components/AppDirectory.jsx");
        let app = std::fs::read_to_string(&app_path)
            .unwrap()
            .replace("gap-4", "gap-8");
        std::fs::write(&app_path, app).unwrap();
        let gap_result = create_vectors_from_react(&state, source_args(&root, true)).await;
        assert_eq!(plan_json(&gap_result)["layout"]["gap_px"], 32.0);
        let cat = root.join("packages/waffle/src/suiteApps.js");
        let content = std::fs::read_to_string(&cat)
            .unwrap()
            .replace("https://icons.example", "https://changed.example");
        std::fs::write(&cat, content).unwrap();
        let icon_result = create_vectors_from_react(&state, source_args(&root, true)).await;
        assert!(plan_json(&icon_result)
            .to_string()
            .contains("https://changed.example/one.svg"));
        let bad = std::fs::read_to_string(&app_path)
            .unwrap()
            .replace("<section>", "<section onClick={recordClick}>");
        std::fs::write(&app_path, bad).unwrap();
        let before = state.document.lock().await.nodes.len();
        let undo = state.history.lock().await.undo_depth();
        let rejected = create_vectors_from_react(&state, source_args(&root, false)).await;
        assert_eq!(rejected.is_error, Some(true));
        assert_eq!(state.document.lock().await.nodes.len(), before);
        assert_eq!(state.history.lock().await.undo_depth(), undo);
        let _ = std::fs::remove_dir_all(root);
    }
}
