//! Frame-graph IR — the normative type contract from 02 §2, pinned in code.
//!
//! Properties (02 §2, restated as the contract this module guarantees):
//! - A `FrameGraph` is a pure function of (document snapshot, sequence, format,
//!   tick, quality flags): same inputs ⇒ identical graph ⇒ identical pixels.
//! - All keyframe evaluation happens at compile time; the IR carries resolved
//!   params and the evaluator is time-ignorant.
//! - Every node has a content hash — `hash(op, resolved params, input hashes)`
//!   — which is the cache key for the node-result texture cache (02 §5) and
//!   the texture pool underneath it (03 §3.4).
//!
//! P1 consumers: the renderer's texture pool + Tier-B vector conversion pass
//! (03 §2.5/§3.4) key their allocations by [`ContentHash`] and [`TextureDesc`].
//! The compiler (`graph::compile`) and evaluator (`graph::eval`) land in P3.

use glam::Mat3;
use photonic_core::layer::BlendMode;

use crate::contract::{
    AssetId, CaptionBatch, EffectKind, MatteModel, ResolvedGradeOp, ResolvedParams,
    ResolvedTextBlock, Tick, VectorRef, VectorStateKey,
};

/// Index of a node within one compiled [`FrameGraph`] arena. Not stable across
/// compiles — cross-frame identity is [`ContentHash`], never this index.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct IrNodeId(pub u32);

/// Output-port index on a multi-output node (e.g. `ChannelSplit`); 0 for all
/// single-output ops.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct OutPort(pub u8);

/// Content hash of (op, resolved params, input hashes) — the cache identity of
/// a node's result (02 §5). 128-bit (xxh3-128 class) so collisions are a
/// non-concern at cache scale.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ContentHash(pub u128);

/// Working-texture descriptor for the pooled allocator (03 §3.4): always
/// `Rgba16Float`, linear-light Rec.709, premultiplied (D-09); pool buckets
/// round dimensions up to the next 64px multiple.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TextureDesc {
    pub width: u32,
    pub height: u32,
}

impl TextureDesc {
    /// Pool-bucket dimensions: round each dimension up to the next multiple of
    /// 64 so near-identical sequence formats share buckets (03 §3.4).
    pub fn bucket(&self) -> (u32, u32) {
        let up = |v: u32| v.div_ceil(64) * 64;
        (up(self.width), up(self.height))
    }
}

/// Geometry sampling mode for `Transform2D`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Sampling {
    Bilinear,
    Nearest,
}

/// Fit policy for `Resize`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum FitMode {
    /// Scale to fit inside, letterbox/pillarbox as needed.
    Fit,
    /// Scale to cover, cropping overflow.
    Fill,
    /// Non-uniform scale to exact dimensions.
    Stretch,
}

/// Premultiplied, linear-light Rec.709 RGBA (D-09).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LinearColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

/// Single channel selector for `ChannelSplit`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Channel {
    R,
    G,
    B,
    A,
}

/// Sweep axis + orientation for a directional `WipeMix`/`PushMix` transition
/// (08 §2.0b), lowered from `photonic_core::timeline::WipeDirection`. The name
/// states the direction the incoming clip is revealed / pushed toward: e.g.
/// `LeftToRight` reveals the incoming from the left edge as the mix advances.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum WipeDirection {
    LeftToRight,
    RightToLeft,
    TopToBottom,
    BottomToTop,
}

/// One frame-graph operation. Each op is one wgpu render/compute pass (or a
/// CPU worker-thread op where noted), with a CPU `eval_cpu` reference
/// implementation for export determinism and golden tests (02 §2).
#[derive(Clone, Debug, PartialEq)]
pub enum IrOp {
    // ── sources ────────────────────────────────────────────────────────────
    /// Decoded video frame at `src_time` (source-clock ticks, post trim/speed
    /// mapping). YUV→linear conversion + premultiply happen in this op's pass
    /// (03 §3.2/§3.3).
    DecodeVideo {
        asset: AssetId,
        src_time: Tick,
        proxy: bool,
    },
    /// Decoded still image (cached per asset).
    DecodeStill {
        asset: AssetId,
    },
    /// Rasterized vector content (Tier A CPU-readback or Tier B GPU-direct,
    /// 03 §2.5), cached by `doc_state`.
    RasterVector {
        vref: VectorRef,
        doc_state: VectorStateKey,
        w: u32,
        h: u32,
    },
    SolidColor {
        color: LinearColor,
    },

    // ── ops (unary unless stated) ──────────────────────────────────────────
    Transform2D {
        mat: Mat3,
        sampling: Sampling,
    },
    /// Effect pass; arity 0..N comes from the `EffectKind` registry entry
    /// (0-input = generator, e.g. `MaskShapeGen` — 08 §3).
    Effect {
        kind: EffectKind,
        params: ResolvedParams,
    },
    /// Serial corrector stack (07). Params are the keyframe-resolved form of
    /// the authoring `GradeOp`.
    Grade {
        ops: Vec<ResolvedGradeOp>,
    },
    /// Binary over-composite with full 26-blend-mode support via
    /// COMPOSITE_SHADER (03 §2.4). Inputs: a (top), b (bottom).
    Merge {
        mode: BlendMode,
        opacity: f32,
    },
    /// Directional wipe transition (08 §2.0b): a per-pixel `smoothstep` edge
    /// sweeping across `direction` at normalised position `t`, with a `softness`
    /// half-width feather; a premultiplied lerp between the two layers. Inputs:
    /// [incoming, outgoing]. `t` is the compile-time eased mix factor, so distinct
    /// ticks yield distinct content hashes (like the cross-dissolve). At `t == 0`
    /// the output equals `outgoing`; at `t == 1` it equals `incoming`.
    WipeMix {
        direction: WipeDirection,
        softness: f32,
        t: f32,
    },
    /// Directional push transition (08 §2.0b): both layers translate along
    /// `direction` by `t`, the incoming sliding in as the outgoing slides out,
    /// with `ops::transform2d` edge-clamp/pixel-center sampling. Inputs:
    /// [incoming, outgoing]. `t == 0` is `outgoing`, `t == 1` is `incoming`.
    PushMix {
        direction: WipeDirection,
        t: f32,
    },
    CaptionOverlay {
        cue_batch: CaptionBatch,
    },
    Crop,
    Resize {
        w: u32,
        h: u32,
        fit: FitMode,
    },
    /// U²-Net matte inference via photonic-matte — CPU worker-thread op, NOT a
    /// GPU pass; cached aggressively (08 §3 "slow node").
    MatteExtract {
        model: MatteModel,
    },
    /// Styled-text raster for graph `Text` nodes (08 §3); glyphon pipeline.
    TextGen {
        block: ResolvedTextBlock,
    },
    /// Image → single-channel Mask.
    ChannelSplit {
        channel: Channel,
    },
    /// 3-4 Mask inputs → Image.
    ChannelCombine,
    Output {
        w: u32,
        h: u32,
    },
}

/// One node in a compiled frame graph.
#[derive(Clone, Debug, PartialEq)]
pub struct IrNode {
    pub op: IrOp,
    /// Inputs in op-defined order (e.g. Merge: [a/top, b/bottom]), each
    /// addressing a producing node's output port.
    pub inputs: Vec<(IrNodeId, OutPort)>,
    /// Cache identity: hash(op, resolved params, input hashes). Computed at
    /// compile time (02 §2).
    pub content_hash: ContentHash,
}

/// A compiled, topologically-ordered frame graph for one (sequence, format,
/// tick, quality) tuple. Arena indices are compile-local; caching is by
/// [`ContentHash`].
#[derive(Clone, Debug, PartialEq, Default)]
pub struct FrameGraph {
    /// Topological order: every node's inputs precede it.
    pub nodes: Vec<IrNode>,
    /// The terminal `Output` node.
    pub output: Option<IrNodeId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_bucket_rounds_up_to_64() {
        assert_eq!(
            TextureDesc {
                width: 1920,
                height: 1080
            }
            .bucket(),
            (1920, 1088)
        );
        assert_eq!(
            TextureDesc {
                width: 1,
                height: 64
            }
            .bucket(),
            (64, 64)
        );
        assert_eq!(
            TextureDesc {
                width: 1921,
                height: 1081
            }
            .bucket(),
            (1984, 1088)
        );
    }

    #[test]
    fn frame_rate_ticks_are_integral_for_ntsc() {
        use crate::contract::{FrameRate, TICKS_PER_SECOND};
        let r2997 = FrameRate {
            num: 30000,
            den: 1001,
        };
        assert_eq!(r2997.ticks_per_frame().0, TICKS_PER_SECOND / 30000 * 1001);
        assert_eq!(r2997.ticks_per_frame().0 * 30000, TICKS_PER_SECOND * 1001);
    }
}
