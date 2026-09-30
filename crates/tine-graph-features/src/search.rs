//! Search request coordination. A lane cancels its previous request, and the
//! graph snapshot resolves scope before evaluation. Completed/error requests
//! release their exact lane; workspace close cancels both its lanes. Cost
//! O(graph search) for work, O(1) for request lifecycle.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tine_core::model::{PageKind, RefGroup};
use tine_core::query::ir::FriendlyPageMatchScope;
use tine_core::query::ir::ViewSettings;
use tine_core::query_plan::{QueryExecution, QueryExplanation, QueryHasMore, QueryHit};
use tine_store::{
    Cancel, LoadError, PageId, QueryDialect, QueryError, QueryResult, Resolved, SearchRequest,
    Store,
};

/// Per-transport-lane cancellation flags, hidden behind search operations.
#[derive(Default)]
pub struct SearchLanes(Mutex<HashMap<String, Arc<AtomicBool>>>);

impl SearchLanes {
    fn begin<'a>(&'a self, lane: Option<&'a str>) -> SearchLease<'a> {
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
        SearchLease {
            lanes: self,
            lane,
            flag,
        }
    }

    /// Cancel and release both search lanes owned by a closed workspace.
    /// O(1) lookups; other workspaces and replacement requests are unaffected.
    /// An already-running search observes cancellation through its existing flag.
    pub fn close_workspace(&self, workspace: &str) {
        let mut lanes = self.0.lock().unwrap();
        for key in [
            format!("query-workspace:{workspace}"),
            format!("query-workspace:{workspace}:materialize"),
        ] {
            if let Some(flag) = lanes.remove(&key) {
                flag.store(true, Ordering::Release);
            }
        }
    }
}

// The request owns the map entry, including errors and unwinding. A superseded
// request must never remove its successor's cancellation flag (I-20/I-21).
struct SearchLease<'a> {
    lanes: &'a SearchLanes,
    lane: Option<&'a str>,
    flag: Arc<AtomicBool>,
}

impl std::ops::Deref for SearchLease<'_> {
    type Target = AtomicBool;
    fn deref(&self) -> &AtomicBool {
        &self.flag
    }
}

impl Drop for SearchLease<'_> {
    fn drop(&mut self) {
        if let Some(lane) = self.lane {
            let mut lanes = self.lanes.0.lock().unwrap();
            if lanes
                .get(lane)
                .is_some_and(|flag| Arc::ptr_eq(flag, &self.flag))
            {
                lanes.remove(lane);
            }
        }
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
/// Page membership defaults to names/aliases; content and both also inspect
/// blocks. Each section's sort precedes its own limit and sample. Cost
/// O(graph text + output), plus O(matches log matches) for authored sorts;
/// load/query errors are returned.
pub fn run_graph_search(
    store: &Store,
    lanes: &SearchLanes,
    source: String,
    page_limit: usize,
    block_limit: usize,
    lane: Option<&str>,
    explain: bool,
    scope: Option<Scope>,
    page_match_scope: Option<FriendlyPageMatchScope>,
    page_view: Option<ViewSettings>,
    block_view: Option<ViewSettings>,
) -> Result<QueryExecution, SearchError> {
    run_graph_search_after_scope(
        store,
        lanes,
        source,
        page_limit,
        block_limit,
        lane,
        explain,
        scope,
        page_match_scope,
        page_view,
        block_view,
        #[cfg(test)]
        || {},
    )
}

fn run_graph_search_after_scope(
    store: &Store,
    lanes: &SearchLanes,
    source: String,
    page_limit: usize,
    block_limit: usize,
    lane: Option<&str>,
    explain: bool,
    scope: Option<Scope>,
    page_match_scope: Option<FriendlyPageMatchScope>,
    page_view: Option<ViewSettings>,
    block_view: Option<ViewSettings>,
    #[cfg(test)] after_scope: impl FnOnce(),
) -> Result<QueryExecution, SearchError> {
    let view = store.whole_graph().map_err(SearchError::Load)?;
    let lease = lanes.begin(lane);
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
        page_match_scope,
        page_view,
        block_view,
    };
    #[cfg(test)]
    after_scope();
    let execution = match view.search(&request, &Cancel(Arc::clone(&lease.flag))) {
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
    let lease = lanes.begin(lane);
    view.find_blocks(query, limit, &Cancel(Arc::clone(&lease.flag)))
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
    tine_core::query::admit_source(query).map_err(|reason| {
        QueryError::Parse(match reason {
            tine_core::query::SourceRefusal::TooLarge => format!(
                "query-too-large: query source is {} bytes (limit: {} bytes)",
                query.len(),
                tine_core::query::QUERY_SOURCE_MAX_BYTES
            ),
            tine_core::query::SourceRefusal::TooDeep => {
                "query-nesting-too-deep: simplify nested boolean clauses".into()
            }
        })
    })
}

/// Run a bounded simple query. Cost O(query candidates + output).
pub fn run_query(store: &Store, query: &str) -> Result<Arc<Vec<RefGroup>>, SearchError> {
    validate_source(query).map_err(SearchError::Query)?;
    let view = store.whole_graph().map_err(SearchError::Load)?;
    match view
        .query(query, QueryDialect::Simple)
        .map_err(SearchError::Query)?
    {
        QueryResult::Simple(groups) => Ok(groups),
        QueryResult::Advanced(_) => unreachable!(),
    }
}

/// Run an advanced query. Cost O(query candidates + output).
pub fn run_advanced_query(
    store: &Store,
    query: &str,
) -> Result<tine_core::query::AdvancedResult, SearchError> {
    run_advanced_query_after_scope(
        store,
        query,
        #[cfg(test)]
        || {},
    )
}

fn run_advanced_query_after_scope(
    store: &Store,
    query: &str,
    #[cfg(test)] after_scope: impl FnOnce(),
) -> Result<tine_core::query::AdvancedResult, SearchError> {
    validate_source(query).map_err(SearchError::Query)?;
    let view = store.whole_graph().map_err(SearchError::Load)?;
    #[cfg(test)]
    after_scope();
    match view
        .query(query, QueryDialect::Advanced)
        .map_err(SearchError::Query)?
    {
        QueryResult::Advanced(result) => Ok(result),
        QueryResult::Simple(_) => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tine_store::{SaveBase, SaveOutcome};

    #[test]
    fn search_and_advanced_scope_keep_one_generation_during_write() {
        let root = std::env::temp_dir().join(format!(
            "tine-d3-search-scope-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(root.join("pages/Scope.md"), "- TODO oldtoken\n").unwrap();
        let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
        let lanes = SearchLanes::default();
        let scope = Scope {
            name: "Scope".into(),
            kind: PageKind::Page,
            path: None,
        };
        let scoped = run_graph_search_after_scope(
            &store,
            &lanes,
            "oldtoken".into(),
            10,
            10,
            None,
            false,
            Some(scope),
            None,
            None,
            None,
            || {
                let writer = Arc::clone(&store);
                std::thread::spawn(move || {
                    let id = PageId::from("pages/Scope.md");
                    let read = writer.page(&id).unwrap();
                    let mut doc = read.doc;
                    doc.blocks[0].raw = "DONE newtoken".into();
                    assert!(matches!(
                        writer.save(
                            tine_store::EditKind::ReplacePage,
                            &id,
                            SaveBase::Existing(read.rev),
                            &doc
                        ),
                        SaveOutcome::Saved(_)
                    ));
                })
                .join()
                .unwrap();
            },
        )
        .unwrap_or_else(|_| panic!("scoped search failed"));
        assert!(
            !scoped.hits.is_empty(),
            "search must read the resolved generation"
        );

        let id = PageId::from("pages/Scope.md");
        let read = store.page(&id).unwrap();
        let mut doc = read.doc;
        doc.blocks[0].raw = "TODO oldtoken".into();
        assert!(matches!(
            store.save(
                tine_store::EditKind::ReplacePage,
                &id,
                SaveBase::Existing(read.rev),
                &doc
            ),
            SaveOutcome::Saved(_)
        ));
        let advanced = run_advanced_query_after_scope(
            &store,
            r#"[:find (pull ?b [*]) :where (task ?b #{"TODO"})]"#,
            || {
                let writer = Arc::clone(&store);
                std::thread::spawn(move || {
                    let id = PageId::from("pages/Scope.md");
                    let read = writer.page(&id).unwrap();
                    let mut doc = read.doc;
                    doc.blocks[0].raw = "DONE newtoken".into();
                    assert!(matches!(
                        writer.save(
                            tine_store::EditKind::ReplacePage,
                            &id,
                            SaveBase::Existing(read.rev),
                            &doc
                        ),
                        SaveOutcome::Saved(_)
                    ));
                })
                .join()
                .unwrap();
            },
        )
        .unwrap_or_else(|_| panic!("advanced query failed"));
        assert!(
            !advanced.groups.is_empty(),
            "advanced query must read the resolved generation"
        );
        store.close();
        std::fs::remove_dir_all(root).unwrap();
    }

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

        let nested = format!("{}(task TODO){}", "(and ".repeat(128), ")".repeat(128));
        assert!(
            reason(validate_source(&nested).unwrap_err()).starts_with("query-nesting-too-deep:")
        );
    }

    #[test]
    fn completed_workspace_searches_release_their_lanes() {
        let root = std::env::temp_dir().join(format!("tine-life-search-{}", std::process::id()));
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(root.join("pages/P.md"), "- needle\n").unwrap();
        let store = Store::open(&root, Default::default()).unwrap().0;
        let lanes = SearchLanes::default();
        for i in 0..50 {
            let lane = format!("query-workspace:{i}");
            let result = run_graph_search(
                &store,
                &lanes,
                "needle".into(),
                10,
                10,
                Some(&lane),
                false,
                None,
                None,
                None,
                None,
            );
            assert!(result.is_ok());
            assert_eq!(
                lanes.0.lock().unwrap().len(),
                0,
                "I-21: completed search lanes are not graph-lifetime records"
            );
        }
        store.close();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn workspace_close_cancels_its_requests_and_late_drop_keeps_a_successor() {
        let lanes = SearchLanes::default();
        let first = lanes.begin(Some("query-workspace:a"));
        let validating = lanes.begin(Some("query-workspace:a:materialize"));
        let other = lanes.begin(Some("query-workspace:b"));
        lanes.close_workspace("a");
        assert!(first.load(Ordering::Acquire));
        assert!(validating.load(Ordering::Acquire));
        assert!(!other.load(Ordering::Acquire));
        assert_eq!(lanes.0.lock().unwrap().len(), 1);
        let next = lanes.begin(Some("query-workspace:a"));
        drop(first);
        drop(validating);
        assert_eq!(lanes.0.lock().unwrap().len(), 2);
        assert!(!next.load(Ordering::Acquire));
        drop(next);
        drop(other);
        assert!(lanes.0.lock().unwrap().is_empty());
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
