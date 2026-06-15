//! Integration tests against the on-disk demo graph (standard layout).

use logseq_core::Graph;
use std::path::PathBuf;

fn demo_graph() -> Graph {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples/demo-graph");
    Graph::open(root)
}

#[test]
fn lists_journals_and_pages() {
    let g = demo_graph();
    let pages = g.list_pages();
    assert!(pages.iter().any(|p| p.name == "logseq-claude"));
    let journals = g.journals_desc();
    // Newest first.
    assert_eq!(journals.first().map(|j| j.name.as_str()), Some("Jun 14th, 2026"));
    assert!(journals.len() >= 2);
}

#[test]
fn loads_a_page_with_nesting_and_properties() {
    let g = demo_graph();
    let entry = g.find_entry("logseq-claude", logseq_core::PageKind::Page).unwrap();
    let dto = g.load_page(&entry).unwrap();
    assert_eq!(dto.pre_block.as_deref(), Some("title:: logseq-claude\ntags:: project, tooling"));
    // Has a nested child under the first block.
    assert!(dto.blocks[0].children.len() >= 1);
}

#[test]
fn backlinks_to_parameterized_complexity() {
    let g = demo_graph();
    let groups = g.backlinks("parameterized complexity");
    let pages: Vec<&str> = groups.iter().map(|gr| gr.page.as_str()).collect();
    // Referenced from the journal, logseq-claude, and n-fold IP.
    assert!(pages.contains(&"logseq-claude"), "pages: {pages:?}");
    assert!(pages.contains(&"n-fold IP"), "pages: {pages:?}");
}

#[test]
fn publishes_static_html() {
    let g = demo_graph();
    let (dir, n) = g.publish_html().unwrap();
    assert!(n >= 4, "published {n} pages");
    let idx = std::fs::read_to_string(format!("{dir}/index.html")).unwrap();
    assert!(idx.contains(".html"));
    let p = std::fs::read_to_string(format!("{dir}/logseq-claude.html")).unwrap();
    assert!(p.contains("<h1>logseq-claude</h1>"));
    assert!(p.contains("<a class=\"ref\""), "should link [[refs]]");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn query_open_tasks() {
    let g = demo_graph();
    let groups = g.run_query("(task TODO DOING)");
    let raws: Vec<String> = groups
        .iter()
        .flat_map(|gr| gr.blocks.iter().map(|b| b.raw.clone()))
        .collect();
    assert!(raws.iter().any(|r| r.starts_with("TODO Ship the M0")), "got: {raws:?}");
    assert!(raws.iter().any(|r| r.starts_with("DOING Wire up")), "got: {raws:?}");
    // A DONE task must not match.
    assert!(!raws.iter().any(|r| r.contains("DONE Validate")), "got: {raws:?}");
}
