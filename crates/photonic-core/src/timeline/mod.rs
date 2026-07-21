//! Timeline data model (video editor) — pure data + pure functions; no I/O, no
//! GPU, no threads (01-data-model.md §3).
//!
//! This module family is the normative home for the video-editor timeline: time
//! (`Tick`/`FrameRate`), media pool, sequences/tracks/clips, the generic
//! keyframe-animation system, per-clip and project node graphs, color grades,
//! captions, and audio — plus the pure edit ops (`ops`, `graph_ops`) and undo
//! commands (`commands`) that mutate them. The engine (`photonic-video`),
//! renderer, GUI, and MCP layers all build on these types; several core types
//! (`Tick`, `FrameRate`, `AssetId`, `VectorRef`, `VectorStateKey`, `EffectKind`)
//! were relocated here from `photonic-video::contract`, which now re-exports
//! them.

pub mod anim;
pub mod audio;
pub mod captions;
pub mod clip;
pub mod commands;
pub mod effect_kind;
pub mod effect_manifest;
pub mod grade;
pub mod graph;
pub mod graph_ops;
pub mod ids;
pub mod load;
pub mod media;
pub mod ops;
pub mod prop_registry;
pub mod scale;
pub mod sequence;
pub mod time;
pub mod unknown;

// ── Curated re-exports (the surface most callers use) ───────────────────────

pub use anim::{
    cubic_bezier_ease, eval, AnimProps, Interp, Keyframe, PropPath, PropSet, PropValue,
    PropValueKind, PropertyTrack,
};
pub use audio::{
    AudioFade, AudioFxKind, AudioFxUnit, ChannelMap, ClipAudio, ClipAudioParams, FadeShape,
    LoudnessTarget, MasterBus, MasterBusParams, TrackAudio, TrackAudioParams,
};
pub use captions::{
    CaptionAnim, CaptionBackground, CaptionCue, CaptionStyle, CaptionTrack, CaptionWord,
    KaraokeMode, KaraokeStyle,
};
pub use clip::{
    AnchorSpace, Clip, ClipEffect, ClipSource, ClipTransform, EaseCurve, Ratio, SpeedMap,
    TextClipContent, Transition, TransitionKind, TransitionParams, WipeDirection,
};
pub use commands::{
    AnimTarget, AudioCmd, CaptionCmd, ClipTiming, FadeEdge, FormatOp, FxOwner, GraphCmd,
    StyleTarget, TimelineCmd, TrackSettings, TtsCmd,
};
pub use effect_kind::{EffectKind, EffectParams};
// NB: `effect_manifest::Display` is intentionally NOT re-exported at the
// timeline root — the name would collide with `std::fmt::Display` for callers
// that glob `use photonic_core::timeline::*`. Reach it via `effect_manifest::Display`.
pub use effect_manifest::{
    manifest, manifests, AlphaBehaviour, Applicability, BitDepth, Caps, EffectCategory, EffectId,
    EffectManifest, EffectMigration, GpuSupport, MigrationError, OperandSpace, ParamKind,
    ParamSpec, UiHint, MANIFESTS, MIGRATIONS,
};
pub use grade::{
    parse_cdl_xml, write_cdl_xml, CdlParams, CdlXmlError, Grade, GradeMask, GradeOp, GradeOpKind,
    GradeOpParams, LutInterp, MaskRef, WindowShape,
};
pub use graph::{
    FitMode, GraphEdge, GraphNode, GraphNodeParams, GraphOp, InPort, MaskShapeKind, NodeGraph,
    NodePos, OutPort, TextGen, TimeSource,
};
pub use ids::{
    AssetId, BinId, ClipId, CueId, GradeOpId, GraphId, GraphNodeId, GroupId, MarkerCategoryId,
    MarkerId, SequenceId, TrackId,
};
pub use media::{
    AssetKind, AssetSource, AudioStreamInfo, MediaAsset, MediaBin, MediaPool, MediaProbe,
    ProbedColor, ProxyOrigin, ProxyRef, ProxyStatus, VectorRef, VectorStateKey, VideoStreamInfo,
};
pub use ops::{ClipEdge, EditError};
pub use prop_registry::{PropEntry, PropTargetKind};
pub use scale::{
    TARGET_ASSETS_PER_PROJECT, TARGET_CLIPS_PER_SEQUENCE, TARGET_CUES_PER_CAPTION_TRACK,
    TARGET_GRAPH_NODES_PER_COMPOSITION, TARGET_KEYFRAMES_PER_PROPERTY_TRACK,
    TARGET_SEQUENCES_PER_PROJECT, TARGET_TRACKS_PER_SEQUENCE,
};
pub use sequence::{
    GroupKind, GroupNode, Marker, MarkerAnchor, MarkerCategory, MarkerGlyph, ProjectVideoSettings,
    Sequence, SequenceFormat, TimelineProject, Track, TrackKind, ValidationError,
};
pub use time::{FrameRate, Tick, TICKS_PER_SECOND};
pub use unknown::UnknownTag;
