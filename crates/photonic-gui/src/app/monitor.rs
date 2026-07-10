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

        // Reserve a top strip for the aspect/frame bar (CAP-012) and a bottom
        // strip for the transport bar (04 §3.2). The video image sits between.
        const FORMAT_H: f32 = 30.0;
        const TRANSPORT_H: f32 = 40.0;
        let format_rect = egui::Rect::from_min_max(
            rect.min,
            egui::pos2(rect.max.x, (rect.min.y + FORMAT_H).min(rect.max.y)),
        );
        let content_rect = egui::Rect::from_min_max(
            egui::pos2(rect.min.x, format_rect.max.y),
            egui::pos2(rect.max.x, (rect.max.y - TRANSPORT_H).max(format_rect.max.y)),
        );
        let transport_rect =
            egui::Rect::from_min_max(egui::pos2(rect.min.x, content_rect.max.y), rect.max);

        // Background + letterbox/pillarbox bars (04 §3.3).
        painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(12, 12, 14));
        let target_aspect = format.width.max(1) as f32 / format.height.max(1) as f32;
        let avail_w = content_rect.width().max(1.0);
        let avail_h = content_rect.height().max(1.0);
        let avail_aspect = avail_w / avail_h;
        let video_size = if avail_aspect > target_aspect {
            egui::vec2(avail_h * target_aspect, avail_h) // pillarbox
        } else {
            egui::vec2(avail_w, avail_w / target_aspect) // letterbox
        };
        let video_rect = egui::Rect::from_center_size(content_rect.center(), video_size);
        painter.rect_filled(video_rect, 0.0, egui::Color32::from_rgb(24, 24, 28));

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
                    painter.image(tex.id, video_rect, uv, egui::Color32::WHITE);
                    drew_frame = true;
                }
            }
        }

        if self.monitor_safe_area {
            draw_safe_area_guides(&painter, video_rect);
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

        self.draw_transport_bar(ui, transport_rect, doc, history);
        self.draw_format_bar(ui, format_rect, doc, history);
        self.draw_video_shortcut_sheet(ctx);
    }

    /// Aspect/frame bar above the monitor (CAP-012): one-click preset chips to
    /// switch the sequence between 16:9 / 9:16 / 1:1 / 4:5 / 4:3 / 21:9, the
    /// active one highlighted. Clicking a preset activates it (or adds+activates
    /// it if the sequence doesn't have it yet), undoably — so reframing the
    /// whole edit for a different platform is a single, discoverable click.
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
            });
        });
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
                    .selectable_label(self.monitor_safe_area, ph::CROP)
                    .on_hover_text("Safe-area guides")
                    .clicked()
                {
                    self.monitor_safe_area = !self.monitor_safe_area;
                }
            });
        });
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
