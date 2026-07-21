//! Frame-graph IR, compiler, evaluator, and result cache (02 §2).
//!
//! - [`ir`] — the normative IR type contract (02 §2, pinned in P1).
//! - [`compile`] — timeline snapshot → [`ir::FrameGraph`] (02 §2 compilation).
//! - [`ops`] — CPU pixel kernels shared by the reference evaluator.
//! - [`eval_cpu`] — the f32 CPU reference evaluator (golden/parity tests, 03 §6).
//! - [`cache`] — content-hash → GPU-texture result cache over the pool (02 §5).
//! - [`eval`] — the wgpu evaluator (02 §2 evaluation).

pub mod ir;

pub mod cache;
pub mod compile;
pub mod eval;
pub mod eval_cpu;
pub mod ops;
pub mod panorama;
pub mod panorama_gpu;

pub use compile::{
    compile, CompileCode, CompileDiagnostic, CompiledFrame, DiagSeverity, Quality, ViewNodeOverride,
};
