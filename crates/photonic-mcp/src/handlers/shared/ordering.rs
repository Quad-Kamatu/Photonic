/// Compute a sortable z-order key `(layer_order_index, node_index_in_layer)`.
/// Higher = frontmost.
pub(crate) fn node_z_key(
    doc: &photonic_core::document::Document,
    node_id: &uuid::Uuid,
) -> (usize, usize) {
    if let Some(node) = doc.nodes.get(node_id) {
        let layer_pos = doc
            .layer_order
            .iter()
            .position(|id| *id == node.layer_id)
            .unwrap_or(0);
        let node_pos = doc
            .layers
            .get(&node.layer_id)
            .and_then(|l| l.node_ids.iter().position(|id| id == node_id))
            .unwrap_or(0);
        (layer_pos, node_pos)
    } else {
        (0, 0)
    }
}
