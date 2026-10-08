//! Wire-compatible query error text, shared by graph-reading commands.

use tine_store::{Budget, QueryError};

pub(super) fn query_error(error: QueryError) -> String {
    match error {
        QueryError::Cancelled => "cancelled".into(),
        QueryError::Parse(reason) => reason,
        QueryError::InvalidTarget(_) => "invalid page path".into(),
        QueryError::RequestTooLarge { what: Budget::BacklinkFilterRoots, count, limit } =>
            format!("too many backlink filter roots: {count} (limit: {limit})"),
        QueryError::RequestTooLarge { what, count, limit } =>
            format!("request-too-large: {count} {} (limit: {limit})", budget_text(what)),
        QueryError::ExportRequestTooLarge { macros, bytes, macro_limit, byte_limit, processing_cap } =>
            format!("query-export-request-too-large: {macros} macros / {bytes} bytes (request limits: {macro_limit} macros / {byte_limit} bytes; processing cap: {processing_cap} macros)"),
        QueryError::ResultTooLarge { what: Budget::PropertyFacets, .. } =>
            "result-too-large: property facets exceed the construction budget".into(),
        QueryError::ResultTooLarge { what: Budget::ResolvedBlockRows, count, .. } =>
            format!("result-too-large: {count} resolved block-reference rows exceed the construction budget"),
        QueryError::ResultTooLarge { what: Budget::RequestedBlockRefs, count, limit, .. } =>
            format!("result-too-large: {count} requested block references (limit: {limit})"),
        QueryError::ResultTooLarge { what: Budget::ExportBytes, count, limit, .. } =>
            format!("query-export-result-too-large: ~{count} bytes (limit: {limit} bytes)"),
        QueryError::ResultTooLarge { what: Budget::BridgeMatchingBlocks, count, limit, bytes: Some(bytes), byte_limit } =>
            format!("result-too-large: {count} matching blocks (~{bytes} bytes); narrow the query or add (sample N) (limits: {limit} blocks / {byte_limit} bytes)"),
        QueryError::ResultTooLarge { what: Budget::MatchingBlocks, count, limit, byte_limit, .. } =>
            format!("result-too-large: {count} matching blocks; narrow the query or add (sample N) (construction limits: {limit} blocks / {byte_limit} bytes)"),
        QueryError::ResultTooLarge { what: Budget::AdvancedQueryMatches, count, .. } =>
            format!("result-too-large: {count} advanced-query matches; narrow the query"),
        QueryError::ResultTooLarge { what: Budget::SearchHits, count, limit, bytes: Some(bytes), byte_limit } =>
            format!("result-too-large: {count} search hits (~{bytes} bytes); narrow the search (limits: {limit} hits / {byte_limit} bytes)"),
        QueryError::ResultTooLarge { what, count, limit, .. } =>
            format!("result-too-large: {count} {} (limit: {limit})", budget_text(what)),
    }
}

/// Report a reference-read failure: a fixed budget family crosses the wire,
/// while full query detail is written only to the opt-in private debug log.
/// Cost is O(error detail length); callers display fixed text for this family.
pub(super) fn reference_error(error: QueryError) -> String {
    let bounded = matches!(&error, QueryError::ResultTooLarge { .. });
    let detail = query_error(error);
    crate::debug::diag_private("reference-load-failed", &detail);
    if bounded {
        "result-too-large".into()
    } else {
        detail
    }
}

fn budget_text(what: Budget) -> &'static str {
    match what {
        Budget::BacklinkFilterRoots => "backlink filter roots",
        Budget::MatchingBlocks => "matching blocks",
        Budget::BridgeMatchingBlocks => "bridge matching blocks",
        Budget::RequestedBlockRefs => "requested block references",
        Budget::ResolvedBlockRows => "resolved block-reference rows",
        Budget::ExportBytes => "query export bytes",
        Budget::PropertyFacets => "property facets",
        Budget::AdvancedQueryMatches => "advanced-query matches",
        Budget::SearchHits => "search hits",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
