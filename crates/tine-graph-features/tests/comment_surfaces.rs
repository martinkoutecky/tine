//! H9TvA: characterize existing comment outputs pending export/search decisions.
use std::fs;
use tine_core::query::ir::FriendlyPageMatchScope;
use tine_core::query::wire_parse::QueryTextDialect;
use tine_core::query_plan::QueryHit;
use tine_graph_features::{print, publish_query, quick_share};
use tine_store::{Cancel, SearchRequest, Store};

#[test]
fn comment_surface_inventory() {
    let _integration_guard = crate::integration_lock();
    let scratch = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let root = scratch.path().join("graph");
    fs::create_dir_all(root.join("pages")).unwrap();
    for (name, body) in [
        ("Markdown.md", "public:: true\n- Body passage\n  - COMMENTMARKER [[Target]]\n    tine.quote:: Body passage\n    - REPLYMARKER [[Target]]\n"),
        ("Org.org", "#+public: true\n* Body passage\n** COMMENTMARKER [[Target]]\n:PROPERTIES:\n:tine.quote: Body passage\n:END:\n*** REPLYMARKER [[Target]]\n"),
    ] {
        fs::write(root.join("pages").join(name), body).unwrap();
    }
    let store = Store::open(&root, Default::default()).unwrap().0;
    let whole = store.whole_graph().unwrap();
    let search = whole
        .search(
            &SearchRequest {
                text: "COMMENTMARKER".into(),
                transliteration: false,
                within: None,
                page_limit: 10,
                block_limit: 10,
                explain: false,
                page_match_scope: Some(FriendlyPageMatchScope::Both),
                page_view: None,
                block_view: None,
            },
            &Cancel::default(),
        )
        .unwrap();
    let mut blocks = 0;
    let mut pages = 0;
    for hit in &search.hits {
        match hit {
            QueryHit::Block {
                block,
                display_text,
                ..
            } => {
                blocks += 1;
                assert!(display_text.contains("COMMENTMARKER"));
                assert!(!display_text.contains("tine.quote"));
                assert!(block.raw.contains("tine.quote"));
                assert_eq!(block.breadcrumb, ["Body passage"]);
            }
            QueryHit::Page { .. } => pages += 1,
        }
    }
    assert_eq!(
        (pages, blocks),
        (2, 2),
        "comments also make content-search page hits"
    );
    let backlinks = whole.backlinks("Target").unwrap();
    assert_eq!(backlinks.len(), 2);
    let (_, counts) = whole.page_backlink_counts();
    assert_eq!(
        counts.iter().find(|(name, _)| name == "Target").unwrap().1,
        4
    );
    for name in ["Markdown", "Org"] {
        let html = print::page_print_html(&store, name, Default::default())
            .unwrap()
            .unwrap();
        assert!(html.contains("COMMENTMARKER"));
        assert!(html.contains("REPLYMARKER"));
        assert!(!html.contains("tine.quote"));
    }
    let site = publish_query::publish_static(&store, scratch.path(), "site", true).unwrap();
    let mut html_pages = 0;
    for entry in fs::read_dir(&site.path).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|ext| ext == "html") {
            let html = fs::read_to_string(path).unwrap();
            if html.contains("COMMENTMARKER") {
                html_pages += 1;
                assert!(html.contains("REPLYMARKER"));
                assert!(!html.contains("tine.quote"));
            }
        }
    }
    assert_eq!(html_pages, 2);
    let index =
        fs::read_to_string(std::path::Path::new(&site.path).join("search-index.js")).unwrap();
    assert!(index.contains("COMMENTMARKER"));
    assert!(index.contains("REPLYMARKER"));
    let bundle = vec![("index.html".into(), b"<head></head>".to_vec())];
    let live = publish_query::publish_live_with_sheets_in(
        &store,
        scratch.path(),
        "live",
        true,
        &bundle,
        vec![],
        scratch.path(),
    )
    .unwrap();
    let snapshot =
        fs::read_to_string(std::path::Path::new(&live.path).join("app/snapshot.json")).unwrap();
    assert!(snapshot.contains("COMMENTMARKER"));
    assert!(snapshot.contains("REPLYMARKER"));
    assert!(snapshot.contains("tine.quote"));
    let request = publish_query::QueryExportRequest {
        argument: "@page and name = 'Markdown' or name = 'Org'".into(),
        dialect: QueryTextDialect::MacroTql,
        properties: vec![],
        current_page: None,
        name: "Comments".into(),
        host_block_id: None,
        folder: None,
        replace: false,
        asset_budget_bytes: None,
        include_query_text: false,
    };
    let prepared =
        quick_share::prepare(&store, &request, "2026-11-13T00:00:00Z", scratch.path()).unwrap();
    let bytes =
        quick_share::gunzip_bounded(&prepared.payload, quick_share::MAX_DOCUMENT_BYTES).unwrap();
    let document = String::from_utf8(bytes).unwrap();
    assert!(document.contains("COMMENTMARKER"));
    assert!(document.contains("REPLYMARKER"));
    assert!(
        document.contains("tine.quote"),
        "quick share retains source properties"
    );
}
