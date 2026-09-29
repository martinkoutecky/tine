//! Master b81cc6e90 ("scoped verified windows"): a current-page block search
//! applies its page scope BEFORE the result window is counted. On master the
//! SQL interactive cursor consumed newer out-of-scope rows and could leave the
//! scoped page's own match outside the window. og has no SQL cursor (the scope
//! narrows the in-memory page set first, `QueryPlan::friendly_for_page`), so
//! this pins the outcome, through the public `WholeGraph::search`, at a window
//! far smaller than the out-of-scope match count.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use tine_core::query_plan::QueryHit;
use tine_store::{Cancel, OpenOptions, PageId, SearchRequest, Store};

#[test]
fn a_page_scoped_block_search_is_not_consumed_by_out_of_scope_matches() {
    let dir = std::env::temp_dir().join(format!("tine-scoped-window-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("pages")).unwrap();
    std::fs::create_dir_all(dir.join("journals")).unwrap();
    // Sixty other pages whose blocks match at least as well as the target's.
    for n in 0..60 {
        std::fs::write(dir.join(format!("pages/Other {n:02}.md")), "- needle\n").unwrap();
    }
    std::fs::write(
        dir.join("pages/Target.md"),
        "- a long block that mentions the needle only in its middle, after many words\n",
    )
    .unwrap();
    let (store, _, _) = Store::open(&dir, OpenOptions::default()).expect("open");
    let graph = store.whole_graph().expect("load");
    let search = |within: Option<&str>, block_limit: usize| {
        graph
            .search(
                &SearchRequest {
                    text: "needle".into(),
                    within: within.map(PageId::from),
                    page_limit: 0,
                    block_limit,
                    explain: false,
                    page_match_scope: None,
                    page_view: None,
                    block_view: None,
                },
                &Cancel(Arc::new(AtomicBool::new(false))),
            )
            .expect("search")
    };

    let scoped = search(Some("pages/Target.md"), 2);
    let paths: Vec<&str> = scoped
        .hits
        .iter()
        .map(|hit| match hit {
            QueryHit::Block { path, .. } => path.as_str(),
            other => panic!("a scoped search is block-only: {other:?}"),
        })
        .collect();
    assert_eq!(
        paths,
        vec!["pages/Target.md"],
        "the scoped page's own match must survive a window smaller than the out-of-scope matches"
    );
    // The unscoped control: the window really is smaller than the match count.
    assert_eq!(search(None, 2).hits.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}
