use egui::{Align2, Color32, FontId, Painter, Pos2};
use photonic_core::node::NodeId;
use std::f32::consts::{FRAC_PI_2, TAU};

// ── Layout constants ──────────────────────────────────────────────────────────

const INNER_R: f32 = 40.0;
const OUTER_R: f32 = 130.0;
const LABEL_R: f32 = 90.0;
const ARC_STEPS: usize = 8;

/// Maximum items shown on a single page of the wheel.
const PAGE_SIZE: usize = 8;

/// Inner/outer radii of the page-indicator ring (drawn inside the dead zone).
const PAGE_IND_R_INNER: f32 = 12.0;
const PAGE_IND_R_OUTER: f32 = 24.0;
/// Angular gap (radians) between adjacent page-indicator segments.
const PAGE_IND_GAP: f32 = 0.08;

// ── Context ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum WheelNodeKind {
    Path,
    Group,
    Text,
}

#[derive(Debug, Clone)]
pub enum WheelContext {
    EmptyCanvas {
        canvas_x: f64,
        canvas_y: f64,
    },
    SingleNode {
        node_id: NodeId,
        node_kind: WheelNodeKind,
    },
    MultiNode {
        node_ids: Vec<NodeId>,
    },
}

// ── Actions ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum WheelAction {
    CreateRect,
    CreateRoundedRect,
    CreateEllipse,
    CreatePolygon,
    CreateStar,
    CreateText,
    DuplicateNode(NodeId),
    DeleteNode(NodeId),
    BringForward(NodeId),
    SendBackward(NodeId),
    BringToFront(NodeId),
    SendToBack(NodeId),
    GroupSelected,
    DeleteSelected,
    BoolUnion,
    BoolSubtract,
    BoolIntersect,
    BoolExclude,
    CopyAsSvg(NodeId),  // single-node context
    CopyAsSvgSelection, // multi-node context
    AddAnchorPoints(NodeId),
    SimplifyPath(NodeId),         // path nodes only
    OutlineStroke(NodeId),        // path nodes only
    ReversePathDirection(NodeId), // path nodes only
    AverageAnchorPoints(NodeId),  // path nodes only
    ClosePath(NodeId),            // path nodes only — close open subpaths
    InvertColors(NodeId),         // single-node context
    InvertColorsSelected,         // multi-node context
    ConvertToGrayscale(NodeId),   // single-node context
    ConvertToGrayscaleSelected,   // multi-node context
    UngroupNode(NodeId),          // group context only
}

// ── Menu item ─────────────────────────────────────────────────────────────────

pub struct RadialMenuItem {
    pub label: &'static str,
    pub action: WheelAction,
}

// ── Wheel state ───────────────────────────────────────────────────────────────

pub struct WheelState {
    /// Screen-space center where the wheel was opened.
    pub origin: Pos2,
    /// Canvas-space coords at open time (used by CreateShapeAtPos).
    pub canvas_pos: (f64, f64),
    /// All items across all pages.
    pub items: Vec<RadialMenuItem>,
    /// Current page index (0-based).
    pub page: usize,
    /// Hovered segment index within the current page, or None.
    pub hovered: Option<usize>,
    /// Short label shown above the wheel identifying the current context.
    pub context_label: &'static str,
}

// ── Item builder ──────────────────────────────────────────────────────────────

pub fn build_wheel_items(ctx: &WheelContext) -> Vec<RadialMenuItem> {
    match ctx {
        WheelContext::EmptyCanvas { .. } => vec![
            RadialMenuItem {
                label: "Rect",
                action: WheelAction::CreateRect,
            },
            RadialMenuItem {
                label: "Round Rect",
                action: WheelAction::CreateRoundedRect,
            },
            RadialMenuItem {
                label: "Ellipse",
                action: WheelAction::CreateEllipse,
            },
            RadialMenuItem {
                label: "Polygon",
                action: WheelAction::CreatePolygon,
            },
            RadialMenuItem {
                label: "Star",
                action: WheelAction::CreateStar,
            },
            RadialMenuItem {
                label: "Text",
                action: WheelAction::CreateText,
            },
        ],

        WheelContext::SingleNode { node_id, node_kind } => {
            let id = *node_id;
            let mut items = vec![
                RadialMenuItem {
                    label: "Duplicate",
                    action: WheelAction::DuplicateNode(id),
                },
                RadialMenuItem {
                    label: "Delete",
                    action: WheelAction::DeleteNode(id),
                },
                RadialMenuItem {
                    label: "Fwd",
                    action: WheelAction::BringForward(id),
                },
                RadialMenuItem {
                    label: "Back",
                    action: WheelAction::SendBackward(id),
                },
                RadialMenuItem {
                    label: "To Front",
                    action: WheelAction::BringToFront(id),
                },
                RadialMenuItem {
                    label: "To Back",
                    action: WheelAction::SendToBack(id),
                },
                RadialMenuItem {
                    label: "Copy as SVG",
                    action: WheelAction::CopyAsSvg(id),
                },
                RadialMenuItem {
                    label: "Invert",
                    action: WheelAction::InvertColors(id),
                },
                RadialMenuItem {
                    label: "Grayscale",
                    action: WheelAction::ConvertToGrayscale(id),
                },
            ];
            if matches!(node_kind, WheelNodeKind::Path) {
                items.push(RadialMenuItem {
                    label: "Add Anchors",
                    action: WheelAction::AddAnchorPoints(id),
                });
                items.push(RadialMenuItem {
                    label: "Simplify",
                    action: WheelAction::SimplifyPath(id),
                });
                items.push(RadialMenuItem {
                    label: "Outline Stroke",
                    action: WheelAction::OutlineStroke(id),
                });
                items.push(RadialMenuItem {
                    label: "Reverse",
                    action: WheelAction::ReversePathDirection(id),
                });
                items.push(RadialMenuItem {
                    label: "Average",
                    action: WheelAction::AverageAnchorPoints(id),
                });
                items.push(RadialMenuItem {
                    label: "Close Path",
                    action: WheelAction::ClosePath(id),
                });
            }
            if matches!(node_kind, WheelNodeKind::Group) {
                items.push(RadialMenuItem {
                    label: "Ungroup",
                    action: WheelAction::UngroupNode(id),
                });
            }
            items
        }

        WheelContext::MultiNode { .. } => vec![
            RadialMenuItem {
                label: "Group All",
                action: WheelAction::GroupSelected,
            },
            RadialMenuItem {
                label: "Delete All",
                action: WheelAction::DeleteSelected,
            },
            RadialMenuItem {
                label: "Union",
                action: WheelAction::BoolUnion,
            },
            RadialMenuItem {
                label: "Subtract",
                action: WheelAction::BoolSubtract,
            },
            RadialMenuItem {
                label: "Intersect",
                action: WheelAction::BoolIntersect,
            },
            RadialMenuItem {
                label: "Exclude",
                action: WheelAction::BoolExclude,
            },
            RadialMenuItem {
                label: "Copy as SVG",
                action: WheelAction::CopyAsSvgSelection,
            },
            RadialMenuItem {
                label: "Invert All",
                action: WheelAction::InvertColorsSelected,
            },
            RadialMenuItem {
                label: "Grayscale All",
                action: WheelAction::ConvertToGrayscaleSelected,
            },
        ],
    }
}

// ── WheelState impl ───────────────────────────────────────────────────────────

impl WheelState {
    pub fn new(origin: Pos2, canvas_pos: (f64, f64), ctx: &WheelContext) -> Self {
        let context_label = match ctx {
            WheelContext::EmptyCanvas { .. } => "Canvas",
            WheelContext::SingleNode {
                node_kind: WheelNodeKind::Group,
                ..
            } => "Group",
            WheelContext::SingleNode {
                node_kind: WheelNodeKind::Text,
                ..
            } => "Text",
            WheelContext::SingleNode { .. } => "Shape",
            WheelContext::MultiNode { .. } => "Selection",
        };
        Self {
            origin,
            canvas_pos,
            items: build_wheel_items(ctx),
            page: 0,
            hovered: None,
            context_label,
        }
    }

    /// Number of pages needed to show all items.
    pub fn page_count(&self) -> usize {
        if self.items.is_empty() {
            1
        } else {
            (self.items.len() + PAGE_SIZE - 1) / PAGE_SIZE
        }
    }

    /// Slice of items visible on the current page.
    pub fn current_page_items(&self) -> &[RadialMenuItem] {
        let start = self.page * PAGE_SIZE;
        let end = (start + PAGE_SIZE).min(self.items.len());
        &self.items[start..end]
    }

    /// Advance to the next page, wrapping around.
    pub fn next_page(&mut self) {
        let pages = self.page_count();
        self.page = (self.page + 1) % pages;
        self.hovered = None;
    }

    /// Go back to the previous page, wrapping around.
    pub fn prev_page(&mut self) {
        let pages = self.page_count();
        self.page = (self.page + pages - 1) % pages;
        self.hovered = None;
    }

    /// Update the hovered segment based on cursor position (relative to current page).
    pub fn update_hover(&mut self, cursor: Pos2) {
        let dx = cursor.x - self.origin.x;
        let dy = cursor.y - self.origin.y;
        let dist = (dx * dx + dy * dy).sqrt();

        let page_items = self.current_page_items();
        if dist < INNER_R || dist > OUTER_R || page_items.is_empty() {
            self.hovered = None;
            return;
        }

        let n = page_items.len() as f32;
        // Shift by half a segment so that hover regions are centred on visual segments.
        // Without this offset, regions start at the segment boundaries rather than the
        // segment midpoints, making the right half of each segment map to the next one.
        let angle = (dy.atan2(dx) + FRAC_PI_2 + TAU / (n * 2.0)).rem_euclid(TAU);
        let idx = (angle / (TAU / n)).floor() as usize;
        self.hovered = Some(idx.min(page_items.len() - 1));
    }

    /// Paint the wheel overlay. Must be called before any tool-handler `return`.
    pub fn draw(&self, painter: &Painter) {
        let page_items = self.current_page_items();
        if page_items.is_empty() {
            return;
        }

        let n = page_items.len();
        let seg_angle = TAU / n as f32;

        let bg_normal = Color32::from_rgba_unmultiplied(30, 30, 45, 220);
        let bg_hovered = Color32::from_rgba_unmultiplied(110, 86, 207, 240);
        let border = Color32::from_rgba_unmultiplied(90, 80, 160, 200);
        let text_color = Color32::from_rgb(220, 220, 235);
        let center_bg = Color32::from_rgba_unmultiplied(20, 20, 35, 200);

        let label_normal = Color32::from_rgb(180, 175, 210);
        let label_hovered = Color32::from_rgb(255, 255, 255);

        // ── Pie segments ──────────────────────────────────────────────────────
        for i in 0..n {
            let hovered = self.hovered == Some(i);
            let start = i as f32 * seg_angle - seg_angle / 2.0 - FRAC_PI_2;
            let end = start + seg_angle;
            let fill = if hovered { bg_hovered } else { bg_normal };
            let border_color = if hovered {
                Color32::from_rgba_unmultiplied(160, 130, 255, 240)
            } else {
                border
            };
            let border_width = if hovered { 2.0 } else { 1.0 };

            let pts = arc_polygon(self.origin, INNER_R, OUTER_R, start, end, ARC_STEPS);
            painter.add(egui::Shape::Path(egui::epaint::PathShape {
                points: pts,
                closed: true,
                fill,
                stroke: egui::epaint::PathStroke::new(border_width, border_color),
            }));
        }

        // ── Labels ────────────────────────────────────────────────────────────
        for i in 0..n {
            let hovered = self.hovered == Some(i);
            let mid = i as f32 * seg_angle - FRAC_PI_2;
            let col = if hovered { label_hovered } else { label_normal };
            let size = if hovered { 12.0 } else { 11.0 };
            painter.text(
                egui::pos2(
                    self.origin.x + mid.cos() * LABEL_R,
                    self.origin.y + mid.sin() * LABEL_R,
                ),
                Align2::CENTER_CENTER,
                page_items[i].label,
                FontId::proportional(size),
                col,
            );
        }

        // ── Context label pill (above the wheel) ─────────────────────────────
        self.draw_context_label(painter, border, center_bg, text_color);

        // ── Center dead-zone circle ───────────────────────────────────────────
        painter.circle_filled(self.origin, INNER_R - 2.0, center_bg);
        painter.circle_stroke(self.origin, INNER_R - 2.0, egui::Stroke::new(1.0, border));

        // ── Page indicator ring (only when multiple pages exist) ──────────────
        let page_count = self.page_count();
        if page_count > 1 {
            self.draw_page_indicator(painter, page_count);
        }
    }

    /// Draw a pill-shaped label above the wheel showing the context name.
    fn draw_context_label(
        &self,
        painter: &Painter,
        border: Color32,
        bg: Color32,
        text_color: Color32,
    ) {
        let font = FontId::proportional(12.0);
        let galley =
            painter.layout_no_wrap(self.context_label.to_string(), font.clone(), text_color);

        let pad_x = 10.0_f32;
        let pad_y = 4.0_f32;
        let pill_w = galley.size().x + pad_x * 2.0;
        let pill_h = galley.size().y + pad_y * 2.0;

        // Centre the pill horizontally above the wheel's top edge.
        let pill_center_y = self.origin.y - OUTER_R - pill_h / 2.0 - 6.0;
        let pill_rect = egui::Rect::from_center_size(
            egui::pos2(self.origin.x, pill_center_y),
            egui::vec2(pill_w, pill_h),
        );

        painter.rect(pill_rect, pill_h / 2.0, bg, egui::Stroke::new(1.0, border));
        painter.galley(
            egui::pos2(pill_rect.min.x + pad_x, pill_rect.min.y + pad_y),
            galley,
            text_color,
        );
    }

    /// Draw a segmented ring inside the dead zone showing current page position.
    fn draw_page_indicator(&self, painter: &Painter, page_count: usize) {
        let seg_span = TAU / page_count as f32 - PAGE_IND_GAP;
        let ind_active = Color32::from_rgba_unmultiplied(110, 86, 207, 255);
        let ind_inactive = Color32::from_rgba_unmultiplied(60, 55, 90, 200);
        let ind_border = Color32::from_rgba_unmultiplied(80, 70, 130, 180);

        for p in 0..page_count {
            // Start each segment at the top (-PI/2), spaced evenly clockwise.
            let center_angle = p as f32 * TAU / page_count as f32 - FRAC_PI_2;
            let start = center_angle - seg_span / 2.0;
            let end = start + seg_span;

            let fill = if p == self.page {
                ind_active
            } else {
                ind_inactive
            };
            let pts = arc_polygon(
                self.origin,
                PAGE_IND_R_INNER,
                PAGE_IND_R_OUTER,
                start,
                end,
                4,
            );
            painter.add(egui::Shape::Path(egui::epaint::PathShape {
                points: pts,
                closed: true,
                fill,
                stroke: egui::epaint::PathStroke::new(0.5, ind_border),
            }));
        }
    }
}

// ── Geometry helpers ──────────────────────────────────────────────────────────

fn arc_polygon(
    center: Pos2,
    r_inner: f32,
    r_outer: f32,
    start: f32,
    end: f32,
    steps: usize,
) -> Vec<Pos2> {
    let mut pts = Vec::with_capacity(steps * 2 + 2);

    for s in 0..=steps {
        let a = start + (end - start) * (s as f32 / steps as f32);
        pts.push(egui::pos2(
            center.x + a.cos() * r_inner,
            center.y + a.sin() * r_inner,
        ));
    }
    for s in (0..=steps).rev() {
        let a = start + (end - start) * (s as f32 / steps as f32);
        pts.push(egui::pos2(
            center.x + a.cos() * r_outer,
            center.y + a.sin() * r_outer,
        ));
    }

    pts
}
