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
    let _integration_guard = crate::integration_lock();
    let dir = crate::test_support::fresh_dir("tine-scoped-window-");
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
                    transliteration: false,
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

/// GH #899: the launcher may paint pages before the block scan finishes. The
/// two provider windows must keep the combined search's complete answer, order,
/// match evidence and overflow flags, including optional Pinyin and diagnostics.
#[test]
fn gh899_independent_page_and_block_pools_keep_the_combined_answer() {
    let _integration_guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    for n in 0..150 {
        std::fs::write(
            dir.path().join(format!("pages/Topic {n}.md")),
            format!("alias:: Alias {n}\n\n- needle {n} [[Phantom/Child]]\n"),
        )
        .unwrap();
    }
    for (name, body) in [
        ("北京", "- needle beijing\n"),
        ("beijing", "- needle\n"),
        ("Café", "- omitted needle\n"),
        ("Work___Plan", "title:: Work/Plan\n\n- needle\n"),
    ] {
        std::fs::write(dir.path().join(format!("pages/{name}.md")), body).unwrap();
    }
    let (store, _, _) = Store::open(dir.path(), Default::default()).unwrap();
    let graph = store.whole_graph().unwrap();
    for transliteration in [false, true] {
        for text in [
            "needle",
            "alias",
            "topic 1",
            "Work",
            "Phantom",
            "bj",
            "beijing",
            "cafe",
            "needle -omitted",
            "\"needle\"",
            "/needle/",
            "/(broken/",
        ] {
            let search = |page_limit, block_limit| {
                graph
                    .search(
                        &SearchRequest {
                            transliteration,
                            text: text.into(),
                            within: None,
                            page_limit,
                            block_limit,
                            explain: false,
                            page_match_scope: None,
                            page_view: None,
                            block_view: None,
                        },
                        &Cancel(Arc::new(AtomicBool::new(false))),
                    )
                    .unwrap()
            };
            let combined = search(100, 100);
            let pages = search(100, 0);
            let blocks = search(0, 100);
            let split: Vec<_> = pages.hits.iter().chain(&blocks.hits).collect();
            assert_eq!(
                serde_json::to_value(&combined.hits).unwrap(),
                serde_json::to_value(split).unwrap(),
                "{text:?}, Pinyin={transliteration}"
            );
            assert_eq!(combined.has_more.pages, pages.has_more.pages);
            assert_eq!(combined.has_more.blocks, blocks.has_more.blocks);
            assert_eq!(
                serde_json::to_value(&combined.diagnostics).unwrap(),
                serde_json::to_value(&pages.diagnostics).unwrap(),
                "page-first diagnostics for {text:?}"
            );
        }
    }
    store.close();
}
