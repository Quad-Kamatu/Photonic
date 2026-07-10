//! Bottom timeline panel (video-editor-module `04-ui-mode-timeline.md` §2).
//!
//! Module family mirroring the existing `panels/` split-by-concern pattern:
//! `layout` (zoom/scroll mapping), `ruler` (time ruler + playhead + markers),
//! `tracks` (header column), `clips` (culled, batched lane painting), `interact`
//! (pure hit-testing/snapping + drag state), and `ops_bridge` (the sole
//! intent→`ops`→`CommandHistory` sink). Every mutation flows through
//! `ops_bridge`; the panel itself never touches `doc.timeline` directly (the one
//! sanctioned exception, `height_px`, lives in `ops_bridge::set_track_height`).

pub mod clips;
pub mod interact;
pub mod layout;
pub mod ops_bridge;
pub mod ruler;
pub mod tracks;

pub use layout::TimelineView;

use super::PhotonicApp;
use interact::{DragKind, DragState, Marquee};
use layout::{EDGE_ZONE_PX, RULER_HEIGHT_PX, SNAP_THRESHOLD_PX, ZOOM_STEP};
use photonic_core::document::Document;
use photonic_core::history::CommandHistory;
use photonic_core::timeline::{
    ClipId, ClipTiming, FrameRate, Sequence, SequenceId, Tick, TrackId, TrackKind,
};

const TOOLBAR_H: f32 = 24.0;
const DRAG_ID: &str = "timeline_drag_state";
const MARQUEE_ID: &str = "timeline_marquee";

impl PhotonicApp {
    /// The bottom timeline panel's entry point (04 §1.1/§2). Registered by
    /// `app/mod.rs` as a `TopBottomPanel::bottom("timeline")` gated on
    /// `self.mode == AppMode::Video`.
    pub(crate) fn draw_timeline_panel(
        &mut self,
        ui: &mut egui::Ui,
        doc: &mut Document,
        history: &mut CommandHistory,
    ) {
        // Invariant: video mode ⇒ a project exists (04 §1.3). The first action on
        // a document with no project creates it lazily below.
        let frame_rate = active_frame_rate(doc);

        let has_sequence = doc
            .timeline
            .as_ref()
            .and_then(|p| p.active_sequence)
            .and_then(|id| doc.timeline.as_ref().and_then(|p| p.sequences.get(&id)))
            .is_some();
        if !has_sequence {
            draw_empty_affordance(ui, doc, history, frame_rate);
            return;
        }
        let seq_id = doc.timeline.as_ref().unwrap().active_sequence.unwrap();

        // Pull session state into locals so helper fns don't fight the `&mut self`
        // borrow; written back at the end.
        let mut view = self.timeline_view;
        let mut playhead = self.playhead;
        let mut selection = std::mem::take(&mut self.timeline_selection);
        let mut snap = self.timeline_snap_enabled;

        let full = ui.max_rect();
        let toolbar_rect = egui::Rect::from_min_size(full.min, egui::vec2(full.width(), TOOLBAR_H));
        draw_mini_toolbar(ui, toolbar_rect, doc, seq_id, &mut view, &mut snap);

        let below =
            egui::Rect::from_min_max(egui::pos2(full.left(), toolbar_rect.bottom()), full.max);
        let header_w = view.header_width_px;
        let header_col = egui::Rect::from_min_max(
            below.min,
            egui::pos2(below.left() + header_w, below.bottom()),
        );
        let lane_col =
            egui::Rect::from_min_max(egui::pos2(header_col.right(), below.top()), below.max);
        let lane_left = lane_col.left();
        let ruler_rect =
            egui::Rect::from_min_size(lane_col.min, egui::vec2(lane_col.width(), RULER_HEIGHT_PX));
        let lanes_rect = egui::Rect::from_min_max(
            egui::pos2(lane_col.left(), ruler_rect.bottom()),
            lane_col.max,
        );
        view.last_lane_width_px = lanes_rect.width();

        // ── Column splitter (header width) ──────────────────────────────────
        let splitter = egui::Rect::from_min_max(
            egui::pos2(header_col.right() - 2.0, below.top()),
            egui::pos2(header_col.right() + 2.0, below.bottom()),
        );
        let sresp = ui.interact(splitter, ui.id().with("tl_splitter"), egui::Sense::drag());
        if sresp.hovered() || sresp.dragged() {
            ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::ResizeHorizontal);
        }
        if sresp.dragged() {
            view.set_header_width(header_w + sresp.drag_delta().x);
        }

        // ── Scroll / zoom over the lane area ────────────────────────────────
        handle_scroll_zoom(ui, full, lane_left, &mut view);

        // ── Rows: draw headers + lanes, collect hit rects ───────────────────
        let seq = doc
            .timeline
            .as_ref()
            .unwrap()
            .sequences
            .get(&seq_id)
            .unwrap();
        let rows = tracks::track_rows(seq);
        let total_h: f32 = rows.iter().map(|r| r.height).sum();
        let max_scroll = (total_h - lanes_rect.height()).max(0.0);
        view.track_scroll_px = view.track_scroll_px.clamp(0.0, max_scroll);

        let colors = lane_colors(ui);
        let mut hits: Vec<interact::HitCandidate> = Vec::new();
        {
            // Clip the painter to the lanes rect so scrolled rows don't overflow.
            let lane_painter = ui.painter_at(lanes_rect);
            let mut y = lanes_rect.top() - view.track_scroll_px;
            for row in &rows {
                let row_top = y;
                let row_bot = y + row.height;
                y = row_bot;
                // Vertical cull.
                if row_bot < lanes_rect.top() || row_top > lanes_rect.bottom() {
                    continue;
                }
                let lane_rect = egui::Rect::from_min_max(
                    egui::pos2(lanes_rect.left(), row_top),
                    egui::pos2(lanes_rect.right(), row_bot),
                );
                let seq = doc
                    .timeline
                    .as_ref()
                    .unwrap()
                    .sequences
                    .get(&seq_id)
                    .unwrap();
                if let Some(track) = seq.track(row.id) {
                    // Lane separator.
                    lane_painter.line_segment(
                        [
                            egui::pos2(lanes_rect.left(), row_bot),
                            egui::pos2(lanes_rect.right(), row_bot),
                        ],
                        egui::Stroke::new(1.0, colors.clip_stroke),
                    );
                    let painted = clips::paint_lane(
                        &lane_painter,
                        &view,
                        track,
                        lane_rect,
                        &selection,
                        &colors,
                    );
                    for pc in painted {
                        // Automation-lane indicator (04 §4.1): keyframe diamonds
                        // along an animated clip's body, painted by the keyframe
                        // editor module so the two surfaces stay in sync.
                        if let Some(clip) = track.clips.iter().find(|c| c.id == pc.clip) {
                            crate::panels::video::keyframe_editor::paint_clip_automation(
                                &lane_painter,
                                &view,
                                lanes_rect.left(),
                                clip,
                                pc.rect,
                                colors.selected_stroke,
                            );
                        }
                        hits.push(interact::HitCandidate {
                            track: row.id,
                            clip: pc.clip,
                            rect: pc.rect,
                            locked: row.locked,
                        });
                    }
                }
            }
        }

        // ── Track headers (drawn after lanes so widgets sit on top) ─────────
        {
            let mut y = header_col.top() - view.track_scroll_px;
            for row in &rows {
                let row_top = y;
                let row_bot = y + row.height;
                y = row_bot;
                if row_bot < header_col.top() || row_top > header_col.bottom() - 28.0 {
                    continue;
                }
                let hrect = egui::Rect::from_min_max(
                    egui::pos2(header_col.left(), row_top),
                    egui::pos2(header_col.right(), row_bot.min(header_col.bottom() - 28.0)),
                );
                tracks::draw_header(ui, hrect, doc, history, seq_id, *row);
            }
            // Add-track controls pinned to the header column's bottom.
            let footer = egui::Rect::from_min_max(
                egui::pos2(header_col.left(), header_col.bottom() - 28.0),
                header_col.max,
            );
            tracks::draw_add_controls(ui, footer, doc, history, seq_id);
        }

        // ── Ruler + playhead ────────────────────────────────────────────────
        ruler::draw_ruler(
            ui,
            doc,
            history,
            seq_id,
            &view,
            ruler_rect,
            lane_left,
            &mut playhead,
        );

        // ── Clip interaction (select / drag / marquee / context) ────────────
        let content_rect = egui::Rect::from_min_max(ruler_rect.min, lanes_rect.max);
        self_interact(
            ui,
            doc,
            history,
            seq_id,
            &view,
            &mut playhead,
            &mut selection,
            snap,
            lane_left,
            frame_rate,
            lanes_rect,
            &rows,
            &hits,
        );

        // ── Media-pool asset drop (05 §2) ───────────────────────────────────
        // A drag started in the media pool drawer carries an `AssetDrag`
        // payload; dropping it over a lane inserts a clip there via the
        // ops_bridge path (kind-checked, one undo step). While hovering, a
        // frame-snapped insertion caret previews the landing tick.
        let asset_payload =
            egui::DragAndDrop::payload::<crate::panels::media_pool::AssetDrag>(ui.ctx());
        if let (Some(payload), Some(pos)) = (asset_payload, ui.ctx().pointer_latest_pos()) {
            if lanes_rect.contains(pos) {
                let tpf = frame_rate.ticks_per_frame().0.max(1);
                let mut at = view.x_to_tick(pos.x, lane_left).0.max(0);
                if snap {
                    at = (at / tpf) * tpf;
                }
                let at = Tick(at);
                // y → row under the cursor (same walk as the paint loop).
                // Locked tracks reject drops too (14-nle-parity QW-2
                // watch-out): `target` stays `None` over a locked lane, so no
                // caret shows and the release below is a no-op there.
                let mut yy = lanes_rect.top() - view.track_scroll_px;
                let mut target: Option<TrackId> = None;
                for row in &rows {
                    let (top, bot) = (yy, yy + row.height);
                    yy = bot;
                    if pos.y >= top && pos.y < bot {
                        if !row.locked {
                            target = Some(row.id);
                        }
                        break;
                    }
                }
                // Hover caret — suppressed over a locked lane so the absent
                // caret itself signals "can't drop here".
                if target.is_some() {
                    let x = view.tick_to_x(at, lane_left);
                    ui.painter_at(lanes_rect).line_segment(
                        [
                            egui::pos2(x, lanes_rect.top()),
                            egui::pos2(x, lanes_rect.bottom()),
                        ],
                        egui::Stroke::new(2.0, colors.selected_stroke.gamma_multiply(0.8)),
                    );
                }
                if ui.input(|i| i.pointer.any_released()) {
                    egui::DragAndDrop::clear_payload(ui.ctx());
                    if let Some(track) = target {
                        ops_bridge::insert_asset_clip(
                            doc,
                            history,
                            seq_id,
                            track,
                            payload.asset,
                            at,
                        );
                    }
                }
            }
        }

        // Playhead line over everything (drawn last).
        ruler::draw_playhead_line(
            &ui.painter_at(content_rect),
            &view,
            playhead,
            content_rect,
            lane_left,
            colors.selected_stroke,
        );

        // Keyframe / curve editor (04 §4.1, 01 §6): a floating editor that
        // auto-targets the selected clip. Invoked from here — the one video-mode
        // draw path that already holds `doc`, `history`, the playhead, and the
        // live selection — so it needs no `app/mod.rs` call-site wiring. Every
        // edit flows through a pure core keyframe op → `CommandHistory`.
        crate::panels::video::keyframe_editor::draw_window(
            ui.ctx(),
            doc,
            history,
            &mut self.keyframe_editor_target,
            &selection,
            playhead,
        );

        // Write session state back.
        self.timeline_view = view;
        self.playhead = playhead;
        self.timeline_selection = selection;
        if snap != self.timeline_snap_enabled {
            self.timeline_snap_enabled = snap;
            self.prefs.timeline_snap_enabled = snap;
        }
    }
}

/// The `pub(crate)` timeline command entry points (04 §5). The mode-switch story
/// wires command dispatch (`video.*` `CommandId`s) to these — they are unused
/// until then, hence the `dead_code` allowance. Exact names are load-bearing:
/// `timeline_zoom_in`/`timeline_zoom_out`/`timeline_zoom_fit`/
/// `timeline_toggle_snap`/`timeline_playhead_home`/`timeline_playhead_end`/
/// `timeline_prev_edit_point`/`timeline_next_edit_point`/
/// `timeline_split_at_playhead`.
#[allow(dead_code)]
impl PhotonicApp {
    /// Zoom the timeline in one step, anchored on the playhead (`+`, `video.zoom_in`).
    pub(crate) fn timeline_zoom_in(&mut self) {
        self.timeline_view
            .zoom_around(ZOOM_STEP, self.playhead, 0.0);
    }

    /// Zoom out one step (`-`, `video.zoom_out`).
    pub(crate) fn timeline_zoom_out(&mut self) {
        self.timeline_view
            .zoom_around(1.0 / ZOOM_STEP, self.playhead, 0.0);
    }

    /// Zoom to fit the active sequence's content (`Shift+Z`, `video.zoom_fit`).
    pub(crate) fn timeline_zoom_fit(&mut self, doc: &Document) {
        let extent = active_sequence(doc)
            .map(|s| s.content_end())
            .unwrap_or(Tick::ZERO);
        let w = self.timeline_view.last_lane_width_px.max(1.0);
        self.timeline_view.fit(extent, w);
    }

    /// Toggle snapping (`N`, `video.toggle_snap`).
    pub(crate) fn timeline_toggle_snap(&mut self) {
        self.timeline_snap_enabled = !self.timeline_snap_enabled;
        self.prefs.timeline_snap_enabled = self.timeline_snap_enabled;
    }

    /// Move the playhead to the sequence start (`Home`, `video.playhead_home`).
    pub(crate) fn timeline_playhead_home(&mut self) {
        self.playhead = Tick::ZERO;
    }

    /// Move the playhead to the sequence end (`End`, `video.playhead_end`).
    pub(crate) fn timeline_playhead_end(&mut self, doc: &Document) {
        if let Some(s) = active_sequence(doc) {
            self.playhead = s.content_end();
        }
    }

    /// Jump to the previous edit point (`Shift+←`, `video.prev_edit_point`).
    pub(crate) fn timeline_prev_edit_point(&mut self, doc: &Document) {
        if let Some(s) = active_sequence(doc) {
            let pts = edit_points(s);
            if let Some(prev) = pts.iter().rev().find(|t| **t < self.playhead) {
                self.playhead = *prev;
            }
        }
    }

    /// Jump to the next edit point (`Shift+→`, `video.next_edit_point`).
    pub(crate) fn timeline_next_edit_point(&mut self, doc: &Document) {
        if let Some(s) = active_sequence(doc) {
            let pts = edit_points(s);
            if let Some(next) = pts.iter().find(|t| **t > self.playhead) {
                self.playhead = *next;
            }
        }
    }

    /// Split the selected clip(s) at the playhead (`S`, `video.split_at_playhead`).
    pub(crate) fn timeline_split_at_playhead(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
    ) {
        let Some(seq_id) = doc.timeline.as_ref().and_then(|p| p.active_sequence) else {
            return;
        };
        let at = self.playhead;
        // Collect (track, clip) targets: selected clips the playhead is strictly
        // inside. If nothing selected, split whatever clip is under the playhead.
        let mut targets: Vec<(TrackId, ClipId)> = Vec::new();
        if let Some(s) = doc.timeline.as_ref().and_then(|p| p.sequences.get(&seq_id)) {
            for t in s.tracks() {
                // Locked tracks reject the split too (14-nle-parity QW-2) —
                // this is a keyboard/command path, not `hit_at`-gated, so it
                // needs its own guard.
                if t.locked {
                    continue;
                }
                for c in &t.clips {
                    let inside = at > c.start && at < c.end();
                    let hit = self.timeline_selection.contains(&c.id)
                        || self.timeline_selection.is_empty();
                    if inside && hit {
                        targets.push((t.id, c.id));
                    }
                }
            }
        }
        for (track, clip) in targets {
            ops_bridge::split(doc, history, seq_id, track, clip, at);
        }
    }
}

// ── Free helpers ────────────────────────────────────────────────────────────

fn active_frame_rate(doc: &Document) -> FrameRate {
    doc.timeline
        .as_ref()
        .map(|p| {
            p.active_sequence
                .and_then(|id| p.sequences.get(&id))
                .map(|s| s.frame_rate)
                .unwrap_or(p.settings.default_frame_rate)
        })
        .unwrap_or(FrameRate::FPS_30)
}

#[allow(dead_code)] // used by the not-yet-wired command methods above
fn active_sequence(doc: &Document) -> Option<&Sequence> {
    let p = doc.timeline.as_ref()?;
    p.active_sequence.and_then(|id| p.sequences.get(&id))
}

/// All edit points (clip edges + markers) of a sequence, sorted/deduped.
#[allow(dead_code)] // used by the not-yet-wired prev/next edit-point commands
fn edit_points(seq: &Sequence) -> Vec<Tick> {
    let mut pts = vec![Tick::ZERO];
    for t in seq.tracks() {
        for c in &t.clips {
            pts.push(c.start);
            pts.push(c.end());
        }
    }
    for m in &seq.markers {
        pts.push(m.at);
    }
    pts.sort();
    pts.dedup();
    pts
}

/// Empty-project affordance (04 §1.3): a hint + Add Track buttons that create the
/// project (and first sequence) lazily on first click.
fn draw_empty_affordance(
    ui: &mut egui::Ui,
    doc: &mut Document,
    history: &mut CommandHistory,
    frame_rate: FrameRate,
) {
    ui.vertical_centered(|ui| {
        ui.add_space(24.0);
        ui.weak("No video track yet — add one or drag media here.");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let avail = ui.available_width();
            ui.add_space((avail - 260.0).max(0.0) * 0.5);
            if ui.button("Add video track").clicked() {
                if let Some(seq) = ops_bridge::ensure_project_and_sequence(doc, history, frame_rate)
                {
                    ops_bridge::add_track(doc, history, seq, TrackKind::Video);
                }
            }
            if ui.button("Add audio track").clicked() {
                if let Some(seq) = ops_bridge::ensure_project_and_sequence(doc, history, frame_rate)
                {
                    ops_bridge::add_track(doc, history, seq, TrackKind::Audio);
                }
            }
        });
    });
}

fn draw_mini_toolbar(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    doc: &Document,
    seq_id: SequenceId,
    view: &mut TimelineView,
    snap: &mut bool,
) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, ui.visuals().faint_bg_color);
    let bh = 20.0;
    let mut x = rect.left() + 4.0;
    let y = rect.top() + (rect.height() - bh) * 0.5;

    let fit = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(bh, bh));
    if ui
        .put(fit, egui::Button::new("⤢").small())
        .on_hover_text("Zoom to fit (Shift+Z)")
        .clicked()
    {
        let extent = doc
            .timeline
            .as_ref()
            .and_then(|p| p.sequences.get(&seq_id))
            .map(|s| s.content_end())
            .unwrap_or(Tick::ZERO);
        view.fit(extent, view.last_lane_width_px.max(1.0));
    }
    x += bh + 4.0;

    let zi = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(bh, bh));
    if ui
        .put(zi, egui::Button::new("+").small())
        .on_hover_text("Zoom in (+)")
        .clicked()
    {
        view.zoom_around(ZOOM_STEP, view.scroll_ticks, 0.0);
    }
    x += bh + 2.0;
    let zo = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(bh, bh));
    if ui
        .put(zo, egui::Button::new("−").small())
        .on_hover_text("Zoom out (−)")
        .clicked()
    {
        view.zoom_around(1.0 / ZOOM_STEP, view.scroll_ticks, 0.0);
    }
    x += bh + 8.0;

    let snap_rect = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(bh + 24.0, bh));
    if ui
        .put(snap_rect, egui::SelectableLabel::new(*snap, "Snap"))
        .on_hover_text("Toggle snapping (N)")
        .clicked()
    {
        *snap = !*snap;
    }
    x += bh + 28.0;

    // Ripple-mode indicator: reflects whether Shift is held live (13 §1.1).
    let shift = ui.input(|i| i.modifiers.shift);
    let rip = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(56.0, bh));
    ui.put(
        rip,
        egui::Label::new(egui::RichText::new("Ripple").small().color(if shift {
            ui.visuals().selection.stroke.color
        } else {
            ui.visuals().weak_text_color()
        })),
    )
    .on_hover_text("Hold Shift while trimming to ripple");
}

fn handle_scroll_zoom(ui: &egui::Ui, full: egui::Rect, lane_left: f32, view: &mut TimelineView) {
    if !ui.rect_contains_pointer(full) {
        return;
    }
    let (scroll, ctrl, shift, pointer) = ui.input(|i| {
        (
            i.raw_scroll_delta,
            i.modifiers.command || i.modifiers.ctrl,
            i.modifiers.shift,
            i.pointer.hover_pos(),
        )
    });
    if scroll == egui::Vec2::ZERO {
        return;
    }
    if ctrl {
        // Ctrl+scroll → zoom toward the cursor (04 §2.1).
        let anchor = pointer
            .map(|p| view.x_to_tick(p.x, lane_left))
            .unwrap_or(view.scroll_ticks);
        let factor = if scroll.y > 0.0 {
            ZOOM_STEP
        } else {
            1.0 / ZOOM_STEP
        };
        view.zoom_around(factor, anchor, lane_left);
    } else if shift {
        // Shift+scroll → horizontal pan.
        let dt = view.px_to_ticks(-scroll.y);
        view.scroll_ticks = Tick((view.scroll_ticks.0 + dt.0).max(0));
    } else {
        // Plain scroll → vertical track pan.
        view.track_scroll_px = (view.track_scroll_px - scroll.y).max(0.0);
    }
}

fn lane_colors(ui: &egui::Ui) -> clips::LaneColors {
    let v = ui.visuals();
    clips::LaneColors {
        clip_fill: v.faint_bg_color,
        clip_stroke: v.widgets.noninteractive.bg_stroke.color,
        selected_fill: v.selection.bg_fill,
        selected_stroke: v.selection.stroke.color,
        label: v.text_color(),
        transition: v.warn_fg_color,
        offline: v.error_fg_color,
        locked_hatch: v.weak_text_color().gamma_multiply(0.5),
    }
}

/// The clip-area interaction: selection, drag gestures (move/trim/roll/slip/
/// slide/ripple-trim), marquee, and the context menu. Preview during drag,
/// commit once on release (one undo step per gesture).
#[allow(clippy::too_many_arguments)]
fn self_interact(
    ui: &mut egui::Ui,
    doc: &mut Document,
    history: &mut CommandHistory,
    seq_id: SequenceId,
    view: &TimelineView,
    playhead: &mut Tick,
    selection: &mut Vec<ClipId>,
    snap: bool,
    lane_left: f32,
    frame_rate: FrameRate,
    lanes_rect: egui::Rect,
    rows: &[tracks::TrackRow],
    hits: &[interact::HitCandidate],
) {
    let resp = ui.interact(
        lanes_rect,
        ui.id().with("timeline_lanes"),
        egui::Sense::click_and_drag(),
    );
    let drag_id = egui::Id::new(DRAG_ID);
    let marquee_id = egui::Id::new(MARQUEE_ID);

    // Which clip/zone is under a given screen pos. Locked-track candidates
    // are never a hit (14-nle-parity QW-2) — see `interact::hit_at`. This one
    // function backs selection, drag-start, and the context-menu target, so
    // the lock guard applies uniformly to all three.
    let hit_at = |pos: egui::Pos2| -> Option<(TrackId, ClipId, egui::Rect, interact::ClipZone)> {
        interact::hit_at(pos, EDGE_ZONE_PX, hits)
    };

    // ── Selection on click ──────────────────────────────────────────────────
    if resp.clicked() {
        let mods = ui.input(|i| i.modifiers);
        if let Some(pos) = resp.interact_pointer_pos() {
            if let Some((_t, clip, _r, _z)) = hit_at(pos) {
                apply_selection(selection, clip, mods);
            } else if !mods.ctrl && !mods.shift && !mods.command {
                selection.clear();
            }
        }
    }

    // ── Drag start ──────────────────────────────────────────────────────────
    if resp.drag_started() {
        let mods = ui.input(|i| i.modifiers);
        if let Some(pos) = resp.interact_pointer_pos() {
            let seq = doc
                .timeline
                .as_ref()
                .unwrap()
                .sequences
                .get(&seq_id)
                .unwrap();
            if let Some((track, clip, _rect, zone)) = hit_at(pos) {
                // Selection follows the grabbed clip unless it is already in a
                // multi-selection being dragged.
                if !selection.contains(&clip) {
                    apply_selection(selection, clip, mods);
                }
                if let Some(state) = start_clip_drag(
                    seq, track, clip, zone, mods, pos, view, lane_left, *playhead,
                ) {
                    ui.data_mut(|d| d.insert_temp(drag_id, state));
                }
            } else {
                // Empty space → marquee.
                ui.data_mut(|d| {
                    d.insert_temp(
                        marquee_id,
                        Marquee {
                            start: pos,
                            additive: mods.shift || mods.ctrl || mods.command,
                        },
                    )
                });
            }
        }
    }

    // ── Drag update: preview ghost + snap guide ─────────────────────────────
    if resp.dragged() {
        if let Some(mut state) = ui.data(|d| d.get_temp::<DragState>(drag_id)) {
            state.moved = true;
            if let Some(pos) = resp.interact_pointer_pos() {
                let seq = doc
                    .timeline
                    .as_ref()
                    .unwrap()
                    .sequences
                    .get(&seq_id)
                    .unwrap();
                state.dest_track = track_at_y(rows, lanes_rect, view, pos.y).unwrap_or(state.track);
                preview_drag(
                    ui, seq, &state, view, lane_left, snap, frame_rate, pos, lanes_rect,
                );
            }
            ui.data_mut(|d| d.insert_temp(drag_id, state));
        } else if let Some(m) = ui.data(|d| d.get_temp::<Marquee>(marquee_id)) {
            if let Some(pos) = resp.interact_pointer_pos() {
                let rect = egui::Rect::from_two_pos(m.start, pos);
                ui.painter_at(lanes_rect).rect(
                    rect,
                    0.0,
                    egui::Color32::TRANSPARENT,
                    egui::Stroke::new(1.0, ui.visuals().selection.stroke.color),
                );
                // Locked-track clips are excluded from marquee selection too
                // (14-nle-parity QW-2 — fully inert, not just edit-blocked).
                interact::apply_marquee(
                    rect,
                    hits.iter().filter(|h| !h.locked).map(|h| (h.rect, h.clip)),
                    m.additive,
                    selection,
                );
            }
        }
    }

    // ── Drag release: commit ────────────────────────────────────────────────
    if resp.drag_stopped() {
        let state = ui.data(|d| d.get_temp::<DragState>(drag_id));
        ui.data_mut(|d| {
            d.remove::<DragState>(drag_id);
            d.remove::<Marquee>(marquee_id);
        });
        if let Some(state) = state {
            if state.moved {
                if let Some(pos) = resp.interact_pointer_pos() {
                    commit_drag(
                        doc, history, seq_id, &state, view, lane_left, snap, frame_rate, pos,
                    );
                }
            }
        }
    }

    // ── Context menu ────────────────────────────────────────────────────────
    let menu_target = resp.hover_pos().and_then(hit_at).map(|(t, c, _, _)| (t, c));
    let ph = *playhead;
    resp.context_menu(|ui| clip_context_menu(ui, doc, history, seq_id, menu_target, ph));
}

fn apply_selection(selection: &mut Vec<ClipId>, clip: ClipId, mods: egui::Modifiers) {
    if mods.ctrl || mods.command {
        if let Some(i) = selection.iter().position(|c| *c == clip) {
            selection.remove(i);
        } else {
            selection.push(clip);
        }
    } else if mods.shift {
        if !selection.contains(&clip) {
            selection.push(clip);
        }
    } else {
        selection.clear();
        selection.push(clip);
    }
}

/// Which track row contains screen-y `y` (for cross-track moves).
fn track_at_y(
    rows: &[tracks::TrackRow],
    lanes_rect: egui::Rect,
    view: &TimelineView,
    y: f32,
) -> Option<TrackId> {
    let mut top = lanes_rect.top() - view.track_scroll_px;
    for r in rows {
        let bot = top + r.height;
        if y >= top && y < bot {
            return Some(r.id);
        }
        top = bot;
    }
    None
}

/// Resolve a drag-start into a [`DragState`] (04 §2.3/§2.4 modifier scheme).
#[allow(clippy::too_many_arguments)]
fn start_clip_drag(
    seq: &Sequence,
    track: TrackId,
    clip: ClipId,
    zone: interact::ClipZone,
    mods: egui::Modifiers,
    pos: egui::Pos2,
    view: &TimelineView,
    lane_left: f32,
    playhead: Tick,
) -> Option<DragState> {
    let t = seq.track(track)?;
    let idx = t.clips.iter().position(|c| c.id == clip)?;
    let c = &t.clips[idx];
    let grab_tick = view.x_to_tick(pos.x, lane_left);

    // Resolve (kind, primary clip). For a roll, `primary` is always the LEFT
    // clip and `Roll.right` its right neighbour, so the shared boundary is
    // `primary.end()` (04 §2.4: roll is hit-test-based, not a modifier). The
    // neighbour ids for a flush boundary drive the (pure) resolution below.
    let left_shared = idx
        .checked_sub(1)
        .and_then(|i| t.clips.get(i))
        .filter(|prev| prev.end() == c.start)
        .map(|prev| prev.id);
    let right_shared = t
        .clips
        .get(idx + 1)
        .filter(|next| next.start == c.end())
        .map(|next| next.id);
    let (kind, primary) =
        interact::resolve_drag_kind(zone, mods.alt, mods.shift, clip, left_shared, right_shared);

    let candidates = interact::build_snap_candidates(seq, track, primary, playhead);
    let orig = ClipTiming::of(t.clips.iter().find(|c| c.id == primary)?);
    Some(DragState {
        kind,
        track,
        clip: primary,
        grab_tick,
        orig,
        dest_track: track,
        candidates,
        moved: false,
    })
}

/// Snap + quantize the moving edge tick for a drag.
fn resolved_tick(
    raw: Tick,
    state: &DragState,
    snap: bool,
    frame_rate: FrameRate,
    view: &TimelineView,
) -> Tick {
    let threshold = view.px_to_ticks(SNAP_THRESHOLD_PX);
    interact::snap_and_quantize(raw, &state.candidates, threshold, snap, frame_rate)
}

/// Draw the ghost + snap guide for an in-progress drag.
#[allow(clippy::too_many_arguments)]
fn preview_drag(
    ui: &egui::Ui,
    seq: &Sequence,
    state: &DragState,
    view: &TimelineView,
    lane_left: f32,
    snap: bool,
    frame_rate: FrameRate,
    pos: egui::Pos2,
    lanes_rect: egui::Rect,
) {
    let Some(t) = seq.track(state.track) else {
        return;
    };
    let Some(c) = t.clips.iter().find(|c| c.id == state.clip) else {
        return;
    };
    let accent = ui.visuals().selection.stroke.color;
    let painter = ui.painter_at(lanes_rect);
    let delta_raw = view.x_to_tick(pos.x, lane_left) - state.grab_tick;

    let (edge_tick, y_shift) = match state.kind {
        DragKind::Move | DragKind::Slide => {
            let new_start =
                resolved_tick(state.orig.start + delta_raw, state, snap, frame_rate, view);
            (new_start, 0.0)
        }
        DragKind::TrimStart | DragKind::RippleTrimStart => (
            resolved_tick(state.orig.start + delta_raw, state, snap, frame_rate, view),
            0.0,
        ),
        DragKind::TrimEnd | DragKind::RippleTrimEnd => {
            let end = state.orig.start + state.orig.duration + delta_raw;
            (resolved_tick(end, state, snap, frame_rate, view), 0.0)
        }
        DragKind::Roll { .. } => (
            resolved_tick(c.start + delta_raw, state, snap, frame_rate, view),
            0.0,
        ),
        DragKind::Slip => (c.start, 0.0),
    };
    let _ = y_shift;

    // Ghost outline of the clip at its previewed position.
    let (gx0, gx1) = match state.kind {
        DragKind::Move | DragKind::Slide => (edge_tick, edge_tick + c.duration),
        DragKind::TrimStart | DragKind::RippleTrimStart => (edge_tick, c.end()),
        DragKind::TrimEnd | DragKind::RippleTrimEnd | DragKind::Roll { .. } => (c.start, edge_tick),
        DragKind::Slip => (c.start, c.end()),
    };
    let x0 = view.tick_to_x(gx0, lane_left);
    let x1 = view.tick_to_x(gx1, lane_left);
    let ghost = egui::Rect::from_min_max(
        egui::pos2(x0.min(x1), lanes_rect.top() + 2.0),
        egui::pos2(x0.max(x1), lanes_rect.bottom() - 2.0),
    );
    painter.rect(
        ghost,
        egui::Rounding::same(3.0),
        accent.gamma_multiply(0.15),
        egui::Stroke::new(1.0, accent),
    );

    // Snap guide: a vertical accent line at the resolved edge (13 §1.1).
    let gx = view.tick_to_x(edge_tick, lane_left);
    painter.line_segment(
        [
            egui::pos2(gx, lanes_rect.top()),
            egui::pos2(gx, lanes_rect.bottom()),
        ],
        egui::Stroke::new(1.0, accent.gamma_multiply(0.6)),
    );

    // Duration tooltip for trims (13 §1.3/1.6 non-color confirmation).
    if matches!(
        state.kind,
        DragKind::TrimStart
            | DragKind::TrimEnd
            | DragKind::RippleTrimStart
            | DragKind::RippleTrimEnd
    ) {
        let dur = (gx1 - gx0).0.max(0);
        painter.text(
            egui::pos2(gx + 4.0, lanes_rect.top() + 4.0),
            egui::Align2::LEFT_TOP,
            ruler::timecode(Tick(dur), frame_rate),
            egui::FontId::monospace(10.0),
            ui.visuals().text_color(),
        );
    }
}

/// Commit a finished drag as one undo step through `ops_bridge`.
#[allow(clippy::too_many_arguments)]
fn commit_drag(
    doc: &mut Document,
    history: &mut CommandHistory,
    seq_id: SequenceId,
    state: &DragState,
    view: &TimelineView,
    lane_left: f32,
    snap: bool,
    frame_rate: FrameRate,
    pos: egui::Pos2,
) {
    let delta_raw = view.x_to_tick(pos.x, lane_left) - state.grab_tick;
    match state.kind {
        DragKind::Move => {
            let new_start =
                resolved_tick(state.orig.start + delta_raw, state, snap, frame_rate, view);
            let new_start = if new_start.0 < 0 {
                Tick::ZERO
            } else {
                new_start
            };
            if state.dest_track != state.track {
                ops_bridge::move_clip_cross_track(
                    doc,
                    history,
                    seq_id,
                    state.track,
                    state.dest_track,
                    state.clip,
                    new_start,
                );
            } else {
                ops_bridge::move_clip(doc, history, seq_id, state.track, state.clip, new_start);
            }
        }
        DragKind::TrimStart | DragKind::RippleTrimStart => {
            let new_start =
                resolved_tick(state.orig.start + delta_raw, state, snap, frame_rate, view);
            let end = state.orig.start + state.orig.duration;
            if new_start >= end {
                return;
            }
            let new = ClipTiming {
                start: new_start,
                duration: end - new_start,
                source_in: state.orig.source_in + (new_start - state.orig.start),
            };
            if matches!(state.kind, DragKind::RippleTrimStart) {
                ops_bridge::ripple_trim(doc, history, seq_id, state.track, state.clip, new);
            } else {
                ops_bridge::trim(doc, history, seq_id, state.track, state.clip, new);
            }
        }
        DragKind::TrimEnd | DragKind::RippleTrimEnd => {
            let raw_end = state.orig.start + state.orig.duration + delta_raw;
            let new_end = resolved_tick(raw_end, state, snap, frame_rate, view);
            if new_end <= state.orig.start {
                return;
            }
            let new = ClipTiming {
                start: state.orig.start,
                duration: new_end - state.orig.start,
                source_in: state.orig.source_in,
            };
            if matches!(state.kind, DragKind::RippleTrimEnd) {
                ops_bridge::ripple_trim(doc, history, seq_id, state.track, state.clip, new);
            } else {
                ops_bridge::trim(doc, history, seq_id, state.track, state.clip, new);
            }
        }
        DragKind::Roll { right } => {
            // `state.clip` is the left clip; `right` the neighbor.
            let boundary = state.orig.start + state.orig.duration;
            let new_boundary = resolved_tick(boundary + delta_raw, state, snap, frame_rate, view);
            let delta = new_boundary - boundary;
            if delta.0 != 0 {
                ops_bridge::roll(doc, history, seq_id, state.track, state.clip, right, delta);
            }
        }
        DragKind::Slip => {
            // Drag right → reveal earlier source (source_in decreases).
            let d = resolved_tick(state.grab_tick + delta_raw, state, snap, frame_rate, view)
                - state.grab_tick;
            let new_source_in = state.orig.source_in - d;
            if new_source_in.0 >= 0 {
                ops_bridge::slip(doc, history, seq_id, state.track, state.clip, new_source_in);
            }
        }
        DragKind::Slide => {
            let new_start =
                resolved_tick(state.orig.start + delta_raw, state, snap, frame_rate, view);
            let delta = new_start - state.orig.start;
            if delta.0 != 0 {
                ops_bridge::slide(doc, history, seq_id, state.track, state.clip, delta);
            }
        }
    }
}

fn clip_context_menu(
    ui: &mut egui::Ui,
    doc: &mut Document,
    history: &mut CommandHistory,
    seq_id: SequenceId,
    target: Option<(TrackId, ClipId)>,
    playhead: Tick,
) {
    let Some((track, clip)) = target else {
        ui.label("No clip");
        return;
    };
    let (inside, enabled) = doc
        .timeline
        .as_ref()
        .and_then(|p| p.sequences.get(&seq_id))
        .and_then(|s| s.track(track))
        .and_then(|t| t.clips.iter().find(|c| c.id == clip))
        .map(|c| (playhead > c.start && playhead < c.end(), c.enabled))
        .unwrap_or((false, true));

    if ui
        .add_enabled(inside, egui::Button::new("Split at playhead"))
        .clicked()
    {
        ops_bridge::split(doc, history, seq_id, track, clip, playhead);
        ui.close_menu();
    }
    if ui.button("Delete").clicked() {
        ops_bridge::remove_clip(doc, history, seq_id, track, clip);
        ui.close_menu();
    }
    if ui.button("Ripple delete").clicked() {
        ops_bridge::ripple_delete(doc, history, seq_id, track, clip);
        ui.close_menu();
    }
    let label = if enabled { "Disable" } else { "Enable" };
    if ui.button(label).clicked() {
        ops_bridge::set_clip_enabled(doc, history, seq_id, track, clip, !enabled);
        ui.close_menu();
    }
    ui.separator();
    ui.add_enabled(false, egui::Button::new("Add transition in"))
        .on_disabled_hover_text("P6");
    ui.add_enabled(false, egui::Button::new("Add transition out"))
        .on_disabled_hover_text("P6");
    ui.add_enabled(false, egui::Button::new("Open as node composition"))
        .on_disabled_hover_text("P8");
}
