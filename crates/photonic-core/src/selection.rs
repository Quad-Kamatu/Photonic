use crate::node::NodeId;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// The current selection state of the document.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Selection {
    pub node_ids: HashSet<NodeId>,
}

impl Selection {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn single(id: NodeId) -> Self {
        let mut s = Self::new();
        s.node_ids.insert(id);
        s
    }

    pub fn from_ids(ids: impl IntoIterator<Item = NodeId>) -> Self {
        Self {
            node_ids: ids.into_iter().collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.node_ids.is_empty()
    }

    pub fn contains(&self, id: &NodeId) -> bool {
        self.node_ids.contains(id)
    }

    pub fn add(&mut self, id: NodeId) {
        self.node_ids.insert(id);
    }

    pub fn remove(&mut self, id: &NodeId) {
        self.node_ids.remove(id);
    }

    pub fn clear(&mut self) {
        self.node_ids.clear();
    }

    pub fn toggle(&mut self, id: NodeId) {
        if self.node_ids.contains(&id) {
            self.node_ids.remove(&id);
        } else {
            self.node_ids.insert(id);
        }
    }

    pub fn count(&self) -> usize {
        self.node_ids.len()
    }

    pub fn ids(&self) -> impl Iterator<Item = &NodeId> {
        self.node_ids.iter()
    }
}
