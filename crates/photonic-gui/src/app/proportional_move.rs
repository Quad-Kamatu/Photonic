//! Proportional Move — the Direct Select sub-variant's falloff math.
//!
//! The implementation (and its unit tests) now live in
//! [`photonic_core::ops::proportional`] so the interactive GUI tool and the
//! `proportional_move_anchor` MCP tool share one source of truth. This module
//! re-exports it, so the tool's call sites (`proportional_move::…`) and overlay
//! keep working unchanged.
pub use photonic_core::ops::proportional::*;
