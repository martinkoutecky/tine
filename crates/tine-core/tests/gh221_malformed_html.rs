//! GH #221: a page containing a malformed HTML fragment panicked the parser, so
//! the page was silently dropped from the search index. The panic came from
//! lsdoc's fail-safe guard ("v2 parser does not yet own \"md\" input") and is
//! owned by lsdoc >= v0.5.4; this test pins the user-visible outcome — the page
//! loads and stays searchable — against a future parser regression.

use tine_core::Graph;

#[test]
fn gh221_malformed_html_fragment_indexes_without_panic() {
    let dir = std::env::temp_dir().join(format!("gh221-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("pages")).unwrap();
    // The reporter's minimal repro: one line, no trailing newline.
    std::fs::write(dir.join("pages/test-page.md"), b"- <div </div><").unwrap();

    let g = Graph::open(&dir);
    let entries = g.list_pages();
    let entry = entries
        .iter()
        .find(|p| p.name == "test-page")
        .expect("page listed")
        .clone();
    let dto = g.load_page(&entry).expect("load ok");
    let exec = g.run_graph_search("div", 100, 100, false);
    let failures = g.page_index_failures();

    let _ = std::fs::remove_dir_all(&dir);

    assert!(!dto.blocks.is_empty(), "page has no blocks");
    assert_eq!(exec.hits.len(), 1, "block must be searchable");
    assert!(
        failures.is_empty(),
        "page excluded from index: {failures:?}"
    );
}
