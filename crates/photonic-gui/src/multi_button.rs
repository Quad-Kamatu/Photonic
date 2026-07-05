//! A pill-shaped segmented "multi-button" — an egui port of Squidhub's
//! `MultiButton` React component.
//!
//! At rest every item is icon-only, taking an equal share of a fixed-width
//! rounded pill. Hovering an item smoothly expands it to icon + label while the
//! other items compress back to icon-only and the inter-item dividers fade out.
//! The container width never changes (it is sized to the single widest expanded
//! state), so the control never reflows its surroundings.

use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, Ui, Vec2};

/// One segment of a [`multi_button`].
pub struct MultiButtonItem<'a> {
    /// Phosphor icon glyph shown at rest and while expanded.
    pub icon: &'a str,
    /// Label revealed when this item is hovered.
    pub label: &'a str,
    /// Tooltip text on hover.
    pub hover: &'a str,
    /// Greyed-out and non-clickable when false.
    pub enabled: bool,
}

impl<'a> MultiButtonItem<'a> {
    pub fn new(icon: &'a str, label: &'a str, hover: &'a str) -> Self {
        Self {
            icon,
            label,
            hover,
            enabled: true,
        }
    }
}

/// Draw the pill and return the index of the item clicked this frame, if any.
///
/// `id_salt` must be stable and unique per instance so the hover/animation
/// state persists across frames.
pub fn multi_button(ui: &mut Ui, id_salt: &str, items: &[MultiButtonItem]) -> Option<usize> {
    let n = items.len();
    if n == 0 {
        return None;
    }

    // ── Metrics ─────────────────────────────────────────────────────────────
    const HEIGHT: f32 = 24.0;
    const CELL: f32 = 24.0; // icon column width
    const PAD_END: f32 = 10.0; // trailing space after a revealed label
    const DIVIDER: f32 = 1.0;
    const DUR: f32 = 0.18; // animation time (s)
    let icon_font = FontId::proportional(14.0);
    let label_font = FontId::proportional(12.0);

    let accent = Color32::from_rgb(130, 105, 225);
    let visuals = ui.visuals().clone();
    let text_col = visuals.widgets.inactive.fg_stroke.color;
    let pill_bg = visuals.widgets.inactive.bg_fill.gamma_multiply(0.6);
    let divider_col = visuals.weak_text_color();

    // Widest revealed label defines the (fixed) expanded slot width.
    let max_label = items
        .iter()
        .map(|it| {
            ui.fonts(|f| {
                f.layout_no_wrap(it.label.to_owned(), label_font.clone(), text_col)
                    .rect
                    .width()
            })
        })
        .fold(0.0_f32, f32::max)
        + PAD_END;

    let id = ui.make_persistent_id(id_salt);
    let hovered: Option<usize> = ui.data(|d| d.get_temp(id)).flatten();

    let container_w = n as f32 * CELL + max_label + (n as f32 - 1.0) * DIVIDER;
    let avail = container_w - (n as f32 - 1.0) * DIVIDER;
    let rest_item_w = avail / n as f32;

    // ── Per-segment animated widths ─────────────────────────────────────────
    // Animate each toward its target, then normalise so the row always fills
    // exactly `avail` (no mid-transition gap or overflow).
    let ctx = ui.ctx().clone();
    let mut anim: Vec<f32> = (0..n)
        .map(|i| {
            let target = if hovered == Some(i) {
                CELL + max_label
            } else if hovered.is_some() {
                CELL
            } else {
                rest_item_w
            };
            ctx.animate_value_with_time(id.with(("seg", i)), target, DUR)
        })
        .collect();
    let sum: f32 = anim.iter().sum::<f32>().max(1.0);
    let scale = avail / sum;
    for w in &mut anim {
        *w *= scale;
    }

    // At rest icons are centred in their equal share; on any hover they slide
    // flush-left (offset 0). Animate the shared offset for a smooth slide.
    let icon_off_target = if hovered.is_some() {
        0.0
    } else {
        (rest_item_w - CELL) * 0.5
    };
    let icon_off = ctx.animate_value_with_time(id.with("iconoff"), icon_off_target, DUR);
    let divider_alpha = ctx.animate_value_with_time(
        id.with("divider_alpha"),
        if hovered.is_some() { 0.0 } else { 1.0 },
        DUR,
    );

    // ── Allocate + paint ────────────────────────────────────────────────────
    let (rect, _) = ui.allocate_exact_size(Vec2::new(container_w, HEIGHT), Sense::hover());
    let rounding = egui::Rounding::same(HEIGHT * 0.5);
    let painter = ui.painter();
    painter.rect_filled(rect, rounding, pill_bg);
    painter.rect_stroke(
        rect,
        rounding,
        Stroke::new(1.0, visuals.widgets.inactive.bg_stroke.color),
    );

    let mut clicked = None;
    let mut new_hovered = None;
    let mut x = rect.left();

    for (i, item) in items.iter().enumerate() {
        let w = anim[i];
        let seg = Rect::from_min_size(egui::pos2(x, rect.top()), Vec2::new(w, HEIGHT));
        let sense = if item.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let resp = ui.interact(seg, id.with(("hit", i)), sense);
        if resp.hovered() && item.enabled {
            new_hovered = Some(i);
        }
        if resp.clicked() {
            clicked = Some(i);
        }

        let seg_painter = painter.with_clip_rect(seg);

        // Hover highlight — fills the whole segment, rounding only the outer
        // corners of the end segments so it hugs the pill's shape.
        if resp.hovered() && item.enabled {
            let r = HEIGHT * 0.5;
            let hi_rounding = egui::Rounding {
                nw: if i == 0 { r } else { 0.0 },
                sw: if i == 0 { r } else { 0.0 },
                ne: if i == n - 1 { r } else { 0.0 },
                se: if i == n - 1 { r } else { 0.0 },
            };
            seg_painter.rect_filled(seg, hi_rounding, accent.gamma_multiply(0.22));
        }

        // Icon.
        let icon_col = if item.enabled {
            if hovered == Some(i) {
                accent
            } else {
                text_col
            }
        } else {
            text_col.gamma_multiply(0.4)
        };
        let icon_center = egui::pos2(seg.left() + icon_off + CELL * 0.5, seg.center().y);
        seg_painter.text(
            icon_center,
            Align2::CENTER_CENTER,
            item.icon,
            icon_font.clone(),
            icon_col,
        );

        // Label — fades in only for the active item.
        let label_alpha =
            ctx.animate_value_with_time(id.with(("lbl", i)), (hovered == Some(i)) as u8 as f32, DUR);
        if label_alpha > 0.01 {
            seg_painter.text(
                egui::pos2(seg.left() + CELL, seg.center().y),
                Align2::LEFT_CENTER,
                item.label,
                label_font.clone(),
                text_col.gamma_multiply(label_alpha),
            );
        }

        if item.enabled {
            resp.on_hover_text(item.hover);
        }

        x += w;

        // Divider between items.
        if i < n - 1 {
            if divider_alpha > 0.01 {
                painter.vline(
                    x + DIVIDER * 0.5,
                    (rect.top() + 5.0)..=(rect.bottom() - 5.0),
                    Stroke::new(1.0, divider_col.gamma_multiply(divider_alpha)),
                );
            }
            x += DIVIDER;
        }
    }

    ui.data_mut(|d| d.insert_temp::<Option<usize>>(id, new_hovered));
    clicked.filter(|&i| items[i].enabled)
}
