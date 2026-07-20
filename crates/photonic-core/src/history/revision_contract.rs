//! FAILING-BY-DESIGN TDD contract tests for the P1 renderer prerequisite:
//! `docs/specs/video-editor/03-render-color-pipeline.md` §2.1's revision
//! counter + affected-node tracking.
//!
//! **This entire module is gated behind the `video-p1-contract` Cargo
//! feature, which is off by default.** The API these tests exercise —
//! `CommandHistory::revision()`, `CommandHistory::changes_since()`,
//! `Command::affected_nodes()`, and the `ChangeSummary` type — does not exist
//! in this crate yet (03 §2.1 documents the exact gap against current code).
//! Writing these tests against real `CommandHistory`/`Command` types means
//! they cannot compile without that API existing, so `#[ignore]` alone would
//! not keep `cargo test --workspace` green (ignore skips *running*, not
//! *compiling*). The feature gate is the mechanism that does: the module is
//! not even parsed under default features, so `cargo build/test --workspace`
//! is unaffected by this file's existence.
//!
//! Enabling the feature today is expected to FAIL TO COMPILE — that compile
//! failure *is* the red phase of TDD for an API that doesn't exist yet:
//!
//! ```sh
//! cargo test -p photonic-core --features video-p1-contract
//! ```
//!
//! P1 implementation work (03 §2.1's "Spec position") makes this compile and
//! pass; the module then graduates to always-on (feature gate removed) as
//! part of that same change. See `docs/specs/video-editor/14-qa-spec.md` §5.

#![cfg(feature = "video-p1-contract")]

use crate::document::Document;
use crate::history::{Command, CommandHistory};
use crate::node::{NodeId, PathNode, SceneNode, SceneNodeKind};
use crate::path::PathData;
use std::collections::HashSet;

fn make_doc() -> Document {
    Document::new("revision-contract-test", 100.0, 100.0)
}

fn make_node(doc: &Document) -> SceneNode {
    let layer_id = doc.active_layer_id.unwrap();
    SceneNode::new(
        "rect",
        layer_id,
        SceneNodeKind::Path(PathNode::new(PathData::rect(0.0, 0.0, 10.0, 10.0))),
    )
}

// ─── revision() ─────────────────────────────────────────────────────────────

/// 03 §2.1: "extend this field, don't add a parallel one... add
/// `pub fn revision(&self) -> u64`."
#[test]
fn revision_accessor_exists_and_bumps_on_execute() {
    let mut history = CommandHistory::new(200);
    let mut doc = make_doc();
    let before = history.revision();

    let node = make_node(&doc);
    let layer_id = node.layer_id;
    history.execute(
        Command::AddNode {
            node,
            layer_id: Some(layer_id),
        },
        &mut doc,
    );

    assert_ne!(
        history.revision(),
        before,
        "execute() must bump revision (03 §2.1: 'execute/undo/redo each bump self.revision')"
    );
}

#[test]
fn revision_bumps_on_undo_and_redo() {
    let mut history = CommandHistory::new(200);
    let mut doc = make_doc();

    let node = make_node(&doc);
    let layer_id = node.layer_id;
    history.execute(
        Command::AddNode {
            node,
            layer_id: Some(layer_id),
        },
        &mut doc,
    );
    let after_execute = history.revision();

    history.undo(&mut doc);
    let after_undo = history.revision();
    assert_ne!(
        after_undo, after_execute,
        "undo() must bump revision (03 §2.1)"
    );

    history.redo(&mut doc);
    let after_redo = history.revision();
    assert_ne!(
        after_redo, after_undo,
        "redo() must bump revision (03 §2.1)"
    );
}

// ─── Command::affected_nodes() ──────────────────────────────────────────────

/// 03 §2.1: "add `fn affected_nodes(&self) -> SmallVec<[NodeId; 4]>` to
/// `Command`, one match arm per existing variant returning the id(s) already
/// stored on it."
#[test]
fn affected_nodes_reports_the_node_a_command_touches() {
    let doc = make_doc();
    let node = make_node(&doc);
    let node_id = node.id;
    let layer_id = node.layer_id;

    let cmd = Command::AddNode {
        node,
        layer_id: Some(layer_id),
    };
    let affected: Vec<NodeId> = cmd.affected_nodes().into_iter().collect();
    assert_eq!(
        affected,
        vec![node_id],
        "AddNode's affected_nodes() must report the node it adds"
    );
}

#[test]
fn affected_nodes_for_update_node_reports_the_updated_id() {
    let doc = make_doc();
    let old_node = make_node(&doc);
    let mut new_node = old_node.clone();
    new_node.opacity = 0.5;
    let node_id = new_node.id;

    let cmd = Command::UpdateNode {
        old: old_node,
        new: new_node,
    };
    let affected: Vec<NodeId> = cmd.affected_nodes().into_iter().collect();
    assert_eq!(affected, vec![node_id]);
}

// ─── changes_since() ─────────────────────────────────────────────────────────

/// 03 §2.1: "Expose `fn changes_since(&self, from: u64) -> ChangeSummary
/// { revision: u64, touched: HashSet<NodeId>, overflowed: bool }`."
#[test]
fn changes_since_reports_touched_node_after_single_execute() {
    let mut history = CommandHistory::new(200);
    let mut doc = make_doc();
    let baseline = history.revision();

    let node = make_node(&doc);
    let node_id = node.id;
    let layer_id = node.layer_id;
    history.execute(
        Command::AddNode {
            node,
            layer_id: Some(layer_id),
        },
        &mut doc,
    );

    let summary = history.changes_since(baseline);
    assert!(
        !summary.overflowed,
        "a single-command gap must not overflow"
    );
    assert_eq!(summary.revision, history.revision());
    assert!(
        summary.touched.contains(&node_id),
        "changes_since(baseline) must include the node the execute()'d command touched"
    );
}

#[test]
fn changes_since_unions_touched_nodes_across_multiple_commands() {
    let mut history = CommandHistory::new(200);
    let mut doc = make_doc();
    let baseline = history.revision();

    let node_a = make_node(&doc);
    let id_a = node_a.id;
    let layer_id = node_a.layer_id;
    history.execute(
        Command::AddNode {
            node: node_a,
            layer_id: Some(layer_id),
        },
        &mut doc,
    );

    let node_b = make_node(&doc);
    let id_b = node_b.id;
    history.execute(
        Command::AddNode {
            node: node_b,
            layer_id: Some(layer_id),
        },
        &mut doc,
    );

    let summary = history.changes_since(baseline);
    let expected: HashSet<NodeId> = [id_a, id_b].into_iter().collect();
    assert_eq!(
        summary.touched, expected,
        "changes_since must union touched nodes across every command since `from`"
    );
}

#[test]
fn changes_since_from_current_revision_is_empty_and_not_overflowed() {
    let mut history = CommandHistory::new(200);
    let mut doc = make_doc();

    let node = make_node(&doc);
    let layer_id = node.layer_id;
    history.execute(
        Command::AddNode {
            node,
            layer_id: Some(layer_id),
        },
        &mut doc,
    );

    let now = history.revision();
    let summary = history.changes_since(now);
    assert!(summary.touched.is_empty());
    assert!(!summary.overflowed);
}

/// 03 §2.1: "a small ring (last ~64 entries)... `overflowed = true` when
/// `from` predates the ring, signaling 'invalidate everything.'"
#[test]
fn changes_since_overflows_when_from_predates_the_ring() {
    let mut history = CommandHistory::new(1000);
    let mut doc = make_doc();
    let baseline = history.revision();

    // Push well past the documented ~64-entry ring so `baseline` is stale.
    for _ in 0..100 {
        let node = make_node(&doc);
        let layer_id = node.layer_id;
        history.execute(
            Command::AddNode {
                node,
                layer_id: Some(layer_id),
            },
            &mut doc,
        );
    }

    let summary = history.changes_since(baseline);
    assert!(
        summary.overflowed,
        "changes_since() must report overflowed=true once `from` predates the retained ring \
         (03 §2.1: 'older history is covered by unknown range -> invalidate all fallback, \
         never a correctness issue, only a cache-hit-rate one')"
    );
}
