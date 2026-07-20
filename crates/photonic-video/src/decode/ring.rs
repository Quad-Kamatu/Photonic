//! Per-source decoded-frame ring, keyed by presentation tick (02 §3).
//!
//! Holds ±N frames around the playhead (default 24 forward / 6 back at preview
//! quality). A reader worker [`push`](FrameRing::push)es decoded frames; the
//! engine thread [`frame_covering`](FrameRing::frame_covering)s the playhead.
//! [`SharedRing`] wraps the ring in a `Mutex` + `Condvar` so the two threads
//! hand frames off without the consumer busy-waiting.

use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use photonic_core::timeline::Tick;

use super::DecodedFrame;

/// Default forward window at preview quality (02 §3).
/// 24 frames ≈ 0.8 s @ 30 fps — smoother scrub-ahead on Linux/Windows.
pub const DEFAULT_FWD: usize = 24;
/// Default backward window at preview quality (02 §3).
pub const DEFAULT_BACK: usize = 6;

/// A pts-keyed decoded-frame ring for one decode source. Not itself
/// thread-safe; share via [`SharedRing`].
pub struct FrameRing {
    fwd_cap: usize,
    back_cap: usize,
    playhead: Tick,
    frames: BTreeMap<Tick, Arc<DecodedFrame>>,
}

impl FrameRing {
    pub fn new(fwd_cap: usize, back_cap: usize) -> Self {
        FrameRing {
            fwd_cap,
            back_cap,
            playhead: Tick::ZERO,
            frames: BTreeMap::new(),
        }
    }

    /// Preview-quality ring with the 16-fwd / 4-back defaults.
    pub fn preview() -> Self {
        Self::new(DEFAULT_FWD, DEFAULT_BACK)
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn playhead(&self) -> Tick {
        self.playhead
    }

    /// Lowest resident pts (oldest decoded frame), if any.
    pub fn oldest(&self) -> Option<Tick> {
        self.frames.keys().next().copied()
    }

    /// Highest resident pts (decode frontier), if any.
    pub fn newest(&self) -> Option<Tick> {
        self.frames.keys().next_back().copied()
    }

    /// Insert a decoded frame (idempotent on pts) and prune to the window.
    pub fn push(&mut self, frame: DecodedFrame) {
        self.frames.insert(frame.pts, Arc::new(frame));
        self.prune();
    }

    /// The frame whose presentation interval `[pts, next_pts)` covers `t` — the
    /// greatest-pts frame with `pts <= t` (02 §4 present rule).
    pub fn frame_covering(&self, t: Tick) -> Option<Arc<DecodedFrame>> {
        self.frames
            .range(..=t)
            .next_back()
            .map(|(_, f)| Arc::clone(f))
    }

    /// Exact-pts lookup.
    pub fn get(&self, pts: Tick) -> Option<Arc<DecodedFrame>> {
        self.frames.get(&pts).map(Arc::clone)
    }

    pub fn contains(&self, pts: Tick) -> bool {
        self.frames.contains_key(&pts)
    }

    /// Move the playhead and re-prune (frames now outside the window age out).
    pub fn set_playhead(&mut self, t: Tick) {
        self.playhead = t;
        self.prune();
    }

    /// Drop everything (used on a discontinuous seek to a new GOP).
    pub fn clear(&mut self) {
        self.frames.clear();
    }

    /// Keep at most `back_cap` frames before the playhead and `fwd_cap` at or
    /// after it; evict by ring position (oldest on each side first).
    fn prune(&mut self) {
        // Backward side: frames strictly before the playhead.
        let back: Vec<Tick> = self
            .frames
            .range(..self.playhead)
            .map(|(t, _)| *t)
            .collect();
        if back.len() > self.back_cap {
            for t in &back[..back.len() - self.back_cap] {
                self.frames.remove(t);
            }
        }
        // Forward side: frames at or after the playhead.
        let fwd: Vec<Tick> = self
            .frames
            .range(self.playhead..)
            .map(|(t, _)| *t)
            .collect();
        if fwd.len() > self.fwd_cap {
            for t in &fwd[self.fwd_cap..] {
                self.frames.remove(t);
            }
        }
    }
}

/// Thread-safe handle to a [`FrameRing`]: a reader worker fills, the engine
/// thread drains, and [`wait_for_frame`](SharedRing::wait_for_frame) blocks the
/// consumer until a covering frame arrives (or a deadline elapses) instead of
/// spinning.
#[derive(Clone)]
pub struct SharedRing {
    inner: Arc<(Mutex<FrameRing>, Condvar)>,
}

impl SharedRing {
    pub fn new(ring: FrameRing) -> Self {
        SharedRing {
            inner: Arc::new((Mutex::new(ring), Condvar::new())),
        }
    }

    pub fn preview() -> Self {
        Self::new(FrameRing::preview())
    }

    /// Reader worker: push a decoded frame and wake any waiting consumer.
    pub fn push(&self, frame: DecodedFrame) {
        let (lock, cvar) = &*self.inner;
        // Poison-tolerant: a panic in one worker must not wedge the ring for the
        // engine present thread. The `FrameRing` has no cross-field invariant
        // that a mid-operation panic could leave broken, so recover the guard.
        lock.lock().unwrap_or_else(|e| e.into_inner()).push(frame);
        cvar.notify_all();
    }

    /// Consumer: the frame covering `t` if resident, without blocking.
    pub fn frame_covering(&self, t: Tick) -> Option<Arc<DecodedFrame>> {
        let (lock, _) = &*self.inner;
        lock.lock()
            .unwrap_or_else(|e| e.into_inner())
            .frame_covering(t)
    }

    /// Exact-pts lookup without blocking.
    pub fn get(&self, pts: Tick) -> Option<Arc<DecodedFrame>> {
        let (lock, _) = &*self.inner;
        lock.lock().unwrap_or_else(|e| e.into_inner()).get(pts)
    }

    /// Consumer: block until a frame with `pts >= t` is resident (so the frame
    /// covering `t` is known-final), or `timeout` elapses. Returns the covering
    /// frame, or `None` on timeout.
    pub fn wait_for_frame(&self, t: Tick, timeout: Duration) -> Option<Arc<DecodedFrame>> {
        let (lock, cvar) = &*self.inner;
        let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        let deadline = std::time::Instant::now() + timeout;
        loop {
            // A frame at/after t means the covering frame won't change with more decode.
            if guard.frames.range(t..).next().is_some() {
                return guard.frame_covering(t);
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                return guard.frame_covering(t);
            }
            // Poison-tolerant: recover the guard out of a `PoisonError` returned
            // by `wait_timeout` (a producer worker panicked) so the present
            // thread keeps servicing frames rather than panicking in turn.
            let (g, res) = match cvar.wait_timeout(guard, deadline - now) {
                Ok(pair) => pair,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard = g;
            if res.timed_out() {
                return guard.frame_covering(t);
            }
        }
    }

    /// Lowest resident pts (oldest decoded frame), if any.
    pub fn oldest(&self) -> Option<Tick> {
        let (lock, _) = &*self.inner;
        lock.lock().unwrap_or_else(|e| e.into_inner()).oldest()
    }

    /// Highest resident pts (decode frontier), if any.
    pub fn newest(&self) -> Option<Tick> {
        let (lock, _) = &*self.inner;
        lock.lock().unwrap_or_else(|e| e.into_inner()).newest()
    }

    pub fn set_playhead(&self, t: Tick) {
        let (lock, _) = &*self.inner;
        lock.lock()
            .unwrap_or_else(|e| e.into_inner())
            .set_playhead(t);
    }

    pub fn clear(&self) {
        let (lock, _) = &*self.inner;
        lock.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    pub fn len(&self) -> usize {
        let (lock, _) = &*self.inner;
        lock.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::DecodedPlanes;

    fn frame(pts: Tick) -> DecodedFrame {
        DecodedFrame {
            pts,
            planes: DecodedPlanes::Yuv420 {
                width: 2,
                height: 2,
                y: vec![0; 4],
                cb: vec![0],
                cr: vec![0],
            },
        }
    }

    #[test]
    fn frame_covering_picks_greatest_pts_at_or_before() {
        let mut ring = FrameRing::new(16, 4);
        ring.push(frame(Tick(0)));
        ring.push(frame(Tick(100)));
        ring.push(frame(Tick(200)));
        assert_eq!(ring.frame_covering(Tick(150)).unwrap().pts, Tick(100));
        assert_eq!(ring.frame_covering(Tick(200)).unwrap().pts, Tick(200));
        assert!(ring.frame_covering(Tick(-1)).is_none());
    }

    #[test]
    fn prune_keeps_window_around_playhead() {
        // Playhead is set first (as in real fill: the ring prunes against the
        // current playhead on every push), then frames arrive around it.
        let mut ring = FrameRing::new(2, 1); // 2 fwd, 1 back
        ring.set_playhead(Tick(500));
        for t in [300, 400, 500, 600, 700] {
            ring.push(frame(Tick(t)));
        }
        // back: only 1 frame < 500 kept (the newest before playhead, 400).
        assert!(ring.contains(Tick(400)));
        assert!(!ring.contains(Tick(300)));
        // fwd: the 2 frames closest to (>=) the playhead kept (500, 600); the
        // farthest-ahead (700) is dropped once the forward window is full.
        assert!(ring.contains(Tick(500)));
        assert!(ring.contains(Tick(600)));
        assert!(!ring.contains(Tick(700)));
    }

    #[test]
    fn shared_ring_hands_off_across_threads() {
        let ring = SharedRing::new(FrameRing::new(16, 4));
        let producer = ring.clone();
        let h = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            producer.push(frame(Tick(1000)));
        });
        // Consumer blocks until the producer pushes a frame at/after 1000.
        let got = ring.wait_for_frame(Tick(1000), Duration::from_secs(2));
        h.join().unwrap();
        assert_eq!(got.unwrap().pts, Tick(1000));
    }

    #[test]
    fn wait_for_frame_times_out_without_panicking() {
        let ring = SharedRing::new(FrameRing::new(16, 4));
        let got = ring.wait_for_frame(Tick(1000), Duration::from_millis(10));
        assert!(got.is_none());
    }

    #[test]
    fn poisoned_ring_stays_readable() {
        // Simulate a decode worker panicking while holding the ring lock: the
        // mutex is now poisoned. The engine present thread must still be able to
        // read frames instead of panicking on a poisoned `lock().unwrap()`.
        let ring = SharedRing::new(FrameRing::new(16, 4));
        ring.push(frame(Tick(500)));

        let poisoner = ring.clone();
        let h = std::thread::spawn(move || {
            // Poison the mutex by panicking while the guard is held.
            poisoner.push(frame(Tick(600)));
            let (lock, _) = &*poisoner.inner;
            let _guard = lock.lock().unwrap();
            panic!("decode worker died mid-operation");
        });
        assert!(h.join().is_err());

        // All read/write paths must recover the poisoned guard, not panic.
        assert_eq!(ring.frame_covering(Tick(550)).unwrap().pts, Tick(500));
        assert_eq!(ring.get(Tick(600)).unwrap().pts, Tick(600));
        assert_eq!(ring.oldest(), Some(Tick(500)));
        assert_eq!(ring.newest(), Some(Tick(600)));
        assert_eq!(ring.len(), 2);
        ring.push(frame(Tick(700)));
        assert_eq!(ring.newest(), Some(Tick(700)));
        // A blocking wait must also read through poison (frame already resident).
        // The frame covering t=500 is the 500 frame (greatest pts <= 500).
        let got = ring.wait_for_frame(Tick(500), Duration::from_millis(10));
        assert_eq!(got.unwrap().pts, Tick(500));
        ring.set_playhead(Tick(600));
        ring.clear();
        assert!(ring.is_empty());
    }
}
