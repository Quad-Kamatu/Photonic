//! Time representation for the timeline (01 §1).
//!
//! All timeline positions and durations are `i64` ticks at 705,600,000
//! ticks/second — the "flick", the smallest unit that exactly divides every
//! supported frame rate (including 1001-denominator NTSC rates) and the common
//! audio rates. No `f32`/`f64` time appears anywhere in the data model; the UI
//! converts to seconds/frames only at the edge.

use serde::{Deserialize, Serialize};

/// Ticks per second — the "flick". Exactly divides 24/25/30/48/50/60/90/100/120
/// fps (incl. `×1000/1001` NTSC) and 44.1/48/88.2/96/192 kHz audio (01 §1).
pub const TICKS_PER_SECOND: i64 = 705_600_000;

/// A timeline position or duration in [`TICKS_PER_SECOND`] ticks.
///
/// Negative values are permitted only for deltas (e.g. a ripple shift); stored
/// positions and durations are non-negative, enforced by the edit ops.
#[derive(
    Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Tick(pub i64);

impl Tick {
    pub const ZERO: Tick = Tick(0);

    /// Ticks for a whole number of seconds.
    #[inline]
    pub const fn from_seconds(secs: i64) -> Tick {
        Tick(secs * TICKS_PER_SECOND)
    }

    /// Fractional seconds as an `f64` (UI-edge conversion only — never fed back
    /// into the data model).
    #[inline]
    pub fn as_seconds_f64(self) -> f64 {
        self.0 as f64 / TICKS_PER_SECOND as f64
    }

    /// Saturating add, guarding against overflow on pathological inputs.
    #[inline]
    pub fn saturating_add(self, other: Tick) -> Tick {
        Tick(self.0.saturating_add(other.0))
    }

    /// Saturating subtract.
    #[inline]
    pub fn saturating_sub(self, other: Tick) -> Tick {
        Tick(self.0.saturating_sub(other.0))
    }
}

impl std::ops::Add for Tick {
    type Output = Tick;
    #[inline]
    fn add(self, rhs: Tick) -> Tick {
        Tick(self.0 + rhs.0)
    }
}

impl std::ops::Sub for Tick {
    type Output = Tick;
    #[inline]
    fn sub(self, rhs: Tick) -> Tick {
        Tick(self.0 - rhs.0)
    }
}

/// A rational frame rate, e.g. `30000/1001` for 29.97 fps (01 §1).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FrameRate {
    pub num: u32,
    pub den: u32,
}

impl FrameRate {
    pub const FPS_24: FrameRate = FrameRate { num: 24, den: 1 };
    pub const FPS_25: FrameRate = FrameRate { num: 25, den: 1 };
    pub const FPS_30: FrameRate = FrameRate { num: 30, den: 1 };
    pub const FPS_60: FrameRate = FrameRate { num: 60, den: 1 };
    /// 29.97 (NTSC).
    pub const FPS_29_97: FrameRate = FrameRate {
        num: 30000,
        den: 1001,
    };
    /// 23.976 (NTSC film).
    pub const FPS_23_976: FrameRate = FrameRate {
        num: 24000,
        den: 1001,
    };

    #[inline]
    pub const fn new(num: u32, den: u32) -> FrameRate {
        FrameRate { num, den }
    }

    /// Ticks per frame: `TICKS_PER_SECOND * den / num`, rounded to the nearest
    /// tick. For every supported rate this is exact (integral); for an exotic
    /// rate the rounding introduces sub-tick error (< 1 µs over 10 min), which
    /// [`is_exact`](Self::is_exact) flags for a UI warning.
    #[inline]
    pub fn ticks_per_frame(&self) -> Tick {
        debug_assert!(self.num > 0, "frame rate numerator must be > 0");
        let num = self.num.max(1) as i64;
        let den = self.den as i64;
        // Round-to-nearest: (a + num/2) / num.
        Tick((TICKS_PER_SECOND * den + num / 2) / num)
    }

    /// Frame index containing tick `t` (floor toward negative infinity so that
    /// exact frame boundaries map to their own frame).
    #[inline]
    pub fn frame_at(&self, t: Tick) -> i64 {
        let tpf = self.ticks_per_frame().0.max(1);
        t.0.div_euclid(tpf)
    }

    /// Snap `t` down to the frame boundary at or before it.
    #[inline]
    pub fn snap(&self, t: Tick) -> Tick {
        let tpf = self.ticks_per_frame().0.max(1);
        Tick(self.frame_at(t) * tpf)
    }

    /// Tick position of the start of frame `frame`.
    #[inline]
    pub fn frame_start(&self, frame: i64) -> Tick {
        Tick(frame * self.ticks_per_frame().0)
    }

    /// True when [`ticks_per_frame`](Self::ticks_per_frame) divides exactly (no
    /// rounding). All the SPEC's supported rates are exact; only exotic rates
    /// (e.g. 23.9-something custom) are not, and the UI surfaces that.
    #[inline]
    pub fn is_exact(&self) -> bool {
        let num = self.num.max(1) as i64;
        let den = self.den as i64;
        (TICKS_PER_SECOND * den) % num == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_per_frame_is_exact_for_supported_rates() {
        for fr in [
            FrameRate::FPS_24,
            FrameRate::FPS_25,
            FrameRate::FPS_30,
            FrameRate::FPS_60,
            FrameRate::FPS_29_97,
            FrameRate::FPS_23_976,
            FrameRate::new(48, 1),
            FrameRate::new(50, 1),
            FrameRate::new(120, 1),
        ] {
            assert!(fr.is_exact(), "{fr:?} should be exact");
            // Exactness means num * ticks_per_frame == TICKS_PER_SECOND * den.
            assert_eq!(
                fr.ticks_per_frame().0 * fr.num as i64,
                TICKS_PER_SECOND * fr.den as i64,
                "{fr:?} ticks_per_frame not exact"
            );
        }
    }

    #[test]
    fn frame_at_and_snap_are_consistent() {
        let fr = FrameRate::FPS_30;
        let tpf = fr.ticks_per_frame().0;
        // A tick one below frame 5's boundary is still in frame 4.
        assert_eq!(fr.frame_at(Tick(5 * tpf - 1)), 4);
        assert_eq!(fr.frame_at(Tick(5 * tpf)), 5);
        // Snap lands exactly on the boundary.
        assert_eq!(fr.snap(Tick(5 * tpf + 7)), Tick(5 * tpf));
        assert_eq!(fr.snap(Tick(5 * tpf)), Tick(5 * tpf));
    }

    #[test]
    fn exotic_rate_is_flagged_inexact_but_rounds() {
        // 11 fps does not divide TICKS_PER_SECOND (the flick has no factor of 11).
        let fr = FrameRate::new(11, 1);
        assert!(!fr.is_exact());
        // Still produces a sensible nearest-tick value.
        assert_eq!(fr.ticks_per_frame().0, (TICKS_PER_SECOND + 5) / 11);
    }

    #[test]
    fn tick_arithmetic() {
        assert_eq!(Tick::from_seconds(2), Tick(2 * TICKS_PER_SECOND));
        assert_eq!(Tick(10) + Tick(5), Tick(15));
        assert_eq!(Tick(10) - Tick(5), Tick(5));
        assert_eq!(Tick::from_seconds(1).as_seconds_f64(), 1.0);
    }
}
