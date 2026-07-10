//! Clips (01 §5): the atomic timeline elements.
//!
//! A clip positions a source in a sequence with trim, speed, an animatable
//! transform, an ordered effect stack, an optional grade and per-clip
//! composition, transitions, and audio. The composition (when set) substitutes
//! only the clip's *source* op; transform/effects/grade/reframe still apply on
//! top (02 §2 step 3).

use super::anim::{AnimProps, PropSet};
use super::audio::ClipAudio;
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

/// Clip speed. Keyframed speed ramps are a post-v1 non-goal; the enum leaves
/// room for them.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "speed", rename_all = "snake_case")]
pub enum SpeedMap {
    Constant(Ratio),
}

impl Default for SpeedMap {
    fn default() -> Self {
        SpeedMap::Constant(Ratio::ONE)
    }
}

impl SpeedMap {
    /// Source time for a clip-relative time delta `dt` (01 §5.1: exact rational
    /// arithmetic; `source = source_in + dt * speed`). Returns the source-time
    /// delta only (caller adds `source_in`).
    pub fn source_delta(self, dt: Tick) -> Tick {
        match self {
            SpeedMap::Constant(r) => Tick(dt.0 * r.num as i64 / r.den.max(1) as i64),
        }
    }
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
