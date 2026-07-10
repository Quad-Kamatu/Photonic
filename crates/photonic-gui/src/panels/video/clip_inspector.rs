//! `DrawerGroup::ClipInspector` panel (04 §4.1) — the selected clip's
//! transform / speed / effects-stack / transition params, the `Clip`/`ClipEffect`
//! analogue of the vector Inspector. Panel shell owned by 04; widgets source
//! from `prop_registry` (01 §6.2). Keyframe-editor targeting lives on
//! [`super::VideoPanelUi::keyframe_editor_target`].
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
use photonic_core::timeline::{
    ops, prop_registry, Clip, ClipId, EffectKind, PropTargetKind, PropValue, Ratio, SequenceId,
    SpeedMap, TimelineProject, Tick, Transition, TransitionKind, TransitionParams, TrackId,
    TICKS_PER_SECOND,
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

    draw_transform_section(ui, ctx, project, seq_id, track_id, clip, &mut action);
    draw_speed_section(ui, project, seq_id, track_id, clip, &mut action);
    draw_reframe_section(ui, project, seq_id, track_id, clip, active_format, &mut action);
    draw_effects_section(ui, project, seq_id, track_id, clip, &mut action);
    draw_transitions_section(ui, project, seq_id, track_id, clip, &mut action);

    if action.is_some() {
        ctx.action = action;
    }
}

// ── Clip location (session `ClipId` → (seq, track, &Clip)) ─────────────────

/// `ctx.video.selection` (04 §2.6 session state) carries only `ClipId`s, so
/// the inspector resolves the owning sequence/track itself — no other panel
/// exposes this lookup.
fn locate_clip(
    project: &TimelineProject,
    clip_id: ClipId,
) -> Option<(SequenceId, TrackId, &Clip)> {
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
                    changed |=
                        transform_row_suffixed(ui, "Rotation", &mut new_t.rotation, " rad");
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
    let mut drag = egui::DragValue::new(v).speed(0.5).fixed_decimals(2);
    if let Some(r) = range {
        drag = drag.range(r);
    }
    ui.add(drag).changed()
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

// ── Speed (13 §5.1, 01 §5.1) ─────────────────────────────────────────────────

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
            let SpeedMap::Constant(r) = clip.speed;
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
                // Rational approximation at 1/1000 precision — exact enough
                // for a UI-driven speed field (01 §5.1's "exact rational" rule
                // governs storage/eval, not that every possible value must be
                // keyboard-reachable at infinite precision).
                let den: u32 = 1000;
                let mag = ((pct / 100.0) * den as f64).round().max(1.0) as i32;
                let num = if reversed { -mag } else { mag };
                let mut new_clip = clip.clone();
                new_clip.speed = SpeedMap::Constant(Ratio::new(num, den));
                set_clip_coalesced(project, seq, track, new_clip, action);
            }
        });
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
                    changed |=
                        transform_row_suffixed(ui, "Rotation", &mut new_t.rotation, " rad");
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

fn effect_label(kind: EffectKind) -> &'static str {
    match kind {
        EffectKind::Blur => "Blur",
        EffectKind::Sharpen => "Sharpen",
        EffectKind::Glow => "Glow",
        EffectKind::ChromaKey => "Chroma Key",
        EffectKind::LumaKey => "Luma Key",
        EffectKind::Invert => "Invert",
        EffectKind::MaskShapeGen => "Mask Shape",
        _ => "Effect",
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
            let drop_rect = ui.available_rect_before_wrap();
            for i in 0..clip.effects.len() {
                let effect = &clip.effects[i];
                ui.push_id(("clip_effect_row", clip.id, i), |ui| {
                    ui.horizontal(|ui| {
                        let mut enabled = effect.enabled;
                        if ui.checkbox(&mut enabled, "").changed() {
                            let mut new_clip = clip.clone();
                            new_clip.effects[i].enabled = enabled;
                            set_clip_discrete(project, seq, track, new_clip, action);
                        }
                        let label = if enabled {
                            RichText::new(effect_label(effect.kind))
                        } else {
                            RichText::new(effect_label(effect.kind)).color(MUTED)
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
                            if let Ok(cmd) = ops::reorder_effects(
                                project,
                                seq,
                                track,
                                clip.id,
                                swapped_order(clip.effects.len(), i, i - 1),
                            ) {
                                *action = Some(PanelAction::ClipEditDiscrete(cmd));
                            }
                        }
                        if ui
                            .add_enabled(
                                i + 1 < clip.effects.len(),
                                egui::Button::new("\u{2193}").small(),
                            )
                            .on_hover_text("Move down")
                            .clicked()
                        {
                            if let Ok(cmd) = ops::reorder_effects(
                                project,
                                seq,
                                track,
                                clip.id,
                                swapped_order(clip.effects.len(), i, i + 1),
                            ) {
                                *action = Some(PanelAction::ClipEditDiscrete(cmd));
                            }
                        }
                        if ui
                            .button("\u{2715}")
                            .on_hover_text("Remove effect")
                            .clicked()
                        {
                            if let Ok(cmd) = ops::remove_effect(project, seq, track, clip.id, i) {
                                *action = Some(PanelAction::ClipEditDiscrete(cmd));
                            }
                        }
                    });
                    egui::CollapsingHeader::new("params")
                        .id_salt(("clip_effect_params", clip.id, i))
                        .default_open(false)
                        .show(ui, |ui| {
                            draw_effect_params(ui, project, seq, track, clip, i, action);
                        });
                });
            }
            if clip.effects.is_empty() {
                ui.label(
                    RichText::new(
                        "No effects — drag one from the Effects browser, or double-click it \
                         there to apply to this clip.",
                    )
                    .color(MUTED)
                    .small(),
                );
            }

            // Drop target for an Effects-browser drag (13 §6.3: "the Clip
            // Inspector's effects-stack section" is a valid drop target,
            // mirroring the media-pool → timeline `AssetDrag` idiom).
            if let Some(payload) =
                egui::DragAndDrop::payload::<super::effects_browser::EffectDrag>(ui.ctx())
            {
                let hovering = ui
                    .ctx()
                    .pointer_latest_pos()
                    .is_some_and(|p| drop_rect.contains(p));
                if hovering {
                    ui.painter().rect_stroke(
                        drop_rect,
                        3.0,
                        egui::Stroke::new(1.5, ACCENT),
                    );
                    if ui.input(|i| i.pointer.any_released()) {
                        egui::DragAndDrop::clear_payload(ui.ctx());
                        if let Ok(cmd) = ops::add_effect(
                            project,
                            seq,
                            track,
                            clip.id,
                            photonic_core::timeline::ClipEffect::new(payload.kind),
                            None,
                        ) {
                            *action = Some(PanelAction::ClipEditDiscrete(cmd));
                        }
                    }
                }
            }
        });
}

/// The `new_order` permutation `reorder_effects` expects: swap the elements
/// currently at `a`/`b` in the identity ordering.
fn swapped_order(len: usize, a: usize, b: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..len).collect();
    order.swap(a, b);
    order
}

fn draw_effect_params(
    ui: &mut Ui,
    project: &TimelineProject,
    seq: SequenceId,
    track: TrackId,
    clip: &Clip,
    effect_idx: usize,
    action: &mut Option<PanelAction>,
) {
    let effect = &clip.effects[effect_idx];
    let entries = prop_registry::entries(PropTargetKind::Effect(effect.kind));
    if entries.is_empty() {
        ui.label(RichText::new("No parameters.").color(MUTED).small());
        return;
    }
    egui::Grid::new(("clip_effect_params_grid", clip.id, effect_idx))
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
                        let mut drag = egui::DragValue::new(&mut v).speed(0.01);
                        if let Some((lo, hi)) = entry.range {
                            drag = drag.range(lo..=hi);
                        }
                        if ui.add(drag).changed() {
                            new_value = Some(PropValue::Float(v));
                        }
                    }
                    PropValue::Bool(b) => {
                        let mut v = b;
                        if ui.checkbox(&mut v, "").changed() {
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
                    let mut new_clip = clip.clone();
                    new_clip.effects[effect_idx].params.base.set(entry.path, v);
                    set_clip_coalesced(project, seq, track, new_clip, action);
                }
                ui.end_row();
            }
        });
}

/// `"params.radius"` → `"radius"` for a compact grid label.
fn param_short_label(path: &str) -> &str {
    path.rsplit('.').next().unwrap_or(path)
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
}
