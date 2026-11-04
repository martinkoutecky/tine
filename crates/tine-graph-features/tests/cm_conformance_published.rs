//! Conceptual model conformance, the published site (CM step 2, census Q13):
//! every cm-conformance fixture graph (`crates/tine-store/tests/fixtures/
//! cm-conformance/`) published by the static export (`publish_live`, all
//! pages), reduced to what the name answers read: the pages without their
//! blocks, the entries, the `aliases` pairs and `answers`. The viewer test
//! `src/publishedConformance.test.ts` installs `publishedBackend` over each and
//! compares the published click path and `getPage` with the fixture's frontend
//! case (the live store's answers over the whole graph).
//!
//! Written to `tests/fixtures/published-answers/conformance/<fixture>.json`;
//! a stale file fails here. `CM_REFRESH=1` rewrites them. A fixture the static
//! export refuses is written as `{"refused": "<reason>"}`.

use std::fs;
use std::path::Path;

use serde_json::{json, Value};
use tine_graph_features::publish_query::publish_live;
use tine_store::Store;

use tine_store as store_crate;

#[path = "../../tine-store/src/cm_conformance_shared_tests.rs"]
mod shared;

use shared::GraphSpec;

fn bundle() -> Vec<(String, Vec<u8>)> {
    vec![(
        "index.html".into(),
        b"<!doctype html><html><head><title>Tine</title></head><body></body></html>".to_vec(),
    )]
}

/// The snapshot's name-answer parts (blocks and baked answers dropped).
fn reduced(snapshot: &Value) -> Value {
    let pages: Vec<Value> = snapshot["pages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|page| {
            json!({ "name": page["name"], "kind": page["kind"], "title": page["title"],
                "path": page["path"], "format": page["format"], "pre_block": null,
                "blocks": [], "read_only": true })
        })
        .collect();
    json!({ "schema": snapshot["schema"], "name": snapshot["name"], "exported_at": "",
        "home": snapshot["home"], "pages": pages, "entries": snapshot["entries"],
        "backlinks": {}, "block_ref_counts": {}, "aliases": snapshot["aliases"], "icons": {},
        "queries": [], "answers": snapshot["answers"] })
}

#[test]
fn published_conformance_snapshots_match_the_writer() {
    let _integration_guard = crate::integration_lock();
    let refresh = std::env::var_os("CM_REFRESH").is_some();
    let source = shared::fixture_dir(env!("CARGO_MANIFEST_DIR"));
    let out = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/published-answers/conformance");
    let mut paths: Vec<_> = fs::read_dir(&source)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.extension().is_some_and(|x| x == "json")
                && p.file_name()
                    .is_some_and(|n| n != "expected-differences.json")
        })
        .collect();
    paths.sort();
    let mut stale = Vec::new();
    for path in paths {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let v: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let spec = GraphSpec::from_fixture(&name, &v);
        let root = tempfile::tempdir().unwrap();
        let graph = root.path().join("graph");
        spec.write(&graph);
        let output = root.path().join("output");
        fs::create_dir_all(&output).unwrap();
        let store = Store::open(&graph, Default::default()).unwrap().0;
        let value = match publish_live(&store, &output, "Site", true, &bundle()) {
            Ok(_) => {
                let snapshot: Value = serde_json::from_slice(
                    &fs::read(output.join("site/app/snapshot.json")).unwrap(),
                )
                .unwrap();
                reduced(&snapshot)
            }
            Err(error) => json!({ "refused": error.to_string() }),
        };
        store.close();
        let text = serde_json::to_string_pretty(&value).unwrap() + "\n";
        let file = out.join(format!("{name}.json"));
        if refresh {
            fs::create_dir_all(&out).unwrap();
            fs::write(&file, &text).unwrap();
        } else if fs::read_to_string(&file).unwrap_or_default() != text {
            stale.push(name);
        }
    }
    assert!(
        stale.is_empty(),
        "published conformance snapshots are stale: {stale:?}; rerun with CM_REFRESH=1 and review"
    );
}
