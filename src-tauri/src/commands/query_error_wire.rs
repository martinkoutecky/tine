//! Record reference failures in the desktop's opt-in private debug log.

pub(crate) use tine_graph_features::query_error_wire::query_error;
use tine_store::QueryError;

pub(crate) fn reference_error(error: QueryError) -> String {
    tine_graph_features::query_error_wire::reference_error(error, |detail| {
        crate::debug::diag_private("reference-load-failed", detail);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tine_store::Budget;

    #[test]
    fn reference_budget_crosses_wire_as_fixed_family() {
        let bounded = QueryError::ResultTooLarge {
            what: Budget::MatchingBlocks,
            count: 20_001,
            limit: 20_000,
            bytes: None,
            byte_limit: 32 * 1024 * 1024,
        };
        assert_eq!(reference_error(bounded), "result-too-large");
        assert_eq!(reference_error(QueryError::Cancelled), "cancelled");
    }
}
