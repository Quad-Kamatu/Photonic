//! Navigator panel — a read-only mini-map of all visible nodes plus
//! click-to-navigate. Renders a scaled bird's-eye thumbnail of the document
//! (the selected node highlighted) and, when the user clicks or drags inside
//! the thumbnail, emits a [`PanelAction::CenterViewOn`] so the main canvas
//! recenters on that point — the conventional Illustrator/Photoshop Navigator
//! behaviour.
//!
//! Each path is drawn as a decimated outline (not just its bounding box) so the
//! silhouette is recognizable, while a per-node vertex budget
//! ([`NAV_LOD_VERTICES`]) keeps the cost low. Text and raster nodes — which have
//! no vector outline — fall back to a bounding-box rectangle.

use egui::{RichText, Ui};
use kurbo::{BezPath, PathEl};
use photonic_core::{node::NodeId, Document, SceneNode, SceneNodeKind};

use super::PanelAction;

/// Fallback canvas extent used when the document has no visible nodes.
const FALLBACK_W: f64 = 800.0;
const FALLBACK_H: f64 = 600.0;

/// Per-node vertex budget for the decimated outline drawn in the mini-map.
/// Paths with more flattened vertices are simplified down to this; simpler
/// shapes are drawn as-is. Low enough to stay cheap, high enough that common
/// shapes (stars, gears, blobs, logos) stay recognizable.
const NAV_LOD_VERTICES: usize = 28;

/// Samples per bezier segment when flattening curves to a polyline. Coarse on
/// purpose — the mini-map is at most 200px wide.
const CURVE_SAMPLES: usize = 8;

/// A flattened subpath in world space: the points plus whether it is closed
/// (closed → filled silhouette, open → stroked polyline).
struct Subpath {
    pts: Vec<(f64, f64)>,
    closed: bool,
}

/// A node's precomputed mini-map representation.
struct NavNode {
    /// World-space bounding box: x, y, w, h (used for canvas-bounds + fallback).
    bx: f64,
    by: f64,
    bw: f64,
    bh: f64,
    /// Decimated world-space outline. Empty → draw the bounding-box rect.
    outline: Vec<Subpath>,
    id: NodeId,
}

/// Draw the Navigator collapsing section.
///
/// `forced_open` mirrors the inspector search behaviour: `Some(true)` forces the
/// header open while a property search is active, `None` leaves the user's
/// expanded/collapsed state untouched.
///
/// Returns `Some(PanelAction::CenterViewOn { .. })` when the user clicks or
/// drags inside the thumbnail to recenter the canvas there.
pub fn draw_navigator(
    ui: &mut Ui,
    doc: &Document,
    selected_id: Option<NodeId>,
    forced_open: Option<bool>,
) -> Option<PanelAction> {
    let mut action: Option<PanelAction> = None;

    egui::CollapsingHeader::new("Navigator")
        .default_open(false)
        .open(forced_open)
        .show(ui, |ui: &mut Ui| {
            // Collect visible nodes with their bbox + decimated outline.
            let nav_nodes: Vec<NavNode> = doc
                .nodes_in_draw_order()
                .into_iter()
                .filter(|n| n.visible)
                .filter_map(|n: &SceneNode| {
                    let lb = n.local_bounds()?;
                    let (x0, y0) = n.transform.apply(lb.x0, lb.y0);
                    let (x1, y1) = n.transform.apply(lb.x1, lb.y1);
                    let bx = x0.min(x1);
                    let by = y0.min(y1);
                    let bw = (x1 - x0).abs().max(1.0_f64);
                    let bh = (y1 - y0).abs().max(1.0_f64);
                    Some(NavNode {
                        bx,
                        by,
                        bw,
                        bh,
                        outline: node_outline(n),
                        id: n.id,
                    })
                })
                .collect();

            // Canvas bounds: expand by real outline points where available (so
            // rotated shapes get their true extent), else by the bbox corners.
            let mut bounds = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for nn in &nav_nodes {
                if nn.outline.is_empty() {
                    expand(&mut bounds, nn.bx, nn.by);
                    expand(&mut bounds, nn.bx + nn.bw, nn.by + nn.bh);
                } else {
                    for s in &nn.outline {
                        for &(x, y) in &s.pts {
                            expand(&mut bounds, x, y);
                        }
                    }
                }
            }
            let (mut min_x, mut min_y, mut max_x, mut max_y) = bounds;
            if min_x == f64::MAX {
                min_x = 0.0;
                min_y = 0.0;
                max_x = FALLBACK_W;
                max_y = FALLBACK_H;
            }
            let canvas_w = (max_x - min_x).max(1.0);
            let canvas_h = (max_y - min_y).max(1.0);

            // Allocate a fixed-height thumbnail area. `click_and_drag` lets the
            // user both tap a spot and scrub the view across the document.
            let nav_w = ui.available_width().min(200.0);
            let nav_h = (nav_w * (canvas_h / canvas_w) as f32).clamp(40.0, 160.0);
            let (response, painter) =
                ui.allocate_painter(egui::vec2(nav_w, nav_h), egui::Sense::click_and_drag());
            let rect = response.rect;
            painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(30, 30, 40));

            // Map canvas space → thumbnail space (uniform scale, centered).
            let sx = nav_w as f64 / canvas_w;
            let sy = nav_h as f64 / canvas_h;
            let scale = sx.min(sy) as f32;
            let off_x = rect.min.x + ((nav_w as f64 - canvas_w * scale as f64) * 0.5) as f32;
            let off_y = rect.min.y + ((nav_h as f64 - canvas_h * scale as f64) * 0.5) as f32;
            let to_screen = |x: f64, y: f64| -> egui::Pos2 {
                egui::pos2(
                    off_x + ((x - min_x) * scale as f64) as f32,
                    off_y + ((y - min_y) * scale as f64) as f32,
                )
            };

            // Draw each node: decimated outline where we have one, else a rect.
            for nn in &nav_nodes {
                let is_selected = selected_id == Some(nn.id);
                let fill_color = if is_selected {
                    egui::Color32::from_rgba_unmultiplied(100, 180, 255, 180)
                } else {
                    egui::Color32::from_rgba_unmultiplied(180, 180, 200, 110)
                };
                let stroke_color = if is_selected {
                    egui::Color32::WHITE
                } else {
                    egui::Color32::from_rgba_unmultiplied(150, 150, 170, 160)
                };

                if nn.outline.is_empty() {
                    let r = egui::Rect::from_two_pos(
                        to_screen(nn.bx, nn.by),
                        to_screen(nn.bx + nn.bw, nn.by + nn.bh),
                    );
                    painter.rect_filled(r, 1.0, fill_color);
                    if is_selected {
                        painter.rect_stroke(r, 1.0, egui::Stroke::new(1.0, egui::Color32::WHITE));
                    }
                    continue;
                }

                for s in &nn.outline {
                    let pts: Vec<egui::Pos2> =
                        s.pts.iter().map(|&(x, y)| to_screen(x, y)).collect();
                    if s.closed && pts.len() >= 3 {
                        painter.add(egui::Shape::convex_polygon(
                            pts,
                            fill_color,
                            egui::Stroke::new(0.6, stroke_color),
                        ));
                    } else if pts.len() >= 2 {
                        // Open path: stroke the polyline, no fill.
                        painter.add(egui::Shape::line(pts, egui::Stroke::new(0.8, stroke_color)));
                    }
                }
            }

            // ── Click / drag to navigate ──────────────────────────────────────
            // Invert the canvas→thumbnail mapping to recover the canvas-space
            // point under the pointer, then ask the main loop to recenter there.
            if response.clicked() || response.dragged() {
                if let Some(p) = response.interact_pointer_pos() {
                    if scale > 0.0 {
                        let canvas_x = min_x + ((p.x - off_x) / scale) as f64;
                        let canvas_y = min_y + ((p.y - off_y) / scale) as f64;
                        action = Some(PanelAction::CenterViewOn { canvas_x, canvas_y });
                    }
                }
            }
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
            response.on_hover_text("Click or drag to recenter the canvas here");

            // Stats line.
            ui.add_space(2.0);
            ui.label(
                RichText::new(format!(
                    "{} nodes  {:.0}×{:.0}",
                    nav_nodes.len(),
                    canvas_w,
                    canvas_h
                ))
                .small()
                .weak(),
            );
        });

    action
}

/// Grow a `(min_x, min_y, max_x, max_y)` bounds tuple to include `(x, y)`.
fn expand(b: &mut (f64, f64, f64, f64), x: f64, y: f64) {
    if x < b.0 {
        b.0 = x;
    }
    if y < b.1 {
        b.1 = y;
    }
    if x > b.2 {
        b.2 = x;
    }
    if y > b.3 {
        b.3 = y;
    }
}

/// Build a decimated, world-space outline for a node within the
/// [`NAV_LOD_VERTICES`] budget. Returns empty for non-path nodes (text/raster),
/// which the caller renders as a bounding-box rect.
fn node_outline(node: &SceneNode) -> Vec<Subpath> {
    let SceneNodeKind::Path(p) = &node.kind else {
        return Vec::new();
    };
    let bez = p.path_data.to_bez_path();
    let mut subs = flatten_subpaths(&bez);

    // Decimate to the per-node budget, sharing it across subpaths in proportion
    // to their original vertex counts (min 3 each so each subpath survives).
    let total: usize = subs.iter().map(|s| s.pts.len()).sum();
    // `total > NAV_LOD_VERTICES` already implies `total > 0` (the budget is
    // non-zero), so it also guards the `/ total` below.
    if total > NAV_LOD_VERTICES {
        for s in subs.iter_mut() {
            let target =
                ((NAV_LOD_VERTICES as f64) * (s.pts.len() as f64 / total as f64)).round() as usize;
            decimate(&mut s.pts, target);
        }
    }

    // Local → world space.
    for s in subs.iter_mut() {
        for pt in s.pts.iter_mut() {
            *pt = node.transform.apply(pt.0, pt.1);
        }
    }
    subs.retain(|s| s.pts.len() >= 2);
    subs
}

/// Flatten a `BezPath` into polyline subpaths in its own (local) coordinates,
/// tracking whether each subpath is closed.
fn flatten_subpaths(bez: &BezPath) -> Vec<Subpath> {
    let mut subs: Vec<Subpath> = Vec::new();
    let mut cur: Vec<(f64, f64)> = Vec::new();
    let mut closed = false;
    let mut pen = (0.0_f64, 0.0_f64);

    for el in bez.elements() {
        match el {
            PathEl::MoveTo(p) => {
                if cur.len() >= 2 {
                    subs.push(Subpath {
                        pts: std::mem::take(&mut cur),
                        closed,
                    });
                } else {
                    cur.clear();
                }
                closed = false;
                pen = (p.x, p.y);
                cur.push(pen);
            }
            PathEl::LineTo(p) => {
                pen = (p.x, p.y);
                cur.push(pen);
            }
            PathEl::QuadTo(c, p) => {
                let (x0, y0) = pen;
                for i in 1..=CURVE_SAMPLES {
                    let t = i as f64 / CURVE_SAMPLES as f64;
                    let u = 1.0 - t;
                    cur.push((
                        u * u * x0 + 2.0 * u * t * c.x + t * t * p.x,
                        u * u * y0 + 2.0 * u * t * c.y + t * t * p.y,
                    ));
                }
                pen = (p.x, p.y);
            }
            PathEl::CurveTo(c1, c2, p) => {
                let (x0, y0) = pen;
                for i in 1..=CURVE_SAMPLES {
                    let t = i as f64 / CURVE_SAMPLES as f64;
                    let u = 1.0 - t;
                    cur.push((
                        u * u * u * x0
                            + 3.0 * u * u * t * c1.x
                            + 3.0 * u * t * t * c2.x
                            + t * t * t * p.x,
                        u * u * u * y0
                            + 3.0 * u * u * t * c1.y
                            + 3.0 * u * t * t * c2.y
                            + t * t * t * p.y,
                    ));
                }
                pen = (p.x, p.y);
            }
            PathEl::ClosePath => {
                closed = true;
            }
        }
    }
    if cur.len() >= 2 {
        subs.push(Subpath { pts: cur, closed });
    }
    subs
}

/// Reduce a polyline to at most `target` vertices by repeatedly dropping the
/// interior vertex whose removal changes the silhouette least — the area of the
/// triangle it forms with its neighbours (Visvalingam–Whyatt). Endpoints are
/// preserved. `target` is floored at 3 so a shape never collapses to a line.
fn decimate(points: &mut Vec<(f64, f64)>, target: usize) {
    let target = target.max(3);
    while points.len() > target {
        let mut min_area = f64::INFINITY;
        let mut min_i = 1;
        for i in 1..points.len() - 1 {
            let a = points[i - 1];
            let b = points[i];
            let c = points[i + 1];
            // Twice the triangle area (constant factor is irrelevant for argmin).
            let area = ((b.0 - a.0) * (c.1 - a.1) - (c.0 - a.0) * (b.1 - a.1)).abs();
            if area < min_area {
                min_area = area;
                min_i = i;
            }
        }
        points.remove(min_i);
    }
}
