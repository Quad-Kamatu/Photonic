//! Adaptive hotbar — an always-on, slim second toolbar row that surfaces the
//! most relevant tools/actions for the current selection context (Phase 4 of the
//! #154 drawer-UI redesign).
//!
//! The hotbar is a *registry + ranking* layer; it never reimplements any
//! operation. Each [`HotbarItem`] either selects a [`Tool`] (reusing the app's
//! existing tool-apply path) or fires a [`HotbarAction`] that the app maps to the
//! existing [`crate::panels::PanelAction`] variants against the live selection.
//!
//! Two modes, chosen in Edit ▸ Behavior:
//! - **Static** — the curated default order for the active context bucket.
//! - **Adaptive** — the same items re-ordered by the user's own usage
//!   (frequency-with-mild-time-decay, persisted per bucket in prefs), with a few
//!   leading **pinned** slots that never reorder so the bar stays calm.

use crate::tools::Tool;
use egui_phosphor::regular as ph;
use serde::{Deserialize, Serialize};

/// How many leading slots are pinned (kept in their default position) in Adaptive
/// mode. Only the items *after* these slots are re-ranked by usage, so the bar
/// never fully rearranges under the user.
pub const PINNED_SLOTS: usize = 2;

/// Whether the hotbar shows a fixed curated order or adapts to usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum HotbarMode {
    /// Curated default order per context bucket (the calm default).
    #[default]
    Static,
    /// Items re-ranked by the user's own per-bucket usage.
    Adaptive,
}

impl HotbarMode {
    pub fn label(self) -> &'static str {
        match self {
            HotbarMode::Static => "Static",
            HotbarMode::Adaptive => "Adaptive",
        }
    }
}

/// Context bucket derived from the current selection each frame. Drives which
/// curated item set the hotbar renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HotbarBucket {
    /// Nothing selected — surface common tools.
    Empty,
    /// A single shape / path / group / image node — surface object actions.
    Shape,
    /// A single text node — surface text-relevant object actions.
    Text,
    /// Two or more nodes selected — surface multi-object actions.
    Multi,
}

impl HotbarBucket {
    /// Stable key used for the usage-score map persisted in prefs.
    pub fn key(self) -> &'static str {
        match self {
            HotbarBucket::Empty => "empty",
            HotbarBucket::Shape => "shape",
            HotbarBucket::Text => "text",
            HotbarBucket::Multi => "multi",
        }
    }
}

/// What invoking a hotbar item does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotbarEffect {
    /// Select a tool (reuses the app's tool-apply path verbatim).
    Tool(Tool),
    /// Fire a selection-scoped verb the app maps to existing `PanelAction`(s).
    Action(HotbarAction),
}

/// Selection-scoped verbs the hotbar can fire. The app resolves each to the
/// existing [`crate::panels::PanelAction`](s) using the live selection — the
/// hotbar never constructs document mutations itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotbarAction {
    Duplicate,
    Delete,
    Group,
    Ungroup,
    BringToFront,
    SendToBack,
    BoolUnion,
    BoolSubtract,
    AlignLeft,
    AlignCenterH,
    CopyAsSvg,
    Invert,
    Grayscale,
}

/// A single registry entry: a stable id, a phosphor icon, a tooltip, and an
/// effect.
#[derive(Debug, Clone, Copy)]
pub struct HotbarItem {
    /// Stable identifier — also the key under which usage is tracked.
    pub id: &'static str,
    pub icon: &'static str,
    pub tooltip: &'static str,
    pub effect: HotbarEffect,
}

const fn tool(id: &'static str, t: Tool) -> HotbarItem {
    HotbarItem {
        id,
        icon: t.icon(),
        tooltip: t.label(),
        effect: HotbarEffect::Tool(t),
    }
}

const fn action(
    id: &'static str,
    icon: &'static str,
    tooltip: &'static str,
    a: HotbarAction,
) -> HotbarItem {
    HotbarItem {
        id,
        icon,
        tooltip,
        effect: HotbarEffect::Action(a),
    }
}

// ── Curated default sets per bucket ─────────────────────────────────────────

/// Empty canvas: the common drawing/selection tools. First two are pinned.
const EMPTY_ITEMS: &[HotbarItem] = &[
    tool("tool.select", Tool::Select),
    tool("tool.pen", Tool::Pen),
    tool("tool.direct_select", Tool::DirectSelect),
    tool("tool.rect", Tool::Rectangle),
    tool("tool.ellipse", Tool::Ellipse),
    tool("tool.text", Tool::Text),
    tool("tool.line", Tool::Line),
    tool("tool.polygon", Tool::Polygon),
    tool("tool.star", Tool::Star),
];

/// Base single-object actions that apply to *any* lone node — a path, a group,
/// an image, or text. These are the items the Shape and Text buckets share; the
/// fill-only colour ops (Invert / Grayscale) are *not* here because they only
/// run on a path with paintable colour (see [`FILL_ITEMS`]). First two
/// (Duplicate, Delete) are pinned.
const OBJECT_ITEMS: &[HotbarItem] = &[
    action("act.duplicate", ph::COPY, "Duplicate", HotbarAction::Duplicate),
    action("act.delete", ph::TRASH, "Delete", HotbarAction::Delete),
    action(
        "act.front",
        ph::ARROW_LINE_UP,
        "Bring to Front",
        HotbarAction::BringToFront,
    ),
    action(
        "act.back",
        ph::ARROW_LINE_DOWN,
        "Send to Back",
        HotbarAction::SendToBack,
    ),
    action("act.copy_svg", ph::CODE, "Copy as SVG", HotbarAction::CopyAsSvg),
];

/// Curated set for a single **text** node. Text nodes have no path fill/stroke,
/// so the colour ops (Invert / Grayscale) — whose handlers only mutate
/// `SceneNodeKind::Path` — would be inert dead buttons here and are omitted. The
/// remaining verbs all apply to a text node (CopyAsSvg works on any node via
/// `export_nodes_as_svg`).
const TEXT_ITEMS: &[HotbarItem] = OBJECT_ITEMS;

/// Fill-only colour ops, appended to the Shape bucket *only* when the lone node
/// is a path whose colour the op can actually change (a non-empty fill or an
/// enabled stroke). Gating these keeps them from being dead buttons on a group,
/// an image, or a fill-less path.
const FILL_ITEMS: &[HotbarItem] = &[
    action("act.invert", ph::SWAP, "Invert Colors", HotbarAction::Invert),
    action(
        "act.grayscale",
        ph::CIRCLE_HALF,
        "Convert to Grayscale",
        HotbarAction::Grayscale,
    ),
];

/// The single-group extra: Ungroup is only appended for a single group node.
const UNGROUP_ITEM: HotbarItem = action(
    "act.ungroup",
    ph::CORNERS_OUT,
    "Ungroup",
    HotbarAction::Ungroup,
);

/// Multi-selection actions. First two (Group, Duplicate) are pinned.
const MULTI_ITEMS: &[HotbarItem] = &[
    action("act.group", ph::FRAME_CORNERS, "Group", HotbarAction::Group),
    action("act.duplicate", ph::COPY, "Duplicate", HotbarAction::Duplicate),
    action("act.delete", ph::TRASH, "Delete", HotbarAction::Delete),
    action("act.bool_union", ph::UNITE, "Unite", HotbarAction::BoolUnion),
    action(
        "act.bool_subtract",
        ph::SUBTRACT,
        "Subtract",
        HotbarAction::BoolSubtract,
    ),
    action(
        "act.align_left",
        ph::ALIGN_LEFT,
        "Align Left",
        HotbarAction::AlignLeft,
    ),
    action(
        "act.align_center_h",
        ph::ALIGN_CENTER_HORIZONTAL,
        "Align Center (horizontal)",
        HotbarAction::AlignCenterH,
    ),
    action(
        "act.front",
        ph::ARROW_LINE_UP,
        "Bring to Front",
        HotbarAction::BringToFront,
    ),
    action(
        "act.back",
        ph::ARROW_LINE_DOWN,
        "Send to Back",
        HotbarAction::SendToBack,
    ),
    action("act.copy_svg", ph::CODE, "Copy as SVG", HotbarAction::CopyAsSvg),
    action("act.invert", ph::SWAP, "Invert Colors", HotbarAction::Invert),
    action(
        "act.grayscale",
        ph::CIRCLE_HALF,
        "Convert to Grayscale",
        HotbarAction::Grayscale,
    ),
];

/// The curated default item list for a bucket, in static order.
///
/// For the single-node **Shape** bucket, the fill-only colour ops (Invert /
/// Grayscale) are appended *only* when `single_is_fillable_path` — i.e. the lone
/// node is a path the op can actually recolour — so they never appear as dead
/// buttons on a group/image. For a single group node, Ungroup is appended (it
/// only applies there). The **Text** bucket gets [`TEXT_ITEMS`], which omits the
/// colour ops entirely (a text node has no path fill to invert).
pub fn default_items(
    bucket: HotbarBucket,
    single_is_group: bool,
    single_is_fillable_path: bool,
) -> Vec<HotbarItem> {
    match bucket {
        HotbarBucket::Empty => EMPTY_ITEMS.to_vec(),
        HotbarBucket::Shape => {
            let mut v = OBJECT_ITEMS.to_vec();
            if single_is_fillable_path {
                v.extend_from_slice(FILL_ITEMS);
            }
            if single_is_group {
                v.push(UNGROUP_ITEM);
            }
            v
        }
        HotbarBucket::Text => TEXT_ITEMS.to_vec(),
        HotbarBucket::Multi => MULTI_ITEMS.to_vec(),
    }
}

/// Produce the ordered item list for a bucket under a mode.
///
/// - **Static**: the curated default order unchanged.
/// - **Adaptive**: the first [`PINNED_SLOTS`] items stay put; the remainder are
///   stably re-ordered by descending usage `score`. Stable sort means a
///   cold-start (all scores equal/zero) preserves the default order, so the bar
///   never reshuffles without data.
///
/// This is called only when the cache is (re)built — on bucket/mode change — not
/// per click or per frame, keeping the order calm.
pub fn ordered_items(
    bucket: HotbarBucket,
    single_is_group: bool,
    single_is_fillable_path: bool,
    mode: HotbarMode,
    score: impl Fn(&str) -> f32,
) -> Vec<HotbarItem> {
    let defaults = default_items(bucket, single_is_group, single_is_fillable_path);
    if mode == HotbarMode::Static {
        return defaults;
    }
    let pin = PINNED_SLOTS.min(defaults.len());
    let mut out = defaults[..pin].to_vec();
    let mut rest = defaults[pin..].to_vec();
    // Stable sort: ties (incl. all-zero cold start) keep curated order.
    rest.sort_by(|a, b| {
        score(b.id)
            .partial_cmp(&score(a.id))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out.extend(rest);
    out
}

// ── Rendering ───────────────────────────────────────────────────────────────

/// Render the hotbar's items as a slim icon row. Items that don't fit collapse
/// into a trailing "More" (…) popover. A tool item is highlighted when it is the
/// active tool. Returns the invoked item, if any (handled by the app).
pub fn render(ui: &mut egui::Ui, items: &[HotbarItem], active_tool: Tool) -> Option<HotbarItem> {
    let mut invoked: Option<HotbarItem> = None;
    ui.horizontal(|ui| {
        // Rough per-item width for the overflow split (icon button + spacing).
        let per = 30.0_f32;
        let avail = ui.available_width();
        let capacity = (avail / per).floor().max(1.0) as usize;

        let (head, tail): (&[HotbarItem], &[HotbarItem]) = if items.len() <= capacity {
            (items, &[])
        } else {
            // Reserve one slot for the More button.
            let n = capacity.saturating_sub(1).max(1).min(items.len());
            items.split_at(n)
        };

        for item in head {
            if hotbar_button(ui, item, active_tool).clicked() {
                invoked = Some(*item);
            }
        }

        if !tail.is_empty() {
            ui.menu_button(ph::DOTS_THREE, |ui| {
                for item in tail {
                    if ui
                        .button(format!("{}  {}", item.icon, item.tooltip))
                        .clicked()
                    {
                        invoked = Some(*item);
                        ui.close_menu();
                    }
                }
            })
            .response
            .on_hover_text("More…");
        }
    });
    invoked
}

/// One hotbar button: icon-only, highlighted when it represents the active tool.
fn hotbar_button(ui: &mut egui::Ui, item: &HotbarItem, active_tool: Tool) -> egui::Response {
    let is_active = matches!(item.effect, HotbarEffect::Tool(t) if t == active_tool);
    ui.selectable_label(is_active, egui::RichText::new(item.icon))
        .on_hover_text(item.tooltip)
}
