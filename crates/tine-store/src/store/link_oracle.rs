//! Test oracles for TQL `links`, `linked_by` and `near`
//! (docs/contracts/tql-links-near.md): the link table's size. Gated with the
//! module (`test-faults`).
use super::*;

impl WholeGraph {
    /// Test probe: this view's TQL link table (`links`, `linked_by`, `near`)
    /// as (nodes, links, estimated heap bytes), building it if needed.
    pub fn link_table_stats(&self) -> (usize, usize, usize) {
        let table = self.graph.link_graph(&|| false).expect("never cancelled");
        (
            table.node_count(),
            table.edge_count(),
            table.estimated_bytes(),
        )
    }
}
