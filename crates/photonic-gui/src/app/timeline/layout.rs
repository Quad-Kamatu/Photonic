//! Timeline zoom/scroll session state (video-editor-module `04-ui-mode-timeline.md`
//! §2.1). Types + tick↔pixel mapping only — no drawing here (that's `ruler.rs`,
//! `tracks.rs`, `clips.rs`, filled in by the P2 wave).

use photonic_core::timeline::{Tick, TICKS_PER_SECOND};

/// Default zoom: 100 screen px per second of timeline.
const DEFAULT_PIXELS_PER_SECOND: f64 = 100.0;

/// Timeline panel zoom/scroll — GUI session state (04 §6), not document state.
/// `pixels_per_tick` is the one deliberate `f64` time value in the video-editor
/// surface: 01 §1 bans `f32`/`f64` time in the *data model*, but this struct is
/// GUI-only and converts at the edge, per that rule's own carve-out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimelineView {
    /// Zoom: screen pixels per tick. Clamped by callers to a sane min/max.
    pub pixels_per_tick: f64,
    /// Leftmost visible tick.
    pub scroll_ticks: Tick,
    /// Vertical scroll across track rows, in pixels.
    pub track_scroll_px: f32,
}

impl Default for TimelineView {
    fn default() -> Self {
        Self {
            pixels_per_tick: DEFAULT_PIXELS_PER_SECOND / TICKS_PER_SECOND as f64,
            scroll_ticks: Tick::ZERO,
            track_scroll_px: 0.0,
        }
    }
}

impl TimelineView {
    /// Map a tick to an x coordinate within a lane whose left edge is at
    /// `lane_left_px` (04 §2.1).
    pub fn tick_to_x(&self, t: Tick, lane_left_px: f32) -> f32 {
        lane_left_px + ((t.0 - self.scroll_ticks.0) as f64 * self.pixels_per_tick) as f32
    }

    /// Inverse of [`Self::tick_to_x`].
    pub fn x_to_tick(&self, x: f32, lane_left_px: f32) -> Tick {
        Tick(((x - lane_left_px) as f64 / self.pixels_per_tick) as i64 + self.scroll_ticks.0)
    }
}
