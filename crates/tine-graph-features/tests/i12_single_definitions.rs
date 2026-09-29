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
    if ["model.rs", "watch.rs", "store.rs"]
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
