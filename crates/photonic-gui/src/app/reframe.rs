//! On-canvas reframe transform handles for the video-mode program monitor
//! (04 §3.3, 05 §4.2, CAP-012). Pure hit-test/mapping math lives here and is
//! unit-tested; [`draw_reframe_handles`] (impure — needs `ui`/`doc`/
//! `history`) is called once per frame from
//! `app/monitor.rs::draw_video_monitor`, right beside its
//! `draw_safe_area_guides` sibling call — same "overlay drawn directly on the
//! canvas rect" family (04 §3.3: "reuses the existing vector-canvas
//! selection-transform-handle widget verbatim, parametrized to edit
//! `ClipTransform`").
//!
//! **Semantics note (a documented seam this module resolves):**
//! `photonic_video::graph::compile::clip_transform_matrix`'s own comment
//! reads "exact reframe/anchor pixel semantics finalize with the UI story" —
//! this module is that story pinning the convention down: `x`/`y` are
//! format-pixel translation offsets from center, `scale_x`/`scale_y` are
//! multipliers of a nominal box, `rotation` is radians (matching the
//! engine's `glam::Mat3::from_angle`, which takes radians).
//!
//! v1 scope: single-clip selection only (04/01 are silent on multi-select
//! reframe editing, the same open question 13 §16 finding #6 flags for the
//! Clip Inspector) and a fixed nominal handle-box size rather than the
//! clip's true decoded content bounds (thumbnail/content-bounds access is a
//! documented seam elsewhere, `app/engine.rs`'s "no per-clip decoded-frame
//! access yet") — position/scale/rotation are all real, undoable edits
//! either way; only the handle box's *resting* size is a simplification.

use egui::{Pos2, Rect, Vec2};
use photonic_core::document::Document;
use photonic_core::history::{Command, CommandHistory};
use photonic_core::timeline::{
    ops, Clip, ClipId, ClipTransform, SequenceFormat, TrackId,
};

const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x6E, 0x56, 0xCF); // `primary`
const HANDLE_RADIUS: f32 = 5.0;
/// The nominal reframe box, as a fraction of the format's width/height, at
/// `scale_x == scale_y == 1.0` (v1 simplification — see module doc).
const NOMINAL_FRACTION: f32 = 0.4;
const ROTATE_HANDLE_GAP: f32 = 24.0;

// ── Pure mapping/hit-test math (unit-tested) ────────────────────────────────

/// Screen-pixels-per-format-pixel scale factor for `video_rect` (the format
/// spans it edge-to-edge — `app/monitor.rs::draw_video_monitor`'s own
/// letterbox/pillarbox fit math).
pub(crate) fn format_to_screen_scale(video_rect: Rect, format_w: u32, format_h: u32) -> f32 {
    let fw = format_w.max(1) as f32;
    let fh = format_h.max(1) as f32;
    (video_rect.width() / fw).min(video_rect.height() / fh)
}

/// Screen-space center of a clip's current reframe transform within
/// `video_rect`.
pub(crate) fn handle_center(video_rect: Rect, t: &ClipTransform, scale: f32) -> Pos2 {
    video_rect.center() + Vec2::new(t.x as f32, t.y as f32) * scale
}

/// A screen-space pointer delta, converted to a format-pixel delta (the
/// inverse of the scale mapping above).
pub(crate) fn screen_delta_to_format(delta: Vec2, scale: f32) -> (f64, f64) {
    let s = if scale.abs() > f32::EPSILON { scale } else { 1.0 };
    ((delta.x / s) as f64, (delta.y / s) as f64)
}

/// The four corners of the rotated reframe box, in screen space.
pub(crate) fn box_corners(center: Pos2, half_size: Vec2, rotation_rad: f64) -> [Pos2; 4] {
    let (s, c) = (rotation_rad.sin() as f32, rotation_rad.cos() as f32);
    let local = [
        Vec2::new(-half_size.x, -half_size.y),
        Vec2::new(half_size.x, -half_size.y),
        Vec2::new(half_size.x, half_size.y),
        Vec2::new(-half_size.x, half_size.y),
    ];
    local.map(|p| center + Vec2::new(p.x * c - p.y * s, p.x * s + p.y * c))
}

/// The rotate handle's screen position: `gap` past the box's top edge,
/// rotated with the box.
pub(crate) fn rotate_handle_pos(center: Pos2, half_h: f32, rotation_rad: f64, gap: f32) -> Pos2 {
    let (s, c) = (rotation_rad.sin() as f32, rotation_rad.cos() as f32);
    let local = Vec2::new(0.0, -(half_h + gap));
    center + Vec2::new(local.x * c - local.y * s, local.x * s + local.y * c)
}

/// New uniform scale from a corner-handle drag: `base_scale` grows/shrinks by
/// the ratio of the new to old distance from `center`.
pub(crate) fn scale_from_corner_drag(
    center: Pos2,
    handle_pos: Pos2,
    pointer_pos: Pos2,
    base_scale: f64,
) -> f64 {
    let base_dist = (handle_pos - center).length().max(1.0);
    let new_dist = (pointer_pos - center).length().max(1.0);
    (base_scale * (new_dist / base_dist) as f64).max(0.01)
}

/// New rotation (radians) from a rotate-handle drag: the angle of
/// `pointer_pos` around `center`, offset so straight-up is zero rotation.
pub(crate) fn rotation_from_handle_drag(center: Pos2, pointer_pos: Pos2) -> f64 {
    let v = pointer_pos - center;
    v.y.atan2(v.x) as f64 + std::f64::consts::FRAC_PI_2
}

fn nominal_half_size(format: &SequenceFormat, scale: f32, t: &ClipTransform) -> Vec2 {
    Vec2::new(
        format.width as f32 * NOMINAL_FRACTION * scale * t.scale_x.max(0.0) as f32 * 0.5,
        format.height as f32 * NOMINAL_FRACTION * scale * t.scale_y.max(0.0) as f32 * 0.5,
    )
}

// ── Impure: draw + drive (04 §3.3, CAP-012) ─────────────────────────────────

/// Locate `clip_id` on any track of `seq_id`'s sequence, returning its
/// owning track and a clone (the caller needs an owned `Clip` to mutate and
/// re-submit via `ops::set_clip_prop` anyway).
fn locate(
    project: &photonic_core::timeline::TimelineProject,
    seq_id: photonic_core::timeline::SequenceId,
    clip_id: ClipId,
) -> Option<(TrackId, Clip)> {
    let seq = project.sequences.get(&seq_id)?;
    for track in seq.tracks() {
        if let Some(c) = track.clips.iter().find(|c| c.id == clip_id) {
            return Some((track.id, c.clone()));
        }
    }
    None
}

fn commit_reframe(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq_id: photonic_core::timeline::SequenceId,
    track_id: TrackId,
    mut clip: Clip,
    active_format: usize,
    new_t: ClipTransform,
) {
    let Some(project) = doc.timeline.as_ref() else {
        return;
    };
    clip.reframe.insert(active_format, new_t);
    if let Ok(cmd) = ops::set_clip_prop(project, seq_id, track_id, clip) {
        // Coalesced: the global pointer-down/up `begin_coalescing`/
        // `end_coalescing` wrap already folds a streamed drag into one undo
        // step (same mechanism every timeline/vector drag uses).
        history.execute(Command::Timeline(cmd), doc);
    }
}

/// Draw + drive on-canvas reframe handles for the selected clip, if exactly
/// one clip is selected and the active sequence/format resolve. `video_rect`
/// is the letterboxed/pillarboxed rect the format is presented in (same rect
/// `draw_safe_area_guides` uses).
pub(crate) fn draw_reframe_handles(
    ui: &mut egui::Ui,
    video_rect: Rect,
    doc: &mut Document,
    history: &mut CommandHistory,
    selection: &[ClipId],
) {
    let [clip_id] = selection else {
        return; // none or multi-selected — 04/01 are silent on multi (13 §16 #6-adjacent)
    };
    let clip_id = *clip_id;
    let Some(project) = doc.timeline.as_ref() else {
        return;
    };
    let Some(seq_id) = project.active_sequence else {
        return;
    };
    let Some(seq) = project.sequences.get(&seq_id) else {
        return;
    };
    let active_format = seq.active_format;
    let format = seq.format().clone();
    let Some((track_id, clip)) = locate(project, seq_id, clip_id) else {
        return;
    };

    let scale = format_to_screen_scale(video_rect, format.width, format.height);
    let base = clip
        .reframe
        .get(&active_format)
        .copied()
        .unwrap_or(clip.transform.base);
    let center = handle_center(video_rect, &base, scale);
    let half = nominal_half_size(&format, scale, &base);
    let corners = box_corners(center, half, base.rotation);
    let scale_handle = corners[2]; // bottom-right
    let rotate_handle = rotate_handle_pos(center, half.y, base.rotation, ROTATE_HANDLE_GAP);

    let painter = ui.painter();
    for i in 0..4 {
        painter.line_segment([corners[i], corners[(i + 1) % 4]], egui::Stroke::new(1.0, ACCENT));
    }
    painter.line_segment(
        [Pos2::new((corners[0].x + corners[1].x) / 2.0, (corners[0].y + corners[1].y) / 2.0), rotate_handle],
        egui::Stroke::new(1.0, ACCENT),
    );
    painter.circle_filled(rotate_handle, HANDLE_RADIUS, ACCENT);
    painter.circle_filled(scale_handle, HANDLE_RADIUS, ACCENT);

    // Move: drag anywhere inside the box.
    let move_poly_rect = Rect::from_points(&corners);
    let move_id = ui.id().with(("reframe_move", clip_id));
    let move_resp = ui.interact(move_poly_rect, move_id, egui::Sense::drag());
    if move_resp.dragged() {
        let (dx, dy) = screen_delta_to_format(move_resp.drag_delta(), scale);
        let mut new_t = base;
        new_t.x += dx;
        new_t.y += dy;
        commit_reframe(doc, history, seq_id, track_id, clip.clone(), active_format, new_t);
    }

    // Scale: bottom-right corner handle.
    let scale_id = ui.id().with(("reframe_scale", clip_id));
    let scale_rect = Rect::from_center_size(scale_handle, Vec2::splat(HANDLE_RADIUS * 3.0));
    let scale_resp = ui.interact(scale_rect, scale_id, egui::Sense::drag());
    if scale_resp.dragged() {
        if let Some(pointer) = scale_resp.interact_pointer_pos() {
            let ratio = scale_from_corner_drag(center, scale_handle, pointer, 1.0);
            let mut new_t = base;
            new_t.scale_x = (base.scale_x * ratio).max(0.01);
            new_t.scale_y = (base.scale_y * ratio).max(0.01);
            commit_reframe(doc, history, seq_id, track_id, clip.clone(), active_format, new_t);
        }
    }

    // Rotate: handle above the box.
    let rotate_id = ui.id().with(("reframe_rotate", clip_id));
    let rotate_rect = Rect::from_center_size(rotate_handle, Vec2::splat(HANDLE_RADIUS * 3.0));
    let rotate_resp = ui.interact(rotate_rect, rotate_id, egui::Sense::drag());
    if rotate_resp.dragged() {
        if let Some(pointer) = rotate_resp.interact_pointer_pos() {
            let mut new_t = base;
            new_t.rotation = rotation_from_handle_drag(center, pointer);
            commit_reframe(doc, history, seq_id, track_id, clip, active_format, new_t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_to_screen_scale_picks_the_tighter_axis() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 500.0));
        // 16:9 format into a wider-than-16:9 rect → height-bound.
        let s = format_to_screen_scale(rect, 1920, 1080);
        assert!((s - 500.0 / 1080.0).abs() < 1e-6);
    }

    #[test]
    fn handle_center_offsets_by_scaled_xy() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(200.0, 100.0));
        let t = ClipTransform {
            x: 10.0,
            y: -5.0,
            ..ClipTransform::default()
        };
        let c = handle_center(rect, &t, 2.0);
        assert_eq!(c, rect.center() + Vec2::new(20.0, -10.0));
    }

    #[test]
    fn screen_delta_to_format_is_inverse_of_scale() {
        let (dx, dy) = screen_delta_to_format(Vec2::new(20.0, 40.0), 2.0);
        assert!((dx - 10.0).abs() < 1e-9);
        assert!((dy - 20.0).abs() < 1e-9);
    }

    #[test]
    fn screen_delta_to_format_never_divides_by_zero() {
        let (dx, dy) = screen_delta_to_format(Vec2::new(20.0, 40.0), 0.0);
        assert!((dx - 20.0).abs() < 1e-9);
        assert!((dy - 40.0).abs() < 1e-9);
    }

    #[test]
    fn box_corners_unrotated_is_axis_aligned_rect() {
        let center = Pos2::new(100.0, 100.0);
        let half = Vec2::new(50.0, 20.0);
        let c = box_corners(center, half, 0.0);
        assert!((c[0] - Pos2::new(50.0, 80.0)).length() < 1e-3);
        assert!((c[2] - Pos2::new(150.0, 120.0)).length() < 1e-3);
    }

    #[test]
    fn box_corners_quarter_turn_swaps_axes() {
        let center = Pos2::new(0.0, 0.0);
        let half = Vec2::new(50.0, 20.0);
        let c = box_corners(center, half, std::f64::consts::FRAC_PI_2);
        // A point that was at local (+half.x, -half.y) rotates to
        // (+half.y, +half.x) under a +90° rotation.
        assert!((c[1].x - 20.0).abs() < 1e-3);
        assert!((c[1].y - 50.0).abs() < 1e-3);
    }

    #[test]
    fn rotate_handle_pos_sits_above_box_when_unrotated() {
        let center = Pos2::new(0.0, 0.0);
        let p = rotate_handle_pos(center, 20.0, 0.0, 10.0);
        assert!((p.x).abs() < 1e-3);
        assert!((p.y - -30.0).abs() < 1e-3);
    }

    #[test]
    fn scale_from_corner_drag_doubling_distance_doubles_scale() {
        let center = Pos2::new(0.0, 0.0);
        let handle = Pos2::new(10.0, 0.0);
        let pointer = Pos2::new(20.0, 0.0);
        let s = scale_from_corner_drag(center, handle, pointer, 1.0);
        assert!((s - 2.0).abs() < 1e-6);
    }

    #[test]
    fn scale_from_corner_drag_never_goes_non_positive() {
        let center = Pos2::new(0.0, 0.0);
        let handle = Pos2::new(10.0, 0.0);
        let pointer = Pos2::new(0.0, 0.0); // dragged onto the center
        let s = scale_from_corner_drag(center, handle, pointer, 1.0);
        assert!(s > 0.0);
    }

    #[test]
    fn rotation_from_handle_drag_straight_up_is_zero() {
        let center = Pos2::new(0.0, 0.0);
        let pointer = Pos2::new(0.0, -50.0);
        let r = rotation_from_handle_drag(center, pointer);
        assert!(r.abs() < 1e-6);
    }

    #[test]
    fn rotation_from_handle_drag_right_is_quarter_turn() {
        let center = Pos2::new(0.0, 0.0);
        let pointer = Pos2::new(50.0, 0.0);
        let r = rotation_from_handle_drag(center, pointer);
        assert!((r - std::f64::consts::FRAC_PI_2).abs() < 1e-6);
    }
}
