use super::*;

impl CommandHistory {
    pub fn new(max_depth: usize) -> Self {
        let mut nodes = std::collections::HashMap::new();
        nodes.insert(
            0,
            HistNode {
                id: 0,
                parent: None,
                command: None,
                children: vec![],
                primary_child: None,
            },
        );
        Self {
            nodes,
            root: 0,
            current: 0,
            next_id: 1,
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

    /// Ceiling on the total number of retained tree nodes. The step limit bounds
    /// the *current path* depth; this bounds the whole tree so many sibling
    /// branches can't grow memory without bound. Generous relative to the step
    /// ceiling so real branching is kept, but finite.
    fn node_cap(&self) -> usize {
        self.max_depth.saturating_mul(4).max(self.max_depth + 1)
    }

    /// Trim the edit tree back within limits. Cheap — no serialization. First
    /// re-roots while the current path is deeper than the step ceiling (dropping
    /// the oldest states and any branches hanging off them), then prunes the
    /// oldest off-path leaves until the total node count is within `node_cap`.
    /// Latches a warning on the first node actually dropped.
    pub(crate) fn enforce_steps(&mut self) {
        let mut dropped = false;
        while self.depth_of(self.current) > self.max_depth {
            if !self.reroot_once() {
                break;
            }
            dropped = true;
        }
        while self.nodes.len() > self.node_cap() + 1 {
            if !self.prune_oldest_offpath_leaf() {
                break;
            }
            dropped = true;
        }
        // Recovered comfortably under the ceiling → re-arm the warning latch.
        if self.depth_of(self.current) * 10 < self.max_depth * 9 {
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

        let mut dropped = false;
        // Re-measure each round: the tree is a graph, not a flat list, so an
        // incremental byte estimate isn't as clean. Node counts are bounded by
        // `node_cap`, so at most O(cap) rounds. Prefer dropping the oldest state
        // (re-root) while above the retained-steps floor, then prune off-path
        // branches, then as a last resort re-root down to a single step.
        while self.history_byte_size() > limit {
            let progressed = if self.depth_of(self.current) > Self::MIN_RETAINED_STEPS {
                self.reroot_once()
            } else if self.prune_oldest_offpath_leaf() {
                true
            } else if self.depth_of(self.current) > 1 {
                self.reroot_once()
            } else {
                false
            };
            if !progressed {
                // Only a single step plus un-trimmable checkpoints/branches
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

    /// Fraction of the configured size budget the serialized history currently
    /// occupies (`bytes / limit`), or `None` when no size cap is set. Lets the
    /// GUI warn *before* the cap starts trimming (#197). Serializes the history,
    /// so callers should throttle (the GUI reuses its ~1.5 s size-check cadence).
    pub fn size_pressure(&self) -> Option<f32> {
        let limit = self.size_limit_bytes?;
        if limit == 0 {
            return Some(f32::INFINITY);
        }
        Some(self.history_byte_size() as f32 / limit as f32)
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
        // mergeable same-target command into the gesture's anchor node's edge
        // command instead of adding a new node, so one continuous drag records a
        // single undo step. Only merges once the gesture has anchored
        // (`coalesce_started`) and when `Command::coalesce` accepts the pair.
        if self.coalescing && self.coalesce_started && self.current != self.root {
            let merged = self
                .nodes
                .get(&self.current)
                .and_then(|n| n.command.as_ref())
                .and_then(|last| Command::coalesce(last, &cmd));
            if let Some(merged) = merged {
                cmd.apply(doc);
                reevaluate_constraints(doc);
                if let Some(n) = self.nodes.get_mut(&self.current) {
                    n.command = Some(merged);
                }
                self.gui_debounce.schedule(desc);
                return;
            }
        }

        cmd.apply(doc);
        reevaluate_constraints(doc);
        // Add a new child under HEAD. Crucially we do NOT discard the old redo
        // path — if HEAD already had children (i.e. we're editing after an undo),
        // this new node becomes a *sibling branch* and the old future is kept.
        let id = self.next_id;
        self.next_id += 1;
        let parent = self.current;
        self.nodes.insert(
            id,
            HistNode {
                id,
                parent: Some(parent),
                command: Some(cmd),
                children: vec![],
                primary_child: None,
            },
        );
        if let Some(p) = self.nodes.get_mut(&parent) {
            p.children.push(id);
            p.primary_child = Some(id);
        }
        self.current = id;
        // Enforce the step ceiling on the hot path (cheap). The optional size
        // cap is enforced separately via `enforce_size` (off the hot path,
        // since it must serialize the history to measure it).
        self.enforce_steps();
        // While a gesture is open, the node just added becomes the anchor that
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

    /// Undo the current edit — move HEAD toward the root, applying the inverse of
    /// the edge command. The branch we came from is remembered as `primary_child`
    /// so redo returns to it.
    pub fn undo(&mut self, doc: &mut Document) -> bool {
        let cur = self.current;
        let (parent, cmd) = match self.nodes.get(&cur) {
            Some(n) => match (n.parent, n.command.clone()) {
                (Some(p), Some(c)) => (p, c),
                _ => return false,
            },
            None => return false,
        };
        if let Some(inv) = cmd.inverse(doc) {
            inv.apply(doc);
            reevaluate_constraints(doc);
            if let Some(pn) = self.nodes.get_mut(&parent) {
                pn.primary_child = Some(cur);
            }
            self.current = parent;
            true
        } else {
            // Can't invert — stay put.
            false
        }
    }

    /// Redo — move HEAD down its primary child (the most recently visited/created
    /// branch), applying that edge command.
    pub fn redo(&mut self, doc: &mut Document) -> bool {
        let cur = self.current;
        let child = match self.nodes.get(&cur) {
            Some(n) => n.primary_child.or_else(|| n.children.last().copied()),
            None => None,
        };
        if let Some(c) = child {
            if let Some(cmd) = self.nodes.get(&c).and_then(|n| n.command.clone()) {
                cmd.apply(doc);
                reevaluate_constraints(doc);
                self.current = c;
                return true;
            }
        }
        false
    }

    pub fn can_undo(&self) -> bool {
        self.nodes.get(&self.current).is_some_and(|n| n.parent.is_some())
    }

    pub fn can_redo(&self) -> bool {
        self.nodes
            .get(&self.current)
            .is_some_and(|n| !n.children.is_empty())
    }

    pub fn undo_depth(&self) -> usize {
        self.depth_of(self.current)
    }

    pub fn redo_depth(&self) -> usize {
        self.primary_chain(self.current, usize::MAX).len()
    }

    /// Return up to `limit` undo entries (the path from HEAD to the root) as
    /// `(step_index, description)` pairs, newest first. `step_index` is 1-based
    /// (1 = most recent).
    pub fn history_entries(&self, limit: usize) -> Vec<(usize, String)> {
        let mut out = Vec::new();
        let mut id = self.current;
        while out.len() < limit {
            let (cmd, parent) = match self.nodes.get(&id) {
                Some(n) => match (&n.command, n.parent) {
                    (Some(c), Some(p)) => (c.description(), p),
                    _ => break,
                },
                None => break,
            };
            out.push((out.len() + 1, cmd));
            id = parent;
        }
        out
    }

    /// Return up to `limit` redo entries (the primary future chain below HEAD) as
    /// `(step_index, description)` pairs, newest edit first. `step_index` is
    /// 1-based (1 = the newest future edit — the farthest down the chain).
    pub fn redo_entries(&self, limit: usize) -> Vec<(usize, String)> {
        let mut chain = self.primary_chain(self.current, usize::MAX);
        chain.reverse(); // newest (farthest) first
        chain
            .into_iter()
            .take(limit)
            .enumerate()
            .filter_map(|(i, id)| {
                self.nodes
                    .get(&id)
                    .and_then(|n| n.command.as_ref())
                    .map(|c| (i + 1, c.description()))
            })
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

        // Collect UpdateNode commands that touched this node along the current
        // undo path, newest first.
        let mut hits: Vec<SceneNode> = Vec::new(); // each hit's `old` (pre-mutation state)
        for id in self.ancestor_edges(self.current) {
            if let Some(cmd) = self.nodes.get(&id).and_then(|n| n.command.as_ref()) {
                collect_node_olds(cmd, node_id, &mut hits);
                if hits.len() >= steps {
                    break;
                }
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
