use super::*;

impl CommandHistory {
    // ── Gesture coalescing (#182) ────────────────────────────────────────────

    /// Open a coalescing gesture. While open, mergeable same-target commands
    /// streamed through [`execute`] fold into a single undo step instead of
    /// pushing one entry per pointer tick. Idempotent: the GUI calls this every
    /// frame the pointer is down, and only the first call (per gesture) arms it —
    /// re-calling while already open must NOT reset `coalesce_started`, or a mid-
    /// gesture edit would start a fresh anchor and stop folding.
    pub fn begin_coalescing(&mut self) {
        if !self.coalescing {
            self.coalescing = true;
            self.coalesce_started = false;
        }
    }

    /// Close the current coalescing gesture. Called on pointer release, after
    /// that frame's edit handlers have run, so a final same-frame edit still
    /// folds into the one step. Between gestures normal per-command pushes resume.
    pub fn end_coalescing(&mut self) {
        self.coalescing = false;
        self.coalesce_started = false;
    }

    /// Whether a coalescing gesture is currently open (test/introspection).
    pub fn is_coalescing(&self) -> bool {
        self.coalescing
    }

    /// Call once per frame from the render loop.  If a user action was recorded
    /// and 30 seconds have passed with no further actions, flushes the pending
    /// checkpoint.  Safe to call even when no action is pending.
    pub fn tick_checkpoint(&mut self, doc: &Document) {
        if let Some(desc) = self.gui_debounce.tick() {
            self.create_checkpoint(desc, doc);
        }
    }

    /// Called by the MCP server after each successful mutating tool call.
    /// Resets the 60-second debounce window, extending it on rapid sequential calls.
    pub fn schedule_mcp_checkpoint(&mut self, desc: impl Into<String>) {
        self.mcp_debounce.schedule(desc);
    }

    /// Return the pending MCP checkpoint description, if the debounce window
    /// has not flushed yet. This is also useful to inspect the pending state
    /// without waiting for the background checkpoint task.
    pub fn pending_mcp_checkpoint(&self) -> Option<String> {
        self.mcp_debounce.pending_desc.clone()
    }

    /// Called periodically by the MCP background task (every ~10 s).
    /// Flushes the pending checkpoint once 60 seconds have elapsed since the
    /// last MCP mutation — a true debounce so burst tool calls produce only
    /// one checkpoint.
    pub fn tick_mcp_checkpoint(&mut self, doc: &Document) {
        if let Some(desc) = self.mcp_debounce.tick() {
            self.create_checkpoint(desc, doc);
        }
    }
}
