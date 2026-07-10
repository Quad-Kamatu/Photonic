//! Golden-case generator (11-testing-phasing.md §1.1) — authors the
//! `project.photon` + `meta.toml` fixtures under repo-root `tests/golden/video/`
//! from the core timeline builder API, so the corpus is reproducible from code
//! rather than hand-edited JSON.
//!
//! This is a normal integration test marked `#[ignore]` so it never runs in the
//! default `cargo test` path (it writes into the working tree). Run it
//! explicitly, then bless the PNGs and review both diffs:
//!
//! ```text
//! cargo test -p photonic-video --test gen_video_golden -- --ignored
//! PHOTONIC_BLESS_GOLDEN=1 cargo test -p photonic-video --test golden_frames -- --test-threads=1
//! git diff --stat tests/golden/video/    # reviewed before commit (11 §1.4)
//! ```
//!
//! Entity ids are assigned deterministically (`Uuid::from_u128`) so a
//! regeneration is a clean no-op diff unless the case structure actually
//! changed — matching the reviewed-bless discipline.
//!
//! Cases exercise the P3 `IrOp` set that `eval_cpu` implements (02 §2):
//! `SolidColor`, `Merge` (over + a non-Normal blend mode via a composition),
//! `Transform2D`, `Crop`/`Resize` (via a project graph), multi-track fold,
//! Adjustment re-root, and keyframe-resolved opacity across sampled ticks.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use photonic_core::layer::BlendMode;
use photonic_core::timeline::{
    AssetId, Clip, ClipEffect, ClipId, ClipSource, EffectKind, FitMode, FrameRate, GraphEdge,
    GraphId, GraphNode, GraphNodeId, GraphOp, InPort, Interp, Keyframe, NodeGraph, OutPort,
    PropPath, PropValue, Sequence, SequenceId, Tick, TimelineProject, Track, TrackId, TrackKind,
};
use photonic_core::Color;
use uuid::Uuid;

/// A 2-second clip at 30 fps (60 frames) — every case's clip span.
const TWO_SEC: Tick = Tick(2 * photonic_core::timeline::TICKS_PER_SECOND);

fn uid(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

// ── deterministic builders ───────────────────────────────────────────────────

fn new_seq(name: &str, w: u32, h: u32, base: u128) -> Sequence {
    let mut s = Sequence::new(name, FrameRate::FPS_30, w, h);
    s.id = SequenceId::from(uid(base));
    s
}

fn vtrack(name: &str, id: u128) -> Track {
    let mut t = Track::new(TrackKind::Video, name);
    t.id = TrackId::from(uid(id));
    t
}

fn solid_clip(color: Color, id: u128) -> Clip {
    let mut c = Clip::new(ClipSource::SolidColor { color }, Tick(0), TWO_SEC);
    c.id = ClipId::from(uid(id));
    c
}

fn asset_clip(asset_uid: u128, id: u128) -> Clip {
    let mut c = Clip::new(
        ClipSource::Asset {
            asset: AssetId::from(uid(asset_uid)),
        },
        Tick(0),
        TWO_SEC,
    );
    c.id = ClipId::from(uid(id));
    c
}

fn node(op: GraphOp, id: u128) -> GraphNode {
    let mut g = GraphNode::new(op);
    g.id = GraphNodeId::from(uid(id));
    g
}

// ── cases ────────────────────────────────────────────────────────────────────

/// A single opaque `SolidColor` clip — locks the sRGB→linear→premultiply
/// conversion and the bare `SolidColor → Transform2D → Output` chain.
fn case_solid_color() -> TimelineProject {
    let base = 1_000_000;
    let mut project = TimelineProject::new();
    let mut seq = new_seq("solid_color", 64, 64, base);
    let mut v1 = vtrack("V1", base + 10);
    v1.clips.push(solid_clip(
        Color {
            r: 0.20,
            g: 0.40,
            b: 0.85,
            a: 1.0,
        },
        base + 20,
    ));
    seq.video_tracks.push(v1);
    project.insert_sequence(seq);
    project
}

/// Opaque blue over opaque red on two tracks — `Merge` (Normal, over) and the
/// multi-track fold; the top fully covers the backdrop.
fn case_merge_opaque_over() -> TimelineProject {
    let base = 2_000_000;
    let mut project = TimelineProject::new();
    let mut seq = new_seq("merge_opaque_over", 64, 64, base);
    let mut v1 = vtrack("V1", base + 10);
    v1.clips.push(solid_clip(
        Color {
            r: 0.85,
            g: 0.10,
            b: 0.10,
            a: 1.0,
        },
        base + 20,
    ));
    let mut v2 = vtrack("V2", base + 11);
    v2.clips.push(solid_clip(
        Color {
            r: 0.10,
            g: 0.20,
            b: 0.85,
            a: 1.0,
        },
        base + 21,
    ));
    seq.video_tracks.push(v1);
    seq.video_tracks.push(v2);
    project.insert_sequence(seq);
    project
}

/// White over black at 50% opacity — the exact 50/50 linear blend through the
/// fold `Merge` opacity.
fn case_merge_half_opacity() -> TimelineProject {
    let base = 3_000_000;
    let mut project = TimelineProject::new();
    let mut seq = new_seq("merge_half_opacity", 64, 64, base);
    let mut v1 = vtrack("V1", base + 10);
    v1.clips.push(solid_clip(Color::BLACK, base + 20));
    let mut v2 = vtrack("V2", base + 11);
    let mut white = solid_clip(Color::WHITE, base + 21);
    white.transform.base.opacity = 0.5;
    v2.clips.push(white);
    seq.video_tracks.push(v1);
    seq.video_tracks.push(v2);
    project.insert_sequence(seq);
    project
}

/// A per-clip composition `SolidColor(grey) Merge{Screen} ClipIn → Output` — the
/// only path to a non-Normal blend mode in eval_cpu (track folds are always
/// Normal). Locks the `Screen` blend math (03 §4.4 rule 4).
fn case_merge_screen_blend() -> TimelineProject {
    let base = 4_000_000;
    let mut project = TimelineProject::new();
    let mut seq = new_seq("merge_screen_blend", 64, 64, base);

    let clip_in = node(GraphOp::ClipIn, base + 30);
    let mut solid = node(GraphOp::SolidColor, base + 31);
    solid.params.base.0.set(
        "params.color",
        PropValue::Color(Color {
            r: 0.5,
            g: 0.5,
            b: 0.5,
            a: 1.0,
        }),
    );
    let merge = node(
        GraphOp::Merge {
            mode: BlendMode::Screen,
        },
        base + 32,
    );
    let output = node(GraphOp::Output, base + 33);
    let (ci, so, mg, ou) = (clip_in.id, solid.id, merge.id, output.id);
    let mut nodes = HashMap::new();
    for n in [clip_in, solid, merge, output] {
        nodes.insert(n.id, n);
    }
    let graph = NodeGraph {
        id: GraphId::from(uid(base + 40)),
        name: "screen-comp".into(),
        nodes,
        edges: vec![
            GraphEdge {
                from: (so, OutPort::PRIMARY),
                to: (mg, InPort::A),
            },
            GraphEdge {
                from: (ci, OutPort::PRIMARY),
                to: (mg, InPort::B),
            },
            GraphEdge {
                from: (mg, OutPort::PRIMARY),
                to: (ou, InPort::PRIMARY),
            },
        ],
        output: ou,
        ui: HashMap::new(),
    };
    let gid = graph.id;
    project.graphs.insert(gid, graph);

    let mut v1 = vtrack("V1", base + 10);
    let mut clip = solid_clip(
        Color {
            r: 0.30,
            g: 0.10,
            b: 0.55,
            a: 1.0,
        },
        base + 20,
    );
    clip.composition = Some(gid);
    v1.clips.push(clip);
    seq.video_tracks.push(v1);
    project.insert_sequence(seq);
    project
}

/// A quadrant-pattern source (from the harness `PatternProvider`) scaled 0.6 and
/// rotated ~17° about the frame centre — a non-uniform `Transform2D` a solid
/// cannot express.
fn case_transform2d_scaled() -> TimelineProject {
    let base = 5_000_000;
    let mut project = TimelineProject::new();
    let mut seq = new_seq("transform2d_scaled", 80, 80, base);
    let mut v1 = vtrack("V1", base + 10);
    let mut clip = asset_clip(base + 90, base + 20);
    clip.transform.base.scale_x = 0.6;
    clip.transform.base.scale_y = 0.6;
    clip.transform.base.rotation = 0.30; // radians (~17°)
    clip.transform.base.anchor_x = 40.0; // frame centre for 80×80
    clip.transform.base.anchor_y = 40.0;
    v1.clips.push(clip);
    seq.video_tracks.push(v1);
    project.insert_sequence(seq);
    project
}

/// Red + white@0.5 fold to pink, then a top Adjustment clip (with a passthrough
/// Blur effect) re-roots the composite below — locks that the re-root passes the
/// fold through unchanged (no dropped/duplicated stack).
fn case_adjustment_reroot() -> TimelineProject {
    let base = 6_000_000;
    let mut project = TimelineProject::new();
    let mut seq = new_seq("adjustment_reroot", 64, 64, base);
    let mut v1 = vtrack("V1", base + 10);
    v1.clips.push(solid_clip(
        Color {
            r: 0.85,
            g: 0.10,
            b: 0.10,
            a: 1.0,
        },
        base + 20,
    ));
    let mut v2 = vtrack("V2", base + 11);
    let mut white = solid_clip(Color::WHITE, base + 21);
    white.transform.base.opacity = 0.5;
    v2.clips.push(white);
    let mut v3 = vtrack("V3", base + 12);
    let mut adj = Clip::new(ClipSource::Adjustment, Tick(0), TWO_SEC);
    adj.id = ClipId::from(uid(base + 22));
    adj.effects.push(ClipEffect::new(EffectKind::Blur));
    v3.clips.push(adj);
    seq.video_tracks.push(v1);
    seq.video_tracks.push(v2);
    seq.video_tracks.push(v3);
    project.insert_sequence(seq);
    project
}

/// A project graph `Crop → Resize{Stretch} → Output` spliced after a single
/// pattern clip — exercises the `Crop` and `Resize` IrOps (both identity/
/// passthrough at format size in P3, so the golden locks they do not corrupt
/// the frame).
fn case_crop_resize_passthrough() -> TimelineProject {
    let base = 7_000_000;
    let mut project = TimelineProject::new();
    let mut seq = new_seq("crop_resize_passthrough", 80, 80, base);

    let crop = node(GraphOp::Crop, base + 31);
    let resize = node(
        GraphOp::Resize {
            fit: FitMode::Stretch,
        },
        base + 32,
    );
    let output = node(GraphOp::Output, base + 33);
    let (cr, rz, ou) = (crop.id, resize.id, output.id);
    let mut nodes = HashMap::new();
    for n in [crop, resize, output] {
        nodes.insert(n.id, n);
    }
    // Crop's input is unwired → the program (the pattern clip) feeds it
    // (08 §5 program-splice).
    let pg = NodeGraph {
        id: GraphId::from(uid(base + 40)),
        name: "crop-resize-pg".into(),
        nodes,
        edges: vec![
            GraphEdge {
                from: (cr, OutPort::PRIMARY),
                to: (rz, InPort::PRIMARY),
            },
            GraphEdge {
                from: (rz, OutPort::PRIMARY),
                to: (ou, InPort::PRIMARY),
            },
        ],
        output: ou,
        ui: HashMap::new(),
    };
    let pgid = pg.id;
    project.graphs.insert(pgid, pg);
    project.project_graph = Some(pgid);

    let mut v1 = vtrack("V1", base + 10);
    v1.clips.push(asset_clip(base + 90, base + 20));
    seq.video_tracks.push(v1);
    project.insert_sequence(seq);
    project
}

/// White-over-red with opacity keyframed 1.0→0.0 across the clip — sampled at
/// three ticks (frames 0/30/45 → opacity 1.0/0.5/0.25 → white / pink /
/// mostly-red), exercising compile-time keyframe resolution and multi-tick
/// sampling.
fn case_opacity_ramp() -> TimelineProject {
    let base = 8_000_000;
    let mut project = TimelineProject::new();
    let mut seq = new_seq("opacity_ramp", 64, 64, base);
    let mut v1 = vtrack("V1", base + 10);
    v1.clips.push(solid_clip(
        Color {
            r: 0.85,
            g: 0.10,
            b: 0.10,
            a: 1.0,
        },
        base + 20,
    ));
    let mut v2 = vtrack("V2", base + 11);
    let mut white = solid_clip(Color::WHITE, base + 21);
    {
        let track = white
            .transform
            .track_mut(&PropPath::new("transform.opacity"));
        track.insert_keyframe(Keyframe::new(
            Tick(0),
            PropValue::Float(1.0),
            Interp::Linear,
        ));
        track.insert_keyframe(Keyframe::new(
            TWO_SEC,
            PropValue::Float(0.0),
            Interp::Linear,
        ));
    }
    v2.clips.push(white);
    seq.video_tracks.push(v1);
    seq.video_tracks.push(v2);
    project.insert_sequence(seq);
    project
}

// ── corpus writer ────────────────────────────────────────────────────────────

/// One case's authored fixture: builder output + sampled frames + note.
struct Case {
    name: &'static str,
    project: TimelineProject,
    format_index: usize,
    /// `(frame number, name)` sample points (11 §1.1: cut points, mid-clip,
    /// keyframe extremes — named so failures are legible).
    frames: Vec<(i64, &'static str)>,
    notes: &'static str,
}

fn corpus_root() -> PathBuf {
    // Not canonicalized: the dir may not exist yet on a fresh checkout.
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/video")
}

fn write_case(root: &Path, case: &Case) {
    let dir = root.join(case.name);
    std::fs::create_dir_all(dir.join("expected/cpu"))
        .unwrap_or_else(|e| panic!("create {dir:?}/expected/cpu: {e}"));

    let json = serde_json::to_string_pretty(&case.project)
        .unwrap_or_else(|e| panic!("serialize {} project: {e}", case.name));
    std::fs::write(dir.join("project.photon"), json)
        .unwrap_or_else(|e| panic!("write {dir:?}/project.photon: {e}"));

    let mut meta = String::new();
    meta.push_str(&format!("# Golden case: {}\n", case.name));
    meta.push_str("# Authored by tests/gen_video_golden.rs (11 §1.1). Bless PNGs with\n");
    meta.push_str("# PHOTONIC_BLESS_GOLDEN=1 cargo test -p photonic-video --test golden_frames.\n");
    meta.push_str(&format!("notes = {:?}\n", case.notes));
    meta.push_str(&format!("format_index = {}\n", case.format_index));
    meta.push_str("doc_generation = 1\n\n");
    for (n, name) in &case.frames {
        meta.push_str("[[frames]]\n");
        meta.push_str(&format!("n = {n}\n"));
        meta.push_str(&format!("name = {name:?}\n\n"));
    }
    std::fs::write(dir.join("meta.toml"), meta)
        .unwrap_or_else(|e| panic!("write {dir:?}/meta.toml: {e}"));
}

fn all_cases() -> Vec<Case> {
    vec![
        Case {
            name: "solid_color",
            project: case_solid_color(),
            format_index: 0,
            frames: vec![(0, "start")],
            notes: "one opaque SolidColor clip; sRGB→linear→premultiply",
        },
        Case {
            name: "merge_opaque_over",
            project: case_merge_opaque_over(),
            format_index: 0,
            frames: vec![(0, "opaque-over")],
            notes: "opaque blue over red; Merge Normal + two-track fold",
        },
        Case {
            name: "merge_half_opacity",
            project: case_merge_half_opacity(),
            format_index: 0,
            frames: vec![(0, "half")],
            notes: "white over black at 0.5 opacity; 50/50 linear blend",
        },
        Case {
            name: "merge_screen_blend",
            project: case_merge_screen_blend(),
            format_index: 0,
            frames: vec![(0, "screen")],
            notes: "composition Merge{Screen} over ClipIn; non-Normal blend math",
        },
        Case {
            name: "transform2d_scaled",
            project: case_transform2d_scaled(),
            format_index: 0,
            frames: vec![(0, "scaled-rotated")],
            notes: "quadrant pattern scaled 0.6 + rotated ~17° about centre",
        },
        Case {
            name: "adjustment_reroot",
            project: case_adjustment_reroot(),
            format_index: 0,
            frames: vec![(0, "reroot")],
            notes: "adjustment clip re-roots a red+white@0.5 pink fold, unchanged",
        },
        Case {
            name: "crop_resize_passthrough",
            project: case_crop_resize_passthrough(),
            format_index: 0,
            frames: vec![(0, "passthrough")],
            notes: "project-graph Crop→Resize over a pattern clip; identity at format size",
        },
        Case {
            name: "opacity_ramp",
            project: case_opacity_ramp(),
            format_index: 0,
            frames: vec![(0, "kf-start"), (30, "kf-mid"), (45, "kf-late")],
            notes: "keyframed opacity 1.0→0.0; sampled white / pink / mostly-red",
        },
    ]
}

/// Regenerate every `project.photon` + `meta.toml` under `tests/golden/video/`.
/// `#[ignore]` so the default `cargo test` never writes to the working tree.
#[test]
#[ignore = "generator: writes fixtures into tests/golden/video/ — run with --ignored"]
fn generate_golden_projects() {
    let root = corpus_root();
    let cases = all_cases();
    for case in &cases {
        write_case(&root, case);
    }
    eprintln!(
        "wrote {} golden case(s) to {} — now bless: \
         PHOTONIC_BLESS_GOLDEN=1 cargo test -p photonic-video --test golden_frames -- --test-threads=1",
        cases.len(),
        root.display()
    );
}
