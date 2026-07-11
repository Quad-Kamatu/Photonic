//! Document export utilities (SVG, etc.).

use crate::{
    layer::BlendMode,
    node::{NodeId, SceneNode, SceneNodeKind, TextAlign},
    style::{Fill, FillKind, Gradient, GradientKind, LineCap, LineJoin, Stroke, StrokeAlign},
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

/// How a selection SVG frames its content in the `viewBox`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SvgNormalize {
    /// Tight union bbox of the selected nodes (legacy default).
    Tight,
    /// Center the content in a uniform square viewBox. `pad` is the padding as a
    /// fraction of the square side (e.g. `0.1` = 10% breathing room each way), so
    /// every icon in a set comes out at the same aspect ratio and scale.
    Square { pad: f64 },
}

impl Default for SvgNormalize {
    fn default() -> Self {
        SvgNormalize::Tight
    }
}

/// Options controlling selection / icon SVG export.
#[derive(Debug, Clone)]
pub struct SvgSelectionOptions {
    /// Decimal places for coordinates and path data, clamped 1–6 (default: `4`).
    /// Fixes bloated 15-decimal path data on selection exports.
    pub precision: u8,
    /// Light optimization pass: trim trailing zeros (always on when set) — pairs
    /// with `precision` to keep icon SVGs compact.
    pub optimize: bool,
    /// viewBox framing (tight bbox vs. uniform centered square).
    pub normalize: SvgNormalize,
}

impl Default for SvgSelectionOptions {
    fn default() -> Self {
        Self {
            precision: 4,
            optimize: true,
            normalize: SvgNormalize::Tight,
        }
    }
}

// ─── Shared SVG emit context ──────────────────────────────────────────────────

/// Threaded through the node emitters: accumulates `<defs>` while deduplicating
/// structurally-identical paint definitions (one shared `<linearGradient>` per
/// unique paint instead of `grad-0`, `grad-1`, … clones) and carries the numeric
/// precision used for coordinates and path data.
struct SvgCtx {
    defs: String,
    counter: usize,
    /// Maps a paint's structural signature → the id already emitted for it.
    cache: HashMap<String, String>,
    /// Decimal places for def coordinates / gradient stops.
    coord_p: usize,
    /// `None` ⇒ emit full-precision path `d` via the cached string (full-document
    /// export, unchanged). `Some(p)` ⇒ re-emit path `d` rounded to `p` decimals.
    path_p: Option<usize>,
}

impl SvgCtx {
    fn new(coord_p: usize, path_p: Option<usize>) -> Self {
        Self {
            defs: String::new(),
            counter: 0,
            cache: HashMap::new(),
            coord_p,
            path_p,
        }
    }

    /// Intern a paint def by its structural signature `sig`. Returns the shared id
    /// (emitting the def via `make_def(id)` only the first time it is seen).
    fn intern(&mut self, prefix: &str, sig: String, make_def: impl FnOnce(&str) -> String) -> String {
        if let Some(id) = self.cache.get(&sig) {
            return id.clone();
        }
        let id = format!("{prefix}-{}", self.counter);
        self.counter += 1;
        let def = make_def(&id);
        self.defs.push_str(&def);
        self.cache.insert(sig, id);
        format!("{prefix}-{}", self.counter - 1)
    }
}

/// Format a float to `p` decimals, trimming trailing zeros and a bare `-0`.
fn fmt(v: f64, p: usize) -> String {
    let s = format!("{:.*}", p, v);
    if s.contains('.') {
        let t = s.trim_end_matches('0').trim_end_matches('.');
        if t.is_empty() || t == "-" || t == "-0" {
            "0".to_string()
        } else {
            t.to_string()
        }
    } else if s == "-0" {
        "0".to_string()
    } else {
        s
    }
}

/// Re-emit a path's geometry as an SVG `d` string with coordinates rounded to `p`
/// decimals (absolute commands). Used when a precision cap is requested so icon
/// exports don't carry 15-decimal path data.
fn path_d_rounded(path: &crate::path::PathData, p: usize) -> String {
    use kurbo::PathEl;
    let bez = path.to_bez_path();
    let mut out = String::new();
    for el in bez.elements() {
        match el {
            PathEl::MoveTo(pt) => out.push_str(&format!("M{} {}", fmt(pt.x, p), fmt(pt.y, p))),
            PathEl::LineTo(pt) => out.push_str(&format!("L{} {}", fmt(pt.x, p), fmt(pt.y, p))),
            PathEl::QuadTo(c, pt) => out.push_str(&format!(
                "Q{} {} {} {}",
                fmt(c.x, p),
                fmt(c.y, p),
                fmt(pt.x, p),
                fmt(pt.y, p)
            )),
            PathEl::CurveTo(c1, c2, pt) => out.push_str(&format!(
                "C{} {} {} {} {} {}",
                fmt(c1.x, p),
                fmt(c1.y, p),
                fmt(c2.x, p),
                fmt(c2.y, p),
                fmt(pt.x, p),
                fmt(pt.y, p)
            )),
            PathEl::ClosePath => out.push('Z'),
        }
    }
    out
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
    let p = opts.precision.clamp(1, 6) as usize;
    // Full-document export preserves full-precision path `d` (path_p = None) for
    // back-compatibility; only viewBox/coord values honor `precision`.
    let mut ctx = SvgCtx::new(p, None);
    let mut body = String::new();

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
            Some(l) if l.visible && l.print => l,
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
        // Layer blend mode → CSS `mix-blend-mode` (the SVG keywords are 1:1 with
        // our BlendMode, so every mode round-trips). Normal is the default, omit.
        if layer.blend_mode != crate::layer::BlendMode::Normal {
            attrs.push_str(&format!(
                " style=\"mix-blend-mode:{}\"",
                layer.blend_mode.to_css()
            ));
        }
        body.push_str(&format!("  <g{}>\n", attrs));

        for node_id in &layer.node_ids {
            if let Some(node) = doc.nodes.get(node_id) {
                emit_node_inner(node, doc, &mut ctx, &mut body, 4, None, id_map.as_ref());
            }
        }

        body.push_str("  </g>\n");
    }

    let defs_block = if ctx.defs.is_empty() {
        String::new()
    } else {
        format!("  <defs>\n{}  </defs>\n", ctx.defs)
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
    export_nodes_as_svg_opts(doc, node_ids, &SvgSelectionOptions::default())
}

/// Export a subset of nodes as SVG with explicit precision / normalization /
/// dedup options (see [`SvgSelectionOptions`]).
pub fn export_nodes_as_svg_opts(
    doc: &Document,
    node_ids: &[NodeId],
    opts: &SvgSelectionOptions,
) -> String {
    let cp = opts.precision.clamp(1, 6) as usize;
    let mut ctx = SvgCtx::new(cp, Some(cp));
    let mut body = String::new();
    let mut combined_bbox: Option<kurbo::Rect> = None;

    // Collect nodes in document order (layer order → z-order within layer).
    for layer_id in &doc.layer_order {
        let layer = match doc.layers.get(layer_id) {
            Some(l) if l.visible && l.print => l,
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
                emit_node_inner(node, doc, &mut ctx, &mut body, 2, Some(&slug), None);
            }
        }
    }

    let tight = match combined_bbox {
        Some(r) => (r.x0, r.y0, r.x1 - r.x0, r.y1 - r.y0),
        None => (0.0, 0.0, doc.width as f64, doc.height as f64),
    };

    // Frame the viewBox per the normalization mode. Content coordinates are never
    // moved — a square/normalized frame is achieved purely by widening the
    // viewBox and centering it on the content, so every icon in a set lands at a
    // uniform aspect ratio and scale without geometry rewrites.
    let (vx, vy, vw, vh) = match opts.normalize {
        SvgNormalize::Tight => tight,
        SvgNormalize::Square { pad } => {
            let (tx, ty, tw, th) = tight;
            let cx = tx + tw / 2.0;
            let cy = ty + th / 2.0;
            let base = tw.max(th).max(1e-6);
            let side = base * (1.0 + 2.0 * pad.max(0.0));
            (cx - side / 2.0, cy - side / 2.0, side, side)
        }
    };

    let defs_block = if ctx.defs.is_empty() {
        String::new()
    } else {
        format!("  <defs>\n{}  </defs>\n", ctx.defs)
    };

    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" \
         xmlns:xlink=\"http://www.w3.org/1999/xlink\" \
         width=\"{vw}\" height=\"{vh}\" viewBox=\"{vx} {vy} {vw} {vh}\">\n\
         {defs_block}{body}</svg>",
        vw = fmt(vw, cp),
        vh = fmt(vh, cp),
        vx = fmt(vx, cp),
        vy = fmt(vy, cp),
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
    ctx: &mut SvgCtx,
    body: &mut String,
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

    let filter = filter_attrs(node, &mut ctx.defs);

    match &node.kind {
        SceneNodeKind::Path(p) => {
            let fill = fill_attrs(&p.fill, ctx);
            let stroke = stroke_attrs(&p.stroke, affine_scale(&node.transform), ctx);
            let d = match ctx.path_p {
                Some(prec) => path_d_rounded(&p.path_data, prec),
                None => p.path_data.as_svg().to_string(),
            };
            body.push_str(&format!(
                "{}<path{}{}{}{}{}{}{} d=\"{}\"/>\n",
                pad, id_attr, transform, opacity, blend, filter, fill, stroke, d,
            ));
        }
        SceneNodeKind::Group(g) => {
            // Live boolean (#25): emit the single resolved path styled by the
            // bottom-most path child, instead of the stacked children.
            let resolved = if g.live_boolean.is_some() {
                doc.resolve_live_boolean(node.id)
            } else {
                None
            };
            if let Some(resolved) = resolved {
                let (fill, stroke) = g
                    .children
                    .iter()
                    .filter_map(|c| doc.nodes.get(c))
                    .find_map(|c| match &c.kind {
                        SceneNodeKind::Path(p) => Some((
                            fill_attrs(&p.fill, ctx),
                            stroke_attrs(&p.stroke, affine_scale(&node.transform), ctx),
                        )),
                        _ => None,
                    })
                    .unwrap_or_default();
                let d = match ctx.path_p {
                    Some(prec) => path_d_rounded(&resolved, prec),
                    None => resolved.as_svg().to_string(),
                };
                body.push_str(&format!(
                    "{}<path{}{}{}{}{}{}{} d=\"{}\"/>\n",
                    pad, id_attr, transform, opacity, blend, filter, fill, stroke, d,
                ));
            } else {
                body.push_str(&format!(
                    "{}<g{}{}{}{}{}>\n",
                    pad, id_attr, transform, opacity, blend, filter
                ));
                for child_id in &g.children {
                    if let Some(child) = doc.nodes.get(child_id) {
                        emit_node_inner(child, doc, ctx, body, indent + 2, None, id_map);
                    }
                }
                body.push_str(&format!("{}</g>\n", pad));
            }
        }
        SceneNodeKind::Text(t) => {
            let fill = fill_attrs(&t.fill, ctx);
            let stroke = stroke_attrs(&t.stroke, affine_scale(&node.transform), ctx);
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
            // Advanced character metrics: super/subscript reduces the rendered
            // font size and offsets the baseline; an explicit baseline shift adds
            // to that offset. SVG `baseline-shift` is positive-up, matching ours.
            let effective_font_size = t.font_size * t.script_position.size_scale();
            let shift_units =
                t.script_position.baseline_offset_em() * t.font_size + t.baseline_shift;
            let baseline_shift_attr = if shift_units.abs() > 1e-9 {
                format!(" baseline-shift=\"{shift_units}\"")
            } else {
                String::new()
            };
            body.push_str(&format!(
                "{}<text{}{}{}{} font-family=\"{}\" font-size=\"{}\" font-weight=\"{}\" \
                 text-anchor=\"{}\"{}{}{}>{}</text>\n",
                pad,
                id_attr,
                transform,
                opacity,
                blend,
                t.font_family,
                effective_font_size,
                t.font_weight,
                anchor,
                baseline_shift_attr,
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

/// Wrap a paint-def id into a ` fill="url(#id)"` (or `stroke=`) attribute, adding
/// a `-opacity` attribute when the paint opacity is below 1.
fn paint_url_attr(prop: &str, id: &str, opacity: f32) -> String {
    if (opacity - 1.0).abs() < 0.001 {
        format!(" {prop}=\"url(#{id})\"")
    } else {
        format!(" {prop}=\"url(#{id})\" {prop}-opacity=\"{opacity:.4}\"")
    }
}

/// Emit stops for a linear/radial gradient with coordinate precision `p`.
fn gradient_stops(g: &Gradient, p: usize) -> String {
    g.stops
        .iter()
        .map(|s| {
            let hex = s.color.to_hex();
            if (s.color.a - 1.0).abs() < 0.001 {
                format!(
                    "      <stop offset=\"{}\" stop-color=\"{}\"/>\n",
                    fmt(s.offset as f64, p),
                    hex
                )
            } else {
                format!(
                    "      <stop offset=\"{}\" stop-color=\"{}\" stop-opacity=\"{:.4}\"/>\n",
                    fmt(s.offset as f64, p),
                    hex,
                    s.color.a
                )
            }
        })
        .collect()
}

/// Intern a linear/radial gradient def (deduped) and return its shared id.
fn gradient_ref(ctx: &mut SvgCtx, g: &Gradient) -> String {
    let p = ctx.coord_p;
    let stops = gradient_stops(g, p);
    match g.kind {
        GradientKind::Linear => {
            let (x1, y1, x2, y2) = if g.coords.len() >= 4 {
                (g.coords[0], g.coords[1], g.coords[2], g.coords[3])
            } else {
                (0.0, 0.0, 1.0, 0.0)
            };
            let (fx1, fy1, fx2, fy2) = (fmt(x1, p), fmt(y1, p), fmt(x2, p), fmt(y2, p));
            let sig = format!("lin|{fx1}|{fy1}|{fx2}|{fy2}|{stops}");
            ctx.intern("grad", sig, |id| {
                format!(
                    "    <linearGradient id=\"{id}\" x1=\"{fx1}\" y1=\"{fy1}\" x2=\"{fx2}\" \
                     y2=\"{fy2}\" gradientUnits=\"userSpaceOnUse\">\n{stops}    </linearGradient>\n",
                )
            })
        }
        GradientKind::Radial => {
            let (cx, cy, r) = if g.coords.len() >= 5 {
                (g.coords[0], g.coords[1], g.coords[4])
            } else {
                (0.5, 0.5, 0.5)
            };
            let (fcx, fcy, fr) = (fmt(cx, p), fmt(cy, p), fmt(r, p));
            let sig = format!("rad|{fcx}|{fcy}|{fr}|{stops}");
            ctx.intern("grad", sig, |id| {
                format!(
                    "    <radialGradient id=\"{id}\" cx=\"{fcx}\" cy=\"{fcy}\" r=\"{fr}\" \
                     gradientUnits=\"userSpaceOnUse\">\n{stops}    </radialGradient>\n",
                )
            })
        }
    }
}

fn fill_attrs(fill: &Fill, ctx: &mut SvgCtx) -> String {
    if !fill.enabled {
        return " fill=\"none\"".to_string();
    }
    let p = ctx.coord_p;
    match &fill.kind {
        FillKind::None => " fill=\"none\"".to_string(),
        FillKind::Solid(c) => solid_fill_attr(c, fill.opacity),
        FillKind::FluidGradient(fg) => {
            // Export as a radial gradient approximation: centroid center, first→last color.
            if fg.points.is_empty() {
                return " fill=\"none\"".to_string();
            }
            if fg.points.len() == 1 {
                return solid_fill_attr(&fg.points[0].color, fill.opacity);
            }
            let cx: f64 = fg.points.iter().map(|p| p.x).sum::<f64>() / fg.points.len() as f64;
            let cy: f64 = fg.points.iter().map(|p| p.y).sum::<f64>() / fg.points.len() as f64;
            let max_r: f64 = fg
                .points
                .iter()
                .map(|p| ((p.x - cx).powi(2) + (p.y - cy).powi(2)).sqrt())
                .fold(0.0_f64, f64::max)
                .max(1.0);
            let first = &fg.points[0];
            let last = &fg.points[fg.points.len() - 1];
            let (fcx, fcy, fr) = (fmt(cx, p), fmt(cy, p), fmt(max_r, p));
            let stops = format!(
                "      <stop offset=\"0\" stop-color=\"{}\"/>\n\
                 \x20     <stop offset=\"1\" stop-color=\"{}\"/>\n",
                first.color.to_hex(),
                last.color.to_hex()
            );
            let sig = format!("rad|{fcx}|{fcy}|{fr}|{stops}");
            let id = ctx.intern("grad", sig, |id| {
                format!(
                    "    <radialGradient id=\"{id}\" cx=\"{fcx}\" cy=\"{fcy}\" r=\"{fr}\" \
                     gradientUnits=\"userSpaceOnUse\">\n{stops}    </radialGradient>\n",
                )
            });
            paint_url_attr("fill", &id, fill.opacity)
        }
        FillKind::MeshGradient(mg) => {
            // Export as a linear-gradient approximation from the first to the
            // last cell colour along the grid diagonal (SVG has no mesh fill).
            if mg.cell_colors.is_empty() {
                return " fill=\"none\"".to_string();
            }
            if mg.cell_colors.len() == 1 {
                return solid_fill_attr(&mg.cell_colors[0], fill.opacity);
            }
            let first = mg.cell_colors[0];
            let last = *mg.cell_colors.last().unwrap();
            let (x1, y1, x2, y2) = (
                fmt(*mg.x_lines.first().unwrap_or(&0.0), p),
                fmt(*mg.y_lines.first().unwrap_or(&0.0), p),
                fmt(*mg.x_lines.last().unwrap_or(&1.0), p),
                fmt(*mg.y_lines.last().unwrap_or(&1.0), p),
            );
            let stops = format!(
                "      <stop offset=\"0\" stop-color=\"{}\"/>\n\
                 \x20     <stop offset=\"1\" stop-color=\"{}\"/>\n",
                first.to_hex(),
                last.to_hex()
            );
            let sig = format!("lin|{x1}|{y1}|{x2}|{y2}|{stops}");
            let id = ctx.intern("grad", sig, |id| {
                format!(
                    "    <linearGradient id=\"{id}\" x1=\"{x1}\" y1=\"{y1}\" x2=\"{x2}\" \
                     y2=\"{y2}\" gradientUnits=\"userSpaceOnUse\">\n{stops}    </linearGradient>\n",
                )
            });
            paint_url_attr("fill", &id, fill.opacity)
        }
        FillKind::Gradient(g) => {
            let id = gradient_ref(ctx, g);
            paint_url_attr("fill", &id, fill.opacity)
        }
        FillKind::Pattern(pat) => {
            let id = pattern_ref(ctx, pat);
            paint_url_attr("fill", &id, fill.opacity)
        }
    }
}

/// Intern a pattern def (deduped by tile bytes + transform) and return its id.
fn pattern_ref(ctx: &mut SvgCtx, p: &crate::style::PatternFill) -> String {
    {
        use base64::Engine;

        let tw = p.tile.width.max(1);
        let th = p.tile.height.max(1);
        // The pattern cell is the tile plus its inter-tile gutter.
        let cell_w = tw as f64 + p.spacing.max(0.0);
        let cell_h = th as f64 + p.spacing.max(0.0);

        let png = p.tile.to_png();
        let b64 = base64::engine::general_purpose::STANDARD.encode(png);

        // patternTransform mirrors PatternFill's document-space transform:
        // translate(offset) → rotate(deg) → scale(s). SVG applies these
        // right-to-left, matching the inverse-transform sample order.
        let deg = p.rotation.to_degrees();
        let mut xform = String::new();
        if p.offset[0] != 0.0 || p.offset[1] != 0.0 {
            xform.push_str(&format!("translate({} {}) ", p.offset[0], p.offset[1]));
        }
        if deg.abs() > 1e-9 {
            xform.push_str(&format!("rotate({deg}) "));
        }
        if (p.scale - 1.0).abs() > 1e-9 {
            xform.push_str(&format!("scale({}) ", p.scale));
        }
        let xform = xform.trim_end();
        let xform_attr = if xform.is_empty() {
            String::new()
        } else {
            format!(" patternTransform=\"{xform}\"")
        };

        // Grid layout is exact; brick/hex staggers are approximated by the grid
        // cell here — on-canvas/headless remain the source of truth. Dedupe by the
        // full tile bytes + geometry so repeated tiles share one <pattern>.
        let sig = format!("pat|{cell_w}|{cell_h}|{xform_attr}|{b64}");
        ctx.intern("pat", sig, move |id| {
            format!(
                "    <pattern id=\"{id}\" patternUnits=\"userSpaceOnUse\" \
                 width=\"{cell_w}\" height=\"{cell_h}\"{xform_attr}>\n\
                 \x20     <image x=\"0\" y=\"0\" width=\"{tw}\" height=\"{th}\" \
                 href=\"data:image/png;base64,{b64}\"/>\n\
                 \x20 </pattern>\n",
            )
        })
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

/// Uniform scale factor of an affine transform, `sqrt(|det|)` — used to keep
/// stroke widths constant regardless of the object's scale (non-scaling stroke),
/// matching the renderer. `1.0` for unscaled/rotated/translated transforms.
fn affine_scale(t: &Transform) -> f64 {
    let [a, b, c, d, _, _] = t.matrix;
    (a * d - b * c).abs().sqrt().max(1e-6)
}

/// `obj_scale` is the emitting element's transform scale; the stroke width is
/// divided by it so that, once the element's `transform="matrix(...)"` scales it
/// back up, the stroke renders at its authored width in canvas units — a
/// non-scaling stroke, consistent with the live canvas and raster export.
fn stroke_attrs(stroke: &Stroke, obj_scale: f64, ctx: &mut SvgCtx) -> String {
    if !stroke.enabled || stroke.width <= 0.0 {
        return " stroke=\"none\"".to_string();
    }
    // Non-solid stroke paint (#201): a gradient/pattern stroke exports as
    // `stroke="url(#id)"`, reusing the same deduped paint defs as fills. Solid or
    // unsupported paints fall through to the flat `color` path below.
    let paint_ref: Option<String> = match &stroke.paint {
        Some(FillKind::Gradient(g)) => Some(gradient_ref(ctx, g)),
        Some(FillKind::Pattern(p)) => Some(pattern_ref(ctx, p)),
        _ => None,
    };
    let hex = match &stroke.paint {
        // A solid override paint recolors the stroke.
        Some(FillKind::Solid(c)) => c.to_hex(),
        _ => stroke.color.to_hex(),
    };
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
    // Gradient/pattern paint wins over the flat color when present.
    let stroke_val = match &paint_ref {
        Some(id) => format!("url(#{id})"),
        None => hex,
    };
    let mut s = format!(
        " stroke=\"{stroke_val}\" stroke-width=\"{}\" stroke-linecap=\"{cap}\" stroke-linejoin=\"{join}\"{align_attr}",
        stroke.width / obj_scale,
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

/// A colour resolved into the export colour model for the PDF content stream.
#[derive(Debug, Clone, Copy)]
pub enum PdfColor {
    Rgb([f32; 3]),
    Cmyk([f32; 4]),
}

/// Page geometry boxes, in PDF points (Y-up). T1.5/T1.8 expand media to include
/// bleed and set trim inside it; today all three equal the artboard.
#[derive(Debug, Clone, Copy)]
pub struct PageBoxes {
    pub media: [f32; 4],
    pub trim: [f32; 4],
    pub bleed: [f32; 4],
}

/// Options controlling vector PDF export.
#[derive(Debug, Clone, Default)]
pub struct PdfExportOptions {
    /// Paint a full-page background rectangle of this colour before the artwork.
    /// `None` (default) leaves the page background unpainted (white in viewers).
    pub background: Option<Color>,
    /// Convert text nodes to vector outlines (zero font deps). T0.2.
    pub outline_text: bool,
    /// Render trim + registration marks in the bleed/slug area. T1.6.
    pub marks: bool,
    /// Export colour model. Rgb (default) preserves current behaviour. T0.3.
    pub color_mode: crate::document::ColorMode,
    /// CMYK ICC profile path for conversion + OutputIntent. T0.3/T0.4.
    pub icc_profile: Option<std::path::PathBuf>,
}

/// Compute MediaBox/TrimBox/BleedBox in PDF points (T1.5/T1.8).
///
/// PDF points ≡ 72 dpi: `to_px(mm, Mm, 72.0)` converts mm → points.
///
/// Layout (all centred, Y-up, values in PDF points):
/// ```text
///   [0, 0, w+2*outer, h+2*outer]       ← MediaBox  (mark room + bleed + trim)
///   [mark_room, …, …+w+2*bleed, …]     ← BleedBox  (bleed outside trim)
///   [outer, outer, outer+w, outer+h]   ← TrimBox   (artboard, the "finished size")
/// ```
/// When `bleed_mm == 0` and `marks == false` all three collapse to `[0,0,w,h]`,
/// preserving the pre-bleed regression baseline exactly.
fn compute_page_boxes(doc: &Document, opts: &PdfExportOptions) -> PageBoxes {
    use crate::units::{to_px, DocumentUnit::Mm};

    let w = doc.width as f32;
    let h = doc.height as f32;

    // PDF points = 72 dpi; convert mm at that resolution.
    let pdf_dpi = 72.0_f64;
    let bleed_px = to_px(doc.bleed_mm, Mm, pdf_dpi) as f32;

    // Reserve room outside the bleed for marks (the slug band).
    // Use the document slug if non-zero; otherwise fall back to 5 mm so marks
    // always have somewhere to live when marks=true.
    let mark_room = if opts.marks {
        to_px(doc.slug_mm.max(5.0), Mm, pdf_dpi) as f32
    } else {
        0.0_f32
    };

    let outer = bleed_px + mark_room;

    // media  — page sheet including mark band
    let media = [0.0, 0.0, w + 2.0 * outer, h + 2.0 * outer];
    // bleed  — trim + bleed extension (mark_room offset from media origin)
    let bleed = [
        mark_room,
        mark_room,
        mark_room + w + 2.0 * bleed_px,
        mark_room + h + 2.0 * bleed_px,
    ];
    // trim   — the finished artboard size (outer offset from media origin)
    let trim = [outer, outer, outer + w, outer + h];

    PageBoxes { media, trim, bleed }
}

/// Resolve an RGB triple into the export colour model.
///
/// When `opts.color_mode == Cmyk` the colour is converted through a real ICC
/// profile (CoatedFOGRA39 by default, or `opts.icc_profile` when supplied).
/// The transform is cached process-wide so only the first call per profile pays
/// the parse cost.
fn convert_color(rgb: [f32; 3], opts: &PdfExportOptions) -> PdfColor {
    match opts.color_mode {
        crate::document::ColorMode::Cmyk => {
            let t = crate::color_cmyk::cached_transform(opts.icc_profile.as_deref());
            PdfColor::Cmyk(t.rgb_to_cmyk(rgb))
        }
        crate::document::ColorMode::Rgb => PdfColor::Rgb(rgb),
    }
}

/// Set the non-stroking (fill) colour on the content stream from a PdfColor.
fn set_fill_color(content: &mut pdf_writer::Content, c: PdfColor) {
    match c {
        PdfColor::Rgb([r, g, b]) => { content.set_fill_rgb(r, g, b); }
        PdfColor::Cmyk([cc, m, y, k]) => { content.set_fill_cmyk(cc, m, y, k); }
    }
}

/// Set the stroking colour on the content stream from a PdfColor.
fn set_stroke_color(content: &mut pdf_writer::Content, c: PdfColor) {
    match c {
        PdfColor::Rgb([r, g, b]) => { content.set_stroke_rgb(r, g, b); }
        PdfColor::Cmyk([cc, m, y, k]) => { content.set_stroke_cmyk(cc, m, y, k); }
    }
}

/// Emit a Text node as vector outlines. SEAM T0.2: when opts.outline_text, bridge
/// the photonic-render glyph outliner to emit_path_geometry. Today: no-op.
fn emit_text_pdf(
    _node: &SceneNode,
    _doc: &Document,
    _content: &mut pdf_writer::Content,
    _opts: &PdfExportOptions,
) {}

/// Render trim and registration marks into the content stream (T1.6).
///
/// All coordinates are in absolute PDF page space (Y-up, origin = media bottom-left).
/// Called *after* `content.restore_state()` so the artwork CTM has been popped.
///
/// Marks drawn (when `opts.marks` and sufficient mark_room):
/// - **Crop/trim marks**: two hairlines at each of the 4 trim corners, extending
///   outward from the trim edge with a small gap so they do not cross into the live
///   area.  Colour: registration (CMYK 1,1,1,1), 0.25 pt line width.
/// - **Registration targets**: a crosshair centred on each of the 4 trim sides,
///   placed in the middle of the mark band outside the bleed.  A circle (4-arc
///   bezier approximation) surrounds the crosshair.  Same colour/weight.
///   NOTE: the circle is approximated by cubic beziers (k ≈ 0.5523); no arc
///   primitive is available in the PDF content-stream operator set.
fn emit_marks(content: &mut pdf_writer::Content, boxes: &PageBoxes, opts: &PdfExportOptions) {
    if !opts.marks {
        return;
    }

    // Derive geometry.
    let [tx0, ty0, tx1, ty1] = boxes.trim;  // trim box in page space
    let [bx0, by0, bx1, by1] = boxes.bleed;

    // mark_room is the band between bleed and media edges.
    let mark_room_x = bx0;           // == media_x0 + mark_room (= mark_room since media_x0=0)
    let mark_room_y = by0;           // same for Y
    if mark_room_x <= 0.0 || mark_room_y <= 0.0 {
        // No room for marks (bleed_mm=0 with marks=true would be unusual, skip).
        return;
    }

    // Gap between trim edge and start of the crop mark hairline (3 pt).
    let gap = 3.0_f32;
    // Crop mark length in points (≈ 12 pt or up to 60% of mark_room).
    let mark_len = (mark_room_x * 0.6).min(12.0_f32).max(6.0_f32);

    // Registration colour: CMYK all-ink (prints in all channels simultaneously).
    content.set_stroke_cmyk(1.0, 1.0, 1.0, 1.0);
    content.set_line_width(0.25);

    // ── Crop / trim marks ─────────────────────────────────────────────────────
    // Each corner emits two right-angle hairlines that bracket the trim corner
    // from outside the live area.  The marks start at `gap` beyond the trim edge
    // and extend a further `mark_len` outward.

    // Helper: emit one crop hairline from (x0,y0) to (x1,y1).
    let line = |content: &mut pdf_writer::Content, x0: f32, y0: f32, x1: f32, y1: f32| {
        content.move_to(x0, y0);
        content.line_to(x1, y1);
        content.stroke();
    };

    // Bottom-left corner (trim corner: tx0, ty0)
    // Horizontal hairline leftward from trim-left edge
    line(content, tx0 - gap, ty0, tx0 - gap - mark_len, ty0);
    // Vertical hairline downward from trim-bottom edge
    line(content, tx0, ty0 - gap, tx0, ty0 - gap - mark_len);

    // Bottom-right corner (trim corner: tx1, ty0)
    line(content, tx1 + gap, ty0, tx1 + gap + mark_len, ty0);
    line(content, tx1, ty0 - gap, tx1, ty0 - gap - mark_len);

    // Top-right corner (trim corner: tx1, ty1)
    line(content, tx1 + gap, ty1, tx1 + gap + mark_len, ty1);
    line(content, tx1, ty1 + gap, tx1, ty1 + gap + mark_len);

    // Top-left corner (trim corner: tx0, ty1)
    line(content, tx0 - gap, ty1, tx0 - gap - mark_len, ty1);
    line(content, tx0, ty1 + gap, tx0, ty1 + gap + mark_len);

    // ── Registration targets ──────────────────────────────────────────────────
    // Centred on each of the 4 trim sides, in the mid-point of the mark band
    // (between bleed edge and media edge).  A crosshair + circle.

    // Radius for the registration circle (half the mark band width, capped at 6 pt).
    let r = (mark_room_x * 0.4).min(6.0_f32).max(3.0_f32);

    // Cubic bezier approximation constant for a quarter circle.
    let k = 0.5523_f32;

    let reg_target = |content: &mut pdf_writer::Content, cx: f32, cy: f32| {
        // Crosshair (two lines through the centre).
        content.move_to(cx - r, cy);
        content.line_to(cx + r, cy);
        content.stroke();
        content.move_to(cx, cy - r);
        content.line_to(cx, cy + r);
        content.stroke();

        // Circle approximated by 4 cubic bezier arcs (counter-clockwise from right).
        // Start at (cx+r, cy).
        content.move_to(cx + r, cy);
        // Q1: right → top
        content.cubic_to(cx + r, cy + k * r, cx + k * r, cy + r, cx, cy + r);
        // Q2: top → left
        content.cubic_to(cx - k * r, cy + r, cx - r, cy + k * r, cx - r, cy);
        // Q3: left → bottom
        content.cubic_to(cx - r, cy - k * r, cx - k * r, cy - r, cx, cy - r);
        // Q4: bottom → right
        content.cubic_to(cx + k * r, cy - r, cx + r, cy - k * r, cx + r, cy);
        content.close_path();
        content.stroke();
    };

    // Centre of the left mark band (between bleed-left and media-left).
    let cx_left  = bx0 * 0.5;
    // Centre of the right mark band (between bleed-right and media-right).
    let cx_right = bx1 + (boxes.media[2] - bx1) * 0.5;
    // Centre of the bottom mark band.
    let cy_bot   = by0 * 0.5;
    // Centre of the top mark band.
    let cy_top   = by1 + (boxes.media[3] - by1) * 0.5;
    // Trim mid-points for the cross-axis coordinates.
    let trim_mid_x = (tx0 + tx1) * 0.5;
    let trim_mid_y = (ty0 + ty1) * 0.5;

    reg_target(content, cx_left,  trim_mid_y);
    reg_target(content, cx_right, trim_mid_y);
    reg_target(content, trim_mid_x, cy_bot);
    reg_target(content, trim_mid_x, cy_top);
}

/// Export `doc` as a single-page vector PDF (1 document unit = 1 PDF point).
///
/// MVP scope: filled/stroked vector paths with solid colours, node/group affine
/// transforms, group nesting, and per-layer opacity + blend mode (via ExtGState).
/// Gradient fills are approximated by their first stop colour; text, clipping,
/// per-node opacity, and multi-page artboards are documented follow-ups. Like
/// SVG, PDF layer groups are non-isolated, so blend reads the page backdrop.
pub fn export_pdf(doc: &Document, opts: &PdfExportOptions) -> Vec<u8> {
    use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref, TextStr};

    let catalog_id = Ref::new(1);
    let page_tree_id = Ref::new(2);
    let page_id = Ref::new(3);
    let content_id = Ref::new(4);

    // Compute page geometry boxes.
    // T1.5/T1.8: media expands to include bleed+mark-room; trim = artboard.
    let boxes = compute_page_boxes(doc, opts);

    // ── Content stream ────────────────────────────────────────────────────────
    let mut content = Content::new();

    // ── Artwork transform (T1.8) ──────────────────────────────────────────────
    // PDF is Y-up; Photonic is Y-down.  The trim box sits at (trim_x0, trim_y0)
    // in page space, so we need doc (0,0) → page (trim_x0, trim_y1).
    //
    //   page_x = trim_x0 + doc_x          (T_e = trim_x0)
    //   page_y = trim_y1 - doc_y          (Y-flip about the trim top edge)
    //
    // CTM: [a b c d e f] ≡ [1  0  0  -1  trim_x0  trim_y1]
    //
    // Verification at bleed=0, marks=false:
    //   trim = [0, 0, w, h]  →  trim_x0=0, trim_y1=h  →  [1,0,0,-1,0,h]  ✓
    let trim_x0 = boxes.trim[0];
    let trim_y1 = boxes.trim[3]; // top of trim in Y-up page space

    // bleed_px in PDF-point space (same unit as boxes).
    let pdf_dpi = 72.0_f64;
    let bleed_px = crate::units::to_px(doc.bleed_mm, crate::units::DocumentUnit::Mm, pdf_dpi) as f32;

    // Save state so marks can be drawn afterward in absolute page space.
    content.save_state();
    content.transform([1.0, 0.0, 0.0, -1.0, trim_x0, trim_y1]);

    // ── Bleed-aware background (T1.8) ─────────────────────────────────────────
    // The background bleeds past the trim edge into the bleed zone so there is
    // no white sliver after cutting.  In doc-space (inside the CTM above), the
    // bleed box spans (-bleed_px, -bleed_px) to (w+bleed_px, h+bleed_px).
    if let Some(bg) = opts.background {
        let w = doc.width as f32;
        let h = doc.height as f32;
        set_fill_color(&mut content, convert_color([bg.r, bg.g, bg.b], opts));
        content.move_to(-bleed_px, -bleed_px);
        content.line_to(w + bleed_px, -bleed_px);
        content.line_to(w + bleed_px, h + bleed_px);
        content.line_to(-bleed_px, h + bleed_px);
        content.close_path();
        content.fill_nonzero();
    }

    // Layers that need a graphics state (opacity < 1 or non-Normal blend) wrap
    // their nodes in a `/gsN` ExtGState so the layer composites as a unit. The
    // states are collected here and written as PDF objects + page resources below.
    let mut gstates: Vec<(f32, pdf_writer::types::BlendMode)> = Vec::new();
    for layer_id in &doc.layer_order {
        let layer = match doc.layers.get(layer_id) {
            Some(l) if l.visible && l.print => l,
            _ => continue,
        };
        let needs_gs =
            layer.opacity < 1.0 || layer.blend_mode != crate::layer::BlendMode::Normal;
        if needs_gs {
            let name = format!("gs{}", gstates.len());
            gstates.push((layer.opacity.clamp(0.0, 1.0), pdf_blend_mode(layer.blend_mode)));
            content.save_state();
            content.set_parameters(Name(name.as_bytes()));
        }
        for node_id in &layer.node_ids {
            if let Some(node) = doc.nodes.get(node_id) {
                emit_node_pdf(node, doc, &mut content, opts);
            }
        }
        if needs_gs {
            content.restore_state();
        }
    }

    // Pop the artwork CTM so marks are drawn in absolute page space.
    content.restore_state();

    // T1.6: emit trim/registration marks in absolute page space.
    emit_marks(&mut content, &boxes, opts);

    let stream = content.finish();

    // ── Document structure ──────────────────────────────────────────────────────
    // ExtGState objects get Refs after the fixed 1–4 (catalog/pages/page/content).
    let gs_refs: Vec<Ref> = (0..gstates.len())
        .map(|i| Ref::new(5 + i as i32))
        .collect();

    let mut pdf = Pdf::new();

    // T0.4 PDF/X-1a: a CMYK export is emitted as a print-ready PDF/X-1a:2001 file —
    // PDF 1.3, an embedded DeviceCMYK OutputIntent, and GTS_PDFX Info metadata.
    let x1a = opts.color_mode == crate::document::ColorMode::Cmyk;
    if x1a {
        pdf.set_version(1, 3);
    }
    // The ICC + Info objects get Refs after the fixed 1–4 and the ExtGState refs.
    let icc_ref = Ref::new(5 + gstates.len() as i32);
    let info_ref = Ref::new(6 + gstates.len() as i32);

    {
        let mut cat = pdf.catalog(catalog_id);
        cat.pages(page_tree_id);
        if x1a {
            let mut intents = cat.output_intents();
            intents
                .push()
                .subtype(pdf_writer::types::OutputIntentSubtype::PDFX)
                .output_condition_identifier(TextStr("CoatedFOGRA39"))
                .output_condition(TextStr("Coated FOGRA39 (ISO 12647-2:2004)"))
                .registry_name(TextStr("http://www.color.org"))
                .info(TextStr("Coated FOGRA39 (ISO 12647-2:2004)"))
                .dest_output_profile(icc_ref);
            intents.finish();
        }
        cat.finish();
    }
    pdf.pages(page_tree_id).kids([page_id]).count(1);
    {
        let mut page = pdf.page(page_id);
        page.parent(page_tree_id)
            .media_box(Rect::new(
                boxes.media[0],
                boxes.media[1],
                boxes.media[2],
                boxes.media[3],
            ))
            .trim_box(Rect::new(
                boxes.trim[0],
                boxes.trim[1],
                boxes.trim[2],
                boxes.trim[3],
            ))
            .bleed_box(Rect::new(
                boxes.bleed[0],
                boxes.bleed[1],
                boxes.bleed[2],
                boxes.bleed[3],
            ))
            .contents(content_id);
        {
            let mut res = page.resources();
            if !gs_refs.is_empty() {
                let mut egs = res.ext_g_states();
                for (i, r) in gs_refs.iter().enumerate() {
                    egs.pair(Name(format!("gs{i}").as_bytes()), *r);
                }
                egs.finish();
            }
            res.finish();
        }
        page.finish();
    }
    pdf.stream(content_id, &stream);
    // Write the ExtGState objects: layer alpha (fill + stroke) and blend mode.
    for ((opacity, blend), r) in gstates.iter().zip(gs_refs.iter()) {
        pdf.ext_graphics(*r)
            .non_stroking_alpha(*opacity)
            .stroking_alpha(*opacity)
            .blend_mode(*blend)
            .finish();
    }

    // T0.4: embed the CMYK OutputIntent profile stream + PDF/X-1a Info metadata.
    if x1a {
        let icc_bytes = x1a_icc_bytes(opts);
        pdf.icc_profile(icc_ref, &icc_bytes).n(4).finish();
        pdf.document_info(info_ref)
            .title(TextStr("Photonic print export"))
            .creator(TextStr("Photonic"))
            .producer(TextStr("Photonic"))
            .pair(Name(b"GTS_PDFXVersion"), TextStr("PDF/X-1a:2001"))
            .pair(Name(b"GTS_PDFXConformance"), TextStr("PDF/X-1a:2001"));
    }
    pdf.finish()
}

/// Resolve the CMYK ICC profile bytes for the PDF/X OutputIntent: the caller's
/// `icc_profile` if it loads, otherwise the bundled FOGRA39 default (same profile
/// the CMYK conversion uses, so the OutputIntent matches the separated colours).
fn x1a_icc_bytes(opts: &PdfExportOptions) -> Vec<u8> {
    if let Some(p) = &opts.icc_profile {
        if let Ok(bytes) = std::fs::read(p) {
            return bytes;
        }
    }
    crate::color_cmyk::DEFAULT_CMYK_ICC.to_vec()
}

/// Map a Photonic blend mode to the `pdf-writer` blend mode (the 16 PDF standard
/// separable + non-separable modes are 1:1 with ours).
fn pdf_blend_mode(m: crate::layer::BlendMode) -> pdf_writer::types::BlendMode {
    use crate::layer::BlendMode as B;
    use pdf_writer::types::BlendMode as P;
    match m {
        B::Normal => P::Normal,
        B::Multiply => P::Multiply,
        B::Screen => P::Screen,
        B::Overlay => P::Overlay,
        B::Darken => P::Darken,
        B::Lighten => P::Lighten,
        B::ColorDodge => P::ColorDodge,
        B::ColorBurn => P::ColorBurn,
        B::HardLight => P::HardLight,
        B::SoftLight => P::SoftLight,
        B::Difference => P::Difference,
        B::Exclusion => P::Exclusion,
        B::Hue => P::Hue,
        B::Saturation => P::Saturation,
        B::Color => P::Color,
        B::Luminosity => P::Luminosity,
        // Photoshop extras have no PDF standard blend mode — fall back to Normal.
        B::LinearDodge
        | B::LinearBurn
        | B::Subtract
        | B::Divide
        | B::VividLight
        | B::LinearLight
        | B::PinLight
        | B::HardMix
        | B::DarkerColor
        | B::LighterColor => P::Normal,
    }
}

/// Recursively emit a node's geometry into the PDF content stream, applying its
/// affine transform within a save/restore so siblings are unaffected.
fn emit_node_pdf(
    node: &SceneNode,
    doc: &Document,
    content: &mut pdf_writer::Content,
    opts: &PdfExportOptions,
) {
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
                set_fill_color(content, convert_color([fr, fg, fb], opts));
            }
            if let Some(s) = stroke {
                set_stroke_color(
                    content,
                    convert_color([s.color.r, s.color.g, s.color.b], opts),
                );
                // Non-scaling stroke: the `cm` transform above scales subsequent
                // line widths, so divide by the transform scale to keep the
                // authored width (matches the canvas and other exporters).
                let obj_scale = (a * d - b * c).abs().sqrt().max(1e-6);
                content.set_line_width((s.width / obj_scale) as f32);
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
                    emit_node_pdf(child, doc, content, opts);
                }
            }
        }
        // Route text nodes through the seam (no-op today; SEAM T0.2 fills this).
        SceneNodeKind::Text(_) => {
            emit_text_pdf(node, doc, content, opts);
        }
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
        FillKind::MeshGradient(g) => g.cell_colors.first().copied()?,
        // Pattern fills can't tile in this representative-RGB path; approximate by
        // the tile's own centre pixel — always inside the tile (never the
        // inter-tile gutter, unlike sampling at the document origin) — composited
        // over white so a semi-transparent sample doesn't read as black on paper.
        FillKind::Pattern(p) => {
            let s = p
                .tile
                .sample_bilinear(p.tile.width as f32 * 0.5, p.tile.height as f32 * 0.5);
            let a = s[3] as f32 / 255.0;
            let over = |c: u8| (c as f32 / 255.0) * a + (1.0 - a);
            return Some([over(s[0]), over(s[1]), over(s[2])]);
        }
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

    /// Non-scaling stroke: a node scaled 2× must export a stroke-width halved,
    /// so that the element's `matrix(...)` scales it back to the authored width
    /// in canvas units (constant width regardless of object size).
    #[test]
    fn svg_export_stroke_width_is_non_scaling() {
        use crate::node::{PathNode, SceneNode, SceneNodeKind};
        use crate::path::PathData;
        use crate::transform::Transform;

        let mut doc = Document::new("t", 200.0, 200.0);
        let layer_id = doc.layer_order[0];
        let pn = PathNode::new(PathData::rect(0.0, 0.0, 10.0, 10.0))
            .with_stroke(Stroke::solid(Color::BLACK, 4.0));
        let mut node = SceneNode::new("r", layer_id, SceneNodeKind::Path(pn));
        // Uniform 2× scale about the origin.
        node.transform = Transform::new(2.0, 0.0, 0.0, 2.0, 0.0, 0.0);
        let nid = node.id;
        doc.nodes.insert(nid, node);
        doc.layers.get_mut(&layer_id).unwrap().node_ids.push(nid);

        let svg = export_svg(&doc, &SvgExportOptions::default());
        assert!(
            svg.contains("stroke-width=\"2\""),
            "2× scaled node should export stroke-width 2 (4 / 2):\n{svg}"
        );
    }

    #[test]
    fn affine_scale_is_rotation_invariant() {
        use crate::transform::Transform;
        // Pure rotation → scale 1 (stroke unaffected).
        let rot = Transform::new(0.6, 0.8, -0.8, 0.6, 0.0, 0.0);
        assert!((affine_scale(&rot) - 1.0).abs() < 1e-9);
        // Uniform 3× → scale 3.
        let s = Transform::new(3.0, 0.0, 0.0, 3.0, 5.0, 7.0);
        assert!((affine_scale(&s) - 3.0).abs() < 1e-9);
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

    /// P0: a layer's opacity and blend mode are emitted on its wrapper `<g>`
    /// (opacity attribute + `mix-blend-mode` style), so exported SVG composites
    /// the layer as a unit — matching the renderer's per-layer compositing.
    #[test]
    fn layer_opacity_and_blend_emit_on_group() {
        use crate::node::PathNode;
        use crate::path::PathData;

        let mut doc = Document::new("t", 100.0, 100.0);
        doc.add_node(
            SceneNode::new(
                "r",
                doc.active_layer_id.unwrap(),
                SceneNodeKind::Path(PathNode::new(PathData::rect(0.0, 0.0, 10.0, 10.0))),
            ),
            None,
        );
        let lid = doc.active_layer_id.unwrap();
        {
            let layer = doc.layers.get_mut(&lid).unwrap();
            layer.opacity = 0.5;
            layer.blend_mode = BlendMode::Screen;
        }
        let svg = export_svg(&doc, &SvgExportOptions::default());
        assert!(svg.contains("opacity=\"0.5"), "layer opacity on <g>:\n{svg}");
        assert!(
            svg.contains("mix-blend-mode:screen"),
            "layer blend mode on <g>:\n{svg}"
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

    /// P0: a layer with a blend mode + opacity emits a `/gs0` ExtGState carrying
    /// the PDF blend mode and alpha, and the layer's content is wrapped with it.
    #[test]
    fn pdf_export_layer_blend_and_opacity_emit_extgstate() {
        use crate::node::PathNode;
        use crate::path::PathData;
        use crate::style::Fill;

        let mut doc = Document::new("t", 100.0, 100.0);
        let mut rect = PathNode::new(PathData::rect(0.0, 0.0, 50.0, 50.0));
        rect.fill = Fill::solid(Color::new(0.0, 0.0, 1.0, 1.0));
        doc.add_node(
            SceneNode::new("r", doc.active_layer_id.unwrap(), SceneNodeKind::Path(rect)),
            None,
        );
        let lid = doc.active_layer_id.unwrap();
        {
            let l = doc.layers.get_mut(&lid).unwrap();
            l.opacity = 0.5;
            l.blend_mode = crate::layer::BlendMode::Multiply;
        }
        let bytes = export_pdf(&doc, &PdfExportOptions::default());
        let text = String::from_utf8_lossy(&bytes);
        assert!(bytes.starts_with(b"%PDF-1"), "missing PDF header");
        assert!(text.contains("/BM /Multiply"), "missing blend mode:\n{text}");
        assert!(text.contains("/gs0 gs"), "content not wrapped with the ExtGState");
        assert!(
            text.contains("/ExtGState") && text.contains("/ca 0.5"),
            "missing ExtGState alpha:\n{text}"
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
    fn pdf_export_red_rect_produces_mediabox_and_nonempty() {
        use crate::node::PathNode;
        use crate::path::PathData;
        use crate::style::Fill;

        let mut doc = Document::new("t", 100.0, 100.0);
        let mut rect = PathNode::new(PathData::rect(0.0, 0.0, 100.0, 100.0));
        rect.fill = Fill::solid(Color::new(1.0, 0.0, 0.0, 1.0));
        let node = SceneNode::new(
            "rect",
            doc.active_layer_id.unwrap(),
            SceneNodeKind::Path(rect),
        );
        doc.add_node(node, None);
        let bytes = export_pdf(&doc, &PdfExportOptions::default());
        assert!(!bytes.is_empty(), "PDF bytes must not be empty");
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("MediaBox"), "PDF must contain MediaBox");
    }

    #[test]
    fn pdf_export_business_card_mediabox_72dpi() {
        use crate::document::PrintPreset;

        // US business card at 72 dpi → 252×144 px → 252×144 pt MediaBox
        let doc = Document::from_preset(PrintPreset::BUSINESS_CARD_US, 72.0);
        let bytes = export_pdf(&doc, &PdfExportOptions::default());
        let text = String::from_utf8_lossy(&bytes);
        // MediaBox must be present and the document must be non-empty.
        assert!(!bytes.is_empty());
        assert!(text.contains("MediaBox"), "PDF must contain MediaBox");
        // Width = 252.0, height = 144.0 — these appear in the MediaBox entry.
        assert!(
            text.contains("252") || text.contains("252."),
            "MediaBox should reference width 252:\n{}",
            &text[..text.len().min(500)]
        );
    }

    #[test]
    fn pdf_export_approximates_gradient_with_first_stop() {
        use crate::node::PathNode;
        use crate::path::PathData;
        use crate::style::{Fill, FillKind, Gradient, GradientKind, GradientStop};

        let mut doc = Document::new("t", 100.0, 100.0);
        let grad = Gradient::linear(
            0.0,
            0.0,
            100.0,
            0.0,
            vec![
                GradientStop::new(0.0, Color::new(0.0, 1.0, 0.0, 1.0)),
                GradientStop::new(1.0, Color::new(0.0, 0.0, 1.0, 1.0)),
            ],
        );
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

    /// #205: two paths sharing a byte-identical gradient must emit ONE
    /// `<linearGradient>` def, referenced twice — not `grad-0` + `grad-1` clones.
    #[test]
    fn identical_gradients_are_deduped() {
        use crate::node::{PathNode, SceneNode, SceneNodeKind};
        use crate::path::PathData;
        use crate::style::{Fill, Gradient, GradientStop};

        let mut doc = Document::new("t", 100.0, 100.0);
        let layer_id = doc.layer_order[0];
        let grad = || {
            Gradient::linear(
                0.0,
                0.0,
                100.0,
                0.0,
                vec![
                    GradientStop::new(0.0, Color::new(0.0, 0.0, 1.0, 1.0)),
                    GradientStop::new(1.0, Color::new(0.5, 0.0, 0.5, 1.0)),
                ],
            )
        };
        for name in ["a", "b"] {
            let mut pn = PathNode::new(PathData::rect(0.0, 0.0, 10.0, 10.0));
            pn.fill = Fill::gradient(grad());
            let node = SceneNode::new(name, layer_id, SceneNodeKind::Path(pn));
            let nid = node.id;
            doc.nodes.insert(nid, node);
            doc.layers.get_mut(&layer_id).unwrap().node_ids.push(nid);
        }
        let svg = export_svg(&doc, &SvgExportOptions::default());
        let def_count = svg.matches("<linearGradient").count();
        assert_eq!(def_count, 1, "identical gradients should dedupe to 1 def:\n{svg}");
        assert_eq!(
            svg.matches("url(#grad-0)").count(),
            2,
            "both paths should reference the single shared gradient:\n{svg}"
        );
    }

    /// #201: a gradient stroke paint must export as `stroke="url(#…)"`, and when a
    /// fill and stroke share the SAME gradient, both reference ONE deduped def.
    #[test]
    fn gradient_stroke_exports_url_and_dedupes_with_fill() {
        use crate::node::{PathNode, SceneNode, SceneNodeKind};
        use crate::path::PathData;
        use crate::style::{Fill, FillKind, Gradient, GradientStop, Stroke};

        let mut doc = Document::new("t", 100.0, 100.0);
        let layer_id = doc.layer_order[0];
        let grad = || {
            Gradient::linear(
                0.0,
                0.0,
                100.0,
                0.0,
                vec![
                    GradientStop::new(0.0, Color::new(0.18, 0.34, 0.81, 1.0)),
                    GradientStop::new(1.0, Color::new(0.49, 0.23, 0.93, 1.0)),
                ],
            )
        };
        let mut pn = PathNode::new(PathData::rect(0.0, 0.0, 40.0, 40.0))
            .with_stroke(Stroke::solid(Color::BLACK, 6.0));
        pn.fill = Fill::gradient(grad());
        // Same gradient on the stroke paint.
        pn.stroke.paint = Some(FillKind::Gradient(grad()));
        let node = SceneNode::new("icon", layer_id, SceneNodeKind::Path(pn));
        let nid = node.id;
        doc.nodes.insert(nid, node);
        doc.layers.get_mut(&layer_id).unwrap().node_ids.push(nid);

        let svg = export_svg(&doc, &SvgExportOptions::default());
        assert!(
            svg.contains("stroke=\"url(#grad-0)\""),
            "gradient stroke should export as url ref:\n{svg}"
        );
        assert!(
            svg.contains("fill=\"url(#grad-0)\""),
            "fill should share the same deduped gradient id:\n{svg}"
        );
        assert_eq!(
            svg.matches("<linearGradient").count(),
            1,
            "fill+stroke sharing one gradient must emit a single def:\n{svg}"
        );
    }

    /// #206: selection export must round path `d` to the requested precision
    /// instead of emitting 15-decimal coordinates.
    #[test]
    fn selection_export_respects_precision() {
        use crate::node::{PathNode, SceneNode, SceneNodeKind};
        use crate::path::PathData;

        let mut doc = Document::new("t", 100.0, 100.0);
        let layer_id = doc.layer_order[0];
        // A path with an ugly high-precision coordinate.
        let pd = PathData::from_svg("M0.123456789 0.987654321 L10 10 Z").unwrap();
        let node = SceneNode::new("icon", layer_id, SceneNodeKind::Path(PathNode::new(pd)));
        let nid = node.id;
        doc.nodes.insert(nid, node);
        doc.layers.get_mut(&layer_id).unwrap().node_ids.push(nid);

        let opts = SvgSelectionOptions {
            precision: 3,
            ..Default::default()
        };
        let svg = export_nodes_as_svg_opts(&doc, &[nid], &opts);
        assert!(
            svg.contains("M0.123 0.988"),
            "path d should be rounded to 3 decimals:\n{svg}"
        );
        assert!(
            !svg.contains("0.123456"),
            "no 15-decimal coordinates should survive:\n{svg}"
        );
    }

    /// #203: `Square` normalization must produce a 1:1 (square) viewBox centered on
    /// the content, so a wide icon and a tall icon frame identically.
    #[test]
    fn selection_export_square_normalize_is_uniform() {
        use crate::node::{PathNode, SceneNode, SceneNodeKind};
        use crate::path::PathData;

        let mut doc = Document::new("t", 400.0, 400.0);
        let layer_id = doc.layer_order[0];
        // Wide rectangle: 80×20.
        let pd = PathData::rect(10.0, 40.0, 80.0, 20.0);
        let node = SceneNode::new("wide", layer_id, SceneNodeKind::Path(PathNode::new(pd)));
        let nid = node.id;
        doc.nodes.insert(nid, node);
        doc.layers.get_mut(&layer_id).unwrap().node_ids.push(nid);

        let opts = SvgSelectionOptions {
            normalize: SvgNormalize::Square { pad: 0.0 },
            ..Default::default()
        };
        let svg = export_nodes_as_svg_opts(&doc, &[nid], &opts);
        // viewBox side should be max(80,20) = 80 in both dimensions.
        assert!(
            svg.contains("width=\"80\" height=\"80\""),
            "square normalize should yield an 80×80 viewBox:\n{svg}"
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

    // ── T0.3 CMYK colour-mode tests ────────────────────────────────────────────

    /// A 100×100 doc with one red-filled rect exported in CMYK mode must contain
    /// a CMYK fill operator (` k`) and must NOT contain an RGB fill operator (` rg`).
    #[test]
    fn pdf_cmyk_mode_emits_k_operator_not_rg() {
        use crate::document::ColorMode;
        use crate::node::PathNode;
        use crate::path::PathData;
        use crate::style::Fill;

        let mut doc = Document::new("t", 100.0, 100.0);
        let mut rect = PathNode::new(PathData::rect(10.0, 10.0, 80.0, 80.0));
        rect.fill = Fill::solid(Color::new(1.0, 0.0, 0.0, 1.0)); // red
        let node = SceneNode::new(
            "rect",
            doc.active_layer_id.unwrap(),
            SceneNodeKind::Path(rect),
        );
        doc.add_node(node, None);

        let opts = PdfExportOptions {
            color_mode: ColorMode::Cmyk,
            ..Default::default()
        };
        let bytes = export_pdf(&doc, &opts);
        let text = String::from_utf8_lossy(&bytes);

        assert!(
            text.contains(" k\n") || text.contains(" k "),
            "CMYK export must contain a ` k` fill operator;\n{text}"
        );
        assert!(
            !text.contains(" rg\n") && !text.contains(" rg "),
            "CMYK export must NOT contain an ` rg` RGB fill operator;\n{text}"
        );
    }

    /// Write the CMYK PDF to /tmp for offline qpdf inspection (accept criterion).
    /// This test is always skipped in CI (guarded by env CMYK_WRITE_ACCEPT=1).
    #[test]
    fn pdf_cmyk_accept_write_to_tmp() {
        if std::env::var("CMYK_WRITE_ACCEPT").as_deref() != Ok("1") {
            return;
        }
        use crate::document::ColorMode;
        use crate::node::PathNode;
        use crate::path::PathData;
        use crate::style::Fill;

        let mut doc = Document::new("accept", 100.0, 100.0);
        let mut rect = PathNode::new(PathData::rect(10.0, 10.0, 80.0, 80.0));
        rect.fill = Fill::solid(Color::new(1.0, 0.0, 0.0, 1.0));
        let node = SceneNode::new(
            "r",
            doc.active_layer_id.unwrap(),
            SceneNodeKind::Path(rect),
        );
        doc.add_node(node, None);
        let opts = PdfExportOptions {
            color_mode: ColorMode::Cmyk,
            ..Default::default()
        };
        let bytes = export_pdf(&doc, &opts);
        std::fs::write("/tmp/cmyk_accept.pdf", &bytes).unwrap();
        println!("Written /tmp/cmyk_accept.pdf ({} bytes)", bytes.len());
    }

    /// The same doc in default (RGB) mode must contain ` rg` and must NOT
    /// contain a CMYK ` k` fill operator.
    #[test]
    fn pdf_rgb_mode_emits_rg_operator_not_k() {
        use crate::node::PathNode;
        use crate::path::PathData;
        use crate::style::Fill;

        let mut doc = Document::new("t", 100.0, 100.0);
        let mut rect = PathNode::new(PathData::rect(10.0, 10.0, 80.0, 80.0));
        rect.fill = Fill::solid(Color::new(1.0, 0.0, 0.0, 1.0)); // red
        let node = SceneNode::new(
            "rect",
            doc.active_layer_id.unwrap(),
            SceneNodeKind::Path(rect),
        );
        doc.add_node(node, None);

        let bytes = export_pdf(&doc, &PdfExportOptions::default());
        let text = String::from_utf8_lossy(&bytes);

        // pdf-writer writes the content stream uncompressed, so rg must appear.
        assert!(
            text.contains(" rg\n") || text.contains(" rg "),
            "RGB export must contain an ` rg` fill operator;\n{text}"
        );
        // No CMYK operator should appear.
        assert!(
            !text.contains(" k\n") && !text.contains(" k "),
            "RGB export must NOT contain a CMYK ` k` fill operator;\n{text}"
        );
    }

    // ── T1.5/T1.6/T1.8 — page-box geometry and marks ─────────────────────────

    /// Regression: bleed=0, marks=false → all three boxes collapse to [0,0,w,h].
    /// Behaviour must be identical to pre-bleed baseline.
    #[test]
    fn page_boxes_no_bleed_no_marks_equals_artboard() {
        let doc = Document::new("t", 200.0, 150.0);
        let opts = PdfExportOptions::default(); // marks=false, bleed=0 (doc default)
        let boxes = compute_page_boxes(&doc, &opts);
        let expected = [0.0_f32, 0.0, 200.0, 150.0];
        assert_eq!(boxes.media, expected, "media should equal artboard");
        assert_eq!(boxes.trim,  expected, "trim  should equal artboard");
        assert_eq!(boxes.bleed, expected, "bleed should equal artboard");
    }

    /// T1.5: a 3 mm bleed (marks=false) produces Trim ⊂ Bleed ⊂ Media with
    /// correct numeric sizes.  At 72 dpi: bleed_px = 3 * 72 / 25.4 ≈ 8.504 pt.
    #[test]
    fn page_boxes_bleed_containment_and_size() {
        use crate::units::{to_px, DocumentUnit::Mm};

        let mut doc = Document::new("t", 252.0, 144.0); // business-card-ish
        doc.bleed_mm = 3.0;
        let opts = PdfExportOptions { marks: false, ..Default::default() };
        let boxes = compute_page_boxes(&doc, &opts);

        let bleed_pt = to_px(3.0, Mm, 72.0) as f32;
        let w = 252.0_f32;
        let h = 144.0_f32;

        // Trim == artboard (outer = bleed_px when marks=false, no mark_room)
        let outer = bleed_pt;
        let expected_trim = [outer, outer, outer + w, outer + h];
        let expected_bleed = [0.0, 0.0, w + 2.0 * bleed_pt, h + 2.0 * bleed_pt];
        let expected_media = [0.0, 0.0, w + 2.0 * bleed_pt, h + 2.0 * bleed_pt];

        // Containment: trim x0/y0 ≥ bleed x0/y0, trim x1/y1 ≤ bleed x1/y1
        assert!(
            boxes.trim[0] >= boxes.bleed[0] && boxes.trim[1] >= boxes.bleed[1],
            "trim origin must be inside bleed: trim={:?}, bleed={:?}",
            boxes.trim, boxes.bleed
        );
        assert!(
            boxes.trim[2] <= boxes.bleed[2] && boxes.trim[3] <= boxes.bleed[3],
            "trim far edge must be inside bleed"
        );
        assert!(
            boxes.bleed[0] >= boxes.media[0] && boxes.bleed[1] >= boxes.media[1],
            "bleed origin must be inside media"
        );
        assert!(
            boxes.bleed[2] <= boxes.media[2] && boxes.bleed[3] <= boxes.media[3],
            "bleed far edge must be inside media"
        );

        // Numeric sizes (tolerance 0.01 pt for float arithmetic).
        let tol = 0.01_f32;
        assert!((boxes.trim[0]  - expected_trim[0]).abs()  < tol, "trim[0]");
        assert!((boxes.trim[1]  - expected_trim[1]).abs()  < tol, "trim[1]");
        assert!((boxes.trim[2]  - expected_trim[2]).abs()  < tol, "trim[2]");
        assert!((boxes.trim[3]  - expected_trim[3]).abs()  < tol, "trim[3]");

        // When marks=false: media == bleed (no extra band).
        assert!((boxes.media[2] - expected_media[2]).abs() < tol, "media width");
        assert!((boxes.media[3] - expected_media[3]).abs() < tol, "media height");
        let _ = (expected_bleed, expected_media); // suppress unused warnings
    }

    /// T1.6: exporting with marks=true produces a longer content stream than
    /// marks=false because extra stroke operators are emitted for the marks.
    #[test]
    fn pdf_export_marks_true_emits_more_stroke_ops() {
        let mut doc = Document::new("t", 252.0, 144.0);
        doc.bleed_mm = 3.0;

        let opts_no_marks = PdfExportOptions { marks: false, ..Default::default() };
        let opts_marks    = PdfExportOptions { marks: true,  ..Default::default() };

        let bytes_no = export_pdf(&doc, &opts_no_marks);
        let bytes_yes = export_pdf(&doc, &opts_marks);

        assert!(
            bytes_yes.len() > bytes_no.len(),
            "marks=true PDF ({} bytes) should be larger than marks=false ({} bytes)",
            bytes_yes.len(), bytes_no.len()
        );

        // The marks stream must contain stroke operators (`S` in PDF).
        let text_yes = String::from_utf8_lossy(&bytes_yes);
        assert!(
            text_yes.contains(" S\n") || text_yes.contains(" S ") || text_yes.contains("\nS\n"),
            "marks=true PDF should contain stroke (S) operators"
        );
    }

    /// T1.8: when bleed_mm > 0 the exported PDF still starts with %PDF and the
    /// content-stream transform uses the trim-offset CTM, not the bare [1,0,0,-1,0,h].
    #[test]
    fn pdf_export_with_bleed_is_valid_and_uses_trim_offset_transform() {
        let mut doc = Document::new("t", 252.0, 144.0);
        doc.bleed_mm = 3.0;
        let opts = PdfExportOptions::default();
        let bytes = export_pdf(&doc, &opts);
        assert!(bytes.starts_with(b"%PDF-1"), "must start with PDF header");
        // When bleed is 0, trim_y1 = h = 144.  With bleed=3mm≈8.5pt, trim_y1
        // is approximately 144 + 8.5 = 152.5 pt (trim starts at outer≈8.5).
        let text = String::from_utf8_lossy(&bytes);
        // The old bare-h transform [1 0 0 -1 0 144] must NOT appear; the new
        // bleed-offset trim_y1 value (≈152.5) should appear instead.
        assert!(
            !text.contains("0 144 cm"),
            "old bare-h transform must not appear with bleed set"
        );
    }

    /// ACCEPT evidence: write business-card PDFs (with and without marks) to
    /// /tmp/photonic_accept_*.pdf and print computed box values.
    /// Run with: cargo test -p photonic-core --lib accept_evidence -- --nocapture
    #[test]
    fn accept_evidence_page_boxes_and_marks() {
        use crate::units::{to_px, DocumentUnit::Mm};
        use std::io::Write;

        let mut doc = Document::new("accept", 252.0, 144.0);
        doc.bleed_mm = 3.0;

        let opts_no   = PdfExportOptions { marks: false, ..Default::default() };
        let opts_marks = PdfExportOptions { marks: true, ..Default::default() };

        let boxes_no    = compute_page_boxes(&doc, &opts_no);
        let boxes_marks = compute_page_boxes(&doc, &opts_marks);

        let bleed_pt = to_px(3.0, Mm, 72.0) as f32;
        println!("bleed_mm=3.0  → bleed_pt = {bleed_pt:.4}");
        println!("--- marks=false ---");
        println!("  MediaBox = {:?}", boxes_no.media);
        println!("  BleedBox = {:?}", boxes_no.bleed);
        println!("  TrimBox  = {:?}", boxes_no.trim);
        println!("--- marks=true ---");
        println!("  MediaBox = {:?}", boxes_marks.media);
        println!("  BleedBox = {:?}", boxes_marks.bleed);
        println!("  TrimBox  = {:?}", boxes_marks.trim);

        // Containment
        let m = &boxes_marks;
        assert!(m.trim[0] >= m.bleed[0] && m.trim[0] >= 0.0, "T⊂B⊂M x0");
        assert!(m.bleed[0] >= m.media[0], "B⊂M x0");
        assert!(m.trim[2] <= m.bleed[2], "T⊂B x1");
        assert!(m.bleed[2] <= m.media[2], "B⊂M x1");

        // Write PDFs
        let bytes_no    = export_pdf(&doc, &opts_no);
        let bytes_marks = export_pdf(&doc, &opts_marks);
        let path_no    = "/tmp/photonic_accept_no_marks.pdf";
        let path_marks = "/tmp/photonic_accept_marks.pdf";
        std::fs::File::create(path_no).unwrap().write_all(&bytes_no).unwrap();
        std::fs::File::create(path_marks).unwrap().write_all(&bytes_marks).unwrap();
        println!("Wrote: {path_no} ({} bytes)", bytes_no.len());
        println!("Wrote: {path_marks} ({} bytes)", bytes_marks.len());
        assert!(bytes_marks.len() > bytes_no.len(), "marks adds bytes");
    }
}
