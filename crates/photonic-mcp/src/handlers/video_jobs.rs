//! Video-engine runtime + async-job infrastructure for the MCP surface
//! (10-mcp-tools.md §2 AppState extension, §6 job pattern).
//!
//! ## Engine bridge (§2, adapted)
//!
//! 10 §2 sketches `VideoEngine` construction in `main.rs` with the winit
//! `GpuContext` shared in GUI mode. That wiring is the app-integration
//! story's seam; this module keeps the MCP crate self-contained instead:
//! the engine is created **lazily on the first engine-backed tool call**
//! via [`photonic_video::VideoEngine::headless`] (own adapter, both run
//! modes). `None` from the adapter request = no GPU — every engine-backed
//! tool then degrades to a structured `EngineUnavailable` error (10 §7's
//! "fails with a clear error rather than blocking the rest of the surface").
//!
//! ## The shadow-document bridge
//!
//! `EngineSession` (02 §1) polls `Arc<std::sync::Mutex<{Document,
//! CommandHistory}>>`, but `AppState` shares its document/history behind
//! **tokio** mutexes (`server.rs:63`). Rather than fork the facade's lock
//! type (photonic-video is another story's territory), the bridge binds the
//! session to a *shadow* std-mutex pair and copies the `TimelineProject`
//! (cheap `Clone`, 01) from the real document into the shadow before every
//! engine-backed call ([`EngineBridge::sync_timeline`]), bumping the shadow
//! history's revision (`CommandHistory::reset`) so the engine re-snapshots.
//! Lock order is real-doc → shadow (never both shadow locks held while
//! taking the real one), and the engine thread only ever `try_lock`s the
//! shadow pair — no deadlock is possible across the three acquirers.
//!
//! ## Job registry (§6)
//!
//! First async-job pattern in the MCP surface (searched: none existed).
//! Start tools insert a [`JobHandle`] and return a `job_id` immediately;
//! `get_job_status`/`cancel_job` poll/flag it; terminal jobs are retained
//! [`JOB_RETENTION`] then evicted by the same background interval task that
//! flushes debounced checkpoints (`server.rs::run`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};

use photonic_core::history::CommandHistory;
use photonic_core::timeline::TimelineProject;
use photonic_core::Document;
use photonic_video::{EngineCmd, EngineFrame, EngineSession, ProxyMode, VideoEngine};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::server::AppState;

// ─── Engine bridge ───────────────────────────────────────────────────────────

/// Lazily-initialized engine slot held by `AppState` (10 §2's
/// `video_engine` field, lazy-headless variant — see module docs).
#[derive(Default)]
pub struct VideoEngineHandle {
    cell: OnceLock<Option<EngineBridge>>,
}

impl VideoEngineHandle {
    pub fn new() -> Self {
        Self::default()
    }

    /// The bridge, creating engine + session on first use. `None` = no GPU
    /// adapter available (the adapter-skip convention).
    pub fn bridge(&self) -> Option<&EngineBridge> {
        self.cell.get_or_init(EngineBridge::create).as_ref()
    }
}

/// One `EngineSession` + the shadow document pair it snapshots from, plus
/// the session-scoped state the MCP layer owns (proxy mode, transport lock).
pub struct EngineBridge {
    engine: VideoEngine,
    session: EngineSession,
    shadow_doc: Arc<StdMutex<Document>>,
    shadow_history: Arc<StdMutex<CommandHistory>>,
    /// Serializes seek-then-wait transactions (`render_frame_at`, transport
    /// tools) so two concurrent calls can't interleave their target ticks.
    transport: tokio::sync::Mutex<()>,
    /// Last mode set via `set_proxy_mode` — `EngineStatus` doesn't echo it,
    /// so `render_frame_at` restores from here after a per-call override.
    proxy_mode: StdMutex<ProxyMode>,
}

impl EngineBridge {
    fn create() -> Option<EngineBridge> {
        let engine = VideoEngine::headless()?;
        let shadow_doc = Arc::new(StdMutex::new(Document::new(
            "video-engine-shadow",
            1.0,
            1.0,
        )));
        let shadow_history = Arc::new(StdMutex::new(CommandHistory::new(1)));
        let session = engine.open_session(Arc::clone(&shadow_doc), Arc::clone(&shadow_history));
        Some(EngineBridge {
            engine,
            session,
            shadow_doc,
            shadow_history,
            transport: tokio::sync::Mutex::new(()),
            proxy_mode: StdMutex::new(ProxyMode::default()),
        })
    }

    pub fn engine(&self) -> &VideoEngine {
        &self.engine
    }

    pub fn session(&self) -> &EngineSession {
        &self.session
    }

    /// Serialize a seek-then-wait transaction (held across the whole tool
    /// call for `render_frame_at` and the transport tools).
    pub async fn lock_transport(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.transport.lock().await
    }

    pub fn proxy_mode(&self) -> ProxyMode {
        *self.proxy_mode.lock().expect("proxy_mode poisoned")
    }

    pub fn set_proxy_mode(&self, mode: ProxyMode) {
        *self.proxy_mode.lock().expect("proxy_mode poisoned") = mode;
        self.session.send(EngineCmd::SetProxyMode(mode));
    }

    /// Copy the real document's timeline into the shadow document the engine
    /// snapshots from (module docs). No-op when nothing changed, so repeated
    /// readonly calls don't force the engine to re-snapshot/re-present.
    pub async fn sync(&self, state: &AppState) {
        let timeline = state.document.lock().await.timeline.clone();
        self.sync_timeline(timeline);
    }

    /// Non-async form for callers that already hold/cloned the timeline.
    pub fn sync_timeline(&self, timeline: Option<TimelineProject>) {
        let mut doc = self.shadow_doc.lock().expect("shadow doc poisoned");
        if doc.timeline == timeline {
            return;
        }
        doc.timeline = timeline;
        drop(doc);
        // `reset()` is the revision-bump primitive: the shadow history never
        // carries commands (the real history owns undo), it exists purely so
        // the engine's `changes_since` poll sees the timeline move.
        self.shadow_history
            .lock()
            .expect("shadow history poisoned")
            .reset();
    }

    /// The shadow revision the engine must reach to have snapshotted the
    /// latest `sync_timeline` result.
    pub fn shadow_revision(&self) -> u64 {
        self.shadow_history
            .lock()
            .expect("shadow history poisoned")
            .revision()
    }

    /// Wait until the engine's published `doc_revision` catches up with the
    /// shadow history (i.e. all future presents use the synced timeline).
    pub async fn wait_engine_synced(&self, timeout: Duration) -> bool {
        let want = self.shadow_revision();
        let deadline = Instant::now() + timeout;
        loop {
            if self.session.status().doc_revision >= want {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }

    /// Wait for a frame **newer than** `prev` (pointer-compared) matching
    /// `pred`. Pair with: sync → `wait_engine_synced` → capture `prev` →
    /// send `Seek` → this — so a stale pre-seek frame can never satisfy it.
    pub async fn wait_fresh_frame(
        &self,
        prev: Option<Arc<EngineFrame>>,
        timeout: Duration,
        pred: impl Fn(&EngineFrame) -> bool,
    ) -> Option<Arc<EngineFrame>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(frame) = self.session.latest_frame() {
                let fresh = match &prev {
                    Some(p) => !Arc::ptr_eq(p, &frame),
                    None => true,
                };
                if fresh && pred(&frame) {
                    return Some(frame);
                }
            }
            if Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }
}

// ─── Job registry (10 §6) ────────────────────────────────────────────────────

pub type JobId = Uuid;

/// Terminal-state retention before GC eviction (10 §6: "retained 10 minutes
/// ... comfortably exceeds the debounced-checkpoint window").
pub const JOB_RETENTION: Duration = Duration::from_secs(600);

/// 10 §6's status enum. `Failed` carries the §8-style structured code so
/// agents can branch on error kind when polling.
#[derive(Clone, Debug)]
pub enum JobStatus {
    Queued,
    Running { progress: f32, message: String },
    Done { result: Value },
    Failed { error_code: String, message: String },
    Cancelled,
}

impl JobStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            JobStatus::Done { .. } | JobStatus::Failed { .. } | JobStatus::Cancelled
        )
    }

    pub fn to_json(&self) -> Value {
        match self {
            JobStatus::Queued => json!({ "state": "queued" }),
            JobStatus::Running { progress, message } => {
                json!({ "state": "running", "progress": progress, "message": message })
            }
            JobStatus::Done { result } => json!({ "state": "done", "result": result }),
            JobStatus::Failed {
                error_code,
                message,
            } => json!({ "state": "failed", "error_code": error_code, "message": message }),
            JobStatus::Cancelled => json!({ "state": "cancelled" }),
        }
    }
}

pub struct JobHandle {
    pub kind: String,
    pub status: JobStatus,
    /// Cooperative cancel flag — workers poll it between units of work
    /// (`export_frames` checks between frames, 02 §7).
    pub cancel: Arc<AtomicBool>,
    pub created: Instant,
    /// Set when `status` turned terminal; drives GC retention.
    pub finished: Option<Instant>,
}

#[derive(Default)]
pub struct JobRegistry {
    jobs: HashMap<JobId, JobHandle>,
}

impl JobRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a queued job; returns its id and the shared cancel flag the
    /// worker should poll.
    pub fn start(&mut self, kind: impl Into<String>) -> (JobId, Arc<AtomicBool>) {
        let id = Uuid::new_v4();
        let cancel = Arc::new(AtomicBool::new(false));
        self.jobs.insert(
            id,
            JobHandle {
                kind: kind.into(),
                status: JobStatus::Queued,
                cancel: Arc::clone(&cancel),
                created: Instant::now(),
                finished: None,
            },
        );
        (id, cancel)
    }

    /// Update a job's status; stamps `finished` on the terminal transition.
    /// Silently ignores unknown ids (a worker may outlive a GC'd entry).
    pub fn set_status(&mut self, id: JobId, status: JobStatus) {
        if let Some(job) = self.jobs.get_mut(&id) {
            if status.is_terminal() && job.finished.is_none() {
                job.finished = Some(Instant::now());
            }
            job.status = status;
        }
    }

    pub fn get(&self, id: JobId) -> Option<&JobHandle> {
        self.jobs.get(&id)
    }

    /// Set the cancel flag. Returns `None` for unknown ids; `Some(true)` if
    /// the job was still live (worker will observe the flag), `Some(false)`
    /// if it had already reached a terminal state.
    pub fn request_cancel(&mut self, id: JobId) -> Option<bool> {
        let job = self.jobs.get_mut(&id)?;
        job.cancel.store(true, Ordering::Relaxed);
        Some(!job.status.is_terminal())
    }

    /// Evict terminal jobs older than `retention` (10 §6 lifetime/GC).
    pub fn gc(&mut self, retention: Duration) {
        self.jobs.retain(|_, job| match job.finished {
            Some(at) => at.elapsed() < retention,
            None => true,
        });
    }

    pub fn status_json(&self, id: JobId) -> Option<Value> {
        self.get(id).map(|job| {
            json!({
                "job_id": id,
                "kind": job.kind,
                "status": job.status.to_json(),
            })
        })
    }
}

/// Convenience for workers: update status through the `AppState` handle
/// without each call site re-spelling the lock/poison dance.
pub fn set_job_status(jobs: &Arc<StdMutex<JobRegistry>>, id: JobId, status: JobStatus) {
    if let Ok(mut reg) = jobs.lock() {
        reg.set_status(id, status);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_registry_lifecycle_and_gc() {
        let mut reg = JobRegistry::new();
        let (id, cancel) = reg.start("fake");
        assert!(matches!(reg.get(id).unwrap().status, JobStatus::Queued));

        reg.set_status(
            id,
            JobStatus::Running {
                progress: 0.5,
                message: "halfway".into(),
            },
        );
        assert!(reg.get(id).unwrap().finished.is_none());

        // Cancellation of a live job flags the worker's AtomicBool.
        assert_eq!(reg.request_cancel(id), Some(true));
        assert!(cancel.load(Ordering::Relaxed));

        reg.set_status(id, JobStatus::Cancelled);
        assert!(reg.get(id).unwrap().finished.is_some());
        // Cancelling a terminal job reports "already finished".
        assert_eq!(reg.request_cancel(id), Some(false));

        // GC: terminal + retention elapsed ⇒ evicted (zero retention here).
        reg.gc(Duration::from_secs(0));
        assert!(reg.get(id).is_none(), "terminal job must be GC'd");
        assert_eq!(reg.request_cancel(id), None, "evicted ⇒ JobNotFound");

        // Live jobs are never GC'd regardless of age.
        let (live, _) = reg.start("fake");
        reg.gc(Duration::from_secs(0));
        assert!(reg.get(live).is_some(), "non-terminal job must survive GC");
    }

    #[test]
    fn job_status_json_shapes() {
        let s = JobStatus::Failed {
            error_code: "AssetOffline".into(),
            message: "gone".into(),
        }
        .to_json();
        assert_eq!(s["state"], "failed");
        assert_eq!(s["error_code"], "AssetOffline");
        let s = JobStatus::Done {
            result: json!({"output_path": "/tmp/x.mp4"}),
        }
        .to_json();
        assert_eq!(s["state"], "done");
        assert_eq!(s["result"]["output_path"], "/tmp/x.mp4");
    }
}
