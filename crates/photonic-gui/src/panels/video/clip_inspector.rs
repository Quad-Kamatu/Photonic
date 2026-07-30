//! `DrawerGroup::ClipInspector` panel (04 §4.1) — the selected clip's
//! transform / speed / effects-stack / keyframes / transition params, the
//! `Clip`/`ClipEffect` analogue of the vector Inspector. Panel shell owned by
//! 04; widgets source from `prop_registry` (01 §6.2).
//!
//! The "Keyframes" section ([`draw_keyframes_section`]) docks
//! [`super::keyframe_editor::draw_embedded`] inline — the single
//! Effect-Controls surface (14 §G-9) — instead of requiring the separate
//! floating `egui::Window` (`keyframe_editor.rs::draw_window`). That float
//! remains available (pop-out button in the section, or a per-field
//! keyframe-diamond click) for live-playhead scrubbing / the wider
//! side-by-side layout; both write [`super::VideoPanelUi::keyframe_editor_target`].
//!
//! Every mutation here is built as a real `photonic_core::timeline::ops::*`
//! call (reading `ctx.doc`, never mutating it) and handed to the app via
//! [`crate::panels::PanelAction::ClipEditDiscrete`]/`ClipEditCoalesced` —
//! `PropPanelCtx` carries `doc: &Document` but no `&mut CommandHistory` (same
//! reason the media-pool panel routes through `PanelAction::Media*`), so
//! those two generic carriers are this panel's `ops_bridge` equivalent at the
//! drawer boundary. No direct `doc.timeline` mutation anywhere below —
//! grep-checked (13 §normative rule, 01 §10 CAP-019 parity).

use crate::panels::{PanelAction, PropPanelCtx};
use egui::{Color32, RichText, Ui};
use egui_phosphor::regular as ph;
// `SpeedKey` isn't re-exported at `timeline::` root (only `SpeedMap` is);
// reached via the `clip` submodule directly, same precedent as
// `app/timeline/mod.rs`'s `use photonic_core::timeline::clip::LinkGroupId`.
use photonic_core::timeline::clip::SpeedKey;
// `VfxOwner` (K-B1/K-B2 effect-stack scope) isn't re-exported at `timeline::`
// root either — same precedent as `SpeedKey` above.
use super::param_expr;
use photonic_core::timeline::commands::VfxOwner;
use photonic_core::timeline::{
    ops, prop_registry, Clip, ClipId, EffectKind, EffectParams, PropTargetKind, PropValue, Ratio,
    SequenceId, SpeedMap, Tick, TimelineProject, TrackId, TrackKind, Transition, TransitionKind,
    TransitionParams, TICKS_PER_SECOND,
};

const MUTED: Color32 = Color32::from_rgb(0x7A, 0x7A, 0x9A); // `secondary`
const ACCENT: Color32 = Color32::from_rgb(0x6E, 0x56, 0xCF); // `primary`

/// Left-rail Clip Inspector drawer. Reads the timeline selection and clip data
/// via `ctx` (`ctx.video.selection`, `ctx.doc`).
pub(crate) fn draw_clip_inspector(ui: &mut Ui, ctx: &mut PropPanelCtx) {
    let Some(project) = ctx.doc.timeline.as_ref() else {
        ui.label(RichText::new("No video project yet.").color(MUTED));
        return;
    };
    let selection = ctx.video.selection;
    if selection.is_empty() {
        ui.label(RichText::new("No clip selected.").color(MUTED));
        return;
    }
    if selection.len() > 1 {
        // 13 §16 finding #6: multi-clip common-value editing is unresolved by
        // any spec doc; showing the (single-clip) inspector for the first
        // selected clip is the documented interim behavior, not silent data
        // loss — the other clips are simply not editable from here yet.
        ui.label(
            RichText::new(format!(
                "{} clips selected — showing the first (multi-clip editing not yet supported).",
                selection.len()
            ))
            .color(MUTED)
            .small(),
        );
    }
    let clip_id = selection[0];
    let Some((seq_id, track_id, clip)) = locate_clip(project, clip_id) else {
        ui.label(RichText::new("Selected clip no longer exists.").color(MUTED));
        return;
    };

    let mut action: Option<PanelAction> = None;
    let active_format = project
        .sequences
        .get(&seq_id)
        .map(|s| s.active_format)
        .unwrap_or(0);

    ui.label(RichText::new(&clip.name).strong());
    ui.add_space(2.0);

    // K-A6: numeric Edit Duration form (position / in / out / duration + ripple).
    if ui
        .button(format!("{} Edit duration…", ph::TIMER))
        .on_hover_text("Frame-accurate position, source in/out, and duration (K-A6)")
        .clicked()
    {
        action = Some(PanelAction::OpenEditDuration {
            seq: seq_id,
            track: track_id,
            clip: clip_id,
        });
    }
    // K-B14: freeze at the clip's current source_in (inspector has no playhead;
    // the command / context menu freeze at the playhead).
    if ui
        .button(format!("{} Freeze frame", ph::PAUSE))
        .on_hover_text("Hold the current source frame for this clip's duration (zero speed; K-B14)")
        .clicked()
    {
        action = Some(PanelAction::FreezeFrame {
            seq: seq_id,
            track: track_id,
            clip: clip_id,
            at: Tick::ZERO, // relative to current source_in via existing speed
        });
    }
    ui.add_space(4.0);

    draw_transform_section(ui, ctx, project, seq_id, track_id, clip, &mut action);
    draw_speed_section(ui, project, seq_id, track_id, clip, &mut action);
    draw_reframe_section(
        ui,
        project,
        seq_id,
        track_id,
        clip,
        active_format,
        &mut action,
    );
    draw_level_horizon_section(
        ui,
        project,
        seq_id,
        track_id,
        clip,
        active_format,
        &mut action,
    );
    draw_effects_section(ui, project, seq_id, track_id, clip, &mut action);
    draw_keyframes_section(ui, ctx, project, clip, &mut action);
    draw_transitions_section(ui, project, seq_id, track_id, clip, &mut action);

    if action.is_some() {
        ctx.action = action;
    }
}

// ── Clip location (session `ClipId` → (seq, track, &Clip)) ─────────────────

/// `ctx.video.selection` (04 §2.6 session state) carries only `ClipId`s, so
/// the inspector resolves the owning sequence/track itself — no other panel
/// exposes this lookup.
fn locate_clip(project: &TimelineProject, clip_id: ClipId) -> Option<(SequenceId, TrackId, &Clip)> {
    for (seq_id, seq) in &project.sequences {
        for track in seq.tracks() {
            if let Some(clip) = track.clips.iter().find(|c| c.id == clip_id) {
                return Some((*seq_id, track.id, clip));
            }
        }
    }
    None
}

/// Push `new_clip` as `SetClipProp`, coalesced (drag-scrub numeric fields).
fn set_clip_coalesced(
    project: &TimelineProject,
    seq: SequenceId,
    track: TrackId,
    new_clip: Clip,
    action: &mut Option<PanelAction>,
) {
    if let Ok(cmd) = ops::set_clip_prop(project, seq, track, new_clip) {
        *action = Some(PanelAction::ClipEditCoalesced(cmd));
    }
}

/// Push `new_clip` as `SetClipProp`, one discrete undo step (toggle/button).
fn set_clip_discrete(
    project: &TimelineProject,
    seq: SequenceId,
    track: TrackId,
    new_clip: Clip,
    action: &mut Option<PanelAction>,
) {
    if let Ok(cmd) = ops::set_clip_prop(project, seq, track, new_clip) {
        *action = Some(PanelAction::ClipEditDiscrete(cmd));
    }
}

/// A keyframe-diamond toggle beside an animatable field (13 §5.1): clicking it
/// targets the keyframe editor at this clip (`VideoPanelUi::keyframe_editor_target`,
/// tagged `[clip_inspector]`) rather than adding/removing a keyframe inline —
/// per-field add/remove-at-playhead is the keyframe editor's own job once that
/// surface exists; this panel wires the field for its documented purpose
/// (targeting) without inventing a second keyframing UI.
fn keyframe_diamond(ui: &mut Ui, ctx: &mut PropPanelCtx, clip_id: ClipId, has_track: bool) {
    let glyph = if has_track { "\u{25C6}" } else { "\u{25C7}" }; // filled / hollow diamond
    let color = if has_track { ACCENT } else { MUTED };
    if ui
        .add(egui::Button::new(RichText::new(glyph).color(color)).small())
        .on_hover_text("Animate this property (opens the keyframe editor)")
        .clicked()
    {
        *ctx.video.keyframe_editor_target = Some(clip_id);
    }
}

// ── Transform (13 §5.1) ──────────────────────────────────────────────────────

fn draw_transform_section(
    ui: &mut Ui,
    ctx: &mut PropPanelCtx,
    project: &TimelineProject,
    seq: SequenceId,
    track: TrackId,
    clip: &Clip,
    action: &mut Option<PanelAction>,
) {
    egui::CollapsingHeader::new("Transform")
        .default_open(true)
        .id_salt("clip_inspector_transform")
        .show(ui, |ui| {
            let base = clip.transform.base;
            let has_track = !clip.transform.tracks.is_empty();
            let mut new_t = base;
            let mut changed = false;
            egui::Grid::new("clip_transform_grid")
                .num_columns(3)
                .spacing([4.0, 2.0])
                .show(ui, |ui| {
                    changed |= transform_row(ui, "X", &mut new_t.x, None);
                    keyframe_diamond(ui, ctx, clip.id, has_track);
                    ui.end_row();
                    changed |= transform_row(ui, "Y", &mut new_t.y, None);
                    keyframe_diamond(ui, ctx, clip.id, has_track);
                    ui.end_row();
                    changed |= transform_row(ui, "Scale X", &mut new_t.scale_x, Some(0.0..=1000.0));
                    keyframe_diamond(ui, ctx, clip.id, has_track);
                    ui.end_row();
                    changed |= transform_row(ui, "Scale Y", &mut new_t.scale_y, Some(0.0..=1000.0));
                    keyframe_diamond(ui, ctx, clip.id, has_track);
                    ui.end_row();
                    changed |= transform_row_suffixed(ui, "Rotation", &mut new_t.rotation, " rad");
                    keyframe_diamond(ui, ctx, clip.id, has_track);
                    ui.end_row();
                    changed |= transform_row(ui, "Anchor X", &mut new_t.anchor_x, None);
                    keyframe_diamond(ui, ctx, clip.id, has_track);
                    ui.end_row();
                    changed |= transform_row(ui, "Anchor Y", &mut new_t.anchor_y, None);
                    keyframe_diamond(ui, ctx, clip.id, has_track);
                    ui.end_row();
                    changed |= transform_row(ui, "Opacity", &mut new_t.opacity, Some(0.0..=1.0));
                    keyframe_diamond(ui, ctx, clip.id, has_track);
                    ui.end_row();
                });
            if changed {
                let mut new_clip = clip.clone();
                new_clip.transform.base = new_t;
                set_clip_coalesced(project, seq, track, new_clip, action);
            }
        });
}

fn transform_row(
    ui: &mut Ui,
    label: &str,
    v: &mut f64,
    range: Option<std::ops::RangeInclusive<f64>>,
) -> bool {
    ui.label(label);
    // K-B6: arithmetic / %vars + middle-click reset. Transform siblings are not
    // threaded here — frame size alone is enough for `%w`/`%h` style maths.
    let range_tuple = range.as_ref().map(|r| (*r.start(), *r.end()));
    let default = param_expr::neutral_float_default(range_tuple);
    let vars = param_expr::vars_from_params([], None, None);
    param_expr::float_drag(ui, v, default, range_tuple, &vars, 0.5)
}

/// Same as [`transform_row`] with a unit suffix — used for `rotation`, which
/// `photonic_video::graph::compile::clip_transform_matrix` evaluates in
/// radians (`glam::Mat3::from_angle`), unlike every other transform field
/// (plain document-space units) — the suffix keeps that unit legible instead
/// of a bare unlabeled float (04 §3.3/`app/reframe.rs` pins the same
/// convention for the on-canvas rotate handle).
fn transform_row_suffixed(ui: &mut Ui, label: &str, v: &mut f64, suffix: &str) -> bool {
    ui.label(label);
    let drag = egui::DragValue::new(v)
        .speed(0.02)
        .fixed_decimals(3)
        .suffix(suffix);
    ui.add(drag).changed()
}

// ── Speed (13 §5.1, 01 §5.1, 14-nle-parity G-11) ─────────────────────────────

/// UI percent + reverse flag → an exact `Ratio` at 1/1000 precision — exact
/// enough for a UI-driven speed field (01 §5.1's "exact rational" rule
/// governs storage/eval, not that every possible value must be
/// keyboard-reachable at infinite precision). Shared by the constant-speed
/// field and every ramp-point row below.
fn ratio_from_pct(pct: f64, reversed: bool) -> Ratio {
    let den: u32 = 1000;
    let mag = ((pct / 100.0) * den as f64).round().max(1.0) as i32;
    Ratio::new(if reversed { -mag } else { mag }, den)
}

/// The piecewise-constant ramp speed in effect at clip-relative tick `t`,
/// mirroring `photonic_core::timeline::clip`'s private `integrate_ramp`
/// segment definition (the first key's ratio holds before it, the last key's
/// after it; an empty ramp is identity). `sorted` must already be sorted by
/// `at` (every caller here sorts once up front rather than re-sorting per
/// lookup). Used only to seed a newly-added point with the speed already in
/// effect there, so adding a point never itself changes existing playback.
fn ramp_ratio_at(sorted: &[SpeedKey], t: Tick) -> Ratio {
    sorted
        .iter()
        .rev()
        .find(|k| k.at <= t)
        .or_else(|| sorted.first())
        .map(|k| k.ratio)
        .unwrap_or(Ratio::ONE)
}

fn draw_speed_section(
    ui: &mut Ui,
    project: &TimelineProject,
    seq: SequenceId,
    track: TrackId,
    clip: &Clip,
    action: &mut Option<PanelAction>,
) {
    egui::CollapsingHeader::new("Speed")
        .default_open(false)
        .id_salt("clip_inspector_speed")
        .show(ui, |ui| {
            let mut is_ramped = matches!(clip.speed, SpeedMap::Keyframed { .. });
            let toggled = ui
                .checkbox(&mut is_ramped, "Speed ramp (variable speed)")
                .on_hover_text(
                    "Off: one constant speed for the whole clip. On: a ramp of \
                     speed points you add/edit below (G-11).",
                )
                .changed();
            if toggled {
                let mut new_clip = clip.clone();
                new_clip.speed = if is_ramped {
                    // Seed with a single key at tick 0 holding the current
                    // constant ratio for the whole clip (one key covers
                    // `(-inf, +inf)` per `integrate_ramp`), so flipping the
                    // toggle never itself changes existing playback.
                    let seed = match clip.speed {
                        SpeedMap::Constant(r) => r,
                        SpeedMap::Keyframed { .. } => Ratio::ONE,
                    };
                    SpeedMap::Keyframed {
                        keys: vec![SpeedKey::new(Tick::ZERO, seed)],
                    }
                } else {
                    let r = match &clip.speed {
                        SpeedMap::Keyframed { keys } => {
                            let mut sorted = keys.clone();
                            sorted.sort_by_key(|k| k.at.0);
                            ramp_ratio_at(&sorted, Tick::ZERO)
                        }
                        SpeedMap::Constant(r) => *r,
                    };
                    SpeedMap::Constant(r)
                };
                set_clip_discrete(project, seq, track, new_clip, action);
            }
            ui.add_space(2.0);
            match &clip.speed {
                SpeedMap::Constant(r) => {
                    draw_constant_speed_row(ui, *r, project, seq, track, clip, action);
                }
                SpeedMap::Keyframed { keys } => {
                    draw_speed_ramp_editor(ui, keys, project, seq, track, clip, action);
                }
            }
        });
}

fn draw_constant_speed_row(
    ui: &mut Ui,
    r: Ratio,
    project: &TimelineProject,
    seq: SequenceId,
    track: TrackId,
    clip: &Clip,
    action: &mut Option<PanelAction>,
) {
    let mut pct = r.as_f64().abs() * 100.0;
    let mut reversed = r.num < 0;
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Speed:");
        changed |= ui
            .add(
                egui::DragValue::new(&mut pct)
                    .speed(1.0)
                    .range(1.0..=10000.0)
                    .suffix("%"),
            )
            .changed();
        changed |= ui.checkbox(&mut reversed, "Reverse").changed();
    });
    if changed {
        let mut new_clip = clip.clone();
        new_clip.speed = SpeedMap::Constant(ratio_from_pct(pct, reversed));
        set_clip_coalesced(project, seq, track, new_clip, action);
    }
}

/// Speed-ramp editor (G-11): one row per keyframe (clip-relative position in
/// seconds, speed %, reverse, remove), plus "Add point"/"Clear ramp". Kept
/// deliberately simple per the story's scope — add/remove/drag-the-numeric-
/// field editing here; full on-clip rubber-band dragging in the timeline
/// lane is a later story (the badge in `app/timeline/clips.rs` covers the
/// on-clip cue for now).
fn draw_speed_ramp_editor(
    ui: &mut Ui,
    keys: &[SpeedKey],
    project: &TimelineProject,
    seq: SequenceId,
    track: TrackId,
    clip: &Clip,
    action: &mut Option<PanelAction>,
) {
    ui.label(
        RichText::new("Piecewise ramp — each point holds its speed until the next point.")
            .color(MUTED)
            .small(),
    );
    let mut sorted: Vec<SpeedKey> = keys.to_vec();
    sorted.sort_by_key(|k| k.at.0);
    let clip_secs = clip.duration.as_seconds_f64().max(0.01);

    // One mutation per frame at most: whichever row/button changed last wins,
    // same one-write-per-frame shape as every other section in this file.
    let mut new_keys: Option<Vec<SpeedKey>> = None;
    let mut discrete = false;

    if sorted.is_empty() {
        ui.label(
            RichText::new("No ramp points yet — add one to start shaping the ramp.")
                .color(MUTED)
                .small(),
        );
    } else {
        egui::Grid::new(("clip_speed_ramp_grid", clip.id))
            .num_columns(4)
            .spacing([4.0, 2.0])
            .show(ui, |ui| {
                ui.label(RichText::new("At").color(MUTED).small());
                ui.label(RichText::new("Speed").color(MUTED).small());
                ui.label(RichText::new("Rev").color(MUTED).small());
                ui.label("");
                ui.end_row();

                for (i, key) in sorted.iter().enumerate() {
                    let mut at_secs = key.at.as_seconds_f64();
                    let mut pct = key.ratio.as_f64().abs() * 100.0;
                    let mut reversed = key.ratio.num < 0;
                    let at_resp = ui.add(
                        egui::DragValue::new(&mut at_secs)
                            .speed(0.05)
                            .range(0.0..=clip_secs)
                            .suffix("s"),
                    );
                    let pct_resp = ui.add(
                        egui::DragValue::new(&mut pct)
                            .speed(1.0)
                            .range(1.0..=10000.0)
                            .suffix("%"),
                    );
                    let rev_resp = ui.checkbox(&mut reversed, "");
                    let remove = ui
                        .add(egui::Button::new(RichText::new(ph::X)).small())
                        .on_hover_text("Remove point");
                    if at_resp.changed() || pct_resp.changed() || rev_resp.changed() {
                        let mut edited = sorted.clone();
                        edited[i] = SpeedKey::new(
                            Tick((at_secs * TICKS_PER_SECOND as f64).round() as i64),
                            ratio_from_pct(pct, reversed),
                        );
                        new_keys = Some(edited);
                    }
                    if remove.clicked() {
                        let mut edited = sorted.clone();
                        edited.remove(i);
                        new_keys = Some(edited);
                        discrete = true;
                    }
                    ui.end_row();
                }
            });
    }

    ui.horizontal(|ui| {
        if ui
            .add(egui::Button::new(RichText::new(format!("{} Add point", ph::PLUS))).small())
            .clicked()
        {
            let mut edited = sorted.clone();
            let default_at = match sorted.last() {
                None => Tick(clip.duration.0 / 2),
                Some(last) => {
                    let one_sec_later = Tick(last.at.0.saturating_add(TICKS_PER_SECOND));
                    if one_sec_later < clip.duration {
                        one_sec_later
                    } else {
                        // Clip too short for a full extra second — split the
                        // remaining gap to the clip's end instead.
                        Tick((last.at.0 + clip.duration.0) / 2)
                    }
                }
            }
            .min(clip.duration);
            let default_ratio = ramp_ratio_at(&sorted, default_at);
            edited.push(SpeedKey::new(default_at, default_ratio));
            new_keys = Some(edited);
            discrete = true;
        }
        if !sorted.is_empty() && ui.small_button("Clear ramp").clicked() {
            new_keys = Some(Vec::new());
            discrete = true;
        }
    });

    if let Some(mut edited) = new_keys {
        edited.sort_by_key(|k| k.at.0);
        let mut new_clip = clip.clone();
        new_clip.speed = SpeedMap::Keyframed { keys: edited };
        if discrete {
            set_clip_discrete(project, seq, track, new_clip, action);
        } else {
            set_clip_coalesced(project, seq, track, new_clip, action);
        }
    }
}

// ── Reframe (05 §4.2, CAP-012) ───────────────────────────────────────────────

fn draw_reframe_section(
    ui: &mut Ui,
    project: &TimelineProject,
    seq: SequenceId,
    track: TrackId,
    clip: &Clip,
    active_format: usize,
    action: &mut Option<PanelAction>,
) {
    // Recomputed here (not a 9th parameter — clippy's too-many-arguments
    // limit) from the same `project`/`seq` every other section already takes.
    let format_count = project
        .sequences
        .get(&seq)
        .map(|s| s.formats.len())
        .unwrap_or(1);
    egui::CollapsingHeader::new("Reframe")
        .default_open(false)
        .id_salt("clip_inspector_reframe")
        .show(ui, |ui| {
            // Per-format override dot row (13 §5.1: "small format-index chip
            // row shows which formats already have overrides").
            ui.horizontal(|ui| {
                for i in 0..format_count {
                    let overridden = clip.reframe.contains_key(&i);
                    let color = if i == active_format { ACCENT } else { MUTED };
                    let glyph = if overridden { "\u{25CF}" } else { "\u{25CB}" };
                    ui.label(RichText::new(glyph).color(color))
                        .on_hover_text(format!(
                            "Format {i}{}{}",
                            if i == active_format { " (active)" } else { "" },
                            if overridden { " — has override" } else { "" }
                        ));
                }
            });

            let has_override = clip.reframe.contains_key(&active_format);
            let base = clip
                .reframe
                .get(&active_format)
                .copied()
                .unwrap_or(clip.transform.base);
            let mut new_t = base;
            let mut changed = false;
            egui::Grid::new("clip_reframe_grid")
                .num_columns(2)
                .spacing([4.0, 2.0])
                .show(ui, |ui| {
                    changed |= transform_row(ui, "X", &mut new_t.x, None);
                    ui.end_row();
                    changed |= transform_row(ui, "Y", &mut new_t.y, None);
                    ui.end_row();
                    changed |= transform_row(ui, "Scale X", &mut new_t.scale_x, Some(0.0..=1000.0));
                    ui.end_row();
                    changed |= transform_row(ui, "Scale Y", &mut new_t.scale_y, Some(0.0..=1000.0));
                    ui.end_row();
                    changed |= transform_row_suffixed(ui, "Rotation", &mut new_t.rotation, " rad");
                    ui.end_row();
                });
            if changed {
                let mut new_clip = clip.clone();
                new_clip.reframe.insert(active_format, new_t);
                set_clip_coalesced(project, seq, track, new_clip, action);
            }
            ui.add_enabled_ui(has_override, |ui| {
                if ui.button("Reset reframe for this format").clicked() {
                    let mut new_clip = clip.clone();
                    new_clip.reframe.remove(&active_format);
                    set_clip_discrete(project, seq, track, new_clip, action);
                }
            });
        });
}

// ── Level horizon (18 DJI parity D-5) ───────────────────────────────────────

fn draw_level_horizon_section(
    ui: &mut Ui,
    project: &TimelineProject,
    seq: SequenceId,
    track: TrackId,
    clip: &Clip,
    active_format: usize,
    action: &mut Option<PanelAction>,
) {
    let Some(sequence) = project.sequences.get(&seq) else {
        return;
    };
    if sequence.track(track).map(|t| t.kind) != Some(TrackKind::Video) {
        return;
    }
    let format = sequence
        .formats
        .get(active_format)
        .unwrap_or_else(|| sequence.format());

    egui::CollapsingHeader::new("Level Horizon")
        .default_open(false)
        .id_salt("clip_inspector_level_horizon")
        .show(ui, |ui| {
            ui.label(
                RichText::new(format!(
                    "Applies to active format: {} ({} x {})",
                    format.name, format.width, format.height
                ))
                .color(MUTED)
                .small(),
            )
            .on_hover_text(
                "This correction creates or updates only the active format's reframe override.",
            );

            let effective = clip
                .reframe
                .get(&active_format)
                .copied()
                .unwrap_or(clip.transform.base);
            let mut correction_degrees =
                crate::app::reframe::normalized_horizon_degrees(effective.rotation)
                    .unwrap_or(0.0);
            ui.horizontal(|ui| {
                let label = ui.label("Correction");
                let changed = ui
                    .add(
                        egui::DragValue::new(&mut correction_degrees)
                            .speed(0.1)
                            .fixed_decimals(2)
                            .range(-180.0..=180.0)
                            .suffix(" deg"),
                    )
                    .labelled_by(label.id)
                    .on_hover_text(
                        "Roll correction for the active format, shown in degrees. Zero applies no correction.",
                    )
                    .changed();
                if changed && correction_degrees.is_finite() {
                    let mut new_t = effective;
                    new_t.rotation = correction_degrees.clamp(-180.0, 180.0).to_radians();
                    let mut new_clip = clip.clone();
                    new_clip.reframe.insert(active_format, new_t);
                    set_clip_coalesced(project, seq, track, new_clip, action);
                }
            });

            let centered = crate::app::reframe::horizon_auto_crop_is_centered(
                format.width as f64,
                format.height as f64,
                &effective,
            );
            let crop_scales = crate::app::reframe::horizon_auto_crop_scales(
                format.width as f64,
                format.height as f64,
                &effective,
            );
            let needs_crop = crop_scales.is_some_and(|(scale_x, scale_y)| {
                (scale_x - effective.scale_x).abs() > f64::EPSILON
                    || (scale_y - effective.scale_y).abs() > f64::EPSILON
            });
            let crop_tooltip = if !centered {
                "Auto-crop requires zero X/Y translation and an anchor at the active format's center."
            } else if crop_scales.is_none() {
                "Auto-crop requires a finite correction angle and positive finite X/Y scales."
            } else if !needs_crop {
                "The current scale already hides the rotated corners."
            } else {
                "Raises both scales by the smallest shared multiplier needed to hide the rotated corners."
            };
            let crop_response = ui
                .add_enabled(needs_crop, egui::Button::new("Auto-crop to hide corners"))
                .on_hover_text(crop_tooltip);
            if let (true, Some((scale_x, scale_y))) = (crop_response.clicked(), crop_scales) {
                let mut new_t = effective;
                // A shared multiplier preserves intentional X/Y scale ratios;
                // only scale changes, so position, anchor, opacity, and the
                // user's correction angle remain exactly as entered.
                new_t.scale_x = scale_x;
                new_t.scale_y = scale_y;
                let mut new_clip = clip.clone();
                new_clip.reframe.insert(active_format, new_t);
                set_clip_discrete(project, seq, track, new_clip, action);
            }
        });
}

// ── Effects stack (13 §5.1, 08 §2) ──────────────────────────────────────────

/// Whether `clip` carries any effect in its stack (enabled or disabled) — a
/// glanceable "has effects" signal. Exposed `pub(crate)` so another surface
/// (e.g. a future `app/timeline/clips.rs` fx badge, 14 §M-8) can read it
/// without re-deriving the effects-stack check itself; this story's territory
/// stops at `clip_inspector.rs`/`effects_browser.rs`, so wiring the badge into
/// the timeline lane paint is left to whoever picks that up — until then this
/// has no in-crate caller, hence the allow.
#[allow(dead_code)]
pub(crate) fn clip_has_effects(clip: &Clip) -> bool {
    !clip.effects.is_empty()
}

/// Display name for a stacked effect — prefer the live manifest name so
/// K-B16 catalogue entries (and any future id) show correctly; fall back to
/// the seven v1 kind labels, then a generic "Effect".
fn effect_label(effect: &photonic_core::timeline::ClipEffect) -> String {
    use photonic_core::timeline::manifest;
    if !effect.id.is_empty() {
        if let Some(m) = manifest(effect.id.clone()) {
            return m.name.to_string();
        }
        // Unknown / future id: surface the id rather than a blank "Effect".
        return effect.id.as_str().to_string();
    }
    match effect.kind {
        EffectKind::Blur => "Blur".into(),
        EffectKind::Sharpen => "Sharpen".into(),
        EffectKind::Glow => "Glow".into(),
        EffectKind::ChromaKey => "Chroma Key".into(),
        EffectKind::LumaKey => "Luma Key".into(),
        EffectKind::Invert => "Invert".into(),
        EffectKind::MaskShapeGen => "Mask Shape".into(),
        EffectKind::Unknown(tag) => tag.as_str().to_string(),
        _ => "Effect".into(),
    }
}

/// Which of the four effect stacks (26 §10 K-B1/K-B2, 35 §2) the Effects
/// section is editing. Session-only UI state, parked in egui temp memory
/// rather than `VideoPanelUi` — same idiom `node_editor.rs` uses for its
/// per-graph view state, and it keeps this story inside `panels/video/`.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
enum FxScopeTab {
    #[default]
    Clip,
    Track,
    Master,
    Asset,
}

fn fx_scope_id() -> egui::Id {
    egui::Id::new("clip_inspector_fx_scope")
}

/// The asset a clip inherits an effect stack from (K-B2), if it has one.
/// `Adjustment` / `SolidColor` / `Text` / nested-sequence clips have no bin
/// asset, so the Asset tab is unavailable for them.
fn clip_asset(clip: &Clip) -> Option<photonic_core::timeline::AssetId> {
    match clip.source {
        photonic_core::timeline::ClipSource::Asset { asset }
        | photonic_core::timeline::ClipSource::Vector { asset } => Some(asset),
        _ => None,
    }
}

fn owner_for(tab: FxScopeTab, seq: SequenceId, track: TrackId, clip: &Clip) -> Option<VfxOwner> {
    match tab {
        FxScopeTab::Clip => Some(VfxOwner::Clip(clip.id)),
        FxScopeTab::Track => Some(VfxOwner::Track(track)),
        FxScopeTab::Master => Some(VfxOwner::Master(seq)),
        FxScopeTab::Asset => clip_asset(clip).map(VfxOwner::Asset),
    }
}

/// One line of orientation per scope — which stack the rows below belong to and
/// where it sits in the evaluation order (35 §2: asset → clip → track → master).
fn scope_hint(tab: FxScopeTab, project: &TimelineProject, track: TrackId, clip: &Clip) -> String {
    match tab {
        FxScopeTab::Clip => format!("This clip only — \"{}\".", clip.name),
        FxScopeTab::Track => {
            let name = project
                .sequences
                .values()
                .find_map(|s| s.track(track))
                .map(|t| t.name.clone())
                .unwrap_or_default();
            format!("Every clip on track \"{name}\", after they fold together.")
        }
        FxScopeTab::Master => {
            "The whole sequence, after all tracks are merged. Applied last.".to_string()
        }
        FxScopeTab::Asset => {
            "Every timeline instance of this media, before the clip's own stack.".to_string()
        }
    }
}

fn draw_effects_section(
    ui: &mut Ui,
    project: &TimelineProject,
    seq: SequenceId,
    track: TrackId,
    clip: &Clip,
    action: &mut Option<PanelAction>,
) {
    egui::CollapsingHeader::new("Effects")
        .default_open(true)
        .id_salt("clip_inspector_effects")
        .show(ui, |ui| {
            let mut tab: FxScopeTab = ui.data(|d| d.get_temp(fx_scope_id())).unwrap_or_default();
            let has_asset = clip_asset(clip).is_some();
            ui.horizontal_wrapped(|ui| {
                for (t, label) in [
                    (FxScopeTab::Clip, "Clip"),
                    (FxScopeTab::Track, "Track"),
                    (FxScopeTab::Master, "Master"),
                    (FxScopeTab::Asset, "Asset"),
                ] {
                    let enabled = t != FxScopeTab::Asset || has_asset;
                    let resp = ui.add_enabled(enabled, egui::SelectableLabel::new(tab == t, label));
                    if resp.clicked() {
                        tab = t;
                    }
                    if t == FxScopeTab::Asset && !has_asset {
                        resp.on_hover_text(
                            "This clip has no bin asset (adjustment / title / solid).",
                        );
                    }
                }
            });
            if tab == FxScopeTab::Asset && !has_asset {
                tab = FxScopeTab::Clip;
            }
            ui.data_mut(|d| d.insert_temp(fx_scope_id(), tab));
            ui.label(
                RichText::new(scope_hint(tab, project, track, clip))
                    .color(MUTED)
                    .small(),
            );

            let Some(owner) = owner_for(tab, seq, track, clip) else {
                return;
            };
            let Ok(stack) = ops::effect_stack(project, owner) else {
                ui.label(RichText::new("Stack unavailable.").color(MUTED).small());
                return;
            };

            let drop_rect = ui.available_rect_before_wrap();
            for i in 0..stack.len() {
                let effect = &stack[i];
                ui.push_id(("fx_row", scope_salt(owner), i), |ui| {
                    ui.horizontal(|ui| {
                        let mut enabled = effect.enabled;
                        if ui.checkbox(&mut enabled, "").changed() {
                            let mut new_effect = effect.clone();
                            new_effect.enabled = enabled;
                            if let Ok(cmd) = ops::set_effect_scoped(project, owner, i, new_effect) {
                                *action = Some(PanelAction::ClipEditDiscrete(cmd));
                            }
                        }
                        let name = effect_label(effect);
                        let label = if enabled {
                            RichText::new(name)
                        } else {
                            RichText::new(name).color(MUTED)
                        };
                        ui.label(label);
                        // Keyboard-reachable reorder fallback (13 §16 finding
                        // #1's flagged a11y gap for drag-only stacks) — up/down
                        // buttons instead of/alongside drag.
                        if ui
                            .add_enabled(i > 0, egui::Button::new("\u{2191}").small())
                            .on_hover_text("Move up")
                            .clicked()
                        {
                            if let Ok(cmd) = ops::reorder_effects_scoped(
                                project,
                                owner,
                                swapped_order(stack.len(), i, i - 1),
                            ) {
                                *action = Some(PanelAction::ClipEditDiscrete(cmd));
                            }
                        }
                        if ui
                            .add_enabled(i + 1 < stack.len(), egui::Button::new("\u{2193}").small())
                            .on_hover_text("Move down")
                            .clicked()
                        {
                            if let Ok(cmd) = ops::reorder_effects_scoped(
                                project,
                                owner,
                                swapped_order(stack.len(), i, i + 1),
                            ) {
                                *action = Some(PanelAction::ClipEditDiscrete(cmd));
                            }
                        }
                        if ui
                            .button("\u{2715}")
                            .on_hover_text("Remove effect")
                            .clicked()
                        {
                            if let Ok(cmd) = ops::remove_effect_scoped(project, owner, i) {
                                *action = Some(PanelAction::ClipEditDiscrete(cmd));
                            }
                        }
                    });
                    egui::CollapsingHeader::new("params")
                        .id_salt(("fx_params", scope_salt(owner), i))
                        .default_open(false)
                        .show(ui, |ui| {
                            draw_effect_params(ui, project, owner, effect, i, action);
                        });
                });
            }
            // K-B4: apply a saved preset to *this* scope, or save this scope's
            // stack (plus its grade) as a new one. Applying is one undo unit;
            // saving is a config-file write and is not undoable — the bar's
            // hover text says which is which.
            super::effect_presets::draw_scope_preset_bar(ui, project, owner, action);

            if stack.is_empty() {
                // Only the clip stack has the double-click shortcut (the
                // Effects browser applies to the *clip selection*); every scope
                // accepts a drag onto this section.
                let hint = if tab == FxScopeTab::Clip {
                    "No effects — drag one from the Effects browser, or double-click it \
                     there to apply to this clip."
                } else {
                    "No effects — drag one from the Effects browser onto this section."
                };
                ui.label(RichText::new(hint).color(MUTED).small());
            }

            // Drop target for an Effects-browser drag (13 §6.3: "the Clip
            // Inspector's effects-stack section" is a valid drop target,
            // mirroring the media-pool → timeline `AssetDrag` idiom). The drop
            // lands on whichever scope tab is showing.
            if let Some(payload) =
                egui::DragAndDrop::payload::<super::effects_browser::EffectDrag>(ui.ctx())
            {
                let hovering = ui
                    .ctx()
                    .pointer_latest_pos()
                    .is_some_and(|p| drop_rect.contains(p));
                if hovering {
                    ui.painter()
                        .rect_stroke(drop_rect, 3.0, egui::Stroke::new(1.5, ACCENT));
                    if ui.input(|i| i.pointer.any_released()) {
                        egui::DragAndDrop::clear_payload(ui.ctx());
                        let effect =
                            photonic_core::timeline::ClipEffect::from_manifest(payload.id.clone())
                                .or_else(|| {
                                    // Fallback for a payload whose id this build no longer
                                    // knows (shouldn't happen for MANIFESTS-driven drags).
                                    payload
                                        .id
                                        .legacy_kind()
                                        .map(photonic_core::timeline::ClipEffect::new)
                                });
                        if let Some(effect) = effect {
                            if let Ok(cmd) = ops::add_effect_scoped(project, owner, effect, None) {
                                *action = Some(PanelAction::ClipEditDiscrete(cmd));
                            }
                        }
                    }
                }
            }
        });
}

/// A stable per-scope egui id seed, so switching tabs does not collide widget
/// ids between two stacks that happen to have the same row count. `VfxOwner`
/// is `Hash`, so it is its own salt — this exists only to name the intent.
fn scope_salt(owner: VfxOwner) -> VfxOwner {
    owner
}

/// The `new_order` permutation `reorder_effects` expects: swap the elements
/// currently at `a`/`b` in the identity ordering.
fn swapped_order(len: usize, a: usize, b: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..len).collect();
    order.swap(a, b);
    order
}

/// Static (non-keyframed) param editor for one stacked effect at any scope.
/// Writes a `SetEffect` command — the one carrier that works for clip, track,
/// master and asset stacks alike (a track stack has no `SetClipProp`-style
/// whole-owner snapshot to ride).
fn draw_effect_params(
    ui: &mut Ui,
    project: &TimelineProject,
    owner: VfxOwner,
    effect: &photonic_core::timeline::ClipEffect,
    effect_idx: usize,
    action: &mut Option<PanelAction>,
) {
    let entries = prop_registry::entries(PropTargetKind::Effect(effect.kind));
    if entries.is_empty() {
        ui.label(RichText::new("No parameters.").color(MUTED).small());
        return;
    }
    // K-B6: cross-param refs (`%radius`, bare `amount`) + optional frame size.
    let float_pairs: Vec<(&str, f64)> = effect
        .params
        .base
        .entries
        .iter()
        .filter_map(|(p, v)| match v {
            PropValue::Float(f) => Some((p.as_str(), *f)),
            _ => None,
        })
        .collect();
    let (frame_w, frame_h) = active_frame_size(project);
    let vars = param_expr::vars_from_params(float_pairs, frame_w, frame_h);
    let seeded = EffectParams::seed(PropTargetKind::Effect(effect.kind));

    egui::Grid::new(("fx_params_grid", scope_salt(owner), effect_idx))
        .num_columns(2)
        .spacing([4.0, 2.0])
        .show(ui, |ui| {
            for entry in entries {
                let Some(cur) = effect.params.base.get(entry.path) else {
                    continue;
                };
                ui.label(param_short_label(entry.path));
                let mut new_value: Option<PropValue> = None;
                match *cur {
                    PropValue::Float(f) => {
                        let mut v = f;
                        let default = match seeded.get(entry.path) {
                            Some(PropValue::Float(d)) => *d,
                            _ => param_expr::neutral_float_default(entry.range),
                        };
                        if param_expr::float_drag(ui, &mut v, default, entry.range, &vars, 0.01) {
                            new_value = Some(PropValue::Float(v));
                        }
                    }
                    PropValue::Bool(b) => {
                        let mut v = b;
                        let resp = ui.checkbox(&mut v, "");
                        let mut changed = resp.changed();
                        // Middle-click reset → seed default (false for bools).
                        if resp.middle_clicked() && v {
                            v = false;
                            changed = true;
                        }
                        if changed {
                            new_value = Some(PropValue::Bool(v));
                        }
                    }
                    PropValue::Color(c) => {
                        let mut v = c;
                        if crate::color_popup::ColorPopup::swatch_color(ui, &mut v).changed() {
                            new_value = Some(PropValue::Color(v));
                        }
                    }
                    PropValue::Vec2(v2) => {
                        let mut v = v2;
                        let r0 = ui.add(egui::DragValue::new(&mut v[0]).speed(0.01));
                        let r1 = ui.add(egui::DragValue::new(&mut v[1]).speed(0.01));
                        if r0.changed() || r1.changed() {
                            new_value = Some(PropValue::Vec2(v));
                        }
                    }
                    PropValue::Enum(e) => {
                        let mut v = e;
                        if ui.add(egui::DragValue::new(&mut v)).changed() {
                            new_value = Some(PropValue::Enum(v));
                        }
                    }
                }
                if let Some(v) = new_value {
                    let mut new_effect = effect.clone();
                    new_effect.params.base.set(entry.path, v);
                    // Coalesced: a drag on one param is one undo unit
                    // (`TimelineCmd::coalesce` merges same owner + index).
                    if let Ok(cmd) = ops::set_effect_scoped(project, owner, effect_idx, new_effect)
                    {
                        *action = Some(PanelAction::ClipEditCoalesced(cmd));
                    }
                }
                ui.end_row();
            }
        });
}

/// Active sequence frame size for K-B6 `%w`/`%h` expressions, if known.
fn active_frame_size(project: &TimelineProject) -> (Option<f64>, Option<f64>) {
    let Some(seq_id) = project.active_sequence else {
        return (None, None);
    };
    let Some(seq) = project.sequences.get(&seq_id) else {
        return (None, None);
    };
    let fmt = seq
        .formats
        .get(seq.active_format)
        .or_else(|| seq.formats.first());
    match fmt {
        Some(f) => (Some(f.width as f64), Some(f.height as f64)),
        None => (None, None),
    }
}

/// `"params.radius"` → `"radius"` for a compact grid label.
fn param_short_label(path: &str) -> &str {
    path.rsplit('.').next().unwrap_or(path)
}

// ── Keyframes (13 §5.2, 14 §G-9 effect-controls unification) ────────────────

/// The docked keyframe/curve editor section — replaces the floating
/// `egui::Window` as the default, always-visible per-clip Effect-Controls
/// surface (14 §G-9: "two surfaces for one concept" between this inspector's
/// fixed Motion/Opacity/Speed fields and the curve editor). The actual
/// picker + curve drawing is [`super::keyframe_editor::draw_embedded`],
/// reused verbatim from the floating editor (`keyframe_editor.rs:draw_window`)
/// rather than reimplemented; this fn is only the section chrome plus a
/// pop-out button that still opens the float for live-playhead scrubbing or
/// the wider side-by-side layout (watch-out: "keep the float option").
fn draw_keyframes_section(
    ui: &mut Ui,
    ctx: &mut PropPanelCtx,
    project: &TimelineProject,
    clip: &Clip,
    action: &mut Option<PanelAction>,
) {
    egui::CollapsingHeader::new("Keyframes")
        .default_open(true)
        .id_salt("clip_inspector_keyframes")
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Transform + effect params, animated over the clip")
                        .color(MUTED)
                        .small(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(egui::Button::new(RichText::new(ph::ARROW_SQUARE_OUT)).small())
                        .on_hover_text("Pop out to a floating window (live playhead scrubbing)")
                        .clicked()
                    {
                        *ctx.video.keyframe_editor_target = Some(clip.id);
                    }
                });
            });
            if let Some(a) = super::keyframe_editor::draw_embedded(ui, project, clip) {
                *action = Some(a);
            }
        });
}

// ── Transitions (13 §5.1, 08 §2.0b) ─────────────────────────────────────────

const TRANSITION_KINDS: [TransitionKind; 5] = [
    TransitionKind::CrossDissolve,
    TransitionKind::DipToBlack,
    TransitionKind::DipToColor,
    TransitionKind::Wipe,
    TransitionKind::Push,
];

fn transition_label(kind: TransitionKind) -> &'static str {
    match kind {
        TransitionKind::CrossDissolve => "Cross Dissolve",
        TransitionKind::DipToBlack => "Dip to Black",
        TransitionKind::DipToColor => "Dip to Color",
        TransitionKind::Wipe => "Wipe",
        TransitionKind::Push => "Push",
        // Forward-compat (39 §2.2): show the preserved tag; renders as a cut.
        TransitionKind::Unknown(t) => t.as_str(),
        // `#[non_exhaustive]`: a kind a newer build adds shows a placeholder.
        _ => "Unsupported",
    }
}

fn draw_transitions_section(
    ui: &mut Ui,
    project: &TimelineProject,
    seq: SequenceId,
    track: TrackId,
    clip: &Clip,
    action: &mut Option<PanelAction>,
) {
    egui::CollapsingHeader::new("Transitions")
        .default_open(false)
        .id_salt("clip_inspector_transitions")
        .show(ui, |ui| {
            draw_one_transition(ui, project, seq, track, clip, true, action);
            ui.separator();
            draw_one_transition(ui, project, seq, track, clip, false, action);
        });
}

fn draw_one_transition(
    ui: &mut Ui,
    project: &TimelineProject,
    seq: SequenceId,
    track: TrackId,
    clip: &Clip,
    is_in: bool,
    action: &mut Option<PanelAction>,
) {
    let label = if is_in { "In" } else { "Out" };
    let current = if is_in {
        &clip.transition_in
    } else {
        &clip.transition_out
    };
    ui.horizontal(|ui| {
        ui.label(label);
        match current {
            None => {
                if ui.small_button("Add Transition").clicked() {
                    let mut new_clip = clip.clone();
                    let t = Transition::new(
                        TransitionKind::CrossDissolve,
                        Tick(TICKS_PER_SECOND / 2), // 0.5s default overlap
                    );
                    if is_in {
                        new_clip.transition_in = Some(t);
                    } else {
                        new_clip.transition_out = Some(t);
                    }
                    set_clip_discrete(project, seq, track, new_clip, action);
                }
            }
            Some(t) => {
                let mut kind = t.kind;
                egui::ComboBox::new(("transition_kind", clip.id, is_in), "")
                    .selected_text(transition_label(kind))
                    .show_ui(ui, |ui| {
                        for k in TRANSITION_KINDS {
                            ui.selectable_value(&mut kind, k, transition_label(k));
                        }
                    });
                let mut secs = t.duration.as_seconds_f64();
                let dur_changed = ui
                    .add(
                        egui::DragValue::new(&mut secs)
                            .speed(0.05)
                            .range(0.01..=60.0)
                            .suffix("s"),
                    )
                    .changed();
                if kind != t.kind || dur_changed {
                    let mut new_clip = clip.clone();
                    let new_t = Transition {
                        kind,
                        duration: Tick((secs * TICKS_PER_SECOND as f64) as i64),
                        params: if kind == t.kind {
                            t.params
                        } else {
                            TransitionParams::default()
                        },
                    };
                    if is_in {
                        new_clip.transition_in = Some(new_t);
                    } else {
                        new_clip.transition_out = Some(new_t);
                    }
                    set_clip_coalesced(project, seq, track, new_clip, action);
                }
                if ui.small_button("Remove").clicked() {
                    let mut new_clip = clip.clone();
                    if is_in {
                        new_clip.transition_in = None;
                    } else {
                        new_clip.transition_out = None;
                    }
                    set_clip_discrete(project, seq, track, new_clip, action);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swapped_order_swaps_only_the_two_indices() {
        assert_eq!(swapped_order(4, 1, 2), vec![0, 2, 1, 3]);
        assert_eq!(swapped_order(3, 0, 2), vec![2, 1, 0]);
        assert_eq!(swapped_order(1, 0, 0), vec![0]);
    }

    /// The scope tabs resolve to the right owner, and the Asset tab is only
    /// offered for a clip that actually has a bin asset (K-B2).
    #[test]
    fn scope_tabs_resolve_to_the_right_owner() {
        use photonic_core::timeline::{AssetId, ClipSource};

        let seq = SequenceId::new();
        let track = TrackId::new();
        let asset = AssetId::new();
        let media_clip = Clip::new(ClipSource::Asset { asset }, Tick(0), Tick(1000));
        assert_eq!(clip_asset(&media_clip), Some(asset));
        assert_eq!(
            owner_for(FxScopeTab::Clip, seq, track, &media_clip),
            Some(VfxOwner::Clip(media_clip.id))
        );
        assert_eq!(
            owner_for(FxScopeTab::Track, seq, track, &media_clip),
            Some(VfxOwner::Track(track))
        );
        assert_eq!(
            owner_for(FxScopeTab::Master, seq, track, &media_clip),
            Some(VfxOwner::Master(seq))
        );
        assert_eq!(
            owner_for(FxScopeTab::Asset, seq, track, &media_clip),
            Some(VfxOwner::Asset(asset))
        );

        // An adjustment clip has no bin asset: the Asset tab has no owner.
        let adj = Clip::new(ClipSource::Adjustment, Tick(0), Tick(1000));
        assert_eq!(clip_asset(&adj), None);
        assert_eq!(owner_for(FxScopeTab::Asset, seq, track, &adj), None);
        assert_eq!(
            owner_for(FxScopeTab::Clip, seq, track, &adj),
            Some(VfxOwner::Clip(adj.id))
        );
    }

    /// The panel's four buttons (add / remove / reorder / param) all build a
    /// real `ops::*_scoped` command against a track stack, and each is one
    /// undoable step whose inverse restores the exact prior stack. This is the
    /// same code path `draw_effects_section` runs; the widget layer above it is
    /// what egui owns.
    #[test]
    fn track_scope_panel_actions_build_undoable_commands() {
        use photonic_core::history::Command;
        use photonic_core::timeline::time::FrameRate;
        use photonic_core::timeline::{ClipEffect, ClipSource, Sequence, TimelineProject, Track};
        use photonic_core::Document;

        let mut project = TimelineProject::new();
        let mut sequence = Sequence::new("S", FrameRate::FPS_30, 1920, 1080);
        let mut vtrack = Track::new(TrackKind::Video, "V1");
        vtrack
            .clips
            .push(Clip::new(ClipSource::Adjustment, Tick(0), Tick(100)));
        let track_id = vtrack.id;
        sequence.video_tracks.push(vtrack);
        project.insert_sequence(sequence);
        let mut doc = Document::new("t", 100.0, 100.0);
        doc.timeline = Some(project);

        let owner = VfxOwner::Track(track_id);
        for kind in [EffectKind::Blur, EffectKind::Sharpen] {
            let p = doc.timeline.as_ref().unwrap();
            let cmd = ops::add_effect_scoped(p, owner, ClipEffect::new(kind), None).unwrap();
            Command::Timeline(cmd).apply(&mut doc);
        }
        assert_eq!(
            ops::effect_stack(doc.timeline.as_ref().unwrap(), owner)
                .unwrap()
                .len(),
            2
        );

        // "Move up" on row 1 — the panel's `swapped_order` permutation.
        let p = doc.timeline.as_ref().unwrap();
        let cmd = ops::reorder_effects_scoped(p, owner, swapped_order(2, 1, 0)).unwrap();
        let inverse = cmd.inverse(&doc).unwrap();
        Command::Timeline(cmd).apply(&mut doc);
        assert_eq!(
            ops::effect_stack(doc.timeline.as_ref().unwrap(), owner).unwrap()[0].kind,
            EffectKind::Sharpen
        );
        Command::Timeline(inverse).apply(&mut doc);
        assert_eq!(
            ops::effect_stack(doc.timeline.as_ref().unwrap(), owner).unwrap()[0].kind,
            EffectKind::Blur
        );

        // The enable checkbox and the param grid both go through SetEffect.
        let p = doc.timeline.as_ref().unwrap();
        let mut off = ops::effect_stack(p, owner).unwrap()[0].clone();
        off.enabled = false;
        let cmd = ops::set_effect_scoped(p, owner, 0, off).unwrap();
        let inverse = cmd.inverse(&doc).unwrap();
        Command::Timeline(cmd).apply(&mut doc);
        assert!(!ops::effect_stack(doc.timeline.as_ref().unwrap(), owner).unwrap()[0].enabled);
        Command::Timeline(inverse).apply(&mut doc);
        assert!(ops::effect_stack(doc.timeline.as_ref().unwrap(), owner).unwrap()[0].enabled);

        // The "×" button.
        let p = doc.timeline.as_ref().unwrap();
        let cmd = ops::remove_effect_scoped(p, owner, 0).unwrap();
        let inverse = cmd.inverse(&doc).unwrap();
        Command::Timeline(cmd).apply(&mut doc);
        assert_eq!(
            ops::effect_stack(doc.timeline.as_ref().unwrap(), owner)
                .unwrap()
                .len(),
            1
        );
        Command::Timeline(inverse).apply(&mut doc);
        let stack = ops::effect_stack(doc.timeline.as_ref().unwrap(), owner).unwrap();
        assert_eq!(stack.len(), 2);
        assert_eq!(stack[0].kind, EffectKind::Blur, "re-inserted at index 0");
    }

    #[test]
    fn param_short_label_strips_prefix() {
        assert_eq!(param_short_label("params.radius"), "radius");
        assert_eq!(param_short_label("transform.x"), "x");
        assert_eq!(param_short_label("noprefix"), "noprefix");
    }

    #[test]
    fn clip_has_effects_reflects_stack_emptiness() {
        use photonic_core::timeline::ClipSource;

        let mut clip = Clip::new(ClipSource::Adjustment, Tick(0), Tick(1000));
        assert!(!clip_has_effects(&clip));
        clip.effects
            .push(photonic_core::timeline::ClipEffect::new(EffectKind::Blur));
        assert!(clip_has_effects(&clip));
    }

    #[test]
    fn ratio_from_pct_round_trips_forward_and_reverse() {
        assert_eq!(ratio_from_pct(100.0, false), Ratio::new(1000, 1000));
        assert_eq!(ratio_from_pct(200.0, false), Ratio::new(2000, 1000));
        assert_eq!(ratio_from_pct(50.0, true), Ratio::new(-500, 1000));
        // Never zero even at the floor — a 0-speed clip would divide by zero
        // downstream in `SpeedMap::source_delta`.
        assert_eq!(ratio_from_pct(0.0, false), Ratio::new(1, 1000));
    }

    #[test]
    fn ramp_ratio_at_holds_first_before_and_last_after() {
        let keys = vec![
            SpeedKey::new(Tick(1000), Ratio::new(500, 1000)),
            SpeedKey::new(Tick(3000), Ratio::new(2000, 1000)),
        ];
        // Already sorted by `at` — every caller sorts before calling.
        assert_eq!(ramp_ratio_at(&keys, Tick(0)), Ratio::new(500, 1000));
        assert_eq!(ramp_ratio_at(&keys, Tick(1000)), Ratio::new(500, 1000));
        assert_eq!(ramp_ratio_at(&keys, Tick(2000)), Ratio::new(500, 1000));
        assert_eq!(ramp_ratio_at(&keys, Tick(3000)), Ratio::new(2000, 1000));
        assert_eq!(ramp_ratio_at(&keys, Tick(9999)), Ratio::new(2000, 1000));
    }

    #[test]
    fn ramp_ratio_at_empty_ramp_is_identity() {
        assert_eq!(ramp_ratio_at(&[], Tick(500)), Ratio::ONE);
    }
}
