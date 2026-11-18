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

/// Every `[[spelling]]` page reference written in the exported files (the
/// fixture publishes all pages): the names a reader of the site can click.
fn linked_spellings(spec: &GraphSpec) -> Vec<String> {
    let mut out = Vec::new();
    for text in spec.files.values() {
        let mut rest = text.as_str();
        while let Some(start) = rest.find("[[") {
            rest = &rest[start + 2..];
            let Some(end) = rest.find("]]") else { break };
            let inner = &rest[..end];
            // `[[file:…][label]]` and `[[target][label]]` are Org links, not
            // page references.
            if !inner.is_empty()
                && !inner.contains("[[")
                && !inner.contains("][")
                && !inner.starts_with("file:")
            {
                out.push(inner.to_owned());
            }
            rest = &rest[end + 2..];
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The published answers cover every spelling the exported content links: each
/// has an inventory row of its own (keyed as the store keys it), so the site
/// answers it from the store's row and never by a guess of the viewer
/// (CM step 2, Q13; manager 2026-11-04). A spelling that only a visitor types
/// (a hand-written `#/page/…` address) is not in the content and has no row.
fn assert_linked_spellings_have_rows(
    name: &str,
    spec: &GraphSpec,
    snapshot: &Value,
    missing: &mut Vec<String>,
    checked: &mut usize,
) {
    let rows = snapshot["answers"]["inventory"].as_array().unwrap();
    for spelling in linked_spellings(spec) {
        *checked += 1;
        let key = tine_core::refs::page_key(&spelling);
        if !rows.iter().any(|row| row["key"] == key.as_str()) {
            missing.push(format!("{name}: [[{spelling}]] (key {key:?})"));
        }
    }
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
    let mut missing = Vec::new();
    let mut checked = 0;
    for path in paths {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let v: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let spec = GraphSpec::from_fixture(&name, &v);
        let root = tempfile::tempdir().unwrap();
        let graph = root.path().join("graph");
        spec.write(&graph);
        if !spec.listed_apart(&graph) {
            // This volume folds two of its file names into one: the committed
            // snapshot answers a graph that cannot exist here.
            eprintln!(
                "SKIP on this volume: fixture {name}: two of its file names name one file here"
            );
            continue;
        }
        let output = root.path().join("output");
        fs::create_dir_all(&output).unwrap();
        let store = Store::open(&graph, Default::default()).unwrap().0;
        let value = match publish_live(&store, &output, "Site", true, true, &bundle()) {
            Ok(_) => {
                let snapshot: Value = serde_json::from_slice(
                    &fs::read(output.join("site/app/snapshot.json")).unwrap(),
                )
                .unwrap();
                assert_linked_spellings_have_rows(
                    &name,
                    &spec,
                    &snapshot,
                    &mut missing,
                    &mut checked,
                );
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
    // 110 linked spellings over 62 fixtures on 2026-11-04.
    assert!(checked >= 100, "only {checked} linked spellings checked");
    assert!(
        missing.is_empty(),
        "linked spellings with no published inventory row: {missing:?}"
    );
    assert!(
        stale.is_empty(),
        "published conformance snapshots are stale: {stale:?}; rerun with CM_REFRESH=1 and review"
    );
}

/// REG-CM-YEARLESS-DAY-KEY (round 4, review item 4): the published site
/// names a year-less day as the app does. Its page DTO and entry are named by
/// the day's handle (`2026_11_02`) with its label `11-02` as the title, its
/// reference panel is keyed by that name and holds its own references, and a
/// day's blocks stay in another page's panel. Before the fix the DTO was
/// named `11-02`, so the viewer loaded the label's slot while the route
/// rendered the handle's (a blank day), and the backlink closure dropped the
/// journal sources. Markdown and Org, journals/ and pages/ day files, label
/// page present and absent.
#[test]
fn a_published_year_less_day_is_named_by_its_handle() {
    let _integration_guard = crate::integration_lock();
    for (day_rel, day_text) in [
        ("journals/2026_11_02.md", "- the day [[Topic]]\n"),
        ("journals/2026_11_02.org", "* the day [[Topic]]\n"),
        (
            "pages/Day.md",
            "title:: 2026-11-02\n\n- the day [[Topic]]\n",
        ),
    ] {
        for label_page in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let graph = root.path().join("graph");
            let mut files = vec![
                (
                    "logseq/config.edn",
                    "{:journal/page-title-format \"MM-dd\"}\n",
                ),
                (day_rel, day_text),
                ("journals/2025_11_02.md", "- last year [[Topic]]\n"),
                (
                    "pages/Notes.md",
                    "- day [[2026-11-02]]\n- label [[11-02]]\n",
                ),
                ("pages/Topic.md", "- topic\n"),
            ];
            if label_page {
                files.push(("pages/11-02.md", "- a label page [[Topic]]\n"));
            }
            for (rel, text) in &files {
                let path = graph.join(rel);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, text).unwrap();
            }
            let case = format!("{day_rel}, label page {label_page}");
            let output = root.path().join("output");
            fs::create_dir_all(&output).unwrap();
            let store = Store::open(&graph, Default::default()).unwrap().0;
            publish_live(&store, &output, "Site", true, true, &bundle()).expect(&case);
            // A query export owns each result group by the name the group
            // gives; the day's host page is the day.
            let request = tine_graph_features::publish_query::QueryExportRequest {
                argument: "[[Topic]]".into(),
                dialect: tine_core::query::wire_parse::QueryTextDialect::MacroQuery,
                properties: Vec::new(),
                current_page: Some("2026_11_02".into()),
                name: "Topic export".into(),
                host_block_id: None,
                folder: Some("probe".into()),
                replace: false,
                asset_budget_bytes: None,
                include_query_text: false,
                include_comments: None,
            };
            let plan =
                tine_graph_features::publish_query::plan_query(&store, &request).expect(&case);
            let mut exported: Vec<_> = plan.pages.iter().map(|page| page.path.as_str()).collect();
            exported.sort();
            assert!(
                exported.contains(&day_rel) && exported.contains(&"journals/2025_11_02.md"),
                "{case}: {exported:?}"
            );
            assert!(plan.selects_host_page, "{case}");
            store.close();
            let snapshot: Value =
                serde_json::from_slice(&fs::read(output.join("site/app/snapshot.json")).unwrap())
                    .unwrap();
            let named = |list: &str, path: &str| {
                (snapshot[list].as_array().unwrap().iter())
                    .find(|page| page["path"] == path)
                    .map(|page| (page["name"].clone(), page.get("title").cloned()))
            };
            for (path, handle) in [
                (day_rel, "2026_11_02"),
                ("journals/2025_11_02.md", "2025_11_02"),
            ] {
                assert_eq!(
                    named("pages", path),
                    Some((json!(handle), Some(json!("11-02")))),
                    "{case}: {path} page"
                );
                assert_eq!(
                    named("entries", path).map(|(name, _)| name),
                    Some(json!(handle)),
                    "{case}"
                );
            }
            let panel = |name: &str| -> Vec<(String, String)> {
                let mut rows: Vec<_> =
                    (snapshot["backlinks"][name].as_array().into_iter().flatten())
                        .flat_map(|group| {
                            (group["blocks"].as_array().unwrap().iter()).map(move |block| {
                                (
                                    group["page"].as_str().unwrap().to_owned(),
                                    block["raw"].as_str().unwrap().to_owned(),
                                )
                            })
                        })
                        .collect();
                rows.sort();
                rows
            };
            let mut topic = vec![
                ("2025_11_02".to_owned(), "last year [[Topic]]".to_owned()),
                ("2026_11_02".to_owned(), "the day [[Topic]]".to_owned()),
            ];
            if label_page {
                topic.push(("11-02".to_owned(), "a label page [[Topic]]".to_owned()));
            }
            topic.sort();
            assert_eq!(panel("Topic"), topic, "{case}: Topic panel");
            assert_eq!(
                panel("2026_11_02"),
                [("Notes".to_owned(), "day [[2026-11-02]]".to_owned())],
                "{case}: the day's panel"
            );
            if label_page {
                assert_eq!(
                    panel("11-02"),
                    [("Notes".to_owned(), "label [[11-02]]".to_owned())],
                    "{case}: the label page's panel"
                );
            }
        }
    }
}
