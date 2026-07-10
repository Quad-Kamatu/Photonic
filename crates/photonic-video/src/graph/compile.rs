//! Frame-graph compiler (02 §2 "Compilation"): lowers a timeline snapshot at one
//! `(sequence, format, tick, quality)` tuple into a [`FrameGraph`] IR.
//!
//! The compiler is a **pure function** of its inputs — same inputs ⇒ identical
//! graph ⇒ (via the evaluator) identical pixels (02 §2's normative property).
//! Every keyframe is resolved here, at compile time, so the evaluator is
//! time-ignorant. Every node carries a [`ContentHash`] of
//! `hash(op discriminant, resolved params, input hashes)` — deterministic across
//! runs (no `Instant`, no random state), which is what makes the node-result
//! cache (02 §5) and golden tests (11) possible. Identical subgraphs collapse
//! to one node via that hash (the mechanism behind `TimeOffset` dedup, 02 §2
//! step 7 / 08 §3.4).
//!
//! The numbered steps below mirror 02 §2 exactly:
//! 1. per enabled video track, find the clip covering `t`;
//! 2. per clip build the chain source → Transform2D → effects → grade;
//! 3. per-clip composition splices the clip's **source op only** (02 §2 step 3 /
//!    08 §4), the still-applied Transform2D/effects/grade chain riding on top;
//! 4. fold tracks with `Merge`, Adjustment clips re-rooting the stack below;
//! 5. `CaptionOverlay` from enabled caption tracks covering `t`;
//! 6. splice the project graph (08 §5) between the fold result and `Output`;
//! 7. `TimeOffset` expansion by re-lowering the upstream subgraph at `t−offset`
//!    (dedup-by-hash keeps it bounded; soft cap 4 distinct offsets);
//! 8. constant-fold / dead-branch-eliminate (disabled clips, opacity 0).

use std::collections::{HashMap, HashSet};

use glam::{Mat3, Vec2};
use photonic_core::layer::BlendMode;
use photonic_core::timeline::{
    self, AnimProps, AssetKind, CaptionAnim, CaptionCue, CaptionStyle, CaptionTrack, CaptionWord,
    Clip, ClipSource, ClipTransform, EaseCurve, EffectKind, Grade, GradeOp, GradeOpKind,
    GradeOpParams, GraphId, GraphNode, GraphNodeId, GraphNodeParams, GraphOp, InPort, KaraokeMode,
    LutInterp, NodeGraph, PropPath, PropValue, Sequence, SequenceFormat, SequenceId,
    TextClipContent, TimelineProject, TrackKind, TransitionKind,
};
use photonic_core::Color;
use photonic_render::caption::CaptionWordRun;

use crate::contract::{
    AssetId, CaptionBatch, CaptionCueRun, MatteModel, ResolvedParams, ResolvedTextBlock, Tick,
    VectorRef, VectorStateKey, TICKS_PER_SECOND,
};
use crate::graph::ir::{
    Channel, ContentHash, FitMode, FrameGraph, IrNode, IrNodeId, IrOp, LinearColor, OutPort,
    Sampling, TextureDesc,
};

/// Preview vs full-resolution compile flags (02 §2's "quality flags"). `proxy`
/// selects proxy media where available (session state, `SetProxyMode`).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Quality {
    /// Decode proxy media instead of originals (preview). Export forces `false`.
    /// `Default` (`false`) is full quality.
    pub proxy: bool,
}

impl Quality {
    /// Preview quality (proxy media allowed).
    pub const PREVIEW: Quality = Quality { proxy: true };
    /// Full quality (originals; the export/scopes path).
    pub const FULL: Quality = Quality { proxy: false };
}

/// Session-only viewer pin (08 §6.7): reroute the effective output to a specific
/// node of the graph currently being edited, without changing the DAG that gets
/// built (shared upstream nodes stay cache-compatible). Never carried by export.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ViewNodeOverride {
    pub graph: GraphId,
    pub node: GraphNodeId,
}

/// A compile diagnostic (02 §2 step 3 / 08 §6.6). Carries the offending
/// `GraphNodeId` where one applies so the node editor can badge the exact node,
/// not just show a generic "composition failed" toast.
#[derive(Clone, Debug, PartialEq)]
pub struct CompileDiagnostic {
    pub message: String,
    pub graph: Option<GraphId>,
    pub node: Option<GraphNodeId>,
}

impl CompileDiagnostic {
    fn plain(message: impl Into<String>) -> Self {
        CompileDiagnostic {
            message: message.into(),
            graph: None,
            node: None,
        }
    }
    fn at(graph: GraphId, node: GraphNodeId, message: impl Into<String>) -> Self {
        CompileDiagnostic {
            message: message.into(),
            graph: Some(graph),
            node: Some(node),
        }
    }
}

/// The result of a compile: the graph plus any diagnostics (never black-frames
/// silently — a failed splice falls back to the default chain and records why).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CompiledFrame {
    pub graph: FrameGraph,
    pub diagnostics: Vec<CompileDiagnostic>,
}

/// Soft cap on distinct `TimeOffset` values per composition (02 §2 step 7 /
/// 08 §3.4): beyond this a diagnostic warns but compilation still proceeds.
pub const TIME_OFFSET_SOFT_CAP: usize = 4;

/// Compile the active sequence at `tick` in `format_index` to a frame graph.
///
/// `view_override` (08 §6.7) is session state — pass `None` for export/headless.
pub fn compile(
    project: &TimelineProject,
    sequence: SequenceId,
    format_index: usize,
    tick: Tick,
    quality: Quality,
    view_override: Option<ViewNodeOverride>,
) -> CompiledFrame {
    let mut b = Builder::new();

    let Some(seq) = project.sequences.get(&sequence) else {
        b.diag(CompileDiagnostic::plain(format!(
            "compile: unknown sequence {sequence}"
        )));
        return b.finish(None);
    };
    let format_index = format_index.min(seq.formats.len().saturating_sub(1));
    let Some(format) = seq.formats.get(format_index) else {
        b.diag(CompileDiagnostic::plain(format!(
            "compile: sequence {sequence} has no formats"
        )));
        return b.finish(None);
    };

    let mut cycle = HashSet::new();
    cycle.insert(sequence);

    // Steps 1–4: fold the enabled video tracks bottom→top.
    let program = fold_sequence(&mut b, project, seq, format_index, format, tick, quality, &mut cycle);

    // Step 5: caption overlay (enabled caption tracks with a cue covering t).
    let program = splice_captions(&mut b, seq, format, tick, program);

    // Step 6: project graph splice (08 §5) — between the fold result and Output.
    let program = splice_project_graph(&mut b, project, program, format, tick);

    // Terminal Output node (02 §2). Its input is the program, or transparent
    // black for an empty sequence.
    let out_input = program.unwrap_or_else(|| b.transparent(format));
    let output = b.push(
        IrOp::Output {
            w: format.width,
            h: format.height,
        },
        vec![(out_input, OutPort::default())],
    );

    // Step (08 §6.7): viewer pinning reroutes the effective output.
    let effective = resolve_view_override(&mut b, view_override, output);
    b.finish(Some(effective))
}

// ── Builder ─────────────────────────────────────────────────────────────────

/// Arena builder with content-hash dedup. Nodes are appended in dependency order
/// (every input is pushed before its consumer), so the finished `nodes` vector is
/// already topologically sorted (02 §2 "topo-sorted at build").
struct Builder {
    nodes: Vec<IrNode>,
    /// content hash → node id, so an identical (op, inputs) subgraph is emitted
    /// once (TimeOffset dedup, 02 §2 step 7).
    dedup: HashMap<u128, IrNodeId>,
    diagnostics: Vec<CompileDiagnostic>,
    /// Records every lowered `(graph, node)` → IR id so a `ViewNodeOverride`
    /// (08 §6.7) can reroute output to a pinned node.
    view_index: HashMap<(GraphId, GraphNodeId), IrNodeId>,
}

impl Builder {
    fn new() -> Self {
        Builder {
            nodes: Vec::new(),
            dedup: HashMap::new(),
            diagnostics: Vec::new(),
            view_index: HashMap::new(),
        }
    }

    fn diag(&mut self, d: CompileDiagnostic) {
        self.diagnostics.push(d);
    }

    /// Append a node, deduplicating by content hash. Inputs must already exist.
    fn push(&mut self, op: IrOp, inputs: Vec<(IrNodeId, OutPort)>) -> IrNodeId {
        let input_hashes: Vec<u128> = inputs
            .iter()
            .map(|(id, _)| self.nodes[id.0 as usize].content_hash.0)
            .collect();
        let hash = content_hash(&op, &inputs, &input_hashes);
        if let Some(&existing) = self.dedup.get(&hash.0) {
            return existing;
        }
        let id = IrNodeId(self.nodes.len() as u32);
        self.nodes.push(IrNode {
            op,
            inputs,
            content_hash: hash,
        });
        self.dedup.insert(hash.0, id);
        id
    }

    /// A transparent-black premultiplied `SolidColor` sized to `format` — the
    /// universal "nothing here" input (missing-input default, 08 §3.3).
    fn transparent(&mut self, _format: &SequenceFormat) -> IrNodeId {
        self.push(
            IrOp::SolidColor {
                color: LinearColor {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.0,
                },
            },
            vec![],
        )
    }

    fn finish(self, output: Option<IrNodeId>) -> CompiledFrame {
        CompiledFrame {
            graph: FrameGraph {
                nodes: self.nodes,
                output,
            },
            diagnostics: self.diagnostics,
        }
    }
}

fn resolve_view_override(
    b: &mut Builder,
    view: Option<ViewNodeOverride>,
    real_output: IrNodeId,
) -> IrNodeId {
    let Some(view) = view else {
        return real_output;
    };
    match b.view_index.get(&(view.graph, view.node)) {
        Some(&id) => id,
        None => {
            b.diag(CompileDiagnostic::at(
                view.graph,
                view.node,
                "view override target is not on the active output path; showing real output",
            ));
            real_output
        }
    }
}

// ── Step 1–4: track fold ──────────────────────────────────────────────────────

/// Fold one sequence's enabled video tracks (bottom→top) into a single program
/// node, honouring Adjustment re-rooting. Returns `None` for an empty program.
#[allow(clippy::too_many_arguments)]
fn fold_sequence(
    b: &mut Builder,
    project: &TimelineProject,
    seq: &Sequence,
    format_index: usize,
    format: &SequenceFormat,
    tick: Tick,
    quality: Quality,
    cycle: &mut HashSet<SequenceId>,
) -> Option<IrNodeId> {
    let mut acc: Option<IrNodeId> = None;

    for track in &seq.video_tracks {
        if track.kind != TrackKind::Video || !track.enabled {
            continue;
        }
        let clips = track.clips.as_slice();
        let Some(idx) = covering_clip_index(clips, tick) else {
            continue;
        };
        let clip = &clips[idx];
        if !clip.enabled {
            continue; // step 8: disabled clip is a dead branch.
        }

        // Adjustment clips (step 4): re-root the composite below through the
        // clip's effect/grade chain rather than contributing a source.
        if matches!(clip.source, ClipSource::Adjustment) {
            if let Some(below) = acc {
                let dt = tick - clip.start;
                acc = Some(apply_effect_grade_chain(b, clip, below, dt));
            }
            continue;
        }

        // Transition partner (02 §2 step 1 / 08 §2.0b): during a clip-overlap
        // window, blend the outgoing + incoming partner by the eased mix factor.
        // A successful transition contributes a single track image at opacity 1
        // (each partner's own opacity is baked into its side of the mix).
        if let Some(tr) = active_transition(clips, idx, tick) {
            if let Some(node) = build_transition(
                b, project, seq, format_index, format, clips, &tr, tick, quality, cycle,
            ) {
                acc = Some(fold_over(b, acc, node, 1.0));
                continue;
            }
            // Partner unavailable (disabled / opacity-0 / Adjustment): fall
            // through to the plain covering-clip render below.
        }

        let Some((image, opacity)) = build_clip_chain(
            b,
            project,
            seq,
            format_index,
            format,
            clip,
            tick,
            quality,
            cycle,
        ) else {
            continue; // step 8: invisible / opacity-0 clip folded away.
        };
        acc = Some(fold_over(b, acc, image, opacity));
    }
    acc
}

/// Index of the clip whose `[start, end)` covers `t` on this track (tracks are
/// sorted, non-overlapping — 01 §4).
fn covering_clip_index(clips: &[Clip], t: Tick) -> Option<usize> {
    clips.iter().position(|c| c.start <= t && t < c.end())
}

// ── Transitions (02 §2 step 1 / 08 §2.0b) ─────────────────────────────────────

/// A resolved clip transition covering `tick`: which two clips blend, the mix
/// factor (already eased; `0` = fully outgoing, `1` = fully incoming), and how.
struct ActiveTransition {
    outgoing: usize,
    incoming: usize,
    kind: TransitionKind,
    params: timeline::TransitionParams,
    /// Eased mix in `0..1`.
    t: f32,
}

/// Detect a transition active at `tick` for the covering clip `idx` (08 §2.0b).
/// A `transition_in` borrows the previous clip as the outgoing partner; a
/// `transition_out` borrows the next clip as the incoming partner. Returns `None`
/// when no transition is active or the partner index is out of range.
fn active_transition(clips: &[Clip], idx: usize, tick: Tick) -> Option<ActiveTransition> {
    let clip = &clips[idx];
    // Start-boundary transition: this clip transitions IN from the previous clip.
    if let Some(tr) = &clip.transition_in {
        if idx > 0 && tr.duration.0 > 0 {
            let start = clip.start;
            let end = clip.start + tr.duration;
            if start <= tick && tick < end {
                let raw = (tick - start).0 as f32 / tr.duration.0 as f32;
                return Some(ActiveTransition {
                    outgoing: idx - 1,
                    incoming: idx,
                    kind: tr.kind,
                    params: tr.params,
                    t: ease(tr.params.curve, raw),
                });
            }
        }
    }
    // End-boundary transition: this clip transitions OUT into the next clip.
    if let Some(tr) = &clip.transition_out {
        if idx + 1 < clips.len() && tr.duration.0 > 0 {
            let start = clip.end() - tr.duration;
            let end = clip.end();
            if start <= tick && tick < end {
                let raw = (tick - start).0 as f32 / tr.duration.0 as f32;
                return Some(ActiveTransition {
                    outgoing: idx,
                    incoming: idx + 1,
                    kind: tr.kind,
                    params: tr.params,
                    t: ease(tr.params.curve, raw),
                });
            }
        }
    }
    None
}

/// Build the transition mix node, or `None` when a partner can't contribute
/// (disabled / opacity-0 / Adjustment) so the caller falls back to the plain
/// covering-clip render. Each partner is evaluated at `tick` (the outgoing clip
/// past its own end, into its source handles — the standard NLE overlap model).
#[allow(clippy::too_many_arguments)]
fn build_transition(
    b: &mut Builder,
    project: &TimelineProject,
    seq: &Sequence,
    format_index: usize,
    format: &SequenceFormat,
    clips: &[Clip],
    tr: &ActiveTransition,
    tick: Tick,
    quality: Quality,
    cycle: &mut HashSet<SequenceId>,
) -> Option<IrNodeId> {
    let outgoing_clip = &clips[tr.outgoing];
    let incoming_clip = &clips[tr.incoming];
    if !outgoing_clip.enabled
        || !incoming_clip.enabled
        || matches!(outgoing_clip.source, ClipSource::Adjustment)
        || matches!(incoming_clip.source, ClipSource::Adjustment)
    {
        return None;
    }
    let (out_img, out_op) = build_clip_chain(
        b, project, seq, format_index, format, outgoing_clip, tick, quality, cycle,
    )?;
    let (in_img, in_op) = build_clip_chain(
        b, project, seq, format_index, format, incoming_clip, tick, quality, cycle,
    )?;
    let outgoing = bake_opacity(b, out_img, out_op);
    let incoming = bake_opacity(b, in_img, in_op);
    Some(transition_mix(b, tr.kind, &tr.params, outgoing, incoming, tr.t))
}

/// Fade `node` toward transparent by `opacity` (premultiplied) when `opacity < 1`,
/// so a partner's own clip opacity is baked into its side of a transition before
/// the mix. A fully-opaque partner is returned unchanged.
fn bake_opacity(b: &mut Builder, node: IrNodeId, opacity: f32) -> IrNodeId {
    if opacity >= 1.0 {
        return node;
    }
    let transparent = b.push(
        IrOp::SolidColor {
            color: LinearColor {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            },
        },
        vec![],
    );
    b.push(
        IrOp::Merge {
            mode: BlendMode::Normal,
            opacity,
        },
        vec![
            (node, OutPort::default()),
            (transparent, OutPort::default()),
        ],
    )
}

/// Emit the time-parameterized mix for a transition at eased factor `t` (08 §2.0b).
/// Reuses `Merge` (premultiplied `over`) and `SolidColor` — the mix factor is a
/// compile-time constant, so distinct ticks produce distinct `Merge` opacities
/// (and thus distinct content hashes). Geometric `Wipe`/`Push` fall back to a
/// cross-dissolve in P3 (no directional wipe pass yet) with a diagnostic.
fn transition_mix(
    b: &mut Builder,
    kind: TransitionKind,
    params: &timeline::TransitionParams,
    outgoing: IrNodeId,
    incoming: IrNodeId,
    t: f32,
) -> IrNodeId {
    match kind {
        TransitionKind::CrossDissolve => merge_over(b, incoming, outgoing, t),
        TransitionKind::DipToBlack => dip_through(b, outgoing, incoming, t, opaque_black()),
        TransitionKind::DipToColor => {
            dip_through(b, outgoing, incoming, t, params.color.unwrap_or_else(opaque_black))
        }
        TransitionKind::Wipe | TransitionKind::Push => {
            b.diag(CompileDiagnostic::plain(format!(
                "{kind:?} transition renders as a cross-dissolve in P3 \
                 (directional wipe/push pass pending)"
            )));
            merge_over(b, incoming, outgoing, t)
        }
    }
}

/// `Merge` `top` over `bottom` at `opacity` (Normal blend), the fold primitive
/// shared by every transition kind.
fn merge_over(b: &mut Builder, top: IrNodeId, bottom: IrNodeId, opacity: f32) -> IrNodeId {
    b.push(
        IrOp::Merge {
            mode: BlendMode::Normal,
            opacity: opacity.clamp(0.0, 1.0),
        },
        vec![(top, OutPort::default()), (bottom, OutPort::default())],
    )
}

/// Dip-through-color mix (`DipToBlack`/`DipToColor`, 08 §2.0b): outgoing dips to
/// `color` over the first half (`t < 0.5`), then `color` reveals the incoming
/// over the second half.
fn dip_through(
    b: &mut Builder,
    outgoing: IrNodeId,
    incoming: IrNodeId,
    t: f32,
    color: Color,
) -> IrNodeId {
    let solid = b.push(
        IrOp::SolidColor {
            color: color_to_linear_premult(color),
        },
        vec![],
    );
    if t < 0.5 {
        // outgoing → color; the solid fades in over `[0, 0.5)`.
        merge_over(b, solid, outgoing, t * 2.0)
    } else {
        // color → incoming; the incoming fades in over `[0.5, 1]`.
        merge_over(b, incoming, solid, (t - 0.5) * 2.0)
    }
}

fn opaque_black() -> Color {
    Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    }
}

/// Ease `t∈0..1` under `curve` (08 §2.0b transition easing). `EaseInOut` is the
/// standard smooth-in/out quadratic; the endpoints are exact (0→0, 1→1).
fn ease(curve: EaseCurve, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match curve {
        EaseCurve::Linear => t,
        EaseCurve::EaseIn => t * t,
        EaseCurve::EaseOut => t * (2.0 - t),
        EaseCurve::EaseInOut => {
            if t < 0.5 {
                2.0 * t * t
            } else {
                1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
            }
        }
    }
}

/// Composite `top` over `acc` with `opacity` (premultiplied `over`). A single
/// fully-opaque track needs no `Merge` (keeps the bare-clip graph minimal).
fn fold_over(b: &mut Builder, acc: Option<IrNodeId>, top: IrNodeId, opacity: f32) -> IrNodeId {
    match acc {
        None if opacity >= 1.0 => top,
        None => {
            let transparent = b.push(
                IrOp::SolidColor {
                    color: LinearColor {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.0,
                    },
                },
                vec![],
            );
            b.push(
                IrOp::Merge {
                    mode: BlendMode::Normal,
                    opacity,
                },
                vec![(top, OutPort::default()), (transparent, OutPort::default())],
            )
        }
        Some(bottom) => b.push(
            IrOp::Merge {
                mode: BlendMode::Normal,
                opacity,
            },
            vec![(top, OutPort::default()), (bottom, OutPort::default())],
        ),
    }
}

/// Step 2/3: build one clip's image node and its evaluated opacity, or `None`
/// when it folds away (disabled / opacity 0 — step 8). The chain is
/// source(or composition splice) → Transform2D → effects → grade.
#[allow(clippy::too_many_arguments)]
fn build_clip_chain(
    b: &mut Builder,
    project: &TimelineProject,
    seq: &Sequence,
    format_index: usize,
    format: &SequenceFormat,
    clip: &Clip,
    tick: Tick,
    quality: Quality,
    cycle: &mut HashSet<SequenceId>,
) -> Option<(IrNodeId, f32)> {
    let dt = tick - clip.start; // clip-relative time for keyframe eval (01 §6).

    // Per-format reframe override (CAP-012) is a static transform for this
    // format; otherwise evaluate the animated clip transform at dt.
    let xf = match clip.reframe.get(&format_index) {
        Some(over) => *over,
        None => eval_clip_transform(&clip.transform, dt),
    };
    let opacity = xf.opacity as f32;
    if opacity <= 0.0 {
        return None; // dead branch (step 8).
    }

    // Step 3: composition substitutes the SOURCE op only; else the plain source.
    let source = match clip.composition {
        Some(graph_id) => lower_composition(
            b, project, seq, format_index, format, clip, graph_id, tick, quality, cycle,
        ),
        None => build_clip_source(b, project, seq, format_index, format, clip, tick, quality, cycle),
    };

    // Remainder of step 2's chain, applied on top of the source/composition.
    let mut cur = source;
    cur = b.push(
        IrOp::Transform2D {
            mat: clip_transform_matrix(&xf),
            sampling: Sampling::Bilinear,
        },
        vec![(cur, OutPort::default())],
    );
    cur = apply_effect_grade_chain(b, clip, cur, dt);
    Some((cur, opacity))
}

/// Append the clip's enabled effect stack then its grade (if any) onto `input`.
/// Shared by the normal chain (step 2) and Adjustment re-rooting (step 4). `dt`
/// is clip-relative time (01 §6) — the domain the effect/grade keyframes live in.
fn apply_effect_grade_chain(b: &mut Builder, clip: &Clip, input: IrNodeId, dt: Tick) -> IrNodeId {
    let mut cur = input;
    for fx in &clip.effects {
        if !fx.enabled {
            continue;
        }
        // `Invert` is a real pass in the evaluator (08 §3); the other effect
        // kinds still emit a marker node (correct arity + ordering + content-hash
        // identity) that the evaluator passes through until their `ResolvedParams`
        // payload shape finalizes (P5/P7). The op discriminant + kind participate
        // in the content hash so distinct effects never collide.
        cur = b.push(
            IrOp::Effect {
                kind: fx.kind,
                params: ResolvedParams::default(),
            },
            vec![(cur, OutPort::default())],
        );
    }
    if let Some(grade) = &clip.grade {
        cur = apply_grade(b, grade, cur, dt);
    }
    cur
}

/// Resolve `grade` at `tick` and emit a `Grade` IR op carrying the resolved stack
/// (07 §2/§3), or return `input` unchanged when the grade is bypassed / empty /
/// fully inert. Shared by clip grades (step 2) and graph `Grade`/`Lut` nodes.
fn apply_grade(b: &mut Builder, grade: &Grade, input: IrNodeId, tick: Tick) -> IrNodeId {
    let ops = resolve_grade(grade, tick);
    if ops.is_empty() {
        input
    } else {
        b.push(IrOp::Grade { ops }, vec![(input, OutPort::default())])
    }
}

/// Resolve an authoring [`Grade`] at `tick` into the resolved op stack (07 §2)
/// via `photonic_render::grade::resolve`.
///
/// P3 has no `MediaPool` at the compile layer, so `Lut3d` asset ops resolve inert
/// (dropped → identity). LUT-table resolution lands when a `lut_provider` is
/// threaded through `compile` (needs pool access — out of `graph/` territory).
fn resolve_grade(grade: &Grade, tick: Tick) -> Vec<crate::contract::ResolvedGradeOp> {
    photonic_render::grade::resolve(grade, tick, |_asset: AssetId| {
        None::<std::sync::Arc<photonic_render::Lut3d>>
    })
}

// ── Step 2: clip source ────────────────────────────────────────────────────────

/// Build the clip's source op (after trim + speed source-time mapping, 01 §5.1).
#[allow(clippy::too_many_arguments)]
fn build_clip_source(
    b: &mut Builder,
    project: &TimelineProject,
    seq: &Sequence,
    format_index: usize,
    format: &SequenceFormat,
    clip: &Clip,
    tick: Tick,
    quality: Quality,
    cycle: &mut HashSet<SequenceId>,
) -> IrNodeId {
    let dt = tick - clip.start;
    let src_time = clip.source_in + clip.speed.source_delta(dt);

    match &clip.source {
        ClipSource::Asset { asset } => {
            let kind = project
                .media
                .assets
                .get(asset)
                .map(|a| a.kind)
                .unwrap_or(AssetKind::Video);
            match kind {
                AssetKind::Image => b.push(IrOp::DecodeStill { asset: *asset }, vec![]),
                AssetKind::Video | AssetKind::Audio | AssetKind::VectorDoc | AssetKind::Lut3d => b
                    .push(
                        IrOp::DecodeVideo {
                            asset: *asset,
                            src_time,
                            proxy: quality.proxy,
                        },
                        vec![],
                    ),
            }
        }
        ClipSource::Vector { asset } => {
            let vref = vector_ref_for(project, *asset);
            let doc_state = vector_state_key(*asset, format, src_time);
            b.push(
                IrOp::RasterVector {
                    vref,
                    doc_state,
                    w: format.width,
                    h: format.height,
                },
                vec![],
            )
        }
        ClipSource::NestedSequence { sequence } => {
            build_nested_sequence(b, project, *sequence, format_index, format, src_time, quality, cycle)
        }
        ClipSource::SolidColor { color } => b.push(
            IrOp::SolidColor {
                color: color_to_linear_premult(*color),
            },
            vec![],
        ),
        ClipSource::Adjustment => {
            // Reached only if an Adjustment clip is mis-placed as a source; the
            // fold loop handles real Adjustment clips (step 4). Fall back to
            // transparent so it never contributes a spurious image.
            let _ = seq;
            b.transparent(format)
        }
        ClipSource::Text { content } => {
            // G-12: a title/text clip lowers to the dedicated `TextGen` IR op,
            // its `TextClipContent` (text + shared `CaptionStyle`) resolved into
            // the same `CaptionCueRun` glyph payload the `CaptionOverlay` path
            // consumes (06 §5.3) — one text-render mechanism, not a parallel
            // path. The evaluator burns it over transparent via the glyphon
            // compositor; the clip's own `Transform2D` then places it.
            b.push(
                IrOp::TextGen {
                    block: resolve_text_block(content),
                },
                vec![],
            )
        }
    }
}

/// Recursively compile a nested sequence (CAP-005) and splice its program as a
/// source. Cycle-guarded: a re-entrant sequence yields a transparent placeholder
/// plus a diagnostic (never an infinite recursion / black frame).
#[allow(clippy::too_many_arguments)]
fn build_nested_sequence(
    b: &mut Builder,
    project: &TimelineProject,
    sequence: SequenceId,
    _parent_format_index: usize,
    parent_format: &SequenceFormat,
    src_time: Tick,
    quality: Quality,
    cycle: &mut HashSet<SequenceId>,
) -> IrNodeId {
    if cycle.contains(&sequence) {
        b.diag(CompileDiagnostic::plain(format!(
            "nested-sequence cycle: {sequence} references itself; substituting transparent"
        )));
        return b.transparent(parent_format);
    }
    let Some(nested) = project.sequences.get(&sequence) else {
        b.diag(CompileDiagnostic::plain(format!(
            "nested sequence {sequence} not found; substituting transparent"
        )));
        return b.transparent(parent_format);
    };

    let nested_format_index = nested.active_format.min(nested.formats.len().saturating_sub(1));
    let Some(nested_format) = nested.formats.get(nested_format_index) else {
        b.diag(CompileDiagnostic::plain(format!(
            "nested sequence {sequence} has no formats; substituting transparent"
        )));
        return b.transparent(parent_format);
    };
    // Arm the cycle guard around the recursive fold ONLY: every early return
    // above leaves the visited-set untouched, so a bail-out (missing / no-format
    // nested sequence referenced more than once) never poisons a sibling lower.
    cycle.insert(sequence);
    let program = fold_sequence(
        b,
        project,
        nested,
        nested_format_index,
        nested_format,
        src_time,
        quality,
        cycle,
    );
    cycle.remove(&sequence);

    program.unwrap_or_else(|| b.transparent(parent_format))
}

// ── Step 3 + 7: composition / node-graph lowering ─────────────────────────────

/// Lower a per-clip composition (08 §4): instantiate the graph, bind `ClipIn`
/// to the clip's source op, and return the node feeding `Output`. On a
/// missing-`Output`-input / cycle / type error, fall back to the plain source
/// and surface a diagnostic (02 §2 step 3, 08 §3.3 `Output` row).
#[allow(clippy::too_many_arguments)]
fn lower_composition(
    b: &mut Builder,
    project: &TimelineProject,
    seq: &Sequence,
    format_index: usize,
    format: &SequenceFormat,
    clip: &Clip,
    graph_id: GraphId,
    tick: Tick,
    quality: Quality,
    cycle: &mut HashSet<SequenceId>,
) -> IrNodeId {
    let Some(graph) = project.graphs.get(&graph_id) else {
        b.diag(CompileDiagnostic::plain(format!(
            "clip composition {graph_id} not found; using plain source"
        )));
        return build_clip_source(b, project, seq, format_index, format, clip, tick, quality, cycle);
    };

    let mut lc = LowerCtx {
        project,
        seq,
        format_index,
        format,
        quality,
        graph,
        clip: Some(clip),
        is_project_graph: false,
        program: None,
        offsets: HashSet::new(),
        memo: HashMap::new(),
    };
    match lower_output(b, &mut lc, tick, cycle) {
        Some(node) => node,
        None => {
            b.diag(CompileDiagnostic::at(
                graph_id,
                graph.output,
                "composition Output has no input; falling back to plain clip source",
            ));
            build_clip_source(b, project, seq, format_index, format, clip, tick, quality, cycle)
        }
    }
}

/// Step 6: splice the project graph between the fold result and `Output` (08 §5).
/// The project graph has no `ClipIn`; the program (fold result) enters wherever a
/// primary filter input is left unwired — so a bare `Grade → Output` or
/// `Vignette` graph applies to the final composite. An empty project graph
/// (`Output` with nothing wired) is a passthrough. A missing `Output` input with
/// no program to fall back on skips the splice (08 §3.3 `Output` row).
fn splice_project_graph(
    b: &mut Builder,
    project: &TimelineProject,
    program: Option<IrNodeId>,
    format: &SequenceFormat,
    tick: Tick,
) -> Option<IrNodeId> {
    let Some(graph_id) = project.project_graph else {
        return program;
    };
    let Some(graph) = project.graphs.get(&graph_id) else {
        b.diag(CompileDiagnostic::plain(format!(
            "project graph {graph_id} not found; skipping splice"
        )));
        return program;
    };

    let mut cycle = HashSet::new();
    let mut lc = LowerCtx {
        project,
        seq: project
            .active_sequence
            .and_then(|id| project.sequences.get(&id))
            .unwrap_or_else(|| unreachable_seq(project)),
        format_index: 0,
        format,
        quality: Quality::FULL,
        graph,
        clip: None,
        is_project_graph: true,
        program,
        offsets: HashSet::new(),
        memo: HashMap::new(),
    };
    match lower_output(b, &mut lc, tick, &mut cycle) {
        Some(node) => Some(node),
        None => {
            // Empty / unsatisfied project graph: skip the splice entirely.
            program
        }
    }
}

/// The active sequence is always present when a project graph splice runs (the
/// engine only compiles a real sequence); this helper keeps the borrow simple.
fn unreachable_seq(project: &TimelineProject) -> &Sequence {
    project
        .sequences
        .values()
        .next()
        .expect("project graph splice requires at least one sequence")
}

/// Per-instantiation lowering context for one graph (composition or project).
struct LowerCtx<'a> {
    project: &'a TimelineProject,
    seq: &'a Sequence,
    format_index: usize,
    format: &'a SequenceFormat,
    quality: Quality,
    graph: &'a NodeGraph,
    /// The host clip (`ClipIn` binds to its source); `None` for the project graph.
    clip: Option<&'a Clip>,
    is_project_graph: bool,
    /// The upstream program (fold result); the project-graph's unwired-input default.
    program: Option<IrNodeId>,
    /// Distinct `TimeOffset` values seen (soft-cap diagnostic, step 7).
    offsets: HashSet<i64>,
    /// Memo of `(node, eval-tick)` → IR id within this instantiation.
    memo: HashMap<(GraphNodeId, i64), IrNodeId>,
}

/// Lower the graph's `Output` node's single input at `tick`. Returns `None` when
/// `Output` has no wired input (08 §3.3 `Output` row).
fn lower_output(
    b: &mut Builder,
    lc: &mut LowerCtx,
    tick: Tick,
    cycle: &mut HashSet<SequenceId>,
) -> Option<IrNodeId> {
    let output_id = lc.graph.output;
    let src = input_source(lc.graph, output_id, InPort::PRIMARY)?;
    Some(lower_node(b, lc, src, tick, cycle))
}

/// Find the node feeding `(node, port)` in `graph`'s edge list.
fn input_source(graph: &NodeGraph, node: GraphNodeId, port: InPort) -> Option<GraphNodeId> {
    graph
        .edges
        .iter()
        .find(|e| e.to.0 == node && e.to.1 == port)
        .map(|e| e.from.0)
}

/// Lower one graph node at evaluation time `tick`, memoized per `(node, tick)`.
fn lower_node(
    b: &mut Builder,
    lc: &mut LowerCtx,
    node_id: GraphNodeId,
    tick: Tick,
    cycle: &mut HashSet<SequenceId>,
) -> IrNodeId {
    let key = (node_id, tick.0);
    if let Some(&id) = lc.memo.get(&key) {
        return id;
    }
    let Some(node) = lc.graph.nodes.get(&node_id) else {
        return b.transparent(lc.format);
    };

    let id = lower_node_uncached(b, lc, node, tick, cycle);
    lc.memo.insert(key, id);
    b.view_index.insert((lc.graph.id, node_id), id);
    id
}

fn lower_node_uncached(
    b: &mut Builder,
    lc: &mut LowerCtx,
    node: &GraphNode,
    tick: Tick,
    cycle: &mut HashSet<SequenceId>,
) -> IrNodeId {
    // Resolve the primary input, honouring the missing-input defaults (08 §3.3).
    let primary = || -> Option<GraphNodeId> { input_source(lc.graph, node.id, InPort::PRIMARY) };

    match &node.op {
        GraphOp::Output => {
            // Nested Output is unusual; treat like passthrough of its input.
            match primary() {
                Some(src) => lower_node(b, lc, src, tick, cycle),
                None => b.transparent(lc.format),
            }
        }
        GraphOp::ClipIn => {
            // Bind to the host clip's source op at the (possibly offset) tick.
            match lc.clip {
                Some(clip) => build_clip_source(
                    b,
                    lc.project,
                    lc.seq,
                    lc.format_index,
                    lc.format,
                    clip,
                    tick,
                    lc.quality,
                    cycle,
                ),
                None => {
                    // ClipIn is invalid in the project graph (08 §5): drop it.
                    b.diag(CompileDiagnostic::at(
                        lc.graph.id,
                        node.id,
                        "ClipIn is invalid in the project graph; substituting transparent",
                    ));
                    b.transparent(lc.format)
                }
            }
        }
        GraphOp::MediaIn { asset, .. } => {
            let kind = lc
                .project
                .media
                .assets
                .get(asset)
                .map(|a| a.kind)
                .unwrap_or(AssetKind::Video);
            match kind {
                AssetKind::Image => b.push(IrOp::DecodeStill { asset: *asset }, vec![]),
                _ => b.push(
                    IrOp::DecodeVideo {
                        asset: *asset,
                        src_time: tick,
                        proxy: lc.quality.proxy,
                    },
                    vec![],
                ),
            }
        }
        GraphOp::VectorIn { vref } => b.push(
            IrOp::RasterVector {
                vref: *vref,
                doc_state: vector_state_key_for_ref(*vref, lc.format, tick),
                w: lc.format.width,
                h: lc.format.height,
            },
            vec![],
        ),
        GraphOp::SolidColor => {
            let color = eval_node_color(&node.params, "params.color", Color::BLACK, tick);
            b.push(
                IrOp::SolidColor {
                    color: color_to_linear_premult(color),
                },
                vec![],
            )
        }
        GraphOp::Merge { mode } => {
            let a = input_source(lc.graph, node.id, InPort::A)
                .map(|s| lower_node(b, lc, s, tick, cycle));
            let bottom = input_source(lc.graph, node.id, InPort::B)
                .map(|s| lower_node(b, lc, s, tick, cycle));
            let opacity = eval_node_f32(&node.params, "params.opacity", 1.0, tick);
            match (a, bottom) {
                // Missing a → passthrough b; missing b → passthrough a (08 §3.3).
                (Some(a), Some(bt)) => b.push(
                    IrOp::Merge {
                        mode: *mode,
                        opacity,
                    },
                    vec![(a, OutPort::default()), (bt, OutPort::default())],
                ),
                (Some(a), None) => {
                    let bt = project_default_or_transparent(b, lc);
                    b.push(
                        IrOp::Merge {
                            mode: *mode,
                            opacity,
                        },
                        vec![(a, OutPort::default()), (bt, OutPort::default())],
                    )
                }
                (None, Some(bt)) => bt,
                (None, None) => project_default_or_transparent(b, lc),
            }
        }
        GraphOp::Transform2D => {
            let input = lower_primary_or_default(b, lc, primary(), tick, cycle);
            // Node transform params finalize with the node inspector (P8); P3
            // emits an identity Transform2D so the pass exists and shape is right.
            b.push(
                IrOp::Transform2D {
                    mat: Mat3::IDENTITY,
                    sampling: Sampling::Bilinear,
                },
                vec![(input, OutPort::default())],
            )
        }
        GraphOp::Crop => {
            let input = lower_primary_or_default(b, lc, primary(), tick, cycle);
            b.push(IrOp::Crop, vec![(input, OutPort::default())])
        }
        GraphOp::Resize { fit } => {
            let input = lower_primary_or_default(b, lc, primary(), tick, cycle);
            b.push(
                IrOp::Resize {
                    w: lc.format.width,
                    h: lc.format.height,
                    fit: map_fit(*fit),
                },
                vec![(input, OutPort::default())],
            )
        }
        GraphOp::Grade { grade } => {
            let input = lower_primary_or_default(b, lc, primary(), tick, cycle);
            apply_grade(b, grade, input, tick)
        }
        GraphOp::Lut { asset } => {
            // 08 §2: `Lut` lowers to a single-op `Grade{Lut3d}` — one mechanism,
            // not a parallel LUT path. In P3 the LUT asset resolves inert (no pool
            // at compile), so this is a passthrough until the lut_provider reaches
            // compile; the chain shape is correct now.
            let input = lower_primary_or_default(b, lc, primary(), tick, cycle);
            apply_grade(b, &single_lut_grade(*asset), input, tick)
        }
        GraphOp::Text { .. } => {
            // 08 §2: `Text` lowers to the dedicated `TextGen` IR op (a 0-input
            // styled-text generator). The glyphon raster is P8 (blocked on the
            // still-opaque `ResolvedTextBlock` payload); the evaluator emits a
            // transparent placeholder meanwhile.
            b.push(
                IrOp::TextGen {
                    block: ResolvedTextBlock::default(),
                },
                vec![],
            )
        }
        GraphOp::MaskFromMatte => {
            // 08 §2: lowers to the dedicated `MatteExtract` IR op (U²-Net subject
            // cutout). CPU inference is P8; the evaluator passes the input through
            // (a no-op/opaque mask) until photonic-matte is wired.
            let input = lower_primary_or_default(b, lc, primary(), tick, cycle);
            b.push(
                IrOp::MatteExtract {
                    model: MatteModel::U2NetP,
                },
                vec![(input, OutPort::default())],
            )
        }
        GraphOp::ChannelSplit => {
            // 08 §2: dedicated `ChannelSplit` IR op. The current single-output
            // lowering can't yet route the four (r/g/b/a) out-ports independently,
            // so it emits the alpha channel — the canonical alpha-as-mask output
            // (08 §2 note). Per-out-port routing lands with multi-output lowering.
            let input = lower_primary_or_default(b, lc, primary(), tick, cycle);
            b.push(
                IrOp::ChannelSplit { channel: Channel::A },
                vec![(input, OutPort::default())],
            )
        }
        GraphOp::ChannelCombine => {
            // 08 §2: dedicated `ChannelCombine` IR op. Full four-mask (r/g/b/a)
            // wiring needs per-in-port lowering; P3 feeds the primary input and
            // defaults the rest, matching the evaluator's passthrough.
            let input = lower_primary_or_default(b, lc, primary(), tick, cycle);
            b.push(IrOp::ChannelCombine, vec![(input, OutPort::default())])
        }
        GraphOp::TimeOffset { offset } => {
            // Step 7: re-lower the upstream subgraph at t − offset. Identical
            // (subgraph, time) dedups via content hash; distinct offsets are the
            // only cost. Soft-cap the distinct-offset count with a diagnostic.
            lc.offsets.insert(offset.0);
            if lc.offsets.len() > TIME_OFFSET_SOFT_CAP {
                b.diag(CompileDiagnostic::at(
                    lc.graph.id,
                    node.id,
                    format!(
                        "more than {TIME_OFFSET_SOFT_CAP} distinct TimeOffset values in one \
                         composition; echo/trail cost grows with each"
                    ),
                ));
            }
            let shifted = tick - *offset;
            match primary() {
                Some(src) => lower_node(b, lc, src, shifted, cycle),
                None => b.transparent(lc.format),
            }
        }
        GraphOp::Switch => {
            // `selected` resolves at compile time; P3 picks the primary input
            // (or first connected) — full selected-index eval lands with node
            // params (P8).
            match primary().or_else(|| lc.graph.edges.iter().find(|e| e.to.0 == node.id).map(|e| e.from.0)) {
                Some(src) => lower_node(b, lc, src, tick, cycle),
                None => b.transparent(lc.format),
            }
        }
        GraphOp::Note { .. } => {
            // Pure annotation, never compiled (08 §2); should be unreachable as an
            // ancestor of Output, but be defensive.
            b.transparent(lc.format)
        }
        // Filter/generator effects that lower to `IrOp::Effect`. `Invert` is a
        // real evaluator pass (08 §3); the rest keep arity + ordering +
        // content-hash identity as marker nodes until their `ResolvedParams`
        // payload finalizes (P5/P7). `MaskShape` is a 0-input generator; P3 still
        // routes the missing-input default through it (harmless — the evaluator
        // ignores it), pending generator-arity lowering.
        GraphOp::Blur
        | GraphOp::Sharpen
        | GraphOp::Glow
        | GraphOp::ChromaKey
        | GraphOp::LumaKey
        | GraphOp::Invert
        | GraphOp::MaskShape { .. } => {
            let input = lower_primary_or_default(b, lc, primary(), tick, cycle);
            b.push(
                IrOp::Effect {
                    kind: graph_op_effect_kind(&node.op),
                    params: ResolvedParams::default(),
                },
                vec![(input, OutPort::default())],
            )
        }
    }
}

/// Build a single-op [`Grade`] carrying one `Lut3d` op for the `Lut` graph node
/// (08 §2 — `Lut` is a one-op grade, not a parallel path).
fn single_lut_grade(asset: AssetId) -> Grade {
    let mut grade = Grade::new();
    grade.ops.push(GradeOp::new(
        GradeOpKind::Lut3d,
        GradeOpParams::Lut3d {
            asset,
            intensity: 1.0,
            interp: LutInterp::Trilinear,
        },
    ));
    grade
}

/// Lower a unary op's primary input, or the missing-input default: transparent
/// black for a composition, the program for the project graph (08 §3.3 unary row
/// / §5 program-splice).
fn lower_primary_or_default(
    b: &mut Builder,
    lc: &mut LowerCtx,
    primary: Option<GraphNodeId>,
    tick: Tick,
    cycle: &mut HashSet<SequenceId>,
) -> IrNodeId {
    match primary {
        Some(src) => lower_node(b, lc, src, tick, cycle),
        None => project_default_or_transparent(b, lc),
    }
}

/// The unwired-input default: the program (fold result) in the project graph,
/// transparent black otherwise.
fn project_default_or_transparent(b: &mut Builder, lc: &LowerCtx) -> IrNodeId {
    if lc.is_project_graph {
        if let Some(program) = lc.program {
            return program;
        }
    }
    b.push(
        IrOp::SolidColor {
            color: LinearColor {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            },
        },
        vec![],
    )
}

fn map_fit(fit: photonic_core::timeline::FitMode) -> FitMode {
    use photonic_core::timeline::FitMode as G;
    match fit {
        G::Stretch => FitMode::Stretch,
        G::Contain => FitMode::Fit,
        G::Cover => FitMode::Fill,
    }
}

/// Map a filter/generator `GraphOp` to its `EffectKind` for the `IrOp::Effect`
/// lowering (08 §2). Only the effect-family ops reach here; the non-effect
/// catalog entries (Grade/Lut/Text/MaskFromMatte/Channel*) lower to their own
/// dedicated IR ops in `lower_node_uncached`.
fn graph_op_effect_kind(op: &GraphOp) -> EffectKind {
    match op {
        GraphOp::Blur => EffectKind::Blur,
        GraphOp::Sharpen => EffectKind::Sharpen,
        GraphOp::Glow => EffectKind::Glow,
        GraphOp::ChromaKey => EffectKind::ChromaKey,
        GraphOp::LumaKey => EffectKind::LumaKey,
        GraphOp::Invert => EffectKind::Invert,
        GraphOp::MaskShape { .. } => EffectKind::MaskShapeGen,
        // Unreachable: only the effect-family ops above call this.
        _ => EffectKind::Blur,
    }
}

// ── Caption overlay (step 5) ──────────────────────────────────────────────────

/// Emit one `CaptionOverlay` per enabled caption track with a cue covering `t`
/// (06 §5.3: one node per active track per compiled frame). Each node carries a
/// [`CaptionBatch`] whose words are fully cascade-resolved and whose
/// karaoke/animation state is baked at this tick — so the evaluator stays
/// time-ignorant (02 §2). Tracks with no covering cue contribute nothing; a
/// `None` program (captions over an empty sequence) roots on transparent black.
fn splice_captions(
    b: &mut Builder,
    seq: &Sequence,
    format: &SequenceFormat,
    tick: Tick,
    program: Option<IrNodeId>,
) -> Option<IrNodeId> {
    let mut cur = program;
    for track in &seq.caption_tracks {
        if !track.enabled {
            continue;
        }
        let batch = resolve_caption_batch(track, tick, format);
        if batch.cues.is_empty() {
            continue;
        }
        let input = cur.unwrap_or_else(|| b.transparent(format));
        cur = Some(b.push(
            IrOp::CaptionOverlay { cue_batch: batch },
            vec![(input, OutPort::default())],
        ));
    }
    cur
}

/// Resolve a caption track's cues covering `tick` into a [`CaptionBatch`] of
/// positioned, styled, karaoke-resolved word runs for the render text pipeline
/// (06 §5). Cues are non-overlapping (01 §4), so v1 collects at most one — the
/// loop stays general. Deterministic in `tick` (no wall-clock).
fn resolve_caption_batch(track: &CaptionTrack, tick: Tick, format: &SequenceFormat) -> CaptionBatch {
    let mut cues = Vec::new();
    for cue in &track.cues {
        if cue.start <= tick && tick < cue.end {
            if let Some(run) = resolve_cue(track, cue, tick, format) {
                cues.push(run);
            }
        }
    }
    CaptionBatch { cues }
}

/// Resolve one covering cue at `tick`. Style cascades word → cue → track (01 §7,
/// each override a complete [`CaptionStyle`]); karaoke colour (06 §5.1) and
/// animation state (06 §5.2) are baked here. Returns `None` if nothing is visible
/// (e.g. Typewriter before the first character reveals).
fn resolve_cue(
    track: &CaptionTrack,
    cue: &CaptionCue,
    tick: Tick,
    _format: &SequenceFormat,
) -> Option<CaptionCueRun> {
    let cue_style: &CaptionStyle = cue.style_override.as_ref().unwrap_or(&track.style);
    let anim = cue_style.animation;

    // SlideUp (06 §5.2): whole-cue fade + upward translate over the first 200 ms,
    // both baked deterministically at compile time.
    let (cue_opacity, y_shift) = match anim {
        CaptionAnim::SlideUp => {
            let dur = ms_to_ticks(200).max(1);
            let p = ((tick - cue.start).0 as f32 / dur as f32).clamp(0.0, 1.0);
            (p, (1.0 - p) * 0.05) // slides up from +5% of frame height into place
        }
        _ => (1.0, 0.0),
    };

    let base_pos = cue.position_override.unwrap_or(cue_style.position);
    let anchor = [base_pos[0], base_pos[1] + y_shift];

    let mut words = Vec::with_capacity(cue.words.len());
    for w in &cue.words {
        // Word-level effective style: word → cue → track (each a full style).
        let eff: &CaptionStyle = w
            .style_override
            .as_ref()
            .or(cue.style_override.as_ref())
            .unwrap_or(&track.style);
        let text = reveal_text(&w.text, anim, w, tick);
        let mut color = karaoke_color(eff, w, tick);
        let mut opacity = cue_opacity;
        if let CaptionAnim::FadeWords = anim {
            opacity *= fade_word_opacity(w, tick);
        }
        color[3] = (color[3] as f32 * opacity).round().clamp(0.0, 255.0) as u8;
        words.push(CaptionWordRun {
            text,
            font_family: eff.font_family.clone(),
            font_weight: eff.weight,
            color,
        });
    }
    if words.iter().all(|w| w.text.is_empty()) {
        return None;
    }
    Some(CaptionCueRun {
        words,
        font_size: cue_style.font_size.max(1.0),
        line_height_mul: 1.2,
        anchor,
        max_width: cue_style.max_width,
    })
}

/// Resolve a title/text clip's [`TextClipContent`] (G-12) into a
/// [`ResolvedTextBlock`] carrying a single styled [`CaptionCueRun`] — reusing the
/// caption glyph payload (06 §5.3) so titles render through the one
/// `TextGen`/glyphon mechanism. The clip's [`CaptionStyle`] (shared caption
/// styling vocabulary) supplies font / size / fill / position / wrap width; the
/// whole string is one word run (glyphon shapes embedded spaces itself). An empty
/// string resolves to `None` (nothing to shape ⇒ transparent). `TextClipContent`
/// carries a static style in v1, so there is no tick-varying prop to bake yet —
/// when keyframed title props land they resolve here, exactly as captions bake at
/// the compiled tick (02 §2).
fn resolve_text_block(content: &TextClipContent) -> ResolvedTextBlock {
    if content.text.is_empty() {
        return ResolvedTextBlock::default();
    }
    let style: &CaptionStyle = &content.style;
    ResolvedTextBlock {
        cue: Some(CaptionCueRun {
            words: vec![CaptionWordRun {
                text: content.text.clone(),
                font_family: style.font_family.clone(),
                font_weight: style.weight,
                color: color_to_srgb_bytes(style.fill),
            }],
            font_size: style.font_size.max(1.0),
            line_height_mul: 1.2,
            anchor: style.position,
            max_width: style.max_width,
        }),
    }
}

/// Milliseconds → ticks. `TICKS_PER_SECOND` is a multiple of 1000, so this is
/// exact (no rounding) — keeping animation timing deterministic.
fn ms_to_ticks(ms: i64) -> i64 {
    (TICKS_PER_SECOND / 1000) * ms
}

/// The word's fill colour at `tick`, resolved through its karaoke highlight
/// (06 §5.1), as sRGB straight RGBA bytes. Without a highlight it is the plain
/// fill. FillSweep's intra-glyph split is approximated at word granularity by
/// colour-lerping the sweeping word by the sweep fraction (the true per-glyph
/// split is a render follow-up).
fn karaoke_color(style: &CaptionStyle, w: &CaptionWord, tick: Tick) -> [u8; 4] {
    let Some(k) = style.highlight else {
        return color_to_srgb_bytes(style.fill);
    };
    let c = match k.mode {
        KaraokeMode::WordPop => {
            if w.start <= tick && tick < w.end {
                k.active_color
            } else {
                k.inactive_color
            }
        }
        // Glyph keeps its fill; the underline decoration is a render follow-up.
        KaraokeMode::Underline => style.fill,
        KaraokeMode::FillSweep => {
            if tick < w.start {
                k.inactive_color
            } else if tick >= w.end {
                k.active_color // already-spoken words stay active (standard karaoke read)
            } else {
                let span = (w.end - w.start).0.max(1) as f32;
                let f = ((tick - w.start).0 as f32 / span).clamp(0.0, 1.0);
                lerp_color(k.inactive_color, k.active_color, f)
            }
        }
    };
    color_to_srgb_bytes(c)
}

/// FadeWords opacity (06 §5.2): ramps `0 → 1` over a 150 ms lead-in ending at
/// `w.start`, then holds at `1`.
fn fade_word_opacity(w: &CaptionWord, tick: Tick) -> f32 {
    let lead = ms_to_ticks(150).max(1);
    let start = w.start.0 - lead;
    ((tick.0 - start) as f32 / lead as f32).clamp(0.0, 1.0)
}

/// Typewriter reveal (06 §5.2): the first
/// `floor(char_count * clamp((t − start)/(end − start), 0, 1))` characters of the
/// word; the full text for any other animation.
fn reveal_text(text: &str, anim: CaptionAnim, w: &CaptionWord, tick: Tick) -> String {
    if !matches!(anim, CaptionAnim::Typewriter) {
        return text.to_string();
    }
    let span = (w.end - w.start).0.max(1) as f32;
    let f = ((tick - w.start).0 as f32 / span).clamp(0.0, 1.0);
    let total = text.chars().count();
    let n = (total as f32 * f).floor() as usize;
    text.chars().take(n).collect()
}

fn lerp_color(a: Color, b: Color, f: f32) -> Color {
    let l = |x: f32, y: f32| x + (y - x) * f;
    Color {
        r: l(a.r, b.r),
        g: l(a.g, b.g),
        b: l(a.b, b.b),
        a: l(a.a, b.a),
    }
}

fn color_to_srgb_bytes(c: Color) -> [u8; 4] {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    [q(c.r), q(c.g), q(c.b), q(c.a)]
}

// ── Keyframe resolution ───────────────────────────────────────────────────────

/// Evaluate an `AnimProps<ClipTransform>` at clip-relative `t` (all eight fields).
fn eval_clip_transform(anim: &AnimProps<ClipTransform>, t: Tick) -> ClipTransform {
    if anim.is_static() {
        return anim.base;
    }
    let base = anim.base;
    ClipTransform {
        x: eval_prop_f64(anim, "transform.x", base.x, t),
        y: eval_prop_f64(anim, "transform.y", base.y, t),
        scale_x: eval_prop_f64(anim, "transform.scale_x", base.scale_x, t),
        scale_y: eval_prop_f64(anim, "transform.scale_y", base.scale_y, t),
        rotation: eval_prop_f64(anim, "transform.rotation", base.rotation, t),
        anchor_x: eval_prop_f64(anim, "transform.anchor_x", base.anchor_x, t),
        anchor_y: eval_prop_f64(anim, "transform.anchor_y", base.anchor_y, t),
        opacity: eval_prop_f64(anim, "transform.opacity", base.opacity, t),
    }
}

/// Evaluate one `f64` property lane at `t`, mirroring the mixer's helper.
fn eval_prop_f64<T: timeline::PropSet>(anim: &AnimProps<T>, path: &str, base: f64, t: Tick) -> f64 {
    match anim.track(&PropPath::new(path)) {
        Some(track) => match timeline::eval(track, &PropValue::Float(base), t) {
            PropValue::Float(v) => v,
            _ => base,
        },
        None => base,
    }
}

/// Evaluate a graph node's `f32` param lane at `t` (base value from
/// `EffectParams`, overridden by an animated lane when present).
fn eval_node_f32(anim: &AnimProps<GraphNodeParams>, path: &str, default: f32, t: Tick) -> f32 {
    let base = match anim.base.0.get(path) {
        Some(PropValue::Float(v)) => *v as f32,
        _ => default,
    };
    match anim.track(&PropPath::new(path)) {
        Some(track) => match timeline::eval(track, &PropValue::Float(base as f64), t) {
            PropValue::Float(v) => v as f32,
            _ => base,
        },
        None => base,
    }
}

/// Evaluate a graph node's `Color` param lane at `t`.
fn eval_node_color(anim: &AnimProps<GraphNodeParams>, path: &str, default: Color, t: Tick) -> Color {
    let base = match anim.base.0.get(path) {
        Some(PropValue::Color(c)) => *c,
        _ => default,
    };
    match anim.track(&PropPath::new(path)) {
        Some(track) => match timeline::eval(track, &PropValue::Color(base), t) {
            PropValue::Color(c) => c,
            _ => base,
        },
        None => base,
    }
}

// ── Transforms & color ────────────────────────────────────────────────────────

/// Build a 3×3 affine from an evaluated [`ClipTransform`]: translate about the
/// anchor, rotate, scale, un-anchor, then position. Opacity is not geometric —
/// it drives the fold `Merge` opacity, not this matrix. (Exact reframe/anchor
/// pixel semantics finalize with the UI story; identity in / identity out here.)
fn clip_transform_matrix(t: &ClipTransform) -> Mat3 {
    let anchor = Vec2::new(t.anchor_x as f32, t.anchor_y as f32);
    let pos = Vec2::new(t.x as f32, t.y as f32);
    let scale = Vec2::new(t.scale_x as f32, t.scale_y as f32);
    Mat3::from_translation(pos)
        * Mat3::from_translation(anchor)
        * Mat3::from_angle(t.rotation as f32)
        * Mat3::from_scale(scale)
        * Mat3::from_translation(-anchor)
}

/// sRGB EOTF (gamma → scene-linear), the standard breakpoint form. A vector/
/// solid `Color` is authored in the sRGB display domain; the video graph works
/// in scene-linear premultiplied Rec.709 (D-09), so convert on the way in.
/// (03 §3.3 wants all transfer-function math consolidated into
/// `photonic-core::color`; that refactor is out of P3 scope — this mirrors the
/// existing `raster/adjust.rs` curve exactly.)
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Convert an sRGB straight-alpha [`Color`] to premultiplied scene-linear
/// [`LinearColor`] (D-09).
fn color_to_linear_premult(c: Color) -> LinearColor {
    let a = c.a.clamp(0.0, 1.0);
    LinearColor {
        r: srgb_to_linear(c.r) * a,
        g: srgb_to_linear(c.g) * a,
        b: srgb_to_linear(c.b) * a,
        a,
    }
}

// ── Vector reference resolution ───────────────────────────────────────────────

/// Resolve the `VectorRef` for a `ClipSource::Vector` asset: an embedded-vector
/// asset carries its own `VectorRef`; a file-backed one references the whole
/// external document.
fn vector_ref_for(project: &TimelineProject, asset: AssetId) -> VectorRef {
    use photonic_core::timeline::AssetSource;
    match project.media.assets.get(&asset).map(|a| &a.source) {
        Some(AssetSource::EmbeddedVector { root }) => *root,
        _ => VectorRef::WholeDocument,
    }
}

/// A `VectorStateKey` for a rasterized vector frame (02 §3 / 03 §2.5). The
/// full key hashes referenced-node state + evaluated animated props + size; P3
/// keys on `(vref discriminant, size, src_time)`, which is stable and correct
/// for a non-animated vector doc and conservative (never stale) for an animated
/// one. Referenced-node-state hashing lands with the vector-animation story.
fn vector_state_key(asset: AssetId, format: &SequenceFormat, src_time: Tick) -> VectorStateKey {
    vector_state_key_for_ref(VectorRef::WholeDocument, format, src_time).combine(asset.0.as_u128())
}

fn vector_state_key_for_ref(vref: VectorRef, format: &SequenceFormat, src_time: Tick) -> VectorStateKey {
    use xxhash_rust::xxh3::Xxh3;
    let mut h = Xxh3::new();
    h.update(&[vref_tag(&vref)]);
    match vref {
        VectorRef::Artboard(i) => h.update(&(i as u64).to_le_bytes()),
        VectorRef::Node(id) => h.update(&id.as_u128().to_le_bytes()),
        VectorRef::WholeDocument => {}
    }
    h.update(&format.width.to_le_bytes());
    h.update(&format.height.to_le_bytes());
    h.update(&src_time.0.to_le_bytes());
    VectorStateKey(h.digest128())
}

fn vref_tag(v: &VectorRef) -> u8 {
    match v {
        VectorRef::Artboard(_) => 0,
        VectorRef::Node(_) => 1,
        VectorRef::WholeDocument => 2,
    }
}

trait CombineKey {
    fn combine(self, extra: u128) -> Self;
}
impl CombineKey for VectorStateKey {
    fn combine(self, extra: u128) -> Self {
        use xxhash_rust::xxh3::Xxh3;
        let mut h = Xxh3::new();
        h.update(&self.0.to_le_bytes());
        h.update(&extra.to_le_bytes());
        VectorStateKey(h.digest128())
    }
}

// ── Content hashing ───────────────────────────────────────────────────────────

/// Content hash of `(op discriminant, resolved params, input hashes)` — the
/// cache identity of a node's result (02 §5). xxh3-128; deterministic across
/// runs (no `Instant`/random state).
pub fn content_hash(op: &IrOp, inputs: &[(IrNodeId, OutPort)], input_hashes: &[u128]) -> ContentHash {
    use xxhash_rust::xxh3::Xxh3;
    let mut h = Xxh3::new();
    hash_op(&mut h, op);
    for ((_, port), ih) in inputs.iter().zip(input_hashes) {
        h.update(&[port.0]);
        h.update(&ih.to_le_bytes());
    }
    ContentHash(h.digest128())
}

fn hash_op(h: &mut xxhash_rust::xxh3::Xxh3, op: &IrOp) {
    // A per-variant tag byte, then the resolved params in a fixed order. The
    // remaining opaque resolved-param stubs (`ResolvedParams`, `ResolvedTextBlock`)
    // contribute nothing until their payloads land — extend here when they do.
    // `CaptionOverlay`'s resolved batch (incl. baked karaoke colours) IS hashed,
    // so a mid-word highlight change is a distinct cache identity (06 §5).
    let f32b = |h: &mut xxhash_rust::xxh3::Xxh3, v: f32| h.update(&v.to_bits().to_le_bytes());
    match op {
        IrOp::DecodeVideo { asset, src_time, proxy } => {
            h.update(&[0]);
            h.update(&asset.0.as_u128().to_le_bytes());
            h.update(&src_time.0.to_le_bytes());
            h.update(&[*proxy as u8]);
        }
        IrOp::DecodeStill { asset } => {
            h.update(&[1]);
            h.update(&asset.0.as_u128().to_le_bytes());
        }
        IrOp::RasterVector { vref, doc_state, w, h: gh } => {
            h.update(&[2]);
            h.update(&[vref_tag(vref)]);
            h.update(&doc_state.0.to_le_bytes());
            h.update(&w.to_le_bytes());
            h.update(&gh.to_le_bytes());
        }
        IrOp::SolidColor { color } => {
            h.update(&[3]);
            f32b(h, color.r);
            f32b(h, color.g);
            f32b(h, color.b);
            f32b(h, color.a);
        }
        IrOp::Transform2D { mat, sampling } => {
            h.update(&[4]);
            for v in mat.to_cols_array() {
                f32b(h, v);
            }
            h.update(&[*sampling as u8]);
        }
        IrOp::Effect { kind, params: _ } => {
            h.update(&[5]);
            h.update(&[effect_kind_tag(*kind)]);
        }
        IrOp::Grade { ops } => {
            h.update(&[6]);
            h.update(&(ops.len() as u32).to_le_bytes());
            for op in ops {
                hash_resolved_grade_op(h, op);
            }
        }
        IrOp::Merge { mode, opacity } => {
            h.update(&[7]);
            h.update(&[*mode as u8]);
            f32b(h, *opacity);
        }
        IrOp::CaptionOverlay { cue_batch } => {
            h.update(&[8]);
            hash_caption_batch(h, cue_batch);
        }
        IrOp::Crop => {
            h.update(&[9]);
        }
        IrOp::Resize { w, h: gh, fit } => {
            h.update(&[10]);
            h.update(&w.to_le_bytes());
            h.update(&gh.to_le_bytes());
            h.update(&[*fit as u8]);
        }
        IrOp::MatteExtract { model } => {
            h.update(&[11]);
            h.update(&[*model as u8]);
        }
        IrOp::TextGen { block } => {
            h.update(&[12]);
            // The resolved cue (text, font, colour, layout) IS hashed, so distinct
            // titles are distinct cache identities; an empty/absent cue hashes as a
            // bare tag (transparent).
            match &block.cue {
                Some(cue) => {
                    h.update(&[1]);
                    hash_caption_cue(h, cue);
                }
                None => h.update(&[0]),
            }
        }
        IrOp::ChannelSplit { channel } => {
            h.update(&[13]);
            h.update(&[*channel as u8]);
        }
        IrOp::ChannelCombine => {
            h.update(&[14]);
        }
        IrOp::Output { w, h: gh } => {
            h.update(&[15]);
            h.update(&w.to_le_bytes());
            h.update(&gh.to_le_bytes());
        }
    }
}

/// Hash a resolved [`CaptionBatch`] (06 §5.3) into the content hash: cue layout
/// plus every word's text, font, weight, and baked karaoke colour. This is what
/// makes a karaoke sweep re-render — the same cue at two ticks resolves to
/// different word colours ⇒ different hash ⇒ a cache miss ⇒ a fresh composite.
/// Deterministic: only resolved bytes/f32 bits, no pointer/time state.
fn hash_caption_batch(h: &mut xxhash_rust::xxh3::Xxh3, batch: &CaptionBatch) {
    h.update(&(batch.cues.len() as u32).to_le_bytes());
    for cue in &batch.cues {
        hash_caption_cue(h, cue);
    }
}

/// Hash one resolved [`CaptionCueRun`] — layout plus every word's text, font,
/// weight, and baked colour — into the content hash. Shared by
/// [`hash_caption_batch`] (06 §5.3 `CaptionOverlay`) and the `TextGen` block
/// (G-12 title clips), so both text-raster paths key their cache identically.
fn hash_caption_cue(h: &mut xxhash_rust::xxh3::Xxh3, cue: &CaptionCueRun) {
    let f32b = |h: &mut xxhash_rust::xxh3::Xxh3, v: f32| h.update(&v.to_bits().to_le_bytes());
    f32b(h, cue.font_size);
    f32b(h, cue.line_height_mul);
    f32b(h, cue.anchor[0]);
    f32b(h, cue.anchor[1]);
    f32b(h, cue.max_width);
    h.update(&(cue.words.len() as u32).to_le_bytes());
    for w in &cue.words {
        h.update(&(w.text.len() as u32).to_le_bytes());
        h.update(w.text.as_bytes());
        h.update(&(w.font_family.len() as u32).to_le_bytes());
        h.update(w.font_family.as_bytes());
        h.update(&w.font_weight.to_le_bytes());
        h.update(&w.color);
    }
}

/// Hash one resolved grade op (payload + mask) into the content hash so distinct
/// grades never collide in the node-result cache (02 §5). Deterministic: only
/// resolved f32 params + discriminants, no `Instant`/pointer state.
fn hash_resolved_grade_op(h: &mut xxhash_rust::xxh3::Xxh3, op: &crate::contract::ResolvedGradeOp) {
    use photonic_render::grade::ResolvedGradePayload as P;
    let f32b = |h: &mut xxhash_rust::xxh3::Xxh3, v: f32| h.update(&v.to_bits().to_le_bytes());
    let cdl = |h: &mut xxhash_rust::xxh3::Xxh3, c: &photonic_render::grade::ResolvedCdl| {
        for v in c.slope.iter().chain(&c.offset).chain(&c.power) {
            f32b(h, *v);
        }
        f32b(h, c.sat);
    };
    match &op.mask {
        None => h.update(&[0]),
        Some(m) => {
            h.update(&[1, m.rectangle as u8, m.invert as u8]);
            for v in [
                m.center[0], m.center[1], m.size[0], m.size[1], m.rotation, m.softness,
            ] {
                f32b(h, v);
            }
        }
    }
    match &op.payload {
        P::Exposure { stops } => {
            h.update(&[0]);
            f32b(h, *stops);
        }
        P::Contrast { pivot, amount } => {
            h.update(&[1]);
            f32b(h, *pivot);
            f32b(h, *amount);
        }
        P::WhiteBalance { temp, tint } => {
            h.update(&[2]);
            f32b(h, *temp);
            f32b(h, *tint);
        }
        P::Cdl(c) => {
            h.update(&[3]);
            cdl(h, c);
        }
        P::Curves(c) => {
            h.update(&[4]);
            for arr in [&c.master, &c.red, &c.green, &c.blue] {
                for v in arr.iter() {
                    f32b(h, *v);
                }
            }
            for opt in [&c.hue_vs_hue, &c.hue_vs_sat] {
                match opt {
                    Some(a) => {
                        h.update(&[1]);
                        for v in a.iter() {
                            f32b(h, *v);
                        }
                    }
                    None => h.update(&[0]),
                }
            }
        }
        P::HslQualifier(q) => {
            h.update(&[5]);
            for v in [
                q.hue[0], q.hue[1], q.sat[0], q.sat[1], q.lum[0], q.lum[1], q.softness,
            ] {
                f32b(h, v);
            }
            cdl(h, &q.correction);
        }
        P::Lut3d(l) => {
            h.update(&[6]);
            f32b(h, l.intensity);
            h.update(&[l.tetrahedral as u8]);
            h.update(&(l.table.size as u32).to_le_bytes());
            for v in l.table.domain_min.iter().chain(&l.table.domain_max) {
                f32b(h, *v);
            }
        }
    }
}

fn effect_kind_tag(k: EffectKind) -> u8 {
    // `EffectKind` is `#[non_exhaustive]`; the wildcard covers effects added
    // after P3. Distinct tags matter only for cache-hash disambiguation, so a
    // future variant sharing tag 255 is a benign (rare) cache miss, never a
    // correctness bug — extend this map when new effects land.
    match k {
        EffectKind::Blur => 0,
        EffectKind::Sharpen => 1,
        EffectKind::Glow => 2,
        EffectKind::ChromaKey => 3,
        EffectKind::LumaKey => 4,
        EffectKind::Invert => 5,
        EffectKind::MaskShapeGen => 6,
        _ => 255,
    }
}

/// A convenience for a full [`TextureDesc`] from a format (used by callers that
/// want to pre-size the output pool bucket).
pub fn output_desc(format: &SequenceFormat) -> TextureDesc {
    TextureDesc {
        width: format.width,
        height: format.height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photonic_core::timeline::{
        Clip, FrameRate, GraphEdge, GraphNode, GraphOp, InPort, MediaAsset, NodeGraph, OutPort as GOutPort,
        Sequence, Track, TrackKind,
    };
    use photonic_core::Color;

    fn base_project() -> (TimelineProject, SequenceId) {
        let mut project = TimelineProject::new();
        let seq = Sequence::new("seq", FrameRate::FPS_30, 320, 180);
        let id = seq.id;
        project.insert_sequence(seq);
        (project, id)
    }

    fn add_video_track(project: &mut TimelineProject, seq_id: SequenceId) -> usize {
        let seq = project.sequences.get_mut(&seq_id).unwrap();
        seq.video_tracks.push(Track::new(TrackKind::Video, "V1"));
        seq.video_tracks.len() - 1
    }

    fn solid_clip(color: Color, start: i64, dur: i64) -> Clip {
        Clip::new(ClipSource::SolidColor { color }, Tick(start), Tick(dur))
    }

    #[test]
    fn bare_solid_clip_is_solid_transform_output() {
        let (mut project, seq_id) = base_project();
        let tk = add_video_track(&mut project, seq_id);
        project.sequences.get_mut(&seq_id).unwrap().video_tracks[tk]
            .clips
            .push(solid_clip(Color::BLACK, 0, Tick::from_seconds(2).0));

        let out = compile(&project, seq_id, 0, Tick(0), Quality::PREVIEW, None);
        assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
        // Single opaque track ⇒ no Merge. Chain: SolidColor → Transform2D → Output.
        let ops: Vec<&str> = out.graph.nodes.iter().map(|n| op_name(&n.op)).collect();
        assert_eq!(ops, vec!["SolidColor", "Transform2D", "Output"]);
        let output = out.graph.output.unwrap();
        assert!(matches!(out.graph.nodes[output.0 as usize].op, IrOp::Output { .. }));
    }

    #[test]
    fn empty_sequence_outputs_transparent() {
        let (project, seq_id) = base_project();
        let out = compile(&project, seq_id, 0, Tick(0), Quality::PREVIEW, None);
        // Only a transparent SolidColor feeding Output.
        let ops: Vec<&str> = out.graph.nodes.iter().map(|n| op_name(&n.op)).collect();
        assert_eq!(ops, vec!["SolidColor", "Output"]);
    }

    #[test]
    fn two_opaque_tracks_fold_with_one_merge() {
        let (mut project, seq_id) = base_project();
        for _ in 0..2 {
            let tk = add_video_track(&mut project, seq_id);
            project.sequences.get_mut(&seq_id).unwrap().video_tracks[tk]
                .clips
                .push(solid_clip(Color::BLACK, 0, Tick::from_seconds(2).0));
        }
        let out = compile(&project, seq_id, 0, Tick(0), Quality::PREVIEW, None);
        let merges = out
            .graph
            .nodes
            .iter()
            .filter(|n| matches!(n.op, IrOp::Merge { .. }))
            .count();
        assert_eq!(merges, 1, "two tracks fold with exactly one Merge");
    }

    #[test]
    fn opacity_zero_clip_is_dead_branch() {
        let (mut project, seq_id) = base_project();
        let tk = add_video_track(&mut project, seq_id);
        let mut clip = solid_clip(Color::BLACK, 0, Tick::from_seconds(2).0);
        clip.transform.base.opacity = 0.0;
        project.sequences.get_mut(&seq_id).unwrap().video_tracks[tk]
            .clips
            .push(clip);
        let out = compile(&project, seq_id, 0, Tick(0), Quality::PREVIEW, None);
        // The invisible clip folds away → empty program → transparent → Output.
        let ops: Vec<&str> = out.graph.nodes.iter().map(|n| op_name(&n.op)).collect();
        assert_eq!(ops, vec!["SolidColor", "Output"]);
    }

    #[test]
    fn adjustment_clip_rewraps_the_composite_below() {
        let (mut project, seq_id) = base_project();
        // Bottom track: a solid.
        let tk0 = add_video_track(&mut project, seq_id);
        project.sequences.get_mut(&seq_id).unwrap().video_tracks[tk0]
            .clips
            .push(solid_clip(Color::BLACK, 0, Tick::from_seconds(2).0));
        // Top track: an Adjustment clip with an effect → re-roots the stack below.
        let tk1 = add_video_track(&mut project, seq_id);
        let mut adj = Clip::new(ClipSource::Adjustment, Tick(0), Tick::from_seconds(2));
        adj.effects
            .push(photonic_core::timeline::ClipEffect::new(EffectKind::Blur));
        project.sequences.get_mut(&seq_id).unwrap().video_tracks[tk1]
            .clips
            .push(adj);

        let out = compile(&project, seq_id, 0, Tick(0), Quality::PREVIEW, None);
        // The Effect node's input is the bottom solid (re-root), and no Merge is
        // introduced by the Adjustment (it replaces the accumulator).
        let effect = out
            .graph
            .nodes
            .iter()
            .find(|n| matches!(n.op, IrOp::Effect { .. }))
            .expect("adjustment effect node present");
        let input = effect.inputs[0].0;
        // Its input chain traces back to the bottom clip through that clip's
        // own Transform2D (re-root, not a fresh source).
        assert!(matches!(
            out.graph.nodes[input.0 as usize].op,
            IrOp::Transform2D { .. }
        ));
        assert!(!out.graph.nodes.iter().any(|n| matches!(n.op, IrOp::Merge { .. })));
    }

    #[test]
    fn composition_splices_clip_source_and_keeps_chain() {
        let (mut project, seq_id) = base_project();
        let tk = add_video_track(&mut project, seq_id);
        // A ClipIn → Output composition (the fixed v1 seed).
        let (graph, _clip_in) = NodeGraph::new_clip_composition("comp");
        let gid = graph.id;
        project.graphs.insert(gid, graph);
        let mut clip = solid_clip(Color::BLACK, 0, Tick::from_seconds(2).0);
        clip.composition = Some(gid);
        project.sequences.get_mut(&seq_id).unwrap().video_tracks[tk]
            .clips
            .push(clip);

        let out = compile(&project, seq_id, 0, Tick(0), Quality::PREVIEW, None);
        assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
        // ClipIn binds to the solid source; the still-applied Transform2D rides
        // on top, so the shape matches the plain-clip chain.
        let ops: Vec<&str> = out.graph.nodes.iter().map(|n| op_name(&n.op)).collect();
        assert_eq!(ops, vec!["SolidColor", "Transform2D", "Output"]);
    }

    #[test]
    fn composition_with_merge_over_program() {
        // Composition: SolidColor(a) merged over ClipIn(b) → Output. The comp's
        // SolidColor is WHITE and the host clip's source is BLACK, so the two
        // sources stay distinct (identical colors would content-hash dedup to a
        // single node — see `identical_solid_colors_dedup_to_one_node`; that is
        // correct behaviour, so this fixture keeps them distinct on purpose to
        // prove the Merge really pulls a *second* source).
        let (mut project, seq_id) = base_project();
        let tk = add_video_track(&mut project, seq_id);

        let clip_in = GraphNode::new(GraphOp::ClipIn);
        let mut solid = GraphNode::new(GraphOp::SolidColor);
        solid.params.base.0.set(
            "params.color",
            PropValue::Color(Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }),
        );
        let merge = GraphNode::new(GraphOp::Merge { mode: BlendMode::Normal });
        let output = GraphNode::new(GraphOp::Output);
        let (ci, so, mg, ou) = (clip_in.id, solid.id, merge.id, output.id);
        let mut nodes = std::collections::HashMap::new();
        for n in [clip_in, solid, merge, output] {
            nodes.insert(n.id, n);
        }
        let graph = NodeGraph {
            id: GraphId::new(),
            name: "comp".into(),
            nodes,
            edges: vec![
                GraphEdge { from: (so, GOutPort::PRIMARY), to: (mg, InPort::A) },
                GraphEdge { from: (ci, GOutPort::PRIMARY), to: (mg, InPort::B) },
                GraphEdge { from: (mg, GOutPort::PRIMARY), to: (ou, InPort::PRIMARY) },
            ],
            output: ou,
            ui: std::collections::HashMap::new(),
        };
        let gid = graph.id;
        project.graphs.insert(gid, graph);
        let mut clip = solid_clip(Color::BLACK, 0, Tick::from_seconds(2).0);
        clip.composition = Some(gid);
        project.sequences.get_mut(&seq_id).unwrap().video_tracks[tk]
            .clips
            .push(clip);

        let out = compile(&project, seq_id, 0, Tick(0), Quality::PREVIEW, None);
        assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);

        // Exactly one Merge — the composition's. A single fully-opaque clip folds
        // without a track-fold Merge, so this is unambiguously the comp's node.
        let merge_idx = out
            .graph
            .nodes
            .iter()
            .position(|n| matches!(n.op, IrOp::Merge { .. }))
            .expect("composition Merge present");
        assert_eq!(
            out.graph.nodes.iter().filter(|n| matches!(n.op, IrOp::Merge { .. })).count(),
            1,
            "only the composition's Merge — no spurious track-fold Merge"
        );

        // The Merge pulls two DISTINCT sources: the comp's white SolidColor and
        // the clip's black source bound via ClipIn (02 §2 step 3 — ClipIn binds
        // to the host clip's source op).
        let merge = &out.graph.nodes[merge_idx];
        assert_eq!(merge.inputs.len(), 2, "binary Merge has both inputs wired");
        for (input, _) in &merge.inputs {
            assert!(
                matches!(out.graph.nodes[input.0 as usize].op, IrOp::SolidColor { .. }),
                "each Merge input is a SolidColor source"
            );
        }
        let solids = out
            .graph
            .nodes
            .iter()
            .filter(|n| matches!(n.op, IrOp::SolidColor { .. }))
            .count();
        assert_eq!(solids, 2, "comp SolidColor + clip source via ClipIn stay distinct");

        // Source-substitution (08 §4): the composition replaces ONLY the source
        // op; the still-applied default chain rides on top. So the IR Output's
        // input is the clip's default Transform2D, and that Transform2D's input
        // is the composition's Merge — the comp's Output feeds the default chain,
        // it does not become the terminal output itself.
        let ir_out = out.graph.output.expect("compiled Output present");
        let xf = out.graph.nodes[ir_out.0 as usize].inputs[0].0;
        assert!(
            matches!(out.graph.nodes[xf.0 as usize].op, IrOp::Transform2D { .. }),
            "default Transform2D rides on top of the composition Output"
        );
        let xf_in = out.graph.nodes[xf.0 as usize].inputs[0].0;
        assert_eq!(
            xf_in.0 as usize, merge_idx,
            "the still-applied default chain sits directly on the composition's Merge"
        );
    }

    #[test]
    fn project_graph_filter_applies_to_program() {
        let (mut project, seq_id) = base_project();
        let tk = add_video_track(&mut project, seq_id);
        project.sequences.get_mut(&seq_id).unwrap().video_tracks[tk]
            .clips
            .push(solid_clip(Color::BLACK, 0, Tick::from_seconds(2).0));

        // Project graph: Blur → Output; Blur's input is unwired, so the program
        // (fold result) feeds it (08 §5 program-splice).
        let blur = GraphNode::new(GraphOp::Blur);
        let output = GraphNode::new(GraphOp::Output);
        let (bl, ou) = (blur.id, output.id);
        let mut nodes = std::collections::HashMap::new();
        nodes.insert(bl, blur);
        nodes.insert(ou, output);
        let pg = NodeGraph {
            id: GraphId::new(),
            name: "pg".into(),
            nodes,
            edges: vec![GraphEdge { from: (bl, GOutPort::PRIMARY), to: (ou, InPort::PRIMARY) }],
            output: ou,
            ui: std::collections::HashMap::new(),
        };
        let pgid = pg.id;
        project.graphs.insert(pgid, pg);
        project.project_graph = Some(pgid);

        let out = compile(&project, seq_id, 0, Tick(0), Quality::PREVIEW, None);
        assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
        // An Effect (Blur) node whose input traces to the clip's Transform2D.
        let effect = out
            .graph
            .nodes
            .iter()
            .find(|n| matches!(n.op, IrOp::Effect { .. }))
            .expect("project-graph filter present");
        let input = effect.inputs[0].0;
        assert!(matches!(
            out.graph.nodes[input.0 as usize].op,
            IrOp::Transform2D { .. }
        ));
        // Output's input is the filter (splice sits between program and Output).
        let output = out.graph.output.unwrap();
        let out_in = out.graph.nodes[output.0 as usize].inputs[0].0;
        assert!(matches!(out.graph.nodes[out_in.0 as usize].op, IrOp::Effect { .. }));
    }

    #[test]
    fn nested_sequence_cycle_is_guarded() {
        let (mut project, seq_id) = base_project();
        // A self-nesting clip: sequence contains a clip whose source is itself.
        let tk = add_video_track(&mut project, seq_id);
        let nested = Clip::new(
            ClipSource::NestedSequence { sequence: seq_id },
            Tick(0),
            Tick::from_seconds(2),
        );
        project.sequences.get_mut(&seq_id).unwrap().video_tracks[tk]
            .clips
            .push(nested);
        let out = compile(&project, seq_id, 0, Tick(0), Quality::PREVIEW, None);
        assert!(
            out.diagnostics.iter().any(|d| d.message.contains("cycle")),
            "self-nesting must produce a cycle diagnostic, got {:?}",
            out.diagnostics
        );
    }

    /// A `ClipSource::NestedSequence` clip splices its inner sequence's composite
    /// as the clip's *source* (02 §2 step 3 / CAP-005). The inner sequence here is
    /// a real 2-clip composite — an opaque red backdrop under a half-opacity blue —
    /// and it must ride through the outer clip unchanged, proving the recursive
    /// compile lowers the inner program (not a transparent fallback / single clip).
    #[test]
    fn nested_sequence_composites_inner_as_one_clip() {
        let mut project = TimelineProject::new();

        // Inner sequence: bottom opaque red, top blue at 0.5 opacity. Premultiplied
        // linear `over` (0 and 1 map through sRGB→linear unchanged):
        //   top_eff = (0,0,1,1)·0.5 = (0,0,0.5,0.5)
        //   out     = top_eff + red·(1−0.5) = (0.5, 0, 0.5, 1.0)
        let mut inner = Sequence::new("inner", FrameRate::FPS_30, 4, 4);
        let inner_id = inner.id;
        let mut bottom = Track::new(TrackKind::Video, "V1");
        bottom.clips.push(solid_clip(
            Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 },
            0,
            Tick::from_seconds(2).0,
        ));
        let mut top = Track::new(TrackKind::Video, "V2");
        let mut blue = solid_clip(Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 }, 0, Tick::from_seconds(2).0);
        blue.transform.base.opacity = 0.5;
        top.clips.push(blue);
        inner.video_tracks.push(bottom);
        inner.video_tracks.push(top);
        project.insert_sequence(inner);

        // Outer sequence: one clip whose source is the inner sequence.
        let mut outer = Sequence::new("outer", FrameRate::FPS_30, 4, 4);
        let outer_id = outer.id;
        let mut ot = Track::new(TrackKind::Video, "V1");
        ot.clips.push(Clip::new(
            ClipSource::NestedSequence { sequence: inner_id },
            Tick(0),
            Tick::from_seconds(2),
        ));
        outer.video_tracks.push(ot);
        project.insert_sequence(outer);

        let out = compile(&project, outer_id, 0, Tick(0), Quality::FULL, None);
        assert!(
            out.diagnostics.is_empty(),
            "clean nested compile has no diagnostics, got {:?}",
            out.diagnostics
        );
        // Both inner clips were lowered as the nested source (two distinct solids),
        // not collapsed to a single clip or a transparent fallback.
        let solids = out
            .graph
            .nodes
            .iter()
            .filter(|n| matches!(n.op, IrOp::SolidColor { .. }))
            .count();
        assert!(solids >= 2, "inner composite lowers both source solids, got {solids}");

        // The composited pixels match the inner sequence's blend, evaluated through
        // the deterministic CPU reference (03 §6).
        let img = crate::graph::eval_cpu::evaluate(
            &out.graph,
            (4, 4),
            &mut crate::graph::eval_cpu::EmptyProvider,
        );
        for p in &img.pixels {
            assert!((p[0] - 0.5).abs() < 1e-4, "r={}", p[0]);
            assert!(p[1].abs() < 1e-4, "g={}", p[1]);
            assert!((p[2] - 0.5).abs() < 1e-4, "b={}", p[2]);
            assert!((p[3] - 1.0).abs() < 1e-4, "a={}", p[3]);
        }
    }

    #[test]
    fn identical_solid_colors_dedup_to_one_node() {
        // Two tracks with the exact same solid color: content-hash dedup means
        // one SolidColor node feeds both fold inputs.
        let (mut project, seq_id) = base_project();
        for _ in 0..2 {
            let tk = add_video_track(&mut project, seq_id);
            let mut clip = solid_clip(Color { r: 0.2, g: 0.4, b: 0.6, a: 1.0 }, 0, Tick::from_seconds(2).0);
            // Make the top one semi-transparent so a Merge is forced but the
            // SolidColor + Transform2D still dedup.
            clip.transform.base.opacity = 1.0;
            project.sequences.get_mut(&seq_id).unwrap().video_tracks[tk]
                .clips
                .push(clip);
        }
        let out = compile(&project, seq_id, 0, Tick(0), Quality::PREVIEW, None);
        let solids = out
            .graph
            .nodes
            .iter()
            .filter(|n| matches!(n.op, IrOp::SolidColor { .. }))
            .count();
        // Both tracks share one deduped SolidColor and one deduped Transform2D.
        assert_eq!(solids, 1, "identical solids dedup");
    }

    #[test]
    fn compile_is_deterministic_across_runs() {
        let (mut project, seq_id) = base_project();
        let tk = add_video_track(&mut project, seq_id);
        project.sequences.get_mut(&seq_id).unwrap().video_tracks[tk]
            .clips
            .push(solid_clip(Color { r: 0.1, g: 0.2, b: 0.3, a: 1.0 }, 0, Tick::from_seconds(2).0));
        let a = compile(&project, seq_id, 0, Tick(500), Quality::PREVIEW, None);
        let b = compile(&project, seq_id, 0, Tick(500), Quality::PREVIEW, None);
        let ha: Vec<u128> = a.graph.nodes.iter().map(|n| n.content_hash.0).collect();
        let hb: Vec<u128> = b.graph.nodes.iter().map(|n| n.content_hash.0).collect();
        assert_eq!(ha, hb, "content hashes are run-stable");
    }

    #[test]
    fn asset_video_clip_emits_decode_video_at_mapped_src_time() {
        let (mut project, seq_id) = base_project();
        let asset = MediaAsset::from_file(AssetKind::Video, "/tmp/x.mp4");
        let aid = asset.id;
        project.media.insert(asset);
        let tk = add_video_track(&mut project, seq_id);
        let mut clip = Clip::new(ClipSource::Asset { asset: aid }, Tick(0), Tick::from_seconds(4));
        clip.source_in = Tick(1000);
        project.sequences.get_mut(&seq_id).unwrap().video_tracks[tk]
            .clips
            .push(clip);
        let out = compile(&project, seq_id, 0, Tick(500), Quality::PREVIEW, None);
        let decode = out
            .graph
            .nodes
            .iter()
            .find_map(|n| match &n.op {
                IrOp::DecodeVideo { src_time, proxy, .. } => Some((*src_time, *proxy)),
                _ => None,
            })
            .expect("decode node present");
        // src_time = source_in(1000) + (tick 500 − clip.start 0) at 1× speed = 1500.
        assert_eq!(decode.0, Tick(1500));
        assert!(decode.1, "preview quality requests proxy");
    }

    #[test]
    fn distinct_grades_produce_distinct_hashes() {
        use photonic_core::timeline::grade::{Grade, GradeOp, GradeOpKind, GradeOpParams};
        let grade_node_hash = |stops: f32| -> u128 {
            let (mut project, seq_id) = base_project();
            let tk = add_video_track(&mut project, seq_id);
            let mut clip = solid_clip(Color { r: 0.5, g: 0.5, b: 0.5, a: 1.0 }, 0, Tick::from_seconds(2).0);
            let mut grade = Grade::new();
            grade.ops.push(GradeOp::new(
                GradeOpKind::Exposure,
                GradeOpParams::Exposure { stops },
            ));
            clip.grade = Some(grade);
            project.sequences.get_mut(&seq_id).unwrap().video_tracks[tk]
                .clips
                .push(clip);
            let out = compile(&project, seq_id, 0, Tick(0), Quality::FULL, None);
            out.graph
                .nodes
                .iter()
                .find_map(|n| match &n.op {
                    IrOp::Grade { .. } => Some(n.content_hash.0),
                    _ => None,
                })
                .expect("grade node present")
        };
        // Different resolved params ⇒ different content hash (cache-correct).
        assert_ne!(grade_node_hash(1.0), grade_node_hash(2.0));
        // Same params ⇒ stable hash (determinism, 02 §2).
        assert_eq!(grade_node_hash(1.5), grade_node_hash(1.5));
    }

    #[test]
    fn text_node_lowers_to_textgen() {
        // Project graph: Text → Output. `Text` is a 0-input generator lowering to
        // the dedicated `TextGen` IR op (08 §2).
        let (mut project, seq_id) = base_project();
        let tk = add_video_track(&mut project, seq_id);
        project.sequences.get_mut(&seq_id).unwrap().video_tracks[tk]
            .clips
            .push(solid_clip(Color::BLACK, 0, Tick::from_seconds(2).0));

        let text = GraphNode::new(GraphOp::Text {
            text: photonic_core::timeline::TextGen::default(),
        });
        let output = GraphNode::new(GraphOp::Output);
        let (tx, ou) = (text.id, output.id);
        let mut nodes = std::collections::HashMap::new();
        nodes.insert(tx, text);
        nodes.insert(ou, output);
        let pg = NodeGraph {
            id: GraphId::new(),
            name: "pg".into(),
            nodes,
            edges: vec![GraphEdge { from: (tx, GOutPort::PRIMARY), to: (ou, InPort::PRIMARY) }],
            output: ou,
            ui: std::collections::HashMap::new(),
        };
        let pgid = pg.id;
        project.graphs.insert(pgid, pg);
        project.project_graph = Some(pgid);

        let out = compile(&project, seq_id, 0, Tick(0), Quality::PREVIEW, None);
        assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
        let output = out.graph.output.unwrap();
        let out_in = out.graph.nodes[output.0 as usize].inputs[0].0;
        assert!(
            matches!(out.graph.nodes[out_in.0 as usize].op, IrOp::TextGen { .. }),
            "Text lowered to TextGen feeding Output"
        );
    }

    #[test]
    fn channel_and_matte_nodes_lower_to_dedicated_ir() {
        // Composition: ClipIn → ChannelSplit → MaskFromMatte → Output. Verifies
        // both dedicated IR lowerings (08 §2 §3.4) land as ChannelSplit +
        // MatteExtract, not generic Effect nodes.
        let (mut project, seq_id) = base_project();
        let tk = add_video_track(&mut project, seq_id);

        let clip_in = GraphNode::new(GraphOp::ClipIn);
        let split = GraphNode::new(GraphOp::ChannelSplit);
        let matte = GraphNode::new(GraphOp::MaskFromMatte);
        let output = GraphNode::new(GraphOp::Output);
        let (ci, sp, ma, ou) = (clip_in.id, split.id, matte.id, output.id);
        let mut nodes = std::collections::HashMap::new();
        for n in [clip_in, split, matte, output] {
            nodes.insert(n.id, n);
        }
        let graph = NodeGraph {
            id: GraphId::new(),
            name: "comp".into(),
            nodes,
            edges: vec![
                GraphEdge { from: (ci, GOutPort::PRIMARY), to: (sp, InPort::PRIMARY) },
                GraphEdge { from: (sp, GOutPort::PRIMARY), to: (ma, InPort::PRIMARY) },
                GraphEdge { from: (ma, GOutPort::PRIMARY), to: (ou, InPort::PRIMARY) },
            ],
            output: ou,
            ui: std::collections::HashMap::new(),
        };
        let gid = graph.id;
        project.graphs.insert(gid, graph);
        let mut clip = solid_clip(Color::BLACK, 0, Tick::from_seconds(2).0);
        clip.composition = Some(gid);
        project.sequences.get_mut(&seq_id).unwrap().video_tracks[tk]
            .clips
            .push(clip);

        let out = compile(&project, seq_id, 0, Tick(0), Quality::PREVIEW, None);
        assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
        assert!(
            out.graph.nodes.iter().any(|n| matches!(n.op, IrOp::ChannelSplit { .. })),
            "ChannelSplit lowered to its dedicated IR op"
        );
        assert!(
            out.graph.nodes.iter().any(|n| matches!(n.op, IrOp::MatteExtract { .. })),
            "MaskFromMatte lowered to MatteExtract"
        );
        assert!(
            !out.graph.nodes.iter().any(|n| matches!(n.op, IrOp::Effect { .. })),
            "no generic Effect placeholders for these ops"
        );
    }

    #[test]
    fn dip_to_black_midpoint_is_black() {
        // Two adjacent solids; the second dips-to-black in. At the exact midpoint
        // the frame is black (through-color), regardless of either clip's color.
        let (mut project, seq_id) = base_project();
        let tk = add_video_track(&mut project, seq_id);
        let clips = &mut project.sequences.get_mut(&seq_id).unwrap().video_tracks[tk].clips;
        clips.push(Clip::new(
            ClipSource::SolidColor { color: Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 } },
            Tick(0),
            Tick(100),
        ));
        let mut b = Clip::new(
            ClipSource::SolidColor { color: Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 } },
            Tick(100),
            Tick(100),
        );
        b.transition_in = Some(photonic_core::timeline::Transition::new(
            TransitionKind::DipToBlack,
            Tick(40),
        ));
        clips.push(b);

        // t=120 → raw 0.5 → EaseInOut 0.5 → second phase opacity 0 → pure black.
        let out = compile(&project, seq_id, 0, Tick(120), Quality::FULL, None);
        let img = crate::graph::eval_cpu::evaluate(
            &out.graph,
            (4, 4),
            &mut crate::graph::eval_cpu::EmptyProvider,
        );
        for p in &img.pixels {
            for (c, &v) in p[..3].iter().enumerate() {
                assert!(v.abs() < 1e-4, "dip midpoint black, channel {c} = {v}");
            }
        }
    }

    // ── Caption overlay resolution (06 §5) ────────────────────────────────────

    use photonic_core::timeline::{
        CaptionCue, CaptionStyle, CaptionTrack, CaptionWord, KaraokeMode, KaraokeStyle,
    };

    /// Fetch the single `CaptionOverlay` node's resolved batch + content hash.
    fn caption_node(graph: &FrameGraph) -> (&CaptionBatch, ContentHash) {
        let n = graph
            .nodes
            .iter()
            .find(|n| matches!(n.op, IrOp::CaptionOverlay { .. }))
            .expect("a CaptionOverlay node is present");
        match &n.op {
            IrOp::CaptionOverlay { cue_batch } => (cue_batch, n.content_hash),
            _ => unreachable!(),
        }
    }

    fn wordpop_track() -> CaptionTrack {
        let mut track = CaptionTrack::new("Captions");
        track.style = CaptionStyle {
            highlight: Some(KaraokeStyle {
                mode: KaraokeMode::WordPop,
                active_color: Color { r: 1.0, g: 1.0, b: 0.0, a: 1.0 }, // yellow
                inactive_color: Color { r: 0.5, g: 0.5, b: 0.5, a: 1.0 }, // grey
            }),
            ..CaptionStyle::default()
        };
        // "hello" [0,100), "world" [100,200); cue [0,200).
        let cue = CaptionCue::new(
            Tick(0),
            Tick(200),
            vec![
                CaptionWord::new("hello", Tick(0), Tick(100)),
                CaptionWord::new("world", Tick(100), Tick(200)),
            ],
        );
        track.cues.push(cue);
        track
    }

    /// A covering cue on an enabled caption track lowers to a `CaptionOverlay`
    /// carrying a populated batch (not the old empty default) — the un-stubbing.
    #[test]
    fn covering_cue_populates_caption_batch() {
        let (mut project, seq_id) = base_project();
        let tk = add_video_track(&mut project, seq_id);
        project.sequences.get_mut(&seq_id).unwrap().video_tracks[tk]
            .clips
            .push(solid_clip(Color::BLACK, 0, 400));
        project
            .sequences
            .get_mut(&seq_id)
            .unwrap()
            .caption_tracks
            .push(wordpop_track());

        let out = compile(&project, seq_id, 0, Tick(50), Quality::FULL, None);
        assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
        assert!(
            out.graph.nodes.iter().any(|n| matches!(n.op, IrOp::CaptionOverlay { .. })),
            "graph has a CaptionOverlay node"
        );
        let (batch, _) = caption_node(&out.graph);
        assert_eq!(batch.cues.len(), 1, "one covering cue resolved");
        let cue = &batch.cues[0];
        assert_eq!(cue.words.len(), 2);
        assert_eq!(cue.words[0].text, "hello");
        assert_eq!(cue.words[1].text, "world");
        // Anchor is the track style's default caption position (01 §7).
        assert_eq!(cue.anchor, CaptionStyle::default().position);
    }

    /// No enabled caption track / no covering cue ⇒ no CaptionOverlay node.
    #[test]
    fn no_cue_emits_no_caption_overlay() {
        let (mut project, seq_id) = base_project();
        let tk = add_video_track(&mut project, seq_id);
        project.sequences.get_mut(&seq_id).unwrap().video_tracks[tk]
            .clips
            .push(solid_clip(Color::BLACK, 0, 400));
        project
            .sequences
            .get_mut(&seq_id)
            .unwrap()
            .caption_tracks
            .push(wordpop_track());
        // Tick 300 is past the cue's [0,200) span.
        let out = compile(&project, seq_id, 0, Tick(300), Quality::FULL, None);
        assert!(
            !out.graph.nodes.iter().any(|n| matches!(n.op, IrOp::CaptionOverlay { .. })),
            "no covering cue ⇒ no CaptionOverlay"
        );
        // A disabled track never overlays even when a cue covers the tick.
        project
            .sequences
            .get_mut(&seq_id)
            .unwrap()
            .caption_tracks[0]
            .enabled = false;
        let out = compile(&project, seq_id, 0, Tick(50), Quality::FULL, None);
        assert!(
            !out.graph.nodes.iter().any(|n| matches!(n.op, IrOp::CaptionOverlay { .. })),
            "disabled caption track ⇒ no CaptionOverlay"
        );
    }

    /// WordPop karaoke (06 §5.1): the word whose window contains `t` renders in
    /// `active_color`, the others in `inactive_color`; and the resolved batch's
    /// content hash changes across ticks so the node-result cache re-renders the
    /// sweep (02 §5). Before/mid the second word must differ.
    #[test]
    fn wordpop_karaoke_recolors_and_rehashes() {
        let (mut project, seq_id) = base_project();
        let tk = add_video_track(&mut project, seq_id);
        project.sequences.get_mut(&seq_id).unwrap().video_tracks[tk]
            .clips
            .push(solid_clip(Color::BLACK, 0, 400));
        project
            .sequences
            .get_mut(&seq_id)
            .unwrap()
            .caption_tracks
            .push(wordpop_track());

        let active = [255, 255, 0, 255]; // yellow
        let inactive = [128, 128, 128, 255]; // grey (0.5 → 128)

        let at = |t: i64| compile(&project, seq_id, 0, Tick(t), Quality::FULL, None).graph;

        // t=50: word0 ("hello") active, word1 ("world") inactive.
        let g_before = at(50);
        let (b0, h0) = caption_node(&g_before);
        assert_eq!(b0.cues[0].words[0].color, active, "hello active at t=50");
        assert_eq!(b0.cues[0].words[1].color, inactive, "world inactive at t=50");

        // t=150: swap — word1 ("world") active, word0 inactive.
        let g_mid = at(150);
        let (b1, h1) = caption_node(&g_mid);
        assert_eq!(b1.cues[0].words[0].color, inactive, "hello inactive at t=150");
        assert_eq!(b1.cues[0].words[1].color, active, "world active at t=150");

        // The sweep must change the CaptionOverlay content hash (drives re-render).
        assert_ne!(h0, h1, "karaoke sweep changes the CaptionOverlay content hash");
    }

    fn op_name(op: &IrOp) -> &'static str {
        match op {
            IrOp::DecodeVideo { .. } => "DecodeVideo",
            IrOp::DecodeStill { .. } => "DecodeStill",
            IrOp::RasterVector { .. } => "RasterVector",
            IrOp::SolidColor { .. } => "SolidColor",
            IrOp::Transform2D { .. } => "Transform2D",
            IrOp::Effect { .. } => "Effect",
            IrOp::Grade { .. } => "Grade",
            IrOp::Merge { .. } => "Merge",
            IrOp::CaptionOverlay { .. } => "CaptionOverlay",
            IrOp::Crop => "Crop",
            IrOp::Resize { .. } => "Resize",
            IrOp::MatteExtract { .. } => "MatteExtract",
            IrOp::TextGen { .. } => "TextGen",
            IrOp::ChannelSplit { .. } => "ChannelSplit",
            IrOp::ChannelCombine => "ChannelCombine",
            IrOp::Output { .. } => "Output",
        }
    }
}
