//! The mixer graph (09 §4, §5): per-clip gain/fades → track bus → track
//! volume/pan/solo-mute → master bus → master volume → output, plus meter
//! taps. Runs identically on the interactive mixer-worker path and the
//! offline export path (09 §7) — same function, no device clock, no
//! wall-clock anywhere in this module (export determinism, SS-3, falls out of
//! that for free: see [`Mixer::render_block`]'s doc for the calling contract
//! that makes it hold).
//!
//! **fx_chain is inert in P3** (09 §1's phasing: DSP lands P8). Every track,
//! clip, and master bus signal below is summed and gain/pan-staged exactly per
//! 09 §4's diagram; the `fx_chain` field on [`TrackAudio`]/[`MasterBus`] is
//! read only to confirm it (still) matches the schema, never processed —
//! P8 inserts the EQ/Compressor/Gate/Limiter pass between the "track bus sum"
//! and "volume_db/pan" steps below without changing this function's shape.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use photonic_core::timeline::{
    self, AnimProps, AudioFade, AudioFxKind, AudioFxUnit, ChannelMap, ClipAudio, ClipAudioParams,
    EffectParams, FadeShape, MasterBus, MasterBusParams, PropPath, PropSet, PropValue, Tick,
    TrackAudio, TrackAudioParams, TrackId, TICKS_PER_SECOND,
};

use super::dsp::{AudioDiscontinuity, DspUnit, FxUnit};
use super::{BLOCK_FRAMES, CHANNELS};

/// The mixer's contract with decoded audio: the seam decode implements (real
/// ffmpeg-sidecar PCM) and tests fill with synthetic signals.
///
/// Per 09 §4's sample-rate policy, resampling to the mixer's rate happens at
/// decode-cache time, **not** here — a `PcmSource` MUST already emit at the
/// mixer's configured sample rate ([`Mixer::sample_rate`]); the mixer never
/// resamples per-block. Similarly, per 09 §4's channel policy, a source's
/// `channels()` is always 1 or 2 (multichannel sources are downmixed to
/// stereo at decode time) — [`ClipAudio::channel_map`] operates on that.
pub trait PcmSource: Send {
    /// 1 (mono) or 2 (stereo) — see the trait's channel-policy note above.
    fn channels(&self) -> u16;
    /// MUST equal the owning [`Mixer`]'s sample rate.
    fn sample_rate(&self) -> u32;
    /// Fill `out` (len `frames * channels()`, interleaved) starting at the
    /// source's current internal read position. Returns frames actually
    /// written; if less than `frames` (end of stream), the unwritten tail of
    /// `out` is left untouched — the caller pads with silence.
    fn read(&mut self, out: &mut [f32], frames: usize) -> usize;
}

/// One playing clip, as of this block's start. The elapsed/remaining ticks are
/// **clip-relative** (01 §6): `elapsed` since the clip's timeline-in-point,
/// `remaining` until its timeline-out-point. The playback controller computes
/// these each block — traversing the timeline to find "what's covering tick
/// t" is its job, not the mixer's (this crate's `audio/` owns only the signal
/// path, per this module family's doc).
pub struct ClipVoice<'a> {
    pub audio: &'a ClipAudio,
    pub elapsed: Tick,
    pub remaining: Tick,
    pub source: &'a mut dyn PcmSource,
}

/// One track's contribution to this block: its automation/mute/solo state
/// plus the clips currently sounding on it.
pub struct TrackVoice<'a> {
    pub id: TrackId,
    pub audio: &'a TrackAudio,
    pub clips: Vec<ClipVoice<'a>>,
}

/// A running peak+RMS pair for one meter tap point (09 §4's tap table, §8 UI).
/// Lock-free: the mixer worker writes with `Relaxed` atomics after each block,
/// the GUI thread reads the same way at its own refresh rate — no locking on
/// either side. Values are **instantaneous per-block** (not yet the ~300ms
/// VU-ballistics window 09 §8 describes for the UI widget); ballistics
/// smoothing is presentation logic the P8 UI layer applies on top of these
/// raw per-block readings.
#[derive(Debug, Default)]
pub struct StereoMeter {
    peak: [AtomicU32; 2],
    rms: [AtomicU32; 2],
}

impl StereoMeter {
    pub fn peak(&self) -> [f32; 2] {
        [
            f32::from_bits(self.peak[0].load(Ordering::Relaxed)),
            f32::from_bits(self.peak[1].load(Ordering::Relaxed)),
        ]
    }

    pub fn rms(&self) -> [f32; 2] {
        [
            f32::from_bits(self.rms[0].load(Ordering::Relaxed)),
            f32::from_bits(self.rms[1].load(Ordering::Relaxed)),
        ]
    }

    fn update(&self, block: &[f32]) {
        let frames = block.len() / CHANNELS;
        let mut peak = [0f32; 2];
        let mut sum_sq = [0f64; 2];
        for f in 0..frames {
            for c in 0..CHANNELS {
                let s = block[f * CHANNELS + c];
                let a = s.abs();
                if a > peak[c] {
                    peak[c] = a;
                }
                sum_sq[c] += (s as f64) * (s as f64);
            }
        }
        for c in 0..CHANNELS {
            let rms = if frames == 0 {
                0.0
            } else {
                (sum_sq[c] / frames as f64).sqrt() as f32
            };
            self.peak[c].store(peak[c].to_bits(), Ordering::Relaxed);
            self.rms[c].store(rms.to_bits(), Ordering::Relaxed);
        }
    }
}

/// One-pole smoother (09 §5's de-zipper): every gain/pan param ramps from its
/// previous-block smoothed value to the new block's target, one step per
/// sample. Advances strictly by sample count (never wall-clock), which is
/// what makes [`Mixer::render_block`]'s determinism guarantee hold (09 §7).
#[derive(Debug, Clone, Copy)]
struct Smoother {
    value: f64,
    target: f64,
}

impl Smoother {
    fn new(v: f64) -> Self {
        Smoother {
            value: v,
            target: v,
        }
    }
    fn set_target(&mut self, t: f64) {
        self.target = t;
    }
    fn step(&mut self, coeff: f64) -> f64 {
        self.value += (self.target - self.value) * coeff;
        self.value
    }
}

/// Per-sample one-pole coefficient for a time constant `tau_ms` at `fs` Hz —
/// the same envelope-follower form 09 §6.2 specifies, reused here for the
/// de-zipper (09 §5 gives the same "one-pole filter" description without a
/// separate formula, so this mirrors §6.2 for consistency).
fn one_pole_coeff(tau_ms: f64, fs: f64) -> f64 {
    1.0 - (-1.0 / (tau_ms * 0.001 * fs)).exp()
}

/// 09 §5: gain/pan de-zipper time constant.
const GAIN_PAN_TAU_MS: f64 = 10.0;

#[inline]
fn db_to_linear(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// Equal-power pan law (09 §4, exact formula): `pan` in `-1.0(L)..1.0(R)`.
/// At `pan == 0` both channels are `cos(pi/4) == sin(pi/4) ≈ 0.7071` (-3dB
/// each, constant perceived loudness across the sweep).
#[inline]
fn equal_power_pan(pan: f64) -> (f64, f64) {
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) * std::f64::consts::FRAC_PI_4;
    (angle.cos(), angle.sin())
}

/// Shape-weighted fade envelope (09 §2 `FadeShape`), `u` in `[0,1]`: `0` =
/// silent, `1` = full. `Linear`/`EqualPower` are 09 §4's exact description
/// (constant perceived loudness for `EqualPower`); `Log`/`SCurve` are named by
/// 09 §2 without a closed form, so their curves here are a **documented
/// implementation decision**: `Log` is linear-in-dB with a -60dB floor at
/// `u=0` (a fixed floor, not user-exposed, same spirit as the compressor's
/// fixed soft-knee width, 09 §6.3); `SCurve` is the standard cubic smoothstep.
fn shape_gain(shape: FadeShape, u: f64) -> f64 {
    let u = u.clamp(0.0, 1.0);
    match shape {
        FadeShape::Linear => u,
        FadeShape::EqualPower => (u * std::f64::consts::FRAC_PI_2).sin(),
        FadeShape::Log => {
            const FLOOR_DB: f64 = -60.0;
            db_to_linear(FLOOR_DB * (1.0 - u))
        }
        FadeShape::SCurve => u * u * (3.0 - 2.0 * u),
    }
}

fn fade_in_gain(fade: Option<&AudioFade>, elapsed: Tick) -> f64 {
    match fade {
        None => 1.0,
        Some(f) if f.duration.0 <= 0 => 1.0,
        Some(f) => shape_gain(f.shape, elapsed.0 as f64 / f.duration.0 as f64),
    }
}

fn fade_out_gain(fade: Option<&AudioFade>, remaining: Tick) -> f64 {
    match fade {
        None => 1.0,
        Some(f) if f.duration.0 <= 0 => 1.0,
        Some(f) => shape_gain(f.shape, remaining.0 as f64 / f.duration.0 as f64),
    }
}

/// `ChannelMap`'s per-mode algorithm (01 §5 / 09 §2 name the four variants
/// without spelling out the arithmetic) — **documented implementation
/// decision**: a mono source always duplicates to L=R at unity per channel
/// regardless of `map` (09 §4's channel policy is explicit about this case);
/// for a stereo source, `AsSource`/`StereoLR` pass L/R through unchanged,
/// `ChannelSwap` swaps them, and `MonoDownmix` forces an equal-power-free
/// average-and-duplicate (a plain mono fold, matching the variant's name).
#[inline]
fn read_channel_mapped(
    buf: &[f32],
    src_channels: usize,
    frame: usize,
    map: ChannelMap,
) -> (f32, f32) {
    let idx = frame * src_channels;
    if src_channels <= 1 {
        let m = buf[idx];
        return (m, m);
    }
    match map {
        ChannelMap::AsSource | ChannelMap::StereoLR => (buf[idx], buf[idx + 1]),
        ChannelMap::ChannelSwap => (buf[idx + 1], buf[idx]),
        ChannelMap::MonoDownmix => {
            let m = 0.5 * (buf[idx] + buf[idx + 1]);
            (m, m)
        }
    }
}

/// Evaluate one `f64` property lane at `t`, falling back to `base` for an
/// absent/orphaned lane or a non-`Float` value (defensive; the registry only
/// ever seeds `Float` for these paths, 09 §2).
fn eval_f64<T: PropSet>(anim: &AnimProps<T>, path: &str, base: f64, t: Tick) -> f64 {
    match anim.track(&PropPath::new(path)) {
        Some(track) => match timeline::eval(track, &PropValue::Float(base), t) {
            PropValue::Float(v) => v,
            _ => base,
        },
        None => base,
    }
}

/// Evaluate [`TrackAudioParams`] at sequence tick `t` (09 §2's `volume_db`,
/// `pan` paths).
pub fn eval_track_audio_params(anim: &AnimProps<TrackAudioParams>, t: Tick) -> TrackAudioParams {
    if anim.is_static() {
        return anim.base;
    }
    TrackAudioParams {
        volume_db: eval_f64(anim, "params.volume_db", anim.base.volume_db, t),
        pan: eval_f64(anim, "params.pan", anim.base.pan, t),
    }
}

/// Evaluate [`ClipAudioParams`] at **clip-relative** tick `t` (i.e. a
/// [`ClipVoice::elapsed`] value — 01 §6's `PropertyTrack`s on a clip target
/// are clip-relative, distinct from the sequence tick used for track/master
/// params).
pub fn eval_clip_audio_params(anim: &AnimProps<ClipAudioParams>, t: Tick) -> ClipAudioParams {
    if anim.is_static() {
        return anim.base;
    }
    ClipAudioParams {
        gain_db: eval_f64(anim, "params.gain_db", anim.base.gain_db, t),
    }
}

/// Evaluate [`MasterBusParams`] at sequence tick `t`.
pub fn eval_master_bus_params(anim: &AnimProps<MasterBusParams>, t: Tick) -> MasterBusParams {
    if anim.is_static() {
        return anim.base;
    }
    MasterBusParams {
        volume_db: eval_f64(anim, "params.volume_db", anim.base.volume_db, t),
    }
}

/// Evaluate an [`AnimProps<EffectParams>`] to a flat, block-resolved bag at
/// sequence tick `t` — the audio-fx analogue of [`eval_track_audio_params`].
/// Each animated lane is resolved once per block (never per sample, 09 §5),
/// falling back to the base value for a static/orphaned lane.
fn eval_effect_params(anim: &AnimProps<EffectParams>, t: Tick) -> EffectParams {
    if anim.is_static() {
        return anim.base.clone();
    }
    let mut out = anim.base.clone();
    for (path, value) in out.entries.iter_mut() {
        if let Some(track) = anim.track(path) {
            *value = timeline::eval(track, value, t);
        }
    }
    out
}

/// 31 §4.1 default declick window (samples) — the minimum tail length the
/// mixer caches per track so the next segment's head has material to splice
/// against, independent of any downstream `tail_samples`.
pub const DECLICK_WINDOW_SAMPLES: usize = 1000;
/// 31 §4.1 default declick threshold (dB, 0..30). Carried here for step-8's
/// declick maths; the per-clip/project serde surface (§4.3, §10) lands then.
pub const DECLICK_THRESHOLD_DB: f32 = 2.0;

/// The previous segment's tail handed to the next segment's head (31 §4.2, §8
/// step 4). `tail[c]` is `max(DECLICK_WINDOW_SAMPLES, largest downstream
/// tail_samples)` samples of the **pre-fx** track bus, oldest→newest (so the
/// splice point — the newest sample — is last). Only the graph-shape plumbing
/// is delivered here; the declick algorithm that consumes this is step 8.
pub struct SegmentBoundary {
    pub track: TrackId,
    pub at: Tick,
    pub tail: [Vec<f32>; CHANNELS],
}

/// A mixer-level discontinuity (31 §2.1). Carries the `TrackId`/`Tick` that
/// decide *which* units reset — the payload the payload-free
/// [`AudioDiscontinuity`] doc says belongs at the mixer, keeping `dsp/` free
/// of `Tick`. Applied by the feeder **between** `render_block` calls, never
/// mid-block, so [`Mixer::render_block`]'s wall-clock-free / bit-identical
/// guarantee (a deterministic function of the call sequence) is preserved.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MixerDiscontinuity {
    /// The playhead jumped — everything resets (31 §2.3).
    Seek { to: Tick },
    /// A clip boundary was crossed on `track`. Per 31 §2.3, this resets **only**
    /// that track's chain; the master chain is deliberately NOT reset (a master
    /// compressor must not pump at every cut).
    ClipBoundary { track: TrackId, at: Tick },
    /// The fx chain was edited. `track: None` targets the master chain.
    GraphChanged {
        track: Option<TrackId>,
        from_index: usize,
    },
    /// Sample rate / channel count changed — resets everything and recomputes
    /// rate-dependent coefficients.
    Reconfigure,
}

/// The mixer's live instantiation of one `fx_chain` (31 §8 step 2): the
/// concrete [`FxUnit`]s plus the kind sequence they were built from. That kind
/// sequence is the chain's identity — an [`AudioFxUnit`] has **no stable id**
/// (photonic-core `timeline::audio`), so a reorder is indistinguishable from a
/// rebuild, which is exactly why 31 §2.3 makes `GraphChanged` reset the changed
/// unit *and everything downstream of it*.
#[derive(Default)]
struct FxChainState {
    units: Vec<FxUnit>,
    kinds: Vec<AudioFxKind>,
    /// Test observability (31 §8 step 2 test b): how many times `sync` has
    /// rebuilt, so an unchanged chain can be asserted to allocate zero times.
    rebuild_count: u64,
}

impl FxChainState {
    fn new() -> Self {
        Self::default()
    }

    /// Rebuild `units` iff the kind sequence differs from `spec`. Returns
    /// `Some(first_changed_index)` when the shape changed, else `None`.
    ///
    /// The untouched common prefix keeps its live state; only from the first
    /// divergence onward is rebuilt (fresh units start clean, already
    /// satisfying `GraphChanged` for the rebuilt tail — the caller's explicit
    /// `reset_from` is then idempotent on them). Allocates only on an actual
    /// shape change, so the steady-state path is allocation-free.
    fn sync(&mut self, spec: &[AudioFxUnit]) -> Option<usize> {
        let diverge = self
            .kinds
            .iter()
            .zip(spec.iter())
            .position(|(k, u)| *k != u.kind);
        let first_changed = match diverge {
            Some(i) => i,
            None if self.kinds.len() == spec.len() => return None,
            None => self.kinds.len().min(spec.len()),
        };
        self.units.truncate(first_changed);
        for u in &spec[first_changed..] {
            self.units.push(FxUnit::new(u.kind));
        }
        self.kinds = spec.iter().map(|u| u.kind).collect();
        self.rebuild_count += 1;
        Some(first_changed)
    }

    /// Per-block: evaluate each enabled unit's `AnimProps` at `t` and push the
    /// resolved params in. Disabled units are left untouched.
    fn set_block_params(&mut self, spec: &[AudioFxUnit], t: Tick) {
        for (unit, s) in self.units.iter_mut().zip(spec) {
            if !s.enabled {
                continue;
            }
            let p = eval_effect_params(&s.params, t);
            unit.set_params_from(&p);
        }
    }

    /// Process the block through every enabled, known unit in order. Disabled
    /// and forward-compat (`Unknown`) units are skipped (rendered inert),
    /// keeping their slot so re-enabling does not shift indices.
    fn process_block(&mut self, spec: &[AudioFxUnit], sample_rate: u32, block: &mut [f32]) {
        for (unit, s) in self.units.iter_mut().zip(spec) {
            if !s.enabled || s.kind.is_unknown() {
                continue;
            }
            unit.process(sample_rate, block, None);
        }
    }

    /// Σ latency over enabled units only (31 §3.1). Disabled units contribute
    /// zero but keep their slot. Consumed by the per-track delay-compensation
    /// and playhead-derivation of 31 §3 (a follow-on); exercised now by the
    /// step-2 tests.
    #[allow(dead_code)]
    fn latency_samples(&self, spec: &[AudioFxUnit]) -> u32 {
        self.units
            .iter()
            .zip(spec)
            .filter(|(_, s)| s.enabled)
            .map(|(u, _)| u.latency_samples())
            .sum()
    }

    /// Σ tail over enabled units only (31 §3.3, §4.2).
    fn tail_samples(&self, spec: &[AudioFxUnit]) -> u32 {
        self.units
            .iter()
            .zip(spec)
            .filter(|(_, s)| s.enabled)
            .map(|(u, _)| u.tail_samples())
            .sum()
    }

    /// Reset every unit from `index` onward (31 §2.3's downstream rule).
    fn reset_from(&mut self, index: usize, cause: AudioDiscontinuity) {
        for u in self.units.iter_mut().skip(index) {
            u.reset(cause);
        }
    }
}

/// A per-track ring of the **pre-fx** track bus (31 §8 step 4). Long enough to
/// hand the previous segment's tail to the next segment's head:
/// `len = max(DECLICK_WINDOW_SAMPLES, downstream tail_samples)`. Written every
/// block (a boundary can land on any block); de-interleaved into per-channel
/// `Vec`s only when a boundary is consumed, so the steady-state path allocates
/// nothing.
struct TailBuf {
    len: usize,
    ch: [Vec<f32>; CHANNELS],
    pos: usize,
    filled: usize,
    pending: Option<Tick>,
}

impl TailBuf {
    fn new(len: usize) -> Self {
        let len = len.max(1);
        TailBuf {
            len,
            ch: std::array::from_fn(|_| vec![0.0; len]),
            pos: 0,
            filled: 0,
            pending: None,
        }
    }

    /// Re-length the ring (re-primed with silence). Called only when the
    /// downstream tail length actually changes — never on the steady-state
    /// path.
    fn resize(&mut self, len: usize) {
        let len = len.max(1);
        self.len = len;
        for c in &mut self.ch {
            c.clear();
            c.resize(len, 0.0);
        }
        self.pos = 0;
        self.filled = 0;
        self.pending = None;
    }

    /// Push one interleaved block (`frames * CHANNELS`) into the ring.
    fn write_block(&mut self, block: &[f32]) {
        let frames = block.len() / CHANNELS;
        for f in 0..frames {
            for c in 0..CHANNELS {
                self.ch[c][self.pos] = block[f * CHANNELS + c];
            }
            self.pos = (self.pos + 1) % self.len;
            if self.filled < self.len {
                self.filled += 1;
            }
        }
    }

    /// Chronological (oldest→newest) per-channel copy, front-padded with
    /// silence until the ring has filled — so the newest sample (the splice
    /// point 31 §4 repairs) is always last.
    fn snapshot(&self) -> [Vec<f32>; CHANNELS] {
        std::array::from_fn(|c| {
            let mut out = vec![0.0f32; self.len];
            let start = (self.pos + self.len - self.filled) % self.len;
            for i in 0..self.filled {
                out[self.len - self.filled + i] = self.ch[c][(start + i) % self.len];
            }
            out
        })
    }

    /// Zero the ring, preserving its length (Seek/Reconfigure re-prime,
    /// 31 §2.3).
    fn clear_silence(&mut self) {
        for c in &mut self.ch {
            c.iter_mut().for_each(|s| *s = 0.0);
        }
        self.pos = 0;
        self.filled = 0;
        self.pending = None;
    }
}

struct TrackSmoothState {
    volume_db: Smoother,
    pan: Smoother,
}

/// Circular delay line for latency compensation (31 §3): holds interleaved
/// stereo frames and delays a block by a fixed number of frames.
struct DelayLine {
    /// Interleaved stereo ring, length = `delay_frames * CHANNELS`.
    buf: Vec<f32>,
    /// Write head in frames.
    pos: usize,
    delay_frames: usize,
}

impl DelayLine {
    fn new() -> Self {
        DelayLine {
            buf: Vec::new(),
            pos: 0,
            delay_frames: 0,
        }
    }

    fn set_delay(&mut self, frames: usize) {
        if frames == self.delay_frames {
            return;
        }
        self.delay_frames = frames;
        self.buf = vec![0.0; frames.max(1) * CHANNELS];
        self.pos = 0;
    }

    /// Delay `block` (one `BLOCK_FRAMES` interleaved buffer) by `delay_frames`.
    /// Zero delay is a no-op.
    fn process_block(&mut self, block: &mut [f32]) {
        if self.delay_frames == 0 {
            return;
        }
        debug_assert_eq!(block.len(), BLOCK_FRAMES * CHANNELS);
        for f in 0..BLOCK_FRAMES {
            let wi = (self.pos % self.delay_frames) * CHANNELS;
            let ri = wi; // same slot: read old, then write new
            for c in 0..CHANNELS {
                let idx = f * CHANNELS + c;
                let delayed = self.buf[ri + c];
                self.buf[wi + c] = block[idx];
                block[idx] = delayed;
            }
            self.pos = self.pos.wrapping_add(1);
        }
    }
}

/// The mixer graph. One instance per playing/exporting sequence; owned and
/// driven by the mixer-worker thread (interactive) or the export render loop
/// (offline, 09 §7) — never touched from the cpal callback itself (02 §1).
pub struct Mixer {
    sample_rate: u32,
    gain_pan_coeff: f64,
    track_smooth: HashMap<TrackId, TrackSmoothState>,
    master_smooth: Option<Smoother>,
    track_post_fx_meters: HashMap<TrackId, Arc<StereoMeter>>,
    track_post_fader_meters: HashMap<TrackId, Arc<StereoMeter>>,
    master_post_fx_meter: Arc<StereoMeter>,
    output_meter: Arc<StereoMeter>,
    // Instantiated fx chains (31 §8 step 2), synced from the model each block.
    track_fx: HashMap<TrackId, FxChainState>,
    master_fx: FxChainState,
    // Per-track segment-boundary tail rings (31 §8 step 4).
    tail_cache: HashMap<TrackId, TailBuf>,
    // 31 §3: per-track delay lines for latency compensation.
    track_delay: HashMap<TrackId, DelayLine>,
    /// Last computed graph latency (samples) — for clock offset (31 §3).
    last_graph_latency: u32,
    // Reused per-block scratch (avoids per-block allocation on the mixer-
    // worker hot path; not itself RT-required off the cpal callback thread,
    // but there is no reason to allocate every block either).
    track_bus_scratch: Vec<f32>,
    master_bus_scratch: Vec<f32>,
    clip_scratch: Vec<f32>,
}

impl Mixer {
    /// `sample_rate` MUST match every [`PcmSource`] this mixer will read from
    /// (09 §4: resampling to mix rate happens at decode time, not here) and is
    /// used both for the de-zipper's per-sample coefficient (09 §5) and for
    /// converting clip-relative ticks to sample offsets within a block (exact
    /// integer arithmetic for the sample rates 01 §1 lists as supported).
    pub fn new(sample_rate: u32) -> Self {
        Mixer {
            sample_rate,
            gain_pan_coeff: one_pole_coeff(GAIN_PAN_TAU_MS, sample_rate as f64),
            track_smooth: HashMap::new(),
            master_smooth: None,
            track_post_fx_meters: HashMap::new(),
            track_post_fader_meters: HashMap::new(),
            master_post_fx_meter: Arc::new(StereoMeter::default()),
            output_meter: Arc::new(StereoMeter::default()),
            track_fx: HashMap::new(),
            master_fx: FxChainState::new(),
            tail_cache: HashMap::new(),
            track_delay: HashMap::new(),
            last_graph_latency: 0,
            track_bus_scratch: vec![0.0; BLOCK_FRAMES * CHANNELS],
            master_bus_scratch: vec![0.0; BLOCK_FRAMES * CHANNELS],
            clip_scratch: vec![0.0; BLOCK_FRAMES * CHANNELS],
        }
    }

    /// Total graph latency in samples from the last [`render_block`] (31 §3):
    /// `max(track latencies) + master chain latency`. Used for A/V clock offset.
    pub fn last_graph_latency_samples(&self) -> u32 {
        self.last_graph_latency
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Drop all per-track smoothing state, so the next block for any track
    /// re-initializes at its target (no ramp-in). Useful when reusing a
    /// `Mixer` instance across an unrelated document/session where stale
    /// `TrackId`s could otherwise collide with a new project's ids.
    pub fn reset(&mut self) {
        self.track_smooth.clear();
        self.master_smooth = None;
        // Drop instantiated fx chains and tail rings too: stale `TrackId`s from
        // a prior document would otherwise carry live DSP/tail state into an
        // unrelated one. Chains rebuild from the model on the next block.
        self.track_fx.clear();
        self.master_fx = FxChainState::new();
        self.tail_cache.clear();
        self.track_delay.clear();
        self.last_graph_latency = 0;
    }

    /// Apply a mixer-level discontinuity (31 §2.1/§2.3). MUST be called by the
    /// feeder **between** [`render_block`](Self::render_block) calls (never
    /// mid-block); a discontinuity is thus a deterministic function of the call
    /// sequence, and `render_block`'s wall-clock-free guarantee is preserved.
    ///
    /// The §2.3 policy mapping IS the deliverable:
    /// - `Seek` / `Reconfigure` reset **every** track chain and the master
    ///   chain from unit 0, clear the gain/pan smoothers, and re-prime the tail
    ///   rings with silence. `Reconfigure` additionally recomputes the
    ///   rate-dependent `gain_pan_coeff`.
    /// - `ClipBoundary { track }` resets **only** that track's chain; the master
    ///   chain is deliberately left running (a master compressor must not pump
    ///   at every cut — the single most important assertion in 31 §9 item 5).
    /// - `GraphChanged { track, from_index }` resets that chain from
    ///   `from_index`, then the whole master chain (downstream of every track);
    ///   `track: None` resets only the master chain from `from_index`.
    pub fn notify_discontinuity(&mut self, ev: MixerDiscontinuity) {
        match ev {
            MixerDiscontinuity::Seek { .. } => self.reset_all(AudioDiscontinuity::Seek),
            MixerDiscontinuity::Reconfigure => {
                self.reset_all(AudioDiscontinuity::Reconfigure);
                self.gain_pan_coeff = one_pole_coeff(GAIN_PAN_TAU_MS, self.sample_rate as f64);
            }
            MixerDiscontinuity::ClipBoundary { track, .. } => {
                if let Some(fx) = self.track_fx.get_mut(&track) {
                    fx.reset_from(0, AudioDiscontinuity::ClipBoundary);
                }
                // Master chain intentionally NOT reset (31 §2.3).
            }
            MixerDiscontinuity::GraphChanged { track, from_index } => match track {
                Some(t) => {
                    if let Some(fx) = self.track_fx.get_mut(&t) {
                        fx.reset_from(from_index, AudioDiscontinuity::GraphChanged);
                    }
                    self.master_fx
                        .reset_from(0, AudioDiscontinuity::GraphChanged);
                }
                None => self
                    .master_fx
                    .reset_from(from_index, AudioDiscontinuity::GraphChanged),
            },
        }
    }

    /// Reset every chain wholesale plus the smoothers and tail rings — the
    /// shared body of `Seek`/`Reconfigure`.
    fn reset_all(&mut self, cause: AudioDiscontinuity) {
        for fx in self.track_fx.values_mut() {
            fx.reset_from(0, cause);
        }
        self.master_fx.reset_from(0, cause);
        self.track_smooth.clear();
        self.master_smooth = None;
        for buf in self.tail_cache.values_mut() {
            buf.clear_silence();
        }
    }

    /// Consume the tail captured for `track` at its last segment boundary
    /// (31 §8 step 4). Returns `None` if no boundary is pending; a given
    /// boundary is handed out exactly once.
    pub fn take_segment_boundary(&mut self, track: TrackId) -> Option<SegmentBoundary> {
        let buf = self.tail_cache.get_mut(&track)?;
        let at = buf.pending.take()?;
        Some(SegmentBoundary {
            track,
            at,
            tail: buf.snapshot(),
        })
    }

    /// Drop the cached tail ring for `track` — call on any edit affecting the
    /// track (31 §4.2 "invalidated on edit"). It re-allocates on the next
    /// block.
    pub fn invalidate_tail(&mut self, track: TrackId) {
        self.tail_cache.remove(&track);
    }

    /// Lazily-created meter handle: after `fx_chain` (currently a P3 no-op,
    /// see module doc), before the fader (09 §4 tap table "Track post-fx").
    /// Clone the returned `Arc` once and poll it from the GUI thread — no
    /// further calls into `Mixer` are needed to read it.
    pub fn track_post_fx_meter(&mut self, id: TrackId) -> Arc<StereoMeter> {
        self.track_post_fx_meters
            .entry(id)
            .or_insert_with(|| Arc::new(StereoMeter::default()))
            .clone()
    }

    /// After `volume_db`/pan, before the mute/solo gate (09 §4 tap table
    /// "Track post-fader").
    pub fn track_post_fader_meter(&mut self, id: TrackId) -> Arc<StereoMeter> {
        self.track_post_fader_meters
            .entry(id)
            .or_insert_with(|| Arc::new(StereoMeter::default()))
            .clone()
    }

    /// After master `fx_chain` (currently a P3 no-op) — 09 §4 tap table
    /// "Master post-fx".
    pub fn master_post_fx_meter(&self) -> Arc<StereoMeter> {
        self.master_post_fx_meter.clone()
    }

    /// Final output, post `MasterBusParams.volume_db` — 09 §4 tap table
    /// "Output".
    pub fn output_meter(&self) -> Arc<StereoMeter> {
        self.output_meter.clone()
    }

    /// Render exactly one [`BLOCK_FRAMES`]-frame block (09 §5).
    ///
    /// # Calling contract (determinism, 09 §7)
    ///
    /// Callers MUST invoke this once per consecutive block in strictly
    /// increasing tick order (both the interactive mixer worker and the
    /// offline export loop do this naturally — export just doesn't pace
    /// itself against a device clock). The one-pole smoothers persist across
    /// calls and advance by exactly `BLOCK_FRAMES` samples each time, never by
    /// wall-clock elapsed time — this function reads no clock of any kind —
    /// so "same project, same call sequence" always produces bit-identical
    /// output. A seek/loop discontinuity does not require resetting smoother
    /// state first (09 §5 treats it like any other parameter change: the
    /// smoother ramps toward the new target over the de-zipper's 10ms, same
    /// as a live fader move); call [`Mixer::reset`] only when reusing a
    /// `Mixer` across unrelated sessions.
    ///
    /// `block_start_tick` is the **sequence** tick at this block's start, used
    /// to evaluate `TrackAudioParams`/`MasterBusParams` automation.
    /// `ClipAudioParams` automation is evaluated at each clip's own
    /// [`ClipVoice::elapsed`] (clip-relative, see [`eval_clip_audio_params`]).
    ///
    /// `out` MUST be exactly `BLOCK_FRAMES * CHANNELS` interleaved-stereo
    /// samples.
    pub fn render_block(
        &mut self,
        block_start_tick: Tick,
        tracks: &mut [TrackVoice<'_>],
        master: &MasterBus,
        out: &mut [f32],
    ) {
        assert_eq!(
            out.len(),
            BLOCK_FRAMES * CHANNELS,
            "render_block: out must be exactly one block"
        );

        let coeff = self.gain_pan_coeff;
        let sr = self.sample_rate;
        let tick_per_sample = TICKS_PER_SECOND / self.sample_rate.max(1) as i64;
        let any_solo = tracks.iter().any(|t| t.audio.solo);
        // A track fx-graph change resets the master chain wholesale (master is
        // downstream of every track, 31 §2.3); collected here, applied once
        // after the track loop.
        let mut master_needs_reset = false;

        // 31 §3: pre-sync chains so latency_samples is current, then equalise
        // every track path to max_track_latency before the master sum.
        for track in tracks.iter() {
            let fx = self
                .track_fx
                .entry(track.id)
                .or_insert_with(FxChainState::new);
            if let Some(i) = fx.sync(&track.audio.fx_chain) {
                fx.reset_from(i, AudioDiscontinuity::GraphChanged);
                master_needs_reset = true;
            }
        }
        if let Some(i) = self.master_fx.sync(&master.fx_chain) {
            self.master_fx
                .reset_from(i, AudioDiscontinuity::GraphChanged);
        }
        // Pre-pass sizes delay lines for the current chain shape. Units that
        // report 0 until first process (limiter lookahead) get 0 compensation
        // for one block — acceptable and deterministic. Total latency is
        // re-measured after process_block below.
        let max_track_lat = tracks
            .iter()
            .map(|t| {
                self.track_fx
                    .get(&t.id)
                    .map(|fx| fx.latency_samples(&t.audio.fx_chain))
                    .unwrap_or(0)
            })
            .max()
            .unwrap_or(0);

        self.master_bus_scratch.fill(0.0);

        for track in tracks.iter_mut() {
            self.track_bus_scratch.fill(0.0);

            for clip in track.clips.iter_mut() {
                render_clip_into(
                    clip,
                    tick_per_sample,
                    &mut self.clip_scratch,
                    &mut self.track_bus_scratch,
                );
            }

            // ── 31 §8 step 4: capture the pre-fx track bus into the tail ring
            //    (written every block; a boundary can land on any block). The
            //    capture point is deliberately here — after clip rendering,
            //    before the fx chain — so the declick (step 8) repairs the
            //    *source* splice, not the processed signal. Ring length is
            //    max(DECLICK_WINDOW_SAMPLES, downstream tail_samples) (§4.2). ──
            let downstream_tail = self
                .track_fx
                .get(&track.id)
                .map_or(0, |fx| fx.tail_samples(&track.audio.fx_chain))
                + self.master_fx.tail_samples(&master.fx_chain);
            let tail_len = DECLICK_WINDOW_SAMPLES.max(downstream_tail as usize);
            let buf = self
                .tail_cache
                .entry(track.id)
                .or_insert_with(|| TailBuf::new(tail_len));
            if buf.len != tail_len {
                buf.resize(tail_len);
            }
            buf.write_block(&self.track_bus_scratch);
            // A clip ending within this block is a segment boundary on this
            // track: mark the tail pending so a feeder can hand it to the next
            // segment's head (31 §4.2). Auto-detected from `ClipVoice::remaining`.
            let block_ticks = BLOCK_FRAMES as i64 * tick_per_sample;
            if track.clips.iter().any(|c| c.remaining.0 <= block_ticks) {
                buf.pending = Some(block_start_tick);
            }

            // ── 31 §8 step 2: process the (already-synced) track fx chain. ──
            let fx = self
                .track_fx
                .entry(track.id)
                .or_insert_with(FxChainState::new);
            fx.set_block_params(&track.audio.fx_chain, block_start_tick);
            fx.process_block(&track.audio.fx_chain, sr, &mut self.track_bus_scratch);
            let track_lat = fx.latency_samples(&track.audio.fx_chain);

            // Tap: post-fx (09 §4 tap table "Track post-fx"), now genuinely
            // after the fx chain (31 §8 step 2).
            self.track_post_fx_meter(track.id)
                .update(&self.track_bus_scratch);

            let params = eval_track_audio_params(&track.audio.params, block_start_tick);
            let state = self
                .track_smooth
                .entry(track.id)
                .or_insert_with(|| TrackSmoothState {
                    volume_db: Smoother::new(params.volume_db),
                    pan: Smoother::new(params.pan),
                });
            state.volume_db.set_target(params.volume_db);
            state.pan.set_target(params.pan);

            for f in 0..BLOCK_FRAMES {
                let vdb = state.volume_db.step(coeff);
                let pan = state.pan.step(coeff);
                let lin = db_to_linear(vdb);
                let (gl, gr) = equal_power_pan(pan);
                self.track_bus_scratch[f * CHANNELS] *= (lin * gl) as f32;
                self.track_bus_scratch[f * CHANNELS + 1] *= (lin * gr) as f32;
            }

            // Tap: post-fader, pre-mute-gate (09 §4 tap table).
            self.track_post_fader_meter(track.id)
                .update(&self.track_bus_scratch);

            // 31 §3: delay this track so all paths arrive sample-aligned at
            // the master bus (compensation = max_track_lat − track_lat).
            let compensation = max_track_lat.saturating_sub(track_lat) as usize;
            {
                let dl = self
                    .track_delay
                    .entry(track.id)
                    .or_insert_with(DelayLine::new);
                dl.set_delay(compensation);
                dl.process_block(&mut self.track_bus_scratch);
            }

            // Solo-safe gating (09 §4): any solo active -> only soloed
            // (and not muted) tracks contribute; else normal mute-only gate.
            let active = if any_solo {
                track.audio.solo && !track.audio.mute
            } else {
                !track.audio.mute
            };
            if active {
                for i in 0..BLOCK_FRAMES * CHANNELS {
                    self.master_bus_scratch[i] += self.track_bus_scratch[i];
                }
            }
        }

        // ── 31 §8 step 2: master fx chain. Already synced above; apply any
        //    wholesale reset a track graph change forced, then process. ──
        if master_needs_reset {
            self.master_fx
                .reset_from(0, AudioDiscontinuity::GraphChanged);
        }
        self.master_fx
            .set_block_params(&master.fx_chain, block_start_tick);
        self.master_fx
            .process_block(&master.fx_chain, sr, &mut self.master_bus_scratch);

        // Tap: master post-fx (09 §4 tap table), now after the master fx chain.
        self.master_post_fx_meter.update(&self.master_bus_scratch);

        let master_params = eval_master_bus_params(&master.params, block_start_tick);
        let m = self
            .master_smooth
            .get_or_insert_with(|| Smoother::new(master_params.volume_db));
        m.set_target(master_params.volume_db);

        for f in 0..BLOCK_FRAMES {
            let vdb = m.step(coeff);
            let lin = db_to_linear(vdb) as f32;
            out[f * CHANNELS] = self.master_bus_scratch[f * CHANNELS] * lin;
            out[f * CHANNELS + 1] = self.master_bus_scratch[f * CHANNELS + 1] * lin;
        }

        // Tap: output, post `MasterBusParams.volume_db` (09 §4 tap table).
        self.output_meter.update(out);

        // 31 §3: publish total graph latency *after* process so units that
        // allocate lookahead on first process report correctly.
        let master_lat = self.master_fx.latency_samples(&master.fx_chain);
        let max_track_lat = tracks
            .iter()
            .map(|t| {
                self.track_fx
                    .get(&t.id)
                    .map(|fx| fx.latency_samples(&t.audio.fx_chain))
                    .unwrap_or(0)
            })
            .max()
            .unwrap_or(0);
        self.last_graph_latency = max_track_lat.saturating_add(master_lat);
    }
}

/// Render one clip's contribution for the block into `track_bus` (accumulate,
/// not overwrite — multiple clips can sum onto the same track bus, e.g. a
/// crossfade). `clip_scratch` is reused raw-PCM scratch, resized as needed by
/// the caller to at least `BLOCK_FRAMES * 2`.
fn render_clip_into(
    clip: &mut ClipVoice<'_>,
    tick_per_sample: i64,
    clip_scratch: &mut [f32],
    track_bus: &mut [f32],
) {
    let src_channels = clip.source.channels().max(1) as usize;
    debug_assert!(
        src_channels <= 2,
        "09 §4 channel policy: v1 PcmSources are mono/stereo only"
    );
    let buf = &mut clip_scratch[..BLOCK_FRAMES * src_channels];
    let got = clip.source.read(buf, BLOCK_FRAMES);
    if got < BLOCK_FRAMES {
        buf[got * src_channels..].fill(0.0);
    }

    // ClipAudioParams automation is clip-relative (01 §6) — evaluated at this
    // block's clip-relative start, not the sequence tick.
    let params = eval_clip_audio_params(&clip.audio.params, clip.elapsed);
    let base_gain = db_to_linear(params.gain_db);
    let channel_map = clip.audio.channel_map;
    let fade_in = clip.audio.fade_in.as_ref();
    let fade_out = clip.audio.fade_out.as_ref();

    for f in 0..BLOCK_FRAMES {
        let (sl, sr) = read_channel_mapped(buf, src_channels, f, channel_map);
        let dt = Tick(f as i64 * tick_per_sample);
        let elapsed = clip.elapsed.saturating_add(dt);
        let remaining = clip.remaining.saturating_sub(dt);
        let g = base_gain * fade_in_gain(fade_in, elapsed) * fade_out_gain(fade_out, remaining);
        track_bus[f * CHANNELS] += sl * g as f32;
        track_bus[f * CHANNELS + 1] += sr * g as f32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::test_support::{SilenceSource, StepSource};
    use photonic_core::timeline::{AudioFade, ClipAudio, Interp, Keyframe, TrackAudio};

    const SR: u32 = 48_000;

    fn track_id(n: u32) -> TrackId {
        TrackId(uuid::Uuid::from_u128(n as u128))
    }

    fn silent_clip_voice<'a>(audio: &'a ClipAudio, source: &'a mut dyn PcmSource) -> ClipVoice<'a> {
        ClipVoice {
            audio,
            elapsed: Tick(0),
            remaining: Tick::from_seconds(3600),
            source,
        }
    }

    // ── Pan law ──────────────────────────────────────────────────────────

    #[test]
    fn pan_law_center_is_minus_3db_both_channels() {
        let (gl, gr) = equal_power_pan(0.0);
        let expected = std::f64::consts::FRAC_1_SQRT_2; // cos(pi/4) == sin(pi/4)
        assert!((gl - expected).abs() < 1e-12);
        assert!((gr - expected).abs() < 1e-12);
        let db = 20.0 * gl.log10();
        assert!(
            (db - (-3.0)).abs() < 0.02,
            "center pan should be ~-3dB, got {db}"
        );
    }

    #[test]
    fn pan_law_hard_left_and_right() {
        let (gl, gr) = equal_power_pan(-1.0);
        assert!((gl - 1.0).abs() < 1e-9);
        assert!(gr.abs() < 1e-9);
        let (gl, gr) = equal_power_pan(1.0);
        assert!(gl.abs() < 1e-9);
        assert!((gr - 1.0).abs() < 1e-9);
    }

    // ── Fade shapes ──────────────────────────────────────────────────────

    #[test]
    fn equal_power_fade_midpoint() {
        let g = shape_gain(FadeShape::EqualPower, 0.5);
        assert!((g - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-9);
    }

    #[test]
    fn linear_fade_is_identity() {
        assert_eq!(shape_gain(FadeShape::Linear, 0.0), 0.0);
        assert_eq!(shape_gain(FadeShape::Linear, 0.5), 0.5);
        assert_eq!(shape_gain(FadeShape::Linear, 1.0), 1.0);
    }

    #[test]
    fn fade_shapes_are_monotone_0_to_1() {
        for shape in [
            FadeShape::Linear,
            FadeShape::EqualPower,
            FadeShape::Log,
            FadeShape::SCurve,
        ] {
            // `Log`'s -60dB floor at u=0 is a floor, not literal silence
            // (documented decision on `shape_gain`) — 0.001 linear, not 0.
            let silence_tol = if shape == FadeShape::Log { 1e-2 } else { 1e-6 };
            assert!(
                (shape_gain(shape, 0.0)).abs() < silence_tol,
                "{shape:?} at 0 should be ~silent"
            );
            assert!(
                (shape_gain(shape, 1.0) - 1.0).abs() < 1e-6,
                "{shape:?} at 1 should be unity"
            );
            let mut prev = -1.0;
            for i in 0..=20 {
                let u = i as f64 / 20.0;
                let g = shape_gain(shape, u);
                assert!(g >= prev - 1e-9, "{shape:?} not monotone at {u}");
                prev = g;
            }
        }
    }

    // ── Solo-safe truth table (09 §4) ───────────────────────────────────

    #[test]
    fn solo_safe_truth_table() {
        // (mute, solo) per track -> expected `active` under any_solo=true and
        // any_solo=false, mirroring Mixer::render_block's gating logic
        // directly (kept in lockstep intentionally: this is the spec's truth
        // table, 09 §4).
        let cases = [
            // (mute, solo, active_when_no_solo, active_when_any_solo)
            (false, false, true, false),
            (true, false, false, false),
            (false, true, true, true),
            (true, true, false, false), // mute wins even when soloed
        ];
        for (mute, solo, expect_no_solo, expect_any_solo) in cases {
            let active_no_solo = !mute;
            let active_any_solo = solo && !mute;
            assert_eq!(active_no_solo, expect_no_solo, "mute={mute} solo={solo}");
            assert_eq!(active_any_solo, expect_any_solo, "mute={mute} solo={solo}");
        }
    }

    #[test]
    fn solo_gates_non_soloed_tracks_end_to_end() {
        let mut mixer = Mixer::new(SR);
        let master = master_bus_no_limiter();

        let mut a_audio = TrackAudio::new();
        a_audio.solo = true;
        let mut b_audio = TrackAudio::new();
        b_audio.mute = false;
        b_audio.solo = false;

        let mut src_a = StepSource {
            channels: 2,
            sample_rate: SR,
            value: 1.0,
        };
        let mut src_b = StepSource {
            channels: 2,
            sample_rate: SR,
            value: 1.0,
        };
        let clip_audio = ClipAudio::new();

        let mut tracks = vec![
            TrackVoice {
                id: track_id(1),
                audio: &a_audio,
                clips: vec![silent_clip_voice(&clip_audio, &mut src_a)],
            },
            TrackVoice {
                id: track_id(2),
                audio: &b_audio,
                clips: vec![silent_clip_voice(&clip_audio, &mut src_b)],
            },
        ];

        let mut out = vec![0.0; BLOCK_FRAMES * CHANNELS];
        // Render enough blocks for the de-zipper to fully settle.
        for _ in 0..8 {
            mixer.render_block(Tick(0), &mut tracks, &master, &mut out);
        }
        // Only track A (soloed) should contribute; B is silenced.
        assert!(
            out.iter().all(|&s| s.abs() > 0.01),
            "soloed track should be audible"
        );
    }

    #[test]
    fn mute_silences_track_end_to_end() {
        let mut mixer = Mixer::new(SR);
        let master = master_bus_no_limiter();

        let mut audio = TrackAudio::new();
        audio.mute = true;
        let mut src = StepSource {
            channels: 2,
            sample_rate: SR,
            value: 1.0,
        };
        let clip_audio = ClipAudio::new();
        let mut tracks = vec![TrackVoice {
            id: track_id(1),
            audio: &audio,
            clips: vec![silent_clip_voice(&clip_audio, &mut src)],
        }];

        let mut out = vec![-9.0; BLOCK_FRAMES * CHANNELS];
        mixer.render_block(Tick(0), &mut tracks, &master, &mut out);
        assert!(
            out.iter().all(|&s| s == 0.0),
            "muted track must produce silence"
        );
    }

    // ── De-zipper ────────────────────────────────────────────────────────

    #[test]
    fn de_zipper_bounds_sample_to_sample_delta() {
        let coeff = one_pole_coeff(GAIN_PAN_TAU_MS, SR as f64);
        let mut s = Smoother::new(0.0);
        s.set_target(1.0);
        let mut prev = s.value;
        let mut max_delta = 0.0f64;
        for _ in 0..BLOCK_FRAMES {
            let v = s.step(coeff);
            max_delta = max_delta.max((v - prev).abs());
            prev = v;
        }
        // A hard step (no smoothing) would jump the full 1.0 in one sample.
        // The de-zipper must never take a step anywhere near that size.
        assert!(
            max_delta < 0.05,
            "max per-sample delta too large: {max_delta}"
        );
        // 512 samples @ 48kHz with a 10ms one-pole time constant is ~1.07
        // time constants, so textbook exponential convergence puts it at
        // ~1 - exp(-1.07) ≈ 65% of the way there — not "nearly all the way"
        // (that needs ~5 time constants). Assert the expected ballpark
        // instead of an incorrect near-100% expectation.
        assert!(
            (0.55..0.75).contains(&prev),
            "smoother should be ~65% of the way to target after one block, got {prev}"
        );
    }

    #[test]
    fn de_zipper_reaches_target_and_holds() {
        let coeff = one_pole_coeff(GAIN_PAN_TAU_MS, SR as f64);
        let mut s = Smoother::new(-10.0);
        s.set_target(0.0);
        // ~100 time constants' worth of samples — residual error is
        // astronomically below any meaningful tolerance.
        for _ in 0..(BLOCK_FRAMES * 100) {
            s.step(coeff);
        }
        assert!((s.value - 0.0).abs() < 1e-6);
    }

    // ── Determinism (09 §7) ─────────────────────────────────────────────

    #[test]
    fn render_is_bit_identical_across_runs() {
        fn render_n_blocks(n: usize) -> Vec<f32> {
            let mut mixer = Mixer::new(SR);
            let master = master_bus_no_limiter();
            let mut audio = TrackAudio::new();
            audio.params.base.volume_db = -6.0;
            audio.params.base.pan = 0.3;
            let mut clip_audio = ClipAudio::new();
            clip_audio.fade_in = Some(AudioFade {
                duration: Tick::from_seconds(1),
                shape: FadeShape::EqualPower,
            });
            let mut all = Vec::new();
            for i in 0..n {
                let mut src = crate::audio::test_support::SineSource {
                    channels: 2,
                    sample_rate: SR,
                    freq_hz: 440.0,
                    amplitude: 0.5,
                    phase: 0.0,
                };
                let mut tracks = vec![TrackVoice {
                    id: track_id(1),
                    audio: &audio,
                    clips: vec![ClipVoice {
                        audio: &clip_audio,
                        elapsed: Tick(
                            i as i64 * BLOCK_FRAMES as i64 * (TICKS_PER_SECOND / SR as i64),
                        ),
                        remaining: Tick::from_seconds(3600),
                        source: &mut src,
                    }],
                }];
                let mut out = vec![0.0; BLOCK_FRAMES * CHANNELS];
                mixer.render_block(
                    Tick(i as i64 * BLOCK_FRAMES as i64 * (TICKS_PER_SECOND / SR as i64)),
                    &mut tracks,
                    &master,
                    &mut out,
                );
                all.extend_from_slice(&out);
            }
            all
        }

        let run1 = render_n_blocks(16);
        let run2 = render_n_blocks(16);
        assert_eq!(
            run1, run2,
            "same project + call sequence must be bit-identical"
        );
    }

    // ── Ring/xrun interplay is covered in ring.rs; waveform pyramid values
    //    are covered in waveform.rs. ──────────────────────────────────────

    #[test]
    fn silence_source_produces_silent_output() {
        let mut mixer = Mixer::new(SR);
        let master = master_bus_no_limiter();
        let audio = TrackAudio::new();
        let clip_audio = ClipAudio::new();
        let mut src = SilenceSource {
            channels: 2,
            sample_rate: SR,
        };
        let mut tracks = vec![TrackVoice {
            id: track_id(1),
            audio: &audio,
            clips: vec![silent_clip_voice(&clip_audio, &mut src)],
        }];
        let mut out = vec![-1.0; BLOCK_FRAMES * CHANNELS];
        mixer.render_block(Tick(0), &mut tracks, &master, &mut out);
        assert!(out.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn channel_map_swap_swaps_stereo_channels() {
        // A stereo frame with a distinct L/R value; ChannelSwap must swap
        // which side each ends up on, AsSource/StereoLR must not.
        let buf = [1.0f32, -1.0];
        assert_eq!(
            read_channel_mapped(&buf, 2, 0, ChannelMap::ChannelSwap),
            (-1.0, 1.0)
        );
        assert_eq!(
            read_channel_mapped(&buf, 2, 0, ChannelMap::AsSource),
            (1.0, -1.0)
        );
        assert_eq!(
            read_channel_mapped(&buf, 2, 0, ChannelMap::StereoLR),
            (1.0, -1.0)
        );
    }

    #[test]
    fn channel_map_mono_downmix_averages_stereo() {
        let buf = [1.0f32, -1.0];
        assert_eq!(
            read_channel_mapped(&buf, 2, 0, ChannelMap::MonoDownmix),
            (0.0, 0.0)
        );
    }

    #[test]
    fn mono_source_duplicates_to_both_channels_regardless_of_map() {
        let buf = [0.42f32];
        for map in [
            ChannelMap::AsSource,
            ChannelMap::MonoDownmix,
            ChannelMap::StereoLR,
            ChannelMap::ChannelSwap,
        ] {
            let (l, r) = read_channel_mapped(&buf, 1, 0, map);
            assert_eq!((l, r), (0.42, 0.42), "{map:?} must duplicate mono at unity");
        }
    }

    // ── Param eval helpers ──────────────────────────────────────────────

    #[test]
    fn eval_track_audio_params_static_returns_base() {
        let mut anim = AnimProps::new(TrackAudioParams {
            volume_db: -6.0,
            pan: 0.25,
        });
        let got = eval_track_audio_params(&anim, Tick(1000));
        assert_eq!(got.volume_db, -6.0);
        assert_eq!(got.pan, 0.25);

        // Adding a keyframe makes it non-static and drives the evaluated path.
        // The segment's interpolation is governed by its *starting* keyframe
        // (photonic_core::timeline::anim::eval's documented rule) — Linear
        // goes on the keyframe at Tick(0) to get a linear ramp into Tick(100).
        anim.track_mut(&PropPath::new("params.volume_db"))
            .insert_keyframe(Keyframe::new(
                Tick(0),
                PropValue::Float(-96.0),
                Interp::Linear,
            ));
        anim.track_mut(&PropPath::new("params.volume_db"))
            .insert_keyframe(Keyframe::new(
                Tick(100),
                PropValue::Float(0.0),
                Interp::Hold,
            ));
        let got = eval_track_audio_params(&anim, Tick(50));
        assert_eq!(got.volume_db, -48.0);
    }

    // Test helper: a `MasterBus` without the default limiter so P3 signal-flow
    // tests aren't coupled to a `fx_chain` entry that's inert until P8 anyway.
    fn master_bus_no_limiter() -> MasterBus {
        let mut mb = MasterBus::new();
        mb.fx_chain.clear();
        mb
    }

    // ── 31 §8 step 2: instantiated fx chains ────────────────────────────

    /// (a) `sync` rebuilds only from the first divergence and reports that
    /// index; an unchanged chain does not rebuild.
    #[test]
    fn fx_chain_sync_rebuilds_from_first_change() {
        let mut state = FxChainState::new();

        let spec1 = vec![AudioFxUnit::new(AudioFxKind::Gate)];
        assert_eq!(state.sync(&spec1), Some(0), "empty -> [Gate] builds at 0");
        assert_eq!(state.rebuild_count, 1);

        assert_eq!(state.sync(&spec1), None, "identical chain -> no rebuild");
        assert_eq!(state.rebuild_count, 1);

        let spec2 = vec![
            AudioFxUnit::new(AudioFxKind::Gate),
            AudioFxUnit::new(AudioFxKind::Compressor),
        ];
        assert_eq!(state.sync(&spec2), Some(1), "append -> first change at 1");
        assert_eq!(state.units.len(), 2);
        assert_eq!(state.units[0].kind(), AudioFxKind::Gate, "prefix preserved");
        assert_eq!(state.units[1].kind(), AudioFxKind::Compressor);
    }

    /// (a, via render_block) a track whose `fx_chain` gains a unit between two
    /// `render_block` calls rebuilds.
    #[test]
    fn render_block_rebuilds_track_fx_on_chain_change() {
        let mut mixer = Mixer::new(SR);
        let master = master_bus_no_limiter();
        let id = track_id(1);
        let clip_audio = ClipAudio::new();
        let mut out = vec![0.0; BLOCK_FRAMES * CHANNELS];

        let mut audio1 = TrackAudio::new();
        audio1.fx_chain = vec![AudioFxUnit::new(AudioFxKind::Gate)];
        let mut src = StepSource {
            channels: 2,
            sample_rate: SR,
            value: 0.1,
        };
        let mut tracks = vec![TrackVoice {
            id,
            audio: &audio1,
            clips: vec![silent_clip_voice(&clip_audio, &mut src)],
        }];
        mixer.render_block(Tick(0), &mut tracks, &master, &mut out);
        assert_eq!(mixer.track_fx[&id].rebuild_count, 1);

        let mut audio2 = TrackAudio::new();
        audio2.fx_chain = vec![
            AudioFxUnit::new(AudioFxKind::Gate),
            AudioFxUnit::new(AudioFxKind::Compressor),
        ];
        let mut src2 = StepSource {
            channels: 2,
            sample_rate: SR,
            value: 0.1,
        };
        let mut tracks2 = vec![TrackVoice {
            id,
            audio: &audio2,
            clips: vec![silent_clip_voice(&clip_audio, &mut src2)],
        }];
        mixer.render_block(Tick(0), &mut tracks2, &master, &mut out);
        assert_eq!(
            mixer.track_fx[&id].rebuild_count, 2,
            "chain change -> rebuild"
        );
        assert_eq!(mixer.track_fx[&id].units.len(), 2);
    }

    /// (b) an unchanged chain across 100 blocks performs zero rebuilds beyond
    /// the initial build.
    #[test]
    fn unchanged_chain_performs_zero_rebuilds() {
        let mut mixer = Mixer::new(SR);
        let master = master_bus_no_limiter();
        let id = track_id(1);
        let mut audio = TrackAudio::new();
        audio.fx_chain = vec![
            AudioFxUnit::new(AudioFxKind::Gate),
            AudioFxUnit::new(AudioFxKind::Compressor),
        ];
        let clip_audio = ClipAudio::new();
        let mut out = vec![0.0; BLOCK_FRAMES * CHANNELS];
        for _ in 0..100 {
            let mut src = StepSource {
                channels: 2,
                sample_rate: SR,
                value: 0.1,
            };
            let mut tracks = vec![TrackVoice {
                id,
                audio: &audio,
                clips: vec![silent_clip_voice(&clip_audio, &mut src)],
            }];
            mixer.render_block(Tick(0), &mut tracks, &master, &mut out);
        }
        assert_eq!(
            mixer.track_fx[&id].rebuild_count, 1,
            "one build, no per-block rebuilds"
        );
    }

    /// (c) a disabled unit contributes 0 to `latency_samples` but keeps its
    /// slot index (and its state) across the enable-flag flip.
    #[test]
    fn disabled_unit_contributes_zero_latency_but_keeps_slot() {
        let mut state = FxChainState::new();
        let spec_both = vec![
            AudioFxUnit::new(AudioFxKind::Limiter),
            AudioFxUnit::new(AudioFxKind::Limiter),
        ];
        state.sync(&spec_both);
        let mut block = vec![0.1f32; BLOCK_FRAMES * CHANNELS];
        state.process_block(&spec_both, SR, &mut block);
        let l = state.units[1].latency_samples();
        assert!(l > 0, "limiter reports lookahead after a block");
        assert_eq!(state.latency_samples(&spec_both), 2 * l);

        // Disabling is not a kind change: no rebuild, slot + state preserved,
        // but the unit drops out of the latency sum.
        let mut spec_dis = spec_both.clone();
        spec_dis[1].enabled = false;
        assert_eq!(state.sync(&spec_dis), None, "enable flip is not a rebuild");
        assert_eq!(state.units.len(), 2, "disabled unit keeps its slot");
        assert!(state.units[1].latency_samples() > 0, "state preserved");
        assert_eq!(
            state.latency_samples(&spec_dis),
            l,
            "disabled unit adds 0 to the latency sum"
        );
    }

    // ── 31 §8 step 4: segment-boundary tail plumbing ────────────────────

    fn ending_clip_voice<'a>(audio: &'a ClipAudio, source: &'a mut dyn PcmSource) -> ClipVoice<'a> {
        let tick_per_sample = TICKS_PER_SECOND / SR as i64;
        ClipVoice {
            audio,
            elapsed: Tick(0),
            // Ends within this block -> the mixer marks a boundary.
            remaining: Tick(BLOCK_FRAMES as i64 * tick_per_sample),
            source,
        }
    }

    /// (a) the captured tail's newest samples equal the (pre-fx) clip output;
    /// (b) a second take for the same boundary returns `None`.
    #[test]
    fn segment_boundary_tail_matches_clip_and_consumes_once() {
        let mut mixer = Mixer::new(SR);
        let master = master_bus_no_limiter();
        let id = track_id(1);
        let audio = TrackAudio::new();
        let clip_audio = ClipAudio::new();
        let mut src = StepSource {
            channels: 2,
            sample_rate: SR,
            value: 0.5,
        };
        let mut tracks = vec![TrackVoice {
            id,
            audio: &audio,
            clips: vec![ending_clip_voice(&clip_audio, &mut src)],
        }];
        let mut out = vec![0.0; BLOCK_FRAMES * CHANNELS];
        mixer.render_block(Tick(0), &mut tracks, &master, &mut out);

        let boundary = mixer
            .take_segment_boundary(id)
            .expect("a clip ended this block -> boundary pending");
        assert_eq!(boundary.track, id);
        for c in 0..CHANNELS {
            let tail = &boundary.tail[c];
            assert_eq!(tail.len(), DECLICK_WINDOW_SAMPLES);
            // Unity clip gain, no fades: the pre-fx track bus is a constant 0.5.
            // The newest written samples sit at the end of the chronological
            // snapshot.
            for &s in &tail[tail.len() - 256..] {
                assert!((s - 0.5).abs() < 1e-6, "newest tail sample {s} != 0.5");
            }
        }

        assert!(
            mixer.take_segment_boundary(id).is_none(),
            "a boundary is handed out exactly once"
        );
    }

    /// (c) `invalidate_tail` drops the cached tail.
    #[test]
    fn invalidate_tail_drops_pending_boundary() {
        let mut mixer = Mixer::new(SR);
        let master = master_bus_no_limiter();
        let id = track_id(1);
        let audio = TrackAudio::new();
        let clip_audio = ClipAudio::new();
        let mut src = StepSource {
            channels: 2,
            sample_rate: SR,
            value: 0.5,
        };
        let mut tracks = vec![TrackVoice {
            id,
            audio: &audio,
            clips: vec![ending_clip_voice(&clip_audio, &mut src)],
        }];
        let mut out = vec![0.0; BLOCK_FRAMES * CHANNELS];
        mixer.render_block(Tick(0), &mut tracks, &master, &mut out);

        mixer.invalidate_tail(id);
        assert!(
            mixer.take_segment_boundary(id).is_none(),
            "invalidated tail yields no boundary"
        );
    }

    /// (d) adding a Limiter to the master chain grows `tail_len` to at least
    /// its `tail_samples()`.
    #[test]
    fn master_limiter_grows_tail_len_frames() {
        let mut mixer = Mixer::new(SR);
        let master = MasterBus::new(); // seeded with the default Limiter
        let id = track_id(1);
        let audio = TrackAudio::new();
        let clip_audio = ClipAudio::new();
        let mut out = vec![0.0; BLOCK_FRAMES * CHANNELS];
        // Block 1 primes the limiter (tail_samples 0 -> lookahead); a later
        // block sees the grown downstream tail.
        for _ in 0..3 {
            let mut src = StepSource {
                channels: 2,
                sample_rate: SR,
                value: 0.1,
            };
            let mut tracks = vec![TrackVoice {
                id,
                audio: &audio,
                clips: vec![silent_clip_voice(&clip_audio, &mut src)],
            }];
            mixer.render_block(Tick(0), &mut tracks, &master, &mut out);
        }
        let limiter_tail = mixer.master_fx.tail_samples(&master.fx_chain);
        assert!(limiter_tail > 0, "primed master limiter reports a tail");
        let ring_len = mixer.tail_cache[&id].len;
        assert!(
            ring_len >= limiter_tail as usize,
            "ring ({ring_len}) covers downstream tail ({limiter_tail})"
        );
        assert_eq!(
            ring_len,
            DECLICK_WINDOW_SAMPLES.max(limiter_tail as usize),
            "ring is max(declick window, downstream tail)"
        );
    }

    /// (e) `Seek` leaves the tail buffer length unchanged but silent.
    #[test]
    fn seek_reprimes_tail_ring_but_keeps_length() {
        let mut mixer = Mixer::new(SR);
        let master = master_bus_no_limiter();
        let id = track_id(1);
        let audio = TrackAudio::new();
        let clip_audio = ClipAudio::new();
        let mut out = vec![0.0; BLOCK_FRAMES * CHANNELS];
        for _ in 0..3 {
            let mut src = StepSource {
                channels: 2,
                sample_rate: SR,
                value: 0.5,
            };
            let mut tracks = vec![TrackVoice {
                id,
                audio: &audio,
                clips: vec![silent_clip_voice(&clip_audio, &mut src)],
            }];
            mixer.render_block(Tick(0), &mut tracks, &master, &mut out);
        }
        let len_before = mixer.tail_cache[&id].len;
        assert!(len_before > 0);

        mixer.notify_discontinuity(MixerDiscontinuity::Seek { to: Tick(0) });

        let buf = &mixer.tail_cache[&id];
        assert_eq!(buf.len, len_before, "length preserved across Seek");
        for c in 0..CHANNELS {
            assert!(
                buf.ch[c].iter().all(|&s| s == 0.0),
                "ring re-primed with silence"
            );
        }
    }

    // ── 31 §3 latency compensation ───────────────────────────────────────

    /// Graph latency equals max track-path latency + master chain latency.
    #[test]
    fn graph_latency_sums_max_track_and_master() {
        let mut mixer = Mixer::new(SR);
        let mut master = MasterBus::new();
        master.fx_chain = vec![AudioFxUnit::new(AudioFxKind::Limiter)];
        let id = track_id(1);
        let audio = TrackAudio::new();
        // Dry track (0 latency) + master limiter (lookahead latency).
        let clip_audio = ClipAudio::new();
        let mut out = vec![0.0; BLOCK_FRAMES * CHANNELS];
        // Two blocks: first allocates limiter lookahead, second reports it.
        for _ in 0..2 {
            let mut src = StepSource {
                channels: 2,
                sample_rate: SR,
                value: 0.1,
            };
            let mut tracks = vec![TrackVoice {
                id,
                audio: &audio,
                clips: vec![silent_clip_voice(&clip_audio, &mut src)],
            }];
            mixer.render_block(Tick(0), &mut tracks, &master, &mut out);
        }
        let master_lat = mixer.master_fx.latency_samples(&master.fx_chain);
        assert!(master_lat > 0, "limiter must declare lookahead latency");
        assert_eq!(
            mixer.last_graph_latency_samples(),
            master_lat,
            "dry tracks ⇒ graph latency = master latency alone"
        );
    }

    /// A dry track delayed by a lookahead unit's latency stays sample-aligned
    /// with a wet track that carries the same unit (compensation equalises them).
    #[test]
    fn latency_compensation_equalises_wet_and_dry_tracks() {
        let mut mixer = Mixer::new(SR);
        let master = master_bus_no_limiter();
        let wet_id = track_id(1);
        let dry_id = track_id(2);
        let mut wet_audio = TrackAudio::new();
        wet_audio.fx_chain = vec![AudioFxUnit::new(AudioFxKind::Limiter)];
        let dry_audio = TrackAudio::new();
        let clip_audio = ClipAudio::new();
        let mut out = vec![0.0; BLOCK_FRAMES * CHANNELS];
        // Drive several blocks so delay lines settle.
        for _ in 0..4 {
            let mut wet_src = StepSource {
                channels: 2,
                sample_rate: SR,
                value: 0.5,
            };
            let mut dry_src = StepSource {
                channels: 2,
                sample_rate: SR,
                value: 0.5,
            };
            let mut tracks = vec![
                TrackVoice {
                    id: wet_id,
                    audio: &wet_audio,
                    clips: vec![silent_clip_voice(&clip_audio, &mut wet_src)],
                },
                TrackVoice {
                    id: dry_id,
                    audio: &dry_audio,
                    clips: vec![silent_clip_voice(&clip_audio, &mut dry_src)],
                },
            ];
            mixer.render_block(Tick(0), &mut tracks, &master, &mut out);
        }
        let wet_lat = mixer
            .track_fx
            .get(&wet_id)
            .map(|fx| fx.latency_samples(&wet_audio.fx_chain))
            .unwrap_or(0);
        assert!(wet_lat > 0);
        assert_eq!(
            mixer.last_graph_latency_samples(),
            wet_lat,
            "max track path = wet latency (master dry)"
        );
        // Dry track must hold a delay of exactly wet_lat frames.
        let dry_delay = mixer.track_delay.get(&dry_id).map(|d| d.delay_frames);
        assert_eq!(dry_delay, Some(wet_lat as usize));
        let wet_delay = mixer.track_delay.get(&wet_id).map(|d| d.delay_frames);
        assert_eq!(wet_delay, Some(0));
    }
}
