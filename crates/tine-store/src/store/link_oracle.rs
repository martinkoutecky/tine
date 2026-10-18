//! Test oracles for TQL `links`, `linked_by` and `near`
//! (docs/contracts/tql-links-near.md): the link table's size and the
//! cancellable query entry. Gated with the module (`test-faults`).
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

    /// Test oracle: [`Self::query_ir`], stopping cooperatively once `cancel`
    /// is set, the same flag interactive search takes. `Ok(None)`: cancelled,
    /// no partial answer, nothing memoized. The link work behind `links`,
    /// `linked_by` and `near` (table build, element and seed conditions, the
    /// searches) checks it at least once per page and once per 1024
    /// references or nodes. Not production surface yet: SURFACE.txt rule 3
    /// wants a parameter on `query_ir`, not a sibling; that choice is the
    /// integrator's (RECEIPT-tql-near.md, review r1).
    pub fn query_ir_cancellable(
        &self,
        request: IrRequest<'_>,
        cancel: &Cancel,
    ) -> Result<Option<IrAnswer>, tine_core::query::statistics::StatisticsResourceLimit> {
        crate::query::exec::query_ir_cancellable(&self.graph, request, &|| {
            cancel.0.load(Ordering::Acquire)
        })
    }
}
