//! REG-OG-GH644 at the backend doors the app calls: `get_page` (a link or tag
//! click) and the whole-graph name inventory (`page_inventory`, the `[[`
//! completion source). One page whose preamble folds a blank line into its
//! property node (`title:: X\n\n#+description: y`, or an Org file-level
//! drawer before `#+title:`) panicked the region reader: the initial load
//! stopped, and every page open answered "task N panicked with message
//! \"byte range starts at N+1 but ends at N\"". Invariant I-2: one bad file
//! must not refuse the graph.
use std::fs;
use tine_core::model::PageKind;
use tine_graph_features::pages;
use tine_store::Store;

fn graph(files: &[(&str, &str)]) -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    for dir_name in ["pages", "journals"] {
        fs::create_dir_all(dir.path().join(dir_name)).unwrap();
    }
    for (rel, body) in files {
        fs::write(dir.path().join(rel), body).unwrap();
    }
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    (dir, store)
}

fn assert_graph_serves_links(store: &Store, titled: &str) {
    let view = store
        .whole_graph()
        .expect("the initial load stopped, so `[[` completion has no names");
    let inventory = view.inventory();
    for name in ["Links", titled] {
        assert!(
            inventory.0.iter().any(|entry| entry.name == name),
            "{name} missing from the completion inventory"
        );
    }
    // A link or tag to an existing page opens it; one to a page no file
    // claims yet opens as a new empty page (`None`), never an error.
    for (name, kind, exists) in [
        ("Links", PageKind::Page, true),
        ("2026_10_05", PageKind::Journal, true),
        ("Oct 5th, 2026", PageKind::Journal, true),
        (titled, PageKind::Page, true),
        ("tag", PageKind::Page, false),
    ] {
        let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pages::get_page(store, name, kind)
        }))
        .unwrap_or_else(|_| panic!("get_page({name:?}) panicked"));
        match read {
            Ok(page) => assert_eq!(page.is_some(), exists, "get_page({name:?})"),
            Err(error) => panic!("get_page({name:?}): {error:?}"),
        }
    }
}

#[test]
fn a_property_node_with_a_folded_blank_line_leaves_every_page_openable() {
    for (rel, body, titled) in [
        (
            "pages/Other.md",
            "title:: Titled\n\n#+description: y\n\n- body\n",
            "Titled",
        ),
        (
            "pages/Roam.org",
            ":PROPERTIES:\n:id: 6512\n:END:\n\n#+title: Roam Title\n\n* body\n",
            "Roam Title",
        ),
    ] {
        let (_dir, store) = graph(&[
            (
                "pages/Links.md",
                "- This is a link to a journal page\n\t- [[2026_10_05]] #tag\n",
            ),
            ("journals/2026_10_05.md", "- day\n"),
            (rel, body),
        ]);
        // Before Ready the file-name listing answers; it reads every
        // ordinary page's preamble too.
        let early = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pages::get_page(&store, "Links", PageKind::Page)
        }))
        .unwrap_or_else(|_| panic!("{rel}: an early get_page panicked"));
        assert!(early.is_ok(), "{rel}: {:?}", early.err());
        assert_graph_serves_links(&store, titled);
        store.close();
    }
}
