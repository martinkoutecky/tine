//! Conceptual model step 2, batch 4 (census Q12, harness gap H1): favorites
//! after a CLI rename through the headless owner (`tine serve`: no window, so
//! no TypeScript runs after the verb). The verb is `page.rename`
//! (`owner/pages.rs rename_named_page`), which has no merge: a merge scenario
//! records its refusal. Scenarios, target and pinned differences are shared
//! with the Rust-entry check (`cm_favorites_shared.rs`).

use std::collections::BTreeMap;

use super::*;

#[path = "../../../../crates/tine-graph-features/tests/cm_favorites_shared.rs"]
mod shared;

/// An owner test graph holding only the scenario's files.
fn scenario_graph(scenario: &shared::Scenario) -> Graph {
    let dir = tempfile::tempdir().unwrap();
    let graph = dir.path().join("graph");
    shared::write_graph(&graph, scenario);
    let app_data = dir.path().join("app");
    let owners = dir.path().join("owners");
    std::fs::create_dir_all(&app_data).unwrap();
    std::fs::create_dir_all(&owners).unwrap();
    let store = Arc::new(Store::open(&graph, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    let root = Store::canonical_root(&graph).unwrap();
    Graph {
        _dir: dir,
        root,
        app_data,
        owners,
        store,
    }
}

#[test]
fn favorites_after_a_headless_cli_rename() {
    let mut found = BTreeMap::new();
    for scenario in shared::scenarios() {
        let graph = scenario_graph(&scenario);
        let slot = graph.slot();
        assert!(
            slot.window.get().is_none(),
            "the headless owner has no window"
        );
        let result = verb(
            &slot,
            "page.rename",
            json!({"old": scenario.old, "new": scenario.new}),
        );
        let lines = match result {
            Ok(_) => shared::differences(&graph.root, &scenario, "owner"),
            Err(body) => vec![format!(
                "owner {}: refused ({}): {}",
                scenario.kind, body.code, body.message
            )],
        };
        drop(slot);
        found.insert(scenario.id, lines);
    }
    shared::check_expected("owner", found);
}
