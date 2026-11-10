//! Conceptual model conformance, census Q2 ("what is a file's page name"):
//! the computers of that answer that live in `tine-graph-features`, run on
//! every fixture of `crates/tine-store/tests/fixtures/cm-conformance/`
//! against the store's answer (`shared::answered_name`, the inventory:
//! `model/page_identity.rs` `effective_page_name` family). No bridge needed,
//! so this runs in every gate.
//!
//! - `conflicts::list_sync_conflicts` names the page a sync-conflict copy
//!   shadows (`conflicts.rs sync_copy_base_name`, through `Store::page_of`
//!   since batch 3; before, `graph_text_title` decoded the stem);
//! - `conflicts.rs dto`, the `PageDto` a merged or reverted page is handed
//!   back in (here through `run_revert::revert_page` with no steps; the live
//!   conflict merge and the sync-copy fold build theirs the same way).
//!
//! The store harness (`tine-store` `cm_conformance_tests::file_name_answerers`)
//! runs the store crate's own computers and the OG oracle. Differences: part
//! `conflictNames` of `expected-differences.json`; `CM_REFRESH=1` rewrites it.

use std::fs;

use serde_json::Value;
use tine_graph_features::{conflicts, run_revert};
use tine_store::{PageId, Store};

use tine_store as store_crate;

#[path = "../../tine-store/src/cm_conformance_shared_tests.rs"]
mod shared;

use shared::{Diff, GraphSpec, Section};

fn fixture_specs() -> Vec<GraphSpec> {
    let dir = shared::fixture_dir(env!("CARGO_MANIFEST_DIR"));
    let mut paths: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.extension().is_some_and(|x| x == "json")
                && p.file_name()
                    .is_some_and(|n| n != "expected-differences.json")
        })
        .collect();
    paths.sort();
    paths
        .iter()
        .map(|p| {
            let v: Value = serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap();
            GraphSpec::from_fixture(&p.file_stem().unwrap().to_string_lossy(), &v)
        })
        .collect()
}

#[test]
fn conflict_and_merge_names() {
    let _integration_guard = crate::integration_lock();
    let mut section = Section::default();
    let mut keep = Vec::new();
    for spec in fixture_specs() {
        let dir = tempfile::tempdir().unwrap();
        spec.write(dir.path());
        if !spec.listed_apart(dir.path()) {
            section.skip_on_volume(&spec.name, "");
            continue;
        }
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        let view = store.whole_graph().expect("graph loads");
        let mut diffs = Vec::new();
        for copy in conflicts::list_sync_conflicts(&store).unwrap() {
            let Some(base) = &copy.base_path else {
                continue;
            };
            let answer = shared::answered_name(&view, base);
            if answer.as_deref() != Some(copy.base_name.as_str()) {
                diffs.push(Diff::new(
                    "file name: conflicts.rs sync-copy base name differs from the winner's name",
                    format!(
                        "{}: base name {:?}, {base} is named {answer:?}",
                        copy.path, copy.base_name
                    ),
                ));
            }
        }
        for (rel, text) in &spec.files {
            let Some(answer) = shared::answered_name(&view, rel) else {
                continue;
            };
            let reverted =
                match run_revert::revert_page(&store, &PageId::from(rel.as_str()), text, &[]) {
                    Ok(r) => r,
                    Err(e) => {
                        diffs.push(Diff::new(
                            "harness: revert_page refused",
                            format!("harness: {rel}: {e:?}"),
                        ));
                        continue;
                    }
                };
            for (field, got) in [("name", &reverted.doc.name), ("title", &reverted.doc.title)] {
                if *got != answer {
                    diffs.push(Diff::new(
                        format!("file name: conflicts.rs dto {field} differs from the answer"),
                        format!(
                            "{rel}: merged/reverted PageDto {field} {got:?}, answer {answer:?}"
                        ),
                    ));
                }
            }
        }
        diffs.sort();
        diffs.dedup();
        if !diffs.is_empty() {
            section.fixtures.insert(
                spec.name.clone(),
                diffs.into_iter().map(|d| d.detail).collect(),
            );
        }
        drop(view);
        keep.push(std::thread::spawn(move || drop((store, dir))));
    }
    for h in keep {
        h.join().unwrap();
    }
    shared::check_expected(
        env!("CARGO_MANIFEST_DIR"),
        "conflictNames",
        &section,
        "CM_REFRESH=1 cargo test -p tine-graph-features --test integration cm_conformance_names",
    );
}
