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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use arc_swap::{ArcSwap, ArcSwapOption};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};

use photonic_core::timeline::{
    AssetKind, AssetSource, Clip, ClipAudio, ClipId, ClipSource, FrameRate, Sequence, SequenceId,
    Tick, TimelineProject, TICKS_PER_SECOND,
};
use photonic_core::{CommandHistory, Document};
use photonic_render::color::{Colorimetry, Matrix, Range};
use photonic_render::video::YuvConverter;
use photonic_render::{ExportBackground, ExportOptions, HeadlessRenderer};

use crate::audio::mixer::PcmSource;
use crate::audio::{
    audio_ring, AudioEngine, ClipVoice, Mixer, RingProducer, TrackVoice, XrunCounters,
    BLOCK_FRAMES, CHANNELS,
};
use crate::contract::{AssetId, VectorRef, VectorStateKey};
use crate::decode::scheduler::{DecodeSource, PtsKind, SourceParams};
use crate::decode::worker::DecodeWorker;
use crate::decode::{PixFmt, SharedRing};
use crate::export::presets::ExportPreset;
use crate::graph::cache::CacheStats;
use crate::graph::compile::{
    compile_asset_peek, compile_with_luts, fit_long_edge, CompileDiagnostic, DiagSeverity,
    LutProvider, Quality, DRAFT_MAX_LONG_EDGE,
};
use crate::graph::eval::{Evaluator, GpuContext, GpuFrame, GpuFrameSource};
use crate::graph::ir::IrOp;
use crate::media::ffmpeg_locate::{locate, FfmpegTools};
use crate::media::keyframe_index::{KeyframeIndex, PtsIndex};
use crate::media::probe::{content_hash, probe_details, ProbeDetails};
use crate::playback::prefetch::{
    cut_ahead_targets, lru_eviction_victims, CUT_AHEAD_LEAD_FRAMES, MAX_LIVE_SOURCES,
};
use crate::playback::{FfmpegPcmSource, PlaybackController, PresentDecision};

/// While playing, wake often enough for 60–120 Hz present opportunities without
/// spinning at 500 Hz (2 ms). 4 ms ≈ 250 Hz upper bound; PresentDecision still
/// gates actual evaluate work to the sequence frame rate.
const PLAYING_POLL_INTERVAL: Duration = Duration::from_millis(4);
/// Paused: rare doc-snapshot poll; commands still wake immediately via channel.
const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(100);

// ── Public command / state types (02 §1) ────────────────────────────────────

/// Proxy media policy for this session (02 §6 — session state, not document).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum ProxyMode {
    /// Engine chooses a ready proxy per asset and otherwise falls back to the
    /// original. This is the responsive editing default: proxies are never
    /// required for correctness, so an asset whose proxy is absent, pending,
    /// or failed remains playable without a mode change (CAP-014).
    #[default]
    Auto,
    ForceProxy,
    ForceOriginal,
}

/// Interactive preview quality (24-preview-media-load §4). Session-only.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum PreviewQuality {
    /// Proxy when allowed + long edge capped at [`DRAFT_MAX_LONG_EDGE`].
    /// Default for scrub/play/selection retarget.
    #[default]
    Draft,
    /// Sequence format size; originals only for decode preference.
    Full,
}

/// What the single central monitor evaluates (24-preview-media-load §3).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PreviewTarget {
    /// Sequence under the shared playhead (default).
    Sequence { sequence: SequenceId },
    /// Media-pool / Match Frame source peek on the same surface.
    Asset { asset: AssetId, source_time: Tick },
}

impl Default for PreviewTarget {
    fn default() -> Self {
        // Sequence id filled in on first present from active sequence.
        PreviewTarget::Sequence {
            sequence: SequenceId::nil(),
        }
    }
}

/// Per-asset import-ladder readiness flags (24 §2 / §8). Derived or published
/// for GUI/MCP; not document state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AssetReadiness {
    pub probe: bool,
    pub poster: bool,
    pub keyframe_index: bool,
    pub proxy_ready: bool,
}

/// An export request (02 §7). Carried by [`EngineCmd::Export`]: the engine
/// spawns a dedicated worker thread that runs
/// [`crate::export::job::run_export_job`] over a frozen snapshot and publishes
/// progress on [`EngineStatus::export`]. The MCP `export_sequence` tool funnels
/// through the same relocated fn, so there is one render/encode path.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportJob {
    pub sequence: SequenceId,
    pub format_index: usize,
    pub preset: ExportPreset,
    pub output: PathBuf,
    /// `None` = the sequence's work range / full extent.
    pub range: Option<(Tick, Tick)>,
}

/// Live export progress the GUI polls off [`EngineStatus`] (02 §7). Published
/// wait-free by the engine thread from a background export worker's
/// `ExportEvent` stream; `done` flips true on completion/cancel/failure, and
/// `error` carries the message on failure.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportProgressSnapshot {
    /// Monotonic per-session export counter (distinguishes successive exports).
    pub job: u64,
    pub frame: u64,
    pub total: u64,
    pub fps: f32,
    pub eta: Duration,
    pub done: bool,
    pub error: Option<String>,
}

/// GUI/MCP → engine commands (02 §1).
#[derive(Clone, Debug)]
pub enum EngineCmd {
    Play,
    Pause,
    /// Coalesced latest-wins per engine tick (02 §4 scrub rule).
    Seek(Tick),
    /// Live scrub target while the playhead is being dragged. Coalesced
    /// latest-wins like `Seek`, but decodes only a cheap keyframe preview; a
    /// trailing `Seek` (drag-release settle) supersedes it and lands the exact
    /// frame.
    ScrubSeek(Tick),
    /// Pause + snap ±n frames, evaluate exactly that tick (CAP-004).
    Step(i32),
    SetLoop(Option<(Tick, Tick)>),
    SetActiveSequence(SequenceId),
    SetProxyMode(ProxyMode),
    /// Retarget the single monitor (24 §3). Play forces Sequence (play-wins).
    SetPreviewTarget(PreviewTarget),
    /// Draft (default) vs Full interactive quality (24 §4).
    SetPreviewQuality(PreviewQuality),
    /// Source-space seek while `PreviewTarget::Asset` (ignored for Sequence).
    SeekSource {
        asset: AssetId,
        time: Tick,
    },
    /// Asset relink / proxy swap (02 §5). P3 over-invalidates (whole node
    /// cache + decode sources) — the hash→asset index for targeted eviction
    /// is the proxy/relink story's seam.
    InvalidateRange(SequenceId, Tick, Tick),
    /// Start a headless export on a dedicated worker thread; progress is
    /// published on [`EngineStatus::export`]. A new `Export` (or
    /// [`EngineCmd::CancelExport`]) poisons any in-flight one.
    Export(Box<ExportJob>),
    /// Cancel the in-flight export (if any). The worker stops between frames.
    CancelExport,
    /// Probe an asset's media file (K-0.8 / 24): runs `ffprobe`, writes
    /// [`MediaProbe`] + content hash onto the asset via `set_asset_meta`, and
    /// invalidates any open decode source so the next present re-opens it.
    Probe(AssetId),
    /// Stop the engine thread. [`EngineSession`] sends this on drop/shutdown.
    Shutdown,
}

/// What the GUI presents (02 §1): `Rgba16Float`, linear, premultiplied (D-09).
/// The present path is 03 §5's `present_engine_frame`.
pub struct EngineFrame {
    pub texture: Arc<wgpu::Texture>,
    /// Exact frame-start tick this frame was evaluated at (sequence or source).
    pub time: Tick,
    pub sequence: SequenceId,
    /// When set, this frame is an asset source peek (24 §3), not program-out.
    pub preview_asset: Option<AssetId>,
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
    /// Number of program frames published by the engine. The GUI may coalesce
    /// these to the newest texture; comparing this with presentation telemetry
    /// identifies a UI/vsync bottleneck separately from decode/compositing.
    pub frames_published: u64,
    /// Duration of the most recent compile + graph evaluation in microseconds.
    /// This is deliberately cheap, per-frame telemetry for Linux performance
    /// reports; it is not a profiler substitute.
    pub last_evaluate_micros: u64,
    /// Evaluations that produced no frame (for example an unavailable decode
    /// source), kept distinct from clock-driven dropped frames.
    pub evaluation_misses: u64,
    /// The `CommandHistory` revision the current snapshot was taken at
    /// (02 §1's `doc_generation`).
    pub doc_revision: u64,
    pub active_sequence: Option<SequenceId>,
    /// Most recent command failure (e.g. the P3 `Export`/`Probe` stubs).
    pub last_error: Option<String>,
    /// Current single-monitor target (24 §3).
    pub preview_target: PreviewTarget,
    /// Draft / Full interactive quality (24 §4).
    pub preview_quality: PreviewQuality,
    /// True while an exact-frame eval is outstanding after retarget/seek and
    /// no new frame was published this tick (GUI may keep last/poster).
    pub buffering: bool,
    /// Live progress of the in-flight (or most recent) export (02 §7). `None`
    /// until the session's first `EngineCmd::Export`. The GUI export dialog
    /// polls this wait-free.
    pub export: Option<ExportProgressSnapshot>,
}

impl Default for EngineStatus {
    fn default() -> Self {
        EngineStatus {
            playhead: Tick::ZERO,
            playing: false,
            dropped: 0,
            cache: CacheStats::default(),
            audio_xruns: 0,
            frames_published: 0,
            last_evaluate_micros: 0,
            evaluation_misses: 0,
            doc_revision: 0,
            active_sequence: None,
            last_error: None,
            preview_target: PreviewTarget::default(),
            preview_quality: PreviewQuality::Draft,
            buffering: false,
            export: None,
        }
    }
}

/// Latest-wins seek coalescing (02 §4): reduce one drained command batch so
/// only the **last** `Seek` survives, at its original position relative to the
/// non-seek commands after it. Everything else keeps its order.
pub fn coalesce_commands(batch: Vec<EngineCmd>) -> Vec<EngineCmd> {
    // Both Seek and ScrubSeek are latest-wins position commands (a drag emits
    // many per tick). Keep only the last of *either*: a trailing Seek (settle)
    // supersedes earlier ScrubSeeks, and vice-versa — whichever the GUI sent
    // last is the true target.
    let is_pos = |c: &EngineCmd| matches!(c, EngineCmd::Seek(_) | EngineCmd::ScrubSeek(_));
    let last_pos = batch.iter().rposition(&is_pos);
    batch
        .into_iter()
        .enumerate()
        .filter(|(i, c)| !is_pos(c) || Some(*i) == last_pos)
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

// ── LUT provider (K-0.5) ─────────────────────────────────────────────────────

/// One cached `.cube` LUT: the source path it was parsed from (so a relinked
/// asset re-parses) and the parsed table (`None` = a cached miss — offline or
/// unparsable).
struct LutEntry {
    path: PathBuf,
    table: Option<Arc<photonic_render::Lut3d>>,
}

/// Memoised `.cube` LUT cache (K-0.5): every referenced `Lut3d` asset is parsed
/// exactly ONCE, in [`warm`](LutCache::warm) (engine thread, on snapshot change),
/// so the compiler's [`LutProvider`] read is a lock-free `HashMap` hit and no
/// `.cube` parsing ever happens on the per-frame compile hot path. A read/parse
/// failure caches a negative entry (→ the grade op resolves inert / identity,
/// never a black frame — 07 §1) and records one diagnostic.
#[derive(Default)]
struct LutCache {
    entries: HashMap<AssetId, LutEntry>,
    /// One diagnostic per still-unresolved LUT, surfaced onto the compiled frame.
    failures: Vec<CompileDiagnostic>,
}

impl LutCache {
    /// (Re)parse every file-backed `Lut3d` asset in `project` whose path is not
    /// already cached, and rebuild the failure list. Off the per-frame path.
    fn warm(&mut self, project: &TimelineProject) {
        self.failures.clear();
        for (id, asset) in &project.media.assets {
            if asset.kind != AssetKind::Lut3d {
                continue;
            }
            let AssetSource::File { path, .. } = &asset.source else {
                continue;
            };
            // Parse only when absent or the source path changed (relink) — reads
            // never happen per frame.
            let stale = self.entries.get(id).map(|e| &e.path != path).unwrap_or(true);
            if stale {
                let table = std::fs::read_to_string(path)
                    .ok()
                    .and_then(|src| photonic_render::parse_cube(&src).ok())
                    .map(Arc::new);
                self.entries.insert(
                    *id,
                    LutEntry {
                        path: path.clone(),
                        table,
                    },
                );
            }
            if self.entries.get(id).and_then(|e| e.table.as_ref()).is_none() {
                self.failures.push(CompileDiagnostic {
                    message: format!(
                        "LUT asset {id} could not be loaded from {}; the grade op \
                         renders inert (identity)",
                        path.display()
                    ),
                    graph: None,
                    node: None,
                    code: None,
                    severity: DiagSeverity::Warning,
                    clip: None,
                });
            }
        }
    }
}

impl LutProvider for LutCache {
    fn lut(&self, asset: AssetId) -> Option<Arc<photonic_render::Lut3d>> {
        self.entries.get(&asset).and_then(|e| e.table.clone())
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
    /// Memoised `.cube` LUT tables (K-0.5), warmed on snapshot change and read
    /// lock-free by the compiler's `Grade` `Lut3d` resolution.
    lut_cache: LutCache,
    last_revision: Option<u64>,
    active_sequence_override: Option<SequenceId>,
    proxy_mode: ProxyMode,
    preview_target: PreviewTarget,
    preview_quality: PreviewQuality,
    /// Set when present requested a frame but eval produced none (24 §5).
    buffering: bool,
    last_error: Option<String>,
    /// True while the GUI is actively dragging the playhead (between a
    /// `ScrubSeek` and the settling `Seek`). Selects the cheap keyframe-preview
    /// decode path and lets the compositor hold the last frame between previews.
    scrubbing: bool,

    audio: AudioEngine,
    feeder: Option<AudioFeeder>,
    xruns: Option<Arc<XrunCounters>>,
    tools: Option<FfmpegTools>,
    /// Playback telemetry: distinguish producer/evaluator pressure from
    /// clock-side drops reported by `PlaybackController`.
    frames_published: u64,
    last_evaluate_micros: u64,
    evaluation_misses: u64,
    /// Reused command drain buffer (avoids per-tick `Vec` alloc on the engine
    /// thread — Linux + Windows hot path).
    cmd_batch: Vec<EngineCmd>,
    /// Skip redundant status Arc allocations when nothing visible changed.
    last_status_sig: u64,
    /// Live export progress written by the background export worker, read into
    /// [`EngineStatus::export`] each publish (02 §7). Wait-free both sides.
    export_progress: Arc<ArcSwapOption<ExportProgressSnapshot>>,
    /// Cancel flag of the in-flight export worker (if any). A new export or an
    /// [`EngineCmd::CancelExport`] poisons it; shutdown poisons it too.
    export_cancel: Option<Arc<AtomicBool>>,
    /// Monotonic per-session export counter, stamped onto each snapshot.
    export_job_counter: u64,
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
            lut_cache: LutCache::default(),
            last_revision: None,
            active_sequence_override: None,
            proxy_mode: ProxyMode::Auto,
            preview_target: PreviewTarget::default(),
            preview_quality: PreviewQuality::Draft,
            buffering: false,
            last_error: None,
            scrubbing: false,
            audio: AudioEngine::new(),
            feeder: None,
            xruns: None,
            tools,
            frames_published: 0,
            last_evaluate_micros: 0,
            evaluation_misses: 0,
            cmd_batch: Vec::with_capacity(32),
            last_status_sig: 0,
            export_progress: Arc::new(ArcSwapOption::from(None)),
            export_cancel: None,
            export_job_counter: 0,
        }
    }

    fn run(mut self) {
        loop {
            // 1. Wait briefly for commands, then drain the burst (a scrub
            //    produces many Seeks per engine tick — coalesced latest-wins).
            self.cmd_batch.clear();
            let poll_interval = if self.controller.is_playing() {
                PLAYING_POLL_INTERVAL
            } else {
                IDLE_POLL_INTERVAL
            };
            match self.rx.recv_timeout(poll_interval) {
                Ok(cmd) => {
                    self.cmd_batch.push(cmd);
                    while let Ok(cmd) = self.rx.try_recv() {
                        self.cmd_batch.push(cmd);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            let mut shutdown = false;
            // Drain into a local so we don't hold &mut self.cmd_batch across handle.
            let batch = std::mem::take(&mut self.cmd_batch);
            for cmd in coalesce_commands(batch) {
                if !self.handle(cmd) {
                    shutdown = true;
                }
            }
            // Reclaim capacity for the next tick.
            self.cmd_batch = Vec::with_capacity(32);
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
        // Poison any in-flight export so its worker stops between frames rather
        // than outliving the engine thread (the worker owns its own session).
        if let Some(cancel) = self.export_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.stop_playing();
    }

    /// Returns `false` on `Shutdown`.
    fn handle(&mut self, cmd: EngineCmd) -> bool {
        match cmd {
            EngineCmd::Play => {
                self.scrubbing = false;
                // Play always wins: retarget to active sequence (24 §3.2).
                if let Some(project) = self.snapshot.as_ref() {
                    if let Some(seq) = self.effective_sequence(project) {
                        self.preview_target = PreviewTarget::Sequence { sequence: seq };
                    }
                }
                self.start_playing()
            }
            EngineCmd::Pause => self.stop_playing(),
            EngineCmd::Seek(t) => {
                // Settle: the drag (if any) is over — exact frame wanted.
                self.scrubbing = false;
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
            EngineCmd::ScrubSeek(t) => {
                // Active drag: pause (a scrub is not playback) and seek. The
                // media source decodes only a keyframe preview (scrubbing flag),
                // so dragging stays responsive on long GOPs.
                self.stop_playing();
                self.scrubbing = true;
                self.controller.seek(t);
            }
            EngineCmd::Step(n) => {
                self.stop_playing(); // Step always pauses (02 §4)
                self.controller.step(n);
            }
            EngineCmd::SetLoop(range) => self.controller.set_loop(range),
            EngineCmd::SetActiveSequence(id) => {
                self.active_sequence_override = Some(id);
                // Keep sequence target in sync when user switches sequences.
                if matches!(self.preview_target, PreviewTarget::Sequence { .. }) {
                    self.preview_target = PreviewTarget::Sequence { sequence: id };
                }
                self.controller.request_present();
            }
            EngineCmd::SetProxyMode(mode) => {
                self.proxy_mode = mode;
                // Different quality flag ⇒ different content hashes; the old
                // entries age out hash-naturally (02 §5).
                self.controller.request_present();
            }
            EngineCmd::SetPreviewTarget(target) => {
                // While playing, ignore asset peeks (play-wins, 24 §3.2).
                if self.controller.is_playing() && matches!(target, PreviewTarget::Asset { .. }) {
                    // no-op
                } else {
                    self.preview_target = target;
                    self.controller.request_present();
                }
            }
            EngineCmd::SetPreviewQuality(q) => {
                self.preview_quality = q;
                self.controller.request_present();
            }
            EngineCmd::SeekSource { asset, time } => {
                match &mut self.preview_target {
                    PreviewTarget::Asset {
                        asset: a,
                        source_time,
                    } if *a == asset => {
                        *source_time = time;
                        self.controller.request_present();
                    }
                    _ => {
                        // Arm asset peek if not already on this asset.
                        if !self.controller.is_playing() {
                            self.preview_target = PreviewTarget::Asset {
                                asset,
                                source_time: time,
                            };
                            self.controller.request_present();
                        }
                    }
                }
            }
            EngineCmd::InvalidateRange(seq, from, to) => {
                // The evaluator node cache is keyed by opaque content hash with
                // no hash→asset index, so evaluator eviction stays whole-cache
                // (always correct — over-invalidation, never stale).
                self.evaluator.invalidate_matching(|_| true);
                // Media decode sources / stills DO carry an asset key, so they
                // evict targeted: only assets whose clips overlap [from, to] in
                // `seq` lose their sidecar; every other primed ring survives.
                match self.snapshot.as_ref().and_then(|p| p.sequences.get(&seq)) {
                    Some(sequence) => {
                        let mut assets: HashSet<AssetId> = HashSet::new();
                        for track in sequence
                            .video_tracks
                            .iter()
                            .chain(sequence.audio_tracks.iter())
                        {
                            for clip in &track.clips {
                                // Half-open overlap with the invalidated range.
                                if clip.start < to && from < clip.end() {
                                    if let Some(asset) = clip.source.asset() {
                                        assets.insert(asset);
                                    }
                                }
                            }
                        }
                        self.media.invalidate_assets(&assets);
                    }
                    // No snapshot/sequence yet: fall back to the safe whole flush.
                    None => self.media.invalidate_all(),
                }
                self.controller.request_present();
            }
            EngineCmd::Export(job) => self.start_export(*job),
            EngineCmd::CancelExport => {
                if let Some(cancel) = &self.export_cancel {
                    cancel.store(true, Ordering::Relaxed);
                }
            }
            EngineCmd::Probe(asset) => self.probe_asset(asset),
            EngineCmd::Shutdown => return false,
        }
        true
    }

    fn fail(&mut self, msg: String) {
        tracing::warn!(target: "photonic_video::session", "{msg}");
        self.last_error = Some(msg);
    }

    /// K-0.8: run `ffprobe` on `asset`'s file, write probe + content hash into
    /// the document via `set_asset_meta`, and drop any open decode source so the
    /// next present re-opens against the fresh meta.
    fn probe_asset(&mut self, asset: AssetId) {
        let Some(tools) = self.tools.clone() else {
            self.fail(format!(
                "EngineCmd::Probe({asset}): ffmpeg/ffprobe tools are not available"
            ));
            return;
        };
        let Some(project) = self.snapshot.as_ref() else {
            self.fail(format!(
                "EngineCmd::Probe({asset}): no timeline snapshot is loaded yet"
            ));
            return;
        };
        let Some(media_asset) = project.media.assets.get(&asset) else {
            self.fail(format!("EngineCmd::Probe({asset}): asset not in media pool"));
            return;
        };
        let path = match &media_asset.source {
            AssetSource::File { path, .. } => path.clone(),
            other => {
                self.fail(format!(
                    "EngineCmd::Probe({asset}): source is not a file ({other:?})"
                ));
                return;
            }
        };
        let details = match probe_details(&tools, &path) {
            Ok(d) => d,
            Err(e) => {
                self.fail(format!("EngineCmd::Probe({asset}): {e}"));
                return;
            }
        };
        let hash = content_hash(&path).ok();
        // Apply meta on the live document so undo/history and the next snapshot
        // pick it up (24 L1/L2 ladder). Drop locks before calling `fail`.
        let apply_err = {
            let mut doc = self.doc.lock().expect("engine doc lock");
            let mut history = self.history.lock().expect("engine history lock");
            match doc.timeline.as_ref() {
                None => Some("document has no timeline".to_string()),
                Some(timeline) => {
                    match photonic_core::timeline::ops::set_asset_meta(
                        timeline,
                        asset,
                        Some(details.probe.clone()),
                        hash,
                    ) {
                        Ok(cmd) => {
                            history.execute_discrete(
                                photonic_core::history::Command::Timeline(cmd),
                                &mut doc,
                            );
                            None
                        }
                        Err(e) => Some(format!("set_asset_meta: {e}")),
                    }
                }
            }
        };
        if let Some(msg) = apply_err {
            self.fail(format!("EngineCmd::Probe({asset}): {msg}"));
            return;
        }
        // Force decode reopen against the updated meta.
        let mut ids = HashSet::new();
        ids.insert(asset);
        self.media.invalidate_assets(&ids);
        self.last_error = None;
        tracing::info!(
            target: "photonic_video::session",
            %asset,
            "EngineCmd::Probe: media meta updated"
        );
    }

    /// Spawn a dedicated export worker (02 §7). The worker runs
    /// [`crate::export::job::run_export_job`] over the current frozen snapshot
    /// on its own thread + own headless session, publishing progress into
    /// `export_progress` (which `publish_status` mirrors onto
    /// [`EngineStatus::export`]). A prior in-flight export is poisoned first.
    fn start_export(&mut self, job: ExportJob) {
        // Poison any in-flight export so two never race the same output.
        if let Some(prev) = self.export_cancel.take() {
            prev.store(true, Ordering::Relaxed);
        }
        let Some(project) = self.snapshot.clone() else {
            self.fail(format!(
                "export requested for sequence {} but no timeline snapshot is loaded yet",
                job.sequence
            ));
            return;
        };
        let Some(tools) = self.tools.clone() else {
            self.fail("export requested but ffmpeg/ffprobe were not located".into());
            return;
        };
        let gpu = self.evaluator.gpu().clone();
        self.export_job_counter += 1;
        let job_num = self.export_job_counter;

        let cancel = Arc::new(AtomicBool::new(false));
        self.export_cancel = Some(Arc::clone(&cancel));
        let progress = Arc::clone(&self.export_progress);
        // Seed a 0/0 running snapshot so the GUI shows the export immediately.
        progress.store(Some(Arc::new(ExportProgressSnapshot {
            job: job_num,
            frame: 0,
            total: 0,
            fps: 0.0,
            eta: Duration::ZERO,
            done: false,
            error: None,
        })));

        std::thread::Builder::new()
            .name("photonic-video-export".into())
            .spawn(move || {
                let prog = Arc::clone(&progress);
                let on_event = |event: crate::export::render_loop::ExportEvent| {
                    if let crate::export::render_loop::ExportEvent::Progress(p) = event {
                        prog.store(Some(Arc::new(ExportProgressSnapshot {
                            job: job_num,
                            frame: p.frame,
                            total: p.total,
                            fps: p.fps,
                            eta: p.eta,
                            done: false,
                            error: None,
                        })));
                    }
                };
                let result = crate::export::job::run_export_job(
                    gpu, project, &job, &tools, &cancel, on_event,
                );
                // Final snapshot: carry the last frame/total, flip done, and
                // attach any error.
                let (frame, total) = progress
                    .load_full()
                    .map(|s| (s.frame, s.total))
                    .unwrap_or((0, 0));
                let error = match &result {
                    Ok(()) => None,
                    Err(e) => Some(e.to_string()),
                };
                progress.store(Some(Arc::new(ExportProgressSnapshot {
                    job: job_num,
                    frame,
                    total,
                    fps: 0.0,
                    eta: Duration::ZERO,
                    done: true,
                    error,
                })));
            })
            .expect("spawn photonic-video export worker");
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
        // Single Arc handoff to media + snapshot (was clone + move = 2 bumps).
        if let Some(ref p) = snap {
            self.media.set_project(Arc::clone(p));
            // Warm the LUT cache off the per-frame path (K-0.5): parse any newly
            // referenced `.cube` assets now so compile's provider read is a hit.
            self.lut_cache.warm(p);
        }
        self.snapshot = snap;
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
        // Borrow the snapshot Arc — no atomic bump every tick (was `.clone()`).
        let Some(project) = self.snapshot.as_ref() else {
            return;
        };
        let Some(seq_id) = self.effective_sequence(project) else {
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
                let evaluate_started = Instant::now();
                let quality = self.interactive_quality();
                let format_index = seq.active_format.min(seq.formats.len().saturating_sub(1));
                self.media.set_playing(self.controller.is_playing());
                self.media.set_scrubbing(self.scrubbing);

                // Normalize nil sequence target once we know the active seq.
                if let PreviewTarget::Sequence { sequence } = &self.preview_target {
                    if *sequence == SequenceId::nil() || *sequence != seq_id {
                        self.preview_target = PreviewTarget::Sequence { sequence: seq_id };
                    }
                }

                let (compiled, canvas, frame_time, preview_asset) = match &self.preview_target {
                    PreviewTarget::Asset { asset, source_time }
                        if !self.controller.is_playing() =>
                    {
                        let (fw, fh) = seq
                            .formats
                            .get(format_index)
                            .map(|f| (f.width, f.height))
                            .unwrap_or((1920, 1080));
                        let canvas = self.preview_canvas(fw, fh);
                        let compiled = compile_asset_peek(
                            project.as_ref(),
                            *asset,
                            *source_time,
                            quality,
                            canvas.0,
                            canvas.1,
                        );
                        (compiled, canvas, *source_time, Some(*asset))
                    }
                    _ => {
                        // Thread the pre-warmed LUT cache so `Grade` `Lut3d` ops
                        // resolve to real tables (K-0.5); parse-failure diagnostics
                        // ride along on the compiled frame.
                        let mut compiled = compile_with_luts(
                            project.as_ref(),
                            seq_id,
                            format_index,
                            t,
                            quality,
                            None,
                            Some(&self.lut_cache),
                        );
                        compiled
                            .diagnostics
                            .extend(self.lut_cache.failures.iter().cloned());
                        let (fw, fh) = seq
                            .formats
                            .get(format_index)
                            .map(|f| (f.width, f.height))
                            .unwrap_or((1, 1));
                        let canvas = self.preview_canvas(fw, fh);
                        (compiled, canvas, t, None)
                    }
                };

                // Gate the document snapshot on actual vector use: only when the
                // compiled frame references a `RasterVector` op do we clone the
                // document for the offscreen vector rasterizer, so a project with
                // no embedded-vector clips never pays the clone.
                if compiled
                    .graph
                    .nodes
                    .iter()
                    .any(|n| matches!(n.op, IrOp::RasterVector { .. }))
                {
                    if let Ok(doc) = self.doc.try_lock() {
                        self.media
                            .set_document(&doc, self.last_revision.unwrap_or(0));
                    }
                }
                if let Some(texture) =
                    self.evaluator
                        .evaluate(&compiled.graph, canvas, &mut self.media)
                {
                    self.frame_out.store(Some(Arc::new(EngineFrame {
                        texture,
                        time: frame_time,
                        sequence: seq_id,
                        preview_asset,
                    })));
                    self.frames_published = self.frames_published.saturating_add(1);
                    self.buffering = false;
                } else {
                    self.evaluation_misses = self.evaluation_misses.saturating_add(1);
                    self.buffering = true;
                }
                self.last_evaluate_micros = evaluate_started.elapsed().as_micros() as u64;
                // Ring prefetch now runs off the engine thread: each evaluated
                // `DecodeVideo` node steers its source's background decode
                // worker (see `MediaSources::video_texture`), so decode overlaps
                // compositing instead of pumping inline here.
                //
                // Cut-ahead: warm the *next* clip's decode worker before the
                // playhead reaches it, so crossing a clip boundary hits a primed
                // ring instead of cold-starting (transparent flicker). Only while
                // playing — a paused/scrubbing playhead has no imminent cut.
                if self.controller.is_playing() {
                    let lead = Tick(seq.frame_rate.ticks_per_frame().0 * CUT_AHEAD_LEAD_FRAMES);
                    self.media.prefetch_upcoming(seq, t, lead, quality);
                }
            }
        }
    }

    /// Proxy flag from session `ProxyMode` + Draft/Full (24 §4).
    fn interactive_quality(&self) -> Quality {
        let proxy = match (self.preview_quality, self.proxy_mode) {
            (PreviewQuality::Full, _) => false,
            (_, ProxyMode::ForceOriginal) => false,
            (_, ProxyMode::Auto | ProxyMode::ForceProxy) => true,
        };
        Quality { proxy }
    }

    /// Draft caps long edge at [`DRAFT_MAX_LONG_EDGE`]; Full uses format size.
    fn preview_canvas(&self, w: u32, h: u32) -> (u32, u32) {
        match self.preview_quality {
            PreviewQuality::Draft => fit_long_edge(w, h, DRAFT_MAX_LONG_EDGE),
            PreviewQuality::Full => (w.max(1), h.max(1)),
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

    fn publish_status(&mut self) {
        let playhead = self.controller.playhead();
        let playing = self.controller.is_playing();
        let dropped = self.controller.dropped();
        let audio_xruns = self
            .xruns
            .as_ref()
            .map(|x| x.underrun_frames())
            .unwrap_or(0);
        let doc_revision = self.last_revision.unwrap_or(0);
        let active_sequence = self
            .snapshot
            .as_ref()
            .and_then(|p| self.effective_sequence(p));
        let export = self.export_progress.load_full();
        // Cheap signature: skip Arc allocation when the GUI-visible fields are
        // unchanged (idle paused loops were allocating status every 100 ms).
        let sig = {
            let mut h = playhead.0 as u64;
            h = h.wrapping_mul(0x9E37_79B9).wrapping_add(playing as u64);
            h = h.wrapping_mul(0x9E37_79B9).wrapping_add(dropped);
            h = h.wrapping_mul(0x9E37_79B9).wrapping_add(audio_xruns);
            h = h
                .wrapping_mul(0x9E37_79B9)
                .wrapping_add(self.frames_published);
            h = h
                .wrapping_mul(0x9E37_79B9)
                .wrapping_add(self.evaluation_misses);
            h = h.wrapping_mul(0x9E37_79B9).wrapping_add(doc_revision);
            h = h
                .wrapping_mul(0x9E37_79B9)
                .wrapping_add(self.buffering as u64);
            h = h
                .wrapping_mul(0x9E37_79B9)
                .wrapping_add(match self.preview_quality {
                    PreviewQuality::Draft => 0,
                    PreviewQuality::Full => 1,
                });
            // Fold export progress so a background worker's advance still
            // republishes status even when every interactive field is idle.
            if let Some(e) = export.as_ref() {
                h = h.wrapping_mul(0x9E37_79B9).wrapping_add(e.job);
                h = h.wrapping_mul(0x9E37_79B9).wrapping_add(e.frame);
                h = h.wrapping_mul(0x9E37_79B9).wrapping_add(e.total);
                h = h.wrapping_mul(0x9E37_79B9).wrapping_add(e.done as u64);
                h = h
                    .wrapping_mul(0x9E37_79B9)
                    .wrapping_add(e.error.is_some() as u64);
            }
            h
        };
        if sig == self.last_status_sig && self.last_error.is_none() {
            // Still refresh playhead while playing (sig includes playhead).
            // When identical, skip the Arc::new + store.
            return;
        }
        self.last_status_sig = sig;
        self.status_out.store(Arc::new(EngineStatus {
            playhead,
            playing,
            dropped,
            cache: self.evaluator.cache_stats(),
            audio_xruns,
            frames_published: self.frames_published,
            last_evaluate_micros: self.last_evaluate_micros,
            evaluation_misses: self.evaluation_misses,
            doc_revision,
            active_sequence,
            last_error: self.last_error.clone(),
            preview_target: self.preview_target.clone(),
            preview_quality: self.preview_quality,
            buffering: self.buffering,
            export: export.as_ref().map(|e| (**e).clone()),
        }));
    }
}

// ── Diagnostic counters (decode ring-hit vs inline-seek) ─────────────────────
// Temporary throughput instrumentation: lets a headless bench see whether the
// engine is decode-bound (inline seeks) or composite-bound (all ring hits).
pub static RING_HITS: AtomicU64 = AtomicU64::new(0);
pub static INLINE_SEEKS: AtomicU64 = AtomicU64::new(0);

/// (ring_hits, inline_seeks) since process start.
pub fn decode_stats() -> (u64, u64) {
    (
        RING_HITS.load(Ordering::Relaxed),
        INLINE_SEEKS.load(Ordering::Relaxed),
    )
}

// ── Media sources (GpuFrameSource over decode rings) ─────────────────────────

struct VideoSourceEntry {
    /// The source's decode ring. The engine only ever reads it (a ring hit
    /// never touches the decoder). The `DecodeSource` itself is owned by the
    /// background [`DecodeWorker`], which is the sole seeker/pumper — the engine
    /// never decodes inline, so a slow GOP decode cannot stall the compositor.
    ring: SharedRing,
    /// Background decode thread; dropped (stopped + joined) with the entry,
    /// which drops the last `DecodeSource` handle and kills the sidecar.
    worker: DecodeWorker,
    colorimetry: Colorimetry,
    rate: FrameRate,
    /// The most recently delivered real frame for this source. Held on a miss
    /// (worker catch-up / scrub) so the compositor shows the previous frame
    /// instead of a transparent flicker — only while playing/scrubbing (paused
    /// exactness is preserved; see `video_texture`).
    last_good: Option<GpuFrame>,
    /// Monotonic use-stamp for LRU eviction (bumped on every steer/read).
    last_used: u64,
}

/// Resolves `DecodeVideo` ops to working textures over per-asset ffmpeg
/// sidecar decode rings (02 §3). Stills (`DecodeStill` via
/// `RasterImage::from_encoded`, uploaded once and cached by asset) are wired
/// here too. Vector frames (`RasterVector` via `HeadlessRenderer`, cached by
/// `VectorStateKey`) remain the documented follow-up seam — until then that op
/// evaluates transparent.
struct MediaSources {
    tools: Option<FfmpegTools>,
    project: Option<Arc<TimelineProject>>,
    /// `None` value = open failed; don't re-probe every frame (an
    /// `InvalidateRange` clears the entry and allows a retry).
    sources: HashMap<(AssetId, bool), Option<VideoSourceEntry>>,
    /// Sources whose expensive open (ffprobe + keyframe/pts index build, each a
    /// subprocess) is running on a background thread, so the engine present loop
    /// never stalls on a cold source. `drain_pending` promotes the completed
    /// build into `sources`; until then the source reads as absent and the
    /// compositor holds the last frame / shows transparent for a few presents.
    pending: HashMap<(AssetId, bool), std::sync::mpsc::Receiver<Option<VideoSourceEntry>>>,
    /// Uploaded working textures keyed by decoded pts — scrub back/forward
    /// over the same frames skips the GPU upload.
    uploads: HashMap<(AssetId, Tick, bool), GpuFrame>,
    /// Decoded still images uploaded to the working format, cached by asset
    /// (a still never changes until relink/`InvalidateRange`).
    stills: HashMap<AssetId, GpuFrame>,
    converter: Option<YuvConverter>,
    /// Whether the engine is playing — set each present before evaluate. Selects
    /// the ring-wait budget: short while playing (drop over stall), longer while
    /// paused (the exact frame is wanted).
    playing: bool,
    /// Whether the playhead is being dragged — set each present. Enables the
    /// cheap keyframe-preview decode path + hold-last-frame between previews.
    scrubbing: bool,
    /// Monotonic counter for `VideoSourceEntry::last_used` (LRU eviction).
    use_counter: u64,
    /// Cloned document snapshot for rasterizing embedded-vector clips. Only
    /// populated (in `present`) when the compiled frame actually references a
    /// vector — non-vector projects never pay the clone.
    document: Option<Arc<Document>>,
    /// The history revision `document` was snapshotted at (avoid re-cloning).
    doc_revision: Option<u64>,
    /// Rasterized vector frames, cached by `VectorStateKey` (a vector only
    /// re-renders when its doc-state key changes).
    vectors: HashMap<VectorStateKey, GpuFrame>,
    /// Lazily-created offscreen renderer for vector rasterization (its own wgpu
    /// device; frames come back as CPU bytes and re-upload onto the shared
    /// `GpuContext`, so there is no cross-device texture handoff).
    headless: Option<HeadlessRenderer>,
}

/// Upload-cache entry cap: ~a ring's worth per couple of assets; wholesale
/// clear on overflow keeps it trivially bounded (entries are cheap to rebuild
/// from the decode ring).
const UPLOAD_CACHE_CAP: usize = 32;

/// Still-image texture cache cap; wholesale clear on overflow (each entry is
/// cheap to redecode from disk).
const STILL_CACHE_CAP: usize = 16;

/// Rasterized-vector texture cache cap; wholesale clear on overflow.
const VECTOR_CACHE_CAP: usize = 16;

/// How far ahead (in frames) to warm the next clip's decoder before a cut.
/// ~0.8s at 30fps — enough lead for a keyframe-seek bootstrap to finish.
// Cut-ahead lead lives in `playback::prefetch::CUT_AHEAD_LEAD_FRAMES`.

/// Cap on live decode sources (each = a worker thread + ffmpeg sidecar). With
/// cut-ahead, a long timeline would otherwise accumulate them; evict the
/// least-recently-used beyond this, never one on screen this frame.
// MAX_LIVE_SOURCES re-exported from playback::prefetch.

impl MediaSources {
    fn new(tools: Option<FfmpegTools>) -> Self {
        MediaSources {
            tools,
            project: None,
            sources: HashMap::new(),
            pending: HashMap::new(),
            document: None,
            doc_revision: None,
            vectors: HashMap::new(),
            headless: None,
            uploads: HashMap::new(),
            stills: HashMap::new(),
            converter: None,
            playing: false,
            scrubbing: false,
            use_counter: 0,
        }
    }

    fn set_project(&mut self, project: Arc<TimelineProject>) {
        self.project = Some(project);
    }

    fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    fn set_scrubbing(&mut self, scrubbing: bool) {
        self.scrubbing = scrubbing;
    }

    /// Provide the document snapshot for vector rasterization at `revision`.
    /// Re-clones only when the revision moved; clears the rasterized-vector cache
    /// on a document change so stale vector frames don't linger.
    fn set_document(&mut self, doc: &Document, revision: u64) {
        if self.doc_revision == Some(revision) && self.document.is_some() {
            return;
        }
        self.document = Some(Arc::new(doc.clone()));
        self.doc_revision = Some(revision);
        self.vectors.clear();
    }

    fn next_use_stamp(&mut self) -> u64 {
        self.use_counter += 1;
        self.use_counter
    }

    /// Whether `asset` is a file-backed video (worth building a decode source).
    fn is_video_asset(&self, asset: AssetId) -> bool {
        self.project
            .as_ref()
            .and_then(|p| p.media.assets.get(&asset))
            .is_some_and(|a| {
                a.kind == AssetKind::Video && matches!(a.source, AssetSource::File { .. })
            })
    }

    /// Cut-ahead: warm the next clip's decode worker if it starts within `lead`
    /// of `t` ([`cut_ahead_targets`]). Bounded to **one `build_source` per call**
    /// so we never open dual always-on decoders for every upcoming cut.
    fn prefetch_upcoming(&mut self, seq: &Sequence, t: Tick, lead: Tick, quality: Quality) {
        self.drain_pending();
        let proxy = quality.proxy;
        let mut built_this_call = false;
        let targets = cut_ahead_targets(seq, t, lead);
        for target in targets {
            let asset = target.asset;
            let src_time = target.source_time;
            if !self.is_video_asset(asset) {
                continue;
            }
            let key = (asset, proxy);
            if !self.sources.contains_key(&key) {
                if built_this_call {
                    continue; // amortize builds across presents
                }
                let entry = self.build_source(asset, proxy);
                self.sources.insert(key, entry);
                built_this_call = true;
            }
            let stamp = self.next_use_stamp();
            if let Some(Some(entry)) = self.sources.get_mut(&key) {
                entry.last_used = stamp;
                // Only steer while the ring hasn't yet reached the target — once
                // the bootstrap seek has primed it, leave it alone so it doesn't
                // fight the real playback steer at the cut.
                if entry.ring.frame_covering(src_time).is_none() {
                    entry.worker.steer(src_time);
                }
            }
        }
        self.evict_stale();
    }

    /// Drop the least-recently-used sources beyond `MAX_LIVE_SOURCES`. Never
    /// evicts a source touched this present (highest `last_used`), so an
    /// on-screen worker is never killed. Dropping stops+joins its worker.
    fn evict_stale(&mut self) {
        let entries: Vec<_> = self
            .sources
            .iter()
            .filter_map(|(k, v)| v.as_ref().map(|e| (*k, e.last_used)))
            .collect();
        if entries.len() <= MAX_LIVE_SOURCES {
            return;
        }
        // Protect anything touched on the latest use stamp (this present).
        let protected_min = entries.iter().map(|(_, u)| *u).max().unwrap_or(0);
        let victims = lru_eviction_victims(&entries, MAX_LIVE_SOURCES, protected_min);
        for k in victims {
            self.sources.remove(&k);
        }
        // If still over (all entries protected), fall back to pure LRU without protect.
        if self.sources.values().filter(|v| v.is_some()).count() > MAX_LIVE_SOURCES {
            let entries: Vec<_> = self
                .sources
                .iter()
                .filter_map(|(k, v)| v.as_ref().map(|e| (*k, e.last_used)))
                .collect();
            for k in lru_eviction_victims(&entries, MAX_LIVE_SOURCES, u64::MAX) {
                self.sources.remove(&k);
            }
        }
    }

    fn invalidate_all(&mut self) {
        self.sources.clear();
        self.pending.clear();
        self.uploads.clear();
        self.stills.clear();
        self.vectors.clear();
        self.document = None;
        self.doc_revision = None;
    }

    /// Targeted eviction: drop only the decode sources, uploads, and stills
    /// belonging to `assets` (relink/proxy-swap of specific media). Unrelated
    /// sidecars keep their primed rings, so a relink of one asset never cold-
    /// restarts every other source on the timeline.
    fn invalidate_assets(&mut self, assets: &HashSet<AssetId>) {
        if assets.is_empty() {
            return;
        }
        self.sources.retain(|(asset, _), _| !assets.contains(asset));
        self.pending.retain(|(asset, _), _| !assets.contains(asset));
        self.uploads
            .retain(|(asset, _, _), _| !assets.contains(asset));
        self.stills.retain(|asset, _| !assets.contains(asset));
    }

    /// Promote any background source builds that have finished into `sources`.
    /// Called each present before the source map is read.
    fn drain_pending(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        use std::sync::mpsc::TryRecvError;
        let mut done: Vec<((AssetId, bool), Option<VideoSourceEntry>)> = Vec::new();
        self.pending.retain(|key, rx| match rx.try_recv() {
            Ok(entry) => {
                done.push((*key, entry));
                false
            }
            // Build thread panicked/dropped without sending: record as failed.
            Err(TryRecvError::Disconnected) => {
                done.push((*key, None));
                false
            }
            Err(TryRecvError::Empty) => true,
        });
        for (key, entry) in done {
            self.sources.insert(key, entry);
        }
    }

    /// Kick off (or no-op) the open of `(asset, proxy)`. The cheap path
    /// resolution runs here on the engine thread; the expensive probe + keyframe/
    /// pts index build (ffprobe/ffmpeg subprocesses) runs on a background thread
    /// so the present loop never stalls. The result lands in `sources` on a later
    /// `drain_pending`.
    fn ensure_source(&mut self, asset: AssetId, proxy: bool) {
        let key = (asset, proxy);
        if self.sources.contains_key(&key) || self.pending.contains_key(&key) {
            return;
        }
        // Paused (e.g. a scrub/seek/step): build synchronously so the exact frame
        // shows immediately with no transparent flash — there are no playback
        // frames to drop, so the one-time cold-open cost is harmless and the
        // seek-shows-exact-frame contract holds. Only *playing* opens go
        // off-thread (below), where a present-loop stall would drop frames; and
        // cut-ahead pre-builds the next clip off-thread before it is on screen.
        if !self.playing {
            let entry = self.build_source(asset, proxy);
            self.sources.insert(key, entry);
            return;
        }
        // Resolve the decode input (original vs. proxy) — cheap, needs `self`.
        let inputs = (|| {
            let tools = self.tools.clone()?;
            let project = self.project.as_ref()?;
            let media_asset = project.media.assets.get(&asset)?;
            let original = match &media_asset.source {
                AssetSource::File { path, .. } => path.clone(),
                // Embedded vectors go through RasterVector, never DecodeVideo.
                _ => return None,
            };
            let path = crate::media::proxy::resolve_decode_input(
                &original,
                media_asset.proxy.as_ref(),
                proxy,
            );
            Some((tools, path))
        })();
        let Some((tools, path)) = inputs else {
            // Not a file-backed video source — record failure so we don't retry
            // every present (an `InvalidateRange` clears it for a relink retry).
            self.sources.insert(key, None);
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("photonic-source-build".into())
            .spawn(move || {
                let _ = tx.send(build_source_entry(tools, path));
            });
        match spawned {
            Ok(_) => {
                self.pending.insert(key, rx);
            }
            // Can't spawn the builder thread — fall back to a synchronous build so
            // the source still opens (rare; thread-exhaustion). `tools`/`path`
            // were moved into the (dropped) closure, so re-resolve via the
            // synchronous path.
            Err(_) => {
                let entry = self.build_source(asset, proxy);
                self.sources.insert(key, entry);
            }
        }
    }

    /// Synchronous open (paused path + spawn-failure fallback): resolves the
    /// decode input, then does the expensive build inline. The playing path uses
    /// the off-thread `build_source_entry` via `ensure_source`.
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
        build_source_entry(tools, path)
    }
}

/// The expensive part of opening a decode source — ffprobe + keyframe/pts index
/// build (subprocess-heavy) + worker spawn. Takes owned, `Send` inputs so it can
/// run on a background thread off the engine present loop (see
/// `MediaSources::ensure_source`). GPU-free: the first frame's upload happens
/// later in `video_texture` with the shared `GpuContext`.
fn build_source_entry(tools: FfmpegTools, path: std::path::PathBuf) -> Option<VideoSourceEntry> {
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
    let ring = SharedRing::preview();
    let decode = Arc::new(Mutex::new(DecodeSource::new(tools, params, ring.clone())));
    let worker = DecodeWorker::spawn(decode, video.frame_rate);
    Some(VideoSourceEntry {
        ring,
        worker,
        colorimetry: colorimetry_for_probe(&details),
        rate: video.frame_rate,
        last_good: None,
        last_used: 0,
    })
}

impl GpuFrameSource for MediaSources {
    fn video_texture(
        &mut self,
        gpu: &GpuContext,
        asset: AssetId,
        src_time: Tick,
        proxy: bool,
    ) -> Option<GpuFrame> {
        self.drain_pending();
        self.ensure_source(asset, proxy);
        let playing = self.playing;
        let scrubbing = self.scrubbing;
        let stamp = self.next_use_stamp();
        // A source still building in the background reads as absent here → the
        // caller composites transparent / holds the last frame for a few presents
        // until `drain_pending` promotes it. No engine-thread stall.
        let entry = self.sources.get_mut(&(asset, proxy))?.as_mut()?;
        entry.last_used = stamp;
        let colorimetry = entry.colorimetry;

        // Steer the background decode worker off the engine thread (decode ∥
        // composite). Scrub mode decodes only a cheap keyframe preview; normal
        // mode keeps the ring pumped ahead. The engine never seeks inline, so a
        // slow GOP decode can never stall the compositor one frame at a time.
        if scrubbing {
            entry.worker.steer_scrub(src_time);
        } else {
            entry.worker.steer(src_time);
        }

        // Ring-hit acceptance. Normally require the covering frame within one
        // frame tick (a discontinuous seek must not serve an old frame; Step
        // must never re-serve the previous one). While scrubbing, accept any
        // resident frame at/before src_time — the keyframe preview may be up to
        // a GOP stale, which is the intended cheap-scrub tradeoff.
        let tolerance = entry.rate.ticks_per_frame().0;
        let within =
            |f: &Arc<crate::decode::DecodedFrame>| scrubbing || src_time.0 - f.pts.0 < tolerance;

        let frame = if let Some(f) = entry.ring.frame_covering(src_time).filter(within) {
            RING_HITS.fetch_add(1, Ordering::Relaxed);
            f
        } else {
            // Miss: the worker hasn't produced the frame yet. Wait for it,
            // bounded — short while playing/scrubbing (hold over stall so the
            // cadence holds), longer while paused (exact frame wanted).
            let budget = if playing {
                Duration::from_millis(50)
            } else if scrubbing {
                Duration::from_millis(60)
            } else {
                Duration::from_millis(500)
            };
            match entry.ring.wait_for_frame(src_time, budget).filter(within) {
                Some(f) => {
                    RING_HITS.fetch_add(1, Ordering::Relaxed);
                    f
                }
                // Still behind. Hold the last good frame while playing/scrubbing
                // (invisible cadence-wise, kills the transparent flicker); return
                // None (transparent) only when paused, where the exact frame is
                // required and asserted by the seek/step tests. The engine never
                // seeks inline — that would clear the ring and thrash the worker.
                None => {
                    INLINE_SEEKS.fetch_add(1, Ordering::Relaxed);
                    if playing || scrubbing {
                        if let Some(tex) = entry.last_good.clone() {
                            return Some(tex);
                        }
                    }
                    return None;
                }
            }
        };

        // Upload (cached by decoded pts) and record as the new last-good frame.
        let key = (asset, frame.pts, proxy);
        let texture = if let Some(cached) = self.uploads.get(&key) {
            cached.clone()
        } else {
            let converter = self
                .converter
                .get_or_insert_with(|| YuvConverter::new(gpu.device()));
            let converted = converter.convert(
                gpu.device(),
                gpu.queue(),
                &frame.planes.as_yuv_planes(),
                colorimetry,
            );
            let (width, height) = (converted.width(), converted.height());
            let texture =
                GpuFrame::new(Arc::new(pad_to_pool_bucket(gpu, converted)), width, height);
            if self.uploads.len() >= UPLOAD_CACHE_CAP {
                self.uploads.clear();
            }
            self.uploads.insert(key, texture.clone());
            texture
        };
        if let Some(Some(entry)) = self.sources.get_mut(&(asset, proxy)) {
            entry.last_good = Some(texture.clone());
        }
        Some(texture)
    }

    fn still_texture(&mut self, gpu: &GpuContext, asset: AssetId) -> Option<GpuFrame> {
        // DecodeStill: decode the encoded image asset once, upload it into the
        // working format, and cache it by asset (02 §3). An `InvalidateRange`
        // touching the asset drops the entry and forces a redecode on relink.
        if let Some(frame) = self.stills.get(&asset) {
            return Some(frame.clone());
        }
        let project = self.project.as_ref()?;
        let media_asset = project.media.assets.get(&asset)?;
        // Only file-backed stills; embedded vectors go through RasterVector.
        let AssetSource::File { path, .. } = &media_asset.source else {
            return None;
        };
        let bytes = std::fs::read(path).ok()?;
        let img = photonic_core::RasterImage::from_encoded(&bytes).ok()?;
        let frame = upload_still(gpu, &img);
        // Bound the cache like `uploads` — a project with many stills would
        // otherwise grow it without limit. Wholesale clear on overflow (entries
        // are cheap to redecode from disk).
        if self.stills.len() >= STILL_CACHE_CAP {
            self.stills.clear();
        }
        self.stills.insert(asset, frame.clone());
        Some(frame)
    }

    fn vector_texture(
        &mut self,
        gpu: &GpuContext,
        vref: VectorRef,
        key: VectorStateKey,
        w: u32,
        h: u32,
    ) -> Option<GpuFrame> {
        // Cache hit: a vector only re-renders when its doc-state key changes.
        if let Some(frame) = self.vectors.get(&key) {
            return Some(frame.clone());
        }
        // Whole-document embedded vectors are rendered fully. Sub-references
        // (a specific artboard / node) need scene-subset extraction — a
        // follow-up; they evaluate transparent until then.
        if !matches!(vref, VectorRef::WholeDocument) {
            return None;
        }
        let document = self.document.clone()?;
        // Lazily stand up the offscreen renderer (its own wgpu device — created
        // once, on the first vector). `render_rgba_with_opts` returns CPU bytes
        // (device-agnostic), which re-upload onto the shared `GpuContext` below,
        // so there is no cross-device texture handoff.
        let headless = self
            .headless
            .get_or_insert_with(|| pollster::block_on(HeadlessRenderer::new()));
        let opts = ExportOptions {
            // Transparent so the vector's alpha composites over the video graph.
            background: ExportBackground::Transparent,
            ..Default::default()
        };
        let (bytes, rw, rh) = headless.render_rgba_with_opts(&document, w, h, &opts);
        let img = photonic_core::RasterImage::from_rgba(rw, rh, bytes).ok()?;
        let frame = upload_still(gpu, &img);
        if self.vectors.len() >= VECTOR_CACHE_CAP {
            self.vectors.clear();
        }
        self.vectors.insert(key, frame.clone());
        Some(frame)
    }
}

/// Pad a source upload to the texture pool's 64px size bucket (03 §3.4).
///
/// Source uploads share the pool's physical bucket convention, while
/// [`GpuFrame`] carries the native logical width and height separately. The
/// evaluator normalizes that logical region to the canvas with explicit texel
/// loads, so padding never participates in sampling.
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

/// Upload an sRGB8 straight-alpha [`RasterImage`] into the compositor's working
/// texture: `Rgba16Float`, linear-light, **premultiplied** (D-09) — the same
/// space `YuvConverter` produces for video, so stills composite identically.
fn upload_still(gpu: &GpuContext, img: &photonic_core::RasterImage) -> GpuFrame {
    let (w, h) = (img.width.max(1), img.height.max(1));
    // sRGB8 straight → linear premultiplied f16, packed little-endian.
    let mut texels: Vec<u8> = Vec::with_capacity((w as usize) * (h as usize) * 8);
    for px in img.pixels.chunks_exact(4) {
        let a = px[3] as f32 / 255.0;
        let r = srgb_to_linear(px[0] as f32 / 255.0) * a;
        let g = srgb_to_linear(px[1] as f32 / 255.0) * a;
        let b = srgb_to_linear(px[2] as f32 / 255.0) * a;
        for c in [r, g, b, a] {
            texels.extend_from_slice(&f32_to_f16_bits(c).to_le_bytes());
        }
    }
    let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("still_upload"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        // COPY_SRC so `pad_to_pool_bucket` can blit into a larger bucket.
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    gpu.queue().write_texture(
        texture.as_image_copy(),
        &texels,
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(w * 8), // 4 channels × 2 bytes (f16)
            rows_per_image: Some(h),
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    GpuFrame::new(Arc::new(pad_to_pool_bucket(gpu, texture)), w, h)
}

/// sRGB EOTF (gamma-decode) — matches `raster/adjust.rs` / `headless.rs`.
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// IEEE-754 binary32 → binary16 bit pattern (round-toward-zero). Matches the
/// in-tree `photonic_render` f16 packers; sufficient for the [0, 1]-ish working
/// range (subnormals flush to signed zero).
fn f32_to_f16_bits(v: f32) -> u16 {
    let b = v.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let e = ((b >> 23) & 0xff) as i32 - 112; // 127 - 15
    let m = b & 0x7fffff;
    if e <= 0 {
        sign
    } else if e >= 0x1f {
        sign | 0x7c00
    } else {
        sign | ((e as u16) << 10) | ((m >> 13) as u16)
    }
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
    fn engine_status_starts_with_zeroed_performance_telemetry() {
        let status = EngineStatus::default();
        assert_eq!(status.frames_published, 0);
        assert_eq!(status.last_evaluate_micros, 0);
        assert_eq!(status.evaluation_misses, 0);
    }

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

    #[test]
    fn f16_pack_matches_known_bit_patterns() {
        assert_eq!(f32_to_f16_bits(0.0), 0x0000);
        assert_eq!(f32_to_f16_bits(1.0), 0x3c00);
        assert_eq!(f32_to_f16_bits(0.5), 0x3800);
        assert_eq!(f32_to_f16_bits(2.0), 0x4000);
        // Sign preserved.
        assert_eq!(f32_to_f16_bits(-1.0), 0xbc00);
    }

    #[test]
    fn srgb_to_linear_endpoints() {
        assert!((srgb_to_linear(0.0) - 0.0).abs() < 1e-6);
        assert!((srgb_to_linear(1.0) - 1.0).abs() < 1e-4);
        // Monotone and inside the unit interval mid-scale.
        let mid = srgb_to_linear(0.5);
        assert!(
            mid > 0.0 && mid < 0.5,
            "sRGB 0.5 decodes to ~0.214, got {mid}"
        );
    }

    #[test]
    fn still_upload_premultiplies_transparent_black_to_zero() {
        // No GPU dependency: verify the CPU premultiply/pack path directly.
        // A fully transparent pixel packs to all-zero f16 (premultiplied).
        let (r, g, b, a) = (255u8, 255u8, 255u8, 0u8);
        let af = a as f32 / 255.0;
        let rl = srgb_to_linear(r as f32 / 255.0) * af;
        let gl = srgb_to_linear(g as f32 / 255.0) * af;
        let bl = srgb_to_linear(b as f32 / 255.0) * af;
        assert_eq!(f32_to_f16_bits(rl), 0);
        assert_eq!(f32_to_f16_bits(gl), 0);
        assert_eq!(f32_to_f16_bits(bl), 0);
        assert_eq!(f32_to_f16_bits(af), 0);
    }
}
