//! Load-time timeline finalization (01 §4 invariant enforcement, §6.2 orphaned
//! paths).
//!
//! `Document` derives `Deserialize`, so raw serde cannot run the timeline
//! invariants. Instead [`Document::from_value`](crate::document::Document::from_value)
//! — the single load seam shared by `from_json` and the `.photon` wrapper —
//! calls [`finalize_load`] on the deserialized project. Two passes run there:
//!
//! 1. **Orphan flagging (repair).** Every [`PropertyTrack`] whose
//!    [`PropPath`](super::anim::PropPath) does not resolve in
//!    [`prop_registry`](super::prop_registry) for its owning target kind is
//!    flagged [`orphaned`](super::anim::PropertyTrack::orphaned) rather than
//!    dropped (01 §6.2): an asset/plugin may register the path later, and eval
//!    falls back to `base`. This is idempotent and re-derived on every load, so a
//!    path that becomes known un-orphans automatically.
//!
//! 2. **Sequence validation (reject).** Each loaded [`Sequence`] must satisfy
//!    [`Sequence::validate`] (sorted, non-overlapping, `duration > 0`). A file
//!    whose on-disk clips overlap is *rejected* with a load error rather than
//!    silently repaired: closing a gap requires an editorial decision (which clip
//!    moves, by how much) that the loader cannot make without risking data loss,
//!    so failing loudly is safer than guessing.
//!
//! The effect/grade/audio-fx param bags all declare
//! [`PropSet::TARGET_KIND`](super::anim::PropSet::TARGET_KIND) `= GraphNode`
//! (their generic surface is intentionally lenient), so orphan resolution here
//! keys off the *concrete* kind field (`EffectKind`, `GradeOpKind`, `AudioFxKind`)
//! rather than the generic bound — that is where the real registry blocks live.

use super::anim::PropertyTrack;
use super::clip::ClipSource;
use super::grade::GradeOpParams;
use super::graph::GraphOp;
use super::ids::{ClipId, GradeOpId, GraphId, GraphNodeId, SequenceId, TrackId};
use super::prop_registry::{self, PropTargetKind};
use super::sequence::{TimelineProject, ValidationError};

/// The error returned by [`finalize_load`]: either a per-sequence invariant
/// failure or a corrupt-known-variant rejection (39 §2.2 rule 4). Kept in this
/// crate as a plain `Display` error — [`Document::from_value`] maps it straight
/// into a `serde_json::Error` via `serde::de::Error::custom`.
///
/// [`Document::from_value`]: crate::document::Document::from_value
// `Eq` is intentionally NOT derived: the `Validation` arm wraps
// `ValidationError`, which is not `Eq`. `PartialEq` is sufficient for the
// `assert_eq!`-based tests and for `Document::from_value`'s error mapping.
#[derive(Clone, Debug, PartialEq)]
pub enum LoadError {
    /// A loaded [`Sequence`](super::sequence::Sequence) failed its invariants
    /// (sorted / non-overlapping / positive duration).
    Validation(ValidationError),
    /// A KNOWN internally-tagged variant's payload was malformed and serde's
    /// greedy per-variant `#[serde(untagged)] Unknown` fallback swallowed it,
    /// retaining the KNOWN tag. That is data corruption, not a forward-compat
    /// variant, so the load rejects it rather than rendering it inert.
    CorruptKnownVariant {
        /// The enum whose known variant was corrupted (`"ClipSource"`,
        /// `"GraphOp"`, `"GradeOpParams"`).
        enum_name: &'static str,
        /// The known serialized tag whose payload failed to deserialize.
        tag: String,
    },
}

impl LoadError {
    fn corrupt(enum_name: &'static str, tag: &str) -> Self {
        LoadError::CorruptKnownVariant {
            enum_name,
            tag: tag.to_owned(),
        }
    }
}

impl From<ValidationError> for LoadError {
    fn from(e: ValidationError) -> Self {
        LoadError::Validation(e)
    }
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Validation(e) => write!(f, "{e}"),
            LoadError::CorruptKnownVariant { enum_name, tag } => write!(
                f,
                "corrupt {enum_name}: the known variant `{tag}` has a malformed \
                 payload — this build cannot load it, and it is not a \
                 forward-compatible unknown variant (39 §2.2)"
            ),
        }
    }
}

impl std::error::Error for LoadError {}

/// Flag every track in `tracks` orphaned iff its path is unknown for `kind`.
/// Re-derived from scratch each call (a previously-orphaned path that is now
/// registered clears its flag, and vice versa).
fn flag_tracks(tracks: &mut [PropertyTrack], kind: PropTargetKind) {
    for t in tracks {
        t.orphaned = prop_registry::resolve(kind, t.property.as_str()).is_none();
    }
}

/// Run the load-time passes over a freshly deserialized project: flag orphaned
/// property tracks (repair), reject any corrupt known variant serde swallowed
/// into the `Unknown` fallback (39 §2.2 rule 4), scan for preserved unknown enum
/// variants (diagnose once — 39 §2.2 rule 3), and validate every sequence
/// (reject on violation).
///
/// The returned [`LoadReport`] carries the unknown-variant findings so a caller
/// can surface them exactly once per load; existing callers that discard it are
/// unaffected.
pub fn finalize_load(project: &mut TimelineProject) -> Result<LoadReport, LoadError> {
    finalize_effect_ids(project);
    flag_orphaned_property_tracks(project);
    // Integrity guard BEFORE diagnosing/validating: a retained `Unknown` whose
    // tag is a KNOWN catalog tag is corrupt data (a malformed known variant that
    // serde swallowed), not a forward-compat variant — reject it (39 §2.2 rule 4).
    reject_corrupt_known_variants(project)?;
    let report = LoadReport {
        unknown_variants: collect_unknown_variants(project),
    };
    for seq in project.sequences.values() {
        seq.validate()?;
    }
    Ok(report)
}

// ── Corrupt-known-variant guard (39 §2.2 rule 4) ─────────────────────────────

/// Serialized `source`-tags of the KNOWN [`ClipSource`] variants. Keep in sync
/// with the enum (clip.rs); the `Unknown` fallback is deliberately excluded.
pub const KNOWN_CLIP_SOURCE_TAGS: &[&str] = &[
    "asset",
    "vector",
    "nested_sequence",
    "solid_color",
    "adjustment",
    "text",
];

/// Serialized `kind`-tags of the KNOWN [`GradeOpParams`] variants (grade.rs).
pub const KNOWN_GRADE_PARAM_TAGS: &[&str] = &[
    "exposure",
    "contrast",
    "white_balance",
    "cdl",
    "wheels",
    "curves",
    "hsl_qualifier",
    "lut3d",
];

/// Serialized `op`-tags of the KNOWN [`GraphOp`] variants (graph.rs).
pub const KNOWN_GRAPH_OP_TAGS: &[&str] = &[
    "output",
    "clip_in",
    "media_in",
    "vector_in",
    "solid_color",
    "merge",
    "transform2_d",
    "crop",
    "resize",
    "blur",
    "sharpen",
    "glow",
    "chroma_key",
    "luma_key",
    "mask_shape",
    "mask_from_matte",
    "invert",
    "channel_split",
    "channel_combine",
    "grade",
    "lut",
    "text",
    "time_offset",
    "switch",
    "note",
];

/// The document-integrity half of 39 §2.2 rule 4. The three payload-carrying
/// enums (`ClipSource`, `GraphOp`, `GradeOpParams`) are internally tagged with a
/// per-variant `#[serde(untagged)] Unknown` fallback. That fallback is *greedy*:
/// a KNOWN tag whose payload is malformed deserializes to `Unknown` retaining
/// its original (known) tag rather than erroring. Loading such a value inert
/// would silently accept corrupt data, so any retained `Unknown` whose tag is a
/// known catalog tag is rejected. A genuine forward-compat variant always
/// carries a tag this build does not know, so it is never caught here (it is
/// preserved and diagnosed instead).
///
/// The four fieldless discriminant enums (`EffectKind`, `TransitionKind`,
/// `AudioFxKind`, `GradeOpKind`) serialize as bare strings and have no malformed
/// payload to swallow, so they need no such guard.
fn reject_corrupt_known_variants(project: &TimelineProject) -> Result<(), LoadError> {
    for seq in project.sequences.values() {
        for track in seq.video_tracks.iter().chain(seq.audio_tracks.iter()) {
            for clip in &track.clips {
                if let ClipSource::Unknown(map) = &clip.source {
                    let tag = map.get("source").and_then(|v| v.as_str()).unwrap_or_default();
                    if KNOWN_CLIP_SOURCE_TAGS.contains(&tag) {
                        return Err(LoadError::corrupt("ClipSource", tag));
                    }
                }
                if let Some(grade) = clip.grade.as_ref() {
                    for op in &grade.ops {
                        if let GradeOpParams::Unknown(map) = &op.params.base {
                            let tag = map.get("kind").and_then(|v| v.as_str()).unwrap_or_default();
                            if KNOWN_GRADE_PARAM_TAGS.contains(&tag) {
                                return Err(LoadError::corrupt("GradeOpParams", tag));
                            }
                        }
                    }
                }
            }
        }
    }
    for graph in project.graphs.values() {
        for node in graph.nodes.values() {
            if let GraphOp::Unknown(map) = &node.op {
                let tag = map.get("op").and_then(|v| v.as_str()).unwrap_or_default();
                if KNOWN_GRAPH_OP_TAGS.contains(&tag) {
                    return Err(LoadError::corrupt("GraphOp", tag));
                }
            }
        }
    }
    Ok(())
}

/// The orphan-flagging pass in isolation (see module docs). Walks every
/// animatable `AnimProps` lane the project owns and flags unresolved paths.
pub fn flag_orphaned_property_tracks(project: &mut TimelineProject) {
    for seq in project.sequences.values_mut() {
        for track in seq
            .video_tracks
            .iter_mut()
            .chain(seq.audio_tracks.iter_mut())
        {
            if let Some(audio) = track.audio.as_mut() {
                flag_tracks(&mut audio.params.tracks, PropTargetKind::TrackAudioParams);
                for unit in &mut audio.fx_chain {
                    flag_tracks(&mut unit.params.tracks, PropTargetKind::AudioFx(unit.kind));
                }
            }
            for clip in &mut track.clips {
                flag_tracks(&mut clip.transform.tracks, PropTargetKind::ClipTransform);
                for effect in &mut clip.effects {
                    // An inert effect (unknown manifest id) has no registry block
                    // to resolve against; flagging every track orphaned would
                    // perturb the byte-identical round trip §2.6 requires, so its
                    // tracks are left exactly as loaded.
                    if effect.inert {
                        continue;
                    }
                    flag_tracks(
                        &mut effect.params.tracks,
                        PropTargetKind::Effect(effect.kind),
                    );
                }
                if let Some(grade) = clip.grade.as_mut() {
                    for op in &mut grade.ops {
                        flag_tracks(&mut op.params.tracks, PropTargetKind::GradeOp(op.kind));
                    }
                }
                if let Some(audio) = clip.audio.as_mut() {
                    flag_tracks(&mut audio.params.tracks, PropTargetKind::ClipAudioParams);
                }
            }
        }
        // Master bus params + its fx chain.
        flag_tracks(
            &mut seq.audio_master.params.tracks,
            PropTargetKind::MasterBusParams,
        );
        for unit in &mut seq.audio_master.fx_chain {
            flag_tracks(&mut unit.params.tracks, PropTargetKind::AudioFx(unit.kind));
        }
    }
    // Graph-node params resolve leniently (`PropTargetKind::GraphNode` accepts
    // any path), so they are never orphaned — no pass needed for `project.graphs`.
}

// ── Effect id backfill / migration / inert preservation (spec 30 §2.6, §10) ──

/// Reconcile each clip effect's manifest identity, in place, before orphan
/// flagging runs (spec §10):
///
/// 1. **Backfill.** An absent `id` (empty sentinel — a v4 file) is filled from
///    the effect's legacy `kind`; conversely, an `id` that maps to a legacy kind
///    overwrites `kind`, keeping the two consistent.
/// 2. **Migrate.** For a known id whose stored `version` is older than the
///    manifest's, [`migrate`](super::effect_manifest::migrate) advances the
///    params to the current version. A pre-versioning sentinel (`version == 0`)
///    is assumed current and simply stamped, not migrated.
/// 3. **Inert preservation.** An id with no manifest (a future build's effect)
///    is marked `inert` and disabled, its `params` left untouched so
///    re-serialization is byte-identical (§2.6). Its `kind` is left as its serde
///    default (an `Unknown` tag), so the unknown-variant scan still reports it.
///
/// Load-time migration is deliberately *not* an undoable edit (§10): it runs
/// here, before the history is constructed, so no history code is involved.
fn finalize_effect_ids(project: &mut TimelineProject) {
    use super::effect_manifest::{manifest, migrate};
    for seq in project.sequences.values_mut() {
        for track in seq
            .video_tracks
            .iter_mut()
            .chain(seq.audio_tracks.iter_mut())
        {
            for clip in &mut track.clips {
                for effect in &mut clip.effects {
                    if effect.id.is_empty() {
                        effect.id = effect.kind.effect_id();
                    }
                    if let Some(k) = effect.id.legacy_kind() {
                        effect.kind = k;
                    }
                    match manifest(effect.id.clone()) {
                        Some(m) => {
                            if effect.version == 0 {
                                effect.version = m.version;
                            } else if effect.version < m.version {
                                if let Ok(v) = migrate(&effect.id, effect.version, &mut effect.params.base)
                                {
                                    effect.version = v;
                                }
                            }
                        }
                        None => {
                            effect.inert = true;
                            effect.enabled = false;
                        }
                    }
                }
            }
        }
    }
}

// ── Unknown-variant scan (39 §2.2 rule 3) ───────────────────────────────────

/// Where the first occurrence of an unknown variant was found. Enough to name
/// the offending clip/node in a diagnostic once 36 §3's `Subject` taxonomy
/// lands; until then this interim carrier is what [`LoadReport`] exposes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnknownSite {
    Clip {
        sequence: SequenceId,
        track: TrackId,
        clip: ClipId,
    },
    Effect {
        clip: ClipId,
        index: usize,
    },
    Transition {
        clip: ClipId,
    },
    GradeOp {
        clip: ClipId,
        op: GradeOpId,
    },
    GraphNode {
        graph: GraphId,
        node: GraphNodeId,
    },
    AudioFx {
        /// `None` for a master-bus fx unit; `Some` for a track fx unit.
        track: Option<TrackId>,
        index: usize,
    },
}

/// One deduped unknown-variant finding (39 §2.2 rule 3: once per document load,
/// never per frame). Dedup key is `(enum_name, tag)`; `count` accumulates every
/// occurrence and `first_site` records the first in traversal order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownVariant {
    /// The Rust enum whose unknown variant this is: `"EffectKind"`, `"GraphOp"`,
    /// `"TransitionKind"`, `"GradeOpKind"`, `"AudioFxKind"`, `"ClipSource"`.
    pub enum_name: &'static str,
    /// The verbatim serialized tag this build did not recognize.
    pub tag: String,
    /// Number of occurrences of this exact `(enum_name, tag)` in the document.
    pub count: usize,
    /// Where the first occurrence was found (traversal order).
    pub first_site: UnknownSite,
}

/// The side-band produced by [`finalize_load`]. Empty for a document with no
/// unknown variants.
///
/// This is the interim carrier for 39 §2.2's `Project::UnknownVariantPreserved`
/// warning; wiring it to the real diagnostic taxonomy is a 36 §3 follow-up.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoadReport {
    pub unknown_variants: Vec<UnknownVariant>,
}

/// Accumulates unknown-variant findings, deduping by `(enum_name, tag)` while
/// preserving first-seen order and per-key occurrence counts.
struct UnknownAccum {
    found: Vec<UnknownVariant>,
}

impl UnknownAccum {
    fn new() -> Self {
        UnknownAccum { found: Vec::new() }
    }

    /// Record one occurrence of `tag` for `enum_name` at `site`. The first
    /// occurrence stores `site`; later occurrences only bump `count`.
    fn record(&mut self, enum_name: &'static str, tag: &str, site: UnknownSite) {
        if let Some(v) = self
            .found
            .iter_mut()
            .find(|v| v.enum_name == enum_name && v.tag == tag)
        {
            v.count += 1;
        } else {
            self.found.push(UnknownVariant {
                enum_name,
                tag: tag.to_owned(),
                count: 1,
                first_site: site,
            });
        }
    }
}

/// Scan a finalized project for preserved unknown enum variants (39 §2.2). The
/// result is deduped per `(enum_name, tag)` so a document with the same unknown
/// effect on 50 clips yields ONE finding with `count == 50` — the "diagnose
/// once per load" rule, not once per occurrence.
///
/// Traversal mirrors [`flag_orphaned_property_tracks`] (sequences → video+audio
/// tracks → clips → source / effects / transitions / grade ops, plus track and
/// master-bus fx chains) but ALSO walks `project.graphs`, which the orphan pass
/// skips: `GraphOp::Unknown` is one of the payload-carrying unknown cases and
/// must be diagnosed. Sequences and graphs are visited in id-sorted order so a
/// document with unknowns in more than one sequence reports deterministically.
///
/// A grade op is represented once, under `"GradeOpKind"`: an unknown grade op
/// carries the same tag in both its `kind` and its `params.base`, so scanning
/// the `kind` covers it without double-counting.
pub fn collect_unknown_variants(project: &TimelineProject) -> Vec<UnknownVariant> {
    let mut acc = UnknownAccum::new();

    let mut seq_ids: Vec<SequenceId> = project.sequences.keys().copied().collect();
    seq_ids.sort();
    for seq_id in seq_ids {
        let seq = &project.sequences[&seq_id];
        for track in seq.video_tracks.iter().chain(seq.audio_tracks.iter()) {
            if let Some(audio) = track.audio.as_ref() {
                for (index, unit) in audio.fx_chain.iter().enumerate() {
                    if let Some(t) = unit.kind.unknown_tag() {
                        acc.record(
                            "AudioFxKind",
                            t.as_str(),
                            UnknownSite::AudioFx {
                                track: Some(track.id),
                                index,
                            },
                        );
                    }
                }
            }
            for clip in &track.clips {
                if let Some(tag) = clip.source.unknown_tag() {
                    acc.record(
                        "ClipSource",
                        tag,
                        UnknownSite::Clip {
                            sequence: seq_id,
                            track: track.id,
                            clip: clip.id,
                        },
                    );
                }
                for (index, effect) in clip.effects.iter().enumerate() {
                    if let Some(t) = effect.kind.unknown_tag() {
                        acc.record(
                            "EffectKind",
                            t.as_str(),
                            UnknownSite::Effect {
                                clip: clip.id,
                                index,
                            },
                        );
                    }
                }
                for transition in [clip.transition_in.as_ref(), clip.transition_out.as_ref()]
                    .into_iter()
                    .flatten()
                {
                    if let Some(t) = transition.kind.unknown_tag() {
                        acc.record(
                            "TransitionKind",
                            t.as_str(),
                            UnknownSite::Transition { clip: clip.id },
                        );
                    }
                }
                if let Some(grade) = clip.grade.as_ref() {
                    for op in &grade.ops {
                        if let Some(t) = op.kind.unknown_tag() {
                            acc.record(
                                "GradeOpKind",
                                t.as_str(),
                                UnknownSite::GradeOp {
                                    clip: clip.id,
                                    op: op.id,
                                },
                            );
                        }
                    }
                }
            }
        }
        for (index, unit) in seq.audio_master.fx_chain.iter().enumerate() {
            if let Some(t) = unit.kind.unknown_tag() {
                acc.record(
                    "AudioFxKind",
                    t.as_str(),
                    UnknownSite::AudioFx { track: None, index },
                );
            }
        }
    }

    let mut graph_ids: Vec<GraphId> = project.graphs.keys().copied().collect();
    graph_ids.sort();
    for graph_id in graph_ids {
        let graph = &project.graphs[&graph_id];
        let mut node_ids: Vec<GraphNodeId> = graph.nodes.keys().copied().collect();
        node_ids.sort();
        for node_id in node_ids {
            if let Some(tag) = graph.nodes[&node_id].op.unknown_tag() {
                acc.record(
                    "GraphOp",
                    tag,
                    UnknownSite::GraphNode {
                        graph: graph_id,
                        node: node_id,
                    },
                );
            }
        }
    }

    acc.found
}

#[cfg(test)]
mod unknown_scan_tests {
    use super::*;
    use crate::timeline::{
        Clip, ClipEffect, ClipSource, EffectKind, FrameRate, GraphNode, GraphOp, NodeGraph,
        Sequence, Tick, TimelineProject, Track, TrackKind, UnknownTag,
    };

    /// Three clips sharing ONE unknown effect tag plus one unknown graph op must
    /// yield exactly two findings — counts 3 and 1 — proving "diagnose once per
    /// load" (39 §2.2 rule 3), not once per occurrence, and that `first_site`
    /// points at the first clip in traversal order.
    #[test]
    fn collect_dedups_by_tag_and_accumulates_counts() {
        let mut project = TimelineProject::new();
        let mut seq = Sequence::new("Seq 1", FrameRate::FPS_30, 1920, 1080);
        let mut track = Track::new(TrackKind::Video, "V1");

        let film = EffectKind::Unknown(UnknownTag::intern("film_look"));
        let mut first_clip_id = None;
        for i in 0..3 {
            let mut clip = Clip::new(ClipSource::Adjustment, Tick(i * 1000), Tick(500));
            clip.effects.push(ClipEffect::new(film));
            if first_clip_id.is_none() {
                first_clip_id = Some(clip.id);
            }
            track.clips.push(clip);
        }
        seq.video_tracks.push(track);
        project.insert_sequence(seq);

        let mut graph = NodeGraph::new_project_graph("G1");
        let mut op_obj = serde_json::Map::new();
        op_obj.insert("op".to_string(), serde_json::Value::String("caustics".to_string()));
        let node = GraphNode::new(GraphOp::Unknown(op_obj));
        let node_id = node.id;
        graph.nodes.insert(node.id, node);
        project.graphs.insert(graph.id, graph);

        let found = collect_unknown_variants(&project);
        assert_eq!(found.len(), 2, "one finding per (enum, tag), not per occurrence");

        let effect = found
            .iter()
            .find(|u| u.enum_name == "EffectKind")
            .expect("the shared unknown effect must be reported");
        assert_eq!(effect.tag, "film_look");
        assert_eq!(effect.count, 3, "three clips share one unknown effect → count 3");
        assert_eq!(
            effect.first_site,
            UnknownSite::Effect {
                clip: first_clip_id.unwrap(),
                index: 0,
            },
            "first_site names the first clip in traversal order"
        );

        let graph_op = found
            .iter()
            .find(|u| u.enum_name == "GraphOp")
            .expect("the unknown graph op must be reported");
        assert_eq!(graph_op.tag, "caustics");
        assert_eq!(graph_op.count, 1);
        assert!(matches!(
            graph_op.first_site,
            UnknownSite::GraphNode { node, .. } if node == node_id
        ));
    }

    /// A single-clip video sequence whose only clip carries `source`.
    fn project_with_source(source: ClipSource) -> TimelineProject {
        let mut project = TimelineProject::new();
        let mut seq = Sequence::new("Seq 1", FrameRate::FPS_30, 1920, 1080);
        let mut track = Track::new(TrackKind::Video, "V1");
        track
            .clips
            .push(Clip::new(source, Tick(0), Tick(500)));
        seq.video_tracks.push(track);
        project.insert_sequence(seq);
        project
    }

    /// 39 §2.2 rule 4: a KNOWN `source` tag whose payload is malformed is
    /// swallowed into `ClipSource::Unknown` by serde's greedy untagged fallback
    /// (retaining the known tag) — `finalize_load` must REJECT it, not load it
    /// inert, so corrupt data never masquerades as a forward-compat variant.
    #[test]
    fn finalize_load_rejects_a_malformed_known_clip_source() {
        let bad: ClipSource =
            serde_json::from_value(serde_json::json!({"source":"solid_color","color":"NOPE"}))
                .expect("greedy untagged fallback swallows the malformed known variant");
        assert!(bad.is_unknown() && bad.unknown_tag() == Some("solid_color"));

        let mut project = project_with_source(bad);
        let err = finalize_load(&mut project)
            .expect_err("a corrupt known clip source must be rejected at load");
        assert_eq!(
            err,
            LoadError::CorruptKnownVariant {
                enum_name: "ClipSource",
                tag: "solid_color".to_string(),
            }
        );
        assert!(err.to_string().contains("solid_color"));
    }

    /// The counterpart: a GENUINE unknown tag (one this build does not know) is
    /// preserved and diagnosed, never rejected.
    #[test]
    fn finalize_load_accepts_a_genuine_unknown_clip_source() {
        let src: ClipSource =
            serde_json::from_value(serde_json::json!({"source":"holo_gen","seed":7}))
                .unwrap();
        let mut project = project_with_source(src);
        let report = finalize_load(&mut project).expect("a genuine unknown source loads");
        assert_eq!(report.unknown_variants.len(), 1);
        assert_eq!(report.unknown_variants[0].enum_name, "ClipSource");
        assert_eq!(report.unknown_variants[0].tag, "holo_gen");
    }

    /// The same greedy-swallow hazard on `GradeOpParams` (an internally-tagged
    /// enum reached through `clip.grade`): a malformed KNOWN grade kind is
    /// rejected.
    #[test]
    fn finalize_load_rejects_a_malformed_known_grade_param() {
        use crate::timeline::{Grade, GradeOp, GradeOpKind, GradeOpParams};
        let bad: GradeOpParams =
            serde_json::from_value(serde_json::json!({"kind":"exposure","stops":"NOPE"}))
                .expect("malformed known grade param degrades to Unknown");
        assert!(bad.is_unknown() && bad.unknown_tag() == Some("exposure"));

        let mut project = project_with_source(ClipSource::Adjustment);
        let clip = &mut project
            .sequences
            .values_mut()
            .next()
            .unwrap()
            .video_tracks[0]
            .clips[0];
        let mut op = GradeOp::new(GradeOpKind::Exposure, GradeOpParams::Exposure { stops: 0.0 });
        op.params.base = bad;
        clip.grade = Some(Grade {
            ops: vec![op],
            bypass: false,
        });

        let err = finalize_load(&mut project).expect_err("corrupt known grade param must reject");
        assert_eq!(
            err,
            LoadError::CorruptKnownVariant {
                enum_name: "GradeOpParams",
                tag: "exposure".to_string(),
            }
        );
    }

    /// The known-tag constants must stay in lockstep with the enums, otherwise
    /// the guard silently stops catching a corrupted variant. Rather than trust a
    /// hand-typed snake_case list, this serializes REAL known variants and
    /// asserts serde's emitted tag is present in the constant — catching the
    /// exact renaming mistakes (e.g. `transform2_d`) the list is prone to.
    #[test]
    fn known_tag_constants_track_serde_output() {
        use crate::timeline::GradeOpParams;
        use crate::Color;

        // Every fieldless GraphOp variant serialized here must appear in the
        // constant; the field-carrying variants are covered by construction.
        let graph_ops = [
            GraphOp::Output,
            GraphOp::ClipIn,
            GraphOp::SolidColor,
            GraphOp::Transform2D,
            GraphOp::Crop,
            GraphOp::Blur,
            GraphOp::Sharpen,
            GraphOp::Glow,
            GraphOp::ChromaKey,
            GraphOp::LumaKey,
            GraphOp::MaskFromMatte,
            GraphOp::Invert,
            GraphOp::ChannelSplit,
            GraphOp::ChannelCombine,
            GraphOp::Switch,
        ];
        for op in graph_ops {
            let tag = serde_json::to_value(&op).unwrap()["op"].as_str().unwrap().to_string();
            assert!(
                KNOWN_GRAPH_OP_TAGS.contains(&tag.as_str()),
                "GraphOp tag {tag:?} missing from KNOWN_GRAPH_OP_TAGS"
            );
        }

        // ClipSource: a couple of representative variants (one fieldless, one
        // with a payload) to pin the snake_case rename against serde.
        for src in [
            ClipSource::Adjustment,
            ClipSource::SolidColor {
                color: Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 },
            },
        ] {
            let tag = serde_json::to_value(&src).unwrap()["source"].as_str().unwrap().to_string();
            assert!(KNOWN_CLIP_SOURCE_TAGS.contains(&tag.as_str()), "missing {tag:?}");
        }

        // GradeOpParams likewise.
        let tag = serde_json::to_value(GradeOpParams::Exposure { stops: 0.0 }).unwrap()["kind"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(KNOWN_GRADE_PARAM_TAGS.contains(&tag.as_str()));

        // Genuine unknown tags are deliberately absent from every constant, so a
        // real forward-compat variant is preserved rather than rejected.
        assert!(!KNOWN_CLIP_SOURCE_TAGS.contains(&"holo_gen"));
        assert!(!KNOWN_GRAPH_OP_TAGS.contains(&"caustics"));
        assert!(!KNOWN_GRADE_PARAM_TAGS.contains(&"bloom"));
    }
}
