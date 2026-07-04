use super::*;

impl CommandHistory {
    pub fn new(max_depth: usize) -> Self {
        Self {
            undo_stack: vec![],
            redo_stack: vec![],
            max_depth,
            size_limit_bytes: None,
            warned_at_limit: false,
            pending_warning: None,
            checkpoints: vec![],
            branches: std::collections::HashMap::new(),
            gui_debounce: DebounceCheckpoint::new(30),
            mcp_debounce: DebounceCheckpoint::new(60),
            revision: 0,
            coalescing: false,
            coalesce_started: false,
        }
    }

    // ── Configurable history limits ──────────────────────────────────────────

    /// Soft floor on undo steps the size cap trims down to: while over budget we
    /// keep at least this many recent undo steps before falling back to trimming
    /// the redo stack. As an absolute last resort (redo empty, still over) undo
    /// may be taken below this, down to a single step. Named checkpoints and
    /// branches are deliberate user artifacts and are NEVER auto-trimmed.
    const MIN_RETAINED_STEPS: usize = 5;

    /// Set the retention limits and immediately re-enforce them.
    ///
    /// `max_steps` is the hard step ceiling (always >= 1). `size_bytes` is the
    /// optional cap on the serialized history payload. Cheap and idempotent when
    /// the limits are unchanged, so callers may invoke it every frame.
    pub fn set_limits(&mut self, max_steps: usize, size_bytes: Option<u64>) {
        let max_steps = max_steps.max(1);
        if self.max_depth == max_steps && self.size_limit_bytes == size_bytes {
            return;
        }
        self.max_depth = max_steps;
        self.size_limit_bytes = size_bytes;
        self.enforce_steps();
        self.enforce_size();
    }

    /// The configured step ceiling.
    pub fn max_depth(&self) -> usize {
        self.max_depth
    }

    /// The configured size cap in bytes, if any.
    pub fn size_limit_bytes(&self) -> Option<u64> {
        self.size_limit_bytes
    }

    /// Serialized size, in bytes, of the persistent history payload — exactly
    /// what gets written into the `.photon` file. This is the "history size"
    /// the size cap constrains (the document is measured separately).
    pub fn history_byte_size(&self) -> u64 {
        serde_json::to_vec(&self.snapshot_state())
            .map(|v| v.len() as u64)
            .unwrap_or(0)
    }

    /// Drop oldest undo steps until within the step ceiling. Cheap — no
    /// serialization. Latches a warning on the first step actually dropped.
    pub(crate) fn enforce_steps(&mut self) {
        let mut dropped = false;
        while self.undo_stack.len() > self.max_depth {
            self.undo_stack.remove(0);
            dropped = true;
        }
        // Recovered comfortably under the ceiling → re-arm the warning latch.
        if self.undo_stack.len() * 10 < self.max_depth * 9 {
            self.warned_at_limit = false;
        }
        if dropped {
            self.latch_warning(
                "Project history reached its maximum step count — the oldest \
                 undo steps are being discarded. Raise the limit in \
                 Edit ▸ Behavior ▸ Project History.",
            );
        }
    }

    /// Enforce the optional size cap by trimming the linear undo/redo history
    /// until the serialized payload is within budget. Named checkpoints and
    /// branches are user artifacts and are never auto-deleted — if they alone
    /// exceed the budget, a distinct warning is raised instead. No-op when no
    /// size cap is configured. Returns true if it dropped any step.
    ///
    /// Measures the whole history once, then trims against a running byte
    /// estimate (each removed entry's own serialized size), so the cost is
    /// O(history size) rather than O(entries · history size). One exact
    /// re-measure at the end drives the warning + re-arm decisions.
    pub fn enforce_size(&mut self) -> bool {
        let Some(limit) = self.size_limit_bytes else {
            return false;
        };

        let mut est = self.history_byte_size();
        let mut dropped = false;
        while est > limit {
            // `+1` approximates the JSON array separator per element.
            if self.undo_stack.len() > Self::MIN_RETAINED_STEPS {
                est = est.saturating_sub(entry_byte_size(&self.undo_stack[0]).saturating_add(1));
                self.undo_stack.remove(0);
            } else if !self.redo_stack.is_empty() {
                est = est.saturating_sub(entry_byte_size(&self.redo_stack[0]).saturating_add(1));
                self.redo_stack.remove(0);
            } else if self.undo_stack.len() > 1 {
                est = est.saturating_sub(entry_byte_size(&self.undo_stack[0]).saturating_add(1));
                self.undo_stack.remove(0);
            } else {
                // Only a single undo step plus un-trimmable checkpoints/branches
                // remain. Stop rather than wipe the last step.
                break;
            }
            dropped = true;
        }

        // Exact size now drives the (accurate) warning and the re-arm latch.
        let actual = self.history_byte_size();
        if actual > limit {
            self.latch_warning(
                "Project history exceeds its size limit because of saved \
                 checkpoints or branches — delete some, or raise the limit in \
                 Edit ▸ Behavior ▸ Project History.",
            );
        } else if dropped {
            self.latch_warning(
                "Project history reached its size limit — the oldest undo steps \
                 are being discarded to make room. Raise the limit in \
                 Edit ▸ Behavior ▸ Project History.",
            );
        }
        if actual * 10 < limit * 9 {
            self.warned_at_limit = false;
        }
        dropped
    }

    /// Set the one-shot warning on the rising edge only (so it fires once per
    /// breach, not on every trimmed step), with a context-specific message.
    fn latch_warning(&mut self, msg: &str) {
        if !self.warned_at_limit {
            self.warned_at_limit = true;
            self.pending_warning = Some(msg.to_string());
        }
    }

    /// Take the pending limit warning, if any, for the GUI to display once.
    pub fn take_limit_warning(&mut self) -> Option<String> {
        self.pending_warning.take()
    }

    /// Apply a command and push it onto the undo stack.
    /// Schedules a debounced checkpoint — the snapshot is written after 30 s of
    /// inactivity via [`tick_checkpoint`], so burst operations (e.g. drag) do
    /// not produce a checkpoint on every frame.
    pub fn execute(&mut self, cmd: Command, doc: &mut Document) {
        // Normalize deletion commands into their self-contained `*Full` forms
        // while the target entity still exists, so the pushed undo entry (and
        // the persisted `.photon` history) is always invertible. See
        // [`Command::hydrate`].
        let cmd = cmd.hydrate(doc);
        let desc = cmd.description();

        // Gesture coalescing (#182): during an open pointer gesture, fold a
        // mergeable same-target command into the gesture's anchor undo entry
        // instead of pushing a new step, so one continuous drag records a single
        // undo step. Only merges once the gesture has pushed its anchor
        // (`coalesce_started`), and only when `Command::coalesce` accepts the
        // pair. Redo was already cleared by the anchor push, and `enforce_steps`
        // trims from the front, so mutating `undo_stack.last()` is safe.
        if self.coalescing && self.coalesce_started {
            if let Some(last) = self.undo_stack.last() {
                if let Some(merged) = Command::coalesce(last, &cmd) {
                    cmd.apply(doc);
                    reevaluate_constraints(doc);
                    *self.undo_stack.last_mut().unwrap() = merged;
                    self.gui_debounce.schedule(desc);
                    return;
                }
            }
        }

        cmd.apply(doc);
        reevaluate_constraints(doc);
        self.undo_stack.push(cmd);
        self.redo_stack.clear();
        // Enforce the step ceiling on the hot path (cheap). The optional size
        // cap is enforced separately via `enforce_size` (off the hot path,
        // since it must serialize the history to measure it).
        self.enforce_steps();
        // While a gesture is open, the entry just pushed becomes the anchor that
        // subsequent mergeable ticks fold into.
        if self.coalescing {
            self.coalesce_started = true;
        }
        self.gui_debounce.schedule(desc);
    }

    /// Apply a command as a **discrete** undo step, bypassing gesture coalescing
    /// (#182 fix round 1).
    ///
    /// Gesture coalescing (`coalescing` / `coalesce_started`) is armed purely by
    /// GUI pointer state, but the GUI and the MCP server share one
    /// `Arc<Mutex<CommandHistory>>`. If an external caller (the MCP tool server,
    /// the Lua REPL, or a script) went through the plain [`execute`] while a GUI
    /// pointer happened to be held down (dragging a swatch, panning, an in-progress
    /// marquee, …), its edit would silently fold into — or be absorbed by — the
    /// GUI gesture's anchor entry, collapsing multiple independent AI/script edits
    /// (or an AI edit + the user's own drag) into a single, non-granular undo step.
    ///
    /// Every non-GUI edit source must therefore call this instead of [`execute`].
    /// It snapshots the gesture flags, forces coalescing off for the push so the
    /// command always lands as its own step, then restores the gesture-open flag.
    /// `coalesce_started` is deliberately left `false` afterwards: the pushed
    /// command is now `undo_stack.last()`, so the GUI gesture must re-anchor on its
    /// next tick rather than fold a later pointer tick into this external command.
    pub fn execute_discrete(&mut self, cmd: Command, doc: &mut Document) {
        let was_coalescing = self.coalescing;
        self.coalescing = false;
        self.coalesce_started = false;
        self.execute(cmd, doc);
        // Restore only the gesture-open flag; leave `coalesce_started` false so an
        // in-progress GUI gesture starts a fresh anchor instead of merging into
        // this externally-sourced step.
        self.coalescing = was_coalescing;
    }

    /// Undo the last command.
    pub fn undo(&mut self, doc: &mut Document) -> bool {
        if let Some(cmd) = self.undo_stack.pop() {
            if let Some(inv) = cmd.inverse(doc) {
                inv.apply(doc);
                reevaluate_constraints(doc);
                self.redo_stack.push(cmd);
                return true;
            } else {
                // Can't invert — put it back
                self.undo_stack.push(cmd);
            }
        }
        false
    }

    /// Redo the last undone command.
    pub fn redo(&mut self, doc: &mut Document) -> bool {
        if let Some(cmd) = self.redo_stack.pop() {
            cmd.apply(doc);
            reevaluate_constraints(doc);
            self.undo_stack.push(cmd);
            true
        } else {
            false
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn undo_depth(&self) -> usize {
        self.undo_stack.len()
    }

    pub fn redo_depth(&self) -> usize {
        self.redo_stack.len()
    }

    /// Return the most recent `limit` undo stack entries as `(step_index, description)` pairs,
    /// newest first. `step_index` is 1-based (1 = most recent).
    pub fn history_entries(&self, limit: usize) -> Vec<(usize, String)> {
        self.undo_stack
            .iter()
            .rev()
            .take(limit)
            .enumerate()
            .map(|(i, cmd)| (i + 1, cmd.description()))
            .collect()
    }

    /// Revert a specific node to its state `steps` mutations ago (without
    /// touching any other nodes). Scans the undo stack backwards; counts any
    /// `UpdateNode` or `Batch` command that contained an update to `node_id`.
    ///
    /// Applies the reverted state as a new undoable `UpdateNode` command so the
    /// revert itself can be undone.
    ///
    /// Returns `Some(actual_steps)` — the number of node-specific history
    /// entries that were scanned — or `None` if the node isn't in the document
    /// or has no history.
    pub fn revert_node_steps(
        &mut self,
        node_id: NodeId,
        steps: usize,
        doc: &mut Document,
    ) -> Option<usize> {
        let current = doc.nodes.get(&node_id)?.clone();
        let steps = steps.max(1);

        // Collect UpdateNode commands that touched this node, newest first.
        let mut hits: Vec<SceneNode> = Vec::new(); // each hit's `old` (pre-mutation state)
        for cmd in self.undo_stack.iter().rev() {
            collect_node_olds(cmd, node_id, &mut hits);
            if hits.len() >= steps {
                break;
            }
        }

        if hits.is_empty() {
            return None;
        }

        // The furthest-back `old` is the last element collected.
        let target_state = hits.last().unwrap().clone();
        let actual = hits.len();

        // Apply as a new undoable command.
        self.execute(
            Command::UpdateNode {
                old: current,
                new: target_state,
            },
            doc,
        );

        Some(actual)
    }
}
