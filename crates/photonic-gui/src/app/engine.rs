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
//!    `Rgba8Unorm` texture registered with egui as a native texture; the
//!    monitor paints it with a UV rect cropped to the sequence-format logical
//!    size (the engine's frame textures are pool-bucket padded — see the
//!    facade notes in `photonic_video::session`).
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

use photonic_core::document::Document;
use photonic_core::history::CommandHistory;
use photonic_core::timeline::{SequenceId, Tick};
use photonic_render::video::VideoPresenter;
use photonic_video::{EngineCmd, EngineSession, EngineStatus, ProxyMode, VideoEngine};
use std::sync::{Arc, Mutex as StdMutex};

/// The registered egui texture for the current engine frame.
pub(crate) struct MonitorTexture {
    pub id: egui::TextureId,
    /// Physical (pool-bucket-padded) size of the presented texture.
    pub physical: (u32, u32),
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
    presented: Option<FrameKey>,
    pub(crate) monitor_tex: Option<MonitorTexture>,
    /// The frame the monitor texture currently shows (time + sequence), for
    /// the buffering heuristic.
    pub(crate) presented_frame: Option<(Tick, SequenceId)>,

    // ── Reconciler state (last values actually sent to the engine) ──────────
    sent_playing: Option<bool>,
    sent_loop: Option<Option<(Tick, Tick)>>,
    sent_sequence: Option<SequenceId>,
    sent_proxy: Option<ProxyMode>,
    /// Playhead value the GUI and engine last agreed on — a differing
    /// `self.playhead` means the *user* moved it (ruler scrub, Home/End,
    /// marker jump) and a `Seek` must be sent.
    pub(crate) agreed_playhead: Option<Tick>,
    /// GUI-side proxy-mode intent (media pool toggle).
    pub(crate) proxy_mode: ProxyMode,
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
        let gpu =
            photonic_video::GpuContext::new(renderer.device_arc(), renderer.queue_arc());
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
            presented: None,
            monitor_tex: None,
            presented_frame: None,
            sent_playing: None,
            sent_loop: None,
            sent_sequence: None,
            sent_proxy: None,
            agreed_playhead: None,
            proxy_mode: ProxyMode::Auto,
        }
    }

    /// Wait-free engine status for this frame.
    pub fn status(&self) -> Arc<EngineStatus> {
        self.session.status()
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

    /// Seek and record agreement so the scrub detector stays quiet.
    pub(crate) fn seek(&mut self, to: Tick) {
        self.session.send(EngineCmd::Seek(to));
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
        let key: FrameKey = (
            frame.time,
            frame.sequence,
            Arc::as_ptr(&frame.texture) as usize,
        );
        if self.presented == Some(key) {
            return;
        }
        let size = (frame.texture.width(), frame.texture.height());
        self.ensure_target(device, egui_renderer, size);
        let presenter = self
            .presenter
            .get_or_insert_with(|| VideoPresenter::new(device, wgpu::TextureFormat::Rgba8Unorm));
        let target = self.target.as_ref().expect("ensure_target sets target");

        let src_view = frame.texture.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("engine_frame_present"),
        });
        presenter.present_engine_frame(device, &mut encoder, &src_view, &target.view);
        queue.submit([encoder.finish()]);

        self.presented = Some(key);
        self.presented_frame = Some((frame.time, frame.sequence));
    }

    /// (Re)create the intermediate target + egui registration when the
    /// physical frame size changes.
    fn ensure_target(
        &mut self,
        device: &wgpu::Device,
        egui_renderer: &mut egui_wgpu::Renderer,
        size: (u32, u32),
    ) {
        if self.target.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        if let Some(old) = self.target.take() {
            if let Some(id) = old.egui_id {
                egui_renderer.free_texture(&id);
            }
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("engine_monitor_target"),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // The present pass writes sRGB-encoded values (03 §5); egui
            // samples them as-is and blends in gamma space against the
            // non-sRGB swapchain — same convention as the vector canvas.
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let egui_id =
            egui_renderer.register_native_texture(device, &view, wgpu::FilterMode::Linear);
        self.monitor_tex = Some(MonitorTexture {
            id: egui_id,
            physical: size,
        });
        self.target = Some(PresentTarget {
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
