//! CPU reference evaluator (02 §2 "Evaluation": `eval_cpu`).
//!
//! An f32 implementation of the P3 `IrOp` subset, used as the ground truth for
//! golden tests and GPU parity (03 §6, §4.4). Determinism is exact: the same
//! [`FrameGraph`] and the same [`FrameProvider`] outputs produce a byte-identical
//! f32 image (02 §2's normative property). The evaluator is time-ignorant — every
//! keyframe was already resolved by the compiler.
//!
//! Source ops (`DecodeVideo`/`DecodeStill`/`RasterVector`) are delegated to a
//! [`FrameProvider`] so tests can inject known pixels without a real decoder; the
//! GPU evaluator (`eval.rs`) resolves the same ops against decode rings and the
//! headless vector renderer. Ops without a P3 kernel (`Effect`, `Grade`,
//! `CaptionOverlay`, `MatteExtract`, `TextGen`, `ChannelSplit`/`Combine`) are
//! input-passthrough with the phase noted, exactly as 02 §2 permits.

use photonic_core::layer::BlendMode;

use crate::contract::{AssetId, Tick, VectorRef, VectorStateKey};
use crate::graph::ir::{FitMode, FrameGraph, IrOp};
use crate::graph::ops::{self, Image};

/// Supplies source-op pixels to the CPU evaluator (02 §2's `eval_cpu` source
/// hook). All images are premultiplied linear Rec.709 (D-09). `w`/`h` are the
/// canvas hint (the output format); a provider may return that size or a native
/// size the evaluator resamples downstream.
pub trait FrameProvider {
    fn decode_video(&mut self, asset: AssetId, src_time: Tick, proxy: bool, w: u32, h: u32) -> Image;
    fn decode_still(&mut self, asset: AssetId, w: u32, h: u32) -> Image;
    fn raster_vector(&mut self, vref: VectorRef, key: VectorStateKey, w: u32, h: u32) -> Image;
}

/// A provider that yields transparent black for every source — enough to
/// exercise the SolidColor/Merge/Transform golden math with no real media.
pub struct EmptyProvider;

impl FrameProvider for EmptyProvider {
    fn decode_video(&mut self, _: AssetId, _: Tick, _: bool, w: u32, h: u32) -> Image {
        Image::new(w, h)
    }
    fn decode_still(&mut self, _: AssetId, w: u32, h: u32) -> Image {
        Image::new(w, h)
    }
    fn raster_vector(&mut self, _: VectorRef, _: VectorStateKey, w: u32, h: u32) -> Image {
        Image::new(w, h)
    }
}

/// Evaluate `graph` to its output [`Image`], sizing generators to `canvas`
/// (the active format's pixel dimensions). Nodes are already topologically
/// sorted (every input precedes its consumer), so one forward pass suffices.
pub fn evaluate(graph: &FrameGraph, canvas: (u32, u32), provider: &mut dyn FrameProvider) -> Image {
    let (cw, ch) = (canvas.0.max(1), canvas.1.max(1));
    let mut results: Vec<Option<Image>> = (0..graph.nodes.len()).map(|_| None).collect();

    for (i, node) in graph.nodes.iter().enumerate() {
        let img = {
            let inputs: Vec<&Image> = node
                .inputs
                .iter()
                .map(|(id, _)| {
                    results[id.0 as usize]
                        .as_ref()
                        .expect("input evaluated before consumer (topo order)")
                })
                .collect();
            eval_op(&node.op, &inputs, cw, ch, provider)
        };
        results[i] = Some(img);
    }

    match graph.output {
        Some(out) => results[out.0 as usize]
            .take()
            .unwrap_or_else(|| Image::new(cw, ch)),
        None => Image::new(cw, ch),
    }
}

fn eval_op(
    op: &IrOp,
    inputs: &[&Image],
    cw: u32,
    ch: u32,
    provider: &mut dyn FrameProvider,
) -> Image {
    // Missing-input safety: the compiler always wires unary/binary ops, but be
    // defensive so a malformed graph degrades to transparent, never panics.
    let in0 = || inputs.first().map(|i| (*i).clone()).unwrap_or_else(|| Image::new(cw, ch));

    match op {
        IrOp::DecodeVideo { asset, src_time, proxy } => {
            provider.decode_video(*asset, *src_time, *proxy, cw, ch)
        }
        IrOp::DecodeStill { asset } => provider.decode_still(*asset, cw, ch),
        IrOp::RasterVector { vref, doc_state, w, h } => {
            provider.raster_vector(*vref, *doc_state, *w, *h)
        }
        IrOp::SolidColor { color } => ops::solid(cw, ch, *color),
        IrOp::Transform2D { mat, sampling } => match inputs.first() {
            Some(input) => ops::transform2d(input, *mat, *sampling),
            None => Image::new(cw, ch),
        },
        // Passthrough ops (real kernels land in later phases; 02 §2 permits an
        // input-passthrough marker in P3).
        IrOp::Effect { .. } => in0(),      // P5/P7 effect kernels
        IrOp::Grade { .. } => in0(),       // P7 grade math
        IrOp::CaptionOverlay { .. } => in0(), // P5 glyph batching
        IrOp::MatteExtract { .. } => in0(), // P8 U²-Net inference
        IrOp::ChannelSplit { .. } => in0(),
        IrOp::ChannelCombine => in0(),
        IrOp::TextGen { .. } => Image::new(cw, ch), // P8 styled-text generator
        IrOp::Merge { mode, opacity } => match (inputs.first(), inputs.get(1)) {
            (Some(top), Some(bottom)) => ops::merge(top, bottom, *mode, *opacity),
            (Some(top), None) => (*top).clone(),
            (None, Some(bottom)) => (*bottom).clone(),
            (None, None) => Image::new(cw, ch),
        },
        IrOp::Crop => match inputs.first() {
            Some(input) => ops::crop(input),
            None => Image::new(cw, ch),
        },
        IrOp::Resize { w, h, fit } => match inputs.first() {
            Some(input) => ops::resize(input, *w, *h, *fit),
            None => Image::new(*w, *h),
        },
        IrOp::Output { w, h } => match inputs.first() {
            Some(input) if input.width == *w && input.height == *h => (*input).clone(),
            Some(input) => ops::resize(input, *w, *h, FitMode::Stretch),
            None => Image::new(*w, *h),
        },
    }
}

/// The blend mode a `Merge` node applies (exposed for tests / callers that want
/// to reason about the graph without re-matching the op).
pub fn merge_mode(op: &IrOp) -> Option<BlendMode> {
    match op {
        IrOp::Merge { mode, .. } => Some(*mode),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::compile::{compile, Quality};
    use crate::graph::ir::LinearColor;
    use photonic_core::timeline::{
        Clip, ClipSource, FrameRate, Sequence, SequenceId, TimelineProject, Track, TrackKind,
    };
    use photonic_core::Color;

    fn project_with_two_solids(top: Color, bottom: Color, top_opacity: f64) -> (TimelineProject, SequenceId) {
        let mut project = TimelineProject::new();
        let mut seq = Sequence::new("seq", FrameRate::FPS_30, 4, 4);
        let seq_id = seq.id;
        // bottom track (index 0), top track (index 1)
        let mut t0 = Track::new(TrackKind::Video, "V1");
        t0.clips.push(Clip::new(
            ClipSource::SolidColor { color: bottom },
            crate::contract::Tick(0),
            crate::contract::Tick::from_seconds(2),
        ));
        let mut t1 = Track::new(TrackKind::Video, "V2");
        let mut c = Clip::new(
            ClipSource::SolidColor { color: top },
            crate::contract::Tick(0),
            crate::contract::Tick::from_seconds(2),
        );
        c.transform.base.opacity = top_opacity;
        t1.clips.push(c);
        seq.video_tracks.push(t0);
        seq.video_tracks.push(t1);
        project.insert_sequence(seq);
        (project, seq_id)
    }

    /// SolidColor + Merge golden math: opaque top fully covers the backdrop.
    #[test]
    fn opaque_top_covers_backdrop() {
        // Colors are given in sRGB straight; the compiler converts to linear.
        let red = Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 };
        let blue = Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 };
        let (project, seq_id) = project_with_two_solids(red, blue, 1.0);
        let out = compile(&project, seq_id, 0, crate::contract::Tick(0), Quality::FULL, None);
        let img = evaluate(&out.graph, (4, 4), &mut EmptyProvider);
        // Linear red premultiplied (alpha 1): srgb→linear(1.0) == 1.0, srgb→linear(0.0) == 0.
        for p in &img.pixels {
            assert!((p[0] - 1.0).abs() < 1e-4, "r={}", p[0]);
            assert!(p[1].abs() < 1e-4);
            assert!(p[2].abs() < 1e-4, "b={}", p[2]);
            assert!((p[3] - 1.0).abs() < 1e-4);
        }
    }

    /// Half-opacity top over an opaque backdrop is an exact 50/50 linear blend.
    #[test]
    fn half_opacity_blends_linearly() {
        // White over black at opacity 0.5. srgb→linear(1.0)=1, (0.0)=0.
        let white = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
        let black = Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
        let (project, seq_id) = project_with_two_solids(white, black, 0.5);
        let out = compile(&project, seq_id, 0, crate::contract::Tick(0), Quality::FULL, None);
        let img = evaluate(&out.graph, (4, 4), &mut EmptyProvider);
        for p in &img.pixels {
            for c in 0..3 {
                assert!((p[c] - 0.5).abs() < 1e-4, "channel {c} = {}", p[c]);
            }
            assert!((p[3] - 1.0).abs() < 1e-4);
        }
    }

    /// Determinism (02 §2): the same graph evaluates byte-identically twice.
    #[test]
    fn evaluation_is_byte_identical() {
        let c1 = Color { r: 0.2, g: 0.5, b: 0.9, a: 1.0 };
        let c2 = Color { r: 0.7, g: 0.1, b: 0.3, a: 1.0 };
        let (project, seq_id) = project_with_two_solids(c1, c2, 0.4);
        let out = compile(&project, seq_id, 0, crate::contract::Tick(0), Quality::FULL, None);
        let a = evaluate(&out.graph, (8, 8), &mut EmptyProvider);
        let b = evaluate(&out.graph, (8, 8), &mut EmptyProvider);
        assert_eq!(a.pixels, b.pixels);
    }

    /// A bare SolidColor graph evaluates to the premultiplied linear color.
    #[test]
    fn solid_graph_matches_linear_conversion() {
        let mut project = TimelineProject::new();
        let mut seq = Sequence::new("seq", FrameRate::FPS_30, 2, 2);
        let seq_id = seq.id;
        let mut t = Track::new(TrackKind::Video, "V1");
        // 50% grey in sRGB → ~0.2140 linear.
        t.clips.push(Clip::new(
            ClipSource::SolidColor {
                color: Color { r: 0.5, g: 0.5, b: 0.5, a: 1.0 },
            },
            crate::contract::Tick(0),
            crate::contract::Tick::from_seconds(2),
        ));
        seq.video_tracks.push(t);
        project.insert_sequence(seq);
        let out = compile(&project, seq_id, 0, crate::contract::Tick(0), Quality::FULL, None);
        let img = evaluate(&out.graph, (2, 2), &mut EmptyProvider);
        let expected = {
            let c = 0.5f32;
            ((c + 0.055) / 1.055).powf(2.4)
        };
        for p in &img.pixels {
            assert!((p[0] - expected).abs() < 1e-4, "grey linear {}", p[0]);
        }
        // Sanity on the constant used above.
        assert!((expected - 0.2140).abs() < 1e-3);
        let _ = LinearColor { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
    }
}
