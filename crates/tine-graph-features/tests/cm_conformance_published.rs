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

/// REG-CM-YEARLESS-DAY-KEY round 5 (review R2-1): publication names each
/// source file by its one operational name wherever it admits, slugs, looks
/// up, homes or bounds sheets, so under `MM-dd` two years of `11-02` and the
/// label page `11-02` are three pages, each exported, and a day is a home by
/// its handle. Markdown/Org, `journals/`/`pages/`, one or two years, the
/// label page present or absent.
#[test]
fn publication_admits_homes_and_bounds_sheets_by_a_year_less_days_handle() {
    use tine_graph_features::publish::{sheet_export_inputs, SheetScope};
    use tine_graph_features::publish_query::{
        plan_query, publish_live_home, publish_query, QueryExportRequest,
    };
    let _integration_guard = crate::integration_lock();
    // Every failing check, so one run shows the whole family.
    let mut failures: Vec<String> = Vec::new();
    let snapshot_of = |site: &Path| -> Value {
        serde_json::from_slice(&fs::read(site.join("app/snapshot.json")).unwrap()).unwrap()
    };
    let entries = |snapshot: &Value, name: &str| -> Vec<String> {
        (snapshot["entries"].as_array().unwrap().iter())
            .filter(|entry| entry["name"] == name)
            .map(|entry| entry["path"].as_str().unwrap().to_owned())
            .collect()
    };
    for (day_rel, day_text) in [
        (
            "journals/2026_11_02.md",
            "- marker-this-year [[Topic]]\n  id:: 6566a1b2-0000-4000-8000-000000000001\n",
        ),
        (
            "journals/2026_11_02.org",
            "* marker-this-year [[Topic]]\n:PROPERTIES:\n:id: 6566a1b2-0000-4000-8000-000000000001\n:END:\n",
        ),
        (
            "pages/Day.md",
            "title:: 2026-11-02\n\n- marker-this-year [[Topic]]\n  id:: 6566a1b2-0000-4000-8000-000000000001\n",
        ),
    ] {
        for (two_years, label_page) in [(true, true), (true, false), (false, true), (false, false)]
        {
            let case = format!("{day_rel}, two years {two_years}, label page {label_page}");
            let root = tempfile::tempdir().unwrap();
            let graph = root.path().join("graph");
            let mut files = vec![
                (
                    "logseq/config.edn",
                    "{:journal/page-title-format \"MM-dd\"}\n",
                ),
                (day_rel, day_text),
                ("pages/Topic.md", "- topic\n"),
                (
                    "pages/Sheet.md",
                    "- {{query [[Topic]]}}\n  tine.view:: table\n",
                ),
                ("pages/Embed.md", "- {{embed ((6566a1b2-0000-4000-8000-000000000001))}}\n"),
            ];
            if two_years {
                files.push(("journals/2025_11_02.md", "- marker-last-year [[Topic]]\n"));
            }
            if label_page {
                files.push(("pages/11-02.md", "- marker-label-page [[Topic]]\n"));
            }
            for (rel, text) in &files {
                let path = graph.join(rel);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, text).unwrap();
            }
            let store = Store::open(&graph, Default::default()).unwrap().0;
            let output = root.path().join("output");
            fs::create_dir_all(&output).unwrap();

            // Admission and slugs: every page reaches the static HTML.
            let site = Path::new(
                &publish_live(&store, &output, "Site", true, true, &bundle())
                    .expect(&case)
                    .path,
            )
            .to_owned();
            let html: String = (fs::read_dir(&site).unwrap())
                .map(|file| file.unwrap().path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "html"))
                .map(|path| fs::read_to_string(path).unwrap())
                .collect();
            for (marker, present) in [
                ("marker-this-year", true),
                ("marker-last-year", two_years),
                ("marker-label-page", label_page),
                ("Embedded content is not public", false),
                ("Embedded block not found", false),
            ] {
                if html.contains(marker) != present {
                    failures.push(format!("{case}: static HTML {marker} present {present}"));
                }
            }
            // The default home names an exported page.
            let snapshot = snapshot_of(&site);
            let home = snapshot["home"].as_str().unwrap().to_owned();
            if entries(&snapshot, &home).len() != 1 {
                failures.push(format!(
                    "{case}: default home {home:?} names no exported page"
                ));
            }

            // An explicit home by the day's handle.
            let homed = root.path().join("homed");
            fs::create_dir_all(&homed).unwrap();
            match publish_live_home(
                &store,
                &homed,
                "Site",
                true,
                true,
                Some("2026_11_02"),
                &bundle(),
            ) {
                Ok(receipt) => {
                    let snapshot = snapshot_of(Path::new(&receipt.path));
                    if snapshot["home"] != "2026_11_02"
                        || entries(&snapshot, "2026_11_02") != [day_rel]
                    {
                        failures.push(format!("{case}: explicit home {}", snapshot["home"]));
                    }
                }
                Err(error) => failures.push(format!("{case}: explicit home refused: {error}")),
            }

            // A query export's home never takes the day's name.
            let request = QueryExportRequest {
                argument: "[[Topic]]".into(),
                dialect: tine_core::query::wire_parse::QueryTextDialect::MacroQuery,
                properties: Vec::new(),
                current_page: None,
                name: "2026_11_02".into(),
                host_block_id: None,
                folder: Some("probe".into()),
                replace: false,
                asset_budget_bytes: None,
                include_query_text: false,
                include_comments: None,
            };
            let plan = plan_query(&store, &request).expect(&case);
            let site = publish_query(&store, &request, &plan.fingerprint, &bundle())
                .expect(&case)
                .path;
            let snapshot = snapshot_of(Path::new(&site));
            // Re-planning compares the export's membership by the names its
            // snapshot gives: the day is the same page, not one gone and one new.
            let replan = plan_query(&store, &request).expect(&case);
            let mut planned: Vec<_> = replan.pages.iter().map(|page| page.name.clone()).collect();
            let mut previous = replan.previous_pages.clone().unwrap_or_default();
            planned.sort();
            previous.sort();
            if planned != previous || !planned.iter().any(|name| name == "2026_11_02") {
                failures.push(format!(
                    "{case}: membership {planned:?} vs before {previous:?}"
                ));
            }
            let owners = entries(&snapshot, "2026_11_02");
            if snapshot["home"] == "2026_11_02" || owners != [day_rel] {
                failures.push(format!(
                    "{case}: query home {} beside the day: entries {owners:?}",
                    snapshot["home"]
                ));
            }

            // A query-backed sheet keeps the selected days' rows.
            let mut days = vec!["2026_11_02"];
            if two_years {
                days.push("2025_11_02");
            }
            for scope in [
                SheetScope::Query { request },
                SheetScope::Live {
                    all_pages: true,
                    include_comments: None,
                },
            ] {
                let inputs = sheet_export_inputs(&store, Some(&["Sheet".to_owned()]), Some(&scope))
                    .expect(&case);
                let pages = inputs[0].query.as_ref().map(|query| query.pages.clone());
                let pages = pages.unwrap_or_default();
                if !days.iter().all(|day| pages.iter().any(|page| page == day)) {
                    failures.push(format!("{case}: sheet rows {pages:?} miss {days:?}"));
                }
            }
            store.close();
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// REG-CM-YEARLESS-DAY-KEY round 5b: print and PDF export find a page by the
/// operational name the app passes, so under `MM-dd` a day prints by its
/// handle (`2026_11_02`), never another year's day or the label page `11-02`;
/// the label page prints by its own name; a printed embed of a block on a
/// year-less day finds it; and a printed day's sheet inputs are its own.
/// Markdown/Org, one or two years, the label page present.
#[test]
fn print_and_pdf_find_a_year_less_day_by_its_handle() {
    use tine_graph_features::print::{page_print_html, PrintOpts};
    use tine_graph_features::publish::{sheet_export_inputs, SheetScope};
    let _integration_guard = crate::integration_lock();
    let mut failures: Vec<String> = Vec::new();
    let uuid = "6566a1b2-0000-4000-8000-000000000002";
    for org in [false, true] {
        for two_years in [false, true] {
            let case = format!("org {org}, two years {two_years}");
            let root = tempfile::tempdir().unwrap();
            let graph = root.path().join("graph");
            let day = if org {
                format!(
                    "* marker-this-year\n:PROPERTIES:\n:id: {uuid}\n:END:\n* {{{{query [[Topic]]}}}}\n:PROPERTIES:\n:tine.view: table\n:END:\n"
                )
            } else {
                format!(
                    "- marker-this-year\n  id:: {uuid}\n- {{{{query [[Topic]]}}}}\n  tine.view:: table\n"
                )
            };
            let ext = if org { "org" } else { "md" };
            let mut files = vec![
                (
                    "logseq/config.edn".to_owned(),
                    "{:journal/page-title-format \"MM-dd\"}\n".to_owned(),
                ),
                (format!("journals/2026_11_02.{ext}"), day),
                (
                    "pages/11-02.md".to_owned(),
                    "- marker-label-page\n".to_owned(),
                ),
                ("pages/Topic.md".to_owned(), "- topic\n".to_owned()),
                (
                    "pages/Notes.md".to_owned(),
                    "- a note [[Topic]]\n".to_owned(),
                ),
                (
                    "pages/Embed.md".to_owned(),
                    format!("- {{{{embed (({uuid}))}}}}\n"),
                ),
            ];
            if two_years {
                files.push((
                    "journals/2025_11_02.md".to_owned(),
                    "- marker-last-year\n- {{query [[Topic]]}}\n  tine.view:: table\n".to_owned(),
                ));
            }
            for (rel, text) in &files {
                let path = graph.join(rel);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, text).unwrap();
            }
            let store = Store::open(&graph, Default::default()).unwrap().0;
            let print = |name: &str| page_print_html(&store, name, PrintOpts::default()).unwrap();
            for (name, present, absent) in [
                (
                    "2026_11_02",
                    "marker-this-year",
                    ["marker-last-year", "marker-label-page"],
                ),
                (
                    "11-02",
                    "marker-label-page",
                    ["marker-this-year", "marker-last-year"],
                ),
            ] {
                match print(name) {
                    Some(html) => {
                        if !html.contains(present) || absent.iter().any(|m| html.contains(m)) {
                            failures.push(format!("{case}: print {name} is not that page"));
                        }
                    }
                    None => failures.push(format!("{case}: print {name} found nothing")),
                }
            }
            match print("Embed") {
                Some(html) if html.contains("marker-this-year") => {}
                _ => failures.push(format!("{case}: printed embed of the day's block")),
            }
            let names = ["2026_11_02".to_owned()];
            let scope = SheetScope::Print {
                include_comments: None,
            };
            let inputs = sheet_export_inputs(&store, Some(&names), Some(&scope)).unwrap();
            let pages: Vec<_> = inputs.iter().map(|input| input.page.as_str()).collect();
            if pages != ["2026_11_02"] {
                failures.push(format!("{case}: PDF sheet inputs {pages:?}"));
            }
            store.close();
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// REG-CM-YEARLESS-DAY-KEY round 5b: a published page query's row for a
/// year-less day links by the day's handle and shows its label, as the app
/// shows it (`pageLabel`).
#[test]
fn a_published_page_query_row_shows_a_year_less_days_label() {
    let _integration_guard = crate::integration_lock();
    let root = tempfile::tempdir().unwrap();
    let graph = root.path().join("graph");
    for (rel, text) in [
        (
            "logseq/config.edn",
            "{:journal/page-title-format \"MM-dd\"}\n",
        ),
        (
            "journals/2026_11_02.md",
            "type:: day\n\n- marker-this-year\n",
        ),
        ("pages/Q.md", "- {{query (page-property type day)}}\n"),
    ] {
        let path = graph.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let store = Store::open(&graph, Default::default()).unwrap().0;
    let out = root.path().join("site");
    fs::create_dir_all(&out).unwrap();
    let site = publish_live(&store, &out, "Site", true, true, &bundle())
        .unwrap()
        .path;
    let html: String = fs::read_dir(&site)
        .unwrap()
        .map(|file| file.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "html"))
        .map(|path| fs::read_to_string(path).unwrap())
        .filter(|text| text.contains("query-results"))
        .collect();
    let row = html
        .split("<ul class=\"query-results\">")
        .nth(1)
        .and_then(|rest| rest.split("</ul>").next())
        .unwrap_or_default()
        .to_owned();
    assert!(
        row.contains("data-ref=\"2026_11_02\">11-02</a>"),
        "the row links the handle and shows the label: {row}"
    );
    store.close();
}

/// REG-CM-YEARLESS-DAY-KEY round 5b: a published or printed link written as
/// a date opens the day it names, as in the app: under `MM-dd`,
/// `[[2026-11-02]]` and `#[[2026-11-02]]` link the 2026 day's page and
/// `{{embed [[2026-11-02]]}}` embeds that day, never another year's; the
/// label `[[11-02]]` without its page stays unlinked.
#[test]
fn published_and_printed_date_links_open_a_year_less_day() {
    let _integration_guard = crate::integration_lock();
    let mut failures: Vec<String> = Vec::new();
    let root = tempfile::tempdir().unwrap();
    let graph = root.path().join("graph");
    for (rel, text) in [
        (
            "logseq/config.edn",
            "{:journal/page-title-format \"MM-dd\"}\n",
        ),
        ("journals/2026_11_02.md", "- marker-this-year\n"),
        ("journals/2025_11_02.md", "- marker-last-year\n"),
        (
            "pages/Notes.md",
            "- see [[2026-11-02]] and #[[2026-11-02]] and [[11-02]]\n",
        ),
        ("pages/PE.md", "- {{embed [[2026-11-02]]}}\n"),
    ] {
        let path = graph.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let store = Store::open(&graph, Default::default()).unwrap().0;
    let out = root.path().join("site");
    fs::create_dir_all(&out).unwrap();
    let site = publish_live(&store, &out, "Site", true, true, &bundle())
        .unwrap()
        .path;
    let page = |marker: &str| -> String {
        fs::read_dir(&site)
            .unwrap()
            .map(|file| file.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "html"))
            .map(|path| fs::read_to_string(path).unwrap())
            .filter(|text| text.contains(marker))
            .collect()
    };
    let notes = page("see ");
    for (needle, present) in [
        (
            "<a class=\"ref\" href=\"2026-11-02.html\" data-ref=\"2026_11_02\">",
            true,
        ),
        (
            "<a class=\"tag\" href=\"2026-11-02.html\" data-ref=\"2026_11_02\">",
            true,
        ),
        ("data-ref=\"2026-11-02\"", false),
        ("<span class=\"ref ref-outside\" data-ref=\"11-02\">", true),
    ] {
        if notes.contains(needle) != present {
            failures.push(format!("published links: {needle} present {present}"));
        }
    }
    let embed = page("page-embed");
    if !embed.contains("marker-this-year") || embed.contains("marker-last-year") {
        failures.push("published page embed is not the 2026 day".into());
    }
    match tine_graph_features::print::page_print_html(&store, "PE", Default::default()).unwrap() {
        Some(html) if html.contains("marker-this-year") && !html.contains("marker-last-year") => {}
        _ => failures.push("printed page embed is not the 2026 day".into()),
    }
    store.close();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// REG-CM-YEARLESS-DAY-KEY round 5b: a query exported from a year-less day
/// (the page's first query, or a block by UUID) runs with that day as its
/// current page, named by its handle, so `<% current page %>` is the day, not
/// the label page `11-02`.
#[test]
fn a_query_exported_from_a_year_less_day_has_that_day_as_its_current_page() {
    use tine_graph_features::publish_query::{query_request_by_uuid, query_request_on_page};
    let _integration_guard = crate::integration_lock();
    let root = tempfile::tempdir().unwrap();
    let graph = root.path().join("graph");
    for (rel, text) in [
        (
            "logseq/config.edn",
            "{:journal/page-title-format \"MM-dd\"}\n",
        ),
        (
            "journals/2026_11_02.md",
            "- {{query [[<% current page %>]]}}\n  id:: 6566a1b2-0000-4000-8000-000000000003\n",
        ),
        ("pages/11-02.md", "- label page\n"),
    ] {
        let path = graph.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let store = Store::open(&graph, Default::default()).unwrap().0;
    let view = store.whole_graph().unwrap();
    let store_crate::Resolved::Existing { id, .. } = view.resolve("2026_11_02", true) else {
        panic!("the day's file");
    };
    let on_page = query_request_on_page(&view, &id, None, false).unwrap();
    let by_uuid =
        query_request_by_uuid(&store, "6566a1b2-0000-4000-8000-000000000003", None, false).unwrap();
    assert_eq!(
        (
            on_page.current_page.as_deref(),
            by_uuid.current_page.as_deref()
        ),
        (Some("2026_11_02"), Some("2026_11_02"))
    );
    drop(view);
    store.close();
}

/// REG-CM-YEARLESS-DAY-KEY round 5b: a Quick Share of one year-less day is
/// that day: its home names the day's entry by its handle, and the share is
/// titled by the day's label.
#[test]
fn a_quick_share_of_one_year_less_day_is_homed_on_that_day() {
    use tine_graph_features::publish_query::QueryExportRequest;
    use tine_graph_features::quick_share;
    let _integration_guard = crate::integration_lock();
    let root = tempfile::tempdir().unwrap();
    let graph = root.path().join("graph");
    for (rel, text) in [
        (
            "logseq/config.edn",
            "{:journal/page-title-format \"MM-dd\"}\n",
        ),
        ("journals/2026_11_02.md", "- marker-this-year\n"),
        ("journals/2025_11_02.md", "- marker-last-year\n"),
    ] {
        let path = graph.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let store = Store::open(&graph, Default::default()).unwrap().0;
    let scratch = root.path().join("scratch");
    fs::create_dir_all(&scratch).unwrap();
    let request = QueryExportRequest {
        argument: "(page [[2026-11-02]])".into(),
        dialect: tine_core::query::wire_parse::QueryTextDialect::MacroQuery,
        properties: Vec::new(),
        current_page: None,
        name: "Ignored".into(),
        host_block_id: None,
        folder: None,
        replace: false,
        asset_budget_bytes: None,
        include_query_text: false,
        include_comments: None,
    };
    let prepared =
        quick_share::prepare(&store, &request, "2026-11-19T00:00:00Z", &scratch).unwrap();
    let doc: Value = serde_json::from_slice(
        &quick_share::gunzip_bounded(&prepared.payload, quick_share::MAX_DOCUMENT_BYTES).unwrap(),
    )
    .unwrap();
    let names: Vec<_> = (doc["snapshot"]["pages"].as_array().unwrap().iter())
        .map(|page| page["name"].clone())
        .collect();
    assert_eq!(
        (
            &doc["snapshot"]["home"],
            names,
            prepared.preview.title.as_str()
        ),
        (&json!("2026_11_02"), vec![json!("2026_11_02")], "11-02")
    );
    store.close();
}
