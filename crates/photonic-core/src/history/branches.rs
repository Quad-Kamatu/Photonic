use super::*;

impl CommandHistory {
    // ── Named states (labeled commits) ─────────────────────────────────────
    //
    // A "named branch" is just a **label on a node** in the edit tree. Creating
    // and switching are non-destructive: the whole tree is preserved and a switch
    // is an ordinary (reversible) jump to that commit — unlike the old snapshot
    // branches, which replaced the document and wiped undo/redo.

    /// Set or clear a node's label. Names are unique — assigning a name first
    /// strips it from any other node that currently holds it.
    pub fn set_node_label(&mut self, id: u64, label: Option<String>) {
        let name = label.as_deref().map(str::trim).filter(|s| !s.is_empty());
        match name {
            Some(name) => {
                for n in self.nodes.values_mut() {
                    if n.label.as_deref() == Some(name) {
                        n.label = None;
                    }
                }
                if let Some(n) = self.nodes.get_mut(&id) {
                    n.label = Some(name.to_string());
                }
            }
            None => {
                if let Some(n) = self.nodes.get_mut(&id) {
                    n.label = None;
                }
            }
        }
    }

    /// The label on a node, if any.
    pub fn node_label(&self, id: u64) -> Option<&str> {
        self.nodes.get(&id).and_then(|n| n.label.as_deref())
    }

    /// The node carrying a given label, if any.
    pub fn node_by_label(&self, name: &str) -> Option<u64> {
        self.nodes
            .values()
            .find(|n| n.label.as_deref() == Some(name))
            .map(|n| n.id)
    }

    /// Label the current HEAD state (create/update a named branch there).
    pub fn branch_create(&mut self, name: String) {
        let id = self.current;
        self.set_node_label(id, Some(name));
    }

    /// All named states, sorted by name.
    pub fn branch_list(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .nodes
            .values()
            .filter_map(|n| n.label.clone())
            .collect();
        names.sort();
        names
    }

    /// Jump the document to the state named `name`. Non-destructive — the edit
    /// tree is preserved and the jump is reversible. Returns false if no such
    /// name exists (or the jump failed).
    pub fn branch_switch(&mut self, name: &str, doc: &mut Document) -> bool {
        match self.node_by_label(name) {
            Some(id) => self.jump_to_node(id, doc),
            None => false,
        }
    }

    /// Remove a named state's label. Returns true if the name existed.
    pub fn branch_delete(&mut self, name: &str) -> bool {
        match self.node_by_label(name) {
            Some(id) => {
                self.set_node_label(id, None);
                true
            }
            None => false,
        }
    }
}
