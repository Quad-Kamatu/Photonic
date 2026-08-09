//! Rolling-shutter AC light banding — detection and per-scanline correction.
//!
//! A rolling shutter reads rows sequentially, so under mains-driven lighting
//! (100/120 Hz — twice the 50/60 Hz supply, since light output tracks power)
//! each row integrates a different phase of the flicker. The result is
//! horizontal bands that drift vertically frame to frame. The forward model is
//! multiplicative in **linear** light:
//!
//! ```text
//! I(x, y) = S(x, y) * (1 + m * cos(2*PI*nu*y + phase))
//! ```
//!
//! with `nu` in cycles per row. A single global gain per frame — what every
//! open-source deflicker offers — cannot touch this, because the frame *mean*
//! barely moves: the bands redistribute light across rows rather than changing
//! the total.
//!
//! # Linear light here, unlike exposure stabilization
//!
//! [`super::deflicker`] deliberately corrects in the perceptual domain, because
//! there the goal is equal *visibility* of residual error. Banding is different:
//! it is a physical modulation of the light itself, exactly multiplicative in
//! linear light, so estimating and dividing in linear inverts the actual
//! degradation. The two modules disagree on purpose.
//!
//! # The hard part is not detecting bands, it is not detecting blinds
//!
//! Real scenes are full of horizontal structure — venetian blinds, siding,
//! railings, a horizon. Any per-row measurement sees those as "bands". Two
//! independent discriminators are used together, because each fails alone:
//!
//! 1. [`column_coherence`] — genuine banding multiplies *every column
//!    identically*, so the per-column complex amplitudes at the band frequency
//!    share a phase. Scene structure does not: it has edges, perspective and
//!    occlusion, so its phase varies across x. This works within a single frame,
//!    which is the only option when the bands happen to be stationary.
//! 2. [`detect_drifting`] — differencing consecutive log row profiles cancels
//!    *static* scene content exactly, leaving only what moved. Measured on real
//!    (deliberately worst-case, blind-filled) footage this detects a 0.5 %
//!    band, where the single-frame coherence test needs ~8 %.
//!
//! Below threshold the answer is **no correction at all**, never a weak one:
//! partially "correcting" real scene content is worse than leaving it alone.

use super::ops::Image;

/// Lowest spatial frequency considered a band, in cycles per frame height.
///
/// Below this, natural scenes are dominated by their own vertical gradient — a
/// bright sky over dark ground is two cycles of "band" in every column, and it
/// is perfectly coherent. Excluding the low end is what stops the detector
/// firing on ordinary composition.
pub const MIN_CYCLES: u32 = 4;

/// Highest spatial frequency considered. Beyond this the profile is dominated
/// by sensor noise and codec blocking rather than illumination.
pub const MAX_CYCLES: u32 = 80;

/// Window, in rows, of the spatial high-pass applied before analysis. Removes
/// the scene's slow vertical trend while leaving band-scale ripple intact.
const DETREND_ROWS: usize = 51;

/// Column-phase coherence above which a candidate is accepted as real banding.
pub const COHERENCE_THRESHOLD: f32 = 0.85;

/// Signal-to-median ratio above which a drifting candidate is accepted.
pub const DRIFT_SNR_THRESHOLD: f32 = 6.0;

/// Modulation depth below which a candidate is discarded regardless of how
/// coherent it looks.
///
/// This gate is load-bearing, not cosmetic. On a flat region every column
/// carries the *same* floating-point residue, so that residue is perfectly
/// phase-aligned and [`column_coherence`] reports 1.0 — a confident detection of
/// nothing. Requiring real modulation is what stops a blank wall or a clear sky
/// from being "corrected". 0.2 % is well under the visibility threshold.
pub const MIN_AMPLITUDE: f32 = 0.002;

/// A fitted band, in the frame's own row coordinates.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BandModel {
    /// Cycles per frame height.
    pub cycles: u32,
    /// Modulation depth (`m` in the model above).
    pub amplitude: f32,
    /// Phase at row 0, radians.
    pub phase: f32,
    /// Detector confidence in `0..=1`; below the gate this must not be applied.
    pub confidence: f32,
}

/// Per-row log-luminance profile of `img`, spatially high-passed.
///
/// Linear light (see module docs), column-averaged over pixels with meaningful
/// alpha, then detrended so the scene's vertical gradient does not swamp the
/// band-scale ripple.
pub fn row_profile(img: &Image) -> Vec<f32> {
    let (w, h) = (img.width as usize, img.height as usize);
    let mut prof = Vec::with_capacity(h);
    for y in 0..h {
        let mut sum = 0.0f64;
        let mut n = 0u32;
        for x in 0..w {
            let px = img.pixels[y * w + x];
            let a = px[3];
            if a <= 1e-6 {
                continue;
            }
            let inv = 1.0 / a;
            let lum = 0.2126 * px[0] * inv + 0.7152 * px[1] * inv + 0.0722 * px[2] * inv;
            sum += lum.max(1e-6) as f64;
            n += 1;
        }
        prof.push(if n == 0 {
            0.0
        } else {
            ((sum / n as f64).max(1e-6)).ln() as f32
        });
    }
    detrend(&prof, DETREND_ROWS)
}

/// Subtract a centred moving average — a spatial high-pass over rows.
fn detrend(v: &[f32], window: usize) -> Vec<f32> {
    let n = v.len();
    if n == 0 {
        return Vec::new();
    }
    let half = window / 2;
    (0..n)
        .map(|i| {
            let lo = i.saturating_sub(half);
            let hi = (i + half + 1).min(n);
            let mean = v[lo..hi].iter().sum::<f32>() / (hi - lo) as f32;
            v[i] - mean
        })
        .collect()
}

/// Project `profile` onto `cos`/`sin` at `cycles` per frame height, returning
/// `(amplitude, phase)`.
pub fn fit_at(profile: &[f32], cycles: u32) -> (f32, f32) {
    let n = profile.len();
    if n == 0 {
        return (0.0, 0.0);
    }
    let k = std::f32::consts::TAU * cycles as f32 / n as f32;
    let (mut re, mut im) = (0.0f32, 0.0f32);
    for (y, v) in profile.iter().enumerate() {
        let t = k * y as f32;
        re += v * t.cos();
        im += v * t.sin();
    }
    let scale = 2.0 / n as f32;
    let (re, im) = (re * scale, im * scale);
    (re.hypot(im), (-im).atan2(re))
}

/// Column-phase coherence at `cycles`, in `0..=1`.
///
/// `1` means every column agrees on both amplitude and phase — the signature of
/// a light modulation applied uniformly across the frame. Values well below 1
/// mean the structure differs column to column, i.e. it is scene content.
pub fn column_coherence(img: &Image, cycles: u32) -> f32 {
    let (w, h) = (img.width as usize, img.height as usize);
    if w == 0 || h == 0 {
        return 0.0;
    }
    let k = std::f32::consts::TAU * cycles as f32 / h as f32;
    // Precompute the basis and a Hann window once; both are column-independent.
    let basis: Vec<(f32, f32)> = (0..h)
        .map(|y| {
            let win =
                0.5 * (1.0 - (std::f32::consts::TAU * y as f32 / (h.max(2) - 1) as f32).cos());
            let t = k * y as f32;
            (win * t.cos(), win * t.sin())
        })
        .collect();

    let (mut sum_re, mut sum_im, mut sum_mag) = (0.0f32, 0.0f32, 0.0f32);
    for x in 0..w {
        // Per-column log-luminance, detrended so the column's own vertical
        // gradient does not dominate its phase.
        let col: Vec<f32> = (0..h)
            .map(|y| {
                let px = img.pixels[y * w + x];
                let a = px[3];
                if a <= 1e-6 {
                    return 0.0;
                }
                let inv = 1.0 / a;
                let lum = 0.2126 * px[0] * inv + 0.7152 * px[1] * inv + 0.0722 * px[2] * inv;
                lum.max(1e-6).ln()
            })
            .collect();
        let col = detrend(&col, DETREND_ROWS);
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (y, v) in col.iter().enumerate() {
            re += v * basis[y].0;
            im += v * basis[y].1;
        }
        sum_re += re;
        sum_im += im;
        sum_mag += re.hypot(im);
    }
    if sum_mag <= 1e-12 {
        return 0.0;
    }
    (sum_re.hypot(sum_im) / sum_mag).clamp(0.0, 1.0)
}

/// Single-frame detection: the strongest coherent candidate, or `None`.
///
/// Use when nothing else is available (stationary bands). It is markedly less
/// sensitive than [`detect_drifting`] on cluttered frames — prefer that when a
/// neighbouring frame exists.
pub fn detect_in_frame(img: &Image) -> Option<BandModel> {
    let profile = row_profile(img);
    let mut best: Option<(u32, f32)> = None;
    for cycles in MIN_CYCLES..MAX_CYCLES.min(profile.len() as u32 / 2) {
        let (amp, _) = fit_at(&profile, cycles);
        if best.is_none_or(|(_, b)| amp > b) {
            best = Some((cycles, amp));
        }
    }
    let (cycles, peak) = best?;
    if peak < MIN_AMPLITUDE {
        return None;
    }
    let confidence = column_coherence(img, cycles);
    if confidence < COHERENCE_THRESHOLD {
        return None;
    }
    let (amplitude, phase) = fit_at(&profile, cycles);
    Some(BandModel {
        cycles,
        amplitude,
        phase,
        confidence,
    })
}

/// Detection from a run of consecutive frames, by temporal differencing.
///
/// Differencing consecutive log row profiles cancels static scene content
/// exactly, which is what makes this far more sensitive than the single-frame
/// test. `profiles` must be consecutive frames of the same shot, in order.
///
/// Returns the model fitted to the **last** profile, so the caller can correct
/// that frame. `None` when nothing clears the SNR gate.
pub fn detect_drifting(profiles: &[Vec<f32>]) -> Option<BandModel> {
    if profiles.len() < 2 {
        return None;
    }
    let rows = profiles[0].len();
    if rows < (2 * MIN_CYCLES as usize) {
        return None;
    }
    let diffs: Vec<Vec<f32>> = profiles
        .windows(2)
        .map(|w| w[1].iter().zip(&w[0]).map(|(b, a)| b - a).collect())
        .collect();

    let hi = MAX_CYCLES.min(rows as u32 / 2);
    let mut amps: Vec<(u32, f32)> = Vec::new();
    for cycles in MIN_CYCLES..hi {
        // Median across difference frames: robust to a single frame disturbed by
        // a cut, a flash, or fast local motion.
        let mut per: Vec<f32> = diffs.iter().map(|d| fit_at(d, cycles).0).collect();
        per.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        amps.push((cycles, per[per.len() / 2]));
    }
    if amps.is_empty() {
        return None;
    }
    let mut sorted: Vec<f32> = amps.iter().map(|(_, a)| *a).collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = sorted[sorted.len() / 2].max(1e-9);
    let &(cycles, peak) = amps
        .iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))?;
    let snr = peak / median;
    if snr < DRIFT_SNR_THRESHOLD || peak < MIN_AMPLITUDE {
        return None;
    }
    let last = profiles.last()?;
    let (amplitude, phase) = fit_at(last, cycles);
    if amplitude < MIN_AMPLITUDE {
        return None;
    }
    Some(BandModel {
        cycles,
        amplitude,
        phase,
        // Map SNR onto 0..1 with the gate at ~0.5, so a caller can blend.
        confidence: (snr / (snr + DRIFT_SNR_THRESHOLD)).clamp(0.0, 1.0),
    })
}

/// A band whose phase advances deterministically from frame to frame.
///
/// This is the form a *clip* stores, and it exists because band phase changes
/// every single frame: `phase_n = phase0 + n * dphase`. An analysis that samples
/// on a stride — as the exposure pass does, since a long clip cannot be measured
/// frame by frame — can never carry a usable per-frame phase. Measuring the
/// advance once from a short consecutive burst and extrapolating is both cheaper
/// and correct, because the advance is set by the beat between the mains
/// frequency and the frame rate and does not drift.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BandTrack {
    pub cycles: u32,
    pub amplitude: f32,
    /// Phase at the first frame of the measured burst.
    pub phase0: f32,
    /// Phase advance per frame, radians.
    pub dphase: f32,
    pub confidence: f32,
}

impl BandTrack {
    /// The band as it stands `frame` frames after the burst started.
    pub fn model_at_frame(&self, frame: i64) -> BandModel {
        BandModel {
            cycles: self.cycles,
            amplitude: self.amplitude,
            phase: self.phase0 + self.dphase * frame as f32,
            confidence: self.confidence,
        }
    }
}

/// Wrap to `(-PI, PI]`.
#[inline]
fn wrap(a: f32) -> f32 {
    let t = std::f32::consts::TAU;
    let mut x = (a + std::f32::consts::PI) % t;
    if x < 0.0 {
        x += t;
    }
    x - std::f32::consts::PI
}

/// Estimate a [`BandTrack`] from a run of **consecutive** frame profiles.
///
/// Consecutive is load-bearing: the phase advance is only observable at the
/// frame rate, and a strided burst aliases it to something else entirely.
/// Amplitude and advance are taken as medians so one frame disturbed by fast
/// local motion cannot drag the estimate.
pub fn track_from_burst(profiles: &[Vec<f32>]) -> Option<BandTrack> {
    let base = detect_drifting(profiles)?;
    let fits: Vec<(f32, f32)> = profiles.iter().map(|p| fit_at(p, base.cycles)).collect();
    if fits.len() < 2 {
        return None;
    }
    let mut amps: Vec<f32> = fits.iter().map(|(a, _)| *a).collect();
    amps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let amplitude = amps[amps.len() / 2];
    if amplitude < MIN_AMPLITUDE {
        return None;
    }
    let mut steps: Vec<f32> = fits.windows(2).map(|w| wrap(w[1].1 - w[0].1)).collect();
    steps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(BandTrack {
        cycles: base.cycles,
        amplitude,
        phase0: fits[0].1,
        dphase: steps[steps.len() / 2],
        confidence: base.confidence,
    })
}

/// Divide out a fitted band, per scanline, in linear light.
///
/// `strength` scales the correction (`1.0` = full). The per-row gain is clamped
/// so a bad fit cannot blow out a row, and alpha is untouched.
pub fn apply_correction(img: &Image, model: &BandModel, strength: f32) -> Image {
    let mut out = img.clone();
    let h = img.height as usize;
    let w = img.width as usize;
    if h == 0 || w == 0 {
        return out;
    }
    let amp = model.amplitude * strength.clamp(0.0, 1.0);
    if amp.abs() <= 1e-6 {
        return out;
    }
    let k = std::f32::consts::TAU * model.cycles as f32 / h as f32;
    for y in 0..h {
        let b = amp * (k * y as f32 + model.phase).cos();
        // The model is I = S * (1 + b), so the inverse is a divide. Clamped
        // because an over-large fit would otherwise invert or explode a row.
        let gain = (1.0 / (1.0 + b)).clamp(0.5, 2.0);
        if (gain - 1.0).abs() <= 1e-6 {
            continue;
        }
        for x in 0..w {
            let px = &mut out.pixels[y * w + x];
            px[0] *= gain;
            px[1] *= gain;
            px[2] *= gain;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::ir::LinearColor;

    fn base(w: u32, h: u32, level: f32) -> Image {
        Image::filled(
            w,
            h,
            LinearColor {
                r: level,
                g: level,
                b: level,
                a: 1.0,
            },
        )
    }

    /// Multiply in a synthetic band.
    fn add_band(img: &Image, cycles: u32, amp: f32, phase: f32) -> Image {
        let mut out = img.clone();
        let (w, h) = (img.width as usize, img.height as usize);
        let k = std::f32::consts::TAU * cycles as f32 / h as f32;
        for y in 0..h {
            let m = 1.0 + amp * (k * y as f32 + phase).cos();
            for x in 0..w {
                let px = &mut out.pixels[y * w + x];
                px[0] *= m;
                px[1] *= m;
                px[2] *= m;
            }
        }
        out
    }

    /// Horizontal scene structure with per-column variation — the venetian-blind
    /// false positive the detector must reject. Slats are physically horizontal
    /// but their phase shifts across x (perspective) and they have edges.
    fn blinds(w: u32, h: u32, cycles: u32) -> Image {
        let mut img = base(w, h, 0.4);
        let k = std::f32::consts::TAU * cycles as f32 / h as f32;
        for y in 0..h as usize {
            for x in 0..w as usize {
                // Phase shears with x, and the profile is a hard square edge.
                let skew = 0.9 * (x as f32 / w as f32);
                let s = (k * y as f32 + skew * std::f32::consts::TAU).sin();
                let m = if s > 0.0 { 1.45 } else { 0.55 };
                let px = &mut img.pixels[y * w as usize + x];
                px[0] *= m;
                px[1] *= m;
                px[2] *= m;
            }
        }
        img
    }

    #[test]
    fn fit_recovers_a_known_band() {
        let img = add_band(&base(16, 200, 0.4), 10, 0.08, 0.6);
        let prof = row_profile(&img);
        let (amp, phase) = fit_at(&prof, 10);
        assert!((amp - 0.08).abs() < 0.02, "amplitude {amp} should be ~0.08");
        let dphase = (phase - 0.6).abs();
        assert!(
            dphase < 0.3 || (std::f32::consts::TAU - dphase) < 0.3,
            "phase {phase} should be ~0.6"
        );
    }

    #[test]
    fn uniform_frame_has_no_band() {
        assert!(detect_in_frame(&base(16, 200, 0.4)).is_none());
    }

    #[test]
    fn coherence_separates_bands_from_scene_structure() {
        let banded = add_band(&base(48, 200, 0.4), 12, 0.08, 0.0);
        let c_band = column_coherence(&banded, 12);
        let scene = blinds(48, 200, 12);
        let c_scene = column_coherence(&scene, 12);
        assert!(
            c_band > COHERENCE_THRESHOLD,
            "a real band should be coherent, got {c_band}"
        );
        assert!(
            c_scene < COHERENCE_THRESHOLD,
            "blinds must not read as coherent banding, got {c_scene}"
        );
    }

    #[test]
    fn blinds_are_not_detected_as_bands() {
        assert!(
            detect_in_frame(&blinds(48, 200, 12)).is_none(),
            "horizontal scene structure must not be corrected"
        );
    }

    /// Temporal differencing must find a band far weaker than the single-frame
    /// test can, even with strong static horizontal content in the way.
    #[test]
    fn drifting_detection_is_more_sensitive_than_single_frame() {
        let scene = blinds(48, 200, 9);
        let drift = std::f32::consts::TAU / 3.0;
        let profiles: Vec<Vec<f32>> = (0..6)
            .map(|i| row_profile(&add_band(&scene, 20, 0.01, drift * i as f32)))
            .collect();

        let found = detect_drifting(&profiles).expect("1% drifting band should be detected");
        assert_eq!(found.cycles, 20, "wrong frequency: {found:?}");

        // The same 1 % band is invisible to the single-frame test on this frame.
        let single = detect_in_frame(&add_band(&scene, 20, 0.01, 0.0));
        assert!(
            single.is_none(),
            "single-frame detection should miss this; it found {single:?}"
        );
    }

    /// Static scene content alone must not produce a drifting detection, however
    /// strong its horizontal structure is.
    #[test]
    fn static_scene_yields_no_drifting_detection() {
        let scene = blinds(48, 200, 9);
        let profiles: Vec<Vec<f32>> = (0..6).map(|_| row_profile(&scene)).collect();
        assert!(detect_drifting(&profiles).is_none());
    }

    #[test]
    fn correction_removes_the_band() {
        let clean = base(24, 200, 0.4);
        let banded = add_band(&clean, 12, 0.10, 0.4);
        let model = detect_in_frame(&banded).expect("band should be detected");
        let fixed = apply_correction(&banded, &model, 1.0);

        let ripple = |img: &Image| {
            let p = row_profile(img);
            (p.iter().map(|v| v * v).sum::<f32>() / p.len() as f32).sqrt()
        };
        let (before, after) = (ripple(&banded), ripple(&fixed));
        assert!(
            after < before * 0.25,
            "row ripple {before:.5} -> {after:.5}; expected a 4x reduction"
        );
    }

    #[test]
    fn track_recovers_the_per_frame_phase_advance() {
        let scene = blinds(48, 200, 9);
        let drift = std::f32::consts::TAU / 5.0;
        let profiles: Vec<Vec<f32>> = (0..10)
            .map(|i| row_profile(&add_band(&scene, 16, 0.05, drift * i as f32)))
            .collect();
        let tr = track_from_burst(&profiles).expect("burst should yield a track");
        assert_eq!(tr.cycles, 16);
        assert!(
            (super::wrap(tr.dphase - drift)).abs() < 0.2,
            "advance {} should be ~{drift}",
            tr.dphase
        );
        // Extrapolating to a later frame must land on that frame's real phase.
        let want = super::fit_at(&profiles[7], 16).1;
        let got = tr.model_at_frame(7).phase;
        assert!(
            super::wrap(got - want).abs() < 0.35,
            "extrapolated phase {got} vs measured {want}"
        );
    }

    #[test]
    fn track_is_none_without_a_band() {
        let scene = blinds(48, 200, 9);
        let profiles: Vec<Vec<f32>> = (0..8).map(|_| row_profile(&scene)).collect();
        assert!(track_from_burst(&profiles).is_none());
    }

    #[test]
    fn zero_strength_is_a_no_op() {
        let img = add_band(&base(8, 120, 0.4), 10, 0.1, 0.0);
        let model = BandModel {
            cycles: 10,
            amplitude: 0.1,
            phase: 0.0,
            confidence: 1.0,
        };
        assert_eq!(apply_correction(&img, &model, 0.0).pixels, img.pixels);
    }

    #[test]
    fn correction_preserves_alpha() {
        let mut img = add_band(&base(8, 120, 0.4), 10, 0.1, 0.0);
        for (i, px) in img.pixels.iter_mut().enumerate() {
            px[3] = 0.3 + (i % 5) as f32 * 0.1;
        }
        let model = BandModel {
            cycles: 10,
            amplitude: 0.1,
            phase: 0.0,
            confidence: 1.0,
        };
        let out = apply_correction(&img, &model, 1.0);
        let a: Vec<f32> = img.pixels.iter().map(|p| p[3]).collect();
        let b: Vec<f32> = out.pixels.iter().map(|p| p[3]).collect();
        assert_eq!(a, b);
    }

    #[test]
    fn degenerate_inputs_are_safe() {
        assert!(detect_drifting(&[]).is_none());
        assert!(detect_drifting(&[vec![0.0; 200]]).is_none());
        // Too few rows to host the minimum band frequency.
        assert!(detect_drifting(&[vec![0.0; 4], vec![0.0; 4]]).is_none());
        assert_eq!(row_profile(&Image::new(1, 1)).len(), 1);
    }
}
