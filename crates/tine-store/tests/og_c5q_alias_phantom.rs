//! GH #623 comment 13 / master GH #353 (checkpoint-5 packet Q, REG-OG-GH623-ALIAS-353).
//!
//! An alias names the page that declares it. When the alias text is ALSO
//! referenced somewhere (`[[Book]]`), the referenced-name candidate pool used to
//! add a path-less virtual page called "Book" that often ranked first; selecting
//! it opened a standalone alias-named page instead of the owner. The search
//! executor now treats every alias of a file page as already named, so the alias
//! text is never its own candidate. Same candidate boundary for Ctrl+K search
//! (`WholeGraph::search`) and the `[[` / `#` completion pool
//! (`Store::complete_page_names`). ASCII `,` and fullwidth `，` alias lists are
//! both covered (OG `sep-by-comma` splits `[,，]`).

use std::fs;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use tine_core::query_plan::QueryHit;
use tine_store::{Cancel, SearchRequest, Store};

fn graph() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    fs::create_dir(dir.path().join("journals")).unwrap();
    let w = |name: &str, text: &str| fs::write(dir.path().join("pages").join(name), text).unwrap();
    // One owner with two ASCII-comma aliases, one with two fullwidth-comma aliases.
    w(
        "Research Hub.md",
        "alias:: Book, Reading\n\n- actual reading notes\n",
    );
    w("Reading List.md", "alias:: 书籍，阅读\n\n- 阅读笔记\n");
    // A real page whose name merely overlaps an alias.
    w(
        "Real Reading.md",
        "- a genuinely real page whose name contains an alias\n",
    );
    // The alias texts are referenced elsewhere in the graph.
    w(
        "Notes.md",
        "- see [[Book]], [[Reading]], [[书籍]], [[阅读]] and [[Book Shelf]] for the list\n",
    );
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    (dir, store)
}

fn page_hits(store: &Store, text: &str) -> Vec<(String, String, Option<String>)> {
    let request = SearchRequest {
        text: text.into(),
        within: None,
        page_limit: 100,
        block_limit: 0,
        explain: false,
        page_match_scope: None,
        page_view: None,
        block_view: None,
    };
    store
        .whole_graph()
        .unwrap()
        .search(&request, &Cancel(Arc::new(AtomicBool::new(false))))
        .unwrap()
        .hits
        .into_iter()
        .filter_map(|hit| match hit {
            QueryHit::Page {
                page,
                matched_alias,
                ..
            } => Some((
                page.name.clone(),
                page.rel_path_str().to_owned(),
                matched_alias,
            )),
            QueryHit::Block { .. } => None,
        })
        .collect()
}

#[test]
fn alias_reference_is_never_a_phantom_alias_page() {
    let (_dir, store) = graph();
    for (query, alias, owner) in [
        ("book", "book", "Research Hub"),
        ("BOOK", "book", "Research Hub"),
        ("Reading", "reading", "Research Hub"),
        ("书籍", "书籍", "Reading List"),
        ("阅读", "阅读", "Reading List"),
    ] {
        let hits = page_hits(&store, query);
        assert!(
            hits.iter()
                .all(|(name, path, _)| !path.is_empty() || name.to_lowercase() != alias),
            "{query:?} offered a path-less alias page: {hits:?}"
        );
        assert!(
            hits.iter().any(|(name, path, matched)| name == owner
                && !path.is_empty()
                && matched.is_some()),
            "{query:?} must carry the matched alias on the owner hit: {hits:?}"
        );
        // The `[[` / `#` completion pool shares the candidate boundary.
        let names: Vec<_> = store
            .whole_graph()
            .unwrap()
            .complete_page_names(query, 100)
            .into_iter()
            .filter(|entry| entry.rel_path.is_none())
            .map(|entry| entry.name.to_lowercase())
            .collect();
        assert!(
            !names.iter().any(|name| name == alias),
            "completion offered a path-less alias page for {query:?}: {names:?}"
        );
    }
    // An ordinary page that merely contains alias text, and a referenced page
    // that no alias owns, are unaffected.
    assert!(page_hits(&store, "reading")
        .iter()
        .any(|(name, _, matched)| name == "Real Reading" && matched.is_none()));
    assert!(page_hits(&store, "Book Shelf")
        .iter()
        .any(|(name, _, _)| name == "Book Shelf"));
    store.close();
}
