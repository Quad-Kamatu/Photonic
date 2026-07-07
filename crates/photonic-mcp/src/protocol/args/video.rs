//! Args for the video-domain MCP tool surface (10-mcp-tools.md §3, P2 slice:
//! timeline-EDIT tools only — sequence/track/clip/effects/keyframes/media).
//!
//! Engine-backed tools (probe/proxy/playback/render/export/captions/tts/grade
//! scopes) are P3+ and have no args here.
//!
//! Time-valued fields follow design rule 3 (10 §1.3): every timeline position
//! exposes `*_ticks`/`*_tc`/`*_seconds`, precedence ticks > tc > seconds,
//! resolved by the shared `resolve_tick` helper in `handlers/video.rs`.
//!
//! Clip/track-scoped tools address their target by id alone (`clip_id`,
//! `track_id`) — matching §3's tool tables and the rest of the MCP surface's
//! id-only addressing (`update_node(node_id)` needs no `layer_id`). The
//! handler resolves the owning sequence/track by scanning the project
//! (`handlers/video.rs::locate_clip`/`locate_track`) before calling the
//! `timeline/ops.rs` fn, which does take explicit ids.

use photonic_core::timeline::{
    AssetId, BinId, ClipId, ClipTransform, FrameRate, Interp, MarkerId, PropValue, SequenceFormat,
    SequenceId, TrackId, TrackKind, TransitionKind, TransitionParams,
};
use serde::Deserialize;

// ─── Shared value types ─────────────────────────────────────────────────────

/// A clip source (01 §5 `ClipSource`), as supplied over the wire.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClipSourceArg {
    Asset {
        asset_id: AssetId,
    },
    Vector {
        asset_id: AssetId,
    },
    NestedSequence {
        sequence_id: SequenceId,
    },
    /// `#rrggbb` or `#rrggbbaa`.
    SolidColor {
        color: String,
    },
    Adjustment,
}

/// A generic keyframe/animation target (01 §6). P2 scope: clip transform and
/// clip-effect params only — grade/audio/graph-node targets land with their
/// respective domains (P3+).
#[derive(Debug, Deserialize)]
#[serde(tag = "target", rename_all = "snake_case")]
pub enum AnimTargetArg {
    ClipTransform {
        clip_id: ClipId,
    },
    ClipEffect {
        clip_id: ClipId,
        effect_index: usize,
    },
}

impl AnimTargetArg {
    pub fn clip_id(&self) -> ClipId {
        match self {
            AnimTargetArg::ClipTransform { clip_id }
            | AnimTargetArg::ClipEffect { clip_id, .. } => *clip_id,
        }
    }
}

/// Which mode `set_sequence_format` operates in.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormatOpKind {
    Add,
    Update,
    Remove,
}

/// Which edge a trim op targets.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipEdge {
    In,
    Out,
}

/// `{num, den}` — exact rational speed (01 §5.1).
#[derive(Debug, Deserialize)]
pub struct RatioArg {
    pub num: i32,
    pub den: u32,
}

// ─── Sequence (10 §3.2) ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateSequenceArgs {
    pub name: String,
    /// `{"num": 30, "den": 1}` — see [`FrameRate`].
    pub frame_rate: FrameRate,
    /// At least one required (CAP-012).
    pub formats: Vec<SequenceFormat>,
}

#[derive(Debug, Deserialize)]
pub struct DeleteSequenceArgs {
    pub sequence_id: SequenceId,
}

#[derive(Debug, Deserialize, Default)]
pub struct ListSequencesArgs {}

#[derive(Debug, Deserialize)]
pub struct SetActiveSequenceArgs {
    /// `null`/omitted clears the active sequence.
    #[serde(default)]
    pub sequence_id: Option<SequenceId>,
}

#[derive(Debug, Deserialize)]
pub struct SetSequenceFormatArgs {
    pub sequence_id: SequenceId,
    pub op: FormatOpKind,
    /// Required for `add`/`update`.
    #[serde(default)]
    pub format: Option<SequenceFormat>,
    /// Required for `update`/`remove`.
    #[serde(default)]
    pub format_index: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct SetActiveFormatArgs {
    pub sequence_id: SequenceId,
    pub format_index: usize,
}

/// `null`/omitted `range` clears the sequence's work range.
#[derive(Debug, Deserialize)]
pub struct SetWorkRangeArgs {
    pub sequence_id: SequenceId,
    #[serde(default)]
    pub range: Option<WorkRangeArg>,
}

#[derive(Debug, Deserialize)]
pub struct WorkRangeArg {
    #[serde(default)]
    pub start_ticks: Option<i64>,
    #[serde(default)]
    pub start_tc: Option<String>,
    #[serde(default)]
    pub start_seconds: Option<f64>,
    #[serde(default)]
    pub end_ticks: Option<i64>,
    #[serde(default)]
    pub end_tc: Option<String>,
    #[serde(default)]
    pub end_seconds: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct AddMarkerArgs {
    pub sequence_id: SequenceId,
    #[serde(default)]
    pub at_ticks: Option<i64>,
    #[serde(default)]
    pub at_tc: Option<String>,
    #[serde(default)]
    pub at_seconds: Option<f64>,
    #[serde(default)]
    pub name: Option<String>,
    /// `#rrggbb` or `#rrggbbaa`.
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RemoveMarkerArgs {
    pub marker_id: MarkerId,
}

#[derive(Debug, Deserialize)]
pub struct ListMarkersArgs {
    pub sequence_id: SequenceId,
}

// ─── Track (10 §3.3) ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AddTrackArgs {
    pub sequence_id: SequenceId,
    pub kind: TrackKind,
    #[serde(default)]
    pub name: Option<String>,
    /// Insertion index within the track's lane; defaults to the end.
    #[serde(default)]
    pub index: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct RemoveTrackArgs {
    pub track_id: TrackId,
}

/// Universal track setter (mirrors `TrackSettings`); every field optional,
/// only supplied fields change.
#[derive(Debug, Deserialize)]
pub struct SetTrackPropArgs {
    pub track_id: TrackId,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub locked: Option<bool>,
    #[serde(default)]
    pub height_px: Option<f32>,
}

/// Args shape matches `ops::reorder_track` (single track → target index
/// within its lane), not the spec table's illustrative full
/// `old_order`/`new_order` permutation — `ops.rs` has no permutation-setter
/// fn; this is functionally equivalent (design rule 1: use the existing op
/// as-is).
#[derive(Debug, Deserialize)]
pub struct ReorderTrackArgs {
    pub track_id: TrackId,
    pub new_index: usize,
}

// ─── Clip edit ops (10 §3.4) ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct InsertClipArgs {
    pub track_id: TrackId,
    #[serde(default)]
    pub name: Option<String>,
    /// Position in the sequence. Precedence: at_ticks > at_tc > at_seconds (§1 rule 3).
    #[serde(default)]
    pub start_ticks: Option<i64>,
    #[serde(default)]
    pub start_tc: Option<String>,
    #[serde(default)]
    pub start_seconds: Option<f64>,
    pub source: ClipSourceArg,
    #[serde(default)]
    pub source_in_ticks: Option<i64>,
    #[serde(default)]
    pub source_in_tc: Option<String>,
    #[serde(default)]
    pub source_in_seconds: Option<f64>,
    /// Duration always exact ticks — no dual-unit ambiguity for a length
    /// agents typically compute from probe data (§5).
    pub duration_ticks: i64,
}

#[derive(Debug, Deserialize)]
pub struct MoveClipArgs {
    pub clip_id: ClipId,
    #[serde(default)]
    pub new_start_ticks: Option<i64>,
    #[serde(default)]
    pub new_start_tc: Option<String>,
    #[serde(default)]
    pub new_start_seconds: Option<f64>,
    /// Cross-track move (destination must be the same `TrackKind`, routes
    /// through `ops::move_clip_to_track`). Omit for a same-track move.
    #[serde(default)]
    pub new_track_id: Option<TrackId>,
}

/// Args for `ripple_edit` — trims `edge` to `current_edge_position +
/// delta_ticks` and ripples every later clip on the track to close/open the
/// gap (`ops::ripple_trim`). `edge` uses the same in/out vocabulary as
/// `trim_clip` (in = clip's in-point/`ClipEdge::Start`, out = clip's
/// out-point/`ClipEdge::End`).
#[derive(Debug, Deserialize)]
pub struct RippleEditArgs {
    pub clip_id: ClipId,
    pub edge: ClipEdge,
    pub delta_ticks: i64,
}

#[derive(Debug, Deserialize)]
pub struct TrimClipArgs {
    pub clip_id: ClipId,
    pub edge: ClipEdge,
    #[serde(default)]
    pub new_ticks: Option<i64>,
    #[serde(default)]
    pub new_tc: Option<String>,
    #[serde(default)]
    pub new_seconds: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct SplitClipArgs {
    pub clip_id: ClipId,
    #[serde(default)]
    pub at_ticks: Option<i64>,
    #[serde(default)]
    pub at_tc: Option<String>,
    #[serde(default)]
    pub at_seconds: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct RemoveClipArgs {
    pub clip_id: ClipId,
    /// Shift every later clip on the track left by the removed clip's
    /// duration (`ops::ripple_delete`).
    #[serde(default)]
    pub ripple: bool,
}

#[derive(Debug, Deserialize)]
pub struct RollEditArgs {
    pub clip_id_a: ClipId,
    pub clip_id_b: ClipId,
    /// Delta in ticks applied to the shared edge (positive = later).
    pub delta_ticks: i64,
}

#[derive(Debug, Deserialize)]
pub struct SlipClipArgs {
    pub clip_id: ClipId,
    /// Delta in ticks applied to `source_in` (position on the timeline is
    /// unchanged).
    pub delta_ticks: i64,
}

#[derive(Debug, Deserialize)]
pub struct SlideClipArgs {
    pub clip_id: ClipId,
    pub delta_ticks: i64,
}

// ─── Clip properties (10 §3.5) ──────────────────────────────────────────────

/// Universal clip setter — every field optional, only supplied fields
/// change (mirrors `update_node`'s shape, utility.rs:12). `speed` and
/// `transition_*` have dedicated tools (`set_clip_speed`/`set_transition`) —
/// not repeated here.
#[derive(Debug, Deserialize)]
pub struct SetClipPropArgs {
    pub clip_id: ClipId,
    #[serde(default)]
    pub name: Option<String>,
    /// Full base transform replace (pos/scale/rotation/anchor/opacity).
    #[serde(default)]
    pub transform: Option<ClipTransform>,
    /// Per-`SequenceFormat` static override.
    #[serde(default)]
    pub reframe: Option<ReframeArg>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ReframeArg {
    pub format_index: usize,
    /// `null` clears the override for this format index.
    pub transform: Option<ClipTransform>,
}

/// `SpeedMap` is `Constant`-only in v1 (01 §5.1); there is no ramp variant to
/// request, so this tool never actually rejects with `NotSupportedV1` today —
/// the field is kept for forward compatibility.
#[derive(Debug, Deserialize)]
pub struct SetClipSpeedArgs {
    pub clip_id: ClipId,
    pub ratio: RatioArg,
}

#[derive(Debug, Deserialize)]
pub struct SetTransitionArgs {
    pub clip_id: ClipId,
    pub edge: ClipEdge,
    /// `null` removes the transition on that edge.
    #[serde(default)]
    pub transition: Option<TransitionArg>,
}

#[derive(Debug, Deserialize)]
pub struct TransitionArg {
    pub kind: TransitionKind,
    pub duration_ticks: i64,
    #[serde(default)]
    pub params: TransitionParams,
}

#[derive(Debug, Deserialize, Default)]
pub struct ListClipsArgs {
    #[serde(default)]
    pub sequence_id: Option<SequenceId>,
    #[serde(default)]
    pub track_id: Option<TrackId>,
    /// Optional `[start_ticks, end_ticks)` filter.
    #[serde(default)]
    pub range_start_ticks: Option<i64>,
    #[serde(default)]
    pub range_end_ticks: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct GetClipArgs {
    pub clip_id: ClipId,
}

// ─── Effects (10 §3.6) ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AddEffectArgs {
    pub clip_id: ClipId,
    pub kind: photonic_core::timeline::EffectKind,
    #[serde(default)]
    pub index: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct RemoveEffectArgs {
    pub clip_id: ClipId,
    pub effect_index: usize,
}

#[derive(Debug, Deserialize)]
pub struct ReorderEffectsArgs {
    pub clip_id: ClipId,
    pub new_order: Vec<usize>,
}

/// Sets one `PropPath` under `effects[i].params` (static base value, not a
/// keyframe — use `set_keyframe` for animated params); `path == "enabled"`
/// toggles the effect itself instead of a param.
#[derive(Debug, Deserialize)]
pub struct SetEffectParamArgs {
    pub clip_id: ClipId,
    pub effect_index: usize,
    pub path: String,
    pub value: PropValue,
}

#[derive(Debug, Deserialize, Default)]
pub struct ListEffectKindsArgs {}

// ─── Keyframes (10 §3.7) ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SetKeyframeArgs {
    #[serde(flatten)]
    pub target: AnimTargetArg,
    pub path: String,
    #[serde(default)]
    pub at_ticks: Option<i64>,
    #[serde(default)]
    pub at_tc: Option<String>,
    #[serde(default)]
    pub at_seconds: Option<f64>,
    pub value: PropValue,
    pub interp: Interp,
}

#[derive(Debug, Deserialize)]
pub struct RemoveKeyframeArgs {
    #[serde(flatten)]
    pub target: AnimTargetArg,
    pub path: String,
    #[serde(default)]
    pub at_ticks: Option<i64>,
    #[serde(default)]
    pub at_tc: Option<String>,
    #[serde(default)]
    pub at_seconds: Option<f64>,
}

/// One entry of `batch_set_keyframes`.
#[derive(Debug, Deserialize)]
pub struct KeyframeOpArg {
    #[serde(flatten)]
    pub target: AnimTargetArg,
    pub path: String,
    #[serde(default)]
    pub at_ticks: Option<i64>,
    #[serde(default)]
    pub at_tc: Option<String>,
    #[serde(default)]
    pub at_seconds: Option<f64>,
    pub value: PropValue,
    pub interp: Interp,
}

/// N keyframes as one undo step (design rule 4).
#[derive(Debug, Deserialize)]
pub struct BatchSetKeyframesArgs {
    pub ops: Vec<KeyframeOpArg>,
}

#[derive(Debug, Deserialize)]
pub struct GetKeyframesArgs {
    #[serde(flatten)]
    pub target: AnimTargetArg,
}

// ─── Media (P2 subset: import/relink/list/remove) ───────────────────────────

/// Registers `MediaAsset`s with `probe: None` (ffprobe integration is P3, 02
/// §6) — the result data flags each asset `"probed": false`. Content-hashes
/// the file now (head+tail+len) so `relink_media`'s future by-hash matching
/// has an identity to match against. `bin` names a bin to file the imported
/// asset(s) under — looked up by exact name, created (top-level, no parent)
/// if it doesn't exist yet.
#[derive(Debug, Deserialize)]
pub struct ImportMediaArgs {
    pub paths: Vec<String>,
    #[serde(default)]
    pub bin: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RelinkMediaArgs {
    pub asset_id: AssetId,
    pub new_path: String,
}

/// `bin` filters to assets filed under the bin with that exact name.
#[derive(Debug, Deserialize, Default)]
pub struct ListMediaArgs {
    #[serde(default)]
    pub bin: Option<String>,
}

/// Not in the 10-mcp-tools.md §3.1 catalog table — added to the P2 scope
/// explicitly by the work order (`ops::remove_asset` already exists).
#[derive(Debug, Deserialize)]
pub struct RemoveAssetArgs {
    pub asset_id: AssetId,
}

// ─── Media bins (not in the original §3.1 catalog table — added because the
// media gap-fix (photonic-core commit ab7557f) landed real `MediaAsset.bin`
// support; folded in as standard `create_/remove_/set_/list_` tools rather
// than an op-field mega-tool, since each maps 1:1 to a distinct
// `TimelineCmd` variant, matching design rule 1/2) ─────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateBinArgs {
    pub name: String,
    #[serde(default)]
    pub parent: Option<BinId>,
}

#[derive(Debug, Deserialize)]
pub struct RemoveBinArgs {
    pub bin_id: BinId,
}

/// `null`/omitted `bin_id` moves the asset to the pool root (unfiled).
#[derive(Debug, Deserialize)]
pub struct SetAssetBinArgs {
    pub asset_id: AssetId,
    #[serde(default)]
    pub bin_id: Option<BinId>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ListBinsArgs {}
