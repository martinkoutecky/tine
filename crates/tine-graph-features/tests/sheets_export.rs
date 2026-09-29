//! Static-export sheets (family 7). One fixture graph chains all three layers:
//! Rust `sheet_export_inputs` -> the app's TS evaluator (`src/sheet/staticExport.ts`,
//! `staticExport.chain.test.ts`) -> Rust layout. `fixtures/sheets/inputs.json` and
//! `exports.json` are the two hand-off records; each side asserts its half, so a
//! change to either wire shape fails in one place instead of in a published site.
//! Refresh with `BLESS_SHEETS=1` on the Rust test, then on the TS test.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tine_graph_features::publish::{sheet_export_inputs, SheetScope};
use tine_graph_features::publish_query::{publish_live, publish_live_with_sheets};
use tine_graph_features::SheetExport;
use tine_store::Store;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sheets")
}

/// Removes the scratch graph and output when a test ends.
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
impl std::ops::Deref for Scratch {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

fn open_fixture() -> (Scratch, Store) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let base = std::env::temp_dir().join(format!(
        "tine-sheets-export-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let graph = base.join("graph");
    fs::create_dir_all(graph.join("journals")).unwrap();
    fs::create_dir_all(base.join("output")).unwrap();
    copy(&fixtures().join("graph"), &graph);
    let store = Store::open(&graph, Default::default()).unwrap().0;
    (Scratch(base), store)
}

fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn bundle() -> Vec<(String, Vec<u8>)> {
    vec![
        ("index.html".into(), b"<!doctype html><html><head><title>Tine</title></head><body><div id=\"root\"></div></body></html>".to_vec()),
        ("assets/app.js".into(), b"console.log('app')".to_vec()),
    ]
}

fn sheets_html(base: &Path) -> String {
    fs::read_to_string(base.join("output/export/sheets.html")).unwrap()
}

#[test]
fn inputs_for_the_fixture_match_the_golden_handoff() {
    let (_base, store) = open_fixture();
    let inputs = sheet_export_inputs(&store, None, None).unwrap();
    let actual = serde_json::to_string_pretty(&inputs).unwrap() + "\n";
    let golden = fixtures().join("inputs.json");
    if std::env::var_os("BLESS_SHEETS").is_some() {
        fs::write(&golden, &actual).unwrap();
    }
    assert_eq!(
        actual,
        fs::read_to_string(&golden).unwrap(),
        "inputs.json is stale; BLESS_SHEETS=1"
    );
    // The Rust side names candidates by `tine.view` alone; whether the value is a sheet
    // view is the app's call (I-12), so the non-sheet block is still a candidate.
    assert_eq!(inputs.len(), 7);
    store.close();
}

fn exports() -> Vec<SheetExport> {
    serde_json::from_str(&fs::read_to_string(fixtures().join("exports.json")).unwrap()).unwrap()
}

#[test]
fn the_apps_export_lays_out_as_table_board_and_grid() {
    let (base, store) = open_fixture();
    publish_live_with_sheets(
        &store,
        &base.join("output"),
        "export",
        false,
        &bundle(),
        exports(),
    )
    .unwrap();
    let html = sheets_html(&base);
    // Table: header, formula column, footer aggregates, cells, escaped title text.
    assert!(html.contains("<table class=\"sheet-table\">"), "{html}");
    assert!(html.contains("<span class=\"sheet-formula-marker\">ƒ</span>total"));
    assert!(html.contains("<tfoot>") && html.contains("Sum"));
    assert!(html.contains("<span class=\"sheet-chip\">#x</span>"));
    assert!(
        !html.contains("<script>alert"),
        "row title must stay escaped"
    );
    assert!(html.contains("style=\"background:rgba(251,230,158,0.45)\""));
    // Board: one column per state, cards carry the block anchors.
    assert!(html.contains("<div class=\"sheet-board\">"));
    assert!(html.contains("class=\"sheet-board-col\"") && html.contains("Write tests"));
    // Grid: header row, footer sum of the Qty column (2 + 5).
    assert!(html.contains("<table class=\"sheet-grid\">") && html.contains("<thead>"));
    assert!(html.contains("<td><span class=\"sheet-agg-label\">Sum</span> 7</td>"));
    // A block whose tine.view is not a sheet view keeps its plain outline.
    assert!(html.contains("stays outline"));
    store.close();
}

#[test]
fn sheets_export_without_frontend_keeps_the_outline() {
    // Named divergence (Guide > Exporting): the CLI has no evaluator, so a sheet block
    // publishes as the plain outline of its rows, never as a table.
    let (base, store) = open_fixture();
    publish_live(&store, &base.join("output"), "export", false, &bundle()).unwrap();
    let html = sheets_html(&base);
    for gone in ["sheet-table", "sheet-board", "sheet-grid", "sheet-note"] {
        assert!(
            !html.contains(gone),
            "no-frontend export must not contain {gone}"
        );
    }
    for row in [
        "First",
        "Second",
        "Write tests",
        "Plain card",
        "stays outline",
    ] {
        assert!(html.contains(row), "outline row {row} must still publish");
    }
    store.close();
}

fn mutate(mut all: Vec<Value>, edit: impl Fn(&mut Value)) -> Vec<SheetExport> {
    all.iter_mut().for_each(edit);
    serde_json::from_value(Value::Array(all)).unwrap()
}

fn raw_exports() -> Vec<Value> {
    serde_json::from_str(&fs::read_to_string(fixtures().join("exports.json")).unwrap()).unwrap()
}

fn export_html(sheets: Vec<SheetExport>) -> String {
    let (base, store) = open_fixture();
    publish_live_with_sheets(
        &store,
        &base.join("output"),
        "export",
        false,
        &bundle(),
        sheets,
    )
    .expect("a bad sheet must never fail the export");
    let html = sheets_html(&base);
    store.close();
    html
}

#[test]
fn a_sheet_that_failed_to_compute_exports_the_outline_and_a_visible_note() {
    let html = export_html(mutate(raw_exports(), |x| {
        if x["view"] == "table" {
            *x = serde_json::json!({"page": x["page"], "path": x["path"], "fp": x["fp"],
                "view": "error", "message": "boom <script>"});
        }
    }));
    assert!(html.contains("This sheet could not be computed; showing its outline."));
    assert!(!html.contains("<table class=\"sheet-table\">"));
    assert!(
        html.contains("First") && html.contains("Second"),
        "outline rows survive"
    );
    assert!(!html.contains("<script>"), "the error detail is escaped");
    assert!(
        html.contains("<div class=\"sheet-board\">"),
        "other sheets are unaffected"
    );
}

#[test]
fn a_stale_fingerprint_is_refused_with_a_note() {
    let html = export_html(mutate(raw_exports(), |x| {
        if x["view"] == "board" {
            x["fp"] = "0000000000000000".into();
        }
    }));
    assert!(html.contains("This sheet changed while the export was prepared; showing its outline."));
    assert!(!html.contains("<div class=\"sheet-board\">"));
    assert!(html.contains("Write tests"));
}

#[test]
fn hostile_export_data_is_escaped_bounded_and_never_fails_the_export() {
    let html = export_html(mutate(raw_exports(), |x| {
        if x["view"] == "table" && x["query"] != true {
            x["rows"][0]["bg"] = "red;background:url(javascript:alert(1))".into();
            x["rows"][0]["cells"][0] =
                serde_json::json!({"k": "marker", "raw": "\"><script>x</script>", "text": "<i>"});
            x["rows"][1]["cells"][0] =
                serde_json::json!({"k": "chips", "values": ["<img src=x onerror=1>"]});
            x["footer"][1] = serde_json::json!({"label": "<b>", "text": "<u>"});
        }
    }));
    for bad in [
        "<script>x",
        "<img src=x",
        "javascript:",
        "<script>alert",
        "<u>",
        "<i>",
    ] {
        assert!(!html.contains(bad), "unescaped hostile text {bad:?}");
    }
    assert!(
        html.contains("<table class=\"sheet-table\">"),
        "the sheet itself still renders"
    );
}

#[test]
fn an_oversized_export_is_dropped_to_the_outline_not_rendered_unboundedly() {
    let huge: Vec<SheetExport> = mutate(raw_exports(), |x| {
        if x["view"] == "grid" {
            x["cols"] = 1_000_000u64.into();
        }
    });
    let html = export_html(huge);
    assert!(!html.contains("<table class=\"sheet-grid\">"));
    assert!(
        html.contains("Name") && html.contains("Qty"),
        "grid cells stay as outline rows"
    );
    assert!(
        html.contains("<table class=\"sheet-table\">"),
        "in-budget sheets are kept"
    );
}

/// Structural landmarks of every sheet in a page: what a reader can count, not the
/// styling or the id numbering. Row cells, column counts, aggregate footer cells,
/// board columns with their labels and card counts, grid dimensions.
fn landmarks(html: &str) -> Vec<String> {
    fn region<'a>(html: &'a str, from: &str, to: &str) -> &'a str {
        let start = html.find(from).unwrap_or_else(|| panic!("no {from}"));
        let end = html[start..].find(to).unwrap() + start;
        &html[start..end]
    }
    let heads = |s: &str| s.matches("<th>").count() + s.matches("<th ").count();
    let table = region(html, "<table class=\"sheet-table\">", "</table>");
    let mut out = vec![format!(
        "table th={} foot-td={} rows={}",
        heads(table),
        region(table, "<tfoot>", "</tfoot>").matches("<td").count(),
        region(table, "<tbody>", "</tbody>").matches("<tr").count()
    )];
    let board = region(
        html,
        "<div class=\"sheet-board\">",
        "<table class=\"sheet-grid\">",
    );
    for col in board.split("<div class=\"sheet-board-col\">").skip(1) {
        let label = region(col, "<h3>", "</h3>").trim_start_matches("<h3>");
        let label = label.split("<span").next().unwrap().trim();
        out.push(format!(
            "board {label} cards={}",
            col.split("</ul>").next().unwrap().matches("<li").count()
        ));
    }
    // Master draws no aggregate footer under a grid (a named difference, asserted in
    // the test), so the comparison looks at the header and body only.
    let grid = region(html, "<table class=\"sheet-grid\">", "</table>");
    let grid = grid.split("<tfoot>").next().unwrap();
    out.push(format!(
        "grid th={} td={} rows={}",
        heads(grid),
        grid.matches("<td").count(),
        grid.matches("<tr").count()
    ));
    out
}

#[test]
fn exported_sheets_have_the_structure_of_masters_static_export() {
    // `master-oracle.html` is the outline of master 5dfc84503's `publish_security_fixture`
    // run over this same fixture graph (master computes sheets in Rust; og does not).
    let oracle = fs::read_to_string(fixtures().join("master-oracle.html")).unwrap();
    let (base, store) = open_fixture();
    publish_live_with_sheets(
        &store,
        &base.join("output"),
        "export",
        false,
        &bundle(),
        exports(),
    )
    .unwrap();
    let mine = landmarks(&sheets_html(&base));
    assert_eq!(mine, landmarks(&oracle));
    // Named differences from master, all in the app's favour of "what the app shows":
    // the title column is headed "Block", and a formula that cannot compute shows the
    // app's error cell where master shows an empty cell.
    let html = sheets_html(&base);
    assert!(html.contains("<th>Block</th>") && oracle.contains("<th></th>"));
    assert!(html.contains("sheet-formula-error") && !oracle.contains("sheet-formula-error"));
    // The app's grid column aggregate (`tine.col-aggregates:: 1=sum`) is drawn too.
    assert!(
        html.contains("<tfoot><tr><td></td><td><span class=\"sheet-agg-label\">Sum</span> 7</td>")
            && !oracle.contains("sheet-agg-label\">Sum</span> 7")
    );
    store.close();
}

#[test]
fn inputs_sent_to_the_app_are_bounded() {
    // I-22: a hostile or huge sheet cannot make the hand-off unbounded. Rows past the
    // bound are counted (`omitted`, shown as a note), never silently dropped.
    let (base, store) = open_fixture();
    let mut page = String::from("- Big\n  tine.view:: table\n");
    for i in 0..5_100 {
        page.push_str(&format!("  - row {i}\n"));
    }
    fs::write(base.join("graph/pages/Big.md"), page).unwrap();
    store.scan_refresh().unwrap();
    let inputs = sheet_export_inputs(&store, Some(&["Big".to_owned()]), None).unwrap();
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].rows.len(), 5_000);
    assert_eq!(inputs[0].omitted, 100);
    let json = serde_json::to_value(&inputs[0]).unwrap();
    assert_eq!(json["omitted"], 100);
    store.close();
}

#[test]
fn a_sheet_inside_a_grid_cell_is_found_by_its_path_and_laid_out_in_the_cell() {
    let (base, store) = open_fixture();
    fs::write(
        base.join("graph/pages/Nested.md"),
        "public:: true\n\n- Outer\n  tine.view:: grid\n  -\n    - Inner\n      tine.view:: table\n      - one\n      - two\n",
    )
    .unwrap();
    store.scan_refresh().unwrap();
    let inputs = sheet_export_inputs(&store, Some(&["Nested".to_owned()]), None).unwrap();
    let paths: Vec<_> = inputs.iter().map(|i| i.path.clone()).collect();
    assert_eq!(
        paths,
        vec![vec![0], vec![0, 0, 0]],
        "outer grid, then the table in its cell"
    );
    // Answer as the app would: a table for the inner block only.
    let inner = &inputs[1];
    let sheets: Vec<SheetExport> = serde_json::from_value(serde_json::json!([{
        "page": "Nested", "path": inner.path, "fp": inner.fp, "view": "table",
        "columns": [{"label": "Block", "formula": false}],
        "rows": [
            {"ix": 0, "title": "one", "bg": null, "cells": []},
            {"ix": 1, "title": "two", "bg": null, "cells": []}
        ],
        "footer": null, "filterError": null, "omitted": 0
    }]))
    .unwrap();
    publish_live_with_sheets(
        &store,
        &base.join("output"),
        "export",
        true,
        &bundle(),
        sheets,
    )
    .unwrap();
    let html = fs::read_to_string(base.join("output/export/nested.html")).unwrap();
    assert!(html.contains("<table class=\"sheet-table\">"), "{html}");
    assert!(html.contains("<td>one</td>") && html.contains("<td>two</td>"));
    store.close();
}

/// A board grouped by a multi-valued field shows one card in several columns
/// (a block tagged #a #b sits under both). It is still one block: its anchor
/// and search entry appear once, so every `id` names exactly one element (I-12).
#[test]
fn a_card_in_two_board_columns_keeps_page_anchors_unique() {
    let (base, store) = open_fixture();
    let mut exports: Value =
        serde_json::from_str(&fs::read_to_string(fixtures().join("exports.json")).unwrap())
            .unwrap();
    let board = &mut exports[1]["columns"];
    let card = board[3]["cards"][0].clone();
    board[4]["cards"].as_array_mut().unwrap().push(card);
    publish_live_with_sheets(
        &store,
        &base.join("output"),
        "export",
        false,
        &bundle(),
        serde_json::from_value(exports).unwrap(),
    )
    .unwrap();
    let html = sheets_html(&base);
    assert_eq!(html.matches("Write tests</div>").count(), 2, "{html}");
    let mut ids: Vec<&str> = (html.split(" id=\"").skip(1))
        .map(|rest| &rest[..rest.find('"').unwrap()])
        .collect();
    let all = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), all, "duplicate element ids: {html}");
    let index = fs::read_to_string(base.join("output/export/search-index.js")).unwrap();
    assert_eq!(index.matches("Write tests").count(), 1, "{index}");
    store.close();
}

fn tail_queries_html(base: &Path) -> String {
    fs::read_to_string(base.join("output/export/tail-queries.html")).unwrap()
}

#[test]
fn query_backed_sheets_lay_out_as_table_and_board_in_place_of_the_result_list() {
    let (base, store) = open_fixture();
    publish_live_with_sheets(
        &store,
        &base.join("output"),
        "export",
        false,
        &bundle(),
        exports(),
    )
    .unwrap();
    let html = tail_queries_html(&base);
    // The table: the query's rows under the observed columns, the page column last.
    assert!(html.contains("<table class=\"sheet-table\">"), "{html}");
    assert!(
        html.contains("<th>Page</th>") && html.contains("<td>Sheets</td>"),
        "{html}"
    );
    assert!(html.contains("First"), "{html}");
    // The board: state columns over the same rows, no result list beside it.
    assert!(
        html.contains("<div class=\"sheet-board\">") && html.contains("Write tests"),
        "{html}"
    );
    // The grid face is not a query face: the block keeps its flat result list and outline.
    assert!(html.contains("stays a result list"), "{html}");
    assert_eq!(
        html.matches("<table class=\"sheet-table\">").count(),
        1,
        "{html}"
    );
    assert_eq!(
        html.matches("class=\"query\"").count(),
        1,
        "only the grid block keeps a result list: {html}"
    );
    store.close();
}

#[test]
fn a_query_sheet_whose_result_changed_is_refused_with_a_note_and_shows_the_results() {
    let html = {
        let mut all = raw_exports();
        for x in all
            .iter_mut()
            .filter(|x| x["query"] == true && x["view"] == "table")
        {
            x["fp"] = "0000000000000000".into();
        }
        export_html_of(&all)
    };
    let html = html.1;
    assert!(
        html.contains("This query changed while the export was prepared; showing its results."),
        "{html}"
    );
    assert!(!html.contains("<table class=\"sheet-table\">"), "{html}");
    assert!(
        html.contains("First"),
        "the flat result list still shows it: {html}"
    );
    assert!(
        html.contains("<div class=\"sheet-board\">"),
        "the other query sheet is unaffected"
    );
}

fn export_html_of(all: &[Value]) -> (Scratch, String) {
    let (base, store) = open_fixture();
    publish_live_with_sheets(
        &store,
        &base.join("output"),
        "export",
        false,
        &bundle(),
        serde_json::from_value(Value::Array(all.to_vec())).unwrap(),
    )
    .unwrap();
    let html = tail_queries_html(&base);
    store.close();
    (base, html)
}

#[test]
fn a_query_sheet_leaves_out_a_row_on_an_unpublished_page_and_keeps_the_rest() {
    // Master filters the row and keeps the sheet. The row is dropped BEFORE the app
    // computes cells, counts and aggregates, so no trace of it can reach the table.
    let (base, store) = open_fixture();
    fs::write(
        base.join("graph/pages/Secret.md"),
        "- TODO hidden-secret-row\n",
    )
    .unwrap();
    store.scan_refresh().unwrap();
    let scope = SheetScope::Live { all_pages: false };
    let inputs =
        sheet_export_inputs(&store, Some(&["Tail-queries".to_owned()]), Some(&scope)).unwrap();
    let table = &inputs[0];
    let query = table
        .query
        .as_ref()
        .expect("the query table still has rows");
    assert!(
        query.pages.iter().all(|p| p != "Secret")
            && query
                .rows
                .iter()
                .all(|r| !r.raw.contains("hidden-secret-row")),
        "the private row never reaches the app: {:?}",
        query.pages
    );
    assert!(query.rows.iter().any(|r| r.raw.contains("First")));
    let answer: Vec<SheetExport> = serde_json::from_value(serde_json::json!([{
        "page": table.page, "path": table.path, "fp": query.fp, "query": true, "view": "table",
        "columns": [{"label": "Block", "formula": false}],
        "rows": query.rows.iter().map(|r| serde_json::json!({"ix": 0, "title": r.raw, "bg": null, "cells": []})).collect::<Vec<_>>(),
        "footer": null, "filterError": null, "omitted": 0
    }]))
    .unwrap();
    publish_live_with_sheets(
        &store,
        &base.join("output"),
        "export",
        false,
        &bundle(),
        answer,
    )
    .unwrap();
    let html = tail_queries_html(&base);
    assert!(
        html.contains("<table class=\"sheet-table\">") && html.contains("First"),
        "the sheet stays, with the public rows: {html}"
    );
    assert!(
        !html.contains("hidden-secret-row"),
        "the private row appears nowhere in the export: {html}"
    );
    store.close();
}

#[test]
fn the_scope_the_app_sends_deserializes_as_the_wire_shape() {
    // `src/sheet/staticExport.ts::SheetScope` is the other half of this contract.
    let live: SheetScope =
        serde_json::from_value(serde_json::json!({"kind": "live", "allPages": true})).unwrap();
    assert!(matches!(live, SheetScope::Live { all_pages: true }));
    let query: SheetScope = serde_json::from_value(serde_json::json!({
        "kind": "query",
        "request": {"argument": "(task TODO)", "dialect": "macro_query", "properties": [], "name": "n"}
    }))
    .unwrap();
    assert!(matches!(query, SheetScope::Query { .. }));
}
