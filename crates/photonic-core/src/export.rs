//! Document export utilities (SVG, etc.).

use crate::{
    layer::BlendMode,
    node::{NodeId, SceneNode, SceneNodeKind, TextAlign},
    style::{Fill, FillKind, GradientKind, LineCap, LineJoin, Stroke, StrokeAlign},
    transform::Transform,
    Color, Document,
};
use std::collections::{HashMap, HashSet};

// ─── Export options ───────────────────────────────────────────────────────────

/// Options controlling SVG export output.
#[derive(Debug, Clone)]
pub struct SvgExportOptions {
    /// Emit slugified node/layer names as `id` attributes (default: `true`).
    pub semantic_ids: bool,
    /// Decimal places for SVG dimension and viewBox values, clamped 1–6 (default: `4`).
    pub precision: u8,
    /// Background fill. `None` (the default) exports a transparent SVG — no
    /// background rect is emitted. `Some(color)` emits a full-artboard rect of
    /// that color (e.g. white) behind the artwork.
    pub background: Option<Color>,
}

impl Default for SvgExportOptions {
    fn default() -> Self {
        Self {
            semantic_ids: true,
            precision: 4,
            background: None,
        }
    }
}

// ─── Full-document export ─────────────────────────────────────────────────────

/// Export `doc` as an SVG string.
///
/// - Outputs `<!-- photonic-svg-v1 -->` as the first line for pipeline stability.
/// - Layers are emitted as `<g id="layer-name">` elements in draw order.
/// - When `opts.semantic_ids` is true, every node element receives an `id`
///   derived from its name (slugified, deduplicated with a `-2`/`-3` suffix).
/// - Gradients are collected into a `<defs>` block.
/// - Transforms use SVG `matrix(a,b,c,d,e,f)` syntax (identity is omitted).
pub fn export_svg(doc: &Document, opts: &SvgExportOptions) -> String {
    let mut defs = String::new();
    let mut body = String::new();
    let mut grad_counter: usize = 0;
    let p = opts.precision.clamp(1, 6) as usize;

    // Pre-build node ID map when semantic IDs are enabled.
    let id_map: Option<HashMap<NodeId, String>> = if opts.semantic_ids {
        let mut used: HashSet<String> = HashSet::new();
        let mut map: HashMap<NodeId, String> = HashMap::new();
        for layer_id in &doc.layer_order {
            if let Some(layer) = doc.layers.get(layer_id) {
                for node_id in &layer.node_ids {
                    if let Some(node) = doc.nodes.get(node_id) {
                        collect_node_ids(node, doc, &mut used, &mut map);
                    }
                }
            }
        }
        Some(map)
    } else {
        None
    };

    // Optional background rect. `None` => transparent SVG (no rect emitted).
    if let Some(bg) = opts.background {
        body.push_str(&format!(
            "  <rect width=\"{w:.p$}\" height=\"{h:.p$}\" fill=\"{fill}\"/>\n",
            w = doc.width,
            h = doc.height,
            p = p,
            fill = bg.to_hex(),
        ));
    }

    let mut used_layer_ids: HashSet<String> = HashSet::new();
    for layer_id in &doc.layer_order {
        let layer = match doc.layers.get(layer_id) {
            Some(l) if l.visible => l,
            _ => continue,
        };

        let layer_id_str = if opts.semantic_ids {
            unique_id(&slugify(&layer.name), &mut used_layer_ids)
        } else {
            format!("layer-{}", layer.id)
        };

        let mut attrs = format!(" id=\"{}\"", layer_id_str);
        if (layer.opacity - 1.0).abs() > 0.001 {
            attrs.push_str(&format!(" opacity=\"{:.4}\"", layer.opacity));
        }
        body.push_str(&format!("  <g{}>\n", attrs));

        for node_id in &layer.node_ids {
            if let Some(node) = doc.nodes.get(node_id) {
                emit_node_inner(
                    node,
                    doc,
                    &mut defs,
                    &mut body,
                    &mut grad_counter,
                    4,
                    None,
                    id_map.as_ref(),
                );
            }
        }

        body.push_str("  </g>\n");
    }

    let defs_block = if defs.is_empty() {
        String::new()
    } else {
        format!("  <defs>\n{}  </defs>\n", defs)
    };

    format!(
        "<!-- photonic-svg-v1 -->\n\
         <svg xmlns=\"http://www.w3.org/2000/svg\" \
         xmlns:xlink=\"http://www.w3.org/1999/xlink\" \
         width=\"{w:.p$}\" height=\"{h:.p$}\" viewBox=\"0 0 {w:.p$} {h:.p$}\">\n\
         {defs}{body}</svg>",
        w = doc.width,
        h = doc.height,
        p = p,
        defs = defs_block,
        body = body,
    )
}

// ─── Selection export ─────────────────────────────────────────────────────────

/// Export a subset of nodes as a self-contained SVG with a tight viewBox.
///
/// - `node_ids`: which nodes to include. Returns an empty SVG if none are found.
/// - Node `name` is slugified and used as the `id` attribute on each element.
/// - No artboard background rect is emitted.
/// - viewBox is the union of all selected nodes' world-space bounding boxes;
///   falls back to full document dimensions if no bounds can be computed.
pub fn export_nodes_as_svg(doc: &Document, node_ids: &[NodeId]) -> String {
    let mut defs = String::new();
    let mut body = String::new();
    let mut grad_counter: usize = 0;
    let mut combined_bbox: Option<kurbo::Rect> = None;

    // Collect nodes in document order (layer order → z-order within layer).
    for layer_id in &doc.layer_order {
        let layer = match doc.layers.get(layer_id) {
            Some(l) if l.visible => l,
            _ => continue,
        };
        for node_id in &layer.node_ids {
            if !node_ids.contains(node_id) {
                continue;
            }
            if let Some(node) = doc.nodes.get(node_id) {
                if !node.visible {
                    continue;
                }
                if let Some(wb) = node_world_bbox(node, doc) {
                    combined_bbox = Some(match combined_bbox {
                        None => wb,
                        Some(prev) => prev.union(wb),
                    });
                }
                let slug = slugify(&node.name);
                emit_node_inner(
                    node,
                    doc,
                    &mut defs,
                    &mut body,
                    &mut grad_counter,
                    2,
                    Some(&slug),
                    None,
                );
            }
        }
    }

    let (vx, vy, vw, vh) = match combined_bbox {
        Some(r) => (r.x0, r.y0, r.x1 - r.x0, r.y1 - r.y0),
        None => (0.0, 0.0, doc.width as f64, doc.height as f64),
    };

    let defs_block = if defs.is_empty() {
        String::new()
    } else {
        format!("  <defs>\n{}  </defs>\n", defs)
    };

    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" \
         xmlns:xlink=\"http://www.w3.org/1999/xlink\" \
         width=\"{vw:.4}\" height=\"{vh:.4}\" viewBox=\"{vx:.4} {vy:.4} {vw:.4} {vh:.4}\">\n\
         {defs_block}{body}</svg>",
    )
}

/// Compute the world-space axis-aligned bounding box of a node by applying its
/// affine transform to its local bounding box.  Groups are handled recursively.
fn node_world_bbox(node: &SceneNode, doc: &Document) -> Option<kurbo::Rect> {
    let local = match &node.kind {
        SceneNodeKind::Path(p) => p.path_data.bounding_box()?,
        SceneNodeKind::Group(g) => {
            let mut combined: Option<kurbo::Rect> = None;
            for cid in &g.children {
                if let Some(child) = doc.nodes.get(cid) {
                    if let Some(cb) = node_world_bbox(child, doc) {
                        combined = Some(combined.map_or(cb, |prev| prev.union(cb)));
                    }
                }
            }
            combined?
        }
        SceneNodeKind::Text(_) => return None,
        SceneNodeKind::Raster(r) => {
            if r.is_adjustment_layer() {
                return None;
            }
            kurbo::Rect::new(0.0, 0.0, r.image.width as f64, r.image.height as f64)
        }
    };
    Some(node.transform.to_kurbo().transform_rect_bbox(local))
}

/// Return a deduplicated slug: appends `-2`, `-3`, … when `base` is already taken.
fn unique_id(base: &str, used: &mut HashSet<String>) -> String {
    if !used.contains(base) {
        used.insert(base.to_string());
        return base.to_string();
    }
    let mut n = 2u32;
    loop {
        let candidate = format!("{}-{}", base, n);
        if !used.contains(&candidate) {
            used.insert(candidate.clone());
            return candidate;
        }
        n += 1;
    }
}

/// Recursively populate `map` with slugified, deduplicated IDs for every node.
fn collect_node_ids(
    node: &SceneNode,
    doc: &Document,
    used: &mut HashSet<String>,
    map: &mut HashMap<NodeId, String>,
) {
    let slug = slugify(&node.name);
    let id = unique_id(&slug, used);
    map.insert(node.id, id);
    if let SceneNodeKind::Group(g) = &node.kind {
        for child_id in &g.children {
            if let Some(child) = doc.nodes.get(child_id) {
                collect_node_ids(child, doc, used, map);
            }
        }
    }
}

/// Convert a node name to a URL-safe `id` slug.
fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_dash = true; // start true to suppress leading dashes
    for c in name.chars() {
        if c.is_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    if out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "node".to_string()
    } else {
        out
    }
}

// ─── Node emitters ────────────────────────────────────────────────────────────

fn emit_node_inner(
    node: &SceneNode,
    doc: &Document,
    defs: &mut String,
    body: &mut String,
    grad_counter: &mut usize,
    indent: usize,
    // Explicit ID override (used by selection export).
    id_override: Option<&str>,
    // Map of NodeId → unique slug (used by full-document export).
    id_map: Option<&HashMap<NodeId, String>>,
) {
    if !node.visible {
        return;
    }

    let pad = " ".repeat(indent);
    let id_attr = id_override
        .map(|s| format!(" id=\"{}\"", s))
        .or_else(|| {
            id_map
                .and_then(|m| m.get(&node.id))
                .map(|s| format!(" id=\"{}\"", s))
        })
        .unwrap_or_default();
    let transform = transform_attr(&node.transform);
    let opacity = if (node.opacity - 1.0).abs() > 0.001 {
        format!(" opacity=\"{:.4}\"", node.opacity)
    } else {
        String::new()
    };
    // Non-Normal blend modes round-trip via the CSS `mix-blend-mode` property.
    let blend = if node.blend_mode != BlendMode::Normal {
        format!(" style=\"mix-blend-mode:{}\"", node.blend_mode.to_css())
    } else {
        String::new()
    };

    let filter = filter_attrs(node, defs);

    match &node.kind {
        SceneNodeKind::Path(p) => {
            let fill = fill_attrs(&p.fill, defs, grad_counter);
            let stroke = stroke_attrs(&p.stroke);
            body.push_str(&format!(
                "{}<path{}{}{}{}{}{}{} d=\"{}\"/>\n",
                pad,
                id_attr,
                transform,
                opacity,
                blend,
                filter,
                fill,
                stroke,
                p.path_data.as_svg(),
            ));
        }
        SceneNodeKind::Group(g) => {
            body.push_str(&format!(
                "{}<g{}{}{}{}{}>\n",
                pad, id_attr, transform, opacity, blend, filter
            ));
            for child_id in &g.children {
                if let Some(child) = doc.nodes.get(child_id) {
                    emit_node_inner(
                        child,
                        doc,
                        defs,
                        body,
                        grad_counter,
                        indent + 2,
                        None,
                        id_map,
                    );
                }
            }
            body.push_str(&format!("{}</g>\n", pad));
        }
        SceneNodeKind::Text(t) => {
            let fill = fill_attrs(&t.fill, defs, grad_counter);
            let stroke = stroke_attrs(&t.stroke);
            let anchor = match t.align {
                TextAlign::Left => "start",
                TextAlign::Center => "middle",
                TextAlign::Right => "end",
            };
            let content = t
                .content
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            body.push_str(&format!(
                "{}<text{}{}{}{} font-family=\"{}\" font-size=\"{}\" font-weight=\"{}\" \
                 text-anchor=\"{}\"{}{}>{}</text>\n",
                pad,
                id_attr,
                transform,
                opacity,
                blend,
                t.font_family,
                t.font_size,
                t.font_weight,
                anchor,
                fill,
                stroke,
                content,
            ));
        }
        SceneNodeKind::Raster(r) => {
            // Non-destructive adjustment layers carry no pixels of their own —
            // they recolor the composite beneath them, which a flat SVG cannot
            // represent. Skip them rather than emit a bogus 1×1 placeholder
            // (the .photonic format preserves them; PNG/JPEG bake them in).
            if r.is_adjustment_layer() {
                return;
            }
            // Embed the pixel data as a base64 PNG <image>. The node transform
            // positions/scales it; the image spans its native pixel size.
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(r.image.to_png());
            body.push_str(&format!(
                "{}<image{}{}{}{} width=\"{}\" height=\"{}\" \
                 href=\"data:image/png;base64,{}\"/>\n",
                pad, id_attr, transform, opacity, blend, r.image.width, r.image.height, b64,
            ));
        }
    }
}

fn transform_attr(t: &Transform) -> String {
    let [a, b, c, d, e, f] = t.matrix;
    if (a - 1.0).abs() < 1e-9
        && b.abs() < 1e-9
        && c.abs() < 1e-9
        && (d - 1.0).abs() < 1e-9
        && e.abs() < 1e-9
        && f.abs() < 1e-9
    {
        return String::new();
    }
    format!(" transform=\"matrix({a},{b},{c},{d},{e},{f})\"")
}

/// Emit an SVG `<filter>` for the node's live effects (drop shadow, object blur,
/// feather) into `defs` and return the ` filter="url(#…)"` attribute, or an
/// empty string when no effects are enabled. Effects chain in order: blur the
/// source first, then the drop shadow.
fn filter_attrs(node: &SceneNode, defs: &mut String) -> String {
    let ds = &node.drop_shadow;
    let ob = &node.object_blur;
    let ft = &node.feather;
    if !ds.enabled && !ob.enabled && !ft.enabled {
        return String::new();
    }
    let id = format!("fx{}", node.id.simple());
    let mut prims = String::new();
    // Object blur / feather both soften the graphic; object blur wins if both set.
    if ob.enabled {
        prims.push_str(&format!(
            "    <feGaussianBlur in=\"SourceGraphic\" stdDeviation=\"{:.3}\"/>\n",
            ob.radius
        ));
    } else if ft.enabled {
        prims.push_str(&format!(
            "    <feGaussianBlur in=\"SourceGraphic\" stdDeviation=\"{:.3}\"/>\n",
            ft.radius
        ));
    }
    if ds.enabled {
        let c = &ds.color;
        let hex = format!(
            "#{:02x}{:02x}{:02x}",
            (c.r * 255.0) as u8,
            (c.g * 255.0) as u8,
            (c.b * 255.0) as u8
        );
        prims.push_str(&format!(
            "    <feDropShadow dx=\"{:.3}\" dy=\"{:.3}\" stdDeviation=\"{:.3}\" \
             flood-color=\"{}\" flood-opacity=\"{:.3}\"/>\n",
            ds.dx,
            ds.dy,
            ds.blur,
            hex,
            (c.a * ds.opacity).clamp(0.0, 1.0),
        ));
    }
    // Generous region so blurs/shadows are not clipped.
    defs.push_str(&format!(
        "    <filter id=\"{}\" x=\"-50%\" y=\"-50%\" width=\"200%\" height=\"200%\">\n{}    </filter>\n",
        id, prims
    ));
    format!(" filter=\"url(#{})\"", id)
}

fn fill_attrs(fill: &Fill, defs: &mut String, counter: &mut usize) -> String {
    if !fill.enabled {
        return " fill=\"none\"".to_string();
    }
    match &fill.kind {
        FillKind::None => " fill=\"none\"".to_string(),
        FillKind::Solid(c) => solid_fill_attr(c, fill.opacity),
        FillKind::FluidGradient(fg) => {
            // Export as a radial gradient approximation: first point = center,
            // remaining points as stops at increasing radii (best-effort SVG).
            if fg.points.is_empty() {
                return " fill=\"none\"".to_string();
            }
            if fg.points.len() == 1 {
                return solid_fill_attr(&fg.points[0].color, fill.opacity);
            }
            let id = format!("grad-{}", *counter);
            *counter += 1;
            // Use centroid as gradient center
            let cx: f64 = fg.points.iter().map(|p| p.x).sum::<f64>() / fg.points.len() as f64;
            let cy: f64 = fg.points.iter().map(|p| p.y).sum::<f64>() / fg.points.len() as f64;
            let max_r: f64 = fg
                .points
                .iter()
                .map(|p| ((p.x - cx).powi(2) + (p.y - cy).powi(2)).sqrt())
                .fold(0.0_f64, f64::max)
                .max(1.0);
            // Use first point's color at center, average of outer points at edge
            let first = &fg.points[0];
            let last = &fg.points[fg.points.len() - 1];
            let stops = format!(
                "      <stop offset=\"0\" stop-color=\"{}\"/>\n\
                       <stop offset=\"1\" stop-color=\"{}\"/>\n",
                first.color.to_hex(),
                last.color.to_hex()
            );
            defs.push_str(&format!(
                "    <radialGradient id=\"{id}\" cx=\"{cx}\" cy=\"{cy}\" r=\"{max_r}\" \
                 gradientUnits=\"userSpaceOnUse\">\n{stops}    </radialGradient>\n",
            ));
            if (fill.opacity - 1.0).abs() < 0.001 {
                format!(" fill=\"url(#{id})\"")
            } else {
                format!(" fill=\"url(#{id})\" fill-opacity=\"{:.4}\"", fill.opacity)
            }
        }
        FillKind::MeshGradient(mg) => {
            // Export as a linear gradient approximation between first and last vertex colours.
            if mg.vertices.is_empty() {
                return " fill=\"none\"".to_string();
            }
            if mg.vertices.len() == 1 {
                return solid_fill_attr(&mg.vertices[0].color, fill.opacity);
            }
            let id = format!("grad-{}", *counter);
            *counter += 1;
            let first = &mg.vertices[0];
            let last = &mg.vertices[mg.vertices.len() - 1];
            let stops = format!(
                "      <stop offset=\"0\" stop-color=\"{}\"/>\n\
                       <stop offset=\"1\" stop-color=\"{}\"/>\n",
                first.color.to_hex(),
                last.color.to_hex()
            );
            defs.push_str(&format!(
                "    <linearGradient id=\"{id}\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" \
                 gradientUnits=\"userSpaceOnUse\">\n{stops}    </linearGradient>\n",
                first.x, first.y, last.x, last.y,
            ));
            if (fill.opacity - 1.0).abs() < 0.001 {
                format!(" fill=\"url(#{id})\"")
            } else {
                format!(" fill=\"url(#{id})\" fill-opacity=\"{:.4}\"", fill.opacity)
            }
        }
        FillKind::Gradient(g) => {
            let id = format!("grad-{}", *counter);
            *counter += 1;

            let stops: String = g
                .stops
                .iter()
                .map(|s| {
                    let hex = s.color.to_hex();
                    if (s.color.a - 1.0).abs() < 0.001 {
                        format!(
                            "      <stop offset=\"{}\" stop-color=\"{}\"/>\n",
                            s.offset, hex
                        )
                    } else {
                        format!(
                            "      <stop offset=\"{}\" stop-color=\"{}\" stop-opacity=\"{:.4}\"/>\n",
                            s.offset, hex, s.color.a
                        )
                    }
                })
                .collect();

            match g.kind {
                GradientKind::Linear => {
                    let (x1, y1, x2, y2) = if g.coords.len() >= 4 {
                        (g.coords[0], g.coords[1], g.coords[2], g.coords[3])
                    } else {
                        (0.0, 0.0, 1.0, 0.0)
                    };
                    defs.push_str(&format!(
                        "    <linearGradient id=\"{id}\" x1=\"{x1}\" y1=\"{y1}\" \
                         x2=\"{x2}\" y2=\"{y2}\" gradientUnits=\"userSpaceOnUse\">\n\
                         {stops}\
                         </linearGradient>\n",
                    ));
                }
                GradientKind::Radial => {
                    let (cx, cy, r) = if g.coords.len() >= 5 {
                        (g.coords[0], g.coords[1], g.coords[4])
                    } else {
                        (0.5, 0.5, 0.5)
                    };
                    defs.push_str(&format!(
                        "    <radialGradient id=\"{id}\" cx=\"{cx}\" cy=\"{cy}\" r=\"{r}\" \
                         gradientUnits=\"userSpaceOnUse\">\n\
                         {stops}\
                         </radialGradient>\n",
                    ));
                }
            }

            if (fill.opacity - 1.0).abs() < 0.001 {
                format!(" fill=\"url(#{id})\"")
            } else {
                format!(" fill=\"url(#{id})\" fill-opacity=\"{:.4}\"", fill.opacity)
            }
        }
    }
}

fn solid_fill_attr(c: &Color, fill_opacity: f32) -> String {
    let hex = c.to_hex();
    let opacity = c.a * fill_opacity;
    if (opacity - 1.0).abs() < 0.001 {
        format!(" fill=\"{hex}\"")
    } else {
        format!(" fill=\"{hex}\" fill-opacity=\"{opacity:.4}\"")
    }
}

fn stroke_attrs(stroke: &Stroke) -> String {
    if !stroke.enabled || stroke.width <= 0.0 {
        return " stroke=\"none\"".to_string();
    }
    let hex = stroke.color.to_hex();
    let opacity = stroke.color.a * stroke.opacity;
    let cap = match stroke.line_cap {
        LineCap::Butt => "butt",
        LineCap::Round => "round",
        LineCap::Square => "square",
    };
    let join = match stroke.line_join {
        LineJoin::Miter => "miter",
        LineJoin::Round => "round",
        LineJoin::Bevel => "bevel",
    };

    let align_attr = match stroke.align {
        StrokeAlign::Center => "",
        StrokeAlign::Inside => " stroke-alignment=\"inner\"",
        StrokeAlign::Outside => " stroke-alignment=\"outer\"",
    };
    let mut s = format!(
        " stroke=\"{hex}\" stroke-width=\"{}\" stroke-linecap=\"{cap}\" stroke-linejoin=\"{join}\"{align_attr}",
        stroke.width,
    );
    if join == "miter" && (stroke.miter_limit - 4.0).abs() > 0.001 {
        s.push_str(&format!(" stroke-miterlimit=\"{}\"", stroke.miter_limit));
    }
    if (opacity - 1.0).abs() > 0.001 {
        s.push_str(&format!(" stroke-opacity=\"{opacity:.4}\""));
    }
    if !stroke.dash_array.is_empty() {
        let parts: Vec<String> = stroke.dash_array.iter().map(|d| d.to_string()).collect();
        s.push_str(&format!(" stroke-dasharray=\"{}\"", parts.join(",")));
        if stroke.dash_offset.abs() > 0.001 {
            s.push_str(&format!(" stroke-dashoffset=\"{}\"", stroke.dash_offset));
        }
    }
    s
}

// ─── PDF export (vector) ────────────────────────────────────────────────────

/// Options controlling vector PDF export.
#[derive(Debug, Clone, Default)]
pub struct PdfExportOptions {
    /// Paint a full-page background rectangle of this colour before the artwork.
    /// `None` (default) leaves the page background unpainted (white in viewers).
    pub background: Option<Color>,
}

/// Export `doc` as a single-page vector PDF (1 document unit = 1 PDF point).
///
/// MVP scope: filled/stroked vector paths with solid colours, node/group affine
/// transforms, and group nesting. Gradient fills are approximated by their first
/// stop colour; text, clipping, per-node opacity, blend modes and multi-page
/// artboards are documented follow-ups.
pub fn export_pdf(doc: &Document, opts: &PdfExportOptions) -> Vec<u8> {
    use pdf_writer::{Content, Finish, Pdf, Rect, Ref};

    let w = doc.width as f32;
    let h = doc.height as f32;

    let catalog_id = Ref::new(1);
    let page_tree_id = Ref::new(2);
    let page_id = Ref::new(3);
    let content_id = Ref::new(4);

    // ── Content stream ────────────────────────────────────────────────────────
    let mut content = Content::new();
    // PDF is Y-up with the origin at the bottom-left; Photonic/SVG is Y-down with
    // the origin at the top-left. Flip Y once so document coordinates map directly.
    content.transform([1.0, 0.0, 0.0, -1.0, 0.0, h]);

    if let Some(bg) = opts.background {
        content.set_fill_rgb(bg.r, bg.g, bg.b);
        content.move_to(0.0, 0.0);
        content.line_to(w, 0.0);
        content.line_to(w, h);
        content.line_to(0.0, h);
        content.close_path();
        content.fill_nonzero();
    }

    for layer_id in &doc.layer_order {
        let layer = match doc.layers.get(layer_id) {
            Some(l) if l.visible => l,
            _ => continue,
        };
        for node_id in &layer.node_ids {
            if let Some(node) = doc.nodes.get(node_id) {
                emit_node_pdf(node, doc, &mut content);
            }
        }
    }

    let stream = content.finish();

    // ── Document structure ──────────────────────────────────────────────────────
    let mut pdf = Pdf::new();
    pdf.catalog(catalog_id).pages(page_tree_id);
    pdf.pages(page_tree_id).kids([page_id]).count(1);
    {
        let mut page = pdf.page(page_id);
        page.parent(page_tree_id)
            .media_box(Rect::new(0.0, 0.0, w, h))
            .contents(content_id);
        page.resources().finish();
        page.finish();
    }
    pdf.stream(content_id, &stream);
    pdf.finish()
}

/// Recursively emit a node's geometry into the PDF content stream, applying its
/// affine transform within a save/restore so siblings are unaffected.
fn emit_node_pdf(node: &SceneNode, doc: &Document, content: &mut pdf_writer::Content) {
    if !node.visible {
        return;
    }
    let [a, b, c, d, e, f] = node.transform.matrix;
    content.save_state();
    content.transform([a as f32, b as f32, c as f32, d as f32, e as f32, f as f32]);

    match &node.kind {
        SceneNodeKind::Path(p) => {
            emit_path_geometry(&p.path_data, content);
            let fill = fill_rgb(&p.fill);
            let stroke = if p.stroke.enabled && p.stroke.width > 0.0 {
                Some(&p.stroke)
            } else {
                None
            };
            if let Some([fr, fg, fb]) = fill {
                content.set_fill_rgb(fr, fg, fb);
            }
            if let Some(s) = stroke {
                content.set_stroke_rgb(s.color.r, s.color.g, s.color.b);
                content.set_line_width(s.width as f32);
            }
            match (fill.is_some(), stroke.is_some()) {
                (true, true) => {
                    content.fill_nonzero_and_stroke();
                }
                (true, false) => {
                    content.fill_nonzero();
                }
                (false, true) => {
                    content.stroke();
                }
                // No paint — discard the path so it does not linger in the stream.
                (false, false) => {
                    content.end_path();
                }
            }
        }
        SceneNodeKind::Group(g) => {
            for child_id in &g.children {
                if let Some(child) = doc.nodes.get(child_id) {
                    emit_node_pdf(child, doc, content);
                }
            }
        }
        // Text is omitted in the MVP (PDF text needs embedded/subsetted fonts,
        // which requires a font system not available in photonic-core).
        SceneNodeKind::Text(_) => {}
        // Raster layers are omitted in the MVP (PDF image XObjects + the raster
        // compositing pipeline are out of scope for the vector PDF exporter).
        SceneNodeKind::Raster(_) => {}
    }

    content.restore_state();
}

/// Emit a `PathData`'s segments as PDF path-construction operators. Quadratic
/// segments are elevated to cubics (PDF has no quadratic operator).
fn emit_path_geometry(path: &crate::path::PathData, content: &mut pdf_writer::Content) {
    use kurbo::PathEl;
    let bez = path.to_bez_path();
    let mut cur = (0.0_f64, 0.0_f64);
    for el in bez.elements() {
        match el {
            PathEl::MoveTo(p) => {
                content.move_to(p.x as f32, p.y as f32);
                cur = (p.x, p.y);
            }
            PathEl::LineTo(p) => {
                content.line_to(p.x as f32, p.y as f32);
                cur = (p.x, p.y);
            }
            PathEl::QuadTo(c1, p) => {
                // Quadratic → cubic control-point elevation.
                let c1x = cur.0 + 2.0 / 3.0 * (c1.x - cur.0);
                let c1y = cur.1 + 2.0 / 3.0 * (c1.y - cur.1);
                let c2x = p.x + 2.0 / 3.0 * (c1.x - p.x);
                let c2y = p.y + 2.0 / 3.0 * (c1.y - p.y);
                content.cubic_to(
                    c1x as f32, c1y as f32, c2x as f32, c2y as f32, p.x as f32, p.y as f32,
                );
                cur = (p.x, p.y);
            }
            PathEl::CurveTo(c1, c2, p) => {
                content.cubic_to(
                    c1.x as f32,
                    c1.y as f32,
                    c2.x as f32,
                    c2.y as f32,
                    p.x as f32,
                    p.y as f32,
                );
                cur = (p.x, p.y);
            }
            PathEl::ClosePath => {
                content.close_path();
            }
        }
    }
}

/// Resolve a fill to a representative solid RGB, or `None` when the fill is
/// disabled / `None`. Gradient fills are approximated by their first stop.
fn fill_rgb(fill: &Fill) -> Option<[f32; 3]> {
    if !fill.enabled {
        return None;
    }
    let c = match &fill.kind {
        FillKind::None => return None,
        FillKind::Solid(c) => *c,
        FillKind::Gradient(g) => g.stops.first().map(|s| s.color)?,
        FillKind::FluidGradient(g) => g.points.first().map(|p| p.color)?,
        FillKind::MeshGradient(g) => g.vertices.first().map(|v| v.color)?,
    };
    Some([c.r, c.g, c.b])
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// SVG export must be transparent by default — no opaque background rect
    /// baked behind the artwork (regression for white-background exports).
    #[test]
    fn svg_export_is_transparent_by_default() {
        let doc = Document::new("t", 100.0, 100.0);
        let svg = export_svg(&doc, &SvgExportOptions::default());
        assert!(
            !svg.contains("<rect"),
            "default SVG export should emit no background rect:\n{svg}"
        );
    }

    /// When a background color is requested, a full-artboard rect is emitted.
    #[test]
    fn svg_export_emits_background_rect_when_requested() {
        let doc = Document::new("t", 100.0, 100.0);
        let opts = SvgExportOptions {
            background: Some(Color::WHITE),
            ..Default::default()
        };
        let svg = export_svg(&doc, &opts);
        assert!(svg.contains("<rect"), "expected a background rect:\n{svg}");
        assert!(
            svg.to_lowercase().contains("#ffffff"),
            "expected white background fill:\n{svg}"
        );
    }

    #[test]
    fn blend_mode_css_names_round_trip() {
        use crate::layer::BlendMode::*;
        for mode in [
            Normal, Multiply, Screen, Overlay, Darken, Lighten, ColorDodge, ColorBurn, HardLight,
            SoftLight, Difference, Exclusion, Hue, Saturation, Color, Luminosity,
        ] {
            assert_eq!(BlendMode::from_css(mode.to_css()), Some(mode));
        }
        assert_eq!(BlendMode::from_css("not-a-mode"), None);
    }

    #[test]
    fn blend_mode_survives_svg_round_trip() {
        use crate::node::PathNode;
        use crate::path::PathData;

        let mut doc = Document::new("t", 100.0, 100.0);
        let mut node = SceneNode::new(
            "rect",
            doc.active_layer_id.unwrap(),
            SceneNodeKind::Path(PathNode::new(PathData::rect(0.0, 0.0, 10.0, 10.0))),
        );
        node.blend_mode = BlendMode::Multiply;
        doc.add_node(node, None);

        let svg = export_svg(&doc, &SvgExportOptions::default());
        assert!(
            svg.contains("mix-blend-mode:multiply"),
            "export should emit the CSS blend mode:\n{svg}"
        );

        let reimported = crate::import::import_svg(&svg).expect("re-import");
        let modes: Vec<_> = reimported.nodes.values().map(|n| n.blend_mode).collect();
        assert!(
            modes.contains(&BlendMode::Multiply),
            "blend mode lost on re-import; modes = {modes:?}"
        );
    }

    #[test]
    fn pdf_export_is_a_valid_single_page_pdf() {
        use crate::node::PathNode;
        use crate::path::PathData;
        use crate::style::Fill;

        let mut doc = Document::new("t", 200.0, 150.0);
        let mut rect = PathNode::new(PathData::rect(10.0, 10.0, 80.0, 60.0));
        rect.fill = Fill::solid(Color::new(1.0, 0.0, 0.0, 1.0));
        let node = SceneNode::new(
            "rect",
            doc.active_layer_id.unwrap(),
            SceneNodeKind::Path(rect),
        );
        doc.add_node(node, None);

        let bytes = export_pdf(&doc, &PdfExportOptions::default());
        let text = String::from_utf8_lossy(&bytes);
        assert!(bytes.starts_with(b"%PDF-1"), "missing PDF header");
        assert!(text.contains("%%EOF"), "missing EOF marker");
        assert!(text.contains("/Type /Page"), "missing page object");
        assert!(text.contains("MediaBox"), "missing MediaBox");
        // Red fill colour + a fill-path operator in the (uncompressed) stream.
        assert!(
            text.contains("1 0 0 rg"),
            "missing red fill operator:\n{text}"
        );
        assert!(
            text.contains(" m\n") || text.contains(" m "),
            "missing path move op"
        );
    }

    #[test]
    fn pdf_export_empty_document_is_valid() {
        let doc = Document::new("t", 100.0, 100.0);
        let bytes = export_pdf(&doc, &PdfExportOptions::default());
        assert!(bytes.starts_with(b"%PDF-1"));
        assert!(String::from_utf8_lossy(&bytes).contains("%%EOF"));
    }

    #[test]
    fn pdf_export_approximates_gradient_with_first_stop() {
        use crate::node::PathNode;
        use crate::path::PathData;
        use crate::style::{Fill, FillKind, Gradient, GradientKind, GradientStop};

        let mut doc = Document::new("t", 100.0, 100.0);
        let grad = Gradient {
            kind: GradientKind::Linear,
            stops: vec![
                GradientStop {
                    offset: 0.0,
                    color: Color::new(0.0, 1.0, 0.0, 1.0),
                },
                GradientStop {
                    offset: 1.0,
                    color: Color::new(0.0, 0.0, 1.0, 1.0),
                },
            ],
            coords: vec![0.0, 0.0, 100.0, 0.0],
        };
        let mut p = PathNode::new(PathData::rect(0.0, 0.0, 50.0, 50.0));
        p.fill = Fill {
            kind: FillKind::Gradient(grad),
            opacity: 1.0,
            enabled: true,
        };
        let node = SceneNode::new("g", doc.active_layer_id.unwrap(), SceneNodeKind::Path(p));
        doc.add_node(node, None);

        let text =
            String::from_utf8_lossy(&export_pdf(&doc, &PdfExportOptions::default())).into_owned();
        // First stop is green → "0 1 0 rg".
        assert!(
            text.contains("0 1 0 rg"),
            "expected first-stop green fill:\n{text}"
        );
    }

    #[test]
    fn live_effects_export_svg_filters() {
        use crate::node::PathNode;
        use crate::path::PathData;

        let mut doc = Document::new("t", 100.0, 100.0);
        let mut node = SceneNode::new(
            "rect",
            doc.active_layer_id.unwrap(),
            SceneNodeKind::Path(PathNode::new(PathData::rect(0.0, 0.0, 10.0, 10.0))),
        );
        node.drop_shadow.enabled = true;
        node.drop_shadow.dx = 5.0;
        node.drop_shadow.dy = 6.0;
        node.drop_shadow.blur = 3.0;
        node.object_blur.enabled = true;
        node.object_blur.radius = 2.5;
        doc.add_node(node, None);

        let svg = export_svg(&doc, &SvgExportOptions::default());
        assert!(svg.contains("<filter"), "expected a filter def:\n{svg}");
        assert!(
            svg.contains("<feDropShadow"),
            "expected feDropShadow:\n{svg}"
        );
        assert!(svg.contains("dx=\"5.000\""), "expected shadow dx:\n{svg}");
        assert!(
            svg.contains("<feGaussianBlur"),
            "expected feGaussianBlur:\n{svg}"
        );
        assert!(
            svg.contains("filter=\"url(#fx"),
            "path should reference the filter:\n{svg}"
        );
    }

    #[test]
    fn no_filter_emitted_without_effects() {
        use crate::node::PathNode;
        use crate::path::PathData;

        let mut doc = Document::new("t", 100.0, 100.0);
        let node = SceneNode::new(
            "rect",
            doc.active_layer_id.unwrap(),
            SceneNodeKind::Path(PathNode::new(PathData::rect(0.0, 0.0, 10.0, 10.0))),
        );
        doc.add_node(node, None);
        let svg = export_svg(&doc, &SvgExportOptions::default());
        assert!(!svg.contains("<filter"), "no effects → no filter:\n{svg}");
    }
}
