//! E-cost/I-12: a search with zero physical Page rows must not hydrate the
//! graph-wide page-property index. Exemplar: query_plan/pages.rs.
use std::sync::{atomic::AtomicBool, Arc};
use tine_store::{cost_counters, Cancel, SearchRequest, Store};

#[test]
fn a_block_only_first_search_derives_no_page_facts() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    std::fs::write(
        dir.path().join("pages/Source.md"),
        "owner:: Mira\n- uniqueblockneedle\n",
    )
    .unwrap();
    let (store, _, _) = Store::open(dir.path(), Default::default()).unwrap();
    let graph = store.whole_graph().unwrap();
    cost_counters::reset();
    let result = graph
        .search(
            &SearchRequest {
                text: "uniqueblockneedle".into(),
                within: None,
                page_limit: 10,
                block_limit: 10,
                explain: false,
                page_match_scope: None,
                page_view: None,
                block_view: None,
            },
            &Cancel(Arc::new(AtomicBool::new(false))),
        )
        .unwrap();
    assert_eq!(result.hits.len(), 1);
    assert!(matches!(
        result.hits[0],
        tine_core::query_plan::QueryHit::Block { .. }
    ));
    assert_eq!(
        cost_counters::snapshot().query_facts_derived,
        0,
        "E-cost/I-12: zero Page rows must skip the query index; exemplar query_plan/pages.rs"
    );
}
