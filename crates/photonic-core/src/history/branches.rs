use super::*;

impl CommandHistory {
    // ── Named branches ────────────────────────────────────────────────────

    /// Save the current document state as a named branch.
    /// If a branch with the same name already exists it is overwritten.
    pub fn branch_create(&mut self, name: String, doc: &Document) {
        self.branches.insert(name, doc.clone());
    }

    /// Return a sorted list of all branch names.
    pub fn branch_list(&self) -> Vec<String> {
        let mut names: Vec<String> = self.branches.keys().cloned().collect();
        names.sort();
        names
    }

    /// Restore the document to a named branch snapshot.
    /// Clears undo/redo stacks. Returns `None` if the branch doesn't exist.
    pub fn branch_switch(&mut self, name: &str) -> Option<Document> {
        let snapshot = self.branches.get(name)?.clone();
        // The branch snapshot becomes the new baseline — start history fresh.
        self.init_empty_tree();
        Some(snapshot)
    }

    /// Delete a named branch. Returns `true` if it existed.
    pub fn branch_delete(&mut self, name: &str) -> bool {
        self.branches.remove(name).is_some()
    }

}
