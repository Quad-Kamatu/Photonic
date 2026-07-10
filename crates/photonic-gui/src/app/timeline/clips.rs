//! Clip-lane rendering (04 §2.2): visible-range culling + batched rect painting.
//!
//! No thumbnails or waveforms yet — the decode/waveform engine lands in P3
//! (02-engine.md, 09 §8.1). Each clip renders as a rounded rect with a name
//! label; a `// P3` seam marks where the thumbnail strip / waveform envelope
//! will slot in.

use super::layout::TimelineView;
use photonic_core::timeline::{Clip, ClipId, ClipSource, Tick, Track};

/// Palette for lane painting, resolved from the active egui theme by the caller
/// so this module stays theme-agnostic.
#[derive(Clone, Copy)]
pub(crate) struct LaneColors {
    pub clip_fill: egui::Color32,
    pub clip_stroke: egui::Color32,
    pub selected_fill: egui::Color32,
    pub selected_stroke: egui::Color32,
    pub label: egui::Color32,
    pub transition: egui::Color32,
    pub offline: egui::Color32,
    /// Diagonal-hatch overlay stroke for a locked track's lane (14-nle-parity
    /// QW-2) — a shape-only cue (13 §1.6), same convention as the offline
    /// per-clip stripe, so lock state reads without relying on color alone.
    pub locked_hatch: egui::Color32,
}

/// A painted clip's screen rect, returned for hit-testing in `interact.rs`.
pub(crate) struct PaintedClip {
    pub clip: ClipId,
    pub rect: egui::Rect,
}

/// Fixed swatch palette for `Clip::color_label` (14-nle-parity §M-1, gap #7's
/// UI half) — `(display name, color)` indexed by the label's `u8`. This is
/// deliberately a *separate* set of hues from the `DESIGN.md` theme tokens,
/// not derived from the active egui visuals: the label is organizational
/// data the user assigns, not chrome, the same "functional data-coding, not
/// chrome accent" precedent `DESIGN.md` documents for the node-editor port
/// sockets. `primary`'s violet is deliberately excluded so a label swatch
/// never reads as the selection cue.
pub(crate) const CLIP_LABEL_SWATCHES: &[(&str, egui::Color32)] = &[
    ("Red", egui::Color32::from_rgb(0xE5, 0x48, 0x4D)),
    ("Orange", egui::Color32::from_rgb(0xF5, 0xA5, 0x24)),
    ("Yellow", egui::Color32::from_rgb(0xE8, 0xD7, 0x4A)),
    ("Green", egui::Color32::from_rgb(0x4C, 0xC3, 0x8A)),
    ("Cyan", egui::Color32::from_rgb(0x14, 0xB8, 0xA6)),
    ("Blue", egui::Color32::from_rgb(0x3B, 0x82, 0xF6)),
    ("Pink", egui::Color32::from_rgb(0xEC, 0x48, 0x99)),
    ("Gray", egui::Color32::from_rgb(0x8B, 0x8F, 0xA3)),
];

/// Resolve a `color_label` index to its swatch color; tolerant of an
/// out-of-range index (e.g. a doc saved against a larger future palette)
/// rather than panicking on it.
pub(crate) fn label_color(label: u8) -> Option<egui::Color32> {
    CLIP_LABEL_SWATCHES.get(label as usize).map(|(_, c)| *c)
}

/// The visible slice of a track's clips for `[first, last]` lane ticks, via the
/// binary-search cull 04 §2.2 mandates (the `Vec<Clip>` is sorted + non-overlapping).
pub(crate) fn visible_clips(track: &Track, first: Tick, last: Tick) -> &[Clip] {
    let start_idx = track
        .clips
        .partition_point(|c| c.start + c.duration < first);
    let end_idx = track.clips.partition_point(|c| c.start <= last);
    let end_idx = end_idx.max(start_idx);
    &track.clips[start_idx..end_idx]
}

/// Whether a clip's source media is unreachable (offline). The media pool /
/// asset probing lands with import (05); until then no clip is offline.
fn is_offline(_clip: &Clip) -> bool {
    // P3/05 seam: resolve `ClipSource::Asset`/`Vector` against the media pool
    // and return true when the file is missing.
    false
}

/// Paint one track's visible clips into `lane_rect`, batching all rects through a
/// single `painter.extend` (04 §7a-b) and returning their screen rects for
/// hit-testing. Labels are drawn per-clip after the batch (bounded by the cull).
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_lane(
    painter: &egui::Painter,
    view: &TimelineView,
    track: &Track,
    lane_rect: egui::Rect,
    selection: &[ClipId],
    colors: &LaneColors,
) -> Vec<PaintedClip> {
    let first = view.scroll_ticks;
    let last = view.x_to_tick(lane_rect.width(), 0.0);
    let clips = visible_clips(track, first, last);

    let rounding = egui::Rounding::same(3.0); // `sm` per DESIGN.md / 13 §1.5
    let mut shapes: Vec<egui::Shape> = Vec::with_capacity(clips.len() * 2);
    let mut painted = Vec::with_capacity(clips.len());
    let top = lane_rect.top() + 2.0;
    let bottom = lane_rect.bottom() - 2.0;

    for c in clips {
        let x0 = view.tick_to_x(c.start, lane_rect.left());
        let x1 = view.tick_to_x(c.end(), lane_rect.left());
        // Clamp to the lane so a partly-scrolled clip still paints its visible part.
        let rx0 = x0.max(lane_rect.left());
        let rx1 = x1.min(lane_rect.right());
        if rx1 <= rx0 {
            continue;
        }
        let rect = egui::Rect::from_min_max(egui::pos2(rx0, top), egui::pos2(rx1, bottom));
        let selected = selection.contains(&c.id);

        let mut fill = if selected {
            colors.selected_fill
        } else {
            colors.clip_fill
        };
        if !c.enabled {
            // Disabled clips render at ~50% opacity (13 §1.2).
            fill = fill.gamma_multiply(0.5);
        }
        shapes.push(egui::Shape::rect_filled(rect, rounding, fill));

        if is_offline(c) {
            // Offline: diagonal-stripe placeholder (01 §3, shape-only per 13 §1.6).
            let step = 8.0;
            let mut x = rect.left();
            while x < rect.right() {
                shapes.push(egui::Shape::line_segment(
                    [
                        egui::pos2(x, rect.bottom()),
                        egui::pos2((x + rect.height()).min(rect.right()), rect.top()),
                    ],
                    egui::Stroke::new(1.0, colors.offline),
                ));
                x += step;
            }
        }

        let stroke = if selected {
            egui::Stroke::new(1.5, colors.selected_stroke)
        } else {
            egui::Stroke::new(1.0, colors.clip_stroke)
        };
        shapes.push(egui::Shape::rect_stroke(rect, rounding, stroke));

        // Organizational color label (14-nle-parity §M-1): a thin cap along
        // the clip's top edge, rounded to match the clip's own `sm` corners.
        // Additive over the selected/disabled fill above — it never replaces
        // those states, only decorates them (13 §1's "shape+color, not
        // color-only" spirit extended to a genuinely-colored affordance).
        if let Some(label) = c.color_label {
            if let Some(mut swatch) = label_color(label) {
                if !c.enabled {
                    swatch = swatch.gamma_multiply(0.5);
                }
                let stripe_h = 3.0_f32.min(rect.height());
                let stripe_rect = egui::Rect::from_min_max(
                    rect.left_top(),
                    egui::pos2(rect.right(), rect.top() + stripe_h),
                );
                let stripe_rounding = egui::Rounding {
                    nw: rounding.nw,
                    ne: rounding.ne,
                    sw: 0.0,
                    se: 0.0,
                };
                shapes.push(egui::Shape::rect_filled(
                    stripe_rect,
                    stripe_rounding,
                    swatch,
                ));
            }
        }

        // Transition badges: small triangles at the edge(s) with a transition
        // (drawn shape-only so they read without color, 13 §1.6).
        if c.transition_in.is_some() {
            shapes.push(edge_triangle(rect, true, colors.transition));
        }
        if c.transition_out.is_some() {
            shapes.push(edge_triangle(rect, false, colors.transition));
        }

        painted.push(PaintedClip { clip: c.id, rect });
    }

    // One batched draw call for the whole lane (04 §7b).
    painter.extend(shapes);

    // P3 seam: thumbnail strip (video/image) + waveform envelope (audio) render
    // here, sampled from the engine's decode ring / waveform pyramid.

    // Labels, clipped to each rect (bounded by the cull, so per-clip is fine).
    for pc in &painted {
        if pc.rect.width() < 18.0 {
            continue;
        }
        if let Some(name) = clip_label(track, pc.clip) {
            let text_pos = egui::pos2(pc.rect.left() + 4.0, pc.rect.top() + 2.0);
            painter.text(
                text_pos,
                egui::Align2::LEFT_TOP,
                elide(&name, pc.rect.width()),
                egui::FontId::proportional(11.0),
                colors.label,
            );
        }
    }

    // Locked-track cue (14-nle-parity QW-2): a diagonal-hatch overlay across
    // the whole lane, drawn last so it reads over clips/labels and empty
    // space alike — lock state is visible independent of what's under it.
    // Interaction is gated separately (`interact.rs::hit_at`); this is
    // paint-only, clips stay fully visible underneath.
    if track.locked {
        paint_locked_hatch(painter, lane_rect, colors.locked_hatch);
    }

    painted
}

/// A small right-angle triangle hugging a clip's left (`inbound`) or right edge.
fn edge_triangle(rect: egui::Rect, inbound: bool, color: egui::Color32) -> egui::Shape {
    let s = 8.0_f32.min(rect.width() * 0.4);
    let pts = if inbound {
        vec![
            rect.left_top(),
            egui::pos2(rect.left() + s, rect.top()),
            egui::pos2(rect.left(), rect.top() + s),
        ]
    } else {
        vec![
            rect.right_top(),
            egui::pos2(rect.right() - s, rect.top()),
            egui::pos2(rect.right(), rect.top() + s),
        ]
    };
    egui::Shape::convex_polygon(pts, color, egui::Stroke::NONE)
}

/// Diagonal-hatch overlay for a locked track's whole lane (14-nle-parity
/// QW-2). Draws full-height diagonal strokes clipped to `rect` — same stripe
/// geometry as the per-clip offline placeholder, just spanning the lane
/// instead of one clip.
fn paint_locked_hatch(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32) {
    let step = 10.0;
    let clipped = painter.with_clip_rect(rect);
    let mut shapes = Vec::new();
    let mut x = rect.left() - rect.height();
    while x < rect.right() {
        shapes.push(egui::Shape::line_segment(
            [
                egui::pos2(x, rect.bottom()),
                egui::pos2(x + rect.height(), rect.top()),
            ],
            egui::Stroke::new(1.0, color),
        ));
        x += step;
    }
    clipped.extend(shapes);
}

fn clip_label(track: &Track, id: ClipId) -> Option<String> {
    let c = track.clips.iter().find(|c| c.id == id)?;
    if !c.name.is_empty() {
        return Some(c.name.clone());
    }
    Some(
        match &c.source {
            ClipSource::Asset { .. } => "Clip",
            ClipSource::Vector { .. } => "Vector",
            ClipSource::NestedSequence { .. } => "Sequence",
            ClipSource::SolidColor { .. } => "Solid",
            ClipSource::Adjustment => "Adjustment",
        }
        .to_string(),
    )
}

/// Roughly elide a label to fit `width_px` (≈6px/char at 11pt proportional).
fn elide(s: &str, width_px: f32) -> String {
    let max_chars = ((width_px - 8.0) / 6.0).floor().max(1.0) as usize;
    if s.chars().count() <= max_chars {
        s.to_string()
    } else if max_chars <= 1 {
        "…".to_string()
    } else {
        let keep: String = s.chars().take(max_chars - 1).collect();
        format!("{keep}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photonic_core::timeline::{FrameRate, Sequence, TrackKind};

    fn track_with(spans: &[(i64, i64)]) -> Track {
        let mut t = Track::new(TrackKind::Video, "V1");
        for (start, dur) in spans {
            t.clips
                .push(Clip::new(ClipSource::Adjustment, Tick(*start), Tick(*dur)));
        }
        t
    }

    #[test]
    fn cull_returns_only_overlapping_clips() {
        // Clips at [0,100), [100,150), [200,210), [1000,1100).
        let t = track_with(&[(0, 100), (100, 50), (200, 10), (1000, 100)]);
        // Window [120, 205] should include the [100,150) and [200,210) clips only.
        let vis = visible_clips(&t, Tick(120), Tick(205));
        assert_eq!(vis.len(), 2);
        assert_eq!(vis[0].start, Tick(100));
        assert_eq!(vis[1].start, Tick(200));
    }

    #[test]
    fn cull_empty_window_before_first_clip() {
        let t = track_with(&[(500, 100), (700, 100)]);
        let vis = visible_clips(&t, Tick(0), Tick(100));
        assert!(vis.is_empty());
    }

    #[test]
    fn cull_window_after_last_clip() {
        let t = track_with(&[(0, 100), (100, 100)]);
        let vis = visible_clips(&t, Tick(500), Tick(600));
        assert!(vis.is_empty());
    }

    #[test]
    fn cull_full_range_returns_all() {
        let t = track_with(&[(0, 100), (100, 100), (200, 100)]);
        let vis = visible_clips(&t, Tick(0), Tick(1000));
        assert_eq!(vis.len(), 3);
    }

    #[test]
    fn label_falls_back_to_source_kind() {
        let mut s = Sequence::new("s", FrameRate::FPS_30, 1920, 1080);
        let t = track_with(&[(0, 100)]);
        let id = t.clips[0].id;
        s.video_tracks.push(t);
        assert_eq!(
            clip_label(&s.video_tracks[0], id).as_deref(),
            Some("Adjustment")
        );
    }

    #[test]
    fn color_label_resolves_every_swatch_in_range() {
        for i in 0..CLIP_LABEL_SWATCHES.len() {
            assert_eq!(label_color(i as u8), Some(CLIP_LABEL_SWATCHES[i].1));
        }
    }

    #[test]
    fn color_label_out_of_range_is_none() {
        assert_eq!(label_color(CLIP_LABEL_SWATCHES.len() as u8), None);
        assert_eq!(label_color(u8::MAX), None);
    }

    #[test]
    fn color_label_palette_excludes_the_selection_accent_violet() {
        // DESIGN.md reserves `primary` (#6E56CF) for the selection cue; a
        // label swatch must not collide with it (see the module doc comment
        // on `CLIP_LABEL_SWATCHES`).
        let primary = egui::Color32::from_rgb(0x6E, 0x56, 0xCF);
        assert!(!CLIP_LABEL_SWATCHES.iter().any(|(_, c)| *c == primary));
    }
}
