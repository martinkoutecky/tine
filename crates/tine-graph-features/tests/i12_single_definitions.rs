use std::fs;
use std::path::Path;

fn violations(file: &str, source: &str) -> Vec<String> {
    let mut found = Vec::new();
    if file.ends_with("pages.rs")
        && (source.contains("fn encoding(") || source.contains(".replace(\"___\""))
    {
        found.push("duplicated page-name encoder".to_owned());
    }
    if (file.ends_with("restore.rs") || file.ends_with("backup.rs"))
        && (source.contains("fn is_graph_text(")
            || source.contains("fn is_asset_sidecar(")
            || source.contains("fn is_sidecar("))
    {
        found.push("duplicated graph-text or sidecar classifier".to_owned());
    }
    if file.ends_with("pages.rs") && !source.contains("tine_core::model::encode_page_name") {
        found.push("page feature does not call canonical encoder".to_owned());
    }
    // Match whole file names: `restore.rs` must not count as `store.rs`.
    if ["/model.rs", "/watch.rs", "/store.rs"]
        .iter()
        .any(|name| file.ends_with(name))
        && !source.contains("is_graph_text_path")
    {
        found.push("graph-text client does not use file_kind".to_owned());
    }
    if ["journals.rs", "pages.rs", "conflicts.rs", "sources.rs"]
        .iter()
        .any(|name| file.ends_with(name))
        && !source.contains("tine_store::is_graph_text(")
    {
        found.push("feature client does not use FileId graph-text classifier".to_owned());
    }
    if file.ends_with("watch.rs") && !source.contains("is_asset_sidecar_path") {
        found.push("watcher sidecar client does not use file_kind".to_owned());
    }
    if ["watch.rs", "store.rs"]
        .iter()
        .any(|name| file.ends_with(name))
        && (source.contains("Some(\"md\" | \"org\")") || source.contains("Some(\"edn\")"))
    {
        found.push("store client repeats a file-kind literal".to_owned());
    }
    if file.ends_with("query_plan.rs")
        && (source.contains("fn crumb_line(") || !source.contains("crate::query::crumb_line"))
    {
        found.push("query plan duplicates breadcrumb answer".to_owned());
    }
    if file.ends_with("model.rs")
        && (source.contains("let mut h: u64 = 0xcbf2")
            || !source.contains("FileRev::from_bytes(s.as_bytes())"))
    {
        found.push("model duplicates raw revision".to_owned());
    }
    if ["pages.rs", "conflicts.rs", "pdf.rs"]
        .iter()
        .any(|name| file.ends_with(name))
        && !source.contains("crate::parsed_text::read(store,")
    {
        found.push("feature client bypasses shared parsed-text admission".to_owned());
    }
    if ["pages.rs", "conflicts.rs", "pdf.rs"]
        .iter()
        .any(|name| file.ends_with(name))
        && (source.contains("parse_input_depth_within_limit")
            || source.contains("headline_levels_within_limit"))
    {
        found.push("feature client repeats parsed-text admission".to_owned());
    }
    found
}

fn assert_clean(file: &str, source: &str) {
    let found = violations(file, source);
    assert!(found.is_empty(), "I-12: use the tine-core encoder and tine-store file-kind definitions; exemplar tine_core::model::encode_page_name. {file}: {found:?}");
}

#[test]
fn shared_answers_have_one_definition() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for file in [
        "crates/tine-graph-features/src/pages.rs",
        "crates/tine-store/src/restore.rs",
        "src-tauri/src/backup.rs",
        "src-tauri/src/backup/restore.rs",
        "crates/tine-store/src/model.rs",
        "crates/tine-store/src/watch.rs",
        "crates/tine-store/src/store.rs",
        "crates/tine-store/src/query_plan.rs",
        "crates/tine-graph-features/src/journals.rs",
        "crates/tine-graph-features/src/conflicts.rs",
        "crates/tine-graph-features/src/sources.rs",
        "crates/tine-graph-features/src/pdf.rs",
    ] {
        let mut source = fs::read_to_string(root.join(file)).unwrap();
        if file.ends_with("query_plan.rs") {
            // The block reader moved into a child module. Keep the breadcrumb
            // check over the whole answerer, including that module.
            source.push_str(
                &fs::read_to_string(root.join("crates/tine-store/src/query_plan/blocks.rs"))
                    .unwrap(),
            );
        }
        if file.ends_with("store/src/watch.rs") {
            // The watcher is split into child modules (the restore baseline
            // walks asset sidecars in `watch/restore.rs`). Keep the file-kind
            // checks over the whole watcher, every child module included.
            let mut children: Vec<_> = fs::read_dir(root.join("crates/tine-store/src/watch"))
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
                .collect();
            children.sort();
            for child in children {
                source.push_str(&fs::read_to_string(child).unwrap());
            }
        }
        assert_clean(file, &source);
    }
    let owner = fs::read_to_string(root.join("crates/tine-store/src/file_kind.rs")).unwrap();
    assert!(owner.contains("pub fn is_graph_text(") && owner.contains("pub fn is_asset_sidecar("),
        "I-12: tine-store owns the graph-text and asset-sidecar answers; exemplar tine_store::file_kind");
    assert!(
        owner.contains("pub(crate) fn is_graph_text_path(")
            && owner.contains("pub(crate) fn is_asset_sidecar_path(")
    );
    let model = fs::read_to_string(root.join("crates/tine-store/src/model.rs")).unwrap();
    let page_file = model
        .split("fn is_page_file(")
        .nth(1)
        .unwrap()
        .split("fn slash_path(")
        .next()
        .unwrap();
    assert!(
        page_file.contains("crate::file_kind::is_graph_text_path")
            && !page_file.contains("Some(\"md\"")
    );
    let query = fs::read_to_string(root.join("crates/tine-store/src/query.rs")).unwrap();
    assert!(query.contains("pub(crate) fn crumb_line("));
    let store = fs::read_to_string(root.join("crates/tine-store/src/store.rs")).unwrap();
    assert!(store.contains("fn fnv_update("));
    let parsed =
        fs::read_to_string(root.join("crates/tine-graph-features/src/parsed_text.rs")).unwrap();
    assert!(
        parsed.contains("pub(crate) fn read(") && parsed.contains("headline_levels_within_limit")
    );
}

#[test]
fn planted_duplicate_classifier_fails() {
    let fake = "fn is_graph_text(p: &Path) -> bool { true }";
    assert!(
        std::panic::catch_unwind(|| assert_clean("restore.rs", fake)).is_err(),
        "I-12: planted duplicate must fail; exemplar tine_store::file_kind"
    );
}

/// Sheet semantics (row filter, grouping, formulas, aggregates) have ONE definition, the
/// app's `src/sheet/*`. The Rust static export only lays out data the app computed
/// (`render_sheets.rs`, family 7), so it must not name a sheet-semantics property or define
/// a computation over cells. Master carried a ~1.3k-line Rust twin of this logic; this is
/// the guard that keeps it from coming back. Exemplar: render_sheets.rs itself.
fn sheet_semantics(source: &str) -> Vec<String> {
    let code: String = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut found: Vec<String> = [
        "tine.group-by",
        "tine.col-aggregates",
        "tine.formula",
        "tine.filter",
        "tine.fields",
        "tine.header",
        "fn aggregate",
        "fn evaluate",
        "fn eval_",
        "fn group",
        "fn sort",
        ".sort(",
        ".sort_by",
        "parse_sheet",
        "sheet_config",
    ]
    .iter()
    .filter(|token| code.contains(**token))
    .map(|token| (*token).to_owned())
    .collect();
    // The single property Rust reads is the candidate marker; validity is the app's call.
    if code.matches("\"tine.").count() != 1 || !code.contains("\"tine.view\"") {
        found.push("reads a sheet property other than the tine.view candidate marker".to_owned());
    }
    found
}

#[test]
fn static_export_sheets_compute_nothing() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source =
        fs::read_to_string(root.join("crates/tine-graph-features/src/render_sheets.rs")).unwrap();
    let found = sheet_semantics(&source);
    assert!(
        found.is_empty(),
        "I-12: render_sheets.rs lays out data the app computed; {found:?} is sheet semantics, which lives only in src/sheet/*. exemplar render_sheets.rs::render_table"
    );
}

#[test]
fn planted_sheet_semantics_in_the_static_export_fail() {
    for planted in [
        "fn aggregate(values: &[f64]) -> f64 { values.iter().sum() }",
        "let g = props.get(\"tine.group-by\");",
        "rows.sort_by(|a, b| a.cmp(b));",
    ] {
        let source = format!("const K: &str = \"tine.view\";\n{planted}");
        assert!(
            !sheet_semantics(&source).is_empty(),
            "I-12: planted sheet semantics must fail the guard: {planted}"
        );
    }
}
