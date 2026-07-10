//! The `VideoEngine` facade + `EngineSession` per-open-document runtime
//! (02 §1, normative).
//!
//! Threading model (02 §1):
//! - **Engine thread** (spawned per session) owns the playback state machine
//!   ([`crate::playback`]), graph compile/eval scheduling, and the media
//!   sources. It receives [`EngineCmd`] via a crossbeam channel and publishes
//!   [`EngineFrame`] + [`EngineStatus`] via `arc-swap` — the GUI never blocks
//!   on the engine.
//! - **Audio thread** — the cpal callback inside [`crate::audio::AudioEngine`]
//!   (opened lazily on the first `Play`); it owns the master clock (02 §4).
//! - **Mixer feeder** — a worker thread that renders mixer blocks from the
//!   snapshot's audio tracks into the lock-free ring the callback drains.
//! - **Decode** — P3 floor: the engine thread drives `DecodeSource` seeks
//!   inline (bounded by the sidecar's restart/backoff containment) and pumps
//!   rings via [`crate::playback::prefetch`]; the N-worker decode pool is the
//!   documented follow-up seam.
//!
//! Document access NEVER blocks mid-playback: the engine polls
//! `CommandHistory::revision`/`changes_since` with `try_lock` and re-snapshots
//! the `TimelineProject` (cheap `Clone`, 01) only when the revision moved;
//! contended locks just reuse the last snapshot (02 §1).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use arc_swap::{ArcSwap, ArcSwapOption};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};

use photonic_core::timeline::{
    AssetSource, Clip, ClipAudio, ClipId, ClipSource, FrameRate, SequenceId, Tick, TimelineProject,
    TICKS_PER_SECOND,
};
use photonic_core::{CommandHistory, Document};
use photonic_render::color::{Colorimetry, Matrix, Range};
use photonic_render::video::convert_yuv_planes_to_working;

use crate::audio::mixer::PcmSource;
use crate::audio::{
    audio_ring, AudioEngine, ClipVoice, Mixer, RingProducer, TrackVoice, XrunCounters,
    BLOCK_FRAMES, CHANNELS,
};
use crate::contract::{AssetId, VectorRef, VectorStateKey};
use crate::decode::scheduler::{DecodeSource, PtsKind, SourceParams};
use crate::decode::{PixFmt, SharedRing};
use crate::export::presets::ExportPreset;
use crate::graph::cache::CacheStats;
use crate::graph::compile::{compile, Quality};
use crate::graph::eval::{Evaluator, GpuContext, GpuFrameSource};
use crate::graph::ir::{FrameGraph, IrOp};
use crate::media::ffmpeg_locate::{locate, FfmpegTools};
use crate::media::keyframe_index::{KeyframeIndex, PtsIndex};
use crate::media::probe::{probe_details, ProbeDetails};
use crate::playback::{prefetch, FfmpegPcmSource, PlaybackController, PresentDecision};

// ── Public command / state types (02 §1) ────────────────────────────────────

/// Proxy media policy for this session (02 §6 — session state, not document).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum ProxyMode {
    /// Engine decides per asset. P3 has no proxy generation yet, so `Auto`
    /// decodes originals (proxies are never required for correctness, CAP-014).
    #[default]
    Auto,
    ForceProxy,
    ForceOriginal,
}

/// An export request (02 §7). Carried by [`EngineCmd::Export`], which is a
/// declared **NotImplemented stub in P3** — headless export goes through
/// [`crate::export::render_loop::export_frames`] directly until the engine
/// wires the job queue.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportJob {
    pub sequence: SequenceId,
    pub format_index: usize,
    pub preset: ExportPreset,
    pub output: PathBuf,
    /// `None` = the sequence's work range / full extent.
    pub range: Option<(Tick, Tick)>,
}

/// GUI/MCP → engine commands (02 §1).
#[derive(Clone, Debug)]
pub enum EngineCmd {
    Play,
    Pause,
    /// Coalesced latest-wins per engine tick (02 §4 scrub rule).
    Seek(Tick),
    /// Pause + snap ±n frames, evaluate exactly that tick (CAP-004).
    Step(i32),
    SetLoop(Option<(Tick, Tick)>),
    SetActiveSequence(SequenceId),
    SetProxyMode(ProxyMode),
    /// Asset relink / proxy swap (02 §5). P3 over-invalidates (whole node
    /// cache + decode sources) — the hash→asset index for targeted eviction
    /// is the proxy/relink story's seam.
    InvalidateRange(SequenceId, Tick, Tick),
    /// **NotImplemented stub in P3** — surfaces on [`EngineStatus::last_error`].
    Export(Box<ExportJob>),
    /// **NotImplemented stub in P3** — surfaces on [`EngineStatus::last_error`].
    Probe(AssetId),
    /// Stop the engine thread. [`EngineSession`] sends this on drop/shutdown.
    Shutdown,
}

/// What the GUI presents (02 §1): `Rgba16Float`, linear, premultiplied (D-09).
/// The present path is 03 §5's `present_engine_frame`.
pub struct EngineFrame {
    pub texture: Arc<wgpu::Texture>,
    /// Exact frame-start tick this frame was evaluated at.
    pub time: Tick,
    pub sequence: SequenceId,
}

/// Engine → GUI state (02 §1: playhead, dropped frames, cache stats, xruns).
#[derive(Clone, Debug)]
pub struct EngineStatus {
    pub playhead: Tick,
    pub playing: bool,
    /// Frames dropped by the cover-interval rule (late > 1 frame, 02 §4).
    pub dropped: u64,
    pub cache: CacheStats,
    /// Audio callback underrun frames (09 §5).
    pub audio_xruns: u64,
    /// The `CommandHistory` revision the current snapshot was taken at
    /// (02 §1's `doc_generation`).
    pub doc_revision: u64,
    pub active_sequence: Option<SequenceId>,
    /// Most recent command failure (e.g. the P3 `Export`/`Probe` stubs).
    pub last_error: Option<String>,
}

impl Default for EngineStatus {
    fn default() -> Self {
        EngineStatus {
            playhead: Tick::ZERO,
            playing: false,
            dropped: 0,
            cache: CacheStats::default(),
            audio_xruns: 0,
            doc_revision: 0,
            active_sequence: None,
            last_error: None,
        }
    }
}

/// Latest-wins seek coalescing (02 §4): reduce one drained command batch so
/// only the **last** `Seek` survives, at its original position relative to the
/// non-seek commands after it. Everything else keeps its order.
pub fn coalesce_commands(batch: Vec<EngineCmd>) -> Vec<EngineCmd> {
    let last_seek = batch.iter().rposition(|c| matches!(c, EngineCmd::Seek(_)));
    batch
        .into_iter()
        .enumerate()
        .filter(|(i, c)| !matches!(c, EngineCmd::Seek(_)) || Some(*i) == last_seek)
        .map(|(_, c)| c)
        .collect()
}

/// Matrix/range selection from a probe (02 §3 "BT.601/709 per probe"): trust
/// the container's tags when present, else the SD/HD resolution heuristic.
pub fn colorimetry_for_probe(details: &ProbeDetails) -> Colorimetry {
    let v = details.probe.video.as_ref();
    let color = v.map(|v| &v.color);
    let matrix = match color.and_then(|c| c.matrix.as_deref()) {
        Some("bt709") => Matrix::Bt709,
        Some("smpte170m") | Some("bt470bg") | Some("smpte240m") => Matrix::Bt601,
        _ => match v {
            Some(v) if v.height >= 720 => Matrix::Bt709,
            Some(_) => Matrix::Bt601,
            None => Matrix::Bt709,
        },
    };
    let range = match color.and_then(|c| c.full_range) {
        Some(true) => Range::Full,
        _ => Range::Limited,
    };
    Colorimetry { matrix, range }
}

// ── Facade ───────────────────────────────────────────────────────────────────

/// The engine facade (02 §1). Owns the shared GPU handle; each open document
/// gets its own [`EngineSession`] (engine thread + audio host + caches).
pub struct VideoEngine {
    gpu: GpuContext,
}

impl VideoEngine {
    /// Share the renderer's wgpu device/queue (02 §1). [`GpuContext`] is the
    /// crate's shared handle type — `GpuContext::new(device, queue)` wraps
    /// whatever the host renderer already owns.
    pub fn new(gpu: GpuContext) -> Self {
        VideoEngine { gpu }
    }

    /// Request an own headless adapter (CLI/MCP/tests). `None` when no GPU
    /// adapter is available (the adapter-skip convention).
    pub fn headless() -> Option<Self> {
        GpuContext::request_blocking().map(VideoEngine::new)
    }

    pub fn gpu(&self) -> &GpuContext {
        &self.gpu
    }

    /// Open a per-document session: spawns the engine thread, which snapshots
    /// `doc.timeline` whenever `history`'s revision moves (never blocking on
    /// either lock).
    pub fn open_session(
        &self,
        doc: Arc<Mutex<Document>>,
        history: Arc<Mutex<CommandHistory>>,
    ) -> EngineSession {
        let (tx, rx) = crossbeam_channel::unbounded();
        let frame: Arc<ArcSwapOption<EngineFrame>> = Arc::new(ArcSwapOption::from(None));
        let status: Arc<ArcSwap<EngineStatus>> =
            Arc::new(ArcSwap::from_pointee(EngineStatus::default()));
        let gpu = self.gpu.clone();
        let frame_out = Arc::clone(&frame);
        let status_out = Arc::clone(&status);
        let join = std::thread::Builder::new()
            .name("photonic-video-engine".into())
            .spawn(move || {
                EngineThread::new(gpu, doc, history, rx, frame_out, status_out).run();
            })
            .expect("spawn photonic-video engine thread");
        EngineSession {
            tx,
            frame,
            status,
            join: Some(join),
        }
    }
}

/// Per-open-document runtime handle (02 §1). Cheap wait-free reads on the GUI
/// side; commands are fire-and-forget.
pub struct EngineSession {
    tx: Sender<EngineCmd>,
    frame: Arc<ArcSwapOption<EngineFrame>>,
    status: Arc<ArcSwap<EngineStatus>>,
    join: Option<JoinHandle<()>>,
}

impl EngineSession {
    /// Send a command to the engine thread. Returns `false` if the engine has
    /// already shut down.
    pub fn send(&self, cmd: EngineCmd) -> bool {
        self.tx.send(cmd).is_ok()
    }

    /// The most recently published frame (wait-free; `None` before the first
    /// evaluation).
    pub fn latest_frame(&self) -> Option<Arc<EngineFrame>> {
        self.frame.load_full()
    }

    /// The most recently published status (wait-free).
    pub fn status(&self) -> Arc<EngineStatus> {
        self.status.load_full()
    }

    /// Stop the engine thread and join it. (Dropping the session does the
    /// same; this form surfaces the join point explicitly.)
    pub fn shutdown(mut self) {
        self.shutdown_inner();
    }

    fn shutdown_inner(&mut self) {
        let _ = self.tx.send(EngineCmd::Shutdown);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for EngineSession {
    fn drop(&mut self) {
        self.shutdown_inner();
    }
}

// ── Engine thread ────────────────────────────────────────────────────────────

struct EngineThread {
    doc: Arc<Mutex<Document>>,
    history: Arc<Mutex<CommandHistory>>,
    rx: Receiver<EngineCmd>,
    frame_out: Arc<ArcSwapOption<EngineFrame>>,
    status_out: Arc<ArcSwap<EngineStatus>>,

    evaluator: Evaluator,
    media: MediaSources,
    controller: PlaybackController,

    snapshot: Option<Arc<TimelineProject>>,
    last_revision: Option<u64>,
    active_sequence_override: Option<SequenceId>,
    proxy_mode: ProxyMode,
    last_error: Option<String>,

    audio: AudioEngine,
    feeder: Option<AudioFeeder>,
    xruns: Option<Arc<XrunCounters>>,
    tools: Option<FfmpegTools>,
}

impl EngineThread {
    fn new(
        gpu: GpuContext,
        doc: Arc<Mutex<Document>>,
        history: Arc<Mutex<CommandHistory>>,
        rx: Receiver<EngineCmd>,
        frame_out: Arc<ArcSwapOption<EngineFrame>>,
        status_out: Arc<ArcSwap<EngineStatus>>,
    ) -> Self {
        let tools = locate().ok();
        EngineThread {
            doc,
            history,
            rx,
            frame_out,
            status_out,
            evaluator: Evaluator::new(gpu),
            media: MediaSources::new(tools.clone()),
            controller: PlaybackController::new(FrameRate::FPS_30),
            snapshot: None,
            last_revision: None,
            active_sequence_override: None,
            proxy_mode: ProxyMode::Auto,
            last_error: None,
            audio: AudioEngine::new(),
            feeder: None,
            xruns: None,
            tools,
        }
    }

    fn run(mut self) {
        loop {
            // 1. Wait briefly for commands, then drain the burst (a scrub
            //    produces many Seeks per engine tick — coalesced latest-wins).
            let mut batch = Vec::new();
            match self.rx.recv_timeout(Duration::from_millis(2)) {
                Ok(cmd) => {
                    batch.push(cmd);
                    while let Ok(cmd) = self.rx.try_recv() {
                        batch.push(cmd);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            let mut shutdown = false;
            for cmd in coalesce_commands(batch) {
                if !self.handle(cmd) {
                    shutdown = true;
                }
            }
            if shutdown {
                break;
            }

            // 2. doc_generation poll (02 §1): revision + changes_since via
            //    try_lock; contended ⇒ reuse the last snapshot.
            self.poll_snapshot();

            // 3. Present per the cover-interval rule.
            self.present();

            // 4. Publish status (wait-free for the reader).
            self.publish_status();
        }
        self.stop_playing();
    }

    /// Returns `false` on `Shutdown`.
    fn handle(&mut self, cmd: EngineCmd) -> bool {
        match cmd {
            EngineCmd::Play => self.start_playing(),
            EngineCmd::Pause => self.stop_playing(),
            EngineCmd::Seek(t) => {
                if self.controller.is_playing() && self.audio.is_started() {
                    // The rtrb ring is a fixed SPSC pair, so an audio-mastered
                    // seek restarts the device stream + feeder at the target
                    // (brief glitch — acceptable scrub behavior; gapless
                    // seek-while-playing is the decode-pool story's seam).
                    self.stop_playing();
                    self.controller.seek(t);
                    self.start_playing();
                } else {
                    self.controller.seek(t);
                }
            }
            EngineCmd::Step(n) => {
                self.stop_playing(); // Step always pauses (02 §4)
                self.controller.step(n);
            }
            EngineCmd::SetLoop(range) => self.controller.set_loop(range),
            EngineCmd::SetActiveSequence(id) => {
                self.active_sequence_override = Some(id);
                self.controller.request_present();
            }
            EngineCmd::SetProxyMode(mode) => {
                self.proxy_mode = mode;
                // Different quality flag ⇒ different content hashes; the old
                // entries age out hash-naturally (02 §5).
                self.controller.request_present();
            }
            EngineCmd::InvalidateRange(_seq, _from, _to) => {
                // P3 over-invalidation: without the hash→asset index (proxy/
                // relink story) we evict everything, which is always correct.
                self.evaluator.invalidate_matching(|_| true);
                self.media.invalidate_all();
                self.controller.request_present();
            }
            EngineCmd::Export(job) => {
                self.fail(format!(
                    "EngineCmd::Export not implemented in P3 (sequence {}); use \
                     export::render_loop::export_frames directly",
                    job.sequence
                ));
            }
            EngineCmd::Probe(asset) => {
                self.fail(format!(
                    "EngineCmd::Probe not implemented in P3 (asset {asset}); use \
                     media::probe::probe_details directly"
                ));
            }
            EngineCmd::Shutdown => return false,
        }
        true
    }

    fn fail(&mut self, msg: String) {
        tracing::warn!(target: "photonic_video::session", "{msg}");
        self.last_error = Some(msg);
    }

    /// Re-snapshot the timeline when the history revision moved. `try_lock`
    /// only — contention just means "reuse the last snapshot this tick".
    fn poll_snapshot(&mut self) {
        let summary = match self.history.try_lock() {
            Ok(h) => h.changes_since(self.last_revision.unwrap_or(0)),
            Err(_) => return,
        };
        if self.last_revision == Some(summary.revision) {
            return;
        }
        let Ok(doc) = self.doc.try_lock() else {
            return; // contended: keep playing off the previous snapshot
        };
        let snap = doc.timeline.as_ref().map(|p| Arc::new(p.clone()));
        drop(doc);
        // `summary.touched`/`overflowed` (vector NodeIds) is the targeted
        // vector-raster invalidation hook: when the session-level RasterVector
        // cache lands (02 §3 vector frames), touched nodes evict matching
        // `VectorStateKey` entries here. Clip/timeline edits already
        // invalidate hash-naturally via recompiled content hashes (02 §5).
        self.last_revision = Some(summary.revision);
        self.snapshot = snap.clone();
        if let Some(p) = snap {
            self.media.set_project(p);
        }
        self.controller.request_present();
    }

    fn effective_sequence(&self, project: &TimelineProject) -> Option<SequenceId> {
        self.active_sequence_override
            .filter(|id| project.sequences.contains_key(id))
            .or(project.active_sequence)
            .or_else(|| project.sequence_order.first().copied())
            .or_else(|| project.sequences.keys().next().copied())
    }

    fn present(&mut self) {
        let Some(project) = self.snapshot.clone() else {
            return;
        };
        let Some(seq_id) = self.effective_sequence(&project) else {
            return;
        };
        let Some(seq) = project.sequences.get(&seq_id) else {
            return;
        };
        self.controller.set_rate(seq.frame_rate);

        match self.controller.tick() {
            PresentDecision::Hold => {}
            PresentDecision::LoopWrap(start) => {
                let was_playing = self.controller.is_playing();
                self.stop_playing();
                self.controller.seek(start);
                if was_playing {
                    self.start_playing();
                }
            }
            PresentDecision::Present(t) => {
                let quality = match self.proxy_mode {
                    ProxyMode::ForceProxy => Quality::PREVIEW,
                    // Auto = originals until proxy generation lands (02 §6).
                    ProxyMode::Auto | ProxyMode::ForceOriginal => Quality::FULL,
                };
                let format_index = seq.active_format.min(seq.formats.len().saturating_sub(1));
                let compiled = compile(&project, seq_id, format_index, t, quality, None);
                let canvas = seq
                    .formats
                    .get(format_index)
                    .map(|f| (f.width, f.height))
                    .unwrap_or((1, 1));
                if let Some(texture) =
                    self.evaluator
                        .evaluate(&compiled.graph, canvas, &mut self.media)
                {
                    self.frame_out.store(Some(Arc::new(EngineFrame {
                        texture,
                        time: t,
                        sequence: seq_id,
                    })));
                }
                if self.controller.is_playing() {
                    // Prefetch v1: keep the on-screen sources' rings pumped
                    // (cut-ahead warmup = seam, playback/prefetch.rs).
                    self.media.prefetch(&compiled.graph);
                }
            }
        }
    }

    fn start_playing(&mut self) {
        if self.controller.is_playing() {
            return;
        }
        let start = self.controller.playhead();
        let (producer, consumer, xruns) = audio_ring();
        match self.audio.start(consumer) {
            Ok(sample_rate) => {
                self.xruns = Some(xruns);
                // Feed the ring from the snapshot's audio tracks; without a
                // snapshot/sequence the ring stays empty and the callback
                // emits silence (the master clock still advances).
                if let Some(project) = self.snapshot.clone() {
                    if let Some(seq) = self.effective_sequence(&project) {
                        self.feeder = Some(spawn_audio_feeder(
                            project,
                            seq,
                            start,
                            sample_rate,
                            producer,
                            self.tools.clone(),
                        ));
                    }
                }
                self.controller.play_audio(self.audio.clock());
            }
            Err(err) => {
                // No device (headless/CI) ⇒ soft clock (02 §4 paused/scrub
                // clock doubles as the device-less playback clock).
                tracing::debug!(
                    target: "photonic_video::session",
                    "audio device unavailable ({err}); playing on soft clock"
                );
                self.controller.play_soft();
            }
        }
    }

    fn stop_playing(&mut self) {
        self.controller.pause();
        self.feeder = None; // Drop stops + joins the feeder thread
        self.audio.stop();
    }

    fn publish_status(&self) {
        self.status_out.store(Arc::new(EngineStatus {
            playhead: self.controller.playhead(),
            playing: self.controller.is_playing(),
            dropped: self.controller.dropped(),
            cache: self.evaluator.cache_stats(),
            audio_xruns: self
                .xruns
                .as_ref()
                .map(|x| x.underrun_frames())
                .unwrap_or(0),
            doc_revision: self.last_revision.unwrap_or(0),
            active_sequence: self
                .snapshot
                .as_ref()
                .and_then(|p| self.effective_sequence(p)),
            last_error: self.last_error.clone(),
        }));
    }
}

// ── Media sources (GpuFrameSource over decode rings) ─────────────────────────

struct VideoSourceEntry {
    decode: DecodeSource,
    colorimetry: Colorimetry,
    rate: FrameRate,
}

/// Resolves `DecodeVideo` ops to working textures over per-asset ffmpeg
/// sidecar decode rings (02 §3). Stills (`DecodeStill` via
/// `RasterImage::from_encoded`, cached by asset) and vector frames
/// (`RasterVector` via `HeadlessRenderer`, cached by `VectorStateKey`) are the
/// documented follow-up seams — until then those ops evaluate transparent.
struct MediaSources {
    tools: Option<FfmpegTools>,
    project: Option<Arc<TimelineProject>>,
    /// `None` value = open failed; don't re-probe every frame (an
    /// `InvalidateRange` clears the entry and allows a retry).
    sources: HashMap<(AssetId, bool), Option<VideoSourceEntry>>,
    /// Uploaded working textures keyed by decoded pts — scrub back/forward
    /// over the same frames skips the GPU upload.
    uploads: HashMap<(AssetId, Tick, bool), Arc<wgpu::Texture>>,
}

/// Upload-cache entry cap: ~a ring's worth per couple of assets; wholesale
/// clear on overflow keeps it trivially bounded (entries are cheap to rebuild
/// from the decode ring).
const UPLOAD_CACHE_CAP: usize = 32;

impl MediaSources {
    fn new(tools: Option<FfmpegTools>) -> Self {
        MediaSources {
            tools,
            project: None,
            sources: HashMap::new(),
            uploads: HashMap::new(),
        }
    }

    fn set_project(&mut self, project: Arc<TimelineProject>) {
        self.project = Some(project);
    }

    fn invalidate_all(&mut self) {
        self.sources.clear();
        self.uploads.clear();
    }

    fn ensure_source(&mut self, asset: AssetId, proxy: bool) {
        if self.sources.contains_key(&(asset, proxy)) {
            return;
        }
        let entry = self.build_source(asset, proxy);
        self.sources.insert((asset, proxy), entry);
    }

    fn build_source(&self, asset: AssetId, proxy: bool) -> Option<VideoSourceEntry> {
        let tools = self.tools.clone()?;
        let project = self.project.as_ref()?;
        let media_asset = project.media.assets.get(&asset)?;
        let original = match &media_asset.source {
            AssetSource::File { path, .. } => path.clone(),
            // Embedded vectors go through RasterVector, never DecodeVideo.
            _ => return None,
        };
        // Proxy selection (02 §6): decode the generated proxy input when it was
        // requested (Quality::PREVIEW / ProxyMode::ForceProxy) and a Ready proxy
        // is present; otherwise the original. Probe/keyframe/pts below then run
        // against the *selected* file, so the whole source (dims, GOP structure,
        // pts model) matches whichever media it decodes. Proxies are never
        // required for correctness — a missing/pending proxy falls back to the
        // original (CAP-014).
        let path =
            crate::media::proxy::resolve_decode_input(&original, media_asset.proxy.as_ref(), proxy);
        let details = probe_details(&tools, &path).ok()?;
        let video = details.probe.video.clone()?;
        let keyframes = KeyframeIndex::build(&tools, &path).ok()?;
        let pts_kind = if details.is_vfr {
            PtsKind::Vfr(Arc::new(PtsIndex::build(&tools, &path).ok()?))
        } else {
            PtsKind::Cfr(video.frame_rate)
        };
        let params = SourceParams {
            input: path,
            width: video.width,
            height: video.height,
            pix_fmt: PixFmt::for_alpha(details.has_alpha),
            pts_kind,
            keyframes,
        };
        Some(VideoSourceEntry {
            decode: DecodeSource::new(tools, params, SharedRing::preview()),
            colorimetry: colorimetry_for_probe(&details),
            rate: video.frame_rate,
        })
    }

    /// Pump the rings of every source the compiled frame references
    /// (prefetch v1 — 02 §3; cut-ahead is the documented seam).
    fn prefetch(&mut self, graph: &FrameGraph) {
        for node in &graph.nodes {
            if let IrOp::DecodeVideo {
                asset,
                src_time,
                proxy,
            } = node.op
            {
                if let Some(Some(entry)) = self.sources.get_mut(&(asset, proxy)) {
                    let _ = prefetch::pump_ahead(&mut entry.decode, src_time, entry.rate);
                }
            }
        }
    }
}

impl GpuFrameSource for MediaSources {
    fn video_texture(
        &mut self,
        gpu: &GpuContext,
        asset: AssetId,
        src_time: Tick,
        proxy: bool,
    ) -> Option<Arc<wgpu::Texture>> {
        self.ensure_source(asset, proxy);
        let entry = self.sources.get_mut(&(asset, proxy))?.as_mut()?;

        // Ring hit: the frame whose nominal [pts, pts+frame) interval covers
        // src_time. Anything further back is stale (a discontinuous seek must
        // not serve an old frame; Step must never re-serve the previous one).
        let tolerance = entry.rate.ticks_per_frame().0;
        let ring_hit = entry
            .decode
            .ring()
            .frame_covering(src_time)
            .filter(|f| src_time.0 - f.pts.0 < tolerance);
        let frame = match ring_hit {
            Some(f) => f,
            // Miss: blocking keyframe seek on this thread (P3 floor — the
            // decode worker pool moves this off the engine thread; failure is
            // contained by the sidecar's restart budget + backoff).
            None => entry.decode.seek(src_time).ok()?,
        };
        let colorimetry = entry.colorimetry;

        let key = (asset, frame.pts, proxy);
        if let Some(cached) = self.uploads.get(&key) {
            return Some(Arc::clone(cached));
        }
        let converted = convert_yuv_planes_to_working(
            gpu.device(),
            gpu.queue(),
            &frame.planes.as_yuv_planes(),
            colorimetry,
        );
        let texture = Arc::new(pad_to_pool_bucket(gpu, converted));
        if self.uploads.len() >= UPLOAD_CACHE_CAP {
            self.uploads.clear();
        }
        self.uploads.insert(key, Arc::clone(&texture));
        Some(texture)
    }

    fn still_texture(&mut self, _gpu: &GpuContext, _asset: AssetId) -> Option<Arc<wgpu::Texture>> {
        // Seam: DecodeStill via `RasterImage::from_encoded`, uploaded once and
        // cached by asset (02 §3) — the stills story. Transparent until then.
        None
    }

    fn vector_texture(
        &mut self,
        _gpu: &GpuContext,
        _vref: VectorRef,
        _key: VectorStateKey,
        _w: u32,
        _h: u32,
    ) -> Option<Arc<wgpu::Texture>> {
        // Seam: RasterVector via `HeadlessRenderer::render_rgba_with_opts`,
        // cached by `VectorStateKey` (02 §3) — the vector-frames story.
        None
    }
}

/// Pad a source upload to the texture pool's 64px size bucket (03 §3.4).
///
/// The evaluator's pooled node-result textures are allocated at **bucket**
/// size (e.g. a 320×180 sequence renders into 320×192 textures) and its blit/
/// merge passes sample normalized `uv` over the *whole* physical texture —
/// exact texel-center identity only holds when producer and consumer share
/// the same physical dimensions. Source uploads come out of
/// `convert_yuv_planes_to_working` at exact media size, so without this pad a
/// same-size blit would resample (vertically stretch 180 → 192). Copying the
/// content into the top-left of a bucket-sized, zero-initialized texture makes
/// every pass in the chain a pixel-exact identity map; the logical region is
/// what `Output`/readback consume. (The evaluator-side alternative — logical-
/// size-aware sampling — belongs to graph/eval and is out of this story's
/// territory.)
fn pad_to_pool_bucket(gpu: &GpuContext, src: wgpu::Texture) -> wgpu::Texture {
    let (w, h) = (src.width(), src.height());
    let bucket = crate::graph::ir::TextureDesc {
        width: w,
        height: h,
    }
    .bucket();
    if bucket == (w, h) {
        return src;
    }
    let padded = gpu.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("video_upload_bucket_padded"),
        size: wgpu::Extent3d {
            width: bucket.0,
            height: bucket.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut enc = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("video_upload_pad"),
        });
    enc.copy_texture_to_texture(
        src.as_image_copy(),
        padded.as_image_copy(),
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue().submit([enc.finish()]);
    padded
}

// ── Audio feeder (mixer worker, 02 §1 / 09 §5) ───────────────────────────────

/// Handle to the mixer worker thread; dropping stops + joins it.
struct AudioFeeder {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl Drop for AudioFeeder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Spawn the mixer worker: renders [`BLOCK_FRAMES`]-frame blocks from the
/// snapshot's audio tracks (voices resolved per block per 09 §4's seam) into
/// the lock-free ring the cpal callback drains.
fn spawn_audio_feeder(
    project: Arc<TimelineProject>,
    sequence: SequenceId,
    start: Tick,
    sample_rate: u32,
    producer: RingProducer,
    tools: Option<FfmpegTools>,
) -> AudioFeeder {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);
    let join = std::thread::Builder::new()
        .name("photonic-video-mixer".into())
        .spawn(move || {
            feeder_main(
                project,
                sequence,
                start,
                sample_rate,
                producer,
                tools,
                stop_flag,
            )
        })
        .expect("spawn photonic-video mixer thread");
    AudioFeeder {
        stop,
        join: Some(join),
    }
}

fn feeder_main(
    project: Arc<TimelineProject>,
    sequence: SequenceId,
    start: Tick,
    sample_rate: u32,
    mut producer: RingProducer,
    tools: Option<FfmpegTools>,
    stop: Arc<AtomicBool>,
) {
    let sample_rate = sample_rate.max(1);
    let block_ticks =
        Tick(((BLOCK_FRAMES as i128 * TICKS_PER_SECOND as i128) / sample_rate as i128) as i64);
    let mut out = vec![0f32; BLOCK_FRAMES * CHANNELS];
    let mut t = start;

    let Some(seq) = project.sequences.get(&sequence) else {
        // No sequence: keep the ring fed with silence so the callback (and
        // master clock) run smoothly.
        while !stop.load(Ordering::Relaxed) {
            if producer.is_full() {
                std::thread::sleep(Duration::from_millis(2));
                continue;
            }
            producer.push_block(&out);
        }
        return;
    };

    let mut mixer = Mixer::new(sample_rate);
    let default_clip_audio = ClipAudio::new();
    // Persistent per-clip PCM sidecars: opened when a clip becomes audible
    // (seeked to its mapped source position), read sequentially block after
    // block, dropped when the clip stops sounding.
    let mut pcm: HashMap<ClipId, FfmpegPcmSource> = HashMap::new();

    while !stop.load(Ordering::Relaxed) {
        if producer.is_full() {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }

        // Which clips sound at t? Audio tracks only in P3 — video-clip
        // embedded audio arrives with the linked-clips story.
        let active: Vec<(&photonic_core::timeline::Track, &Clip)> = seq
            .audio_tracks
            .iter()
            .filter(|track| track.enabled && track.audio.is_some())
            .flat_map(|track| {
                track
                    .clips
                    .iter()
                    .filter(|clip| {
                        clip.enabled
                            && clip.start <= t
                            && t < clip.end()
                            && matches!(clip.source, ClipSource::Asset { .. })
                    })
                    .map(move |clip| (track, clip))
            })
            .collect();

        // Open sidecars for newly-audible clips; drop finished ones.
        if let Some(tools) = tools.as_ref() {
            for (_, clip) in &active {
                if pcm.contains_key(&clip.id) {
                    continue;
                }
                let ClipSource::Asset { asset } = clip.source else {
                    continue;
                };
                let Some(AssetSource::File { path, .. }) =
                    project.media.assets.get(&asset).map(|a| &a.source)
                else {
                    continue;
                };
                // Trim mapping: source_in + elapsed. Speed maps are a seam
                // (audio resampling for non-1:1 speed is the P8 DSP story).
                let src_pos = clip.source_in + (t - clip.start);
                if let Ok(source) = FfmpegPcmSource::spawn(tools, path, src_pos, sample_rate) {
                    pcm.insert(clip.id, source);
                }
            }
        }
        let active_ids: HashSet<ClipId> = active.iter().map(|(_, c)| c.id).collect();
        pcm.retain(|id, _| active_ids.contains(id));

        // Build this block's voice list (09 §4: the playback side resolves
        // "what's audible" once per block; the mixer owns only signal flow).
        let mut refs: HashMap<ClipId, &mut FfmpegPcmSource> =
            pcm.iter_mut().map(|(id, src)| (*id, src)).collect();
        let mut voices: Vec<TrackVoice<'_>> = Vec::new();
        for track in seq
            .audio_tracks
            .iter()
            .filter(|track| track.enabled && track.audio.is_some())
        {
            let track_audio = track.audio.as_ref().expect("filtered to Some");
            let mut clips: Vec<ClipVoice<'_>> = Vec::new();
            for (_, clip) in active.iter().filter(|(tr, _)| tr.id == track.id) {
                if let Some(source) = refs.remove(&clip.id) {
                    clips.push(ClipVoice {
                        audio: clip.audio.as_ref().unwrap_or(&default_clip_audio),
                        elapsed: t - clip.start,
                        remaining: clip.end() - t,
                        source: source as &mut dyn PcmSource,
                    });
                }
            }
            voices.push(TrackVoice {
                id: track.id,
                audio: track_audio,
                clips,
            });
        }

        out.fill(0.0);
        mixer.render_block(t, &mut voices, &seq.audio_master, &mut out);
        producer.push_block(&out);
        t = t + block_ticks;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seek_coalescing_keeps_only_the_last_seek_in_place() {
        let batch = vec![
            EngineCmd::Seek(Tick(1)),
            EngineCmd::Pause,
            EngineCmd::Seek(Tick(2)),
            EngineCmd::Play,
            EngineCmd::Seek(Tick(3)),
            EngineCmd::SetProxyMode(ProxyMode::ForceProxy),
        ];
        let out = coalesce_commands(batch);
        assert_eq!(out.len(), 4);
        assert!(matches!(out[0], EngineCmd::Pause));
        assert!(matches!(out[1], EngineCmd::Play));
        assert!(
            matches!(out[2], EngineCmd::Seek(Tick(3))),
            "only the LAST seek survives, at its original relative position"
        );
        assert!(matches!(
            out[3],
            EngineCmd::SetProxyMode(ProxyMode::ForceProxy)
        ));
    }

    #[test]
    fn seek_coalescing_passes_through_seekless_batches() {
        let out = coalesce_commands(vec![EngineCmd::Play, EngineCmd::Pause]);
        assert_eq!(out.len(), 2);
        let out = coalesce_commands(vec![EngineCmd::Seek(Tick(9))]);
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], EngineCmd::Seek(Tick(9))));
    }
}
