//! Share admission and preparation parse each source once (moved from the lib
//! tests of `share_admission` and `publication_snapshot`). These cases read the
//! process-global cost counters, so each takes `exclusive_integration_lock`:
//! no other case's graph activity runs beside its measurement.
use std::fs;
use tine_core::query::wire_parse::QueryTextDialect;
use tine_graph_features::publish_query::QueryExportRequest;
use tine_graph_features::{quick_share, share_admission};
use tine_store::{cost_counters, Store, WholeGraph};

#[test]
fn accepted_share_parses_each_source_once() {
    let _integration_guard = crate::exclusive_integration_lock();
    let files = vec![
        ("pages/Public.md".to_owned(), b"- text\n".repeat(10_000)),
        ("pages/Other.org".to_owned(), b"* text\n".repeat(10_000)),
    ];
    cost_counters::reset();
    let started = std::time::Instant::now();
    share_admission::check_sizes(
        "Share",
        "",
        files.iter().map(|(path, text)| (path.len(), text.len())),
    )
    .unwrap();
    let (graph, _) = WholeGraph::from_files(files, Vec::new(), false).unwrap();
    share_admission::check_blocks(
        graph
            .corpus(None, false)
            .pages
            .iter()
            .map(|p| p.document.as_ref()),
    )
    .unwrap();
    let count = cost_counters::snapshot();
    eprintln!(
        "share_open sources=2 blocks=20000 outline_parses={} elapsed={:?}",
        count.parses,
        started.elapsed()
    );
    assert_eq!((count.parses, count.full_reads, count.corpus), (2, 2, 1));
}

#[test]
fn share_preparation_parses_each_selected_source_once() {
    let _integration_guard = crate::exclusive_integration_lock();
    let dir = crate::test_support::fresh_dir("tine-share-cost-");
    fs::create_dir_all(dir.join("pages")).unwrap();
    let body = "- selected body with [[Ref]]\n".repeat(60);
    for i in 0..200 {
        fs::write(dir.join(format!("pages/Page {i:03}.md")), &body).unwrap();
    }
    let store = Store::open(&dir, Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    let request = QueryExportRequest {
        // Every page file; `Ref` is the referenced page no file holds.
        argument: "@page and not (name = 'Ref')".into(),
        dialect: QueryTextDialect::MacroTql,
        properties: vec![],
        current_page: None,
        textual_current_page: None,
        name: "Home".into(),
        host_block_id: None,
        folder: None,
        replace: false,
        asset_budget_bytes: None,
        include_query_text: false,
        include_comments: None,
    };
    let mut samples = Vec::new();
    for _ in 0..3 {
        cost_counters::reset();
        let started = std::time::Instant::now();
        let prepared = quick_share::prepare(&store, &request, "2026-10-17T00:00:00Z").unwrap();
        let counts = cost_counters::snapshot();
        assert_eq!(prepared.preview.pages.len(), 200);
        let sample = (
            counts.full_reads,
            counts.preamble_reads,
            counts.parses,
            counts.corpus,
        );
        eprintln!("share writer: 200 pages, 12000 selected blocks, full reads/preambles/parses/corpora={sample:?}, elapsed={:?}", started.elapsed());
        samples.push(sample);
    }
    store.close();
    let _ = fs::remove_dir_all(&dir);
    // The private build graph reads, discovers and parses each selected page
    // and the generated home once, and builds one corpus; the plan's selection
    // is the other corpus (the whole-graph corpus the query selects from).
    assert_eq!(samples, [(201, 201, 201, 2); 3]);
}
