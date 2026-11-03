//! Conceptual model step 2, batch 4 (census Q12, harness gap H1): favorites
//! after a rename or merge through the Rust entry, `pages::rename_or_merge_page`
//! (the app's `rename_page` command and the CLI owner's `page.rename` both call
//! it). Scenarios, target and pinned differences: `cm_favorites_shared.rs`.

use std::collections::BTreeMap;
use std::sync::Arc;

use tine_graph_features::pages;
use tine_store::Store;

#[path = "cm_favorites_shared.rs"]
mod shared;

#[test]
fn favorites_after_rename_through_the_rust_entry() {
    let _integration_guard = crate::integration_lock();
    let mut found = BTreeMap::new();
    for scenario in shared::scenarios() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("graph");
        shared::write_graph(&root, &scenario);
        let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
        store.whole_graph().unwrap();
        let host = crate::test_host(&store);
        let merge_into = scenario.into.as_deref();
        let result = pages::rename_or_merge_page(
            &store,
            &host,
            &scenario.old,
            &scenario.new,
            None,
            merge_into,
            &[],
        );
        drop(host);
        let lines = match result {
            Ok(_) => shared::differences(&root, &scenario, "rust"),
            Err(error) => vec![format!("rust {}: refused: {error}", scenario.kind)],
        };
        found.insert(scenario.id, lines);
    }
    shared::check_expected("rust", found);
}
