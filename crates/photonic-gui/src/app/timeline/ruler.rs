//! Time ruler, playhead, and marker row (04 §2.1, 13 §1.1).
//!
//! The ruler shows zoom-adaptive tick labels (timecode), a click/drag-to-scrub
//! surface that moves the session playhead (frame-snapped), and marker diamonds
//! read from `Sequence.markers`. Markers are click-to-seek here; add/remove is a
//! P-later seam — the committed core has no marker `TimelineCmd`, so editing them
//! undoably needs a core op first (documented in the P2 report).

use super::layout::TimelineView;
use photonic_core::timeline::{FrameRate, Sequence, Tick, TICKS_PER_SECOND};

/// Format a tick as `HH:MM:SS:FF` timecode at `fr` (04 §3.2).
pub(crate) fn timecode(t: Tick, fr: FrameRate) -> String {
    let fps = ((fr.num as f64 / fr.den.max(1) as f64).round() as i64).max(1);
    let f = fr.frame_at(t).max(0);
    let frames = f % fps;
    let total_secs = f / fps;
    let s = total_secs % 60;
    let m = (total_secs / 60) % 60;
    let h = total_secs / 3600;
    format!("{h:02}:{m:02}:{s:02}:{frames:02}")
}

/// Choose a "nice" label interval (in ticks) so labels land roughly every
/// `target_px`. Steps through frame, then 1/2/5/10/15/30/60/120/300/600 s.
fn label_interval(view: &TimelineView, fr: FrameRate, target_px: f32) -> Tick {
    let tpf = fr.ticks_per_frame().0.max(1);
    let candidates_secs = [1, 2, 5, 10, 15, 30, 60, 120, 300, 600];
    // Start with a single frame.
    let mut best = tpf;
    if view.ticks_to_px(Tick(tpf)) >= target_px {
        return Tick(tpf);
    }
    for s in candidates_secs {
        let ticks = s * TICKS_PER_SECOND;
        best = ticks;
        if view.ticks_to_px(Tick(ticks)) >= target_px {
            break;
        }
    }
    Tick(best)
}

/// Draw the ruler into `ruler_rect` and handle scrub/seek, mutating `playhead`.
/// `lane_left` is the x of tick-space origin (the lane's left edge).
pub(crate) fn draw_ruler(
    ui: &mut egui::Ui,
    view: &TimelineView,
    ruler_rect: egui::Rect,
    lane_left: f32,
    seq: &Sequence,
    playhead: &mut Tick,
) {
    let painter = ui.painter_at(ruler_rect);
    let visuals = ui.visuals();
    let tick_col = visuals.weak_text_color();
    let text_col = visuals.text_color();
    let accent = visuals.selection.stroke.color;

    // Background + baseline.
    painter.rect_filled(ruler_rect, 0.0, visuals.faint_bg_color);
    painter.line_segment(
        [ruler_rect.left_bottom(), ruler_rect.right_bottom()],
        egui::Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color),
    );

    // Adaptive tick labels.
    let interval = label_interval(view, seq.frame_rate, 72.0);
    if interval.0 > 0 {
        let first_tick = view.scroll_ticks.0;
        let last_tick = view.x_to_tick(ruler_rect.width(), 0.0).0;
        // Align the first label to an interval boundary at/after the left edge.
        let mut t = (first_tick.div_euclid(interval.0)) * interval.0;
        while t <= last_tick {
            let x = view.tick_to_x(Tick(t), lane_left);
            if x >= ruler_rect.left() - 1.0 && x <= ruler_rect.right() + 1.0 {
                painter.line_segment(
                    [
                        egui::pos2(x, ruler_rect.bottom() - 6.0),
                        egui::pos2(x, ruler_rect.bottom()),
                    ],
                    egui::Stroke::new(1.0, tick_col),
                );
                painter.text(
                    egui::pos2(x + 3.0, ruler_rect.top() + 2.0),
                    egui::Align2::LEFT_TOP,
                    timecode(Tick(t), seq.frame_rate),
                    egui::FontId::monospace(10.0),
                    text_col,
                );
            }
            t += interval.0;
        }
    }

    // Marker diamonds (click-to-seek; add/remove is a core-op seam).
    for m in &seq.markers {
        let x = view.tick_to_x(m.at, lane_left);
        if x < ruler_rect.left() || x > ruler_rect.right() {
            continue;
        }
        let cy = ruler_rect.top() + 5.0;
        let r = 4.0;
        let col = m
            .color
            .map(|c| {
                egui::Color32::from_rgb(
                    (c.r * 255.0) as u8,
                    (c.g * 255.0) as u8,
                    (c.b * 255.0) as u8,
                )
            })
            .unwrap_or(accent);
        painter.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(x, cy - r),
                egui::pos2(x + r, cy),
                egui::pos2(x, cy + r),
                egui::pos2(x - r, cy),
            ],
            col,
            egui::Stroke::NONE,
        ));
    }

    // Scrub/seek: click or drag anywhere on the ruler moves the playhead,
    // frame-snapped. Session state only (04 §6) — never touches history.
    let resp = ui.interact(
        ruler_rect,
        ui.id().with("timeline_ruler"),
        egui::Sense::click_and_drag(),
    );
    if resp.clicked() || resp.dragged() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let raw = view.x_to_tick(pos.x, lane_left);
            let snapped = seq
                .frame_rate
                .snap(if raw.0 < 0 { Tick::ZERO } else { raw });
            *playhead = snapped;
        }
    }
}

/// Draw the full-height playhead line across the ruler + all lanes (13 §1.1).
pub(crate) fn draw_playhead_line(
    painter: &egui::Painter,
    view: &TimelineView,
    playhead: Tick,
    content_rect: egui::Rect,
    lane_left: f32,
    accent: egui::Color32,
) {
    let x = view.tick_to_x(playhead, lane_left);
    if x < lane_left - 1.0 || x > content_rect.right() + 1.0 {
        return;
    }
    painter.line_segment(
        [
            egui::pos2(x, content_rect.top()),
            egui::pos2(x, content_rect.bottom()),
        ],
        egui::Stroke::new(1.0, accent),
    );
    // Small top handle so the playhead is grabbable-looking in the ruler.
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(x - 4.0, content_rect.top()),
            egui::pos2(x + 4.0, content_rect.top()),
            egui::pos2(x, content_rect.top() + 6.0),
        ],
        accent,
        egui::Stroke::NONE,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timecode_formats_hms_frames_at_30fps() {
        let fr = FrameRate::FPS_30;
        assert_eq!(timecode(Tick::ZERO, fr), "00:00:00:00");
        // 1 second + 5 frames.
        let tpf = fr.ticks_per_frame().0;
        let t = Tick(TICKS_PER_SECOND + 5 * tpf);
        assert_eq!(timecode(t, fr), "00:00:01:05");
        // 1 hour exactly.
        assert_eq!(timecode(Tick::from_seconds(3600), fr), "01:00:00:00");
    }

    #[test]
    fn label_interval_grows_when_zoomed_out() {
        let mut v = TimelineView::default();
        let fr = FrameRate::FPS_30;
        let zoomed_in = label_interval(&v, fr, 72.0);
        // Zoom way out → interval must not shrink.
        v.pixels_per_tick /= 100.0;
        let zoomed_out = label_interval(&v, fr, 72.0);
        assert!(zoomed_out.0 >= zoomed_in.0);
    }
}
