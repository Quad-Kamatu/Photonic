//! Keyframe / curve editor (04 §4.1, 01 §6, 13 §5.2/§16) — the editing UI over
//! the generic [`AnimProps`](photonic_core::timeline::AnimProps) animation system.
//!
//! Two surfaces, both owned here:
//! - [`draw_window`] — a floating curve editor for the selected clip: an
//!   animatable-property picker (transform + effect params, 01 §6.2 registry) on
//!   the left, and a Bezier-capable curve editor (property tracks, Hold/Linear/
//!   Bezier easing, add/move/delete keyframes) on the right. Enabling AS-3
//!   motion-graphics: place a `VectorDoc` clip (media pool → timeline) and
//!   animate its `transform.*` here.
//! - [`paint_clip_automation`] — the timeline automation-lane indicator: keyframe
//!   diamonds painted along a clip's body so animated clips read at a glance.
//!
//! **Every mutation flows through a pure core op → `CommandHistory`** — this
//! module never touches `doc.timeline` directly. Drag gestures preview a ghost
//! and commit ONE `execute_discrete` step on release (one undo step per gesture,
//! the same discipline `app/timeline/ops_bridge.rs` uses for clip drags).

use egui::{Align2, Color32, FontId, Pos2, Rect, Rounding, Sense, Shape, Stroke, Vec2};
use egui_phosphor::regular as ph;

use crate::app::timeline::TimelineView;
use photonic_core::document::Document;
use photonic_core::history::{Command, CommandHistory};
use photonic_core::timeline::{
    anim, ops, prop_registry, AnimTarget, Clip, ClipId, FrameRate, Interp, Keyframe, PropPath,
    PropTargetKind, PropValue, PropValueKind, PropertyTrack, Tick, TimelineProject,
};

// ── egui memory keys ─────────────────────────────────────────────────────────

const STATE_ID: &str = "photonic_kf_editor_state";
const DRAG_ID: &str = "photonic_kf_editor_drag";

/// Persistent (session-only) editor UI state, kept in egui memory so this module
/// needs no new `PhotonicApp` fields. Reset when the targeted clip changes.
#[derive(Clone, Copy, Default)]
struct EditorState {
    /// Clip these selections belong to (reset row/keyframe when it changes).
    clip: Option<ClipId>,
    /// Selected property-row index into [`build_rows`]'s flattened list.
    sel_row: usize,
    /// Clip-relative tick of the selected keyframe, if any.
    sel_kf_at: Option<i64>,
    /// Last timeline selection observed — drives auto-open on selection change.
    last_selection: Option<ClipId>,
    /// Whether `last_selection` has ever been recorded (distinguishes "no
    /// selection yet" from "deselected", so the editor doesn't auto-open on the
    /// very first frame before any clip is picked).
    seeded: bool,
}

/// Which grip of a keyframe an in-progress drag holds.
#[derive(Clone, Copy, PartialEq)]
enum Grip {
    /// The keyframe point itself (moves time + value).
    Point,
    /// The outgoing Bezier control handle (near this keyframe).
    HandleOut,
    /// The incoming Bezier control handle (near the next keyframe).
    HandleIn,
}

#[derive(Clone, Copy)]
struct KfDrag {
    row: usize,
    /// Clip-relative tick of the dragged keyframe at gesture start.
    orig_at: i64,
    grip: Grip,
}

// ── Property rows (the animatable-property picker) ────────────────────────────

/// One animatable property of the targeted clip: its registry entry plus the
/// [`AnimTarget`] that addresses its keyframe lane.
struct PropRow {
    group: String,
    label: String,
    path: PropPath,
    target: AnimTarget,
    kind: PropValueKind,
    range: Option<(f64, f64)>,
}

/// Flatten a clip's animatable properties into picker rows: the eight
/// `transform.*` fields, then each effect's registry params (01 §6.2). Grade-op
/// automation is authored from the color page (07); the picker scope here is
/// "clip / effect / transform" per 13 §5.
fn build_rows(clip: &Clip) -> Vec<PropRow> {
    let mut rows = Vec::new();
    for e in prop_registry::entries(PropTargetKind::ClipTransform) {
        rows.push(PropRow {
            group: "Transform".to_string(),
            label: pretty_label(e.path.strip_prefix("transform.").unwrap_or(e.path)),
            path: PropPath::new(e.path),
            target: AnimTarget::ClipTransform { clip: clip.id },
            kind: e.kind,
            range: e.range,
        });
    }
    for (i, fx) in clip.effects.iter().enumerate() {
        let group = effect_name(fx.kind).to_string();
        for e in prop_registry::entries(PropTargetKind::Effect(fx.kind)) {
            rows.push(PropRow {
                group: group.clone(),
                label: pretty_label(e.path.strip_prefix("params.").unwrap_or(e.path)),
                path: PropPath::new(e.path),
                target: AnimTarget::ClipEffect {
                    clip: clip.id,
                    effect_index: i,
                },
                kind: e.kind,
                range: e.range,
            });
        }
    }
    rows
}

fn effect_name(kind: photonic_core::timeline::EffectKind) -> &'static str {
    use photonic_core::timeline::EffectKind as K;
    match kind {
        K::Blur => "Blur",
        K::Sharpen => "Sharpen",
        K::Glow => "Glow",
        K::ChromaKey => "Chroma Key",
        K::LumaKey => "Luma Key",
        K::Invert => "Invert",
        K::MaskShapeGen => "Mask Shape",
        _ => "Effect",
    }
}

/// Turn a registry leaf like `scale_x` / `slope[0]` into a display label.
fn pretty_label(leaf: &str) -> String {
    let mut out = String::with_capacity(leaf.len());
    let mut upper = true;
    for c in leaf.chars() {
        match c {
            '_' => {
                out.push(' ');
                upper = true;
            }
            c if upper => {
                out.extend(c.to_uppercase());
                upper = false;
            }
            c => out.push(c),
        }
    }
    out
}

// ── Reading current animated values (pure) ───────────────────────────────────

/// The keyframe lane for `(target, path)` on `clip`, if one exists.
fn track_for<'a>(
    clip: &'a Clip,
    target: &AnimTarget,
    path: &PropPath,
) -> Option<&'a PropertyTrack> {
    match target {
        AnimTarget::ClipTransform { .. } => clip.transform.track(path),
        AnimTarget::ClipEffect { effect_index, .. } => {
            clip.effects.get(*effect_index)?.params.track(path)
        }
        _ => None,
    }
}

/// The static base value behind `(target, path)` — the value used when no
/// keyframe overrides it.
fn base_value(clip: &Clip, target: &AnimTarget, path: &PropPath) -> Option<PropValue> {
    match target {
        AnimTarget::ClipTransform { .. } => {
            let t = &clip.transform.base;
            let v = match path.as_str() {
                "transform.x" => t.x,
                "transform.y" => t.y,
                "transform.scale_x" => t.scale_x,
                "transform.scale_y" => t.scale_y,
                "transform.rotation" => t.rotation,
                "transform.anchor_x" => t.anchor_x,
                "transform.anchor_y" => t.anchor_y,
                "transform.opacity" => t.opacity,
                _ => return None,
            };
            Some(PropValue::Float(v))
        }
        AnimTarget::ClipEffect { effect_index, .. } => clip
            .effects
            .get(*effect_index)?
            .params
            .base
            .get(path.as_str())
            .copied(),
        _ => None,
    }
}

/// The value of `(target, path)` at clip-relative tick `dt` — the keyframed
/// evaluation if a lane exists, else the base (mirrors the engine's read path,
/// 01 §6 / compile.rs).
fn value_at(clip: &Clip, target: &AnimTarget, path: &PropPath, dt: Tick) -> Option<PropValue> {
    let base = base_value(clip, target, path)?;
    match track_for(clip, target, path) {
        Some(tr) if !tr.keyframes.is_empty() => Some(anim::eval(tr, &base, dt)),
        _ => Some(base),
    }
}

/// Extract a scalar from a [`PropValue`] for the curve editor (Float only; other
/// kinds have no 1-D curve).
fn as_f64(v: &PropValue) -> Option<f64> {
    match v {
        PropValue::Float(f) => Some(*f),
        PropValue::Enum(e) => Some(*e as f64),
        _ => None,
    }
}

// ── Coordinate mapping (pure, unit-tested) ───────────────────────────────────

/// Clip-relative tick of the playhead, or `None` when the playhead is outside
/// the clip (so keyframes are only added within the clip's span).
fn playhead_local(playhead: Tick, clip: &Clip) -> Option<Tick> {
    let dt = playhead - clip.start;
    if dt.0 >= 0 && dt <= clip.duration {
        Some(dt)
    } else {
        None
    }
}

/// Map a clip-relative tick to a screen x inside `rect` (x-axis = `[0, dur]`).
fn tick_to_x(at: Tick, dur: Tick, rect: Rect) -> f32 {
    let d = dur.0.max(1) as f32;
    rect.left() + (at.0 as f32 / d) * rect.width()
}

/// Inverse of [`tick_to_x`]: screen x → clip-relative tick, clamped to `[0, dur]`.
fn x_to_tick(x: f32, dur: Tick, rect: Rect) -> Tick {
    let d = dur.0.max(1) as f32;
    let frac = ((x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0);
    Tick((frac * d).round() as i64)
}

/// Map a value to a screen y inside `rect` (value axis inverted: `vmax` at top).
fn value_to_y(v: f64, vmin: f64, vmax: f64, rect: Rect) -> f32 {
    let span = (vmax - vmin).abs().max(1e-9);
    let frac = ((v - vmin) / span).clamp(0.0, 1.0) as f32;
    rect.bottom() - frac * rect.height()
}

/// Inverse of [`value_to_y`].
fn y_to_value(y: f32, vmin: f64, vmax: f64, rect: Rect) -> f64 {
    let frac = ((rect.bottom() - y) / rect.height().max(1.0)).clamp(0.0, 1.0) as f64;
    vmin + frac * (vmax - vmin)
}

/// The value axis range for a Float lane: the registry range if bounded, else an
/// auto-fit over the keyframe values (with padding) so an unbounded property
/// (position, rotation) still frames its curve.
fn value_range(range: Option<(f64, f64)>, track: Option<&PropertyTrack>, base: f64) -> (f64, f64) {
    if let Some((lo, hi)) = range {
        if hi > lo {
            return (lo, hi);
        }
    }
    let mut lo = base;
    let mut hi = base;
    if let Some(tr) = track {
        for k in &tr.keyframes {
            if let Some(v) = as_f64(&k.value) {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
    }
    if (hi - lo).abs() < 1e-6 {
        // Flat curve: give it a symmetric window around the value.
        let pad = if base.abs() > 1.0 { base.abs() } else { 1.0 };
        (base - pad, base + pad)
    } else {
        let pad = (hi - lo) * 0.15;
        (lo - pad, hi + pad)
    }
}

/// Nearest point within `radius` px of `pos`, returning its index.
fn hit_index(points: &[Pos2], pos: Pos2, radius: f32) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (i, p) in points.iter().enumerate() {
        let d = p.distance(pos);
        if d <= radius && best.map(|(_, bd)| d < bd).unwrap_or(true) {
            best = Some((i, d));
        }
    }
    best.map(|(i, _)| i)
}

/// Screen positions of a Bezier segment's two control handles, in the segment's
/// `[k0..k1]` time×value box. Handles are normalized `[0,1]²` (CSS-like), so
/// `out_handle` sits near `k0` and `in_handle` near `k1`.
fn handle_positions(
    k0: (f32, f32),
    k1: (f32, f32),
    out_handle: [f64; 2],
    in_handle: [f64; 2],
) -> (Pos2, Pos2) {
    let lerp = |a: f32, b: f32, t: f64| a + (b - a) * t as f32;
    let out = Pos2::new(
        lerp(k0.0, k1.0, out_handle[0]),
        lerp(k0.1, k1.1, out_handle[1]),
    );
    let inp = Pos2::new(
        lerp(k0.0, k1.0, in_handle[0]),
        lerp(k0.1, k1.1, in_handle[1]),
    );
    (out, inp)
}

/// Recover a normalized handle `[x,y]` from a dragged screen position within the
/// segment box `k0..k1`; x is clamped to `[0,1]` (CSS-timing rule), y is free but
/// bounded so a fat-fingered drag can't fling the curve to infinity.
fn handle_from_pos(pos: Pos2, k0: (f32, f32), k1: (f32, f32)) -> [f64; 2] {
    let dx = (k1.0 - k0.0) as f64;
    let dy = (k1.1 - k0.1) as f64;
    let nx = if dx.abs() < 1e-6 {
        0.0
    } else {
        ((pos.x - k0.0) as f64 / dx).clamp(0.0, 1.0)
    };
    let ny = if dy.abs() < 1e-6 {
        0.0
    } else {
        ((pos.y - k0.1) as f64 / dy).clamp(-1.0, 2.0)
    };
    [nx, ny]
}

/// Snap a clip-relative tick to the frame grid.
fn snap_frame(at: Tick, fps: FrameRate) -> Tick {
    let tpf = fps.ticks_per_frame().0.max(1);
    Tick(((at.0 + tpf / 2) / tpf) * tpf)
}

// ── Ease presets ─────────────────────────────────────────────────────────────

/// The ease presets offered in the toolbar / context menu (13 §5). Bezier
/// presets are the standard CSS timing functions.
const EASE_PRESETS: &[(&str, Interp)] = &[
    ("Hold", Interp::Hold),
    ("Linear", Interp::Linear),
    (
        "Ease In",
        Interp::Bezier {
            out_handle: [0.42, 0.0],
            in_handle: [1.0, 1.0],
        },
    ),
    (
        "Ease Out",
        Interp::Bezier {
            out_handle: [0.0, 0.0],
            in_handle: [0.58, 1.0],
        },
    ),
    (
        "Ease In-Out",
        Interp::Bezier {
            out_handle: [0.42, 0.0],
            in_handle: [0.58, 1.0],
        },
    ),
];

fn interp_short(i: &Interp) -> &'static str {
    match i {
        Interp::Hold => "Hold",
        Interp::Linear => "Linear",
        Interp::Bezier { .. } => "Bezier",
    }
}

// ── Pending edits → CommandHistory ───────────────────────────────────────────

/// An edit the UI decided to make this frame, applied after rendering releases
/// the (read-only) borrow of the cloned clip.
enum KfAction {
    Set {
        target: AnimTarget,
        path: PropPath,
        kf: Keyframe,
    },
    Move {
        target: AnimTarget,
        path: PropPath,
        old_at: Tick,
        kf: Keyframe,
    },
    Remove {
        target: AnimTarget,
        path: PropPath,
        at: Tick,
    },
    SetInterp {
        target: AnimTarget,
        path: PropPath,
        at: Tick,
        interp: Interp,
    },
}

fn commit(
    history: &mut CommandHistory,
    doc: &mut Document,
    cmd: photonic_core::timeline::TimelineCmd,
) {
    history.execute_discrete(Command::Timeline(cmd), doc);
}

/// Apply one collected [`KfAction`] as a single undo step. Reads the current
/// project to build the pure command, then commits — never mutates `doc.timeline`
/// in place.
fn apply_action(doc: &mut Document, history: &mut CommandHistory, action: KfAction) {
    match action {
        KfAction::Set { target, path, kf } => {
            let Some(p) = doc.timeline.as_ref() else {
                return;
            };
            let cmd = ops::set_keyframe(p, target, path, kf);
            commit(history, doc, cmd);
        }
        KfAction::Remove { target, path, at } => {
            let Some(p) = doc.timeline.as_ref() else {
                return;
            };
            if let Ok(cmd) = ops::remove_keyframe(p, target, path, at) {
                commit(history, doc, cmd);
            }
        }
        KfAction::SetInterp {
            target,
            path,
            at,
            interp,
        } => {
            let Some(p) = doc.timeline.as_ref() else {
                return;
            };
            if let Ok(cmd) = ops::set_keyframe_interp(p, target, path, at, interp) {
                commit(history, doc, cmd);
            }
        }
        KfAction::Move {
            target,
            path,
            old_at,
            kf,
        } => {
            if old_at == kf.at {
                apply_action(doc, history, KfAction::Set { target, path, kf });
                return;
            }
            // Move in time = remove the old lane entry + set the new one, as ONE
            // undo step (both built against the pre-move project state).
            let Some(p) = doc.timeline.as_ref() else {
                return;
            };
            let Ok(rm) = ops::remove_keyframe(p, target.clone(), path.clone(), old_at) else {
                return;
            };
            let set = ops::set_keyframe(p, target, path, kf);
            let batch = Command::Batch(vec![Command::Timeline(rm), Command::Timeline(set)]);
            history.execute_discrete(batch, doc);
        }
    }
}

// ── Clip lookup ──────────────────────────────────────────────────────────────

fn find_clip(p: &TimelineProject, id: ClipId) -> Option<&Clip> {
    p.sequences
        .values()
        .flat_map(|s| s.tracks())
        .flat_map(|t| t.clips.iter())
        .find(|c| c.id == id)
}

fn find_clip_fps(p: &TimelineProject, id: ClipId) -> Option<FrameRate> {
    for s in p.sequences.values() {
        if s.tracks().any(|t| t.clips.iter().any(|c| c.id == id)) {
            return Some(s.frame_rate);
        }
    }
    None
}

// ── egui memory helpers ──────────────────────────────────────────────────────

fn load_state(ctx: &egui::Context) -> EditorState {
    ctx.data_mut(|d| d.get_temp::<EditorState>(egui::Id::new(STATE_ID)))
        .unwrap_or_default()
}

fn save_state(ctx: &egui::Context, st: EditorState) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(STATE_ID), st));
}

// ── Public: floating curve editor window ─────────────────────────────────────

/// The floating keyframe / curve editor (04 §4.1). Auto-targets the clip
/// selected in the timeline (opening on selection change); the window's close
/// button clears the target until a different clip is selected. All state lives
/// in egui memory + the passed `target` field, so no new `PhotonicApp` fields are
/// required.
pub(crate) fn draw_window(
    ctx: &egui::Context,
    doc: &mut Document,
    history: &mut CommandHistory,
    target: &mut Option<ClipId>,
    selection: &[ClipId],
    playhead: Tick,
) {
    let mut st = load_state(ctx);

    // Auto-open on a timeline-selection change (select a clip → its keyframes
    // appear; close → stays closed until a different clip is picked).
    let sel = selection.first().copied();
    if !st.seeded {
        st.last_selection = sel;
        st.seeded = true;
        if sel.is_some() {
            *target = sel;
        }
    } else if sel != st.last_selection {
        *target = sel;
        st.last_selection = sel;
    }

    let Some(clip_id) = *target else {
        save_state(ctx, st);
        return;
    };

    // Clone the targeted clip + its frame rate so rendering holds no borrow of
    // `doc` while we later mutate it through history.
    let resolved = doc.timeline.as_ref().and_then(|p| {
        find_clip(p, clip_id)
            .cloned()
            .zip(find_clip_fps(p, clip_id))
    });
    let Some((clip, fps)) = resolved else {
        *target = None;
        save_state(ctx, st);
        return;
    };

    if st.clip != Some(clip_id) {
        st.clip = Some(clip_id);
        st.sel_row = 0;
        st.sel_kf_at = None;
    }

    let rows = build_rows(&clip);
    if st.sel_row >= rows.len() {
        st.sel_row = 0;
    }

    let mut actions: Vec<KfAction> = Vec::new();
    let mut open = true;

    egui::Window::new(format!("{}  Keyframes", ph::CHART_LINE))
        .id(egui::Id::new("photonic_kf_editor_window"))
        .open(&mut open)
        .resizable(true)
        .collapsible(true)
        .default_size(Vec2::new(560.0, 300.0))
        .min_width(360.0)
        .min_height(200.0)
        .show(ctx, |ui| {
            render_editor(ui, &clip, fps, playhead, &rows, &mut st, &mut actions);
        });

    if !open {
        *target = None;
    }
    save_state(ctx, st);

    for a in actions {
        apply_action(doc, history, a);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_editor(
    ui: &mut egui::Ui,
    clip: &Clip,
    fps: FrameRate,
    playhead: Tick,
    rows: &[PropRow],
    st: &mut EditorState,
    actions: &mut Vec<KfAction>,
) {
    // Header: clip name + local playhead readout.
    ui.horizontal(|ui| {
        let name = if clip.name.is_empty() {
            "Clip"
        } else {
            &clip.name
        };
        ui.label(egui::RichText::new(name).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            match playhead_local(playhead, clip) {
                Some(dt) => ui.label(
                    egui::RichText::new(format!("t {:.2}s", dt.0 as f64 / 30_000.0))
                        .monospace()
                        .weak(),
                ),
                None => ui.label(egui::RichText::new("playhead off-clip").weak().small()),
            };
        });
    });
    ui.separator();

    egui::SidePanel::left("kf_prop_picker")
        .resizable(true)
        .default_width(190.0)
        .min_width(140.0)
        .show_inside(ui, |ui| {
            draw_prop_picker(ui, clip, playhead, rows, st, actions);
        });

    egui::CentralPanel::default().show_inside(ui, |ui| {
        draw_curve_area(ui, clip, fps, playhead, rows, st, actions);
    });
}

/// Left column: the animatable-property picker with a keyframe-diamond toggle
/// per row (13 §5.2 — filled when keyframed, hollow otherwise, `primary` ring
/// when interpolating at the current tick).
fn draw_prop_picker(
    ui: &mut egui::Ui,
    clip: &Clip,
    playhead: Tick,
    rows: &[PropRow],
    st: &mut EditorState,
    actions: &mut Vec<KfAction>,
) {
    let local = playhead_local(playhead, clip);
    let accent = ui.visuals().selection.stroke.color;
    let muted = ui.visuals().weak_text_color();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut last_group: Option<&str> = None;
            for (i, row) in rows.iter().enumerate() {
                if last_group != Some(row.group.as_str()) {
                    if last_group.is_some() {
                        ui.add_space(4.0);
                    }
                    ui.label(
                        egui::RichText::new(row.group.to_uppercase())
                            .small()
                            .color(muted),
                    );
                    last_group = Some(row.group.as_str());
                }

                let track = track_for(clip, &row.target, &row.path);
                let has_any = track.map(|t| !t.keyframes.is_empty()).unwrap_or(false);
                let at_dt = local.and_then(|dt| {
                    track.and_then(|t| t.keyframes.iter().find(|k| k.at == dt).map(|_| dt))
                });
                let interpolating = has_any
                    && local
                        .map(|dt| within_track_span(track.unwrap(), dt))
                        .unwrap_or(false);

                ui.horizontal(|ui| {
                    // Keyframe diamond toggle (add/remove at playhead).
                    let (drect, dresp) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::click());
                    paint_diamond(
                        ui.painter(),
                        drect.center(),
                        5.0,
                        has_any,
                        interpolating,
                        accent,
                        muted,
                    );
                    if local.is_some() {
                        dresp.clone().on_hover_text(if at_dt.is_some() {
                            "Remove keyframe at playhead"
                        } else {
                            "Add keyframe at playhead"
                        });
                    }
                    if dresp.clicked() {
                        if let Some(dt) = local {
                            if let Some(at) = at_dt {
                                actions.push(KfAction::Remove {
                                    target: row.target.clone(),
                                    path: row.path.clone(),
                                    at,
                                });
                            } else if let Some(v) = value_at(clip, &row.target, &row.path, dt) {
                                actions.push(KfAction::Set {
                                    target: row.target.clone(),
                                    path: row.path.clone(),
                                    kf: Keyframe::new(dt, v, default_interp(row.kind)),
                                });
                            }
                        }
                    }

                    // Selectable property label.
                    let selected = st.sel_row == i;
                    if ui.selectable_label(selected, &row.label).clicked() {
                        st.sel_row = i;
                        st.sel_kf_at = None;
                    }

                    // Current value readout (right-aligned, mono).
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let text = local
                            .and_then(|dt| value_at(clip, &row.target, &row.path, dt))
                            .map(fmt_value)
                            .unwrap_or_else(|| "—".to_string());
                        ui.label(egui::RichText::new(text).monospace().small().color(muted));
                    });
                });
            }
        });
}

/// Whether `dt` lies within `[first.at, last.at]` — i.e. the property is being
/// actively interpolated (not merely keyframed somewhere).
fn within_track_span(track: &PropertyTrack, dt: Tick) -> bool {
    match (track.keyframes.first(), track.keyframes.last()) {
        (Some(f), Some(l)) => dt >= f.at && dt <= l.at,
        _ => false,
    }
}

fn default_interp(kind: PropValueKind) -> Interp {
    // Non-interpolable kinds always step; seed them Hold so the declared interp
    // matches the eval behaviour (01 §6).
    match kind {
        PropValueKind::Bool | PropValueKind::Enum => Interp::Hold,
        _ => Interp::Linear,
    }
}

fn fmt_value(v: PropValue) -> String {
    match v {
        PropValue::Float(f) => format!("{f:.3}"),
        PropValue::Vec2([x, y]) => format!("{x:.2},{y:.2}"),
        PropValue::Color(c) => format!("#{:02X}{:02X}{:02X}", to_u8(c.r), to_u8(c.g), to_u8(c.b)),
        PropValue::Bool(b) => if b { "on" } else { "off" }.to_string(),
        PropValue::Enum(e) => e.to_string(),
    }
}

fn to_u8(f: f32) -> u8 {
    (f.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Draw a keyframe diamond: filled when the property is keyframed, hollow
/// outline otherwise, with a `primary` ring when interpolating (13 §5.2).
fn paint_diamond(
    painter: &egui::Painter,
    c: Pos2,
    r: f32,
    filled: bool,
    ring: bool,
    accent: Color32,
    muted: Color32,
) {
    let pts = vec![
        Pos2::new(c.x, c.y - r),
        Pos2::new(c.x + r, c.y),
        Pos2::new(c.x, c.y + r),
        Pos2::new(c.x - r, c.y),
    ];
    if filled {
        painter.add(Shape::convex_polygon(pts.clone(), accent, Stroke::NONE));
    } else {
        painter.add(Shape::convex_polygon(
            pts.clone(),
            Color32::TRANSPARENT,
            Stroke::new(1.0, muted),
        ));
    }
    if ring {
        painter.add(Shape::convex_polygon(
            pts,
            Color32::TRANSPARENT,
            Stroke::new(1.5, accent),
        ));
    }
}

/// Right column: the curve editor plot for the selected property + an ease
/// toolbar. Float properties get the full time×value curve with draggable points
/// and Bezier handles; other kinds fall back to a message (their keyframes are
/// still toggleable from the picker diamond).
#[allow(clippy::too_many_arguments)]
fn draw_curve_area(
    ui: &mut egui::Ui,
    clip: &Clip,
    fps: FrameRate,
    playhead: Tick,
    rows: &[PropRow],
    st: &mut EditorState,
    actions: &mut Vec<KfAction>,
) {
    let Some(row) = rows.get(st.sel_row) else {
        ui.weak("No animatable property.");
        return;
    };

    // Ease toolbar — sets the selected keyframe's outgoing interpolation.
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(&row.label).strong());
        ui.separator();
        let sel_at = st.sel_kf_at.map(Tick);
        let cur_interp = sel_at.and_then(|at| {
            track_for(clip, &row.target, &row.path)
                .and_then(|t| t.keyframes.iter().find(|k| k.at == at).map(|k| k.interp))
        });
        ui.add_enabled_ui(sel_at.is_some(), |ui| {
            for (name, interp) in EASE_PRESETS {
                let active = cur_interp
                    .as_ref()
                    .map(|c| interp_short(c) == interp_short(interp))
                    .unwrap_or(false);
                if ui.selectable_label(active, *name).clicked() {
                    if let Some(at) = sel_at {
                        actions.push(KfAction::SetInterp {
                            target: row.target.clone(),
                            path: row.path.clone(),
                            at,
                            interp: *interp,
                        });
                    }
                }
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(sel_at.is_some(), egui::Button::new(ph::TRASH).small())
                .on_hover_text("Delete selected keyframe (Del)")
                .clicked()
            {
                if let Some(at) = sel_at {
                    actions.push(KfAction::Remove {
                        target: row.target.clone(),
                        path: row.path.clone(),
                        at,
                    });
                    st.sel_kf_at = None;
                }
            }
        });
    });

    if !matches!(row.kind, PropValueKind::Float) {
        ui.centered_and_justified(|ui| {
            ui.weak(format!(
                "{:?} keyframes — toggle from the diamond; curve editing is Float-only in v1.",
                row.kind
            ));
        });
        return;
    }

    draw_float_curve(ui, clip, fps, playhead, row, st, actions);
}

/// The Float curve editor: grid, playhead, keyframe diamonds, interpolation
/// polyline, Bezier handles for the selected keyframe, and pointer editing
/// (double-click add / drag move / right-click menu). Drags preview a ghost and
/// commit ONE step on release.
#[allow(clippy::too_many_arguments)]
fn draw_float_curve(
    ui: &mut egui::Ui,
    clip: &Clip,
    fps: FrameRate,
    playhead: Tick,
    row: &PropRow,
    st: &mut EditorState,
    actions: &mut Vec<KfAction>,
) {
    let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    let v = ui.visuals();
    let accent = v.selection.stroke.color;
    let grid = v.widgets.noninteractive.bg_stroke.color;
    let text = v.text_color();
    let muted = v.weak_text_color();

    let track = track_for(clip, &row.target, &row.path);
    let base = base_value(clip, &row.target, &row.path)
        .and_then(|b| as_f64(&b))
        .unwrap_or(0.0);
    let (vmin, vmax) = value_range(row.range, track, base);
    let dur = clip.duration;

    // Backdrop + border.
    painter.rect_filled(rect, Rounding::same(3.0), v.extreme_bg_color);
    painter.rect_stroke(rect, Rounding::same(3.0), Stroke::new(1.0, grid));

    // Horizontal value gridlines + labels (min / mid / max).
    for frac in [0.0f32, 0.5, 1.0] {
        let y = rect.bottom() - frac * rect.height();
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(1.0, grid.gamma_multiply(0.5)),
        );
        let val = vmin + frac as f64 * (vmax - vmin);
        painter.text(
            Pos2::new(rect.left() + 3.0, y),
            Align2::LEFT_CENTER,
            format!("{val:.2}"),
            FontId::monospace(9.0),
            muted,
        );
    }

    // Interpolation polyline (sample eval across the clip span).
    if let Some(tr) = track.filter(|t| !t.keyframes.is_empty()) {
        let base_pv = PropValue::Float(base);
        let steps = (rect.width() as usize).clamp(16, 512);
        let mut poly = Vec::with_capacity(steps + 1);
        for s in 0..=steps {
            let at = Tick((s as i64 * dur.0.max(1)) / steps as i64);
            if let Some(val) = as_f64(&anim::eval(tr, &base_pv, at)) {
                poly.push(Pos2::new(
                    tick_to_x(at, dur, rect),
                    value_to_y(val, vmin, vmax, rect),
                ));
            }
        }
        painter.add(Shape::line(poly, Stroke::new(1.5, accent)));
    }

    // Playhead marker.
    if let Some(dt) = playhead_local(playhead, clip) {
        let x = tick_to_x(dt, dur, rect);
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(1.0, accent.gamma_multiply(0.7)),
        );
    }

    // Keyframe screen points.
    let kfs: Vec<Keyframe> = track.map(|t| t.keyframes.clone()).unwrap_or_default();
    let points: Vec<Pos2> = kfs
        .iter()
        .filter_map(|k| {
            as_f64(&k.value).map(|val| {
                Pos2::new(
                    tick_to_x(k.at, dur, rect),
                    value_to_y(val, vmin, vmax, rect),
                )
            })
        })
        .collect();

    // ── Pointer editing ──────────────────────────────────────────────────────
    let drag_id = egui::Id::new(DRAG_ID);
    let hover = resp.hover_pos();

    // Selected keyframe's Bezier handle screen positions (if applicable).
    let sel_idx = st
        .sel_kf_at
        .and_then(|at| kfs.iter().position(|k| k.at.0 == at));
    let handles = sel_idx.and_then(|i| {
        let k0 = kfs.get(i)?;
        let _next = kfs.get(i + 1)?; // segment exists ⇒ handles are meaningful
        if let Interp::Bezier {
            out_handle,
            in_handle,
        } = k0.interp
        {
            let p0 = (points[i].x, points[i].y);
            let p1 = (points[i + 1].x, points[i + 1].y);
            Some((i, handle_positions(p0, p1, out_handle, in_handle)))
        } else {
            None
        }
    });

    // Begin a drag: prefer a handle grip, else a keyframe point.
    if resp.drag_started() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let mut started = None;
            if let Some((hi, (out_p, in_p))) = handles {
                if out_p.distance(pos) <= 7.0 {
                    started = Some(KfDrag {
                        row: st.sel_row,
                        orig_at: kfs[hi].at.0,
                        grip: Grip::HandleOut,
                    });
                } else if in_p.distance(pos) <= 7.0 {
                    started = Some(KfDrag {
                        row: st.sel_row,
                        orig_at: kfs[hi].at.0,
                        grip: Grip::HandleIn,
                    });
                }
            }
            if started.is_none() {
                if let Some(i) = hit_index(&points, pos, 8.0) {
                    st.sel_kf_at = Some(kfs[i].at.0);
                    started = Some(KfDrag {
                        row: st.sel_row,
                        orig_at: kfs[i].at.0,
                        grip: Grip::Point,
                    });
                }
            }
            if let Some(d) = started {
                ui.data_mut(|m| m.insert_temp(drag_id, d));
            }
        }
    }

    let active_drag = ui.data(|m| m.get_temp::<KfDrag>(drag_id));

    // Preview during drag.
    if resp.dragged() {
        if let (Some(d), Some(pos)) = (active_drag, resp.interact_pointer_pos()) {
            if d.row == st.sel_row {
                match d.grip {
                    Grip::Point => {
                        let mut at = snap_frame(x_to_tick(pos.x, dur, rect), fps);
                        at = Tick(at.0.clamp(0, dur.0));
                        let val = y_to_value(pos.y, vmin, vmax, rect);
                        let gx = tick_to_x(at, dur, rect);
                        let gy = value_to_y(val, vmin, vmax, rect);
                        paint_diamond(&painter, Pos2::new(gx, gy), 6.0, true, true, accent, muted);
                        painter.text(
                            Pos2::new(gx + 8.0, gy - 8.0),
                            Align2::LEFT_BOTTOM,
                            format!("{val:.2}"),
                            FontId::monospace(9.0),
                            text,
                        );
                    }
                    Grip::HandleOut | Grip::HandleIn => {
                        if let Some(i) = kfs.iter().position(|k| k.at.0 == d.orig_at) {
                            if let (Some(&p0), Some(&p1)) = (points.get(i), points.get(i + 1)) {
                                let h = handle_from_pos(pos, (p0.x, p0.y), (p1.x, p1.y));
                                let hp = Pos2::new(
                                    p0.x + (p1.x - p0.x) * h[0] as f32,
                                    p0.y + (p1.y - p0.y) * h[1] as f32,
                                );
                                let anchor = if d.grip == Grip::HandleOut { p0 } else { p1 };
                                painter.line_segment(
                                    [anchor, hp],
                                    Stroke::new(1.0, accent.gamma_multiply(0.7)),
                                );
                                painter.circle_filled(hp, 3.5, accent);
                            }
                        }
                    }
                }
            }
        }
    }

    // Commit on release.
    if resp.drag_stopped() {
        let d = ui.data(|m| m.get_temp::<KfDrag>(drag_id));
        ui.data_mut(|m| m.remove::<KfDrag>(drag_id));
        if let (Some(d), Some(pos)) = (d, hover.or(resp.interact_pointer_pos())) {
            match d.grip {
                Grip::Point => {
                    if let Some(k) = kfs.iter().find(|k| k.at.0 == d.orig_at) {
                        let mut at = snap_frame(x_to_tick(pos.x, dur, rect), fps);
                        at = Tick(at.0.clamp(0, dur.0));
                        let val = y_to_value(pos.y, vmin, vmax, rect);
                        actions.push(KfAction::Move {
                            target: row.target.clone(),
                            path: row.path.clone(),
                            old_at: Tick(d.orig_at),
                            kf: Keyframe::new(at, PropValue::Float(val), k.interp),
                        });
                        st.sel_kf_at = Some(at.0);
                    }
                }
                Grip::HandleOut | Grip::HandleIn => {
                    if let Some(i) = kfs.iter().position(|k| k.at.0 == d.orig_at) {
                        if let (Some(&p0), Some(&p1)) = (points.get(i), points.get(i + 1)) {
                            let (mut out_h, mut in_h) = match kfs[i].interp {
                                Interp::Bezier {
                                    out_handle,
                                    in_handle,
                                } => (out_handle, in_handle),
                                _ => ([0.0, 0.0], [1.0, 1.0]),
                            };
                            let h = handle_from_pos(pos, (p0.x, p0.y), (p1.x, p1.y));
                            if d.grip == Grip::HandleOut {
                                out_h = h;
                            } else {
                                in_h = h;
                            }
                            actions.push(KfAction::SetInterp {
                                target: row.target.clone(),
                                path: row.path.clone(),
                                at: kfs[i].at,
                                interp: Interp::Bezier {
                                    out_handle: out_h,
                                    in_handle: in_h,
                                },
                            });
                        }
                    }
                }
            }
        }
    }

    // Static keyframe diamonds + Bezier handles (skip the point being dragged).
    let dragging_at = active_drag
        .filter(|d| d.grip == Grip::Point)
        .map(|d| d.orig_at);
    for (i, p) in points.iter().enumerate() {
        if Some(kfs[i].at.0) == dragging_at {
            continue;
        }
        let selected = st.sel_kf_at == Some(kfs[i].at.0);
        paint_diamond(
            &painter,
            *p,
            if selected { 6.0 } else { 4.5 },
            true,
            selected,
            accent,
            muted,
        );
    }
    if active_drag.is_none() {
        if let Some((i, (out_p, in_p))) = handles {
            let p0 = points[i];
            let p1 = points[i + 1];
            painter.line_segment([p0, out_p], Stroke::new(1.0, accent.gamma_multiply(0.6)));
            painter.line_segment([p1, in_p], Stroke::new(1.0, accent.gamma_multiply(0.6)));
            painter.circle_filled(out_p, 3.0, accent);
            painter.circle_filled(in_p, 3.0, accent);
        }
    }

    // Double-click empty space → add a keyframe at the clicked time+value.
    if resp.double_clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            if hit_index(&points, pos, 8.0).is_none() {
                let at = Tick(
                    snap_frame(x_to_tick(pos.x, dur, rect), fps)
                        .0
                        .clamp(0, dur.0),
                );
                let val = y_to_value(pos.y, vmin, vmax, rect);
                actions.push(KfAction::Set {
                    target: row.target.clone(),
                    path: row.path.clone(),
                    kf: Keyframe::new(at, PropValue::Float(val), Interp::Linear),
                });
                st.sel_kf_at = Some(at.0);
            }
        }
    }

    // Single click on empty space clears the keyframe selection.
    if resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            match hit_index(&points, pos, 8.0) {
                Some(i) => st.sel_kf_at = Some(kfs[i].at.0),
                None => st.sel_kf_at = None,
            }
        }
    }

    // Right-click a keyframe → easing / delete menu.
    let menu_idx = hover.and_then(|pos| hit_index(&points, pos, 8.0));
    if let Some(mi) = menu_idx {
        let at = kfs[mi].at;
        resp.context_menu(|ui| {
            ui.label(egui::RichText::new("Easing").small().weak());
            for (name, interp) in EASE_PRESETS {
                if ui.button(*name).clicked() {
                    actions.push(KfAction::SetInterp {
                        target: row.target.clone(),
                        path: row.path.clone(),
                        at,
                        interp: *interp,
                    });
                    ui.close_menu();
                }
            }
            ui.separator();
            if ui
                .button(format!("{}  Delete keyframe", ph::TRASH))
                .clicked()
            {
                actions.push(KfAction::Remove {
                    target: row.target.clone(),
                    path: row.path.clone(),
                    at,
                });
                st.sel_kf_at = None;
                ui.close_menu();
            }
        });
    }

    // Delete key removes the selected keyframe (keyboard path, 13 §16).
    if resp.hovered()
        && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
    {
        if let Some(at) = st.sel_kf_at {
            actions.push(KfAction::Remove {
                target: row.target.clone(),
                path: row.path.clone(),
                at: Tick(at),
            });
            st.sel_kf_at = None;
        }
    }
}

// ── Public: timeline automation-lane indicator ───────────────────────────────

/// Paint a clip's keyframe positions as small diamonds along the top edge of its
/// timeline body (04 §4.1 automation-lane integration). Read-only: a glanceable
/// cue that a clip is animated; editing happens in [`draw_window`]. No-op for a
/// static clip.
pub(crate) fn paint_clip_automation(
    painter: &egui::Painter,
    view: &TimelineView,
    lane_left: f32,
    clip: &Clip,
    clip_rect: Rect,
    accent: Color32,
) {
    // Unique clip-relative keyframe ticks across transform + effect lanes.
    let mut ats: Vec<i64> = Vec::new();
    for tr in &clip.transform.tracks {
        for k in &tr.keyframes {
            ats.push(k.at.0);
        }
    }
    for fx in &clip.effects {
        for tr in &fx.params.tracks {
            for k in &tr.keyframes {
                ats.push(k.at.0);
            }
        }
    }
    if ats.is_empty() {
        return;
    }
    ats.sort_unstable();
    ats.dedup();

    let y = clip_rect.top() + 4.0;
    let r = 2.5;
    let mut shapes: Vec<Shape> = Vec::with_capacity(ats.len());
    for at in ats {
        let x = view.tick_to_x(clip.start + Tick(at), lane_left);
        if x < clip_rect.left() + r || x > clip_rect.right() - r {
            continue;
        }
        shapes.push(Shape::convex_polygon(
            vec![
                Pos2::new(x, y - r),
                Pos2::new(x + r, y),
                Pos2::new(x, y + r),
                Pos2::new(x - r, y),
            ],
            accent,
            Stroke::NONE,
        ));
    }
    painter.extend(shapes);
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use photonic_core::timeline::{ClipSource, EffectKind};

    fn plot() -> Rect {
        Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(200.0, 100.0))
    }

    #[test]
    fn tick_x_roundtrips_within_a_frame() {
        let rect = plot();
        let dur = Tick(3000);
        for at in [0i64, 750, 1500, 3000] {
            let x = tick_to_x(Tick(at), dur, rect);
            let back = x_to_tick(x, dur, rect);
            assert!((back.0 - at).abs() <= 1, "at={at} back={}", back.0);
        }
    }

    #[test]
    fn value_y_roundtrips() {
        let rect = plot();
        for v in [-1.0f64, 0.0, 0.5, 1.0] {
            let y = value_to_y(v, -1.0, 1.0, rect);
            let back = y_to_value(y, -1.0, 1.0, rect);
            assert!((back - v).abs() < 1e-3, "v={v} back={back}");
        }
    }

    #[test]
    fn value_to_y_inverts_axis() {
        let rect = plot();
        // Max value sits at the top (smaller y), min at the bottom.
        assert!(value_to_y(1.0, 0.0, 1.0, rect) < value_to_y(0.0, 0.0, 1.0, rect));
        assert_eq!(value_to_y(1.0, 0.0, 1.0, rect), rect.top());
        assert_eq!(value_to_y(0.0, 0.0, 1.0, rect), rect.bottom());
    }

    #[test]
    fn x_to_tick_clamps_to_span() {
        let rect = plot();
        let dur = Tick(1000);
        assert_eq!(x_to_tick(-50.0, dur, rect).0, 0);
        assert_eq!(x_to_tick(9999.0, dur, rect).0, 1000);
    }

    #[test]
    fn value_range_uses_registry_bounds_when_present() {
        let (lo, hi) = value_range(Some((0.0, 1.0)), None, 0.5);
        assert_eq!((lo, hi), (0.0, 1.0));
    }

    #[test]
    fn value_range_autofits_unbounded_props() {
        let mut tr = PropertyTrack::new("transform.x");
        tr.keyframes.push(Keyframe::new(
            Tick(0),
            PropValue::Float(-10.0),
            Interp::Linear,
        ));
        tr.keyframes.push(Keyframe::new(
            Tick(100),
            PropValue::Float(30.0),
            Interp::Linear,
        ));
        let (lo, hi) = value_range(None, Some(&tr), 0.0);
        assert!(lo < -10.0 && hi > 30.0, "range=({lo},{hi})");
    }

    #[test]
    fn value_range_flat_curve_pads_symmetrically() {
        let (lo, hi) = value_range(None, None, 5.0);
        assert!(lo < 5.0 && hi > 5.0);
        assert!(
            (5.0 - lo - (hi - 5.0)).abs() < 1e-6,
            "symmetric around base"
        );
    }

    #[test]
    fn hit_index_picks_nearest_within_radius() {
        let pts = [
            Pos2::new(0.0, 0.0),
            Pos2::new(50.0, 0.0),
            Pos2::new(100.0, 0.0),
        ];
        assert_eq!(hit_index(&pts, Pos2::new(48.0, 2.0), 8.0), Some(1));
        assert_eq!(hit_index(&pts, Pos2::new(25.0, 0.0), 8.0), None);
    }

    #[test]
    fn playhead_local_bounds() {
        let mut clip = Clip::new(ClipSource::Adjustment, Tick(1000), Tick(500));
        clip.start = Tick(1000);
        assert_eq!(playhead_local(Tick(1000), &clip), Some(Tick(0)));
        assert_eq!(playhead_local(Tick(1250), &clip), Some(Tick(250)));
        assert_eq!(playhead_local(Tick(1500), &clip), Some(Tick(500)));
        assert_eq!(playhead_local(Tick(999), &clip), None);
        assert_eq!(playhead_local(Tick(1501), &clip), None);
    }

    #[test]
    fn snap_frame_quantizes_to_grid() {
        let fps = FrameRate::FPS_30; // 1000 ticks/frame at 30_000 tps.
        let tpf = fps.ticks_per_frame().0;
        let snapped = snap_frame(Tick(tpf + tpf / 3), fps);
        assert_eq!(snapped.0 % tpf, 0);
        assert_eq!(snapped.0, tpf, "rounds down toward nearest frame");
    }

    #[test]
    fn handle_positions_and_inverse_agree() {
        let k0 = (0.0f32, 100.0);
        let k1 = (200.0f32, 0.0);
        let (out_p, _in_p) = handle_positions(k0, k1, [0.5, 0.0], [1.0, 1.0]);
        let recovered = handle_from_pos(out_p, k0, k1);
        assert!((recovered[0] - 0.5).abs() < 1e-3, "x={}", recovered[0]);
    }

    #[test]
    fn build_rows_covers_transform_and_effects() {
        let mut clip = Clip::new(
            ClipSource::Vector {
                asset: photonic_core::timeline::AssetId::new(),
            },
            Tick(0),
            Tick(1000),
        );
        clip.effects
            .push(photonic_core::timeline::ClipEffect::new(EffectKind::Blur));
        let rows = build_rows(&clip);
        // 8 transform props + Blur's radius.
        assert!(rows.iter().any(|r| r.path.as_str() == "transform.x"));
        assert!(rows.iter().any(|r| r.path.as_str() == "transform.opacity"));
        assert!(rows.iter().any(|r| matches!(
            r.target,
            AnimTarget::ClipEffect {
                effect_index: 0,
                ..
            }
        ) && r.path.as_str() == "params.radius"));
    }

    #[test]
    fn value_at_evaluates_keyframes() {
        let mut clip = Clip::new(ClipSource::Adjustment, Tick(0), Tick(1000));
        let path = PropPath::new("transform.x");
        let tr = clip.transform.track_mut(&path);
        tr.insert_keyframe(Keyframe::new(
            Tick(0),
            PropValue::Float(0.0),
            Interp::Linear,
        ));
        tr.insert_keyframe(Keyframe::new(
            Tick(1000),
            PropValue::Float(100.0),
            Interp::Linear,
        ));
        let target = AnimTarget::ClipTransform { clip: clip.id };
        let v = value_at(&clip, &target, &path, Tick(500)).unwrap();
        assert_eq!(as_f64(&v), Some(50.0));
    }

    #[test]
    fn pretty_label_titlecases() {
        assert_eq!(pretty_label("scale_x"), "Scale X");
        assert_eq!(pretty_label("opacity"), "Opacity");
    }
}
