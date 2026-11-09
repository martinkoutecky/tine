use std::sync::{atomic::AtomicBool, Arc};
use tine_core::query_plan::QueryHit;
use tine_store::{Cancel, QueryDialect, QueryResult, SearchRequest, Store};

#[test]
fn page_property_search_keeps_boolean_phrase_and_regex_membership() {
    let _guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    std::fs::write(
        dir.path().join("pages/Host.md"),
        "reviewer:: Avery\nid:: hidden\n\n- Body\n",
    )
    .unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    for (text, count) in [
        ("Avery", 1),
        ("reviewer", 1),
        ("\"reviewer:: Avery\"", 1),
        ("/^reviewer::/", 1),
        ("Missing OR Avery", 1),
        ("Avery -hidden", 1),
        ("Missing Avery", 0),
        ("-Missing", 0),
    ] {
        let found = store
            .whole_graph()
            .unwrap()
            .search(
                &SearchRequest {
                    text: text.into(),
                    transliteration: false,
                    within: None,
                    page_limit: 0,
                    block_limit: 100,
                    explain: false,
                    page_match_scope: None,
                    page_view: None,
                    block_view: None,
                },
                &Cancel(Arc::new(AtomicBool::new(false))),
            )
            .unwrap();
        assert_eq!(found.hits.len(), count, "{text}: {:?}", found.hits);
    }
    store.close();
}

#[test]
fn empty_matching_regex_does_not_spend_its_window_on_hidden_only_headers() {
    let _guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    std::fs::write(
        dir.path().join("pages/Host.md"),
        "id:: hiddenidentity\n\n- Visible body\n",
    )
    .unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let result = store
        .whole_graph()
        .unwrap()
        .search(
            &SearchRequest {
                text: "/^/".into(),
                transliteration: false,
                within: None,
                page_limit: 0,
                block_limit: 1,
                explain: false,
                page_match_scope: None,
                page_view: None,
                block_view: None,
            },
            &Cancel(Arc::new(AtomicBool::new(false))),
        )
        .unwrap();
    assert!(
        matches!(result.hits.as_slice(), [QueryHit::Block { display_text, .. }] if display_text.contains("Visible body")),
        "a hidden-only header must not displace the visible result: {:?}",
        result.hits
    );
    store.close();
}

#[test]
fn ctrl_k_finds_user_property_keys_and_values_in_blocks_and_page_headers() {
    let _guard = crate::integration_lock();
    let mut missing = Vec::new();
    for (ext, source) in [
        ("md", "pagekind:: PageCafé\n\n- Countercheck\n  reviewer:: BlockCafé\n  id:: hiddenidentity\n  collapsed:: hiddenfold\n  tine.view:: hiddenchrome\n  hl-stamp:: hiddenhighlight\n"),
        ("org", "#+pagekind: PageCafé\n\n* Countercheck\n:PROPERTIES:\n:reviewer: BlockCafé\n:id: hiddenidentity\n:collapsed: hiddenfold\n:tine.view: hiddenchrome\n:hl-stamp: hiddenhighlight\n:END:\n"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("pages")).unwrap();
        std::fs::write(dir.path().join(format!("pages/Host.{ext}")), source).unwrap();
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        let graph = store.whole_graph().unwrap();
        let search = |text: &str| {
            graph.search(&SearchRequest {
                text: text.into(), transliteration: false, within: None,
                page_limit: 0, block_limit: 100, explain: false,
                page_match_scope: None, page_view: None, block_view: None,
            }, &Cancel(Arc::new(AtomicBool::new(false)))).unwrap()
        };
        for (term, header) in [("reviewer", false), ("blockcafe", false), ("pagekind", true), ("pagecafe", true)] {
            let result = search(term);
            let [QueryHit::Block { block, display_text, evidence, .. }] = result.hits.as_slice() else {
                missing.push(format!("{ext}: {term} must identify its property owner: {:?}", result.hits));
                continue;
            };
            assert_eq!(block.page_property, header, "{ext}: {term}");
            assert!(!evidence.is_empty(), "a property hit needs snippet evidence");
            assert!(evidence.iter().flat_map(|e| &e.spans).any(|span| {
                let units: Vec<_> = display_text.encode_utf16().skip(span.start).take(span.end - span.start).collect();
                let highlighted = String::from_utf16_lossy(&units);
                tine_core::search_query::canonical_fold(&highlighted).contains(term)
            }), "{ext}: {term} must highlight the matching property text");
            assert!(!display_text.contains("hiddenidentity"));
        }
        for term in ["hiddenidentity", "hiddenfold", "hiddenchrome", "hiddenhighlight", "id", "collapsed", "hl-stamp"] {
            assert!(search(term).hits.is_empty(), "{ext}: hidden property {term}");
        }
        assert_eq!(search("Countercheck").hits.len(), 1);
        for term in ["reviewer", "pagekind"] {
            let result = graph.search(&SearchRequest {
                text: term.into(), transliteration: false, within: None, page_limit: 10,
                block_limit: 10, explain: false,
                page_match_scope: Some(tine_core::query::ir::FriendlyPageMatchScope::Content),
                page_view: None, block_view: None,
            }, &Cancel(Arc::new(AtomicBool::new(false)))).unwrap();
            assert!(result.hits.iter().any(|hit| matches!(hit, QueryHit::Page { page, .. } if page.name == "Host")), "{ext}: content page scope must find {term}");
        }
        let QueryResult::Simple(exact) = graph.query("hiddenidentity", QueryDialect::Simple).unwrap() else { panic!("simple query"); };
        assert!(!exact.is_empty(), "exact queries still read raw property lines");
        let QueryResult::Simple(exact) = graph.query("blockcafe", QueryDialect::Simple).unwrap() else { panic!("simple query"); };
        assert!(exact.is_empty(), "query text keeps exact case and accents (D4)");
        store.close();
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn properties_only_first_root_is_searchable_too() {
    let _guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    std::fs::write(
        dir.path().join("pages/Host.md"),
        "- category:: FirstRootWitness\n- Body\n",
    )
    .unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    for term in ["category", "FirstRootWitness"] {
        let result = store
            .whole_graph()
            .unwrap()
            .search(
                &SearchRequest {
                    text: term.into(),
                    transliteration: false,
                    within: None,
                    page_limit: 0,
                    block_limit: 100,
                    explain: false,
                    page_match_scope: None,
                    page_view: None,
                    block_view: None,
                },
                &Cancel(Arc::new(AtomicBool::new(false))),
            )
            .unwrap();
        assert_eq!(result.hits.len(), 1);
    }
    store.close();
}

#[test]
fn runtime_search_memos_do_not_change_checkpoint_bytes() {
    let _guard = crate::integration_lock();
    for count in [1, 60] {
        let blocks: Vec<_> = (0..count)
            .map(|_| tine_core::doc::DocBlock::new("Body\nreviewer:: Café\nid:: hiddenidentity"))
            .collect();
        let before =
            postcard::to_stdvec(&blocks.iter().map(|b| b.projection()).collect::<Vec<_>>())
                .unwrap();
        for block in &blocks {
            let (text, folded) = block.search_text_pair(true);
            assert!(text.contains("reviewer:: Café"));
            assert!(folded.contains("reviewer:: cafe"));
            assert!(block.search_text_pair(false).1.contains("reviewer:: café"));
        }
        let after = postcard::to_stdvec(&blocks.iter().map(|b| b.projection()).collect::<Vec<_>>())
            .unwrap();
        assert_eq!(after, before);
        eprintln!("I-25: {count} blocks, {} projection bytes before/after; added 0 bytes, 0 files, 0 transport bytes", before.len());
    }
}

#[test]
fn configured_hidden_properties_stay_out_of_block_and_page_search() {
    let _guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    std::fs::create_dir(dir.path().join("logseq")).unwrap();
    std::fs::write(
        dir.path().join("logseq/config.edn"),
        "{:block-hidden-properties #{:private-note}}",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("pages/Host.md"),
        "private-note:: pageSecret\n\n- Body\n  private-note:: blockSecret\n  reviewer:: Avery\n",
    )
    .unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let graph = store.whole_graph().unwrap();
    for term in ["pageSecret", "blockSecret", "private-note"] {
        let result = graph
            .search(
                &SearchRequest {
                    text: term.into(),
                    transliteration: false,
                    within: None,
                    page_limit: 0,
                    block_limit: 100,
                    explain: false,
                    page_match_scope: None,
                    page_view: None,
                    block_view: None,
                },
                &Cancel(Arc::new(AtomicBool::new(false))),
            )
            .unwrap();
        assert!(result.hits.is_empty(), "{term}");
    }
    store.close();
}
