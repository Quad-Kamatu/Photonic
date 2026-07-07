//! Timeline-contract types referenced by the frame-graph IR (`graph::ir`).
//!
//! Shapes pinned by `docs/specs/video-editor/01-data-model.md` §1 (time) and §3
//! (assets). P2 relocates these to `photonic_core::timeline`; until then this is
//! their compile-checked single source. Do NOT add behavior here — types only.

use uuid::Uuid;

/// One tick = 1/705,600,000 s (the "flick"): exactly divides all supported
/// frame rates (incl. 1001-denominator NTSC) and audio rates. 01 §1.
pub const TICKS_PER_SECOND: i64 = 705_600_000;

/// Timeline position or duration in ticks. No float time anywhere in the
/// data model or IR (01 §1).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Tick(pub i64);

/// Rational frame rate, e.g. 30000/1001 for 29.97 fps. 01 §1.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct FrameRate {
    pub num: u32,
    pub den: u32,
}

impl FrameRate {
    /// Exact ticks per frame: `TICKS_PER_SECOND * den / num`, integral for all
    /// supported rates (that integrality is the reason flicks were chosen).
    pub fn ticks_per_frame(&self) -> Tick {
        Tick(TICKS_PER_SECOND * self.den as i64 / self.num as i64)
    }
}

/// Media-pool asset id (01 §3). Newtype over Uuid, matching the repo-wide
/// `NodeId = Uuid` convention with added type safety.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct AssetId(pub Uuid);

/// Reference to vector content inside (or associated with) the document
/// (01 §3): an artboard, a node subtree, or the whole document.
#[derive(Clone, Debug, PartialEq)]
pub enum VectorRef {
    Artboard(usize),
    Node(Uuid),
    WholeDocument,
}

/// Cache key for a rasterized vector frame: hash of the referenced nodes'
/// state + evaluated animated props + output size (02 §3, 03 §2.5).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct VectorStateKey(pub u128);

/// Effect registry discriminant. The v1 catalog is normative in 08 §3 /
/// 01 §6.3; variants are additive-only. Arity (0-input generator vs unary)
/// is a property of the registry entry, not of this enum (02 §2).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
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

/// Keyframe-resolved effect parameters (02 §2: "the IR carries resolved
/// params; the evaluator is time-ignorant"). Payload shape is finalized with
/// the P2 `prop_registry`; opaque at stub stage.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ResolvedParams {
    _pinned_in_p2: (),
}

/// Keyframe-resolved grade operator (07 §2's `ResolvedGradeOp`, the IR-side
/// sibling of the authoring `GradeOp`). Payload finalized in P2/P7.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedGradeOp {
    _pinned_in_p7: (),
}

/// Batch of caption cues covering the compiled tick (06 §4). Glyph-run
/// batching shape finalized in P5.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CaptionBatch {
    _pinned_in_p5: (),
}

/// Matte-extraction model selector (08 §3 `MaskFromMatte`; wraps
/// photonic-matte's U²-Net). Finalized in P8.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MatteModel {
    U2NetP,
}

/// Resolved styled-text block for graph `Text` nodes (08 §3 `TextGen`).
/// Layout/style payload finalized in P8 alongside the node catalog.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ResolvedTextBlock {
    _pinned_in_p8: (),
}
