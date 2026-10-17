//! The share dialog's "this page" selector (share dialog design §1, review
//! R2/R3): the TQL page row `{{tine-query @page and name = '<page>'}}` selects
//! the page itself, so a page with no blocks is exported, and a rename carries
//! it along inside export definition blocks (`export::` / `tine.share::`)
//! while ordinary query blocks keep OG's behaviour.
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tine_core::query::macro_text::query_macro_extent;
use tine_core::query::wire_parse::QueryTextDialect;
use tine_graph_features::pages;
use tine_graph_features::publish_query::{plan_query, publish_query, QueryExportRequest};
use tine_store::Store;

fn graph(files: &[(&str, &str)]) -> (PathBuf, Arc<Store>) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-share-selector-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    for dir in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    fs::write(
        root.join("logseq/config.edn"),
        "{:file/name-format :triple-lowbar}\n",
    )
    .unwrap();
    for (rel, body) in files {
        fs::write(root.join(rel), body).unwrap();
    }
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    (root, store)
}

fn request(argument: &str, dialect: QueryTextDialect) -> QueryExportRequest {
    QueryExportRequest {
        argument: argument.into(),
        dialect,
        properties: vec![],
        current_page: None,
        name: "Share".into(),
        host_block_id: None,
        folder: None,
        replace: false,
        asset_budget_bytes: None,
        include_query_text: false,
    }
}

fn selected(store: &Store, argument: &str, dialect: QueryTextDialect) -> (String, Vec<String>) {
    let plan = plan_query(store, &request(argument, dialect)).unwrap();
    (
        plan.anchor,
        plan.pages.into_iter().map(|p| p.name).collect(),
    )
}

fn bundle() -> Vec<(String, Vec<u8>)> {
    vec![(
        "index.html".into(),
        b"<!doctype html><html><head><title>Tine</title></head><body><div id=\"root\"></div></body></html>"
            .to_vec(),
    )]
}

#[test]
fn the_selector_exports_a_page_with_no_blocks_and_page_keeps_its_block_meaning() {
    let _integration_guard = crate::integration_lock();
    let (root, store) = graph(&[("pages/Empty.md", ""), ("pages/Other.md", "- x\n")]);
    let tql = QueryTextDialect::MacroTql;
    assert_eq!(
        selected(&store, "@page and name = 'Empty'", tql),
        ("page".to_owned(), vec!["Empty".to_owned()])
    );
    assert_eq!(
        selected(&store, "@page and name = 'Empty' or name = 'Other'", tql).1,
        ["Empty", "Other"],
        "the selector stays an ordinary term the builder can combine"
    );
    let request = request("@page and name = 'Empty'", tql);
    let plan = plan_query(&store, &request).unwrap();
    let receipt = publish_query(&store, &request, &plan.fingerprint, &bundle()).unwrap();
    assert_eq!(receipt.pages, 1);
    assert!(PathBuf::from(&receipt.path).join("empty.html").exists());
    // OG `(page "x")` still means the page's BLOCKS (`query_dsl.cljs`
    // `build-page`), so an empty page selects nothing there (D-18).
    assert_eq!(
        selected(&store, "(page \"Empty\")", QueryTextDialect::MacroQuery),
        ("block".to_owned(), Vec::new())
    );
    store.close();
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_rename_carries_export_definitions_and_leaves_ordinary_queries() {
    let _integration_guard = crate::integration_lock();
    let exports = "tine/exports:: true\npublic:: false\n\n\
                   - {{tine-query @page and name = 'Project'}}\n  export:: project-site\n\
                   - {{tine-query @page and name = 'Project/Sub'}}\n  tine.share:: me/abc\n\
                   - {{tine-query @page and name = 'Proj'}}\n  export:: by-alias\n";
    let dashboard = "- {{tine-query @page and name = 'Project'}}\n";
    let (root, store) = graph(&[
        ("pages/Project.md", "alias:: Proj\n\n- notes\n"),
        ("pages/Project___Sub.md", "- sub\n"),
        ("pages/Exports.md", exports),
        ("pages/Dashboard.md", dashboard),
        (
            "pages/Shares.org",
            "* {{tine-query @page and name = 'Project'}}\n:PROPERTIES:\n:export: org-site\n:END:\n",
        ),
    ]);
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
    pages::rename_or_merge_page(&store, &host, "Project", "Project archive", None, None, &[])
        .unwrap();
    drop(host);
    assert!(root.join("pages/Project archive___Sub.md").exists());
    let exports_after = fs::read_to_string(root.join("pages/Exports.md")).unwrap();
    assert_eq!(
        exports_after,
        "tine/exports:: true\npublic:: false\n\n\
         - {{tine-query @page and name = 'Project archive'}}\n  export:: project-site\n\
         - {{tine-query @page and name = 'Project archive/Sub'}}\n  tine.share:: me/abc\n\
         - {{tine-query @page and name = 'Proj'}}\n  export:: by-alias\n",
        "namespace descendants follow; an alias is not the page's name and stays"
    );
    assert_eq!(
        fs::read_to_string(root.join("pages/Shares.org")).unwrap(),
        "* {{tine-query @page and name = 'Project archive'}}\n:PROPERTIES:\n:export: org-site\n:END:\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("pages/Dashboard.md")).unwrap(),
        dashboard,
        "an ordinary query block keeps OG's rename behaviour"
    );
    // A new page under the old name is not what the definition shares.
    fs::write(root.join("pages/Project.md"), "- someone else's notes\n").unwrap();
    store.refresh(tine_store::Depth::Stamps).unwrap();
    let definition = query_macro_extent(&exports_after).unwrap();
    assert_eq!(
        selected(&store, &definition.argument, QueryTextDialect::MacroTql).1,
        ["Project archive"]
    );
    store.close();
    let _ = fs::remove_dir_all(&root);
}
