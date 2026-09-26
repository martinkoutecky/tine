//! Search request coordination. A lane cancels its previous request, and the
//! graph snapshot resolves scope before evaluation. Cost O(graph search).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tine_core::model::{PageKind, RefGroup};
use tine_core::query_plan::{QueryExecution, QueryExplanation, QueryHasMore, QueryHit};
use tine_store::{
    Cancel, LoadError, PageId, QueryDialect, QueryError, QueryResult, Resolved, SearchRequest,
    Store,
};

/// Per-transport-lane cancellation flags, hidden behind search operations.
#[derive(Default)]
pub struct SearchLanes(Mutex<HashMap<String, Arc<AtomicBool>>>);

impl SearchLanes {
    fn begin(&self, lane: Option<&str>) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        if let Some(lane) = lane {
            if let Some(previous) = self
                .0
                .lock()
                .unwrap()
                .insert(lane.to_owned(), Arc::clone(&flag))
            {
                previous.store(true, Ordering::Release);
            }
        }
        flag
    }
}

/// A search can fail while waiting for a snapshot or evaluating a query.
pub enum SearchError {
    Load(LoadError),
    Query(QueryError),
}

/// Optional page scope; an exact path takes priority over name resolution.
pub struct Scope {
    pub name: String,
    pub kind: PageKind,
    pub path: Option<String>,
}

/// Run a graph search under one snapshot, replacing an earlier request on its
/// lane. Cancellation returns an empty execution marked `cancelled`.
/// Cost O(graph search + output).
pub fn run_graph_search(
    store: &Store,
    lanes: &SearchLanes,
    source: String,
    page_limit: usize,
    block_limit: usize,
    lane: Option<&str>,
    explain: bool,
    scope: Option<Scope>,
) -> Result<QueryExecution, SearchError> {
    let view = store.whole_graph().map_err(SearchError::Load)?;
    let flag = lanes.begin(lane);
    let within = scope.map(|scope| match scope.path {
        Some(path) => PageId::from(path),
        None => match view.resolve(&scope.name, scope.kind == PageKind::Journal) {
            Resolved::Existing { id, .. } | Resolved::Absent { id } => id,
            Resolved::Alias { owners } => owners.into_iter().next().expect("alias has an owner"),
        },
    });
    let request = SearchRequest {
        text: source,
        within,
        page_limit,
        block_limit,
        explain,
    };
    let execution = match view.search(&request, &Cancel(flag)) {
        Ok(execution) => execution,
        Err(QueryError::Cancelled) => QueryExecution {
            hits: Vec::new(),
            diagnostics: Vec::new(),
            explanation: QueryExplanation {
                branches: Vec::new(),
            },
            has_more: QueryHasMore::default(),
            cancelled: true,
        },
        Err(error) => return Err(SearchError::Query(error)),
    };
    enforce_execution_budget(&execution).map_err(SearchError::Query)?;
    Ok(execution)
}

/// Literal block search in one lane. Cost O(all blocks + output).
pub fn find_blocks(
    store: &Store,
    lanes: &SearchLanes,
    query: &str,
    limit: usize,
    lane: Option<&str>,
) -> Result<Vec<RefGroup>, SearchError> {
    let view = store.whole_graph().map_err(SearchError::Load)?;
    let flag = lanes.begin(lane);
    view.find_blocks(query, limit, &Cancel(flag))
        .map_err(SearchError::Query)
}

fn enforce_execution_budget(execution: &QueryExecution) -> Result<(), QueryError> {
    let bytes = execution.hits.iter().fold(0usize, |total, hit| {
        total.saturating_add(match hit {
            QueryHit::Page {
                page,
                display_text,
                evidence,
                matched_alias,
                ..
            } => {
                page.name.len()
                    + page.rel_path_str().len()
                    + display_text.len()
                    + matched_alias.as_ref().map_or(0, String::len)
                    + evidence.len() * 128
                    + 256
            }
            QueryHit::Block {
                page,
                block,
                display_text,
                evidence,
                ..
            } => {
                page.len()
                    + tine_core::model::block_dto_estimated_bytes(block)
                    + display_text.len()
                    + evidence.len() * 128
                    + 256
            }
        })
    });
    if let Some(error) = QueryError::bridge_search_hits(execution.hits.len(), bytes) {
        return Err(error);
    }
    Ok(())
}

/// Refuse oversized or over-nested query source before parsing or cache lookup.
/// Cost O(source bytes).
pub fn validate_source(query: &str) -> Result<(), QueryError> {
    if !tine_core::query::query_source_within_limit(query) {
        return Err(QueryError::Parse(format!(
            "query-too-large: query source is {} bytes (limit: {} bytes)",
            query.len(),
            tine_core::query::QUERY_SOURCE_MAX_BYTES
        )));
    }
    if !tine_core::query::query_nesting_within_limit(query) {
        return Err(QueryError::Parse(
            "query-nesting-too-deep: simplify nested boolean clauses".into(),
        ));
    }
    Ok(())
}

/// Run a bounded simple query. Cost O(query candidates + output).
pub fn run_query(store: &Store, query: &str) -> Result<Arc<Vec<RefGroup>>, SearchError> {
    validate_source(query).map_err(SearchError::Query)?;
    let view = store.whole_graph().map_err(SearchError::Load)?;
    match view
        .query(query, QueryDialect::Simple, None)
        .map_err(SearchError::Query)?
    {
        QueryResult::Simple(groups) => Ok(groups),
        QueryResult::Advanced(_) => unreachable!(),
    }
}

/// Resolve the optional current page, then run an advanced query. Cost O(query candidates + output).
pub fn run_advanced_query(
    store: &Store,
    query: &str,
    current_page: Option<&str>,
) -> Result<tine_core::query::AdvancedResult, SearchError> {
    validate_source(query).map_err(SearchError::Query)?;
    let view = store.whole_graph().map_err(SearchError::Load)?;
    let current_id = current_page.map(|name| match view.resolve(name, false) {
        Resolved::Existing { id, .. } | Resolved::Absent { id } => id,
        Resolved::Alias { owners } => owners.into_iter().next().expect("alias has an owner"),
    });
    match view
        .query(query, QueryDialect::Advanced, current_id.as_ref())
        .map_err(SearchError::Query)?
    {
        QueryResult::Advanced(result) => Ok(result),
        QueryResult::Simple(_) => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_oversized_query_source_before_cache_or_parser() {
        fn reason(error: QueryError) -> String {
            match error {
                QueryError::Parse(reason) => reason,
                _ => panic!("unexpected query error"),
            }
        }
        let source = "x".repeat(tine_core::query::QUERY_SOURCE_MAX_BYTES + 1);
        assert!(reason(validate_source(&source).unwrap_err()).starts_with("query-too-large:"));

        let nested = format!("{}(task TODO){}", "(and ".repeat(65), ")".repeat(65));
        assert!(
            reason(validate_source(&nested).unwrap_err()).starts_with("query-nesting-too-deep:")
        );
    }

    #[test]
    fn later_request_cancels_only_its_own_lane() {
        let lanes = SearchLanes::default();
        let first = lanes.begin(Some("editor"));
        let other = lanes.begin(Some("sidebar"));
        let second = lanes.begin(Some("editor"));
        assert!(first.load(Ordering::Acquire));
        assert!(!other.load(Ordering::Acquire));
        assert!(!second.load(Ordering::Acquire));
    }
}
