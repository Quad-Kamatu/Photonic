# 01 — Timeline Data Model

**Normative for:** all docs. **Location:** new module `crates/photonic-core/src/timeline/` (pure data + pure functions; no I/O, no GPU, no threads). **Decisions:** D-01, D-06, D-08.

All types are `Clone + Debug + Serialize + Deserialize + PartialEq` unless noted. All IDs are `Uuid` newtypes, matching the existing `pub type NodeId = Uuid` convention (`node.rs:12`) but as distinct newtypes for type safety:

```rust
macro_rules! id_newtype { ... } // ClipId, TrackId, SequenceId, AssetId, GraphId, GraphNodeId, MarkerId, CueId
pub struct ClipId(pub Uuid); // etc.
```

---

## 1. Time representation

**Ticks (flicks).** All timeline positions and durations are `i64` ticks at **705,600,000 ticks/second** (the "flick": smallest unit that exactly divides 24, 25, 30, 48, 50, 60, 90, 100, 120 fps — including 1001-denominator NTSC rates — and 44.1/48/88.2/96/192 kHz audio rates).

```rust
pub const TICKS_PER_SECOND: i64 = 705_600_000;

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Tick(pub i64);            // position or duration; negative allowed for deltas only

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct FrameRate { pub num: u32, pub den: u32 }   // e.g. 30000/1001

impl FrameRate {
    pub fn ticks_per_frame(&self) -> Tick;   // exact: TICKS_PER_SECOND * den / num (always integral for supported rates)
    pub fn frame_at(&self, t: Tick) -> i64;  // floor
    pub fn snap(&self, t: Tick) -> Tick;     // to frame boundary
}
```

Rules (normative):
- Persistence, edit ops, and the engine speak ticks only. Frames are a display/snapping concept derived from the sequence's `FrameRate`.
- No `f32`/`f64` time anywhere in the data model. UI converts at the edge.
- Unsupported/exotic rates: `ticks_per_frame` rounds to nearest tick; a `FrameRate::is_exact()` flag lets UI warn. Sub-tick error over 10 min is < 1 µs — below SS-3 tolerance.

## 2. Top-level container

```rust
// document.rs — additive field, sibling of `constraints`, `symbols`, etc.
pub struct Document {
    ...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeline: Option<TimelineProject>,
}

pub struct TimelineProject {
    pub media: MediaPool,                       // assets (§3)
    pub sequences: HashMap<SequenceId, Sequence>,
    pub sequence_order: Vec<SequenceId>,        // UI ordering
    pub active_sequence: Option<SequenceId>,
    pub graphs: HashMap<GraphId, NodeGraph>,    // all user node graphs (per-clip + project), one arena (§8)
    pub project_graph: Option<GraphId>,         // spliced after active sequence output
    pub settings: ProjectVideoSettings,         // proxy prefs, cache limits, default rates
}
```

`Document::new()` leaves `timeline: None`; first video-mode action creates it (undoably, §10).

## 3. Media pool

Media is **referenced, never embedded** (SPEC constraint). Contrast with `RasterImage`'s base64-PNG embedding — video files are orders of magnitude too large.

```rust
pub struct MediaPool {
    pub assets: HashMap<AssetId, MediaAsset>,
    pub bins: Vec<MediaBin>,                    // folders; flat list with parent refs
}

pub struct MediaAsset {
    pub id: AssetId,
    pub kind: AssetKind,
    pub source: AssetSource,
    pub probe: Option<MediaProbe>,              // filled by engine after ffprobe; cached in file
    pub proxy: Option<ProxyRef>,                // engine-managed; path + status
    pub content_hash: Option<String>,           // xxh3 of file head+tail+len; relink identity
}

pub enum AssetKind { Video, Audio, Image, VectorDoc, Lut3d }
// VectorDoc: this document or external .photon/.svg
// Lut3d: .cube file, referenced not embedded (07 §1) — gets the same offline/relink handling as media

pub enum AssetSource {
    File { path: PathBuf },                     // absolute; relative-to-project fallback on load (§9)
    EmbeddedVector { root: VectorRef },         // lives inside this Document (artboard or node subtree)
}

pub enum VectorRef { Artboard(usize), Node(NodeId), WholeDocument }

pub struct MediaProbe {                          // subset of ffprobe output we persist
    pub duration: Tick,
    pub video: Option<VideoStreamInfo>,          // w, h, frame_rate: FrameRate, pixel_aspect, color: ProbedColor, keyframe_index_cached: bool
    pub audio: Option<AudioStreamInfo>,          // sample_rate, channels, codec
    pub container: String, pub codec: String,
}
```

Offline/missing media is a first-class state: `MediaAsset` with no reachable file renders diagonal-stripe placeholder; relink matches `content_hash` then filename.

## 4. Sequence, tracks

```rust
pub struct Sequence {
    pub id: SequenceId,
    pub name: String,
    pub frame_rate: FrameRate,
    pub formats: Vec<SequenceFormat>,           // multi-aspect (CAP-012): ≥1 entries
    pub active_format: usize,
    pub video_tracks: Vec<Track>,               // index 0 = bottom of composite stack
    pub audio_tracks: Vec<Track>,
    pub caption_tracks: Vec<CaptionTrack>,      // §7
    pub markers: Vec<Marker>,                   // {id, at: Tick, name, color, note}
    pub audio_master: MasterBus,                // 09-audio-mixer.md
    pub work_range: Option<(Tick, Tick)>,       // in/out for preview + export
}

pub struct SequenceFormat {                      // one aspect-ratio variant
    pub name: String,                            // "16:9", "9:16", ...
    pub width: u32, pub height: u32,
    // per-clip reframe overrides live on the clip (§5, `reframe`), keyed by format index
}

pub struct Track {
    pub id: TrackId,
    pub name: String,
    pub kind: TrackKind,                         // Video | Audio
    pub clips: Vec<Clip>,                        // sorted by start, non-overlapping (invariant, enforced by edit ops)
    pub enabled: bool,                           // video: hidden; audio: muted
    pub locked: bool,
    pub audio: Option<TrackAudio>,               // volume/pan/solo, fx chain, automation (09)
    pub height_px: f32,                          // UI-only but persisted (like existing panel prefs)
}
```

Invariants (enforced by edit ops in §6, checked by `Sequence::validate()` in debug + on load):
- Clips within a track are sorted, non-overlapping, `duration > 0`.
- `clip.start + clip.duration` may exceed any other clip freely across tracks.
- Track vectors are the z/mix order; no separate order table.

## 5. Clip

```rust
pub struct Clip {
    pub id: ClipId,
    pub name: String,                            // defaults to asset name
    pub start: Tick,                             // position in sequence
    pub duration: Tick,
    pub source: ClipSource,
    pub source_in: Tick,                         // offset into source media (trim); 0 for generators
    pub speed: SpeedMap,                         // §5.1
    pub transform: AnimProps<ClipTransform>,     // pos/scale/rotation/anchor/opacity — §6
    pub reframe: HashMap<usize, ClipTransform>,  // per-SequenceFormat static override (CAP-012)
    pub effects: Vec<ClipEffect>,                // ordered stack; each param animatable — §6.3
    pub grade: Option<Grade>,                    // 07-color-grading.md; stored here, evaluated as graph nodes
    pub composition: Option<GraphId>,            // per-clip node graph (D-06); substitutes the clip's SOURCE op only —
                                                 // transform/effects/grade/reframe still apply on top (02 §2 step 3)
    pub transition_in: Option<Transition>,       // {kind, duration, params}; overlaps previous clip
    pub transition_out: Option<Transition>,
    pub audio: Option<ClipAudio>,                // gain, fades, channel map (09)
    pub enabled: bool,
}

pub enum ClipSource {
    Asset { asset: AssetId },                    // video/audio/image via media pool
    Vector { asset: AssetId },                   // AssetKind::VectorDoc — rasterized per frame (CAP-006/021)
    NestedSequence { sequence: SequenceId },     // CAP-005; cycle-checked at edit time
    SolidColor { color: Color },
    Adjustment,                                  // affects everything below (effects/grade apply to composite)
}
```

### 5.1 Speed

```rust
pub enum SpeedMap {
    Constant(Ratio),                             // Ratio { num: i32, den: u32 }; 1/1 default; negative num = reverse
    // Keyframed speed ramps: post-v1 (Non-goal for v1 phases; enum leaves room)
}
```

Mapping: source time = `source_in + (t - start) * speed` in exact rational arithmetic.

## 6. Animation (keyframes)

One generic system animates everything: clip transforms, effect params, audio automation, vector-node properties.

```rust
pub struct AnimProps<T: PropSet> { pub base: T, pub tracks: Vec<PropertyTrack> }

pub struct PropertyTrack {
    pub property: PropPath,                      // e.g. "transform.x", "params.blur_radius" — §6.2
    pub keyframes: Vec<Keyframe>,                // sorted by `at`, unique `at`
}

pub struct Keyframe {
    pub at: Tick,                                // clip-relative
    pub value: PropValue,
    pub interp: Interp,
}

pub enum PropValue { Float(f64), Vec2([f64;2]), Color(Color), Bool(bool), Enum(u32) }

pub enum Interp {
    Hold,
    Linear,
    Bezier { out_handle: [f64;2], in_handle: [f64;2] },  // normalized cubic-bezier ease handles (CSS-like)
}
```

- Evaluation: `fn eval(track: &PropertyTrack, base: &PropValue, t: Tick) -> PropValue` — pure, in core, unit-tested against closed-form expectations (CAP-007 test).
- Bool/Enum interpolate as Hold regardless of `interp`.

### 6.2 PropPath

String paths, validated against a static registry per target kind (`prop_registry.rs`): each effect kind, `ClipTransform`, `TrackAudio`, and the animatable subset of `SceneNode` (transform, opacity, fill colors, stroke width, effect-stack params — all already serde per explorer findings) publish `(path, PropValueKind, range)` entries. Unknown path on load → track kept, flagged `orphaned` (asset may re-register later); never dropped silently.

### 6.3 Effects

```rust
pub struct ClipEffect {
    pub kind: EffectKind,                        // registry enum: Blur, Sharpen, Glow, ChromaKey, LumaKey, Invert, ... — 08 §3's catalog is normative
                                                 // (Transform2D/Crop are dedicated IrOps in 02, NOT effects)
    pub enabled: bool,
    pub params: AnimProps<EffectParams>,         // EffectParams = ordered map PropPath→PropValue seeded from kind defaults
}
```

Raster `AdjustmentSpec`/filter algorithms are ported as effect kinds where GPU-implementable; CPU reference implementations stay for export determinism tests (03 §6).

## 7. Captions

```rust
pub struct CaptionTrack {
    pub id: TrackId,
    pub name: String,
    pub cues: Vec<CaptionCue>,                   // sorted, non-overlapping
    pub style: CaptionStyle,                     // track default
    pub enabled: bool,
}

pub struct CaptionCue {
    pub id: CueId,
    pub start: Tick, pub end: Tick,
    pub words: Vec<CaptionWord>,                 // word-level timing (CAP-009/010)
    pub style_override: Option<CaptionStyle>,
    pub position_override: Option<[f32;2]>,      // normalized sequence coords
}

pub struct CaptionWord { pub text: String, pub start: Tick, pub end: Tick, pub style_override: Option<CaptionStyle> }

pub struct CaptionStyle {
    pub font_family: String, pub font_size: f32, pub weight: u16,
    pub fill: Color, pub stroke: Option<(Color, f32)>,
    pub background: Option<CaptionBackground>,   // {color, corner_radius, padding}
    pub highlight: Option<KaraokeStyle>,         // {mode: FillSweep|WordPop|Underline, active_color, inactive_color}
    pub position: [f32;2], pub max_width: f32,   // normalized
    pub animation: CaptionAnim,                  // None | FadeWords | SlideUp | Typewriter (per-word timing driven)
}
```

Rendering uses the existing text pipeline (glyphon on GPU; CPU compositor path for export) — 06 owns details.

## 8. Node graphs

One arena for all user graphs (per-clip compositions and the project graph):

```rust
pub struct NodeGraph {
    pub id: GraphId,
    pub name: String,
    pub nodes: HashMap<GraphNodeId, GraphNode>,
    pub edges: Vec<GraphEdge>,                   // {from: (GraphNodeId, OutPort), to: (GraphNodeId, InPort)}
    pub output: GraphNodeId,                     // exactly one Output node
    pub ui: HashMap<GraphNodeId, NodePos>,       // editor positions
}

pub struct GraphNode {
    pub id: GraphNodeId,
    pub op: GraphOp,                             // catalog in 08 (MediaIn, ClipIn, Merge, Transform, Blur, Key, Grade, Lut, Text, Mask, Switch, Time, Output, ...)
    pub params: AnimProps<EffectParams>,
}
```

Normative semantics:
- DAG only; edge insertion cycle-checks (edit op fails, never panics).
- `ClipIn` node = "the clip's source after trim/speed, before default effects" — the splice point for per-clip compositions.
- Data-model graphs are *descriptions*; evaluation semantics, port types, and caching live in 02/08. Same `AnimProps` animation system as clips.

## 9. Serialization & migration

- Bump `CURRENT_FORMAT_VERSION` 2 → 3 (`document.rs:104`); add no-op `V2ToV3` migration (`migration.rs`) + `docs/format-versions.md` entry (house style, per repo convention).
- `timeline` is `Option` + `#[serde(default)]` → v2 files load untouched; v3 files without video features omit the key entirely (COMPAT_WINDOW satisfied).
- Asset paths serialize absolute + project-relative (`path`, `rel_path`); loader tries relative first (project moves survive), then absolute, then relink-by-hash.
- Probe data, proxy refs, waveform/keyframe-index caches: probe persists in-file (it's small, needed for offline layout); waveforms/keyframe indices/thumbnails go to a **cache sidecar dir** `<project>.photon.cache/` — never in the JSON (file bloat + churn).

## 10. Undo integration

New nested command keeps `history/mod.rs` churn to one arm per group:

```rust
// history: one new variant
Command::Timeline(TimelineCmd)

pub enum TimelineCmd {
    CreateProject { .. }, AddAsset { .. }, RemoveAsset { .. }, RelinkAsset { asset, old_path, new_path },
    AddSequence { .. }, RemoveSequence { .. },
    SetActiveSequence { old, new }, SetActiveFormat { seq, old, new },  // active_* are document state; changes are undoable
    SetSequenceFormat { .. },                    // covers add/update/remove of format entries (op field; 10 §3.2)
    AddTrack { .. }, RemoveTrack { .. }, SetTrackProp { .. },
    InsertClip { .. }, RemoveClip { .. }, MoveClip { .. }, TrimClip { .. }, SplitClip { .. },
    RippleEdit { .. }, RollEdit { .. }, SlipClip { .. }, SlideClip { .. },
    SetClipProp { old, new },                    // universal property change, mirrors UpdateNode{old,new}
    SetKeyframe { .. }, RemoveKeyframe { .. }, SetKeyframeInterp { .. },
    AddEffect { .. }, RemoveEffect { .. }, ReorderEffects { .. },
    SetGrade { old, new },
    GraphEdit(GraphCmd),                         // add/remove node/edge, set param — 08
    CaptionEdit(CaptionCmd),                     // add/split/merge cues, set text/timing/style — 06
    TtsEdit(TtsCmd),                             // atomic voiceover generate/place (asset+clip+optional captions) — 06 §6
    AudioEdit(AudioCmd),                         // 09
}
```

- Each variant implements `apply`/`inverse`/`description` inside `timeline/commands.rs`; `history/mod.rs` delegates (`Command::Timeline(c) => c.apply(doc)`), so the 4–5-touch-point friction is paid once.
- `coalesce`: drag-move/trim and keyframe-drag coalesce by (variant, clip/keyframe id) like existing `UpdateNode` coalescing.
- Edit ops (`timeline/ops.rs`) are pure functions `fn move_clip(seq: &mut Sequence, ...) -> Result<TimelineCmd, EditError>` producing the command — GUI and MCP both call them (CAP-019 parity).
- **Memory rule:** commands store deltas/ids, never media. `mem_estimate` for timeline commands is O(bytes of changed structs). Playhead position, selection, scroll are **not** document state (no undo entries) — they live in GUI/engine session state.

## 11. What is deliberately NOT in the data model

- Decoded frames, textures, waveform pyramids, thumbnails, keyframe indices → engine caches (02) / sidecar dir (§9).
- Playhead, selection, in/out *while scrubbing* → session state (work_range persists, playhead does not).
- Render/export settings presets → app-level config, except per-sequence format which is document state (§4).
