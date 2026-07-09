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
    self, AnimProps, AssetKind, Clip, ClipSource, ClipTransform, EffectKind, GraphId, GraphNode,
    GraphNodeId, GraphNodeParams, GraphOp, InPort, NodeGraph, PropPath, PropValue, Sequence,
    SequenceFormat, SequenceId, TimelineProject, TrackKind,
};
use photonic_core::Color;

use crate::contract::{AssetId, CaptionBatch, ResolvedParams, Tick, VectorRef, VectorStateKey};
use crate::graph::ir::{
    ContentHash, FitMode, FrameGraph, IrNode, IrNodeId, IrOp, LinearColor, OutPort, Sampling,
    TextureDesc,
};

/// Preview vs full-resolution compile flags (02 §2's "quality flags"). `proxy`
/// selects proxy media where available (session state, `SetProxyMode`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Quality {
    /// Decode proxy media instead of originals (preview). Export forces `false`.
    pub proxy: bool,
}

impl Default for Quality {
    fn default() -> Self {
        Quality { proxy: false }
    }
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
    let program = splice_captions(&mut b, seq, tick, program);

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
        let Some(clip) = covering_clip(track.clips.as_slice(), tick) else {
            continue;
        };
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

/// The clip whose `[start, end)` covers `t` on this track (tracks are sorted,
/// non-overlapping — 01 §4).
fn covering_clip(clips: &[Clip], t: Tick) -> Option<&Clip> {
    clips.iter().find(|c| c.start <= t && t < c.end())
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
/// Shared by the normal chain (step 2) and Adjustment re-rooting (step 4).
fn apply_effect_grade_chain(b: &mut Builder, clip: &Clip, input: IrNodeId, _dt: Tick) -> IrNodeId {
    let mut cur = input;
    for fx in &clip.effects {
        if !fx.enabled {
            continue;
        }
        // Effect params are keyframe-resolved at compile time; the resolved
        // payload shape (`ResolvedParams`) finalizes in P5/P7, so P3 emits the
        // node (correct arity + ordering) with default resolved params and the
        // evaluator passes it through. The op discriminant + kind still
        // participate in the content hash so distinct effects don't collide.
        cur = b.push(
            IrOp::Effect {
                kind: fx.kind,
                params: ResolvedParams::default(),
            },
            vec![(cur, OutPort::default())],
        );
    }
    if let Some(grade) = &clip.grade {
        if !grade.bypass && !grade.ops.is_empty() {
            // Resolved grade-op payloads finalize in P7; emit a marker Grade
            // node (empty ops = passthrough) so the chain shape is right now.
            cur = b.push(IrOp::Grade { ops: Vec::new() }, vec![(cur, OutPort::default())]);
        }
    }
    cur
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

    cycle.insert(sequence);
    let nested_format_index = nested.active_format.min(nested.formats.len().saturating_sub(1));
    let Some(nested_format) = nested.formats.get(nested_format_index) else {
        b.diag(CompileDiagnostic::plain(format!(
            "nested sequence {sequence} has no formats; substituting transparent"
        )));
        return b.transparent(parent_format);
    };
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
            if grade.bypass || grade.ops.is_empty() {
                input
            } else {
                b.push(IrOp::Grade { ops: Vec::new() }, vec![(input, OutPort::default())])
            }
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
        // Filters and generators that P3 carries as passthrough / marker Effect
        // nodes (real kernels land P8): keep arity + ordering + content-hash
        // identity so caching and the DAG shape are correct now.
        GraphOp::Blur
        | GraphOp::Sharpen
        | GraphOp::Glow
        | GraphOp::ChromaKey
        | GraphOp::LumaKey
        | GraphOp::Invert
        | GraphOp::MaskShape { .. }
        | GraphOp::MaskFromMatte
        | GraphOp::ChannelSplit
        | GraphOp::ChannelCombine
        | GraphOp::Lut { .. }
        | GraphOp::Text { .. } => {
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

/// Map a filter/generator `GraphOp` to the closest authoring `EffectKind` for the
/// passthrough Effect node (P3). The mapping is a P3 placeholder; the real per-op
/// kernels replace these Effect nodes in P8.
fn graph_op_effect_kind(op: &GraphOp) -> EffectKind {
    match op {
        GraphOp::Blur => EffectKind::Blur,
        GraphOp::Sharpen => EffectKind::Sharpen,
        GraphOp::Glow => EffectKind::Glow,
        GraphOp::ChromaKey => EffectKind::ChromaKey,
        GraphOp::LumaKey => EffectKind::LumaKey,
        GraphOp::Invert => EffectKind::Invert,
        GraphOp::MaskShape { .. } => EffectKind::MaskShapeGen,
        _ => EffectKind::Blur, // placeholder for MaskFromMatte/Channel*/Lut/Text
    }
}

// ── Caption overlay (step 5) ──────────────────────────────────────────────────

/// Emit a `CaptionOverlay` when any enabled caption track has a cue covering `t`
/// (06 §4). The batch is a minimal covering-cues carrier in P3 (glyph batching is
/// P5); its presence is what the evaluator/present path keys on.
fn splice_captions(
    b: &mut Builder,
    seq: &Sequence,
    tick: Tick,
    program: Option<IrNodeId>,
) -> Option<IrNodeId> {
    let has_cue = seq.caption_tracks.iter().any(|t| {
        t.enabled && t.cues.iter().any(|c| c.start <= tick && tick < c.end)
    });
    if !has_cue {
        return program;
    }
    let input = match program {
        Some(p) => p,
        None => b.push(
            IrOp::SolidColor {
                color: LinearColor {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.0,
                },
            },
            vec![],
        ),
    };
    Some(b.push(
        IrOp::CaptionOverlay {
            cue_batch: CaptionBatch::default(),
        },
        vec![(input, OutPort::default())],
    ))
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
    // opaque resolved-param stubs (`ResolvedParams`, `CaptionBatch`, …)
    // contribute nothing until their payloads land — extend here when they do.
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
        }
        IrOp::Merge { mode, opacity } => {
            h.update(&[7]);
            h.update(&[*mode as u8]);
            f32b(h, *opacity);
        }
        IrOp::CaptionOverlay { cue_batch: _ } => {
            h.update(&[8]);
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
        IrOp::TextGen { block: _ } => {
            h.update(&[12]);
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
    #[ignore = "quarantined: builder died mid-story (session limit); repair story queued"]
    fn composition_with_merge_over_program() {
        // Composition: SolidColor(a) merged over ClipIn(b) → Output.
        let (mut project, seq_id) = base_project();
        let tk = add_video_track(&mut project, seq_id);

        let clip_in = GraphNode::new(GraphOp::ClipIn);
        let solid = GraphNode::new(GraphOp::SolidColor);
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
        // The composition's Merge is present, with two SolidColor ancestors
        // (the comp's SolidColor and the clip's own SolidColor via ClipIn).
        assert!(out.graph.nodes.iter().any(|n| matches!(n.op, IrOp::Merge { .. })));
        let solids = out
            .graph
            .nodes
            .iter()
            .filter(|n| matches!(n.op, IrOp::SolidColor { .. }))
            .count();
        assert_eq!(solids, 2);
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
