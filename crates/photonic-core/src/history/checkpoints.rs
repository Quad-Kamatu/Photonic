use super::*;

impl CommandHistory {
    // ── Persistence (save/restore the full history with the document) ─────────

    /// Capture the persistent history (undo/redo/checkpoints/branches) for
    /// serialization into a `.photon` file. Clones; does not mutate self.
    pub fn snapshot_state(&self) -> HistorySnapshot {
        HistorySnapshot {
            undo_stack: self.undo_stack.clone(),
            redo_stack: self.redo_stack.clone(),
            checkpoints: self.checkpoints.clone(),
            branches: self.branches.clone(),
        }
    }

    /// Replace the persistent history with a restored snapshot (on file open),
    /// then re-enforce the current limits. Configured limits, debounce timers,
    /// and the revision counter are preserved. Bumps `revision` so revision-
    /// keyed caches refresh.
    pub fn restore_state(&mut self, s: HistorySnapshot) {
        self.undo_stack = s.undo_stack;
        self.redo_stack = s.redo_stack;
        self.checkpoints = s.checkpoints;
        self.branches = s.branches;
        self.warned_at_limit = false;
        self.pending_warning = None;
        self.revision = self.revision.wrapping_add(1);
        self.enforce_steps();
        self.enforce_size();
    }

    /// Clear all persistent history (undo/redo/checkpoints/branches) while
    /// keeping the configured limits. Used when opening a document that carries
    /// no embedded history, or on New, so a previous project's history can't
    /// bleed into the freshly loaded one. Bumps `revision`.
    pub fn reset(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.checkpoints.clear();
        self.branches.clear();
        self.warned_at_limit = false;
        self.pending_warning = None;
        self.revision = self.revision.wrapping_add(1);
    }

    // ── Checkpoints (git-style commits) ──────────────────────────────────

    /// Save a named snapshot of the document. Returns the new checkpoint ID.
    /// Keeps at most 50 checkpoints; oldest are dropped when the limit is reached.
    pub fn create_checkpoint(&mut self, name: String, doc: &Document) -> Uuid {
        let id = Uuid::new_v4();
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.checkpoints.push(Checkpoint {
            id,
            name,
            created_at,
            snapshot: doc.clone(),
        });
        const MAX_CHECKPOINTS: usize = 50;
        if self.checkpoints.len() > MAX_CHECKPOINTS {
            self.checkpoints.remove(0);
        }
        id
    }

    /// Return summary info for all checkpoints, oldest first.
    pub fn list_checkpoints(&self) -> Vec<CheckpointInfo> {
        self.checkpoints
            .iter()
            .map(|c| CheckpointInfo {
                id: c.id,
                name: c.name.clone(),
                created_at: c.created_at,
            })
            .collect()
    }

    /// Restore the document to a saved checkpoint. Clears undo/redo stacks.
    /// Returns the snapshot to replace the live document, or `None` if not found.
    pub fn restore_checkpoint(&mut self, id: Uuid) -> Option<Document> {
        let snapshot = self
            .checkpoints
            .iter()
            .find(|c| c.id == id)?
            .snapshot
            .clone();
        self.undo_stack.clear();
        self.redo_stack.clear();
        Some(snapshot)
    }

    /// Return a clone of the document snapshot at `id` without touching
    /// undo/redo stacks. Use this for read-only operations like diffing.
    pub fn get_checkpoint_snapshot(&self, id: Uuid) -> Option<Document> {
        self.checkpoints
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.snapshot.clone())
    }

}
