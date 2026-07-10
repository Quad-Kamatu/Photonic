//! Video-mode program monitor + transport (video-editor-module
//! `04-ui-mode-timeline.md` §1.2, §1.3, §3). Wired to the real engine: when
//! `self.engine` is `Some`, the monitor presents the latest `EngineFrame`
//! (03 §5, presented into an egui native texture by
//! `app/engine.rs::EngineBridge::present_latest` each host frame) and the
//! transport reconciles GUI intent into `EngineCmd`s
//! (`drive_engine_playback`). Also owns mode entry/exit (lazy project
//! creation, §1.3) and the first-run discoverability hints (§1.2).
//!
//! Engine-less hosts (unit tests, GPU-free machines) keep the original
//! wall-clock placeholder playback so the transport/scrub UX still works.

use super::*;
use crate::app::engine;
use crate::app::timeline::ops_bridge;
use crate::commands;
use photonic_core::timeline::{FrameRate, Sequence, SequenceFormat, TICKS_PER_SECOND};
use photonic_video::ProxyMode;

// ── Monitor toolbar polish: playback resolution + image zoom (14 §M-5) ─────
//
// Both selectors are GUI-only state — there is no `PhotonicApp` field for
// either (this story's territory is this file alone), so they live in egui's
// per-`Id` temp storage (`Context::data`/`data_mut`), the same mechanism
// already used for other widget-local state in this crate (e.g.
// `color_popup.rs`, `panels/modify.rs`). Written and read every frame, so it
// persists for the life of the session like a normal field would.

/// GUI-only playback-preview resolution levels for the monitor toolbar. The
/// engine exposes only a binary proxy-media switch today
/// (`photonic_video::ProxyMode` — `Auto`/`ForceProxy`/`ForceOriginal`, see
/// `session.rs`), not a graduated preview-scale command, so `Half` and
/// `Quarter` both collapse onto `ForceProxy` (documented seam: a real
/// half/quarter preview-scale primitive is the engine-side follow-up). The
/// three-way GUI selection is still tracked distinctly so the user's choice
/// reads back correctly and the collapse is the only compromise.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum PlaybackResolution {
    Full,
    Half,
    Quarter,
}

impl PlaybackResolution {
    const ALL: [PlaybackResolution; 3] = [
        PlaybackResolution::Full,
        PlaybackResolution::Half,
        PlaybackResolution::Quarter,
    ];

    fn label(self) -> &'static str {
        match self {
            PlaybackResolution::Full => "Full",
            PlaybackResolution::Half => "1/2",
            PlaybackResolution::Quarter => "1/4",
        }
    }

    /// Map to the engine's actual proxy-mode knob (documented collapse, see
    /// the type doc above).
    fn to_proxy_mode(self) -> ProxyMode {
        match self {
            PlaybackResolution::Full => ProxyMode::ForceOriginal,
            PlaybackResolution::Half | PlaybackResolution::Quarter => ProxyMode::ForceProxy,
        }
    }
}

/// GUI-only monitor image zoom. `Fit` (default) letterboxes the frame into
/// the available content area — the pre-existing behavior. `Actual` shows it
/// at native pixels and is draggable (`pan`) when it overflows the content
/// area.
#[derive(Copy, Clone, Debug, PartialEq, Default)]
struct MonitorZoom {
    mode: ImageZoomMode,
    /// Drag offset from center. Only meaningful in `Actual` mode; reset to
    /// zero whenever the mode switches back to `Fit`.
    pan: egui::Vec2,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
enum ImageZoomMode {
    #[default]
    Fit,
    Actual,
}

const MONITOR_ZOOM_STATE_ID: &str = "photonic.video_monitor.zoom_state";
const MONITOR_RESOLUTION_STATE_ID: &str = "photonic.video_monitor.playback_resolution";

impl PhotonicApp {
    fn monitor_zoom_state(&self, ctx: &egui::Context) -> MonitorZoom {
        ctx.data(|d| d.get_temp(egui::Id::new(MONITOR_ZOOM_STATE_ID)))
            .unwrap_or_default()
    }

    fn set_monitor_zoom_state(&self, ctx: &egui::Context, zoom: MonitorZoom) {
        ctx.data_mut(|d| d.insert_temp(egui::Id::new(MONITOR_ZOOM_STATE_ID), zoom));
    }

    fn monitor_playback_resolution(&self, ctx: &egui::Context) -> PlaybackResolution {
        ctx.data(|d| d.get_temp(egui::Id::new(MONITOR_RESOLUTION_STATE_ID)))
            .unwrap_or(PlaybackResolution::Full)
    }

    /// Persist the chosen resolution and — when an engine is attached —
    /// apply it immediately via the bridge's `proxy_mode` intent field
    /// (reconciled into a real `EngineCmd::SetProxyMode` by
    /// `EngineBridge::apply_proxy_mode`, called each frame from
    /// `drive_playback`). A no-op on engine-less hosts (tests, GPU-free
    /// machines) beyond remembering the GUI selection.
    fn set_monitor_playback_resolution(&mut self, ctx: &egui::Context, res: PlaybackResolution) {
        ctx.data_mut(|d| d.insert_temp(egui::Id::new(MONITOR_RESOLUTION_STATE_ID), res));
        if let Some(bridge) = self.engine.as_mut() {
            bridge.proxy_mode = res.to_proxy_mode();
        }
    }
}

// ── Mode entry/exit (04 §1.2, §1.3) ─────────────────────────────────────────

impl PhotonicApp {
    /// Toggle between Vector/Video for the active tab (04 §1.2). Lazily
    /// creates `doc.timeline` (§1.3) on the way into Video so every entry
    /// path — this toggle, the command palette, the Welcome new-video-project
    /// action, and auto-enter-on-open — keeps `self.mode == Video` and
    /// `doc.timeline.is_some()` in lockstep, per the invariant §1.1's central-
    /// panel branch `debug_assert!`s.
    pub(crate) fn enter_or_exit_video_mode(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
    ) {
        match self.mode {
            AppMode::Video => {
                // Exit-to-Vector always pauses first, unconditionally (04 §7
                // "mode-switch race with in-flight engine playback") — a real
                // `EngineCmd::Pause` so a hidden monitor never keeps decoding.
                self.monitor_playing = false;
                if let Some(bridge) = self.engine.as_mut() {
                    bridge.set_playing(false);
                }
                self.mode = AppMode::Vector;
            }
            AppMode::Vector => {
                self.ensure_timeline_project(doc, history);
                self.mode = AppMode::Video;
                self.maybe_show_first_entry_hints();
            }
        }
        // A drawer open in the old mode is meaningless in the new one (04 §4).
        self.open_drawer = None;
    }

    /// First-video-mode-action project creation (04 §1.3), defaulting to a
    /// 1920x1080/30fps sequence. Used by the toolbar toggle, the command
    /// palette, and auto-enter-on-open. The Welcome "New Video Project" flow
    /// calls [`Self::ensure_timeline_project_with`] directly so the user's
    /// chosen format/frame rate is honored instead of the default.
    pub(crate) fn ensure_timeline_project(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
    ) {
        self.ensure_timeline_project_with(doc, history, 1920, 1080, FrameRate::FPS_30);
    }

    /// Create `doc.timeline` (with one default sequence) if it doesn't exist
    /// yet, undoably via `TimelineCmd::CreateProject` + `AddSequence` (01
    /// §10). No-op if a timeline already exists — callers never need to
    /// check first.
    pub(crate) fn ensure_timeline_project_with(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
        width: u32,
        height: u32,
        frame_rate: FrameRate,
    ) {
        if doc.timeline.is_some() {
            return;
        }
        use photonic_core::timeline::ops;
        history.execute(Command::Timeline(ops::create_project()), doc);
        let seq = Sequence::new("Sequence 1", frame_rate, width, height);
        history.execute(Command::Timeline(ops::add_sequence(seq)), doc);
    }

    /// First-entry hints (04 §1.2): the one-time shortcut overlay auto-opens
    /// the very first time a session enters video mode; `?` re-opens it any
    /// time after regardless of this having already fired once.
    fn maybe_show_first_entry_hints(&mut self) {
        if !self.prefs.video_shortcuts_intro_shown {
            self.show_video_shortcut_sheet = true;
            self.prefs.video_shortcuts_intro_shown = true;
            self.prefs.save();
        }
    }

    /// One-shot check for a CLI/double-click-opened document that already has
    /// a timeline (04 §1.2 "Auto-enter on open") — a project with a timeline
    /// always opens into video mode. Called once, right after
    /// `ensure_initial_tab` on the very first `draw` frame; the
    /// `WelcomeAction::OpenFile` arms handle the equivalent case for files
    /// opened from the welcome screen (`app/mod.rs`, same §1.2 rule).
    pub(crate) fn check_initial_auto_enter(&mut self, doc: &Document) {
        if self.initial_mode_checked {
            return;
        }
        self.initial_mode_checked = true;
        if doc.timeline.is_some() {
            self.mode = AppMode::Video;
        }
    }
}

// ── Sequence/format lookup helpers ──────────────────────────────────────────

/// The active sequence, if a timeline project exists and has one selected.
fn active_sequence(doc: &Document) -> Option<&Sequence> {
    let project = doc.timeline.as_ref()?;
    let id = project.active_sequence?;
    project.sequences.get(&id)
}

/// True if the active sequence has any clip on any track (video or audio).
/// Drives the monitor's empty-state invitation.
fn sequence_has_clips(doc: &Document) -> bool {
    doc.timeline
        .as_ref()
        .and_then(|p| p.active_sequence.and_then(|id| p.sequences.get(&id)))
        .map(|seq| {
            seq.video_tracks
                .iter()
                .chain(seq.audio_tracks.iter())
                .any(|t| !t.clips.is_empty())
        })
        .unwrap_or(false)
}

/// The active sequence's active format, or a 1920x1080 16:9 default when no
/// sequence exists yet (04 §3 "default 16:9 1920x1080 when absent").
fn active_format(doc: &Document) -> SequenceFormat {
    active_sequence(doc)
        .map(|s| s.format().clone())
        .unwrap_or_else(|| SequenceFormat::new("16:9", 1920, 1080))
}

/// The active sequence's frame rate, defaulting to 30fps.
fn active_frame_rate(doc: &Document) -> FrameRate {
    active_sequence(doc)
        .map(|s| s.frame_rate)
        .unwrap_or(FrameRate::FPS_30)
}

/// End tick of the active sequence's content (0 for an empty/absent one).
fn sequence_end_tick(doc: &Document) -> Tick {
    active_sequence(doc)
        .map(|s| s.content_end())
        .unwrap_or(Tick::ZERO)
}

/// Format a tick as `HH:MM:SS:FF` at the given frame rate (04 §3.2).
fn format_timecode(fr: FrameRate, t: Tick) -> String {
    let frame_idx = fr.frame_at(t).max(0);
    let fps = ((fr.num as f64 / fr.den.max(1) as f64).round() as i64).max(1);
    let total_secs = frame_idx / fps;
    let ff = frame_idx % fps;
    let hh = total_secs / 3600;
    let mm = (total_secs % 3600) / 60;
    let ss = total_secs % 60;
    format!("{hh:02}:{mm:02}:{ss:02}:{ff:02}")
}

// ── Transport (04 §3.2, §5.1) ───────────────────────────────────────────────
//
// Transport methods mutate GUI *intent* (`monitor_playing`, reverse/speed,
// `self.playhead`); `drive_engine_playback` reconciles that intent into the
// minimal `EngineCmd` stream each frame (02 §1: Play/Pause/Seek/Step/SetLoop)
// and follows `EngineStatus.playhead` while the engine is playing. With no
// engine attached (tests, GPU-less hosts) the original wall-clock placeholder
// (`advance_monitor_playback`) keeps the UX testable.

impl PhotonicApp {
    pub(crate) fn video_play_pause(&mut self) {
        // Intent only — `drive_engine_playback` sends Play/Pause on the diff.
        self.monitor_playing = !self.monitor_playing;
        if self.monitor_playing {
            self.monitor_play_reverse = false;
            self.monitor_play_speed = 1.0;
        }
    }

    /// J: play reverse; repeated presses ramp speed (reference-NLE convention).
    /// The engine has no reverse primitive (speed-maps story, P8), so with an
    /// engine attached this becomes a coalesced-`Seek` shuttle while the
    /// engine stays paused — see `drive_engine_playback`.
    pub(crate) fn video_play_reverse(&mut self) {
        if self.monitor_playing && self.monitor_play_reverse {
            self.monitor_play_speed = (self.monitor_play_speed * 2.0).min(8.0);
        } else {
            self.monitor_playing = true;
            self.monitor_play_reverse = true;
            self.monitor_play_speed = 1.0;
        }
    }

    /// K: pause.
    pub(crate) fn video_pause(&mut self) {
        self.monitor_playing = false;
    }

    /// L: play forward; repeated presses ramp speed (1× uses the engine's
    /// audio-mastered `Play`; >1× falls back to the `Seek` shuttle).
    pub(crate) fn video_play_forward(&mut self) {
        if self.monitor_playing && !self.monitor_play_reverse {
            self.monitor_play_speed = (self.monitor_play_speed * 2.0).min(8.0);
        } else {
            self.monitor_playing = true;
            self.monitor_play_reverse = false;
            self.monitor_play_speed = 1.0;
        }
    }

    pub(crate) fn video_step_back(&mut self, doc: &Document) {
        self.monitor_playing = false;
        let tpf = active_frame_rate(doc).ticks_per_frame().0.max(1);
        self.playhead = Tick((self.playhead.0 - tpf).max(0));
        if let Some(bridge) = self.engine.as_mut() {
            // Exact-frame step on the engine (02 §4: Step always pauses); the
            // local move above is the optimistic echo of the same arithmetic.
            bridge.step(-1);
            bridge.note_agreed(self.playhead);
        }
    }

    pub(crate) fn video_step_forward(&mut self, doc: &Document) {
        self.monitor_playing = false;
        let tpf = active_frame_rate(doc).ticks_per_frame().0.max(1);
        let mut next = self.playhead.0 + tpf;
        let end = sequence_end_tick(doc).0;
        if end > 0 {
            next = next.min(end);
        }
        self.playhead = Tick(next);
        if let Some(bridge) = self.engine.as_mut() {
            bridge.step(1);
            bridge.note_agreed(self.playhead);
        }
    }

    pub(crate) fn video_playhead_home(&mut self) {
        // Intent only — the reconciler's scrub detector turns the moved
        // playhead into an `EngineCmd::Seek`.
        self.monitor_playing = false;
        self.playhead = Tick::ZERO;
    }

    pub(crate) fn video_playhead_end(&mut self, doc: &Document) {
        self.monitor_playing = false;
        self.playhead = sequence_end_tick(doc);
    }

    /// I: set in-point at playhead (`Sequence::work_range`, 01 §4 document
    /// state). O: set out-point. Undoable via `ops::set_work_range` →
    /// `TimelineCmd::SetWorkRange`, routed through `ops_bridge` like every
    /// other timeline edit (04 §2.3).
    pub(crate) fn video_set_in(&mut self, doc: &mut Document, history: &mut CommandHistory) {
        self.set_work_range_bound(doc, history, true);
    }

    pub(crate) fn video_set_out(&mut self, doc: &mut Document, history: &mut CommandHistory) {
        self.set_work_range_bound(doc, history, false);
    }

    fn set_work_range_bound(
        &mut self,
        doc: &mut Document,
        history: &mut CommandHistory,
        is_in: bool,
    ) {
        let playhead = self.playhead;
        let Some(project) = doc.timeline.as_ref() else {
            return;
        };
        let Some(seq_id) = project.active_sequence else {
            return;
        };
        let Some(seq) = project.sequences.get(&seq_id) else {
            return;
        };
        let (mut in_t, mut out_t) = seq.work_range.unwrap_or((Tick::ZERO, seq.content_end()));
        if is_in {
            in_t = playhead;
            if out_t < in_t {
                out_t = in_t;
            }
        } else {
            out_t = playhead;
            if in_t > out_t {
                in_t = out_t;
            }
        }
        ops_bridge::set_work_range(doc, history, seq_id, Some((in_t, out_t)));
    }

    /// Per-frame playback driver: reconcile GUI intent into `EngineCmd`s and
    /// follow `EngineStatus` when an engine is attached; otherwise fall back
    /// to the wall-clock placeholder. Called once per frame from
    /// [`Self::draw_video_monitor`].
    fn drive_playback(&mut self, ctx: &egui::Context, doc: &Document) {
        if self.engine.is_none() {
            self.advance_monitor_playback(ctx, doc);
            return;
        }

        // Desired-state inputs computed before borrowing the bridge.
        let active_seq = doc.timeline.as_ref().and_then(|p| p.active_sequence);
        let end = sequence_end_tick(doc);
        let loop_range = if self.monitor_loop_enabled && end.0 > 0 {
            Some(
                active_sequence(doc)
                    .and_then(|s| s.work_range)
                    .unwrap_or((Tick::ZERO, end)),
            )
        } else {
            None
        };
        let shuttle = self.monitor_playing
            && (self.monitor_play_reverse || self.monitor_play_speed != 1.0);
        let dt = ctx.input(|i| i.unstable_dt as f64).min(0.25);

        let bridge = self.engine.as_mut().expect("checked above");
        bridge.set_active_sequence(active_seq);
        bridge.apply_proxy_mode();
        bridge.set_loop(loop_range);

        // User scrub (ruler drag, Home/End, marker jump): the playhead moved
        // without the bridge agreeing to it → Seek.
        if bridge.agreed_playhead != Some(self.playhead) {
            bridge.seek(self.playhead);
        }

        if shuttle {
            // Reverse / ramped playback: no engine primitive yet (P8 speed
            // maps), so scrub with coalesced Seeks while the engine is paused.
            bridge.set_playing(false);
            ctx.request_repaint();
            let dir = if self.monitor_play_reverse { -1.0 } else { 1.0 };
            let delta = (dt * TICKS_PER_SECOND as f64 * self.monitor_play_speed * dir) as i64;
            let mut next = self.playhead.0 + delta;
            if self.monitor_loop_enabled && end.0 > 0 {
                next = next.rem_euclid(end.0.max(1));
            } else {
                next = next.max(0);
                if end.0 > 0 && next >= end.0 {
                    next = end.0;
                    self.monitor_playing = false;
                }
            }
            self.playhead = Tick(next);
            bridge.seek(self.playhead);
        } else {
            bridge.set_playing(self.monitor_playing);
            if self.monitor_playing {
                ctx.request_repaint();
                let status = bridge.status();
                if status.playing {
                    // The engine's clock is authoritative at 1× (02 §4).
                    self.playhead = status.playhead;
                    bridge.note_agreed(self.playhead);
                }
            }
        }
    }

    /// Wall-clock placeholder playback for engine-less hosts (pre-P3
    /// behavior, kept for tests and GPU-free machines).
    fn advance_monitor_playback(&mut self, ctx: &egui::Context, doc: &Document) {
        if !self.monitor_playing {
            return;
        }
        ctx.request_repaint();
        let dt = ctx.input(|i| i.unstable_dt as f64).min(0.25);
        let dir = if self.monitor_play_reverse { -1.0 } else { 1.0 };
        let delta = (dt * TICKS_PER_SECOND as f64 * self.monitor_play_speed * dir) as i64;
        let mut next = self.playhead.0 + delta;
        let end = sequence_end_tick(doc).0;
        if self.monitor_loop_enabled && end > 0 {
            next = next.rem_euclid(end.max(1));
        } else {
            next = next.max(0);
            if end > 0 && next >= end {
                next = end;
                self.monitor_playing = false;
            }
        }
        self.playhead = Tick(next);
    }
}

// ── Central-panel content (04 §1.1 point 3, §3) ─────────────────────────────

impl PhotonicApp {
    /// The video-mode program monitor + transport bar, drawn in place of the
    /// vector canvas content when `self.mode == AppMode::Video` (04 §1.1
    /// point 3). With an engine attached this paints the latest presented
    /// `EngineFrame` (03 §5) cropped to the sequence format's logical size;
    /// engine-less hosts keep the dark letterboxed placeholder.
    pub(crate) fn draw_video_monitor(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        rect: egui::Rect,
        doc: &mut Document,
        history: &mut CommandHistory,
    ) {
        self.drive_playback(ctx, doc);
        self.handle_video_keyboard(ctx, doc, history);

        let format = active_format(doc);
        let painter = ui.painter_at(rect);

        // Reserve a top strip for the aspect/frame bar (CAP-012), a thin scrub
        // strip and a transport bar (04 §3.2, QW-5) at the bottom. The video
        // image sits between the format bar and the scrubber.
        const FORMAT_H: f32 = 30.0;
        const SCRUB_H: f32 = 16.0;
        const TRANSPORT_H: f32 = 40.0;
        let format_rect = egui::Rect::from_min_max(
            rect.min,
            egui::pos2(rect.max.x, (rect.min.y + FORMAT_H).min(rect.max.y)),
        );
        let transport_top = (rect.max.y - TRANSPORT_H).max(format_rect.max.y);
        let scrub_top = (transport_top - SCRUB_H).max(format_rect.max.y);
        let content_rect = egui::Rect::from_min_max(
            egui::pos2(rect.min.x, format_rect.max.y),
            egui::pos2(rect.max.x, scrub_top),
        );
        let scrub_rect = egui::Rect::from_min_max(
            egui::pos2(rect.min.x, scrub_top),
            egui::pos2(rect.max.x, transport_top),
        );
        let transport_rect =
            egui::Rect::from_min_max(egui::pos2(rect.min.x, transport_top), rect.max);

        // Background + letterbox/pillarbox bars (04 §3.3).
        painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(12, 12, 14));
        let target_aspect = format.width.max(1) as f32 / format.height.max(1) as f32;
        let avail_w = content_rect.width().max(1.0);
        let avail_h = content_rect.height().max(1.0);

        // Image zoom (14 §M-5 polish): `Fit` letterboxes into the available
        // area exactly as before; `Actual` shows the frame at native pixels,
        // draggable when it overflows the content area.
        let mut zoom = self.monitor_zoom_state(ctx);
        let video_size = match zoom.mode {
            ImageZoomMode::Fit => {
                let avail_aspect = avail_w / avail_h;
                if avail_aspect > target_aspect {
                    egui::vec2(avail_h * target_aspect, avail_h) // pillarbox
                } else {
                    egui::vec2(avail_w, avail_w / target_aspect) // letterbox
                }
            }
            ImageZoomMode::Actual => egui::vec2(format.width as f32, format.height as f32),
        };
        if zoom.mode == ImageZoomMode::Fit {
            zoom.pan = egui::Vec2::ZERO;
        } else {
            // Pan-to-drag, gated to `Actual` so it never competes with the
            // reframe handles' own drag sense over the default Fit view.
            let pan_resp = ui.interact(
                content_rect,
                ui.id().with("monitor_pan_drag"),
                egui::Sense::drag(),
            );
            if pan_resp.dragged() {
                zoom.pan += pan_resp.drag_delta();
            }
            // Clamp so a drag can't push the frame fully out of view — a
            // margin stays reachable even once it's mostly offscreen.
            let max_pan_x = ((video_size.x - avail_w).max(0.0)) / 2.0 + 40.0;
            let max_pan_y = ((video_size.y - avail_h).max(0.0)) / 2.0 + 40.0;
            zoom.pan.x = zoom.pan.x.clamp(-max_pan_x, max_pan_x);
            zoom.pan.y = zoom.pan.y.clamp(-max_pan_y, max_pan_y);
            if pan_resp.hovered() || pan_resp.dragged() {
                ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grab);
            }
        }
        self.set_monitor_zoom_state(ctx, zoom);
        let video_rect = egui::Rect::from_center_size(content_rect.center() + zoom.pan, video_size);
        // Clipped to `content_rect` so a zoomed-in (Actual) frame can't paint
        // over the format/scrub/transport bars above and below it.
        let content_painter = ui.painter_at(content_rect);
        content_painter.rect_filled(video_rect, 0.0, egui::Color32::from_rgb(24, 24, 28));

        // ── EngineFrame presentation (03 §5) ────────────────────────────────
        // `EngineBridge::present_latest` (run by the host each frame, before
        // egui) has already presented the newest frame into a registered
        // native texture; paint it cropped to the format's logical size (the
        // engine texture is pool-bucket padded — facade note).
        let mut drew_frame = false;
        if let Some(bridge) = &self.engine {
            if let (Some(tex), Some((_, fseq))) = (&bridge.monitor_tex, bridge.presented_frame) {
                let active = doc.timeline.as_ref().and_then(|p| p.active_sequence);
                if active == Some(fseq) {
                    let uv = engine::padded_uv((format.width, format.height), tex.physical);
                    content_painter.image(tex.id, video_rect, uv, egui::Color32::WHITE);
                    drew_frame = true;
                }
            }
        }

        if self.monitor_safe_area {
            draw_safe_area_guides(&content_painter, video_rect);
        }

        // Reframe transform handles (04 §3.3, 05 §4.2, CAP-012) — same overlay
        // family as the safe-area guides just above, drawn/driven by the
        // export-dialog story's `app/reframe.rs` (real, undoable edits via
        // `ops::set_clip_prop`, not a preview-only gizmo).
        super::reframe::draw_reframe_handles(
            ui,
            video_rect,
            doc,
            history,
            &self.timeline_selection,
        );

        if self.engine.is_none() {
            painter.text(
                video_rect.center(),
                egui::Align2::CENTER_CENTER,
                "No preview — video engine unavailable on this host",
                egui::FontId::proportional(15.0),
                egui::Color32::from_gray(130),
            );
        }

        // ── Buffering spinner + engine error surface (04 §3.3) ──────────────
        if let Some(bridge) = &self.engine {
            let status = bridge.status();
            let tpf = active_frame_rate(doc).ticks_per_frame().0.max(1);
            let presented_time = bridge.presented_frame.map(|(t, _)| t);
            let buffering = !drew_frame && status.playing
                || engine::is_buffering(status.playing, status.playhead, presented_time, tpf, 4);
            if buffering {
                let spinner_rect = egui::Rect::from_center_size(
                    video_rect.center(),
                    egui::vec2(28.0, 28.0),
                );
                ui.put(spinner_rect, egui::Spinner::new().size(26.0));
            } else if !drew_frame && !sequence_has_clips(doc) {
                // Fresh/empty project — invite the first action instead of a
                // blank monitor (first-impression affordance).
                painter.text(
                    video_rect.center() + egui::vec2(0.0, -9.0),
                    egui::Align2::CENTER_CENTER,
                    format!("{}  Import media to begin", ph::FILM_STRIP),
                    egui::FontId::proportional(16.0),
                    egui::Color32::from_gray(150),
                );
                painter.text(
                    video_rect.center() + egui::vec2(0.0, 15.0),
                    egui::Align2::CENTER_CENTER,
                    "Use the Media panel or drop files, then drag clips onto the timeline below",
                    egui::FontId::proportional(12.0),
                    egui::Color32::from_gray(110),
                );
            }
            if let Some(err) = &status.last_error {
                painter.text(
                    video_rect.left_bottom() + egui::vec2(6.0, -6.0),
                    egui::Align2::LEFT_BOTTOM,
                    err,
                    egui::FontId::proportional(12.0),
                    egui::Color32::from_rgb(235, 130, 100),
                );
            }
        }

        self.draw_monitor_scrubber(ui, scrub_rect, doc);
        self.draw_transport_bar(ui, transport_rect, doc, history);
        self.draw_format_bar(ui, format_rect, doc, history);
        self.draw_video_shortcut_sheet(ctx);
    }

    /// Aspect/frame bar above the monitor (CAP-012): one-click preset chips to
    /// switch the sequence between 16:9 / 9:16 / 1:1 / 4:5 / 4:3 / 21:9, the
    /// active one highlighted. Clicking a preset activates it (or adds+activates
    /// it if the sequence doesn't have it yet), undoably — so reframing the
    /// whole edit for a different platform is a single, discoverable click.
    /// Also hosts the "Fit clips" auto-reframe button (14 §9/CAP-012,
    /// `app/reframe.rs::fit_clips_to_active_format`) — the CapCut "Auto
    /// reframe" affordance that center-fills the selected clip(s) (or every
    /// clip if none are selected) for whichever format is active above.
    fn draw_format_bar(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        doc: &mut Document,
        history: &mut CommandHistory,
    ) {
        let Some(seq_id) = doc.timeline.as_ref().and_then(|p| p.active_sequence) else {
            return;
        };
        let (cur_w, cur_h) = {
            let f = active_format(doc);
            (f.width, f.height)
        };
        // Snapshot before entering the `FnOnce` ui closure below, which
        // already needs to reborrow `doc`/`history` mutably.
        let selection = self.timeline_selection.clone();
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(format!("{} Frame", ph::CROP)).weak());
                ui.separator();
                let mut clicked: Option<(&str, u32, u32)> = None;
                for &(name, w, h) in super::timeline::ops_bridge::ASPECT_PRESETS {
                    let active = w == cur_w && h == cur_h;
                    if ui
                        .selectable_label(active, name)
                        .on_hover_text(format!("Switch sequence to {name} ({w}×{h})"))
                        .clicked()
                    {
                        clicked = Some((name, w, h));
                    }
                }
                if let Some((name, w, h)) = clicked {
                    super::timeline::ops_bridge::switch_to_aspect(history, doc, seq_id, name, w, h);
                }

                ui.separator();
                if ui
                    .button(format!("{} Fit clips", ph::FRAME_CORNERS))
                    .on_hover_text(
                        "Auto-frame the selected clip(s) — or every clip if none are \
                         selected — to fill the current aspect (center-fill, one undo step)",
                    )
                    .clicked()
                {
                    super::reframe::fit_clips_to_active_format(doc, history, &selection);
                }

                // Playback resolution + image zoom (14 §M-5 polish):
                // right-aligned so they read as a distinct group from the
                // aspect/reframe controls above. Added in reverse (right_to_
                // left) order so Resolution reads before Zoom, left to right.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.draw_zoom_selector(ui);
                    ui.separator();
                    self.draw_resolution_selector(ui);
                });
            });
        });
    }

    /// Playback-resolution selector (14 §M-5): Full / 1/2 / 1/4 preview
    /// scale. See [`PlaybackResolution`] for the engine-side mapping.
    fn draw_resolution_selector(&mut self, ui: &mut egui::Ui) {
        let mut res = self.monitor_playback_resolution(ui.ctx());
        let prev = res;
        egui::ComboBox::from_id_salt("monitor_playback_resolution")
            .selected_text(res.label())
            .width(48.0)
            .show_ui(ui, |ui| {
                for opt in PlaybackResolution::ALL {
                    ui.selectable_value(&mut res, opt, opt.label());
                }
            })
            .response
            .on_hover_text(
                "Playback resolution — Full plays originals, 1/2 and 1/4 both \
                 preview at the engine's proxy-media scale",
            );
        ui.label(egui::RichText::new(ph::GAUGE).weak())
            .on_hover_text("Playback resolution");
        if res != prev {
            self.set_monitor_playback_resolution(ui.ctx(), res);
        }
    }

    /// Image-zoom selector (14 §M-5): `Fit` (default, letterboxed) or
    /// `Actual` (100%, native pixels, draggable) for the monitor picture.
    fn draw_zoom_selector(&mut self, ui: &mut egui::Ui) {
        let mut zoom = self.monitor_zoom_state(ui.ctx());
        let prev_mode = zoom.mode;
        egui::ComboBox::from_id_salt("monitor_image_zoom")
            .selected_text(match zoom.mode {
                ImageZoomMode::Fit => "Fit",
                ImageZoomMode::Actual => "100%",
            })
            .width(48.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut zoom.mode, ImageZoomMode::Fit, "Fit");
                ui.selectable_value(&mut zoom.mode, ImageZoomMode::Actual, "100%");
            })
            .response
            .on_hover_text("Image zoom — Fit letterboxes, 100% shows native pixels (drag to pan)");
        ui.label(egui::RichText::new(ph::MAGNIFYING_GLASS).weak())
            .on_hover_text("Image zoom");
        if zoom.mode != prev_mode {
            if zoom.mode == ImageZoomMode::Fit {
                zoom.pan = egui::Vec2::ZERO;
            }
            self.set_monitor_zoom_state(ui.ctx(), zoom);
        }
    }

    /// Play/pause/step buttons, timecode readout, loop + safe-area toggles,
    /// and in/out buttons (04 §3.2).
    fn draw_transport_bar(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        doc: &mut Document,
        history: &mut CommandHistory,
    ) {
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.horizontal_centered(|ui| {
                if ui
                    .button(ph::SKIP_BACK)
                    .on_hover_text("Playhead to Start (Home)")
                    .clicked()
                {
                    self.video_playhead_home();
                }
                if ui
                    .button(ph::CARET_LEFT)
                    .on_hover_text("Step Back One Frame (←)")
                    .clicked()
                {
                    self.video_step_back(doc);
                }
                let play_icon = if self.monitor_playing {
                    ph::PAUSE
                } else {
                    ph::PLAY
                };
                if ui
                    .button(play_icon)
                    .on_hover_text("Play / Pause (Space)")
                    .clicked()
                {
                    self.video_play_pause();
                }
                if ui
                    .button(ph::CARET_RIGHT)
                    .on_hover_text("Step Forward One Frame (→)")
                    .clicked()
                {
                    self.video_step_forward(doc);
                }
                if ui
                    .button(ph::SKIP_FORWARD)
                    .on_hover_text("Playhead to End (End)")
                    .clicked()
                {
                    self.video_playhead_end(doc);
                }

                ui.separator();
                if ui
                    .selectable_label(self.monitor_loop_enabled, ph::REPEAT)
                    .on_hover_text("Loop playback")
                    .clicked()
                {
                    self.monitor_loop_enabled = !self.monitor_loop_enabled;
                }

                ui.separator();
                // Prominent current / total timecode readout (pro-NLE feel):
                // large accent-colored playhead time, muted total after it.
                let fr = active_frame_rate(doc);
                let end = sequence_end_tick(doc);
                ui.label(
                    egui::RichText::new(format_timecode(fr, self.playhead))
                        .monospace()
                        .size(16.0)
                        .strong()
                        .color(egui::Color32::from_rgb(0x9d, 0x8c, 0xf5)),
                );
                ui.label(
                    egui::RichText::new(format!("/ {}", format_timecode(fr, end)))
                        .monospace()
                        .weak(),
                );

                ui.separator();
                if ui.button("I").on_hover_text("Set In Point (I)").clicked() {
                    self.video_set_in(doc, history);
                }
                if ui.button("O").on_hover_text("Set Out Point (O)").clicked() {
                    self.video_set_out(doc, history);
                }

                ui.separator();
                if ui
                    .button(ph::BOOKMARK_SIMPLE)
                    .on_hover_text("Add Marker at Playhead (M)")
                    .clicked()
                {
                    self.timeline_add_marker_at_playhead(doc, history);
                }

                ui.separator();
                if ui
                    .selectable_label(self.monitor_safe_area, ph::CROP)
                    .on_hover_text("Safe-area guides")
                    .clicked()
                {
                    self.monitor_safe_area = !self.monitor_safe_area;
                }
            });
        });
    }

    /// Program-monitor scrub bar (NLE parity QW-5): a thin draggable position
    /// strip under the picture, spanning the whole sequence. Click/drag seeks
    /// the single shared `self.playhead` (frame-snapped, like the timeline
    /// ruler) — writing here updates the same playhead the ruler reads, so
    /// there is never a second playhead.
    fn draw_monitor_scrubber(&mut self, ui: &mut egui::Ui, rect: egui::Rect, doc: &Document) {
        let end = sequence_end_tick(doc);
        let painter = ui.painter_at(rect);
        let visuals = ui.visuals();

        // A centered groove with a small horizontal margin so the playhead
        // handle never clips at the extremes.
        const MARGIN_X: f32 = 6.0;
        let track_left = rect.left() + MARGIN_X;
        let track_width = (rect.width() - 2.0 * MARGIN_X).max(1.0);
        let cy = rect.center().y;
        let groove = egui::Rect::from_min_max(
            egui::pos2(track_left, cy - 2.0),
            egui::pos2(track_left + track_width, cy + 2.0),
        );
        painter.rect_filled(groove, 2.0, visuals.faint_bg_color);

        // No content yet → an inert groove (nothing to scrub).
        if end.0 <= 0 {
            return;
        }

        let accent = visuals.selection.stroke.color;
        let head_x = scrub_tick_to_x(self.playhead, track_left, track_width, end);
        let filled = egui::Rect::from_min_max(
            egui::pos2(track_left, cy - 2.0),
            egui::pos2(head_x, cy + 2.0),
        );
        painter.rect_filled(filled, 2.0, accent);
        painter.circle_filled(egui::pos2(head_x, cy), 5.0, accent);

        // Click/drag anywhere on the strip seeks (frame-snapped).
        let resp = ui.interact(
            rect,
            ui.id().with("monitor_scrubber"),
            egui::Sense::click_and_drag(),
        );
        if resp.clicked() || resp.dragged() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let fr = active_frame_rate(doc);
                self.playhead = scrub_x_to_tick(pos.x, track_left, track_width, end, fr);
            }
        }
        if resp.hovered() {
            ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::ResizeHorizontal);
        }
    }

    /// Video-mode keyboard dispatch (04 §5.1) — the sibling block to the
    /// vector-canvas space-pan/arrow-nudge/WASD-pan input at the resolution
    /// rule in §5.2: mutually exclusive by construction because this is only
    /// ever called from [`Self::draw_video_monitor`], which the central-panel
    /// branch (04 §1.1 point 3) only reaches when `self.mode == Video` — the
    /// vector blocks are simply never reached that frame (`app/mod.rs`'s
    /// central-panel `if self.mode == AppMode::Video { ...; return; }`).
    fn handle_video_keyboard(
        &mut self,
        ctx: &egui::Context,
        doc: &mut Document,
        history: &mut CommandHistory,
    ) {
        if !viewport_kb(ctx) {
            return;
        }
        const KEYS: &[commands::CommandId] = &[
            "video.play_pause",
            "video.play_reverse",
            "video.pause",
            "video.play_forward",
            "video.step_back",
            "video.step_forward",
            "video.prev_edit_point",
            "video.next_edit_point",
            "video.set_in",
            "video.set_out",
            "video.split_at_playhead",
            "video.delete_clip",
            "video.ripple_delete",
            "video.copy",
            "video.cut",
            "video.paste",
            "video.add_marker",
            "video.insert_edit",
            "video.overwrite_edit",
            "video.lift_edit",
            "video.extract_edit",
            "video.toggle_razor",
            "video.toggle_snap",
            "video.zoom_in",
            "video.zoom_out",
            "video.zoom_fit",
            "video.playhead_home",
            "video.playhead_end",
        ];
        for &id in KEYS {
            if self.binding_pressed(ctx, id) {
                self.dispatch_command(id, doc, history);
            }
        }
        // Backspace is a second hardwired delete accelerator alongside the
        // remappable `video.delete_clip`/`video.ripple_delete` (Delete /
        // Shift+Delete) — the keymap holds only one binding per command, so the
        // NLE-standard Backspace-to-delete is matched here like the `?` sheet
        // key below. Shift = ripple; other modifiers are ignored so it never
        // collides with a text edit or a chorded shortcut.
        let backspace_delete = ctx.input(|i| {
            i.key_pressed(egui::Key::Backspace)
                && !i.modifiers.ctrl
                && !i.modifiers.command
                && !i.modifiers.mac_cmd
                && !i.modifiers.alt
        });
        if backspace_delete {
            let ripple = ctx.input(|i| i.modifiers.shift);
            let id = if ripple {
                "video.ripple_delete"
            } else {
                "video.delete_clip"
            };
            self.dispatch_command(id, doc, history);
        }
        // `?` opens the shortcut sheet (04 §1.2) — not a rebindable CommandId,
        // matching the existing Escape/palette-open pattern of a few hardcoded
        // global keys elsewhere in this file.
        let opens_sheet = ctx.input(|i| {
            i.key_pressed(egui::Key::Questionmark)
                || (i.modifiers.shift && i.key_pressed(egui::Key::Slash))
        });
        if opens_sheet {
            self.show_video_shortcut_sheet = true;
        }
    }
}

/// Map a tick in `[0, end]` to an x within `[left, left + width]` for the
/// monitor scrub bar (QW-5). Clamps to the track; `end <= 0` pins to the left.
fn scrub_tick_to_x(t: Tick, left: f32, width: f32, end: Tick) -> f32 {
    if end.0 <= 0 {
        return left;
    }
    let frac = (t.0 as f32 / end.0 as f32).clamp(0.0, 1.0);
    left + frac * width
}

/// Map a pointer x to a frame-snapped tick in `[0, end]` for the monitor scrub
/// bar (QW-5). Inverse of [`scrub_tick_to_x`], then snapped to the frame grid.
fn scrub_x_to_tick(x: f32, left: f32, width: f32, end: Tick, fr: FrameRate) -> Tick {
    if width <= 0.0 || end.0 <= 0 {
        return Tick::ZERO;
    }
    let frac = ((x - left) / width).clamp(0.0, 1.0);
    let raw = Tick((frac as f64 * end.0 as f64).round() as i64);
    fr.snap(raw).clamp(Tick::ZERO, end)
}

/// Action-safe (90%) / title-safe (80%) guide rectangles (04 §3.3).
fn draw_safe_area_guides(painter: &egui::Painter, video_rect: egui::Rect) {
    let action_safe = video_rect.shrink2(video_rect.size() * 0.05);
    let title_safe = video_rect.shrink2(video_rect.size() * 0.10);
    painter.rect_stroke(
        action_safe,
        0.0,
        egui::Stroke::new(
            1.0,
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 110),
        ),
    );
    painter.rect_stroke(
        title_safe,
        0.0,
        egui::Stroke::new(
            1.0,
            egui::Color32::from_rgba_unmultiplied(255, 200, 80, 130),
        ),
    );
}

// ── First-run hints (04 §1.2) ────────────────────────────────────────────────

impl PhotonicApp {
    /// One-time toolbar callout pointing at the Video toggle, anchored below
    /// its rect. Persisted-dismissed via `prefs.video_hint_dismissed`.
    pub(crate) fn draw_video_hint_callout(&mut self, ctx: &egui::Context, anchor: egui::Rect) {
        if self.prefs.video_hint_dismissed || self.mode == AppMode::Video {
            return;
        }
        egui::Area::new(egui::Id::new("video_mode_hint_callout"))
            .order(egui::Order::Foreground)
            .fixed_pos(anchor.left_bottom() + egui::vec2(0.0, 6.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_max_width(240.0);
                    ui.label("New: edit video timelines — click here or Ctrl+Shift+V");
                    if ui.small_button("Got it").clicked() {
                        self.prefs.video_hint_dismissed = true;
                        self.prefs.save();
                    }
                });
            });
    }

    /// One-time shortcut-sheet overlay (04 §1.2), auto-opened on first video-
    /// mode entry and re-openable via `?` thereafter.
    pub(crate) fn draw_video_shortcut_sheet(&mut self, ctx: &egui::Context) {
        if !self.show_video_shortcut_sheet {
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.show_video_shortcut_sheet = false;
            return;
        }
        let mut open = true;
        egui::Window::new("Video Mode Shortcuts")
            .id(egui::Id::new("video_shortcut_sheet"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_width(300.0);
                egui::Grid::new("video_shortcut_grid")
                    .num_columns(2)
                    .spacing([16.0, 6.0])
                    .show(ui, |ui| {
                        // Kept in sync with the `video.*` default bindings in
                        // `commands.rs` — verify there before editing here.
                        const ROWS: &[(&str, &str)] = &[
                            ("Space", "Play / Pause"),
                            ("J / K / L", "Play reverse / Pause / Play forward"),
                            ("← / →", "Step one frame"),
                            ("Shift+← / Shift+→", "Previous / next edit point"),
                            ("I / O", "Set in / out point"),
                            ("S", "Split clip at playhead"),
                            ("Del / Backspace", "Delete selected clip (Shift = ripple)"),
                            ("Ctrl+C / X / V", "Copy / cut / paste clip"),
                            ("M", "Add marker at playhead"),
                            ("N", "Toggle snapping"),
                            ("?", "Show this sheet again"),
                        ];
                        for (key, desc) in ROWS {
                            ui.strong(*key);
                            ui.label(*desc);
                            ui.end_row();
                        }
                    });
            });
        if !open {
            self.show_video_shortcut_sheet = false;
        }
    }
}

#[cfg(test)]
mod scrubber_tests {
    use super::*;

    #[test]
    fn tick_to_x_maps_endpoints_and_midpoint() {
        // 0 → left, end → right, half → middle.
        assert_eq!(scrub_tick_to_x(Tick(0), 100.0, 200.0, Tick(1000)), 100.0);
        assert_eq!(scrub_tick_to_x(Tick(1000), 100.0, 200.0, Tick(1000)), 300.0);
        assert_eq!(scrub_tick_to_x(Tick(500), 100.0, 200.0, Tick(1000)), 200.0);
    }

    #[test]
    fn tick_to_x_clamps_out_of_range_and_handles_empty() {
        // Past the end clamps to the right edge; empty sequence pins to left.
        assert_eq!(scrub_tick_to_x(Tick(5000), 0.0, 200.0, Tick(1000)), 200.0);
        assert_eq!(scrub_tick_to_x(Tick(500), 40.0, 200.0, Tick(0)), 40.0);
    }

    #[test]
    fn x_to_tick_is_frame_snapped_and_clamped() {
        let fr = FrameRate::FPS_30;
        // One second of content.
        let end = Tick(30 * TICKS_PER_SECOND);
        // Clicking left of the track clamps to 0.
        assert_eq!(scrub_x_to_tick(-50.0, 0.0, 300.0, end, fr), Tick::ZERO);
        // Clicking past the right edge clamps to the (frame-snapped) end.
        let right = scrub_x_to_tick(9999.0, 0.0, 300.0, end, fr);
        assert!(right <= end);
        assert_eq!(right, fr.snap(right), "result must sit on a frame boundary");
        // A mid-track click snaps to a frame boundary and stays in range.
        let mid = scrub_x_to_tick(150.0, 0.0, 300.0, end, fr);
        assert_eq!(mid, fr.snap(mid));
        assert!(mid > Tick::ZERO && mid < end);
    }

    #[test]
    fn x_to_tick_handles_degenerate_track_width() {
        let fr = FrameRate::FPS_30;
        assert_eq!(scrub_x_to_tick(10.0, 0.0, 0.0, Tick(1000), fr), Tick::ZERO);
    }
}
