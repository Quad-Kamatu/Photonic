//! Safe local resolution of the bounded Lucide icons used by Check-In.
//!
//! Icon data is parsed from an installed `lucide-react` package or its Vite
//! dependency bundle. JavaScript is never evaluated and resolution never
//! leaves an explicitly supplied module root.

use photonic_core::{
    color::Color,
    node::{GroupNode, SceneNode, SceneNodeKind},
    transform::Transform,
    Document,
};
use regex::Regex;
use std::{collections::HashMap, path::PathBuf};

#[derive(Debug, Clone)]
pub(crate) struct LucideAsset {
    pub name: String,
    pub source_path: PathBuf,
    pub document: Document,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LucideDiagnostic {
    pub code: &'static str,
    pub message: String,
    pub value: String,
}

fn diagnostic(
    code: &'static str,
    message: impl Into<String>,
    value: impl Into<String>,
) -> LucideDiagnostic {
    LucideDiagnostic {
        code,
        message: message.into(),
        value: value.into(),
    }
}

fn icon_slug(name: &str) -> Result<&'static str, LucideDiagnostic> {
    match name {
        "Calendar" => Ok("calendar"),
        "Users" => Ok("users"),
        "LogOut" => Ok("log-out"),
        _ => Err(diagnostic(
            "LUCIDE_ICON_UNSUPPORTED",
            "only Calendar, Users, and LogOut are supported by this snapshot",
            name,
        )),
    }
}

/// Resolve every requested icon before any caller mutates its document.
pub(crate) fn resolve_lucide_icon_set(
    module_roots: &[String],
    names: &[&str],
) -> Result<Vec<LucideAsset>, LucideDiagnostic> {
    if module_roots.is_empty() {
        return Err(diagnostic(
            "LUCIDE_MODULE_ROOTS",
            "module_roots is required for local Lucide resolution",
            "module_roots",
        ));
    }
    let roots: Result<Vec<_>, _> = module_roots.iter().map(std::fs::canonicalize).collect();
    let roots = roots.map_err(|_| {
        diagnostic(
            "LUCIDE_MODULE_ROOTS",
            "a module root does not exist",
            "module_roots",
        )
    })?;
    names
        .iter()
        .map(|name| resolve_lucide_icon(&roots, name))
        .collect()
}

fn resolve_lucide_icon(roots: &[PathBuf], name: &str) -> Result<LucideAsset, LucideDiagnostic> {
    let slug = icon_slug(name)?;
    let direct = [
        format!("node_modules/lucide-react/dist/esm/icons/{slug}.js"),
        format!("apps/checkin/node_modules/lucide-react/dist/esm/icons/{slug}.js"),
    ];
    let bundles = [
        "node_modules/.vite/deps/lucide-react.js",
        "apps/checkin/node_modules/.vite/deps/lucide-react.js",
    ];
    for root in roots {
        for relative in &direct {
            if let Some(path) = checked_candidate(root, relative)? {
                let text = read_source(&path)?;
                return parse_asset(name, slug, path, &text, false);
            }
        }
        for relative in bundles {
            if let Some(path) = checked_candidate(root, relative)? {
                let text = read_source(&path)?;
                return parse_asset(name, slug, path, &text, true);
            }
        }
    }
    Err(diagnostic(
        "LUCIDE_ASSET_NOT_FOUND",
        "installed lucide-react source was not found under module_roots",
        slug,
    ))
}

fn checked_candidate(
    root: &std::path::Path,
    relative: &str,
) -> Result<Option<PathBuf>, LucideDiagnostic> {
    let candidate = root.join(relative);
    if !candidate.exists() {
        return Ok(None);
    }
    let path = std::fs::canonicalize(&candidate).map_err(|_| {
        diagnostic(
            "LUCIDE_ASSET_READ",
            "installed Lucide asset cannot be canonicalized",
            relative,
        )
    })?;
    if !path.starts_with(root) {
        return Err(diagnostic(
            "LUCIDE_ASSET_OUTSIDE_ROOT",
            "installed Lucide asset resolves outside module_roots",
            path.display().to_string(),
        ));
    }
    Ok(Some(path))
}

fn read_source(path: &std::path::Path) -> Result<String, LucideDiagnostic> {
    std::fs::read_to_string(path).map_err(|_| {
        diagnostic(
            "LUCIDE_ASSET_READ",
            "installed Lucide asset is not readable UTF-8",
            path.display().to_string(),
        )
    })
}

fn parse_asset(
    name: &str,
    slug: &str,
    source_path: PathBuf,
    source: &str,
    bundled: bool,
) -> Result<LucideAsset, LucideDiagnostic> {
    let section = if bundled {
        let marker = format!("/icons/{slug}.js");
        let start = source.find(&marker).ok_or_else(|| {
            diagnostic(
                "LUCIDE_ICON_NOT_FOUND",
                "installed Lucide bundle does not contain the requested icon",
                slug,
            )
        })?;
        let tail = &source[start..];
        let end_marker = format!("createLucideIcon(\"{slug}\"");
        let end = tail.find(&end_marker).ok_or_else(|| {
            diagnostic(
                "LUCIDE_SOURCE_UNSUPPORTED",
                "Lucide bundle icon section is malformed",
                slug,
            )
        })?;
        &tail[..end]
    } else {
        source
    };
    let svg = icon_node_section_to_svg(section)?;
    let document = photonic_core::import_svg(&svg).map_err(|error| {
        diagnostic(
            "LUCIDE_SVG_INVALID",
            format!("Lucide geometry could not be imported: {error}"),
            source_path.display().to_string(),
        )
    })?;
    if document.nodes.is_empty()
        || !document
            .nodes
            .values()
            .all(|node| matches!(node.kind, SceneNodeKind::Path(_) | SceneNodeKind::Group(_)))
    {
        return Err(diagnostic(
            "LUCIDE_GEOMETRY_UNSUPPORTED",
            "Lucide icon must contain editable path/group geometry",
            slug,
        ));
    }
    Ok(LucideAsset {
        name: name.to_string(),
        source_path,
        document,
    })
}

fn icon_node_section_to_svg(source: &str) -> Result<String, LucideDiagnostic> {
    let entry_re =
        Regex::new(r#"(?s)\[\s*\"(path|rect|circle|line|polyline|polygon)\"\s*,\s*\{(.*?)\}\s*\]"#)
            .expect("literal regex");
    let attr_re =
        Regex::new(r#"([A-Za-z][A-Za-z0-9]*)\s*:\s*\"([^\"]*)\""#).expect("literal regex");
    let mut elements = Vec::new();
    for entry in entry_re.captures_iter(source) {
        let tag = &entry[1];
        let mut attributes = Vec::new();
        for attribute in attr_re.captures_iter(&entry[2]) {
            let name = &attribute[1];
            if name == "key" {
                continue;
            }
            if !matches!(
                name,
                "d" | "x"
                    | "y"
                    | "x1"
                    | "y1"
                    | "x2"
                    | "y2"
                    | "cx"
                    | "cy"
                    | "r"
                    | "rx"
                    | "ry"
                    | "width"
                    | "height"
                    | "points"
            ) {
                return Err(diagnostic(
                    "LUCIDE_ATTRIBUTE_UNSUPPORTED",
                    "Lucide primitive contains an unsupported attribute",
                    name,
                ));
            }
            let value = xml_escape(&attribute[2]);
            attributes.push(format!(r#"{name}="{value}""#));
        }
        if attributes.is_empty() {
            return Err(diagnostic(
                "LUCIDE_SOURCE_UNSUPPORTED",
                "Lucide primitive has no geometry attributes",
                tag,
            ));
        }
        elements.push(format!(
            r##"<{tag} {} fill="none" stroke="#000000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>"##,
            attributes.join(" ")
        ));
    }
    if elements.is_empty() || elements.len() > 32 {
        return Err(diagnostic(
            "LUCIDE_SOURCE_UNSUPPORTED",
            "Lucide icon must contain between 1 and 32 bounded primitives",
            "iconNode",
        ));
    }
    Ok(format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{}</svg>"##,
        elements.join("")
    ))
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Clone an imported icon into editable Photonic paths at a bounded square.
pub(crate) fn append_lucide_icon(
    asset: &LucideAsset,
    origin: (f64, f64),
    size: f64,
    color: Color,
    layer: uuid::Uuid,
    out: &mut Vec<SceneNode>,
) -> Result<uuid::Uuid, LucideDiagnostic> {
    if !size.is_finite() || size <= 0.0 || size > 4096.0 {
        return Err(diagnostic(
            "LUCIDE_SIZE",
            "Lucide icon size must be finite and between 0 and 4096",
            size.to_string(),
        ));
    }
    let scale = size / 24.0;
    let placement = Transform::new(scale, 0.0, 0.0, scale, origin.0, origin.1);
    let mut id_map = HashMap::new();
    for id in asset.document.nodes.keys() {
        id_map.insert(*id, uuid::Uuid::new_v4());
    }
    let roots: Vec<_> = asset
        .document
        .layer_order
        .iter()
        .filter_map(|id| asset.document.layers.get(id))
        .flat_map(|source_layer| source_layer.node_ids.iter().copied())
        .collect();
    let mut children = Vec::new();
    for root in roots {
        append_node(&asset.document, root, placement, color, layer, &id_map, out);
        if let Some(id) = id_map.get(&root) {
            children.push(*id);
        }
    }
    if children.is_empty() {
        return Err(diagnostic(
            "LUCIDE_GEOMETRY_UNSUPPORTED",
            "Lucide icon has no root geometry",
            &asset.name,
        ));
    }
    let mut group = GroupNode::new();
    group.children = children;
    let mut node = SceneNode::new(
        format!("Lucide icon: {}", asset.name),
        layer,
        SceneNodeKind::Group(group),
    );
    node.tags.push("react-role:icon".into());
    node.tags
        .push(format!("react-icon-box:{},{},{}", origin.0, origin.1, size));
    node.tags
        .push(format!("source-file:{}", asset.source_path.display()));
    let id = node.id;
    out.push(node);
    Ok(id)
}

fn append_node(
    document: &Document,
    old_id: uuid::Uuid,
    parent: Transform,
    color: Color,
    layer: uuid::Uuid,
    id_map: &HashMap<uuid::Uuid, uuid::Uuid>,
    out: &mut Vec<SceneNode>,
) {
    let Some(source) = document.nodes.get(&old_id) else {
        return;
    };
    let world = parent.then(&source.transform);
    let mut node = source.clone();
    node.id = id_map[&old_id];
    node.layer_id = layer;
    node.transform = world;
    if let SceneNodeKind::Path(path) = &mut node.kind {
        path.stroke.color = color;
        path.stroke.paint = None;
        path.stroke.width *= (world.matrix[0] * world.matrix[3]
            - world.matrix[1] * world.matrix[2])
            .abs()
            .sqrt();
    }
    if let SceneNodeKind::Group(group) = &mut node.kind {
        node.transform = Transform::IDENTITY;
        group.children = group
            .children
            .iter()
            .filter_map(|id| id_map.get(id).copied())
            .collect();
        for child in &source_group_children(source) {
            append_node(document, *child, world, color, layer, id_map, out);
        }
    }
    out.push(node);
}

fn source_group_children(node: &SceneNode) -> Vec<uuid::Uuid> {
    match &node.kind {
        SceneNodeKind::Group(group) => group.children.clone(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photonic_core::{node::SceneNodeKind, Document};

    const BUNDLE: &str = r#"
// ../../node_modules/lucide-react/dist/esm/icons/calendar.js
var __iconNode1 = [
  ["path", { d: "M8 2v4", key: "a" }],
  ["rect", { width: "18", height: "18", x: "3", y: "4", rx: "2", key: "b" }]
];
var Calendar = createLucideIcon("calendar", __iconNode1);
// ../../node_modules/lucide-react/dist/esm/icons/log-out.js
var __iconNode2 = [["path", { d: "m16 17 5-5-5-5", key: "c" }]];
var LogOut = createLucideIcon("log-out", __iconNode2);
// ../../node_modules/lucide-react/dist/esm/icons/users.js
var __iconNode3 = [["circle", { cx: "9", cy: "7", r: "4", key: "d" }]];
var Users = createLucideIcon("users", __iconNode3);
"#;

    fn fixture_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("photonic-lucide-{}", uuid::Uuid::new_v4()));
        let bundle = root.join("apps/checkin/node_modules/.vite/deps");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(bundle.join("lucide-react.js"), BUNDLE).unwrap();
        root
    }

    #[test]
    fn resolves_installed_icons_and_builds_bounded_editable_geometry() {
        let root = fixture_root();
        let roots = vec![root.display().to_string()];
        let assets = resolve_lucide_icon_set(&roots, &["Calendar", "Users", "LogOut"]).unwrap();
        assert_eq!(assets.len(), 3);
        let mut doc = Document::new("icons", 100.0, 40.0);
        let layer = doc.active_layer_id.unwrap();
        let mut nodes = Vec::new();
        let mut icon_roots = Vec::new();
        for (index, asset) in assets.iter().enumerate() {
            let root_id = append_lucide_icon(
                asset,
                (index as f64 * 32.0, 0.0),
                20.0,
                Color::BLACK,
                layer,
                &mut nodes,
            )
            .unwrap();
            doc.layers.get_mut(&layer).unwrap().node_ids.push(root_id);
            icon_roots.push(root_id);
        }
        for node in nodes {
            doc.nodes.insert(node.id, node);
        }
        assert_eq!(
            doc.nodes
                .values()
                .filter(|node| matches!(node.kind, SceneNodeKind::Path(_)))
                .count(),
            4
        );
        for (index, root_id) in icon_roots.iter().enumerate() {
            let SceneNodeKind::Group(group) = &doc.nodes[root_id].kind else {
                unreachable!()
            };
            for child in &group.children {
                let node = &doc.nodes[child];
                let SceneNodeKind::Path(path) = &node.kind else {
                    continue;
                };
                let bounds = path.path_data.bounding_box().unwrap();
                let half_stroke = path.stroke.width / 2.0;
                for (x, y) in [
                    node.transform.apply(bounds.x0, bounds.y0),
                    node.transform.apply(bounds.x1, bounds.y1),
                ] {
                    let x0 = index as f64 * 32.0;
                    assert!(x >= x0 - half_stroke - 0.1, "x={x}");
                    assert!(x <= x0 + 20.0 + half_stroke + 0.1, "x={x}");
                    assert!(y >= -half_stroke - 0.1, "y={y}");
                    assert!(y <= 20.0 + half_stroke + 0.1, "y={y}");
                }
                assert!(path.stroke.enabled);
                assert!(path.stroke.width <= 2.0);
            }
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn missing_icon_rejects_before_output_mutation() {
        let root = fixture_root();
        let bundle = root.join("apps/checkin/node_modules/.vite/deps/lucide-react.js");
        let calendar_only = BUNDLE
            .split("// ../../node_modules/lucide-react/dist/esm/icons/log-out.js")
            .next()
            .unwrap();
        std::fs::write(&bundle, calendar_only).unwrap();
        let roots = vec![root.display().to_string()];
        let output = vec![SceneNode::new(
            "sentinel",
            uuid::Uuid::nil(),
            SceneNodeKind::Group(GroupNode::new()),
        )];
        let before = output.clone();
        let error = resolve_lucide_icon_set(&roots, &["Calendar", "Users"]).unwrap_err();
        assert_eq!(error.code, "LUCIDE_ICON_NOT_FOUND");
        assert_eq!(output.len(), before.len());
        assert_eq!(output[0].id, before[0].id);
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_asset_outside_root_is_rejected_without_output_mutation() {
        use std::os::unix::fs::symlink;
        let root = fixture_root();
        let outside =
            std::env::temp_dir().join(format!("lucide-outside-{}.js", uuid::Uuid::new_v4()));
        std::fs::write(&outside, BUNDLE).unwrap();
        let bundle = root.join("apps/checkin/node_modules/.vite/deps/lucide-react.js");
        std::fs::remove_file(&bundle).unwrap();
        symlink(&outside, &bundle).unwrap();
        let roots = vec![root.display().to_string()];
        let output: Vec<SceneNode> = Vec::new();
        let error = resolve_lucide_icon_set(&roots, &["Calendar"]).unwrap_err();
        assert_eq!(error.code, "LUCIDE_ASSET_OUTSIDE_ROOT");
        assert!(output.is_empty());
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_file(outside);
    }

    #[test]
    fn resolves_real_checkin_install_when_acceptance_root_is_supplied() {
        let Ok(root) = std::env::var("PHOTONIC_BGCH_ACCEPTANCE_ROOT") else {
            return;
        };
        let assets = resolve_lucide_icon_set(&[root], &["Calendar", "Users", "LogOut"])
            .expect("installed Check-In Lucide icons should resolve");
        assert_eq!(assets.len(), 3);
        assert!(assets.iter().all(|asset| !asset.document.nodes.is_empty()));
        assert!(assets
            .iter()
            .all(|asset| asset.source_path.to_string_lossy().contains("node_modules")));
    }
}
