//! A sheet owner's `tine.*` configuration in the static export (master f8a797788).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tine_graph_features::publish_query::publish_live_with_sheets;
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

fn exports() -> Vec<SheetExport> {
    serde_json::from_str(&fs::read_to_string(fixtures().join("exports.json")).unwrap()).unwrap()
}

fn publish() -> (Scratch, String, String) {
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
    let index = fs::read_to_string(base.join("output/export/search-index.js")).unwrap();
    store.close();
    (base, html, index)
}

/// Master f8a797788: the sheet owner's `tine.*` view config drives the layout and
/// must not also print as prose chips (master asserts `!html.contains("tine.view::")`).
#[test]
fn a_sheet_owners_view_config_is_not_printed_as_property_chips() {
    let (_b, html, _) = publish();
    let g = html.find("<table class=\"sheet-grid\">").unwrap();
    let grid_end = g + html[g..].find("</table>").unwrap();
    let region = &html[..grid_end];
    for key in [
        "tine.view::",
        "tine.fields::",
        "tine.formula.total::",
        "tine.col-aggregates::",
        "tine.group-by::",
        "tine.header::",
    ] {
        assert!(
            !region.contains(key),
            "{key} printed in a sheet owner's props"
        );
    }
}

#[test]
fn sheet_rows_stay_searchable_and_board_follows_workflow() {
    let (_b, html, index) = publish();
    for row in ["Second", "Write tests", "Implement"] {
        assert!(index.contains(row), "row {row} missing from search index");
    }
    let board = &html[html.find("<div class=\"sheet-board\">").unwrap()
        ..html.find("<table class=\"sheet-grid\">").unwrap()];
    let pos = |s: &str| board.find(s).unwrap_or_else(|| panic!("no {s}"));
    assert!(pos(">TODO") < pos(">DOING") && pos(">DOING") < pos(">(none)"));
}
