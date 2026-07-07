//! Timeline-contract types referenced by the frame-graph IR (`graph::ir`).
//!
//! P2 relocated the canonical data-model types (`Tick`, `FrameRate`, `AssetId`,
//! `VectorRef`, `VectorStateKey`, `EffectKind`, and `TICKS_PER_SECOND`) into
//! `photonic_core::timeline`; this module now **re-exports** them so existing IR
//! code keeps compiling against `crate::contract::*` unchanged. The engine-side
//! *resolved* types below (keyframe-evaluated IR payloads) are not data model —
//! they stay here and are finalized in their respective phases.

// ── Relocated to photonic_core::timeline (canonical home) ───────────────────
pub use photonic_core::timeline::{
    AssetId, EffectKind, FrameRate, Tick, VectorRef, VectorStateKey, TICKS_PER_SECOND,
};

/// Keyframe-resolved effect parameters (02 §2: "the IR carries resolved params;
/// the evaluator is time-ignorant"). Payload shape is finalized with the P2
/// `prop_registry`; opaque at stub stage.
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

/// Batch of caption cues covering the compiled tick (06 §4). Glyph-run batching
/// shape finalized in P5.
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
