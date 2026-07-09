//! Audio DSP units (09 §6) — P8 story.
//!
//! Every unit here is a [`DspUnit`]: it consumes one interleaved-stereo block
//! of already block-resolved params (already evaluated from `AnimProps` and
//! already de-zipper-smoothed by whatever calls it, 09 §5) — "no knowledge
//! of `Tick`/`AnimProps` inside the DSP layer itself" (09 §6 intro). Nothing
//! in this module touches `photonic_video::audio::mixer`, whose `fx_chain`
//! is still an inert pass-through (see that module's doc): the seam a future
//! wiring story needs is each unit's `set_params`/`params()` pair plus its
//! `*Params::from_effect_params(&EffectParams)` conversion (09 §2's
//! `AudioFxUnit.params: AnimProps<EffectParams>`, evaluated once per block
//! the same way `mixer::eval_track_audio_params` already evaluates
//! `TrackAudioParams` — see each submodule's `from_effect_params` doc for the
//! exact `PropPath` strings, which match `photonic_core::timeline::
//! prop_registry`'s `AUDIOFX_*` tables).
//!
//! Submodules: [`biquad`] (RBJ cookbook coefficient design, shared by [`eq`]
//! and [`loudness`]'s K-weighting filter), [`envelope`] (the §6.2 detector
//! shared by [`compressor`] and [`gate`]), [`eq`], [`compressor`], [`gate`],
//! [`limiter`], [`loudness`] (§6.6 EBU R128/BS.1770-4 — an offline analysis
//! pass over a full rendered buffer, not a per-block [`DspUnit`]; see that
//! module's doc for why).
//!
//! The four processing-unit types ([`eq::Eq`], [`compressor::Compressor`],
//! [`gate::Gate`], [`limiter::Limiter`]) are deliberately **not** re-exported
//! at this module's top level: `Eq` mirrors `AudioFxKind::Eq`'s name exactly
//! (for obvious kind<->unit discoverability), which would shadow
//! `std::cmp::Eq` if glob-imported — call sites use the explicit
//! `dsp::eq::Eq` path instead. The `*Params`/band types (no such collision)
//! are re-exported normally.

pub mod biquad;
pub mod compressor;
pub mod envelope;
pub mod eq;
pub mod gate;
pub mod limiter;
pub mod loudness;

pub use compressor::CompressorParams;
pub use eq::{EqParams, PeakBand, ShelfBand};
pub use gate::GateParams;
pub use limiter::LimiterParams;

/// A single DSP processing unit (09 §6): one already-evaluated params set,
/// applied to one block. Every concrete unit ([`eq::Eq`],
/// [`compressor::Compressor`], [`gate::Gate`], [`limiter::Limiter`]) also
/// exposes an inherent `set_params`/`params` pair (not part of this trait,
/// since each takes a different concrete `*Params` type) — call `set_params`
/// once per block with that block's resolved values, then `process`.
pub trait DspUnit: Send {
    /// Process `block` **in place**: interleaved stereo
    /// (`photonic_video::audio::CHANNELS`-interleaved), any positive frame
    /// count divisible by `CHANNELS`. Production callers SHOULD pass exactly
    /// one mixer block (`photonic_video::audio::BLOCK_FRAMES` frames) per 09
    /// §5's "coefficients resolved once per block" budget — this trait
    /// doesn't enforce that length so standalone unit tests can process a
    /// whole tone burst/sweep in one call.
    ///
    /// `sample_rate` is the mixer's configured rate (`Mixer::sample_rate`) —
    /// passed per call rather than fixed at construction so a unit can be
    /// exercised at any rate in isolation.
    ///
    /// `sidechain`, when `Some`, is another buffer of the exact same length
    /// as `block` — the ducking source's post-fx-pre-fader signal (09 §6.3).
    /// Only [`compressor::Compressor`] consults it; every other unit ignores
    /// it.
    fn process(&mut self, sample_rate: u32, block: &mut [f32], sidechain: Option<&[f32]>);
}

/// Per-sample one-pole coefficient for a time constant `tau_ms` at `fs` Hz —
/// the exact form 09 §6.2 gives for the envelope follower's attack/release,
/// reused for every other one-pole smoothing need in this module. Mirrors
/// `mixer.rs`'s identically-behaving private helper; duplicated rather than
/// imported so `dsp/` has zero dependency on `mixer.rs`, per this story's
/// scope boundary (mixer.rs is not to be edited or leaned on).
fn one_pole_coeff(tau_ms: f64, fs: f64) -> f64 {
    1.0 - (-1.0 / (tau_ms * 0.001 * fs)).exp()
}

#[inline]
fn db_to_linear(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

#[inline]
fn lin_to_db(lin: f64) -> f64 {
    20.0 * lin.max(1e-12).log10()
}

#[cfg(test)]
mod determinism_tests {
    //! "process twice = bit-identical" (09 §11's determinism test hook),
    //! covering every [`super::DspUnit`] impl against a fixed synthetic
    //! signal. A deterministic (non-crate, seeded) LCG stands in for noise —
    //! no `rand` dependency needed for a fixed, reproducible bit pattern.

    use super::compressor::{Compressor, CompressorParams};
    use super::eq::{Eq, EqParams, PeakBand};
    use super::gate::{Gate, GateParams};
    use super::limiter::{Limiter, LimiterParams};
    use super::DspUnit;
    use crate::audio::CHANNELS;

    const FS: u32 = 48_000;

    /// A fixed pseudo-random interleaved-stereo signal (deterministic LCG,
    /// not `rand`) mixed with a tone, so every unit sees varied, non-trivial
    /// input.
    fn fixed_signal(frames: usize) -> Vec<f32> {
        let mut out = vec![0f32; frames * CHANNELS];
        let mut state: u32 = 0x2545F491;
        for f in 0..frames {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (state as f32 / u32::MAX as f32) * 2.0 - 1.0;
            let t = f as f64 / FS as f64;
            let tone = (2.0 * std::f64::consts::PI * 440.0 * t).sin() as f32;
            let s = 0.5 * tone + 0.3 * noise;
            out[f * CHANNELS] = s;
            out[f * CHANNELS + 1] = s * 0.8;
        }
        out
    }

    /// Process `buf` forward in the given frame-count chunks (mimicking
    /// successive mixer blocks, to exercise cross-call state — envelope
    /// followers, limiter delay lines — exactly as the real mixer-
    /// worker/export loop would), then whatever's left in one final call.
    fn process_in_chunks<U: DspUnit>(unit: &mut U, buf: &mut [f32], chunk_frames_seq: &[usize]) {
        let mut offset = 0;
        for &chunk in chunk_frames_seq {
            let end = (offset + chunk * CHANNELS).min(buf.len());
            unit.process(FS, &mut buf[offset..end], None);
            offset = end;
            if offset >= buf.len() {
                return;
            }
        }
        let len = buf.len();
        unit.process(FS, &mut buf[offset..len], None);
    }

    fn run_twice<U: DspUnit, F: Fn() -> U>(make: F) -> (Vec<f32>, Vec<f32>) {
        let input = fixed_signal(4000);
        let chunks = [512, 512, 1024, 512, 512, 400, 240];

        let mut out_a = input.clone();
        let mut a = make();
        process_in_chunks(&mut a, &mut out_a, &chunks);

        let mut out_b = input;
        let mut b = make();
        process_in_chunks(&mut b, &mut out_b, &chunks);

        (out_a, out_b)
    }

    #[test]
    fn eq_is_deterministic() {
        let (a, b) = run_twice(|| {
            let mut u = Eq::new();
            let p = EqParams {
                band1: PeakBand {
                    freq_hz: 800.0,
                    gain_db: 6.0,
                    q: 1.2,
                },
                ..EqParams::default()
            };
            u.set_params(p);
            u
        });
        assert_eq!(a, b);
    }

    #[test]
    fn compressor_is_deterministic() {
        let (a, b) = run_twice(|| {
            let mut u = Compressor::new();
            u.set_params(CompressorParams {
                threshold_db: -18.0,
                ratio: 3.0,
                attack_ms: 5.0,
                release_ms: 80.0,
                makeup_db: 2.0,
            });
            u
        });
        assert_eq!(a, b);
    }

    #[test]
    fn gate_is_deterministic() {
        let (a, b) = run_twice(|| {
            let mut u = Gate::new();
            u.set_params(GateParams {
                threshold_db: -30.0,
                attack_ms: 1.0,
                hold_ms: 15.0,
                release_ms: 60.0,
                range_db: 40.0,
            });
            u
        });
        assert_eq!(a, b);
    }

    #[test]
    fn limiter_is_deterministic() {
        let (a, b) = run_twice(|| {
            let mut u = Limiter::new();
            u.set_params(LimiterParams {
                ceiling_db: -1.0,
                release_ms: 50.0,
            });
            u
        });
        assert_eq!(a, b);
    }
}
