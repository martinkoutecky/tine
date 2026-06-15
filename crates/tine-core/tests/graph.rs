//! Integration tests against the on-disk demo graph (standard layout).

use tine_core::Graph;
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
    let entry = g.find_entry("logseq-claude", tine_core::PageKind::Page).unwrap();
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
fn publishes_only_public_pages() {
    let root = std::env::temp_dir().join(format!("tine-publish-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(
        root.join("pages").join("Shared.md"),
        "public:: true\n\n- hello, see [[Secret]]\n",
    )
    .unwrap();
    std::fs::write(root.join("pages").join("Secret.md"), "- private notes\n").unwrap();

    let g = Graph::open(&root);
    let (dir, n) = g.publish_html().unwrap();
    assert_eq!(n, 1, "only the public page is published");
    let p = std::fs::read_to_string(format!("{dir}/shared.html")).unwrap();
    assert!(p.contains("<h1 class=\"page\">Shared</h1>"));
    assert!(p.contains("<a class=\"ref\""), "should link [[refs]]");
    // The private page must not be exported.
    assert!(!std::path::Path::new(&format!("{dir}/secret.html")).exists());

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn search_cache_reflects_saves_and_deletes() {
    use tine_core::model::{BlockDto, PageDto, PageKind};

    // Isolated temp graph so we can mutate it freely.
    let root = std::env::temp_dir().join(format!("tine-cache-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages").join("Seed.md"), "- a seed block\n").unwrap();

    let g = Graph::open(&root);
    // Warms the cache on first search.
    assert_eq!(g.search("zonkwort", 10).len(), 0, "token absent initially");

    // Saving a page with the token must be visible to a subsequent search
    // without any disk re-scan (cache upsert).
    let page = PageDto {
        name: "Fresh".into(),
        kind: PageKind::Page,
        title: "Fresh".into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "x".into(),
            raw: "contains zonkwort here".into(),
            collapsed: false,
            children: vec![],
        }],
    };
    g.save_page(&page).unwrap();
    let hits = g.search("zonkwort", 10);
    assert_eq!(hits.len(), 1, "saved page should be searchable");
    assert_eq!(hits[0].page, "Fresh");

    // Deleting the page removes it from the cache too.
    g.delete_page("Fresh", PageKind::Page).unwrap();
    assert_eq!(g.search("zonkwort", 10).len(), 0, "deleted page should drop out");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn search_ignores_hidden_property_metadata() {
    use tine_core::model::{BlockDto, PageDto, PageKind};

    let root = std::env::temp_dir().join(format!("tine-search-meta-{}", std::process::id()));
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let g = Graph::open(&root);

    // A block whose only occurrence of "qzxmeta" is in a property line (like an
    // id:: uuid or hl-color::) must NOT match — the user can't see it.
    let page = PageDto {
        name: "Meta".into(),
        kind: PageKind::Page,
        title: "Meta".into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "x".into(),
            raw: "a perfectly ordinary block\nsome-prop:: qzxmeta".into(),
            collapsed: false,
            children: vec![],
        }],
    };
    g.save_page(&page).unwrap();
    assert_eq!(
        g.search("qzxmeta", 10).len(),
        0,
        "token only in a property line should not be a search hit"
    );
    // But the visible body is still searchable.
    assert_eq!(g.search("ordinary", 10).len(), 1, "visible body still matches");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn save_preserves_file_format_no_churn() {
    use tine_core::model::PageKind;

    let root = std::env::temp_dir().join(format!("tine-fmt-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    // Logseq style: no trailing newline. Plus one with a newline, and a
    // space-indented file (Tine emits tabs by default — must preserve spaces).
    let no_nl = "- alpha\n\t- beta";
    let with_nl = "- gamma\n";
    let spaces = "- root\n  - two-space child\n    - grandchild";
    std::fs::write(root.join("pages").join("A.md"), no_nl).unwrap();
    std::fs::write(root.join("pages").join("B.md"), with_nl).unwrap();
    std::fs::write(root.join("pages").join("C.md"), spaces).unwrap();

    let g = Graph::open(&root);
    // Load then save unchanged must be byte-identical (no churn): each file's
    // trailing-newline + indent convention is preserved.
    for name in ["A", "B", "C"] {
        let dto = g.load_named(name, PageKind::Page).unwrap().unwrap();
        g.save_page(&dto).unwrap();
    }
    assert_eq!(std::fs::read_to_string(root.join("pages").join("A.md")).unwrap(), no_nl);
    assert_eq!(std::fs::read_to_string(root.join("pages").join("B.md")).unwrap(), with_nl);
    assert_eq!(std::fs::read_to_string(root.join("pages").join("C.md")).unwrap(), spaces);

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn save_refuses_to_clobber_external_change() {
    use tine_core::model::PageKind;

    let root = std::env::temp_dir().join(format!("tine-conflict-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let path = root.join("pages").join("N.md");
    std::fs::write(&path, "- one").unwrap();

    let g = Graph::open(&root);
    // Build the cache (Tine now "knows" N = "- one"), then load it for editing.
    g.search("one", 10);
    let dto = g.load_named("N", PageKind::Page).unwrap().unwrap();

    // An external writer (another app / Syncthing) changes the file.
    std::fs::write(&path, "- EXTERNAL EDIT").unwrap();

    // Saving the now-stale page must fail with a conflict and NOT overwrite.
    let err = g.save_page(&dto).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "- EXTERNAL EDIT");

    // "Keep mine" force-saves over it.
    g.force_save_page(&dto).unwrap();
    assert!(std::fs::read_to_string(&path).unwrap().contains("one"));

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn sync_file_detects_external_change_and_suppresses_self() {
    use tine_core::model::PageKind;

    let root = std::env::temp_dir().join(format!("tine-sync-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let path = root.join("pages").join("S.md");
    std::fs::write(&path, "- before").unwrap();

    let g = Graph::open(&root);
    g.search("before", 10); // build the cache (S = "- before")

    // No external change yet → sync reports nothing.
    assert!(g.sync_file(&path).is_none());

    // External edit → sync reports the entry and refreshes the cache.
    std::fs::write(&path, "- after the change").unwrap();
    let changed = g.sync_file(&path).expect("external change detected");
    assert_eq!(changed.name, "S");
    assert_eq!(changed.kind, PageKind::Page);
    assert_eq!(g.search("after", 10).len(), 1, "cache updated to new content");
    assert_eq!(g.search("before", 10).len(), 0);

    // Re-syncing the same content is a no-op (self-write suppression).
    assert!(g.sync_file(&path).is_none());

    // Deletion is reported and drops it from the cache.
    std::fs::remove_file(&path).unwrap();
    assert!(g.forget_file(&path).is_some());
    assert_eq!(g.search("after", 10).len(), 0);

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn query_between_filters_by_journal_date() {
    let root = std::env::temp_dir().join(format!("tine-between-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(root.join("journals").join("2022_06_15.md"), "- TODO [[scs]] recent\n").unwrap();
    std::fs::write(root.join("journals").join("2019_01_01.md"), "- TODO [[scs]] old\n").unwrap();

    let g = Graph::open(&root);
    let groups = g.run_query(
        "(and (task TODO) (and [[scs]] (between [[Jan 1st, 2021]] [[Jan 1st, 2100]])))",
    );
    let raws: Vec<String> = groups
        .iter()
        .flat_map(|gr| gr.blocks.iter().map(|b| b.raw.clone()))
        .collect();
    assert!(raws.iter().any(|r| r.contains("recent")), "in-range journal matches: {raws:?}");
    assert!(!raws.iter().any(|r| r.contains("old")), "out-of-range journal excluded: {raws:?}");

    std::fs::remove_dir_all(&root).ok();
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
