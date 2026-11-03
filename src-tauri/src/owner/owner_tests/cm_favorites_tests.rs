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

/// A title:: change through the owner's one page-write function
/// (`write::write_page_from_bytes`, which every owner page write and `git
/// run revert` use). No verb edits the page-properties block today (probed:
/// `block.edit`/`block.prop` address only body blocks), so this drives the
/// function the way a revert of a title change reaches it.
fn retitle(
    slot: &crate::state::GraphSlot,
    scenario: &shared::Scenario,
) -> Result<Value, ErrorBody> {
    let page = PageId::from(scenario.path.as_deref().expect("a retitle names its file"));
    let text = scenario.text.clone().unwrap();
    let store = Arc::clone(&slot.store);
    super::super::write::write_page_from_bytes(
        slot,
        &page,
        tine_store::EditKind::ReplacePage,
        None,
        false,
        |_, page, _| {
            Ok(super::super::write::Planned {
                doc: store.page_of(page, text.as_bytes()).unwrap().doc,
                changed: Vec::new(),
            })
        },
    )
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
        let result = if scenario.kind == "retitle" {
            retitle(&slot, &scenario)
        } else {
            verb(
                &slot,
                "page.rename",
                json!({"old": scenario.old, "new": scenario.new}),
            )
        };
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
