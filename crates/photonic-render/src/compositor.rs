//! Pure-CPU document compositor — renders **all** node types in **true draw
//! order** onto a pre-filled RGBA8 buffer.
//!
//! The GPU/headless path ([`crate::headless`]) composites in two separate
//! planes: it first rasterises every vector (`Path`) node on the GPU, then
//! composites every raster node on top in a second pass
//! ([`composite_raster_nodes`](crate::headless)). That ordering is wrong when a
//! vector node sits *between* two raster layers in the layer stack — the vector
//! always lands above all rasters regardless of its real z-position.
//!
//! [`composite_document`] fixes that by walking `doc.nodes_in_draw_order()`
//! **once**, in order, and drawing each node — path or raster — straight onto
//! the shared buffer. A blue rectangle placed between a red raster (below) and a
//! green raster (above) therefore composites in the correct order.
//!
//! The vector rasteriser is a small, deterministic, panic-free CPU triangle
//! filler with 4× supersampled edge coverage for anti-aliasing. Raster nodes
//! (including non-destructive adjustment layers) replicate the exact behaviour
//! of [`crate::headless`]'s `composite_raster_nodes`, sharing the same camera so
//! vector and raster content register pixel-for-pixel.

use crate::{
    canvas::CanvasView,
    tessellator::{tessellate_fill, tessellate_stroke, Mesh},
};
use photonic_core::{
    node::{NodeId, SceneNodeKind},
    raster::blend::blend_rgb,
    style::FillKind,
    transform::Transform,
    BlendMode, Document,
};

/// 4× supersample offsets (a rotated-ish 2×2 grid) used for edge coverage.
const SUBSAMPLES: [(f64, f64); 4] = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)];
const SUB_WEIGHT: f32 = 0.25;

/// Composite every visible node of `doc` onto the pre-filled `base` buffer
/// (RGBA8, straight alpha, length `w*h*4`) in `doc.nodes_in_draw_order()` order.
///
/// The caller is responsible for pre-filling `base` with the background (and
/// artboard) — this function only draws nodes. Because every node — vector or
/// raster — is drawn in a single ordered pass, z-order is correct across mixed
/// vector + raster documents (the bug this module exists to fix).
///
/// Deterministic and panic-free: degenerate, non-finite, or out-of-bounds
/// geometry is clipped or skipped, never panicked on.
pub fn composite_document(base: &mut [u8], w: u32, h: u32, doc: &Document, view: &CanvasView) {
    if w == 0 || h == 0 {
        return;
    }
    let needed = (w as usize) * (h as usize) * 4;
    if base.len() < needed {
        return;
    }

    let eff = group_opacity_map(doc);
    // Reusable per-node coverage accumulator (cleared as it is consumed).
    let mut cov = vec![0.0f32; (w as usize) * (h as usize)];

    for node in doc.nodes_in_draw_order() {
        let nid = node.id;
        // Resolve symbol instances to the live master (+ overrides), exactly as
        // the headless path does, so output matches the GPU renderer.
        let resolved = doc.resolve_render_node(node);
        let node = resolved.as_ref();

        let gop = eff.get(&nid).copied().unwrap_or(1.0);
        if gop <= 0.0 {
            continue;
        }

        match &node.kind {
            SceneNodeKind::Path(pn) => {
                render_path_node(
                    base,
                    w,
                    h,
                    view,
                    &mut cov,
                    &node.transform,
                    node.opacity,
                    gop,
                    node.blend_mode,
                    pn,
                );
            }
            SceneNodeKind::Raster(_) => {
                render_raster_node(base, w, h, doc, view, node, gop);
            }
            // Groups are flattened away by `nodes_in_draw_order`; text is not
            // rendered by the headless path either, so it is skipped here too.
            SceneNodeKind::Group(_) | SceneNodeKind::Text(_) => {}
        }
    }
}

// ─── Group opacity propagation ────────────────────────────────────────────────

/// Map each node id to the product of its ancestor groups' opacities (and 0 if
/// any ancestor group is hidden).
///
/// Copied from [`crate::headless`]'s `group_opacity_map`: `nodes_in_draw_order`
/// flattens groups to leaves and drops the group context, so we recover the
/// ancestor opacity/visibility chain here and fold it into the rendered alpha.
fn group_opacity_map(doc: &Document) -> std::collections::HashMap<NodeId, f32> {
    use std::collections::HashMap;
    let mut parent: HashMap<NodeId, NodeId> = HashMap::new();
    for n in doc.nodes.values() {
        if let SceneNodeKind::Group(g) = &n.kind {
            for c in &g.children {
                parent.insert(*c, n.id);
            }
        }
    }
    let mut out = HashMap::new();
    for id in doc.nodes.keys() {
        let mut op = 1.0f32;
        let mut cur = *id;
        let mut guard = 0;
        while let Some(p) = parent.get(&cur) {
            if let Some(pn) = doc.nodes.get(p) {
                if !pn.visible {
                    op = 0.0;
                }
                op *= pn.opacity;
            }
            cur = *p;
            guard += 1;
            if guard > 64 {
                break;
            }
        }
        out.insert(*id, op);
    }
    out
}

// ─── Vector (path) node rendering ─────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn render_path_node(
    base: &mut [u8],
    w: u32,
    h: u32,
    view: &CanvasView,
    cov: &mut [f32],
    transform: &Transform,
    node_opacity: f32,
    gop: f32,
    blend_mode: BlendMode,
    pn: &photonic_core::node::PathNode,
) {
    // ── Fill ──────────────────────────────────────────────────────────────────
    if pn.fill.enabled && !matches!(pn.fill.kind, FillKind::None) {
        let opacity = pn.fill.opacity * node_opacity * gop;
        if opacity > 0.0 {
            let mesh = tessellate_fill(&pn.path_data, false);
            if let Some(bbox) = rasterize_mesh(cov, w, h, &mesh, transform, view) {
                let kind = &pn.fill.kind;
                composite_coverage(base, w, view, cov, bbox, blend_mode, |cx, cy| {
                    let c = kind.sample_at(cx, cy, opacity);
                    ([c[0], c[1], c[2]], c[3])
                });
            }
        }
    }

    // ── Stroke ─────────────────────────────────────────────────────────────────
    if pn.stroke.enabled && pn.stroke.width > 0.0 {
        let sc = &pn.stroke;
        let alpha = sc.color.a * sc.opacity * node_opacity * gop;
        if alpha > 0.0 {
            let mesh = tessellate_stroke(
                &pn.path_data,
                sc.width as f32,
                sc.line_cap,
                sc.line_join,
                sc.miter_limit as f32,
            );
            if let Some(bbox) = rasterize_mesh(cov, w, h, &mesh, transform, view) {
                let rgb = [sc.color.r, sc.color.g, sc.color.b];
                composite_coverage(base, w, view, cov, bbox, blend_mode, |_, _| (rgb, alpha));
            }
        }
    }
}

/// Screen-space pixel bounding box (`x0`, `y0`, `x1`, `y1`); `x1`/`y1` exclusive.
type Bbox = (u32, u32, u32, u32);

/// Rasterise `mesh` (in path-local coords) into the `cov` accumulator using
/// `transform` (local → canvas) followed by `view` (canvas → screen).
///
/// Each pixel accumulates 4× supersampled coverage in `[0, 1+]` (interior pixels
/// may exceed 1 where triangles share an edge; callers clamp). Returns the
/// touched pixel bbox, or `None` if nothing was drawn. Never panics: non-finite
/// or degenerate triangles are skipped and all writes are clipped to `w`×`h`.
fn rasterize_mesh(
    cov: &mut [f32],
    w: u32,
    h: u32,
    mesh: &Mesh,
    transform: &Transform,
    view: &CanvasView,
) -> Option<Bbox> {
    if mesh.is_empty() || mesh.indices.len() < 3 {
        return None;
    }
    let nverts = mesh.vertices.len();

    // Pre-project every vertex to screen space once.
    let mut screen: Vec<[f64; 2]> = Vec::with_capacity(nverts);
    for v in &mesh.vertices {
        let (cx, cy) = transform.apply(v[0] as f64, v[1] as f64);
        let (sx, sy) = view.canvas_to_screen(cx, cy);
        screen.push([sx, sy]);
    }

    let mut dirty: Option<Bbox> = None;

    for tri in mesh.indices.chunks_exact(3) {
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        if i0 >= nverts || i1 >= nverts || i2 >= nverts {
            continue;
        }
        let a = screen[i0];
        let b = screen[i1];
        let c = screen[i2];
        if !finite2(a) || !finite2(b) || !finite2(c) {
            continue;
        }

        // Signed area ×2; skip degenerate slivers.
        let area2 = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        if !area2.is_finite() || area2.abs() < 1e-9 {
            continue;
        }
        let sign = if area2 > 0.0 { 1.0 } else { -1.0 };

        // Pixel bbox clipped to the buffer.
        let min_x = a[0].min(b[0]).min(c[0]).floor();
        let max_x = a[0].max(b[0]).max(c[0]).ceil();
        let min_y = a[1].min(b[1]).min(c[1]).floor();
        let max_y = a[1].max(b[1]).max(c[1]).ceil();
        let x0 = (min_x.max(0.0) as i64).clamp(0, w as i64) as u32;
        let x1 = (max_x.max(0.0) as i64).clamp(0, w as i64) as u32;
        let y0 = (min_y.max(0.0) as i64).clamp(0, h as i64) as u32;
        let y1 = (max_y.max(0.0) as i64).clamp(0, h as i64) as u32;
        if x1 <= x0 || y1 <= y0 {
            continue;
        }

        for py in y0..y1 {
            for px in x0..x1 {
                let mut acc = 0.0f32;
                for (sx, sy) in SUBSAMPLES {
                    let pxx = px as f64 + sx;
                    let pyy = py as f64 + sy;
                    if point_in_tri(a, b, c, pxx, pyy, sign) {
                        acc += SUB_WEIGHT;
                    }
                }
                if acc > 0.0 {
                    let idx = (py as usize) * (w as usize) + (px as usize);
                    cov[idx] += acc;
                }
            }
        }

        dirty = Some(match dirty {
            None => (x0, y0, x1, y1),
            Some((dx0, dy0, dx1, dy1)) => (dx0.min(x0), dy0.min(y0), dx1.max(x1), dy1.max(y1)),
        });
    }

    dirty
}

#[inline]
fn finite2(p: [f64; 2]) -> bool {
    p[0].is_finite() && p[1].is_finite()
}

/// Edge-function point-in-triangle test consistent with `sign` (the triangle's
/// winding). Points exactly on an edge count as inside.
#[inline]
fn point_in_tri(a: [f64; 2], b: [f64; 2], c: [f64; 2], px: f64, py: f64, sign: f64) -> bool {
    let e0 = (b[0] - a[0]) * (py - a[1]) - (b[1] - a[1]) * (px - a[0]);
    let e1 = (c[0] - b[0]) * (py - b[1]) - (c[1] - b[1]) * (px - b[0]);
    let e2 = (a[0] - c[0]) * (py - c[1]) - (a[1] - c[1]) * (px - c[0]);
    e0 * sign >= 0.0 && e1 * sign >= 0.0 && e2 * sign >= 0.0
}

/// Composite an accumulated coverage mask onto `base` using a per-pixel colour
/// source, then **clear** the touched region of `cov` so it is reusable.
///
/// `src(cx, cy)` returns the straight-alpha source `(rgb, alpha)` at canvas
/// coordinates `(cx, cy)`; the final source alpha is `alpha * clamped_coverage`.
/// Compositing is source-over with the node's blend mode — matching the exact
/// math used by the headless raster compositor.
fn composite_coverage(
    base: &mut [u8],
    w: u32,
    view: &CanvasView,
    cov: &mut [f32],
    bbox: Bbox,
    mode: BlendMode,
    mut src: impl FnMut(f64, f64) -> ([f32; 3], f32),
) {
    let (x0, y0, x1, y1) = bbox;
    for py in y0..y1 {
        for px in x0..x1 {
            let ci = (py as usize) * (w as usize) + (px as usize);
            let coverage = cov[ci];
            cov[ci] = 0.0; // reset for the next node's reuse
            if coverage <= 0.0 {
                continue;
            }
            let coverage = coverage.min(1.0);

            let (cx, cy) = view.screen_to_canvas(px as f64 + 0.5, py as f64 + 0.5);
            let (rgb, a) = src(cx, cy);
            let sa = (a * coverage).clamp(0.0, 1.0);
            if sa <= 0.0 {
                continue;
            }

            let idx = ci * 4;
            composite_pixel(base, idx, rgb, sa, mode);
        }
    }
}

/// Source-over composite of a single straight-alpha source pixel onto `base`,
/// honouring `mode`. Mirrors the per-pixel math in headless `composite_raster_nodes`.
#[inline]
fn composite_pixel(base: &mut [u8], idx: usize, cs: [f32; 3], sa: f32, mode: BlendMode) {
    let b = [
        base[idx] as f32 / 255.0,
        base[idx + 1] as f32 / 255.0,
        base[idx + 2] as f32 / 255.0,
    ];
    let ba = base[idx + 3] as f32 / 255.0;

    let blended = blend_rgb(mode, b, cs);
    let mixed = [
        (1.0 - ba) * cs[0] + ba * blended[0],
        (1.0 - ba) * cs[1] + ba * blended[1],
        (1.0 - ba) * cs[2] + ba * blended[2],
    ];
    let oa = sa + ba * (1.0 - sa);
    if oa > 0.0 {
        for c in 0..3 {
            let co = (mixed[c] * sa + b[c] * ba * (1.0 - sa)) / oa;
            base[idx + c] = (co * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    base[idx + 3] = (oa * 255.0).round().clamp(0.0, 255.0) as u8;
}

// ─── Raster node rendering ────────────────────────────────────────────────────

/// Composite a single raster node (image or adjustment layer) onto `base`.
///
/// Replicates the per-node logic of headless `composite_raster_nodes` exactly,
/// including the inverse-transform + view mapping, bilinear sampling, layer
/// mask, blend mode, and (for adjustment layers) applying the adjustment to a
/// copy of the composite-beneath and blending it back by opacity × mask.
fn render_raster_node(
    base: &mut [u8],
    w: u32,
    h: u32,
    doc: &Document,
    view: &CanvasView,
    node: &photonic_core::node::SceneNode,
    gop: f32,
) {
    let SceneNodeKind::Raster(rn) = &node.kind else {
        return;
    };
    let node_opacity = (node.opacity * gop).clamp(0.0, 1.0);
    if node_opacity <= 0.0 {
        return;
    }

    // ── Non-destructive adjustment layer ──────────────────────────────────────
    if let Some(spec) = &rn.adjustment {
        let Ok(mut buf) = photonic_core::raster::image::RasterImage::from_rgba(
            w,
            h,
            base[..(w as usize * h as usize * 4)].to_vec(),
        ) else {
            return;
        };
        spec.apply(&mut buf, None);
        let mask = rn.mask.as_ref();
        for py in 0..h {
            for px in 0..w {
                let mut amt = node_opacity;
                if let Some(m) = mask {
                    let (cx, cy) = view.screen_to_canvas(px as f64 + 0.5, py as f64 + 0.5);
                    if doc.width > 0.0 && doc.height > 0.0 {
                        let mx = cx / doc.width * m.width as f64;
                        let my = cy / doc.height * m.height as f64;
                        if mx < 0.0 || my < 0.0 || mx >= m.width as f64 || my >= m.height as f64 {
                            amt = 0.0;
                        } else {
                            amt *= m.coverage(mx as u32, my as u32);
                        }
                    }
                }
                if amt <= 0.0 {
                    continue;
                }
                let i = ((py * w + px) * 4) as usize;
                for c in 0..4 {
                    let orig = base[i + c] as f32;
                    let adj = buf.pixels[i + c] as f32;
                    base[i + c] = (orig + (adj - orig) * amt).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        return;
    }

    // ── Pixel image ────────────────────────────────────────────────────────────
    let img = &rn.image;
    if img.width == 0 || img.height == 0 {
        return;
    }
    let affine = node.transform.to_kurbo();
    let inv = affine.inverse();

    // Screen-space AABB of the transformed image rect to bound iteration.
    let corners = [
        (0.0, 0.0),
        (img.width as f64, 0.0),
        (img.width as f64, img.height as f64),
        (0.0, img.height as f64),
    ];
    let (mut min_x, mut min_y) = (f64::MAX, f64::MAX);
    let (mut max_x, mut max_y) = (f64::MIN, f64::MIN);
    for (lx, ly) in corners {
        let (dx, dy) = node.transform.apply(lx, ly);
        let (sx, sy) = view.canvas_to_screen(dx, dy);
        if !sx.is_finite() || !sy.is_finite() {
            return;
        }
        min_x = min_x.min(sx);
        min_y = min_y.min(sy);
        max_x = max_x.max(sx);
        max_y = max_y.max(sy);
    }
    let x0 = (min_x.floor() as i64).max(0);
    let y0 = (min_y.floor() as i64).max(0);
    let x1 = (max_x.ceil() as i64).min(w as i64);
    let y1 = (max_y.ceil() as i64).min(h as i64);

    for py in y0..y1 {
        for px in x0..x1 {
            let (dx, dy) = view.screen_to_canvas(px as f64 + 0.5, py as f64 + 0.5);
            let lp = inv * kurbo::Point::new(dx, dy);
            if lp.x < 0.0 || lp.y < 0.0 || lp.x >= img.width as f64 || lp.y >= img.height as f64 {
                continue;
            }
            let s = img.sample_bilinear(lp.x as f32 - 0.5, lp.y as f32 - 0.5);
            let mut sa = (s[3] as f32 / 255.0) * node_opacity;
            if let Some(mask) = &rn.mask {
                sa *= mask.coverage(lp.x as u32, lp.y as u32);
            }
            if sa <= 0.0 {
                continue;
            }
            let idx = ((py as u32 * w + px as u32) * 4) as usize;
            let cs = [
                s[0] as f32 / 255.0,
                s[1] as f32 / 255.0,
                s[2] as f32 / 255.0,
            ];
            composite_pixel(base, idx, cs, sa, node.blend_mode);
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use photonic_core::{
        color::Color,
        node::{PathNode, RasterNode, SceneNode, SceneNodeKind},
        path::PathData,
        raster::image::RasterImage,
        style::Fill,
        transform::Transform,
        Document,
    };

    fn solid_raster(w: u32, h: u32, rgba: [u8; 4]) -> RasterImage {
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            px.extend_from_slice(&rgba);
        }
        RasterImage::from_rgba(w, h, px).expect("valid raster")
    }

    /// The core bug fix: a vector node placed *between* two raster layers must
    /// composite in true z-order, not above all rasters.
    #[test]
    fn z_interleave_path_between_rasters() {
        const W: f64 = 60.0;
        const H: f64 = 60.0;
        let mut doc = Document::new("t", W, H);

        // (bottom) full-canvas RED raster.
        let red = solid_raster(W as u32, H as u32, [255, 0, 0, 255]);
        doc.add_node(
            SceneNode::new(
                "red",
                Default::default(),
                SceneNodeKind::Raster(RasterNode::new(red)),
            ),
            None,
        );

        // (middle) BLUE filled rectangle path covering the centre (canvas 20..40).
        let mut pn = PathNode::new(PathData::rect(20.0, 20.0, 20.0, 20.0));
        pn.fill = Fill::solid(Color::BLUE);
        doc.add_node(
            SceneNode::new("blue", Default::default(), SceneNodeKind::Path(pn)),
            None,
        );

        // (top) small GREEN raster over the top-left corner (canvas 0..10).
        let green = solid_raster(10, 10, [0, 255, 0, 255]);
        doc.add_node(
            SceneNode::new(
                "green",
                Default::default(),
                SceneNodeKind::Raster(RasterNode::new(green)),
            )
            .with_transform(Transform::translate(0.0, 0.0)),
            None,
        );

        let w = W as u32;
        let h = H as u32;
        let mut view = CanvasView::new(w, h);
        view.fit_to_rect(0.0, 0.0, doc.width, doc.height);

        // Pre-fill with opaque grey (caller's background).
        let mut base = vec![0u8; (w * h * 4) as usize];
        for px in base.chunks_exact_mut(4) {
            px.copy_from_slice(&[40, 40, 40, 255]);
        }

        composite_document(&mut base, w, h, &doc, &view);

        let at = |cx: f64, cy: f64| -> [u8; 4] {
            let (sx, sy) = view.canvas_to_screen(cx, cy);
            let px = (sx.round() as i64).clamp(0, w as i64 - 1) as u32;
            let py = (sy.round() as i64).clamp(0, h as i64 - 1) as u32;
            let i = ((py * w + px) * 4) as usize;
            [base[i], base[i + 1], base[i + 2], base[i + 3]]
        };

        // Centre (canvas 30,30): BLUE — path is above the red raster.
        let center = at(30.0, 30.0);
        assert!(
            center[2] > 200 && center[0] < 60 && center[1] < 60,
            "centre should be blue, got {center:?}"
        );

        // Top-left corner (canvas 4,4): GREEN — top raster is above the path.
        let corner = at(4.0, 4.0);
        assert!(
            corner[1] > 200 && corner[0] < 60 && corner[2] < 60,
            "corner should be green, got {corner:?}"
        );

        // Bottom-right (canvas 50,50): RED — covered only by the base raster.
        let red_only = at(50.0, 50.0);
        assert!(
            red_only[0] > 200 && red_only[1] < 60 && red_only[2] < 60,
            "red-only area should be red, got {red_only:?}"
        );
    }

    /// Degenerate / extreme inputs must never panic.
    #[test]
    fn panic_safety_tiny_doc() {
        let mut doc = Document::new("tiny", 2.0, 2.0);

        // A normal path.
        let mut pn = PathNode::new(PathData::rect(0.0, 0.0, 2.0, 2.0));
        pn.fill = Fill::solid(Color::rgb(0.2, 0.4, 0.9));
        doc.add_node(
            SceneNode::new("p", Default::default(), SceneNodeKind::Path(pn)),
            None,
        );

        // A path with an absurd transform (huge coords) — must clip, not panic.
        let mut big = PathNode::new(PathData::rect(0.0, 0.0, 1.0, 1.0));
        big.fill = Fill::solid(Color::RED);
        doc.add_node(
            SceneNode::new("big", Default::default(), SceneNodeKind::Path(big))
                .with_transform(Transform::scale(1.0e9, 1.0e9)),
            None,
        );

        // A tiny raster.
        let r = solid_raster(1, 1, [10, 20, 30, 255]);
        doc.add_node(
            SceneNode::new(
                "r",
                Default::default(),
                SceneNodeKind::Raster(RasterNode::new(r)),
            ),
            None,
        );

        let mut view = CanvasView::new(2, 2);
        view.fit_to_rect(0.0, 0.0, doc.width, doc.height);

        let mut base = vec![0u8; 2 * 2 * 4];
        composite_document(&mut base, 2, 2, &doc, &view);
        assert_eq!(base.len(), 2 * 2 * 4);

        // Zero-size and undersized buffers are no-ops, not panics.
        composite_document(&mut [], 0, 0, &doc, &view);
        let mut tooshort = vec![0u8; 3];
        composite_document(&mut tooshort, 2, 2, &doc, &view);
    }
}
