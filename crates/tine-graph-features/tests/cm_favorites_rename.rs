//! Conceptual model step 2, batch 4 (census Q12, harness gap H1): favorites
//! after a rename or merge through the Rust entry, `pages::rename_or_merge_page`
//! (the app's `rename_page` command and the CLI owner's `page.rename` both call
//! it), and after a title:: change through a guarded page write followed by
//! `pages::follow_page_identity` (what the app's `follow_page_identity`
//! command and the CLI owner's page-write hook call). Scenarios, target and
//! pinned differences: `cm_favorites_shared.rs`.

use std::collections::BTreeMap;
use std::sync::Arc;

use tine_graph_features::pages;
use tine_store::Store;

#[path = "cm_favorites_shared.rs"]
mod shared;

/// The title:: change arrives as an external editor (or sync) writes it, the
/// in-scope case where no Tine window computes anything; then the backend's
/// answerer moves config.edn's home and favorites. (The app's own saves and
/// the CLI owner's `block.edit` reach the same `follow_page_identity`: the
/// owner check covers the latter.)
fn retitle(
    store: &Store,
    root: &std::path::Path,
    scenario: &shared::Scenario,
) -> std::io::Result<pages::IdentityFollow> {
    let path = scenario.path.as_deref().expect("a retitle names its file");
    std::fs::write(root.join(path), scenario.text.as_deref().unwrap()).unwrap();
    pages::follow_page_identity(store, path, &scenario.old)
}

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
        let result = if scenario.kind == "retitle" {
            retitle(&store, &root, &scenario).map(drop)
        } else {
            let host = crate::test_host(&store);
            pages::rename_or_merge_page(
                &store,
                &host,
                &scenario.old,
                &scenario.new,
                None,
                scenario.into.as_deref(),
                &[],
            )
            .map(drop)
        };
        let lines = match result {
            Ok(()) => shared::differences(&root, &scenario, "rust"),
            Err(error) => vec![format!("rust {}: refused: {error}", scenario.kind)],
        };
        found.insert(scenario.id, lines);
    }
    shared::check_expected("rust", found);
}
