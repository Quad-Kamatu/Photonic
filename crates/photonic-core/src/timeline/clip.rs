//! Clips (01 §5): the atomic timeline elements.
//!
//! A clip positions a source in a sequence with trim, speed, an animatable
//! transform, an ordered effect stack, an optional grade and per-clip
//! composition, transitions, and audio. The composition (when set) substitutes
//! only the clip's *source* op; transform/effects/grade/reframe still apply on
//! top (02 §2 step 3).

use super::anim::{AnimProps, PropSet};
use super::audio::ClipAudio;
use super::captions::CaptionStyle;
use super::effect_kind::{EffectKind, EffectParams};
use super::grade::Grade;
use super::ids::{AssetId, ClipId, GraphId, SequenceId};
use super::prop_registry::PropTargetKind;
use super::time::Tick;
use crate::Color;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// A clip on a track.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: ClipId,
    /// Defaults to the asset name.
    pub name: String,
    /// Position in the sequence.
    pub start: Tick,
    pub duration: Tick,
    pub source: ClipSource,
    /// Offset into the source media (trim); 0 for generators.
    #[serde(default)]
    pub source_in: Tick,
    #[serde(default)]
    pub speed: SpeedMap,
    /// pos/scale/rotation/anchor/opacity (01 §6, animatable).
    pub transform: AnimProps<ClipTransform>,
    /// Per-`SequenceFormat` static override, keyed by format index (CAP-012).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub reframe: HashMap<usize, ClipTransform>,
    /// Ordered effect stack; each param animatable (01 §6.3).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<ClipEffect>,
    /// Color grade (07); stored here, evaluated as graph nodes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grade: Option<Grade>,
    /// Per-clip node graph (D-06); substitutes the clip's SOURCE op only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composition: Option<GraphId>,
    /// Overlaps the previous clip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition_in: Option<Transition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition_out: Option<Transition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<ClipAudio>,
    #[serde(default = "super::grade::default_true")]
    pub enabled: bool,
    /// Organizational color label — index into the GUI's fixed swatch
    /// palette (the palette itself is a GUI concern, out of scope here).
    /// `None` = unlabeled (14 §M-1, gap #7's data half).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_label: Option<u8>,
    /// Groups this clip with its linked partner(s) (e.g. an A/V pair split
    /// from one media import) so an editor can move them as a unit. `None` =
    /// unlinked (14 §M-2, gap #8's data half — the GUI move-together wiring
    /// is a later story).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_group: Option<LinkGroupId>,
}

impl Clip {
    /// A clip covering `[start, start+duration)` from `source`.
    pub fn new(source: ClipSource, start: Tick, duration: Tick) -> Self {
        Clip {
            id: ClipId::new(),
            name: String::new(),
            start,
            duration,
            source,
            source_in: Tick::ZERO,
            speed: SpeedMap::default(),
            transform: AnimProps::new(ClipTransform::default()),
            reframe: HashMap::new(),
            effects: Vec::new(),
            grade: None,
            composition: None,
            transition_in: None,
            transition_out: None,
            audio: None,
            enabled: true,
            color_label: None,
            link_group: None,
        }
    }

    /// End position (exclusive) in the sequence.
    #[inline]
    pub fn end(&self) -> Tick {
        self.start + self.duration
    }

    /// Whether this clip overlaps `[start, end)` on the timeline.
    pub fn overlaps(&self, start: Tick, end: Tick) -> bool {
        self.start < end && start < self.end()
    }
}

/// Identifies a link group — clips carrying the same id (e.g. a split A/V
/// pair) are meant to move together (14 gap #8). Defined here rather than
/// alongside the rest of the id newtypes in `ids.rs` since this story's
/// territory is limited to `clip.rs`/`sequence.rs`/`ops.rs`/`commands.rs`;
/// same derive/shape as the `id_newtype!` family there.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LinkGroupId(pub Uuid);

impl LinkGroupId {
    /// A fresh random (v4) id.
    #[inline]
    pub fn new() -> Self {
        LinkGroupId(Uuid::new_v4())
    }
}

impl Default for LinkGroupId {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

/// What a clip plays.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum ClipSource {
    /// Video/audio/image via the media pool.
    Asset {
        asset: AssetId,
    },
    /// `AssetKind::VectorDoc` — rasterized per frame (CAP-006/021).
    Vector {
        asset: AssetId,
    },
    /// Nested sequence (CAP-005); cycle-checked at edit time.
    NestedSequence {
        sequence: SequenceId,
    },
    SolidColor {
        color: Color,
    },
    /// Affects everything below (effects/grade apply to the composite).
    Adjustment,
    /// A title / text / graphics clip living on a video track (G-12). Carries
    /// styled text the engine renders through its text path (`TextGen`); no
    /// render logic lives here. Reuses the caption [`CaptionStyle`] cascade so
    /// titles and captions share one styling vocabulary.
    Text {
        content: TextClipContent,
    },
}

impl ClipSource {
    /// The asset this source references, if any (for relink/GC).
    pub fn asset(&self) -> Option<AssetId> {
        match self {
            ClipSource::Asset { asset } | ClipSource::Vector { asset } => Some(*asset),
            _ => None,
        }
    }
}

/// Styled text carried by a [`ClipSource::Text`] title / graphics clip (G-12).
/// Reuses the caption [`CaptionStyle`] type for font / size / fill / stroke /
/// background / position so titles and captions share one styling vocabulary;
/// the engine renders it (no render logic here). Serde-additive: `style`
/// defaults so older/hand-written text clips omitting it still load.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextClipContent {
    pub text: String,
    #[serde(default)]
    pub style: CaptionStyle,
}

impl TextClipContent {
    /// Text with the default caption style.
    pub fn new(text: impl Into<String>) -> Self {
        TextClipContent {
            text: text.into(),
            style: CaptionStyle::default(),
        }
    }
}

/// An exact rational speed factor. `num/den`; negative `num` = reverse.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Ratio {
    pub num: i32,
    pub den: u32,
}

impl Ratio {
    pub const ONE: Ratio = Ratio { num: 1, den: 1 };

    pub fn new(num: i32, den: u32) -> Ratio {
        Ratio {
            num,
            den: den.max(1),
        }
    }

    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }
}

/// One control point in a keyframed speed ramp (G-11): at clip-relative
/// timeline tick `at`, the exact-rational playback speed `ratio` takes effect.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SpeedKey {
    pub at: Tick,
    pub ratio: Ratio,
}

impl SpeedKey {
    #[inline]
    pub fn new(at: Tick, ratio: Ratio) -> Self {
        SpeedKey { at, ratio }
    }
}

/// Clip speed. `Constant` is the default and common case; `Keyframed` is a
/// variable-speed ramp (G-11). Serde-additive: the internal `speed` tag keeps
/// existing `constant` documents byte-identical, and `keyframed` is a new tag.
///
/// Ramp interpolation is currently piecewise-constant — each key's `ratio`
/// holds until the next key, and the first/last key's ratio holds before/after
/// the ramp — which integrates to exact integer source ticks. Eased (bezier)
/// segments are a later refinement layered on this data (the on-clip rubber
/// band is a GUI lane); the data shape here already carries the keys.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "speed", rename_all = "snake_case")]
pub enum SpeedMap {
    Constant(Ratio),
    Keyframed { keys: Vec<SpeedKey> },
}

impl Default for SpeedMap {
    fn default() -> Self {
        SpeedMap::Constant(Ratio::ONE)
    }
}

impl SpeedMap {
    /// Source-time delta consumed over the clip-relative interval `[0, dt]`
    /// (01 §5.1: exact rational arithmetic; `source = source_in + dt * speed`).
    /// Returns the source-time delta only (the caller adds `source_in`).
    ///
    /// - `Constant`: exact rational scale of `dt`.
    /// - `Keyframed`: the piecewise-constant speed integrated segment-by-segment
    ///   (∫₀ᵈᵗ speed). Handles negative `dt` (reverse trim) by symmetry and an
    ///   empty ramp as identity (1×). Exact integer arithmetic (i128 accumulate).
    pub fn source_delta(&self, dt: Tick) -> Tick {
        match self {
            SpeedMap::Constant(r) => Tick(scale_ticks(dt.0, *r)),
            SpeedMap::Keyframed { keys } => Tick(integrate_ramp(keys, dt.0)),
        }
    }
}

/// `len * ratio`, exact (multiply-before-divide in i128, saturating back to i64).
#[inline]
fn scale_ticks(len: i64, r: Ratio) -> i64 {
    ((len as i128 * r.num as i128) / r.den.max(1) as i128) as i64
}

/// Integrate a piecewise-constant speed ramp over `[0, target]` (or `[target, 0]`
/// when `target < 0`, negating the result). Each key's ratio holds until the
/// next; the first/last key's ratio holds before/after the ramp. An empty ramp
/// is identity (1×). Keys need not be pre-sorted.
fn integrate_ramp(keys: &[SpeedKey], target: i64) -> i64 {
    if keys.is_empty() {
        return target;
    }
    if target == 0 {
        return 0;
    }
    let mut sorted = keys.to_vec();
    sorted.sort_by_key(|k| k.at.0);
    let (lo, hi) = if target >= 0 {
        (0, target)
    } else {
        (target, 0)
    };
    let n = sorted.len();
    let mut acc: i128 = 0;
    for i in 0..n {
        // Segment `i` spans `[seg_start, seg_end)` at `sorted[i].ratio`.
        let seg_start = if i == 0 { i64::MIN } else { sorted[i].at.0 };
        let seg_end = if i + 1 < n {
            sorted[i + 1].at.0
        } else {
            i64::MAX
        };
        let a = seg_start.max(lo);
        let b = seg_end.min(hi);
        if b > a {
            let r = sorted[i].ratio;
            acc += (b - a) as i128 * r.num as i128 / r.den.max(1) as i128;
        }
    }
    if target < 0 {
        acc = -acc;
    }
    acc as i64
}

/// Animatable clip transform (01 §5/§6). Field names match the `prop_registry`
/// `transform.*` paths.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipTransform {
    pub x: f64,
    pub y: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    pub rotation: f64,
    pub anchor_x: f64,
    pub anchor_y: f64,
    pub opacity: f64,
}

impl Default for ClipTransform {
    fn default() -> Self {
        ClipTransform {
            x: 0.0,
            y: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            rotation: 0.0,
            anchor_x: 0.0,
            anchor_y: 0.0,
            opacity: 1.0,
        }
    }
}

impl PropSet for ClipTransform {
    const TARGET_KIND: PropTargetKind = PropTargetKind::ClipTransform;
}

/// One effect in a clip's ordered stack.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipEffect {
    pub kind: EffectKind,
    #[serde(default = "super::grade::default_true")]
    pub enabled: bool,
    pub params: AnimProps<EffectParams>,
}

impl ClipEffect {
    /// An effect seeded with its kind's default params.
    pub fn new(kind: EffectKind) -> Self {
        ClipEffect {
            kind,
            enabled: true,
            params: AnimProps::new(EffectParams::seed(kind.target_kind())),
        }
    }
}

/// A clip-level transition (01 §5, catalog in 08 §2.0b).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    pub kind: TransitionKind,
    pub duration: Tick,
    #[serde(default)]
    pub params: TransitionParams,
}

impl Transition {
    pub fn new(kind: TransitionKind, duration: Tick) -> Self {
        Transition {
            kind,
            duration,
            params: TransitionParams::default(),
        }
    }
}

/// v1 transition catalog (08 §2.0b). Additive-only.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    CrossDissolve,
    DipToBlack,
    DipToColor,
    Wipe,
    Push,
}

/// Transition parameters (union across the catalog; only the fields relevant to
/// a given `kind` are meaningful).
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransitionParams {
    /// Easing over the overlap window `t∈0..1`.
    #[serde(default)]
    pub curve: EaseCurve,
    /// Through-color for `DipToColor`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    /// Direction for `Wipe`/`Push`.
    #[serde(default)]
    pub direction: WipeDirection,
    /// Edge softness for `Wipe`.
    #[serde(default)]
    pub softness: f32,
}

impl Default for TransitionParams {
    fn default() -> Self {
        TransitionParams {
            curve: EaseCurve::default(),
            color: None,
            direction: WipeDirection::default(),
            softness: 0.0,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EaseCurve {
    Linear,
    EaseIn,
    EaseOut,
    #[default]
    EaseInOut,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WipeDirection {
    #[default]
    Left,
    Right,
    Up,
    Down,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_end_and_overlap() {
        let c = Clip::new(ClipSource::Adjustment, Tick(100), Tick(50));
        assert_eq!(c.end(), Tick(150));
        assert!(c.overlaps(Tick(120), Tick(130)));
        assert!(!c.overlaps(Tick(150), Tick(200)));
        assert!(!c.overlaps(Tick(0), Tick(100)));
    }

    #[test]
    fn speed_source_delta_is_exact() {
        // 2× speed: 100 ticks of timeline maps to 200 ticks of source.
        let s = SpeedMap::Constant(Ratio::new(2, 1));
        assert_eq!(s.source_delta(Tick(100)), Tick(200));
        // Half speed.
        let h = SpeedMap::Constant(Ratio::new(1, 2));
        assert_eq!(h.source_delta(Tick(100)), Tick(50));
        // Reverse.
        let r = SpeedMap::Constant(Ratio::new(-1, 1));
        assert_eq!(r.source_delta(Tick(100)), Tick(-100));
    }

    // ── Speed ramps (G-11) ───────────────────────────────────────────────

    #[test]
    fn speed_map_default_is_constant_one() {
        assert_eq!(SpeedMap::default(), SpeedMap::Constant(Ratio::ONE));
    }

    #[test]
    fn speed_map_constant_serde_tag_unchanged() {
        // Additive discipline: `Constant` still serializes under the `constant`
        // tag with the flattened ratio and no ramp field, so pre-existing saved
        // clips load byte-shape-identically.
        let j = serde_json::to_string(&SpeedMap::Constant(Ratio::new(2, 1))).unwrap();
        assert!(j.contains("\"speed\":\"constant\""));
        assert!(!j.contains("keys"));
        let back: SpeedMap = serde_json::from_str(&j).unwrap();
        assert_eq!(back, SpeedMap::Constant(Ratio::new(2, 1)));
    }

    #[test]
    fn speed_map_keyframed_serde_roundtrip() {
        let ramp = SpeedMap::Keyframed {
            keys: vec![
                SpeedKey::new(Tick(0), Ratio::new(1, 2)),
                SpeedKey::new(Tick(50), Ratio::new(2, 1)),
                SpeedKey::new(Tick(120), Ratio::new(-1, 1)),
            ],
        };
        let j = serde_json::to_string(&ramp).unwrap();
        assert!(j.contains("\"speed\":\"keyframed\""));
        let back: SpeedMap = serde_json::from_str(&j).unwrap();
        assert_eq!(ramp, back);
    }

    #[test]
    fn speed_ramp_source_time_mapping() {
        // A 2× ramp doubles the source advance (100 timeline → 200 source),
        // matching the constant case; a ½× ramp halves it (100 → 50).
        let fast = SpeedMap::Keyframed {
            keys: vec![SpeedKey::new(Tick(0), Ratio::new(2, 1))],
        };
        assert_eq!(fast.source_delta(Tick(100)), Tick(200));
        let slow = SpeedMap::Keyframed {
            keys: vec![SpeedKey::new(Tick(0), Ratio::new(1, 2))],
        };
        assert_eq!(slow.source_delta(Tick(100)), Tick(50));

        // A single-key ramp is exactly equivalent to the matching constant.
        for dt in [Tick(0), Tick(37), Tick(100), Tick(-100)] {
            assert_eq!(
                fast.source_delta(dt),
                SpeedMap::Constant(Ratio::new(2, 1)).source_delta(dt)
            );
        }

        // A two-segment ramp integrates piecewise: 1× over [0,50) = 50 source,
        // then 2× over [50,100) = 100 source ⇒ 150 total; 50 at the split.
        let ramp = SpeedMap::Keyframed {
            keys: vec![
                SpeedKey::new(Tick(0), Ratio::new(1, 1)),
                SpeedKey::new(Tick(50), Ratio::new(2, 1)),
            ],
        };
        assert_eq!(ramp.source_delta(Tick(50)), Tick(50));
        assert_eq!(ramp.source_delta(Tick(100)), Tick(150));

        // Before the first key the first key's ratio holds (reverse scrub).
        assert_eq!(ramp.source_delta(Tick(-20)), Tick(-20));
    }

    #[test]
    fn speed_ramp_empty_is_identity() {
        let empty = SpeedMap::Keyframed { keys: vec![] };
        assert_eq!(empty.source_delta(Tick(100)), Tick(100));
    }

    #[test]
    fn speed_ramp_integrates_regardless_of_key_order() {
        let sorted = SpeedMap::Keyframed {
            keys: vec![
                SpeedKey::new(Tick(0), Ratio::new(1, 1)),
                SpeedKey::new(Tick(50), Ratio::new(2, 1)),
            ],
        };
        let shuffled = SpeedMap::Keyframed {
            keys: vec![
                SpeedKey::new(Tick(50), Ratio::new(2, 1)),
                SpeedKey::new(Tick(0), Ratio::new(1, 1)),
            ],
        };
        assert_eq!(
            sorted.source_delta(Tick(100)),
            shuffled.source_delta(Tick(100))
        );
    }

    // ── Text clips (G-12) ────────────────────────────────────────────────

    #[test]
    fn text_clip_content_new_uses_default_style() {
        let c = TextClipContent::new("Title");
        assert_eq!(c.text, "Title");
        assert_eq!(c.style, CaptionStyle::default());
    }

    #[test]
    fn clip_source_text_serde_roundtrip() {
        let mut content = TextClipContent::new("Chapter One");
        content.style.font_size = 96.0;
        let clip = Clip::new(
            ClipSource::Text {
                content: content.clone(),
            },
            Tick(0),
            Tick(300),
        );
        let j = serde_json::to_string(&clip).unwrap();
        assert!(j.contains("\"source\":\"text\""));
        let back: Clip = serde_json::from_str(&j).unwrap();
        assert_eq!(clip, back);
        assert!(
            matches!(back.source, ClipSource::Text { content } if content.text == "Chapter One")
        );
    }

    #[test]
    fn clip_roundtrip_serde() {
        let mut c = Clip::new(
            ClipSource::Asset {
                asset: AssetId::new(),
            },
            Tick(0),
            Tick(100),
        );
        c.effects.push(ClipEffect::new(EffectKind::Blur));
        c.transition_in = Some(Transition::new(TransitionKind::CrossDissolve, Tick(20)));
        c.color_label = Some(3);
        c.link_group = Some(LinkGroupId::new());
        c.reframe.insert(1, ClipTransform::default());
        let j = serde_json::to_string(&c).unwrap();
        let back: Clip = serde_json::from_str(&j).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn clip_color_label_and_link_group_default_to_none() {
        let c = Clip::new(ClipSource::Adjustment, Tick(0), Tick(10));
        assert_eq!(c.color_label, None);
        assert_eq!(c.link_group, None);
    }

    #[test]
    fn clip_color_label_and_link_group_absent_from_json_when_unset() {
        // Additive-field discipline: an unset optional field must not appear
        // in the serialized form, so pre-existing saved documents that never
        // had these fields still round-trip byte-for-byte-equivalent shape.
        let c = Clip::new(ClipSource::Adjustment, Tick(0), Tick(10));
        let j = serde_json::to_string(&c).unwrap();
        assert!(!j.contains("color_label"));
        assert!(!j.contains("link_group"));
    }

    #[test]
    fn link_group_id_serde_is_transparent_uuid() {
        let id = LinkGroupId::new();
        let j = serde_json::to_string(&id).unwrap();
        let as_uuid: Uuid = serde_json::from_str(&j).unwrap();
        assert_eq!(as_uuid, id.0);
        let back: LinkGroupId = serde_json::from_str(&j).unwrap();
        assert_eq!(back, id);
    }
}
