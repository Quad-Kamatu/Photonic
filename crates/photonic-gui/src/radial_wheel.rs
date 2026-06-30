use egui::{Align2, Color32, FontId, Painter, Pos2};
use photonic_core::node::NodeId;
use std::f32::consts::{FRAC_PI_2, TAU};

// ── Layout constants ──────────────────────────────────────────────────────────

const INNER_R: f32 = 40.0;
const OUTER_R: f32 = 130.0;
const LABEL_R: f32 = 90.0;

/// Peek-tab band: drawn just outside the ring at the left (prev) / right (next).
const PEEK_R_INNER: f32 = OUTER_R + 4.0;
const PEEK_R_OUTER: f32 = OUTER_R + 26.0;
/// Half-angle (radians) of the prev/next peek wedge around the horizontal axis.
const PEEK_HALF_ANGLE: f32 = 0.42;

/// Category-position dots inside the dead zone.
const DOT_R: f32 = 2.0;
const DOT_GAP: f32 = 8.0;

// ── Animation constants ─────────────────────────────────────────────────────────

/// Duration (seconds) of the radial-wipe transition between categories.
const WIPE_DURATION: f64 = 0.18;
/// Angular width (radians) of the soft fade window at the sweep front.
const WIPE_FADE: f32 = 0.9;

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

// ── Menu item / category ────────────────────────────────────────────────────────

pub struct RadialMenuItem {
    pub label: &'static str,
    pub action: WheelAction,
}

/// A named group of verbs that fills one ring of the carousel.
pub struct WheelCategory {
    pub name: &'static str,
    /// Short icon glyph shown in the dead zone (kept ASCII-safe for the font).
    pub icon: &'static str,
    pub items: Vec<RadialMenuItem>,
}

/// Which peeked neighbour the cursor is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeekSide {
    Prev,
    Next,
}

// ── Category builder (context-filtered) ──────────────────────────────────────────
//
// Each of the 31 `WheelAction` variants lives in exactly one category, and each
// category is emitted only for the contexts where every one of its verbs applies:
//   • EmptyCanvas        → Create
//   • SingleNode(Path)   → Object · Order · Path · Color
//   • SingleNode(Text)   → Object · Order · Color
//   • SingleNode(Group)  → Object(+Ungroup) · Order · Color
//   • MultiNode          → Object · Combine · Color

pub fn build_wheel_categories(ctx: &WheelContext) -> Vec<WheelCategory> {
    match ctx {
        WheelContext::EmptyCanvas { .. } => vec![WheelCategory {
            name: "Create",
            icon: "+",
            items: vec![
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
        }],

        WheelContext::SingleNode { node_id, node_kind } => {
            let id = *node_id;

            // Object — duplicate / delete / (ungroup) / copy.
            let mut object = vec![
                RadialMenuItem {
                    label: "Duplicate",
                    action: WheelAction::DuplicateNode(id),
                },
                RadialMenuItem {
                    label: "Delete",
                    action: WheelAction::DeleteNode(id),
                },
            ];
            if matches!(node_kind, WheelNodeKind::Group) {
                object.push(RadialMenuItem {
                    label: "Ungroup",
                    action: WheelAction::UngroupNode(id),
                });
            }
            object.push(RadialMenuItem {
                label: "Copy as SVG",
                action: WheelAction::CopyAsSvg(id),
            });

            let order = vec![
                RadialMenuItem {
                    label: "To Front",
                    action: WheelAction::BringToFront(id),
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
                    label: "To Back",
                    action: WheelAction::SendToBack(id),
                },
            ];

            let color = vec![
                RadialMenuItem {
                    label: "Invert",
                    action: WheelAction::InvertColors(id),
                },
                RadialMenuItem {
                    label: "Grayscale",
                    action: WheelAction::ConvertToGrayscale(id),
                },
            ];

            let mut cats = vec![
                WheelCategory {
                    name: "Object",
                    icon: "▣",
                    items: object,
                },
                WheelCategory {
                    name: "Order",
                    icon: "≡",
                    items: order,
                },
            ];

            // Path verbs only apply to actual path nodes.
            if matches!(node_kind, WheelNodeKind::Path) {
                cats.push(WheelCategory {
                    name: "Path",
                    icon: "∿",
                    items: vec![
                        RadialMenuItem {
                            label: "Add Anchors",
                            action: WheelAction::AddAnchorPoints(id),
                        },
                        RadialMenuItem {
                            label: "Simplify",
                            action: WheelAction::SimplifyPath(id),
                        },
                        RadialMenuItem {
                            label: "Outline Stroke",
                            action: WheelAction::OutlineStroke(id),
                        },
                        RadialMenuItem {
                            label: "Reverse",
                            action: WheelAction::ReversePathDirection(id),
                        },
                        RadialMenuItem {
                            label: "Average",
                            action: WheelAction::AverageAnchorPoints(id),
                        },
                        RadialMenuItem {
                            label: "Close Path",
                            action: WheelAction::ClosePath(id),
                        },
                    ],
                });
            }

            cats.push(WheelCategory {
                name: "Color",
                icon: "◑",
                items: color,
            });
            cats
        }

        WheelContext::MultiNode { .. } => vec![
            WheelCategory {
                name: "Object",
                icon: "▣",
                items: vec![
                    RadialMenuItem {
                        label: "Group All",
                        action: WheelAction::GroupSelected,
                    },
                    RadialMenuItem {
                        label: "Delete All",
                        action: WheelAction::DeleteSelected,
                    },
                    RadialMenuItem {
                        label: "Copy as SVG",
                        action: WheelAction::CopyAsSvgSelection,
                    },
                ],
            },
            WheelCategory {
                name: "Combine",
                icon: "⊕",
                items: vec![
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
                ],
            },
            WheelCategory {
                name: "Color",
                icon: "◑",
                items: vec![
                    RadialMenuItem {
                        label: "Invert All",
                        action: WheelAction::InvertColorsSelected,
                    },
                    RadialMenuItem {
                        label: "Grayscale All",
                        action: WheelAction::ConvertToGrayscaleSelected,
                    },
                ],
            },
        ],
    }
}

// ── Wheel state ───────────────────────────────────────────────────────────────

pub struct WheelState {
    /// Screen-space center where the wheel was opened.
    pub origin: Pos2,
    /// Canvas-space coords at open time (used by CreateShapeAtPos).
    pub canvas_pos: (f64, f64),
    /// Context-filtered categories that the carousel rotates through.
    pub categories: Vec<WheelCategory>,
    /// Currently displayed category index.
    pub current_cat: usize,
    /// Category we are wiping away from (only meaningful while animating).
    prev_cat: Option<usize>,
    /// egui time (seconds) when the current wipe began; None = settled.
    anim_start: Option<f64>,
    /// Honour the reduced-motion preference: swap categories instantly.
    reduced_motion: bool,
    /// Hovered ring-segment index within the current category, or None.
    pub hovered: Option<usize>,
    /// Hovered peek tab (prev/next neighbour), or None.
    pub peek_hovered: Option<PeekSide>,
}

// ── WheelState impl ───────────────────────────────────────────────────────────

impl WheelState {
    pub fn new(
        origin: Pos2,
        canvas_pos: (f64, f64),
        ctx: &WheelContext,
        reduced_motion: bool,
    ) -> Self {
        Self {
            origin,
            canvas_pos,
            categories: build_wheel_categories(ctx),
            current_cat: 0,
            prev_cat: None,
            anim_start: None,
            reduced_motion,
            hovered: None,
            peek_hovered: None,
        }
    }

    /// Items of the category currently filling the ring.
    pub fn current_items(&self) -> &[RadialMenuItem] {
        self.categories
            .get(self.current_cat)
            .map(|c| c.items.as_slice())
            .unwrap_or(&[])
    }

    /// Action under the hovered ring segment, if any.
    pub fn hovered_action(&self) -> Option<WheelAction> {
        let idx = self.hovered?;
        self.current_items().get(idx).map(|it| it.action.clone())
    }

    fn cat_count(&self) -> usize {
        self.categories.len()
    }

    /// Index of the previous / next category (wrapping carousel).
    fn neighbour(&self, side: PeekSide) -> usize {
        let n = self.cat_count().max(1);
        match side {
            PeekSide::Prev => (self.current_cat + n - 1) % n,
            PeekSide::Next => (self.current_cat + 1) % n,
        }
    }

    /// Switch to `idx`, kicking off a radial-wipe (instant under reduced motion).
    fn set_category(&mut self, idx: usize, now: f64) {
        if idx == self.current_cat || self.cat_count() <= 1 {
            return;
        }
        if self.reduced_motion {
            self.prev_cat = None;
            self.anim_start = None;
        } else {
            self.prev_cat = Some(self.current_cat);
            self.anim_start = Some(now);
        }
        self.current_cat = idx;
        self.hovered = None;
        self.peek_hovered = None;
    }

    /// Rotate to the next category (scroll down / wheel-down).
    pub fn next_category(&mut self, now: f64) {
        let idx = self.neighbour(PeekSide::Next);
        self.set_category(idx, now);
    }

    /// Rotate to the previous category (scroll up / wheel-up).
    pub fn prev_category(&mut self, now: f64) {
        let idx = self.neighbour(PeekSide::Prev);
        self.set_category(idx, now);
    }

    /// Jump to whichever peek tab is hovered (no-op if none).
    pub fn jump_peek(&mut self, now: f64) {
        if let Some(side) = self.peek_hovered {
            let idx = self.neighbour(side);
            self.set_category(idx, now);
        }
    }

    /// Eased wipe progress in `0..=1` (1 = settled).
    fn wipe_progress(&self, now: f64) -> f32 {
        match self.anim_start {
            None => 1.0,
            Some(t0) => {
                let raw = ((now - t0) / WIPE_DURATION).clamp(0.0, 1.0) as f32;
                egui::emath::easing::cubic_out(raw)
            }
        }
    }

    /// Whether a wipe is still in flight (drives repaint requests).
    pub fn is_animating(&self, now: f64) -> bool {
        self.anim_start.is_some() && self.wipe_progress(now) < 1.0
    }

    /// Update hovered ring segment + peek tab from the cursor position.
    pub fn update_hover(&mut self, cursor: Pos2) {
        let dx = cursor.x - self.origin.x;
        let dy = cursor.y - self.origin.y;
        let dist = (dx * dx + dy * dy).sqrt();

        self.hovered = None;
        self.peek_hovered = None;

        // Peek tabs sit just outside the ring at the left / right.
        if self.cat_count() > 1 && (PEEK_R_INNER..=PEEK_R_OUTER).contains(&dist) {
            let ang = dy.atan2(dx); // 0 = right (+x), ±PI = left
            if ang.abs() <= PEEK_HALF_ANGLE {
                self.peek_hovered = Some(PeekSide::Next);
                return;
            }
            if (ang.abs() - std::f32::consts::PI).abs() <= PEEK_HALF_ANGLE {
                self.peek_hovered = Some(PeekSide::Prev);
                return;
            }
        }

        let items = self.current_items();
        if dist < INNER_R || dist > OUTER_R || items.is_empty() {
            return;
        }

        let n = items.len() as f32;
        // Shift by half a segment so hover regions centre on the visual segments.
        let angle = (dy.atan2(dx) + FRAC_PI_2 + TAU / (n * 2.0)).rem_euclid(TAU);
        let idx = (angle / (TAU / n)).floor() as usize;
        self.hovered = Some(idx.min(items.len() - 1));
    }

    /// Paint the wheel overlay. Must be called before any tool-handler `return`.
    pub fn draw(&self, painter: &Painter, now: f64) {
        if self.categories.is_empty() {
            return;
        }

        let progress = self.wipe_progress(now);
        let animating = self.anim_start.is_some() && progress < 1.0;

        // ── Ring segments ──────────────────────────────────────────────────────
        // Sweep travels 0 → TAU (+fade) clockwise from the top. A segment's alpha
        // ramps in as the sweep front passes its angular position.
        let sweep = progress * (TAU + WIPE_FADE);

        if animating {
            if let Some(prev) = self.prev_cat {
                // Old ring wipes OUT behind the sweep front.
                self.draw_ring(painter, prev, |seg_angle| {
                    1.0 - ((sweep - seg_angle) / WIPE_FADE).clamp(0.0, 1.0)
                });
            }
            // New ring wipes IN as the sweep passes.
            self.draw_ring(painter, self.current_cat, |seg_angle| {
                ((sweep - seg_angle) / WIPE_FADE).clamp(0.0, 1.0)
            });
        } else {
            self.draw_ring(painter, self.current_cat, |_| 1.0);
        }

        // ── Peek tabs (prev / next neighbours) ────────────────────────────────
        if self.cat_count() > 1 {
            self.draw_peek(painter, PeekSide::Prev);
            self.draw_peek(painter, PeekSide::Next);
        }

        // ── Center dead zone + category indicator ─────────────────────────────
        let center_bg = Color32::from_rgba_unmultiplied(20, 20, 35, 220);
        let border = Color32::from_rgba_unmultiplied(90, 80, 160, 200);
        painter.circle_filled(self.origin, INNER_R - 2.0, center_bg);
        painter.circle_stroke(self.origin, INNER_R - 2.0, egui::Stroke::new(1.0, border));
        self.draw_center_indicator(painter);
    }

    /// Draw one category's ring; `alpha_fn` maps a segment's clockwise-from-top
    /// angle to an opacity factor in `0..=1` (used for the radial wipe).
    fn draw_ring(&self, painter: &Painter, cat_idx: usize, alpha_fn: impl Fn(f32) -> f32) {
        let Some(cat) = self.categories.get(cat_idx) else {
            return;
        };
        let n = cat.items.len();
        if n == 0 {
            return;
        }
        let seg_angle = TAU / n as f32;
        let is_current = cat_idx == self.current_cat;

        let bg_normal = Color32::from_rgba_unmultiplied(30, 30, 45, 220);
        let bg_hovered = Color32::from_rgba_unmultiplied(110, 86, 207, 240);
        let border = Color32::from_rgba_unmultiplied(90, 80, 160, 200);
        let border_hovered = Color32::from_rgba_unmultiplied(160, 130, 255, 240);
        let label_normal = Color32::from_rgb(180, 175, 210);
        let label_hovered = Color32::from_rgb(255, 255, 255);

        // Segments.
        for i in 0..n {
            let a = alpha_fn(i as f32 * seg_angle);
            if a <= 0.003 {
                continue;
            }
            let hovered = is_current && self.hovered == Some(i);
            let start = i as f32 * seg_angle - seg_angle / 2.0 - FRAC_PI_2;
            let end = start + seg_angle;
            let fill = with_alpha(if hovered { bg_hovered } else { bg_normal }, a);
            let stroke_col = with_alpha(if hovered { border_hovered } else { border }, a);
            let stroke_w = if hovered { 2.0 } else { 1.0 };

            let pts = arc_polygon(self.origin, INNER_R, OUTER_R, start, end);
            painter.add(egui::Shape::Path(egui::epaint::PathShape {
                points: pts,
                closed: true,
                fill,
                // PathStroke is anti-aliased; combined with dense tessellation the
                // ring reads as a smooth circle rather than a faceted polygon.
                stroke: egui::epaint::PathStroke::new(stroke_w, stroke_col),
            }));
        }

        // Labels.
        for i in 0..n {
            let a = alpha_fn(i as f32 * seg_angle);
            if a <= 0.05 {
                continue;
            }
            let hovered = is_current && self.hovered == Some(i);
            let mid = i as f32 * seg_angle - FRAC_PI_2;
            let col = with_alpha(if hovered { label_hovered } else { label_normal }, a);
            let size = if hovered { 12.0 } else { 11.0 };
            painter.text(
                egui::pos2(
                    self.origin.x + mid.cos() * LABEL_R,
                    self.origin.y + mid.sin() * LABEL_R,
                ),
                Align2::CENTER_CENTER,
                cat.items[i].label,
                FontId::proportional(size),
                col,
            );
        }
    }

    /// Draw a small clickable tab hinting at the prev / next category.
    fn draw_peek(&self, painter: &Painter, side: PeekSide) {
        let Some(cat) = self.categories.get(self.neighbour(side)) else {
            return;
        };
        let hovered = self.peek_hovered == Some(side);

        // Centre angle: 0 (right) for Next, PI (left) for Prev.
        let center = match side {
            PeekSide::Next => 0.0,
            PeekSide::Prev => std::f32::consts::PI,
        };
        let start = center - PEEK_HALF_ANGLE;
        let end = center + PEEK_HALF_ANGLE;

        let fill = if hovered {
            Color32::from_rgba_unmultiplied(110, 86, 207, 235)
        } else {
            Color32::from_rgba_unmultiplied(40, 38, 62, 215)
        };
        let border = Color32::from_rgba_unmultiplied(90, 80, 160, 200);
        let pts = arc_polygon(self.origin, PEEK_R_INNER, PEEK_R_OUTER, start, end);
        painter.add(egui::Shape::Path(egui::epaint::PathShape {
            points: pts,
            closed: true,
            fill,
            stroke: egui::epaint::PathStroke::new(1.0, border),
        }));

        // Neighbour name, just outside the tab.
        let label_r = PEEK_R_OUTER + 12.0;
        let col = if hovered {
            Color32::from_rgb(255, 255, 255)
        } else {
            Color32::from_rgb(185, 180, 215)
        };
        let anchor = match side {
            PeekSide::Next => Align2::LEFT_CENTER,
            PeekSide::Prev => Align2::RIGHT_CENTER,
        };
        painter.text(
            egui::pos2(self.origin.x + center.cos() * label_r, self.origin.y),
            anchor,
            cat.name,
            FontId::proportional(10.0),
            col,
        );
    }

    /// Draw the current category's icon + name and the carousel position dots.
    fn draw_center_indicator(&self, painter: &Painter) {
        let Some(cat) = self.categories.get(self.current_cat) else {
            return;
        };
        let n = self.cat_count();
        let text_color = Color32::from_rgb(232, 230, 245);
        let icon_color = Color32::from_rgb(170, 150, 235);

        // Icon above the name (skip if it would be too cramped).
        let has_dots = n > 1;
        let name_dy = if has_dots { -2.0 } else { 0.0 };

        if !cat.icon.is_empty() {
            painter.text(
                egui::pos2(self.origin.x, self.origin.y - 13.0),
                Align2::CENTER_CENTER,
                cat.icon,
                FontId::proportional(13.0),
                icon_color,
            );
        }

        painter.text(
            egui::pos2(self.origin.x, self.origin.y + name_dy),
            Align2::CENTER_CENTER,
            cat.name,
            FontId::proportional(11.0),
            text_color,
        );

        // Carousel position dots near the bottom of the dead zone.
        if has_dots {
            let total_w = (n as f32 - 1.0) * DOT_GAP;
            let y = self.origin.y + 14.0;
            let x0 = self.origin.x - total_w / 2.0;
            for i in 0..n {
                let cx = x0 + i as f32 * DOT_GAP;
                let col = if i == self.current_cat {
                    Color32::from_rgb(150, 120, 240)
                } else {
                    Color32::from_rgba_unmultiplied(110, 105, 140, 200)
                };
                painter.circle_filled(egui::pos2(cx, y), DOT_R, col);
            }
        }
    }
}

// ── Geometry helpers ──────────────────────────────────────────────────────────

/// Multiply a colour's alpha by `factor` (clamped), preserving RGB.
fn with_alpha(c: Color32, factor: f32) -> Color32 {
    let a = (c.a() as f32 * factor.clamp(0.0, 1.0)).round() as u8;
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

/// Pick a tessellation step count so the arc reads as a true circle. Roughly one
/// step per ~3° of arc, clamped to a sensible range.
fn arc_steps(start: f32, end: f32) -> usize {
    let span = (end - start).abs();
    ((span / 0.052).ceil() as usize).clamp(12, 96)
}

fn arc_polygon(center: Pos2, r_inner: f32, r_outer: f32, start: f32, end: f32) -> Vec<Pos2> {
    let steps = arc_steps(start, end);
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
