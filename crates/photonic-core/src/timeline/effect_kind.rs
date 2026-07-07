//! Effect registry discriminant and its parameter bag (01 §6.3, 08 §3).
//!
//! `EffectKind` is the canonical home for the effect-registry enum (relocated
//! here from `photonic-video`'s `contract.rs`, which now re-exports it). It is
//! shared by clip-level [`ClipEffect`](super::clip::ClipEffect) and the
//! filter-family `GraphOp` variants (08 §3). Arity (0-input generator vs. unary
//! filter) is a property of the registry entry, not this enum.

use super::anim::{PropPath, PropSet, PropValue};
use super::prop_registry::{self, PropTargetKind};
use serde::{Deserialize, Serialize};

/// The v1 effect catalog (08 §3, normative; additive-only). `Transform2D`/`Crop`
/// are dedicated IR ops in 02, deliberately NOT effects.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EffectKind {
    Blur,
    Sharpen,
    Glow,
    ChromaKey,
    LumaKey,
    Invert,
    MaskShapeGen,
}

impl EffectKind {
    /// The registry key for this effect's params.
    pub fn target_kind(self) -> PropTargetKind {
        PropTargetKind::Effect(self)
    }
}

/// Keyframe-authoring effect parameters: an ordered `PropPath → PropValue` map,
/// seeded from the kind's registry defaults, every entry animatable via the
/// surrounding [`AnimProps`](super::anim::AnimProps).
///
/// Backed by an ordered `Vec` of pairs (not a `HashMap`) so serialization is
/// deterministic — a hard requirement for the export-determinism tests (03 §6)
/// and for stable undo diffs.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EffectParams {
    pub entries: Vec<(PropPath, PropValue)>,
}

impl EffectParams {
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed a param bag from a target kind's registry block, using neutral
    /// defaults (0 / center-of-range for floats, transparent for colors).
    pub fn seed(kind: PropTargetKind) -> Self {
        let mut entries = Vec::new();
        for e in prop_registry::entries(kind) {
            let value = match e.kind {
                super::anim::PropValueKind::Float => {
                    // Neutral default: 0 if in range, else the range min.
                    let v = match e.range {
                        Some((lo, hi)) if (lo..=hi).contains(&0.0) => 0.0,
                        Some((lo, _)) => lo,
                        None => 0.0,
                    };
                    PropValue::Float(v)
                }
                super::anim::PropValueKind::Vec2 => PropValue::Vec2([0.0, 0.0]),
                super::anim::PropValueKind::Color => PropValue::Color(crate::Color {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.0,
                }),
                super::anim::PropValueKind::Bool => PropValue::Bool(false),
                super::anim::PropValueKind::Enum => PropValue::Enum(0),
            };
            entries.push((PropPath::new(e.path), value));
        }
        EffectParams { entries }
    }

    pub fn get(&self, path: &str) -> Option<&PropValue> {
        self.entries
            .iter()
            .find(|(p, _)| p.as_str() == path)
            .map(|(_, v)| v)
    }

    /// Set (or insert) a param, preserving insertion order.
    pub fn set(&mut self, path: impl Into<PropPath>, value: PropValue) {
        let path = path.into();
        if let Some(slot) = self.entries.iter_mut().find(|(p, _)| *p == path) {
            slot.1 = value;
        } else {
            self.entries.push((path, value));
        }
    }
}

impl PropSet for EffectParams {
    // Effect params are an open map shared across effect kinds, graph nodes, and
    // audio-fx units; per-kind path validation happens through `prop_registry`
    // with the concrete kind. The generic bound uses the lenient `GraphNode`
    // target so the trait has a single const.
    const TARGET_KIND: PropTargetKind = PropTargetKind::GraphNode;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_populates_from_registry() {
        let p = EffectParams::seed(PropTargetKind::Effect(EffectKind::Glow));
        assert!(p.get("params.radius").is_some());
        assert!(matches!(p.get("params.tint"), Some(PropValue::Color(_))));
    }

    #[test]
    fn set_preserves_order_and_updates() {
        let mut p = EffectParams::seed(PropTargetKind::Effect(EffectKind::Blur));
        p.set("params.radius", PropValue::Float(12.0));
        assert_eq!(p.get("params.radius"), Some(&PropValue::Float(12.0)));
        let n = p.entries.len();
        p.set("params.radius", PropValue::Float(20.0));
        assert_eq!(p.entries.len(), n, "existing key must not duplicate");
    }

    #[test]
    fn effect_params_roundtrip_serde() {
        let p = EffectParams::seed(PropTargetKind::Effect(EffectKind::ChromaKey));
        let json = serde_json::to_string(&p).unwrap();
        let back: EffectParams = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }
}
