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
    ] {
        assert_clean(file, &fs::read_to_string(root.join(file)).unwrap());
    }
    let owner = fs::read_to_string(root.join("crates/tine-store/src/file_kind.rs")).unwrap();
    assert!(owner.contains("pub fn is_graph_text(") && owner.contains("pub fn is_asset_sidecar("),
        "I-12: tine-store owns the graph-text and asset-sidecar answers; exemplar tine_store::file_kind");
}

#[test]
fn planted_duplicate_classifier_fails() {
    let fake = "fn is_graph_text(p: &Path) -> bool { true }";
    assert!(
        std::panic::catch_unwind(|| assert_clean("restore.rs", fake)).is_err(),
        "I-12: planted duplicate must fail; exemplar tine_store::file_kind"
    );
}
