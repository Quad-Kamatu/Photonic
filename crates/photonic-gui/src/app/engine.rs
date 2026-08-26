//! GUI ↔ `photonic_video::VideoEngine` bridge (Wire phase of 02 §1 / 03 §5).
//!
//! Three jobs live here, all host-side so `photonic-video` stays untouched:
//!
//! 1. **Lock-flavor bridge.** The app's live `Document`/`CommandHistory` sit
//!    under `tokio::sync::Mutex` (shared with the MCP server), but
//!    `VideoEngine::open_session` snapshots through `std::sync::Mutex`. The
//!    bridge owns a std-mutexed *mirror* pair; [`EngineBridge::sync_document`]
//!    copies `doc.timeline` into the mirror whenever the real history revision
//!    moves and bumps the mirror history's revision (`CommandHistory::reset` —
//!    documented to bump `revision`, and the mirror's stacks are always empty
//!    so it is otherwise a no-op). The engine thread then re-snapshots exactly
//!    as if it were watching the real document.
//!
//! 2. **Presentation.** [`EngineBridge::present_latest`] runs the normative
//!    `EngineFrame`→screen present pass (03 §5,
//!    `photonic_render::video::VideoPresenter`) into an intermediate
//!    `Rgba8UnormSrgb` texture registered with egui as a native texture; the
//!    monitor paints it with a UV rect cropped to the frame's logical size
//!    (the engine's frame textures are pool-bucket padded — see the facade
//!    notes in `photonic_video::session`).
//!
//! 3. **Desired-state reconciliation.** Transport methods only mutate GUI
//!    intent (`monitor_playing`, playhead, loop toggle, proxy mode);
//!    `PhotonicApp::drive_playback` (in `app/monitor.rs`) diffs intent against
//!    what was last sent, via the `set_*`/`seek`/`step` methods here, and
//!    emits the minimal `EngineCmd` stream (Play/Pause/Seek/SetLoop/
//!    SetActiveSequence/SetProxyMode). JKL shuttle (reverse / >1× speed) has
//!    no engine-side primitive yet, so it scrubs via coalesced `Seek`s while
//!    the engine stays paused (documented seam: reverse/ramped playback with
//!    audio belongs to the speed-maps story, P8).
//!
//! Timeline clip thumbnails and audio waveform strips are a documented seam:
//! the engine exposes no per-clip decoded-frame or PCM access yet (its
//! `EngineFrame` is program-out only), so wiring them waits for the decode-
//! pool / waveform-pyramid follow-up rather than spawning ad-hoc ffmpeg runs
//! per visible clip here.
//!
//! The master-output meter beside the program monitor (`app/monitor.rs`,
//! NLE-parity Gap G-4) is another documented seam: [`EngineBridge::master_level`]
//! is the accessor the monitor polls, but it returns `None` today because
//! neither `EngineSession` nor `EngineStatus` (`photonic_video::session`)
//! surface a level yet — see that method's doc for the exact shape of the
//! fix (out of this story's territory, `app/{monitor.rs,engine.rs}` only).

use photonic_core::document::Document;
use photonic_core::history::CommandHistory;
use photonic_core::timeline::{SequenceId, Tick};
use photonic_render::video::{PresentChannel, VideoPresenter};
use photonic_video::{
    EngineCmd, EngineSession, EngineStatus, PreviewQuality, PreviewTarget, ProxyMode, VideoEngine,
};
use std::sync::{Arc, Mutex as StdMutex};

/// The registered egui texture for the current engine frame.
pub(crate) struct MonitorTexture {
    pub id: egui::TextureId,
    /// Physical (pool-bucket-padded) size of the presented texture.
    pub physical: (u32, u32),
}

/// Master-bus output level, `[L, R]` linear amplitude, as
/// [`EngineBridge::master_level`] would report it once the engine surfaces
/// one — see that method's doc for the seam this type is shaped to close.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub(crate) struct MasterLevel {
    pub peak: [f32; 2],
    pub rms: [f32; 2],
}

/// Identity of the last-presented frame, so an unchanged frame isn't
/// re-presented every GUI frame.
type FrameKey = (Tick, SequenceId, usize);

/// GUI-side handle to a running engine session plus everything the monitor
/// needs to feed and reflect it. `None` on `PhotonicApp` means "no engine"
/// (tests, GPU-less hosts) and every caller falls back to the pre-P3
/// wall-clock placeholder paths.
pub struct EngineBridge {
    /// Kept alive for the session's lifetime (owns the shared `GpuContext`).
    #[allow(dead_code)]
    engine: VideoEngine,
    pub(crate) session: EngineSession,

    // ── Snapshot mirror (lock-flavor bridge) ────────────────────────────────
    mirror_doc: Arc<StdMutex<Document>>,
    mirror_history: Arc<StdMutex<CommandHistory>>,
    last_synced_revision: Option<u64>,

    // ── Presentation ────────────────────────────────────────────────────────
    presenter: Option<VideoPresenter>,
    target: Option<PresentTarget>,
    /// K-B5 clean (no clip looks) present target, when compare is on.
    compare_target: Option<PresentTarget>,
    presented: Option<FrameKey>,
    pub(crate) monitor_tex: Option<MonitorTexture>,
    /// K-B5 clean side texture (bypassed clip looks).
    pub(crate) compare_tex: Option<MonitorTexture>,
    /// The frame the monitor texture currently shows (time + sequence), for
    /// the buffering heuristic.
    pub(crate) presented_frame: Option<(Tick, SequenceId)>,
    /// Logical size of the frame currently in `monitor_tex`; unlike the active
    /// sequence format this also remains correct for an asset source peek.
    pub(crate) presented_logical_size: Option<(u32, u32)>,
    /// K-B17: colour vs alpha-as-luminance present channel.
    pub(crate) present_channel: PresentChannel,
    /// K-B5: dual clean/graded compare requested.
    pub(crate) compare_effects: bool,
    /// K-B5: vertical split fraction (0=all clean left, 1=all graded).
    pub(crate) compare_split: f32,

    // ── Reconciler state (last values actually sent to the engine) ──────────
    sent_playing: Option<bool>,
    sent_loop: Option<Option<(Tick, Tick)>>,
    sent_sequence: Option<SequenceId>,
    sent_proxy: Option<ProxyMode>,
    sent_preview_quality: Option<PreviewQuality>,
    sent_preview_target: Option<PreviewTarget>,
    /// Playhead value the GUI and engine last agreed on — a differing
    /// `self.playhead` means the *user* moved it (ruler scrub, Home/End,
    /// marker jump) and a `Seek` must be sent.
    pub(crate) agreed_playhead: Option<Tick>,
    /// GUI-side proxy-mode intent (media pool toggle).
    pub(crate) proxy_mode: ProxyMode,
    /// Draft (default) / Full interactive quality (24 §4).
    pub(crate) preview_quality: PreviewQuality,
    /// Desired single-monitor target; play-wins enforced by engine (24 §3).
    pub(crate) preview_target: PreviewTarget,
}

struct PresentTarget {
    #[allow(dead_code)]
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: (u32, u32),
    egui_id: Option<egui::TextureId>,
}

impl EngineBridge {
    /// Build a bridge sharing the windowed renderer's wgpu device/queue
    /// (02 §1 "GUI mode shares the winit GpuContext"). This is the one-line
    /// construction hosts call right after the renderer exists.
    pub fn from_renderer(renderer: &photonic_render::PhotonicRenderer) -> Self {
        let gpu = photonic_video::GpuContext::new(renderer.device_arc(), renderer.queue_arc());
        EngineBridge::new(VideoEngine::new(gpu))
    }

    /// Open a session against fresh mirror state. The mirror starts empty; the
    /// first [`Self::sync_document`] populates it.
    pub fn new(engine: VideoEngine) -> Self {
        let mirror_doc = Arc::new(StdMutex::new(Document::new("engine-mirror", 1.0, 1.0)));
        let mirror_history = Arc::new(StdMutex::new(CommandHistory::new(1)));
        let session = engine.open_session(Arc::clone(&mirror_doc), Arc::clone(&mirror_history));
        EngineBridge {
            engine,
            session,
            mirror_doc,
            mirror_history,
            last_synced_revision: None,
            presenter: None,
            target: None,
            compare_target: None,
            presented: None,
            monitor_tex: None,
            compare_tex: None,
            presented_frame: None,
            presented_logical_size: None,
            present_channel: PresentChannel::Color,
            compare_effects: false,
            compare_split: 0.5,
            sent_playing: None,
            sent_loop: None,
            sent_sequence: None,
            sent_proxy: None,
            sent_preview_quality: None,
            sent_preview_target: None,
            agreed_playhead: None,
            proxy_mode: ProxyMode::Auto,
            preview_quality: PreviewQuality::Draft,
            preview_target: PreviewTarget::default(),
        }
    }

    /// Wait-free engine status for this frame.
    pub fn status(&self) -> Arc<EngineStatus> {
        self.session.status()
    }

    /// Shared wgpu device for background export workers (K-F1 render queue).
    pub fn gpu(&self) -> &photonic_video::GpuContext {
        self.engine.gpu()
    }

    /// The engine's real master-bus output level (NLE-parity Gap G-4: the
    /// slim stereo peak+RMS meter beside the program monitor,
    /// `app/monitor.rs::draw_master_meter`). Linear peak/RMS amplitude per
    /// channel `[L, R]` — the same unit
    /// [`photonic_video::audio::StereoMeter::peak`]/`::rms` publish, so a
    /// caller converts to dB itself (exactly like
    /// `panels/video/audio_mixer.rs`'s master strip does).
    ///
    /// G-4 closed: reads `EngineStatus.master_level` published from the mixer
    /// feeder's live `StereoMeter`. `None` while paused / no audio device —
    /// callers render the silence floor honestly (no fabricated motion).
    pub(crate) fn master_level(&self) -> Option<MasterLevel> {
        self.session.status().master_level.map(|m| MasterLevel {
            peak: m.peak,
            rms: m.rms,
        })
    }

    /// The raw session handle (command sends, frame polls) for hosts/tests.
    pub fn session(&self) -> &EngineSession {
        &self.session
    }

    /// Physical size + egui id of the registered monitor texture, if any.
    pub fn monitor_tex_info(&self) -> Option<((u32, u32), egui::TextureId)> {
        self.monitor_tex.as_ref().map(|t| (t.physical, t.id))
    }

    /// Copy `doc.timeline` into the engine-visible mirror when the real
    /// history revision moved. Contention on either mirror lock (the engine
    /// thread snapshots with `try_lock` and holds only briefly) just retries
    /// next frame — `last_synced_revision` is only advanced on success.
    pub fn sync_document(&mut self, doc: &Document, history: &CommandHistory) {
        let rev = history.revision();
        if self.last_synced_revision == Some(rev) {
            return;
        }
        let (Ok(mut d), Ok(mut h)) = (self.mirror_doc.try_lock(), self.mirror_history.try_lock())
        else {
            return;
        };
        d.timeline = doc.timeline.clone();
        // Cheap public revision bump: the mirror's stacks are always empty, so
        // `reset` only clears empty collections and increments `revision`,
        // which is exactly the signal `EngineThread::poll_snapshot` watches.
        h.reset();
        drop(h);
        drop(d);
        self.last_synced_revision = Some(rev);
    }

    // ── Reconciliation ───────────────────────────────────────────────────────

    /// D-12: ask the engine to analyze `clip` and warm its stabilization cache
    /// (22 §6.5).
    ///
    /// Not reconciled like the `set_*` senders below: this is an explicit user
    /// action, and re-running it on unchanged input is exactly what the
    /// Reanalyze button is for.
    pub(crate) fn send_analyze_stabilization(&self, clip: photonic_core::timeline::ClipId) {
        self.session.send(EngineCmd::AnalyzeStabilization { clip });
    }

    /// Send `cmd` kinds only when the desired value changed since last send.
    pub(crate) fn set_playing(&mut self, playing: bool) {
        if self.sent_playing != Some(playing) {
            self.session.send(if playing {
                EngineCmd::Play
            } else {
                EngineCmd::Pause
            });
            self.sent_playing = Some(playing);
        }
    }

    pub(crate) fn set_loop(&mut self, range: Option<(Tick, Tick)>) {
        if self.sent_loop != Some(range) {
            self.session.send(EngineCmd::SetLoop(range));
            self.sent_loop = Some(range);
        }
    }

    pub(crate) fn set_active_sequence(&mut self, seq: Option<SequenceId>) {
        if let Some(seq) = seq {
            if self.sent_sequence != Some(seq) {
                self.session.send(EngineCmd::SetActiveSequence(seq));
                self.sent_sequence = Some(seq);
            }
        }
    }

    pub(crate) fn apply_proxy_mode(&mut self) {
        if self.sent_proxy != Some(self.proxy_mode) {
            self.session.send(EngineCmd::SetProxyMode(self.proxy_mode));
            self.sent_proxy = Some(self.proxy_mode);
        }
    }

    pub(crate) fn apply_preview_quality(&mut self) {
        if self.sent_preview_quality != Some(self.preview_quality) {
            self.session
                .send(EngineCmd::SetPreviewQuality(self.preview_quality));
            self.sent_preview_quality = Some(self.preview_quality);
        }
    }

    pub(crate) fn apply_preview_target(&mut self) {
        if self.sent_preview_target.as_ref() != Some(&self.preview_target) {
            self.session
                .send(EngineCmd::SetPreviewTarget(self.preview_target.clone()));
            self.sent_preview_target = Some(self.preview_target.clone());
        }
    }

    /// Peek a media-pool asset on the single monitor (24 §3). No-op while
    /// playing (engine enforces play-wins).
    pub(crate) fn peek_asset(&mut self, asset: photonic_core::timeline::AssetId, time: Tick) {
        self.preview_target = PreviewTarget::Asset {
            asset,
            source_time: time,
        };
        self.apply_preview_target();
    }

    /// Seek within a source peek (source clock; G-10 / 24 §3.3).
    pub(crate) fn seek_source(&mut self, asset: photonic_core::timeline::AssetId, time: Tick) {
        self.preview_target = PreviewTarget::Asset {
            asset,
            source_time: time,
        };
        self.session.send(EngineCmd::SeekSource { asset, time });
        self.apply_preview_target();
    }

    /// True when the single monitor is showing a source peek.
    pub(crate) fn preview_is_asset(&self) -> bool {
        matches!(self.preview_target, PreviewTarget::Asset { .. })
    }

    /// Return the monitor to sequence program view.
    pub(crate) fn peek_sequence(&mut self, sequence: SequenceId) {
        self.preview_target = PreviewTarget::Sequence { sequence };
        self.apply_preview_target();
    }

    /// Seek and record agreement so the scrub detector stays quiet.
    pub(crate) fn seek(&mut self, to: Tick) {
        self.session.send(EngineCmd::Seek(to));
        self.agreed_playhead = Some(to);
    }

    /// Live scrub target while the playhead is being dragged: decodes a cheap
    /// keyframe preview. Records agreement like `seek`; the drag-release settle
    /// sends a real `seek` to land the exact frame.
    pub(crate) fn scrub_seek(&mut self, to: Tick) {
        self.session.send(EngineCmd::ScrubSeek(to));
        self.agreed_playhead = Some(to);
    }

    /// Exact-frame step; the engine pauses itself (02 §4). The caller updates
    /// its optimistic local playhead and then records agreement via
    /// [`Self::note_agreed`].
    pub(crate) fn step(&mut self, frames: i32) {
        self.session.send(EngineCmd::Step(frames));
        self.sent_playing = Some(false);
    }

    pub(crate) fn note_agreed(&mut self, playhead: Tick) {
        self.agreed_playhead = Some(playhead);
    }

    // ── Presentation (03 §5) ─────────────────────────────────────────────────

    /// Present the newest `EngineFrame` (if it changed) into the egui-visible
    /// intermediate texture. Called from the host render loop *before*
    /// `egui::Context::run`, so the registered texture is valid for this
    /// frame's paint pass.
    pub fn present_latest(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        egui_renderer: &mut egui_wgpu::Renderer,
    ) {
        let Some(frame) = self.session.latest_frame() else {
            return;
        };
        // Include the present channel so toggling alpha view re-presents the
        // same frame through the alpha pipeline (K-B17).
        let clean_ptr = frame
            .compare_clean
            .as_ref()
            .map(|t| Arc::as_ptr(t) as usize)
            .unwrap_or(0);
        let key: FrameKey = (
            frame.time,
            frame.sequence,
            Arc::as_ptr(&frame.texture) as usize
                ^ (self.present_channel as usize).wrapping_mul(0x9e37_79b9)
                ^ clean_ptr.wrapping_mul(0x85eb_ca6b)
                ^ (self.compare_effects as usize)
                ^ (frame.logical_size.0 as usize).wrapping_mul(0x27d4_eb2d)
                ^ (frame.logical_size.1 as usize).wrapping_mul(0x1656_67b1),
        );
        if self.presented == Some(key) {
            return;
        }
        let size = (frame.texture.width(), frame.texture.height());
        self.ensure_target(device, egui_renderer, size, false);
        if frame.compare_clean.is_some() {
            self.ensure_target(device, egui_renderer, size, true);
        } else {
            self.compare_tex = None;
            if let Some(old) = self.compare_target.take() {
                if let Some(id) = old.egui_id {
                    egui_renderer.free_texture(&id);
                }
            }
        }
        if self.presenter.is_none() {
            self.presenter = Some(VideoPresenter::new(
                device,
                wgpu::TextureFormat::Rgba8UnormSrgb,
            ));
        }
        let channel = self.present_channel;
        let src_view = frame.texture.create_view(&Default::default());
        let clean_view = frame
            .compare_clean
            .as_ref()
            .map(|t| t.create_view(&Default::default()));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("engine_frame_present"),
        });
        {
            let presenter = self.presenter.as_ref().expect("presenter set above");
            let target = self.target.as_ref().expect("ensure_target sets target");
            presenter.present_engine_frame_channel(
                device,
                &mut encoder,
                &src_view,
                &target.view,
                channel,
            );
            if let (Some(cv), Some(ct)) = (clean_view.as_ref(), self.compare_target.as_ref()) {
                presenter.present_engine_frame_channel(device, &mut encoder, cv, &ct.view, channel);
            }
        }
        queue.submit([encoder.finish()]);

        self.presented = Some(key);
        self.presented_frame = Some((frame.time, frame.sequence));
        self.presented_logical_size = Some(frame.logical_size);
    }

    /// Toggle alpha-as-luminance present (K-B17). Forces the next
    /// [`present_latest`] to re-encode the current frame.
    pub fn toggle_alpha_view(&mut self) {
        self.present_channel = match self.present_channel {
            PresentChannel::Color => PresentChannel::Alpha,
            PresentChannel::Alpha => PresentChannel::Color,
        };
        self.presented = None; // force re-present
    }

    pub fn alpha_view(&self) -> bool {
        self.present_channel == PresentChannel::Alpha
    }

    /// K-B5: current compare-effects view flag (43 UI-path tests).
    pub fn compare_effects(&self) -> bool {
        self.compare_effects
    }

    /// K-B5: toggle effect-compare split; sends view-state to the engine.
    pub fn toggle_compare_effects(&mut self) {
        self.compare_effects = !self.compare_effects;
        self.session
            .send(photonic_video::EngineCmd::SetCompareEffects(
                self.compare_effects,
            ));
        self.presented = None;
    }

    /// (Re)create the intermediate target + egui registration when the
    /// physical frame size changes. `for_compare` selects the clean-side target.
    fn ensure_target(
        &mut self,
        device: &wgpu::Device,
        egui_renderer: &mut egui_wgpu::Renderer,
        size: (u32, u32),
        for_compare: bool,
    ) {
        let slot = if for_compare {
            &mut self.compare_target
        } else {
            &mut self.target
        };
        if slot.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        if let Some(old) = slot.take() {
            if let Some(id) = old.egui_id {
                egui_renderer.free_texture(&id);
            }
        }
        let label = if for_compare {
            "engine_monitor_compare_target"
        } else {
            "engine_monitor_target"
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // The present pass writes linear values and hardware encodes them;
            // egui then decodes the native texture before its gamma-space
            // window pass, avoiding a double sRGB transform.
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let egui_id =
            egui_renderer.register_native_texture(device, &view, wgpu::FilterMode::Linear);
        let mon = MonitorTexture {
            id: egui_id,
            physical: size,
        };
        if for_compare {
            self.compare_tex = Some(mon);
        } else {
            self.monitor_tex = Some(mon);
        }
        *slot = Some(PresentTarget {
            texture,
            view,
            size,
            egui_id: Some(egui_id),
        });
    }
}

/// UV rect cropping a pool-bucket-padded engine texture down to the logical
/// (sequence-format) content that sits at its top-left (facade note: physical
/// size is the pool's 64 px bucket, logical content is format-sized).
pub(crate) fn padded_uv(logical: (u32, u32), physical: (u32, u32)) -> egui::Rect {
    let (lw, lh) = (logical.0.max(1) as f32, logical.1.max(1) as f32);
    let (pw, ph) = (physical.0.max(1) as f32, physical.1.max(1) as f32);
    egui::Rect::from_min_max(
        egui::pos2(0.0, 0.0),
        egui::pos2((lw / pw).min(1.0), (lh / ph).min(1.0)),
    )
}

/// Buffering heuristic: playing, but the frame on screen lags the engine
/// playhead by more than `threshold_frames` — show the monitor spinner.
pub(crate) fn is_buffering(
    playing: bool,
    playhead: Tick,
    presented: Option<Tick>,
    ticks_per_frame: i64,
    threshold_frames: i64,
) -> bool {
    if !playing {
        return false;
    }
    let Some(shown) = presented else {
        return true; // playing with nothing on screen yet
    };
    (playhead.0 - shown.0).abs() > ticks_per_frame.max(1) * threshold_frames
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padded_uv_crops_bucket_padding() {
        // 320x180 logical in a 320x192 bucket: u full, v cropped.
        let uv = padded_uv((320, 180), (320, 192));
        assert_eq!(uv.min, egui::pos2(0.0, 0.0));
        assert!((uv.max.x - 1.0).abs() < f32::EPSILON);
        assert!((uv.max.y - 180.0 / 192.0).abs() < 1e-6);
    }

    #[test]
    fn padded_uv_exact_fit_is_full_texture() {
        let uv = padded_uv((1920, 1080), (1920, 1080));
        assert_eq!(uv.max, egui::pos2(1.0, 1.0));
    }

    #[test]
    fn padded_uv_never_exceeds_one_or_divides_by_zero() {
        let uv = padded_uv((100, 100), (64, 0));
        assert!(uv.max.x <= 1.0 && uv.max.y <= 1.0);
    }

    #[test]
    fn buffering_only_while_playing_and_lagging() {
        let tpf = 1000;
        // Not playing → never buffering.
        assert!(!is_buffering(false, Tick(9000), None, tpf, 4));
        // Playing with no frame yet → buffering.
        assert!(is_buffering(true, Tick(0), None, tpf, 4));
        // Small lag (≤ 4 frames) → fine.
        assert!(!is_buffering(true, Tick(4000), Some(Tick(0)), tpf, 4));
        // Large lag → buffering.
        assert!(is_buffering(true, Tick(9000), Some(Tick(0)), tpf, 4));
    }
}
