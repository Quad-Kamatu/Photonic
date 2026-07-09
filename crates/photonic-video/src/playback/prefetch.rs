//! Prefetch v1 (02 §3): keep the active clip's decode ring pumped ahead of the
//! playhead along the play direction.
//!
//! **Cut-ahead seam (02 §3 / §8):** v1 only feeds the ring of sources that are
//! *currently* on screen (the engine derives them from the compiled frame's
//! `DecodeVideo` ops). The ≥ 500 ms cut-ahead warmup — scanning the timeline
//! for the *next* clip's source and starting its decoder early — lands with
//! the decode-worker-pool story; the hooks it needs (`SharedRing` per source,
//! `DecodeSource::seek` restartability) are already in place.

use photonic_core::timeline::{FrameRate, Tick};

use crate::decode::scheduler::DecodeSource;

/// Frames decoded per pump call — small so one engine-loop iteration never
/// stalls on a whole ring fill (the loop runs every ~2 ms and tops the ring
/// up incrementally toward the ring's 16-forward capacity).
pub const PREFETCH_BATCH: usize = 4;

/// How far ahead of the playhead (in frames) the ring should already have a
/// frame before we stop pumping.
pub const PREFETCH_AHEAD_FRAMES: i64 = 8;

/// Top up `source`'s ring ahead of `playhead` (source-time ticks). Returns how
/// many frames were decoded (0 = ring already ahead / end of stream / no
/// active process).
pub fn pump_ahead(source: &mut DecodeSource, playhead: Tick, rate: FrameRate) -> usize {
    if !source.is_active() {
        // Nothing decoded from this source yet this session (first frame comes
        // from the evaluator's on-demand seek); nothing to pump sequentially.
        return 0;
    }
    let target = Tick(rate.snap(playhead).0 + PREFETCH_AHEAD_FRAMES * rate.ticks_per_frame().0);
    if source.ring().frame_covering(target).is_some() {
        return 0; // already pumped ahead
    }
    // Keep the ring's eviction window tracking the consumer.
    source.ring().set_playhead(playhead);
    source.pump(PREFETCH_BATCH).unwrap_or(0)
}
