//! Track-header column: enable/lock toggles, inline rename, height-drag, and the
//! Add/Remove-track controls (04 §2.3 table, §2.6; 13 §1.1).
//!
//! All undoable mutations route through `ops_bridge`; the one direct write is the
//! height-drag (`height_px` is a persisted-but-non-undoable UI field, 04 §2.3).

use super::ops_bridge;
use egui_phosphor::regular as ph;
use photonic_core::document::Document;
use photonic_core::history::{Command, CommandHistory};
use photonic_core::timeline::{ops, Sequence, SequenceId, TrackId, TrackKind, TrackSettings};

/// A laid-out track row (video lanes first, then audio), shared by the header
/// column and the clip-lane area so the two stay vertically aligned.
#[derive(Clone, Copy)]
pub(crate) struct TrackRow {
    pub id: TrackId,
    pub kind: TrackKind,
    pub height: f32,
    /// Locked tracks reject all clip edits (14-nle-parity QW-2); the clip-lane
    /// hit-testing and painting both key off this.
    pub locked: bool,
    /// Index within its own lane group (for reorder bounds).
    pub index_in_kind: usize,
    pub count_in_kind: usize,
}

/// Row layout for a sequence: video tracks top-to-bottom, then audio.
pub(crate) fn track_rows(seq: &Sequence) -> Vec<TrackRow> {
    let mut rows = Vec::new();
    let vc = seq.video_tracks.len();
    for (i, t) in seq.video_tracks.iter().enumerate() {
        rows.push(TrackRow {
            id: t.id,
            kind: TrackKind::Video,
            height: t.height_px,
            locked: t.locked,
            index_in_kind: i,
            count_in_kind: vc,
        });
    }
    let ac = seq.audio_tracks.len();
    for (i, t) in seq.audio_tracks.iter().enumerate() {
        rows.push(TrackRow {
            id: t.id,
            kind: TrackKind::Audio,
            height: t.height_px,
            locked: t.locked,
            index_in_kind: i,
            count_in_kind: ac,
        });
    }
    rows
}

/// egui temp key for the active inline-rename buffer `(TrackId, String)`.
fn rename_id() -> egui::Id {
    egui::Id::new("timeline_track_rename")
}

/// Draw one track header inside `rect`. Applies toggles/rename/remove/move via
/// `ops_bridge`; height-drag writes `height_px` directly (non-undoable).
pub(crate) fn draw_header(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    doc: &mut Document,
    history: &mut CommandHistory,
    seq_id: SequenceId,
    row: TrackRow,
) {
    let (name, enabled, locked, solo, sync_lock) = {
        let Some(t) = doc
            .timeline
            .as_ref()
            .and_then(|p| p.sequences.get(&seq_id))
            .and_then(|s| s.track(row.id))
        else {
            return;
        };
        let solo = t.audio.as_ref().is_some_and(|a| a.solo);
        (t.name.clone(), t.enabled, t.locked, solo, t.sync_lock)
    };

    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    painter.rect_filled(rect, 0.0, visuals.panel_fill);
    painter.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        egui::Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color),
    );

    let pad = 4.0;
    let btn = 18.0;
    let top = rect.top() + pad;

    // Enable/hide (video) or mute (audio) toggle.
    let enable_rect =
        egui::Rect::from_min_size(egui::pos2(rect.left() + pad, top), egui::vec2(btn, btn));
    let enable_glyph = match (row.kind, enabled) {
        (TrackKind::Video, true) => "◉",
        (TrackKind::Video, false) => "◌",
        (TrackKind::Audio, true) => "♪",
        (TrackKind::Audio, false) => "×",
    };
    let enable_tip = match row.kind {
        TrackKind::Video => "Show / hide track",
        TrackKind::Audio => "Mute / unmute track",
    };
    if ui
        .put(enable_rect, egui::Button::new(enable_glyph).small())
        .on_hover_text(enable_tip)
        .clicked()
    {
        ops_bridge::toggle_enabled(doc, history, seq_id, row.id);
    }

    // Solo toggle (audio tracks only — 14-nle-parity QW-6). Sits next to
    // mute, matching the M/S pairing every DAW/NLE audio header uses; video
    // tracks have no solo concept and keep the original 2-button layout.
    let mut next_x = enable_rect.right() + 2.0;
    if row.kind == TrackKind::Audio {
        let solo_rect = egui::Rect::from_min_size(egui::pos2(next_x, top), egui::vec2(btn, btn));
        if ui
            .put(solo_rect, egui::SelectableLabel::new(solo, "S"))
            .on_hover_text("Solo (solo-safe)")
            .clicked()
        {
            toggle_solo(doc, history, seq_id, row.id);
        }
        next_x = solo_rect.right() + 2.0;
    }

    // Lock toggle.
    let lock_rect = egui::Rect::from_min_size(egui::pos2(next_x, top), egui::vec2(btn, btn));
    if ui
        .put(
            lock_rect,
            egui::Button::new(if locked { "L" } else { "·" }).small(),
        )
        .on_hover_text("Lock / unlock track")
        .clicked()
    {
        ops_bridge::toggle_locked(doc, history, seq_id, row.id);
    }

    // Sync-lock toggle (14-nle-parity M-9): ripple/insert edits are meant to
    // shift every sync-locked track together (the ripple-propagation itself
    // is a reported seam — see `toggle_sync_lock`'s doc). Mirrors the lock
    // button's "·" (off) / glyph (on) language.
    let sync_rect = egui::Rect::from_min_size(
        egui::pos2(lock_rect.right() + 2.0, top),
        egui::vec2(btn, btn),
    );
    if ui
        .put(
            sync_rect,
            egui::Button::new(if sync_lock {
                ph::ARROWS_CLOCKWISE
            } else {
                "·"
            })
            .small(),
        )
        .on_hover_text("Sync lock — ripple/insert edits shift sync-locked tracks together")
        .clicked()
    {
        toggle_sync_lock(doc, history, seq_id, row.id);
    }

    // Track display (wrench) menu (14-nle-parity M-10): per-track height
    // presets. Right-aligned so it never collides with the name label.
    let wrench_rect = egui::Rect::from_min_size(
        egui::pos2(rect.right() - pad - btn, top),
        egui::vec2(btn, btn),
    );
    let wrench_resp = ui
        .put(wrench_rect, egui::Button::new(ph::WRENCH).small())
        .on_hover_text("Track display");
    let wrench_popup_id = wrench_resp.id.with("track_display_popup");
    if wrench_resp.clicked() {
        ui.memory_mut(|m| m.toggle_popup(wrench_popup_id));
    }
    egui::popup::popup_below_widget(
        ui,
        wrench_popup_id,
        &wrench_resp,
        egui::PopupCloseBehavior::CloseOnClick,
        |ui| {
            ui.set_min_width(140.0);
            track_display_menu(ui, doc, seq_id, row);
        },
    );

    // Name label / inline rename.
    let name_rect = egui::Rect::from_min_max(
        egui::pos2(sync_rect.right() + 4.0, top),
        egui::pos2(wrench_rect.left() - 4.0, top + btn),
    );
    let editing = ui
        .data(|d| d.get_temp::<(TrackId, String)>(rename_id()))
        .filter(|(tid, _)| *tid == row.id);
    if let Some((_, mut buf)) = editing {
        let resp = ui.put(name_rect, egui::TextEdit::singleline(&mut buf));
        resp.request_focus();
        if resp.lost_focus() {
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) && !buf.trim().is_empty() {
                ops_bridge::rename_track(doc, history, seq_id, row.id, buf.trim().to_string());
            }
            ui.data_mut(|d| d.remove::<(TrackId, String)>(rename_id()));
        } else {
            ui.data_mut(|d| d.insert_temp(rename_id(), (row.id, buf)));
        }
    } else {
        let label = ui.put(
            name_rect,
            egui::Label::new(egui::RichText::new(&name).color(if locked {
                ui.visuals().weak_text_color()
            } else {
                ui.visuals().text_color()
            }))
            .truncate()
            .sense(egui::Sense::click()),
        );
        if label.double_clicked() {
            ui.data_mut(|d| d.insert_temp(rename_id(), (row.id, name.clone())));
        }
        label.context_menu(|ui| header_menu(ui, doc, history, seq_id, row));
    }

    // Height-drag handle: bottom 5px of the header, mirrored across the lane.
    let handle = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.bottom() - 5.0),
        rect.right_bottom(),
    );
    let hr = ui.interact(
        handle,
        ui.id().with(("track_h", row.id)),
        egui::Sense::drag(),
    );
    if hr.hovered() || hr.dragged() {
        ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::ResizeVertical);
    }
    if hr.dragged() {
        let dy = hr.drag_delta().y;
        ops_bridge::set_track_height(doc, seq_id, row.id, row.height + dy);
    }
}

fn header_menu(
    ui: &mut egui::Ui,
    doc: &mut Document,
    history: &mut CommandHistory,
    seq_id: SequenceId,
    row: TrackRow,
) {
    if ui.button("Rename").clicked() {
        let name = doc
            .timeline
            .as_ref()
            .and_then(|p| p.sequences.get(&seq_id))
            .and_then(|s| s.track(row.id))
            .map(|t| t.name.clone())
            .unwrap_or_default();
        ui.data_mut(|d| d.insert_temp(rename_id(), (row.id, name)));
        ui.close_menu();
    }
    if row.index_in_kind > 0 && ui.button("Move up").clicked() {
        ops_bridge::move_track(doc, history, seq_id, row.id, row.index_in_kind - 1);
        ui.close_menu();
    }
    if row.index_in_kind + 1 < row.count_in_kind && ui.button("Move down").clicked() {
        ops_bridge::move_track(doc, history, seq_id, row.id, row.index_in_kind + 1);
        ui.close_menu();
    }
    ui.separator();
    if ui.button("Remove track").clicked() {
        ops_bridge::remove_track(doc, history, seq_id, row.id);
        ui.close_menu();
    }
}

/// Per-track height presets shown by the wrench popup (14-nle-parity M-10)
/// `(display name, height_px)`. Bounds match `ops_bridge::set_track_height`'s
/// own clamp (`28.0..=240.0`).
const TRACK_HEIGHT_PRESETS: &[(&str, f32)] = &[
    ("Small", 36.0),
    ("Medium", 64.0),
    ("Large", 120.0),
    ("Extra large", 200.0),
];

/// Track display (wrench) menu contents (14-nle-parity M-10): height
/// presets, applied through the same non-undoable `set_track_height` path
/// the drag handle uses (04 §2.3's sanctioned direct-write exception).
///
/// A full "toggle thumbnails/waveforms/track-name display" wrench menu per
/// M-10's spec text would need a per-track visibility field on `Track`
/// (`photonic-core/src/timeline/sequence.rs`) — out of this story's
/// territory, so scoped here to what's achievable against the existing
/// model: height presets. Reported as a seam for a follow-up story.
fn track_display_menu(ui: &mut egui::Ui, doc: &mut Document, seq_id: SequenceId, row: TrackRow) {
    ui.label(egui::RichText::new("Track height").weak().small());
    for (name, h) in TRACK_HEIGHT_PRESETS {
        if ui
            .selectable_label((row.height - h).abs() < 1.0, *name)
            .clicked()
        {
            ops_bridge::set_track_height(doc, seq_id, row.id, *h);
        }
    }
}

/// Add-Video / Add-Audio track controls, drawn at the bottom of the header
/// column (04 §2.6). Returns the height consumed.
pub(crate) fn draw_add_controls(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    doc: &mut Document,
    history: &mut CommandHistory,
    seq_id: SequenceId,
) {
    let inner = rect.shrink(4.0);
    let half = (inner.width() - 4.0) * 0.5;
    let v_rect = egui::Rect::from_min_size(inner.min, egui::vec2(half, inner.height()));
    let a_rect = egui::Rect::from_min_size(
        egui::pos2(v_rect.right() + 4.0, inner.top()),
        egui::vec2(half, inner.height()),
    );
    if ui
        .put(v_rect, egui::Button::new("+ Video").small())
        .on_hover_text("Add a video track")
        .clicked()
    {
        ops_bridge::add_track(doc, history, seq_id, TrackKind::Video);
    }
    if ui
        .put(a_rect, egui::Button::new("+ Audio").small())
        .on_hover_text("Add an audio track")
        .clicked()
    {
        ops_bridge::add_track(doc, history, seq_id, TrackKind::Audio);
    }
}

/// Flip an audio track's `TrackAudio::solo` flag (14-nle-parity QW-6). The
/// solo-safe *mixing* resolution already lives in the audio mixer drawer
/// (`panels/video/audio_mixer.rs::resolve_audible`); this control just
/// exposes the same `TrackAudio.solo` bit from the timeline header so both
/// surfaces read/write one piece of state.
///
/// Deliberately mirrors `ops_bridge::set_track_settings`'s
/// snapshot→edit→`ops::set_track_prop`→`history.execute_discrete` pattern
/// rather than calling into `ops_bridge.rs` — this story's territory is
/// `{mod.rs, clips.rs, tracks.rs, interact.rs}` only, and `set_track_settings`
/// is private to `ops_bridge.rs`. No-op if the track has no `TrackAudio`
/// (i.e. isn't an audio track).
fn toggle_solo(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq_id: SequenceId,
    track: TrackId,
) {
    let Some(p) = doc.timeline.as_ref() else {
        return;
    };
    let Some(t) = p.sequences.get(&seq_id).and_then(|s| s.track(track)) else {
        return;
    };
    let mut settings = TrackSettings::of(t);
    let Some(audio) = settings.audio.as_mut() else {
        return;
    };
    audio.solo = !audio.solo;
    if let Ok(cmd) = ops::set_track_prop(p, seq_id, track, settings) {
        history.execute_discrete(Command::Timeline(cmd), doc);
    }
}

/// Flip a track's `Track::sync_lock` bit (14-nle-parity M-9, the header-toggle
/// half). `ops::toggle_sync_lock` is already a complete, pre-built op (unlike
/// solo above, it needs no local snapshot/edit dance), so this is a thin
/// call — but still bypasses `ops_bridge.rs` directly for the same territory
/// reason `toggle_solo` documents above: no `pub(crate) fn toggle_sync_lock`
/// wrapper exists there, and this story's territory doesn't reach that file.
///
/// **Reported seam:** the ripple-propagation this toggle is *meant* to
/// drive — "ripple/insert edits shift every sync-locked track together" (14
/// §M-9) — does not exist yet. `ops_bridge::ripple_trim`/`ripple_delete`
/// only ever shift the target clip's own track (`ops_bridge.rs`'s
/// `expand_link_group_*` module note documents the same gap for link
/// groups). Wiring that needs a change to those functions in `ops_bridge.rs`,
/// out of this story's territory (`clips.rs`/`mod.rs`/`tracks.rs` only) — a
/// follow-up story should thread `sync_lock` through `ripple_trim`/
/// `ripple_delete`/the future insert/overwrite ops the same way link groups
/// are expanded today.
fn toggle_sync_lock(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq_id: SequenceId,
    track: TrackId,
) {
    let Some(p) = doc.timeline.as_ref() else {
        return;
    };
    if let Ok(cmd) = ops::toggle_sync_lock(p, seq_id, track) {
        history.execute_discrete(Command::Timeline(cmd), doc);
    }
}
