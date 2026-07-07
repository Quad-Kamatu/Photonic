//! Generic keyframe animation (01 §6).
//!
//! One system animates everything: clip transforms, effect params, grade-op
//! params, audio automation, and (eventually) vector-node properties. A target
//! struct `T: PropSet` carries a static `base` value plus a list of
//! [`PropertyTrack`]s, each keyframing one [`PropPath`] over clip-relative
//! [`Tick`]s. Evaluation ([`eval`]) is a pure, closed-form function with no I/O
//! — unit-tested against hand-computed expectations (CAP-007).

use super::time::Tick;
use crate::Color;
use serde::{Deserialize, Serialize};

/// A dotted property path addressing one animatable field of a target kind,
/// e.g. `"transform.x"`, `"params.blur_radius"`, `"params.slope[0]"`. Validated
/// against the static registry in [`super::prop_registry`]; an unknown path on
/// load keeps the track but flags it [`PropertyTrack::orphaned`] (01 §6.2).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PropPath(pub String);

impl PropPath {
    pub fn new(s: impl Into<String>) -> Self {
        PropPath(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for PropPath {
    fn from(s: &str) -> Self {
        PropPath(s.to_string())
    }
}

/// The kind of value a property holds. `Bool`/`Enum` do not interpolate — they
/// step (Hold) regardless of a keyframe's declared [`Interp`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropValueKind {
    Float,
    Vec2,
    Color,
    Bool,
    Enum,
}

/// A concrete animatable value.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", content = "v", rename_all = "snake_case")]
pub enum PropValue {
    Float(f64),
    Vec2([f64; 2]),
    Color(Color),
    Bool(bool),
    Enum(u32),
}

impl PropValue {
    pub fn kind(&self) -> PropValueKind {
        match self {
            PropValue::Float(_) => PropValueKind::Float,
            PropValue::Vec2(_) => PropValueKind::Vec2,
            PropValue::Color(_) => PropValueKind::Color,
            PropValue::Bool(_) => PropValueKind::Bool,
            PropValue::Enum(_) => PropValueKind::Enum,
        }
    }
}

/// How a keyframe's value blends into the next keyframe over the segment that
/// starts at this keyframe.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Interp {
    /// Step: hold this keyframe's value until the next keyframe.
    Hold,
    /// Straight-line blend to the next keyframe.
    Linear,
    /// Normalized cubic-bezier ease (CSS-like): control handles in [0,1]² over
    /// the segment's unit `(time, value)` box, endpoints fixed at (0,0)/(1,1).
    Bezier {
        out_handle: [f64; 2],
        in_handle: [f64; 2],
    },
}

/// One keyframe: a clip-relative time, a value, and the interpolation that
/// governs the segment leaving it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    pub at: Tick,
    pub value: PropValue,
    pub interp: Interp,
}

impl Keyframe {
    pub fn new(at: Tick, value: PropValue, interp: Interp) -> Self {
        Keyframe { at, value, interp }
    }
}

/// A keyframe lane for one property. Keyframes are kept sorted by `at` with
/// unique times (enforced by the edit ops / [`Self::insert_keyframe`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PropertyTrack {
    pub property: PropPath,
    pub keyframes: Vec<Keyframe>,
    /// Set when [`property`](Self::property) does not resolve in the registry
    /// for the owning target kind (01 §6.2). The track is retained (an asset or
    /// plugin may re-register the path later) and skipped by evaluation, never
    /// dropped silently.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub orphaned: bool,
}

impl PropertyTrack {
    pub fn new(property: impl Into<PropPath>) -> Self {
        PropertyTrack {
            property: property.into(),
            keyframes: Vec::new(),
            orphaned: false,
        }
    }

    /// Insert or replace a keyframe, keeping the lane sorted with unique `at`.
    pub fn insert_keyframe(&mut self, kf: Keyframe) {
        match self.keyframes.binary_search_by(|k| k.at.cmp(&kf.at)) {
            Ok(i) => self.keyframes[i] = kf,
            Err(i) => self.keyframes.insert(i, kf),
        }
    }

    /// Remove the keyframe at exactly `at`, returning it if present.
    pub fn remove_keyframe(&mut self, at: Tick) -> Option<Keyframe> {
        match self.keyframes.binary_search_by(|k| k.at.cmp(&at)) {
            Ok(i) => Some(self.keyframes.remove(i)),
            Err(_) => None,
        }
    }
}

/// A target of the animation system: a plain struct of animatable fields plus
/// its keyframe lanes. `T` supplies the static base values; the tracks override
/// per-property across time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnimProps<T: PropSet> {
    pub base: T,
    #[serde(default = "Vec::new", skip_serializing_if = "Vec::is_empty")]
    pub tracks: Vec<PropertyTrack>,
}

impl<T: PropSet> AnimProps<T> {
    pub fn new(base: T) -> Self {
        AnimProps {
            base,
            tracks: Vec::new(),
        }
    }

    /// The lane for `path`, if one exists.
    pub fn track(&self, path: &PropPath) -> Option<&PropertyTrack> {
        self.tracks.iter().find(|t| &t.property == path)
    }

    /// The lane for `path`, creating an empty one if absent.
    pub fn track_mut(&mut self, path: &PropPath) -> &mut PropertyTrack {
        if let Some(i) = self.tracks.iter().position(|t| &t.property == path) {
            &mut self.tracks[i]
        } else {
            self.tracks.push(PropertyTrack::new(path.clone()));
            self.tracks.last_mut().unwrap()
        }
    }

    /// True when no property is keyframed — the common case, letting callers
    /// skip animation eval entirely.
    pub fn is_static(&self) -> bool {
        self.tracks.iter().all(|t| t.keyframes.is_empty())
    }
}

/// Marker trait for structs that back an [`AnimProps`]. Ties the target to its
/// registry block ([`super::prop_registry`]) via [`TARGET_KIND`](Self::TARGET_KIND).
pub trait PropSet: Clone + std::fmt::Debug + PartialEq {
    /// The registry key under which this set's `(path, kind, range)` entries
    /// are published (01 §6.2).
    const TARGET_KIND: super::prop_registry::PropTargetKind;
}

/// Evaluate one property lane at clip-relative time `t` (pure, closed-form).
///
/// - Empty lane → `base`.
/// - `t` at/before the first keyframe → the first keyframe's value.
/// - `t` at/after the last keyframe → the last keyframe's value.
/// - Otherwise interpolate the bracketing segment per the *starting* keyframe's
///   [`Interp`]. `Bool`/`Enum` values always Hold, regardless of `interp`.
pub fn eval(track: &PropertyTrack, base: &PropValue, t: Tick) -> PropValue {
    let kfs = &track.keyframes;
    if track.orphaned || kfs.is_empty() {
        return *base;
    }
    if t <= kfs[0].at {
        return kfs[0].value;
    }
    let last = kfs.last().unwrap();
    if t >= last.at {
        return last.value;
    }
    // Find the segment [k0, k1] with k0.at <= t < k1.at.
    let i = match kfs.binary_search_by(|k| k.at.cmp(&t)) {
        Ok(exact) => return kfs[exact].value,
        Err(next) => next - 1, // `next` is the first kf with at > t
    };
    let k0 = &kfs[i];
    let k1 = &kfs[i + 1];

    // Non-interpolable value kinds step regardless of declared interp.
    if matches!(k0.value, PropValue::Bool(_) | PropValue::Enum(_)) {
        return k0.value;
    }

    let span = (k1.at.0 - k0.at.0) as f64;
    let u = if span <= 0.0 {
        0.0
    } else {
        (t.0 - k0.at.0) as f64 / span
    };

    match k0.interp {
        Interp::Hold => k0.value,
        Interp::Linear => lerp_value(&k0.value, &k1.value, u),
        Interp::Bezier {
            out_handle,
            in_handle,
        } => {
            let eased = cubic_bezier_ease(out_handle, in_handle, u);
            lerp_value(&k0.value, &k1.value, eased)
        }
    }
}

/// Linear blend between two values by `f` in [0,1]. Mismatched variants or
/// non-interpolable kinds hold `a`.
fn lerp_value(a: &PropValue, b: &PropValue, f: f64) -> PropValue {
    let l = |x: f64, y: f64| x + (y - x) * f;
    match (a, b) {
        (PropValue::Float(x), PropValue::Float(y)) => PropValue::Float(l(*x, *y)),
        (PropValue::Vec2(x), PropValue::Vec2(y)) => PropValue::Vec2([l(x[0], y[0]), l(x[1], y[1])]),
        (PropValue::Color(x), PropValue::Color(y)) => PropValue::Color(Color {
            r: l(x.r as f64, y.r as f64) as f32,
            g: l(x.g as f64, y.g as f64) as f32,
            b: l(x.b as f64, y.b as f64) as f32,
            a: l(x.a as f64, y.a as f64) as f32,
        }),
        // Bool/Enum or mismatched variants: hold the start value.
        _ => *a,
    }
}

/// Evaluate a CSS-style cubic-bezier easing `(p1, p2)` at normalized time `u`,
/// returning the eased value in [0,1]. Endpoints are fixed at (0,0)/(1,1); the
/// curve is solved for the parameter `s` where `bezier_x(s) == u`, then
/// `bezier_y(s)` is returned. Uses Newton's method with a bisection fallback,
/// the standard approach browsers use for `transition-timing-function`.
pub fn cubic_bezier_ease(p1: [f64; 2], p2: [f64; 2], u: f64) -> f64 {
    let u = u.clamp(0.0, 1.0);
    let (x1, y1, x2, y2) = (p1[0], p1[1], p2[0], p2[1]);
    // Bezier basis on one axis with control points (0, c1, c2, 1).
    let bez = |c1: f64, c2: f64, s: f64| {
        let mt = 1.0 - s;
        3.0 * mt * mt * s * c1 + 3.0 * mt * s * s * c2 + s * s * s
    };
    let dbez = |c1: f64, c2: f64, s: f64| {
        let mt = 1.0 - s;
        3.0 * mt * mt * c1 + 6.0 * mt * s * (c2 - c1) + 3.0 * s * s * (1.0 - c2)
    };
    // Solve bezier_x(s) = u for s.
    let mut s = u; // good initial guess
    for _ in 0..8 {
        let x = bez(x1, x2, s) - u;
        if x.abs() < 1e-9 {
            break;
        }
        let dx = dbez(x1, x2, s);
        if dx.abs() < 1e-9 {
            break;
        }
        s -= x / dx;
    }
    // Bisection fallback if Newton wandered out of range or stalled.
    if !(0.0..=1.0).contains(&s) || (bez(x1, x2, s) - u).abs() > 1e-6 {
        let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
        s = u;
        for _ in 0..32 {
            s = 0.5 * (lo + hi);
            let x = bez(x1, x2, s);
            if (x - u).abs() < 1e-9 {
                break;
            }
            if x < u {
                lo = s;
            } else {
                hi = s;
            }
        }
    }
    bez(y1, y2, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(kfs: Vec<Keyframe>) -> PropertyTrack {
        PropertyTrack {
            property: PropPath::new("params.x"),
            keyframes: kfs,
            orphaned: false,
        }
    }

    #[test]
    fn empty_track_returns_base() {
        let t = track(vec![]);
        let base = PropValue::Float(3.0);
        assert_eq!(eval(&t, &base, Tick(100)), base);
    }

    #[test]
    fn before_first_and_after_last_clamp() {
        let t = track(vec![
            Keyframe::new(Tick(100), PropValue::Float(0.0), Interp::Linear),
            Keyframe::new(Tick(200), PropValue::Float(10.0), Interp::Linear),
        ]);
        let base = PropValue::Float(-1.0);
        assert_eq!(eval(&t, &base, Tick(0)), PropValue::Float(0.0));
        assert_eq!(eval(&t, &base, Tick(500)), PropValue::Float(10.0));
    }

    #[test]
    fn linear_midpoint_is_closed_form() {
        let t = track(vec![
            Keyframe::new(Tick(0), PropValue::Float(0.0), Interp::Linear),
            Keyframe::new(Tick(100), PropValue::Float(10.0), Interp::Linear),
        ]);
        let base = PropValue::Float(0.0);
        assert_eq!(eval(&t, &base, Tick(50)), PropValue::Float(5.0));
        assert_eq!(eval(&t, &base, Tick(25)), PropValue::Float(2.5));
    }

    #[test]
    fn hold_steps_to_next() {
        let t = track(vec![
            Keyframe::new(Tick(0), PropValue::Float(0.0), Interp::Hold),
            Keyframe::new(Tick(100), PropValue::Float(10.0), Interp::Linear),
        ]);
        let base = PropValue::Float(0.0);
        assert_eq!(eval(&t, &base, Tick(99)), PropValue::Float(0.0));
        assert_eq!(eval(&t, &base, Tick(100)), PropValue::Float(10.0));
    }

    #[test]
    fn vec2_and_color_interpolate_componentwise() {
        let t = track(vec![
            Keyframe::new(Tick(0), PropValue::Vec2([0.0, 10.0]), Interp::Linear),
            Keyframe::new(Tick(100), PropValue::Vec2([10.0, 0.0]), Interp::Linear),
        ]);
        let base = PropValue::Vec2([0.0, 0.0]);
        assert_eq!(eval(&t, &base, Tick(50)), PropValue::Vec2([5.0, 5.0]));
    }

    #[test]
    fn bool_and_enum_hold_regardless_of_interp() {
        let t = track(vec![
            Keyframe::new(Tick(0), PropValue::Bool(false), Interp::Linear),
            Keyframe::new(Tick(100), PropValue::Bool(true), Interp::Linear),
        ]);
        let base = PropValue::Bool(false);
        assert_eq!(eval(&t, &base, Tick(50)), PropValue::Bool(false));
        assert_eq!(eval(&t, &base, Tick(100)), PropValue::Bool(true));
    }

    #[test]
    fn linear_bezier_matches_linear() {
        // cubic-bezier(0,0,1,1) is the identity easing → equals Linear exactly.
        let bez = Interp::Bezier {
            out_handle: [0.0, 0.0],
            in_handle: [1.0, 1.0],
        };
        let t = track(vec![
            Keyframe::new(Tick(0), PropValue::Float(0.0), bez),
            Keyframe::new(Tick(100), PropValue::Float(100.0), Interp::Linear),
        ]);
        let base = PropValue::Float(0.0);
        // ease(0.5) == 0.5 for the identity curve.
        match eval(&t, &base, Tick(50)) {
            PropValue::Float(v) => assert!((v - 50.0).abs() < 1e-6, "got {v}"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn ease_in_out_is_symmetric_and_monotone() {
        // Classic ease-in-out cubic-bezier(0.42, 0, 0.58, 1).
        let p1 = [0.42, 0.0];
        let p2 = [0.58, 1.0];
        assert!((cubic_bezier_ease(p1, p2, 0.0) - 0.0).abs() < 1e-6);
        assert!((cubic_bezier_ease(p1, p2, 1.0) - 1.0).abs() < 1e-6);
        // Symmetric about the midpoint: ease(0.5) == 0.5.
        assert!((cubic_bezier_ease(p1, p2, 0.5) - 0.5).abs() < 1e-6);
        // Monotonically increasing.
        let mut prev = -1.0;
        for i in 0..=20 {
            let v = cubic_bezier_ease(p1, p2, i as f64 / 20.0);
            assert!(v >= prev - 1e-9, "not monotone at {i}: {v} < {prev}");
            prev = v;
        }
    }

    #[test]
    fn insert_and_remove_keep_sorted_unique() {
        let mut tr = PropertyTrack::new("params.x");
        tr.insert_keyframe(Keyframe::new(
            Tick(100),
            PropValue::Float(1.0),
            Interp::Hold,
        ));
        tr.insert_keyframe(Keyframe::new(Tick(0), PropValue::Float(0.0), Interp::Hold));
        tr.insert_keyframe(Keyframe::new(Tick(50), PropValue::Float(0.5), Interp::Hold));
        // Replace at 50.
        tr.insert_keyframe(Keyframe::new(Tick(50), PropValue::Float(9.0), Interp::Hold));
        let ats: Vec<i64> = tr.keyframes.iter().map(|k| k.at.0).collect();
        assert_eq!(ats, vec![0, 50, 100]);
        assert_eq!(tr.keyframes[1].value, PropValue::Float(9.0));
        assert!(tr.remove_keyframe(Tick(50)).is_some());
        assert!(tr.remove_keyframe(Tick(50)).is_none());
        assert_eq!(tr.keyframes.len(), 2);
    }
}
