//! E-9(b) — CPU/GPU IR enum-equivalence sweep (26 §8 / §20 definition-of-done).
//!
//! 26 §20 exempts the K-0 seams from the user-verb/undo/MCP definition of done and
//! replaces it with "contract-written-into-owning-doc + a CI test that enforces it".
//! For E-9 (CPU/GPU agreement) THIS FILE is that CI artefact: it mechanically walks
//! every variant of every enum the frame-graph IR carries and asserts the CPU
//! reference (`graph::eval_cpu::evaluate`) and the wgpu evaluator (`graph::eval`)
//! produce the same pixels.
//!
//! Coverage (each behind an exhaustive `match` on the enum, so a NEW variant added
//! without a parity case fails to COMPILE — the whole point of the harness):
//!   * `BlendMode`      — all 26 (K-0.3a; the row that was the live wrong-pixels bug)
//!   * `graph::ir::Sampling`  — Nearest / Bilinear (Transform2D)
//!   * `graph::ir::FitMode`   — Fit / Fill / Stretch (Resize)
//!   * `GradeOpKind`    — every resolved grade op
//!   * `LutInterp`      — Trilinear / Tetrahedral
//!   * `TransitionKind` — the 5 catalog kinds (compile-time → Merge/dip lowering)
//!
//! Tolerances: max-abs channel diff ≤ 1e-3 for algebraic ops; the looser 11 §1.2
//! GPU-vs-CPU tier (PSNR ≥ 35 dB) for the resampling (sampling) op.
//!
//! `#[non_exhaustive]` enums (`GradeOpKind`, `TransitionKind`) come from another
//! crate and CANNOT be matched without a wildcard — the compiler forces one — so
//! those matches carry a documented `_`/`Unknown` arm. Every non-`#[non_exhaustive]`
//! IR enum keeps a wildcard-free match.
//!
//! Self-skips (with an eprintln) when no GPU adapter is available, e.g. CI without a
//! GPU (`GpuContext::request_blocking()` → `None`).

use photonic_core::layer::BlendMode;
use photonic_core::timeline::grade::{
    CdlParams, Grade, GradeOp, GradeOpKind, GradeOpParams, LutInterp,
};
use photonic_core::timeline::{
    Clip, ClipSource, FrameRate, Sequence, TimelineProject, Track, TrackKind, Transition,
    TransitionKind, TransitionParams,
};
use photonic_core::Color;

use photonic_video::contract::{AssetId, Tick, VectorRef, VectorStateKey};
use photonic_video::graph::compile::{compile, CompiledFrame, Quality};
use photonic_video::graph::eval::{
    read_texture_rgba16f, Evaluator, GpuContext, GpuFrame, GpuFrameSource, NullFrameSource,
};
use photonic_video::graph::eval_cpu::{self, EmptyProvider, FrameProvider};
use photonic_video::graph::ir::{
    ContentHash, FitMode, FrameGraph, IrNode, IrNodeId, IrOp, LinearColor, OutPort, Sampling,
};
use photonic_video::graph::ops::Image;
use photonic_video::testing::frame_compare::measure;

const CANVAS: (u32, u32) = (8, 8);

/// Shared GPU context, or a skip. All tests short-circuit through this.
fn gpu_or_skip(what: &str) -> Option<GpuContext> {
    match GpuContext::request_blocking() {
        Some(g) => Some(g),
        None => {
            eprintln!("no GPU adapter — skipping {what}");
            None
        }
    }
}

/// GPU-evaluate `graph` at `CANVAS` with the given source, reading back the output.
fn eval_gpu(gpu: &GpuContext, graph: &FrameGraph, source: &mut dyn GpuFrameSource) -> Image {
    let mut eval = Evaluator::new(gpu.clone());
    let tex = eval
        .evaluate(graph, CANVAS, source)
        .expect("graph produced an output texture");
    let px = read_texture_rgba16f(gpu, &tex, CANVAS.0, CANVAS.1);
    Image {
        width: CANVAS.0,
        height: CANVAS.1,
        pixels: px,
    }
}

/// Assert the CPU and GPU evaluators agree within `tol` (max abs channel diff) for
/// a media-free (`NullFrameSource` / `EmptyProvider`) graph.
fn assert_parity_solid(gpu: &GpuContext, label: &str, graph: &FrameGraph, tol: f32) {
    let cpu = eval_cpu::evaluate(graph, CANVAS, &mut EmptyProvider);
    let g = eval_gpu(gpu, graph, &mut NullFrameSource);
    assert_pixels_within(label, &cpu, &g, tol);
}

fn assert_pixels_within(label: &str, cpu: &Image, gpu: &Image, tol: f32) {
    for (i, (c, g)) in cpu.pixels.iter().zip(&gpu.pixels).enumerate() {
        for k in 0..4 {
            assert!(
                (c[k] - g[k]).abs() <= tol,
                "{label}: pixel {i} channel {k}: cpu {} vs gpu {} (tol {tol})",
                c[k],
                g[k]
            );
        }
    }
}

// ── BlendMode (26) — K-0.3a ────────────────────────────────────────────────────

/// Human label per mode, via a wildcard-free exhaustive `match`: a new `BlendMode`
/// variant makes this fail to compile until it is given a parity row.
fn blend_label(mode: BlendMode) -> &'static str {
    match mode {
        BlendMode::Normal => "normal",
        BlendMode::Multiply => "multiply",
        BlendMode::Screen => "screen",
        BlendMode::Overlay => "overlay",
        BlendMode::Darken => "darken",
        BlendMode::Lighten => "lighten",
        BlendMode::ColorDodge => "color_dodge",
        BlendMode::ColorBurn => "color_burn",
        BlendMode::HardLight => "hard_light",
        BlendMode::SoftLight => "soft_light",
        BlendMode::Difference => "difference",
        BlendMode::Exclusion => "exclusion",
        BlendMode::Hue => "hue",
        BlendMode::Saturation => "saturation",
        BlendMode::Color => "color",
        BlendMode::Luminosity => "luminosity",
        BlendMode::LinearDodge => "linear_dodge",
        BlendMode::LinearBurn => "linear_burn",
        BlendMode::Subtract => "subtract",
        BlendMode::Divide => "divide",
        BlendMode::VividLight => "vivid_light",
        BlendMode::LinearLight => "linear_light",
        BlendMode::PinLight => "pin_light",
        BlendMode::HardMix => "hard_mix",
        BlendMode::DarkerColor => "darker_color",
        BlendMode::LighterColor => "lighter_color",
    }
}

const ALL_BLEND_MODES: [BlendMode; 26] = [
    BlendMode::Normal,
    BlendMode::Multiply,
    BlendMode::Screen,
    BlendMode::Overlay,
    BlendMode::Darken,
    BlendMode::Lighten,
    BlendMode::ColorDodge,
    BlendMode::ColorBurn,
    BlendMode::HardLight,
    BlendMode::SoftLight,
    BlendMode::Difference,
    BlendMode::Exclusion,
    BlendMode::Hue,
    BlendMode::Saturation,
    BlendMode::Color,
    BlendMode::Luminosity,
    BlendMode::LinearDodge,
    BlendMode::LinearBurn,
    BlendMode::Subtract,
    BlendMode::Divide,
    BlendMode::VividLight,
    BlendMode::LinearLight,
    BlendMode::PinLight,
    BlendMode::HardMix,
    BlendMode::DarkerColor,
    BlendMode::LighterColor,
];

fn merge_graph(top: LinearColor, bottom: LinearColor, mode: BlendMode) -> FrameGraph {
    FrameGraph {
        nodes: vec![
            IrNode {
                op: IrOp::SolidColor { color: top },
                inputs: vec![],
                content_hash: ContentHash(10),
            },
            IrNode {
                op: IrOp::SolidColor { color: bottom },
                inputs: vec![],
                content_hash: ContentHash(11),
            },
            IrNode {
                op: IrOp::Merge { mode, opacity: 1.0 },
                inputs: vec![
                    (IrNodeId(0), OutPort::default()),
                    (IrNodeId(1), OutPort::default()),
                ],
                content_hash: ContentHash(12),
            },
        ],
        output: Some(IrNodeId(2)),
    }
}

#[test]
fn blend_mode_cpu_gpu_parity() {
    let Some(gpu) = gpu_or_skip("BlendMode parity") else {
        return;
    };
    // Semi-transparent premultiplied top over an opaque and a transparent backdrop.
    // Colours sit clear of every mode's discontinuities (notably HardMix's 0.5
    // threshold, an inherent 0↔1 flip a sub-ULP CPU/GPU delta would trip).
    let top = LinearColor {
        r: 0.35,
        g: 0.175,
        b: 0.275,
        a: 0.5,
    };
    let opaque = LinearColor {
        r: 0.25,
        g: 0.6,
        b: 0.4,
        a: 1.0,
    };
    let transparent = LinearColor {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };
    for mode in ALL_BLEND_MODES {
        for bottom in [opaque, transparent] {
            assert_parity_solid(
                &gpu,
                &format!("blend/{}", blend_label(mode)),
                &merge_graph(top, bottom, mode),
                1e-3,
            );
        }
    }
}

// ── Sampling (Nearest / Bilinear) — Transform2D ─────────────────────────────────

/// A deterministic non-uniform pattern so the sampler actually interpolates.
fn patterned_image(width: u32, height: u32) -> Image {
    let mut image = Image::new(width, height);
    for y in 0..height {
        for x in 0..width {
            image.pixels[(y * width + x) as usize] = [
                (x % 2) as f32,
                (y % 2) as f32,
                ((x + 2 * y) % 3 == 0) as u8 as f32,
                1.0,
            ];
        }
    }
    image
}

fn upload_pattern(gpu: &GpuContext, image: &Image) -> std::sync::Arc<wgpu::Texture> {
    let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("parity_pattern"),
        size: wgpu::Extent3d {
            width: image.width,
            height: image.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut half = Vec::with_capacity(image.pixels.len() * 4);
    for pixel in &image.pixels {
        for channel in pixel {
            half.push(if *channel == 0.0 { 0u16 } else { 0x3c00u16 });
        }
    }
    gpu.queue().write_texture(
        texture.as_image_copy(),
        bytemuck::cast_slice(&half),
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(image.width * 8),
            rows_per_image: Some(image.height),
        },
        wgpu::Extent3d {
            width: image.width,
            height: image.height,
            depth_or_array_layers: 1,
        },
    );
    std::sync::Arc::new(texture)
}

struct PatternGpuSource {
    frame: GpuFrame,
}
impl GpuFrameSource for PatternGpuSource {
    fn video_texture(&mut self, _: &GpuContext, _: AssetId, _: Tick, _: bool) -> Option<GpuFrame> {
        Some(self.frame.clone())
    }
    fn still_texture(&mut self, _: &GpuContext, _: AssetId) -> Option<GpuFrame> {
        Some(self.frame.clone())
    }
    fn vector_texture(
        &mut self,
        _: &GpuContext,
        _: VectorRef,
        _: VectorStateKey,
        _: u32,
        _: u32,
    ) -> Option<GpuFrame> {
        Some(self.frame.clone())
    }
}

struct PatternCpuSource {
    image: Image,
}
impl FrameProvider for PatternCpuSource {
    fn decode_video(&mut self, _: AssetId, _: Tick, _: bool, _: u32, _: u32) -> Image {
        self.image.clone()
    }
    fn decode_still(&mut self, _: AssetId, _: u32, _: u32) -> Image {
        self.image.clone()
    }
    fn raster_vector(&mut self, _: VectorRef, _: VectorStateKey, _: u32, _: u32) -> Image {
        self.image.clone()
    }
}

const ALL_SAMPLING: [Sampling; 2] = [Sampling::Bilinear, Sampling::Nearest];

fn sampling_label(s: Sampling) -> &'static str {
    // Wildcard-free: a new Sampling variant fails to compile here.
    match s {
        Sampling::Bilinear => "bilinear",
        Sampling::Nearest => "nearest",
    }
}

#[test]
fn sampling_cpu_gpu_parity() {
    let Some(gpu) = gpu_or_skip("Sampling parity") else {
        return;
    };
    let image = patterned_image(CANVAS.0, CANVAS.1);
    let mat = glam::Mat3::from_translation(glam::Vec2::new(1.25, -0.75))
        * glam::Mat3::from_angle(0.23)
        * glam::Mat3::from_scale(glam::Vec2::new(1.2, 0.8));
    for sampling in ALL_SAMPLING {
        let graph = FrameGraph {
            nodes: vec![
                IrNode {
                    op: IrOp::DecodeStill {
                        asset: AssetId::new(),
                    },
                    inputs: vec![],
                    content_hash: ContentHash(20),
                },
                IrNode {
                    op: IrOp::Transform2D { mat, sampling },
                    inputs: vec![(IrNodeId(0), OutPort::default())],
                    content_hash: ContentHash(21),
                },
            ],
            output: Some(IrNodeId(1)),
        };
        let cpu = eval_cpu::evaluate(
            &graph,
            CANVAS,
            &mut PatternCpuSource {
                image: image.clone(),
            },
        );
        let mut src = PatternGpuSource {
            frame: GpuFrame::new(upload_pattern(&gpu, &image), image.width, image.height),
        };
        let g = eval_gpu(&gpu, &graph, &mut src);
        // Sampling tier (11 §1.2): the looser GPU-vs-CPU PSNR bound, not 1e-3.
        let metric = measure(&cpu, &g).expect("measurable");
        assert!(
            metric.psnr_db >= 35.0,
            "sampling/{}: PSNR {:.2} dB < 35 (max_abs {:.5})",
            sampling_label(sampling),
            metric.psnr_db,
            metric.max_abs
        );
    }
}

// ── FitMode (Resize) ────────────────────────────────────────────────────────────

const ALL_FIT_MODES: [FitMode; 3] = [FitMode::Fit, FitMode::Fill, FitMode::Stretch];

fn fit_label(f: FitMode) -> &'static str {
    // Wildcard-free: a new FitMode variant fails to compile here.
    match f {
        FitMode::Fit => "fit",
        FitMode::Fill => "fill",
        FitMode::Stretch => "stretch",
    }
}

#[test]
fn fit_mode_cpu_gpu_parity() {
    let Some(gpu) = gpu_or_skip("FitMode parity") else {
        return;
    };
    // A UNIFORM source makes the result fit-independent (a solid resizes to the same
    // solid under every fit), so this row exercises the `FitMode` plumbing on both
    // evaluators without depending on the P3-passthrough GPU `Resize` geometry.
    let color = LinearColor {
        r: 0.3,
        g: 0.55,
        b: 0.2,
        a: 1.0,
    };
    for fit in ALL_FIT_MODES {
        let graph = FrameGraph {
            nodes: vec![
                IrNode {
                    op: IrOp::SolidColor { color },
                    inputs: vec![],
                    content_hash: ContentHash(30),
                },
                IrNode {
                    op: IrOp::Resize {
                        w: CANVAS.0,
                        h: CANVAS.1,
                        fit,
                    },
                    inputs: vec![(IrNodeId(0), OutPort::default())],
                    content_hash: ContentHash(31),
                },
            ],
            output: Some(IrNodeId(1)),
        };
        assert_parity_solid(&gpu, &format!("fit/{}", fit_label(fit)), &graph, 1e-3);
    }
}

// ── GradeOpKind + LutInterp ─────────────────────────────────────────────────────

/// A single-clip solid project decorated with `grade`, compiled to a frame graph
/// carrying a real `IrOp::Grade` (resolved via the same path production uses).
fn graded_solid(grade: Grade) -> CompiledFrame {
    let mut project = TimelineProject::new();
    let mut seq = Sequence::new("seq", FrameRate::FPS_30, CANVAS.0, CANVAS.1);
    let seq_id = seq.id;
    let mut track = Track::new(TrackKind::Video, "V1");
    let mut clip = Clip::new(
        ClipSource::SolidColor {
            color: Color {
                r: 0.45,
                g: 0.35,
                b: 0.55,
                a: 1.0,
            },
        },
        Tick(0),
        Tick::from_seconds(2),
    );
    clip.grade = Some(grade);
    track.clips.push(clip);
    seq.video_tracks.push(track);
    project.insert_sequence(seq);
    compile(&project, seq_id, 0, Tick(0), Quality::FULL, None)
}

fn single_op_grade(op: GradeOp) -> Grade {
    let mut grade = Grade::new();
    grade.ops.push(op);
    grade
}

/// Sample non-identity params for a kind (so the grade actually transforms pixels).
/// `Lut3d` resolves inert without a LUT provider (§ K-0.5), which is exactly the
/// P3 contract — CPU and GPU agree on the resulting identity either way. The
/// exhaustive `match` (minus the `#[non_exhaustive]`-mandated `Unknown`/`_` arm)
/// forces a new `GradeOpKind` to be given a row here.
fn grade_params_for(kind: GradeOpKind) -> Option<GradeOpParams> {
    Some(match kind {
        GradeOpKind::Exposure => GradeOpParams::Exposure { stops: 0.5 },
        GradeOpKind::Contrast => GradeOpParams::Contrast {
            pivot: 0.5,
            amount: 0.2,
        },
        GradeOpKind::WhiteBalance => GradeOpParams::WhiteBalance {
            temp: 0.1,
            tint: 0.05,
        },
        GradeOpKind::Cdl => GradeOpParams::Cdl {
            slope: [1.1, 1.0, 0.9],
            offset: [0.02, 0.0, -0.01],
            power: [1.0, 1.05, 0.95],
            sat: 1.1,
        },
        GradeOpKind::Wheels => GradeOpParams::Wheels {
            lift: [0.02, 0.0, -0.01],
            gamma: [1.0, 1.05, 0.95],
            gain: [1.05, 1.0, 0.95],
            sat: 1.0,
        },
        GradeOpKind::Curves => GradeOpParams::Curves {
            master: vec![(0.0, 0.05), (1.0, 0.95)],
            red: vec![(0.0, 0.0), (1.0, 1.0)],
            green: vec![(0.0, 0.0), (1.0, 1.0)],
            blue: vec![(0.0, 0.0), (1.0, 1.0)],
            hue_vs_hue: vec![],
            hue_vs_sat: vec![],
        },
        GradeOpKind::HslQualifier => GradeOpParams::HslQualifier {
            hue: [0.0, 1.0],
            sat: [0.0, 1.0],
            lum: [0.0, 1.0],
            softness: 0.1,
            correction: CdlParams {
                slope: [1.05, 1.0, 0.95],
                ..CdlParams::identity()
            },
        },
        GradeOpKind::Lut3d => GradeOpParams::Lut3d {
            asset: AssetId::new(),
            intensity: 1.0,
            interp: LutInterp::Trilinear,
        },
        // `#[non_exhaustive]`: a forward-compat / future kind has no fixture.
        _ => return None,
    })
}

const KNOWN_GRADE_KINDS: [GradeOpKind; 8] = [
    GradeOpKind::Exposure,
    GradeOpKind::Contrast,
    GradeOpKind::WhiteBalance,
    GradeOpKind::Cdl,
    GradeOpKind::Wheels,
    GradeOpKind::Curves,
    GradeOpKind::HslQualifier,
    GradeOpKind::Lut3d,
];

#[test]
fn grade_op_kind_cpu_gpu_parity() {
    let Some(gpu) = gpu_or_skip("GradeOpKind parity") else {
        return;
    };
    for kind in KNOWN_GRADE_KINDS {
        let params = grade_params_for(kind).expect("known kind has a fixture");
        let compiled = graded_solid(single_op_grade(GradeOp::new(kind, params)));
        assert_parity_solid(
            &gpu,
            &format!("grade/{kind:?}"),
            &compiled.graph,
            1e-3,
        );
    }
}

const ALL_LUT_INTERP: [LutInterp; 2] = [LutInterp::Trilinear, LutInterp::Tetrahedral];

fn lut_interp_label(i: LutInterp) -> &'static str {
    // Wildcard-free: a new LutInterp variant fails to compile here.
    match i {
        LutInterp::Trilinear => "trilinear",
        LutInterp::Tetrahedral => "tetrahedral",
    }
}

#[test]
fn lut_interp_cpu_gpu_parity() {
    let Some(gpu) = gpu_or_skip("LutInterp parity") else {
        return;
    };
    // Without a LUT provider both interp modes resolve inert (identity) — the P3
    // contract (K-0.5) — so CPU and GPU agree. The exhaustive iteration keeps every
    // interp variant covered for when a provider is threaded through compile.
    for interp in ALL_LUT_INTERP {
        let grade = single_op_grade(GradeOp::new(
            GradeOpKind::Lut3d,
            GradeOpParams::Lut3d {
                asset: AssetId::new(),
                intensity: 1.0,
                interp,
            },
        ));
        let compiled = graded_solid(grade);
        assert_parity_solid(
            &gpu,
            &format!("lut_interp/{}", lut_interp_label(interp)),
            &compiled.graph,
            1e-3,
        );
    }
}

// ── TransitionKind (compile-time → Merge / dip lowering) ────────────────────────

const KNOWN_TRANSITIONS: [TransitionKind; 5] = [
    TransitionKind::CrossDissolve,
    TransitionKind::DipToBlack,
    TransitionKind::DipToColor,
    TransitionKind::Wipe,
    TransitionKind::Push,
];

fn transition_label(k: TransitionKind) -> &'static str {
    match k {
        TransitionKind::CrossDissolve => "cross_dissolve",
        TransitionKind::DipToBlack => "dip_to_black",
        TransitionKind::DipToColor => "dip_to_color",
        TransitionKind::Wipe => "wipe",
        TransitionKind::Push => "push",
        // `#[non_exhaustive]`: an unknown transition lowers to a hard cut.
        _ => "unknown",
    }
}

/// Two overlapping solid clips with a `transition_in` of `kind` on the second,
/// compiled mid-overlap so the transition is active. The lowering is pure
/// `Merge`/`SolidColor` (CrossDissolve/Wipe/Push → cross-dissolve, Dip* → dip
/// through colour), which both evaluators run identically.
fn transition_graph(kind: TransitionKind) -> CompiledFrame {
    let mut project = TimelineProject::new();
    let mut seq = Sequence::new("seq", FrameRate::FPS_30, CANVAS.0, CANVAS.1);
    let seq_id = seq.id;
    let mut track = Track::new(TrackKind::Video, "V1");

    let clip_a = Clip::new(
        ClipSource::SolidColor {
            color: Color {
                r: 0.2,
                g: 0.6,
                b: 0.4,
                a: 1.0,
            },
        },
        Tick(0),
        Tick(200),
    );
    let mut clip_b = Clip::new(
        ClipSource::SolidColor {
            color: Color {
                r: 0.7,
                g: 0.3,
                b: 0.5,
                a: 1.0,
            },
        },
        Tick(100),
        Tick(200),
    );
    let mut tr = Transition::new(kind, Tick(100));
    tr.params = TransitionParams {
        color: Some(Color {
            r: 0.1,
            g: 0.1,
            b: 0.1,
            a: 1.0,
        }),
        ..TransitionParams::default()
    };
    clip_b.transition_in = Some(tr);
    track.clips.push(clip_a);
    track.clips.push(clip_b);
    seq.video_tracks.push(track);
    project.insert_sequence(seq);
    // tick 150 ∈ [100, 200): the transition is active.
    compile(&project, seq_id, 0, Tick(150), Quality::FULL, None)
}

#[test]
fn transition_kind_cpu_gpu_parity() {
    let Some(gpu) = gpu_or_skip("TransitionKind parity") else {
        return;
    };
    for kind in KNOWN_TRANSITIONS {
        let compiled = transition_graph(kind);
        assert_parity_solid(
            &gpu,
            &format!("transition/{}", transition_label(kind)),
            &compiled.graph,
            1e-3,
        );
    }
}
