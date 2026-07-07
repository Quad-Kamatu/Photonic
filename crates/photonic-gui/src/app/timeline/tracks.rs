//! Track-header column: enable/lock toggles, inline rename, height-drag, and the
//! Add/Remove-track controls (04 §2.3 table, §2.6; 13 §1.1).
//!
//! All undoable mutations route through `ops_bridge`; the one direct write is the
//! height-drag (`height_px` is a persisted-but-non-undoable UI field, 04 §2.3).

use super::ops_bridge;
use photonic_core::document::Document;
use photonic_core::history::CommandHistory;
use photonic_core::timeline::{Sequence, SequenceId, TrackId, TrackKind};

/// A laid-out track row (video lanes first, then audio), shared by the header
/// column and the clip-lane area so the two stay vertically aligned.
#[derive(Clone, Copy)]
pub(crate) struct TrackRow {
    pub id: TrackId,
    pub kind: TrackKind,
    pub height: f32,
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
    let (name, enabled, locked) = {
        let Some(t) = doc
            .timeline
            .as_ref()
            .and_then(|p| p.sequences.get(&seq_id))
            .and_then(|s| s.track(row.id))
        else {
            return;
        };
        (t.name.clone(), t.enabled, t.locked)
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

    // Lock toggle.
    let lock_rect = egui::Rect::from_min_size(
        egui::pos2(enable_rect.right() + 2.0, top),
        egui::vec2(btn, btn),
    );
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

    // Name label / inline rename.
    let name_rect = egui::Rect::from_min_max(
        egui::pos2(lock_rect.right() + 4.0, top),
        egui::pos2(rect.right() - pad, top + btn),
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
