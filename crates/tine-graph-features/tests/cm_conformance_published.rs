//! Frozen generation-1 conformance snapshots still carry every linked spelling.

use std::fs;
use std::path::Path;

use serde_json::Value;
use tine_store as store_crate;

#[path = "../../tine-store/src/cm_conformance_shared_tests.rs"]
mod shared;

use shared::GraphSpec;

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
fn frozen_published_conformance_covers_linked_spellings() {
    let _integration_guard = crate::integration_lock();
    let source = shared::fixture_dir(env!("CARGO_MANIFEST_DIR"));
    let frozen = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/published-answers/conformance");
    let mut missing = Vec::new();
    let mut checked = 0;
    for entry in fs::read_dir(source).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|ext| ext != "json")
            || path
                .file_name()
                .is_some_and(|name| name == "expected-differences.json")
        {
            continue;
        }
        let name = path.file_stem().unwrap().to_str().unwrap();
        let source: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let snapshot: Value =
            serde_json::from_slice(&fs::read(frozen.join(path.file_name().unwrap())).unwrap())
                .unwrap();
        if snapshot["refused"].is_string() {
            continue;
        }
        assert_eq!(snapshot["schema"], 1, "{name}");
        let spec = GraphSpec::from_fixture(name, &source);
        assert_linked_spellings_have_rows(name, &spec, &snapshot, &mut missing, &mut checked);
    }
    assert!(checked >= 100, "only {checked} linked spellings checked");
    assert!(
        missing.is_empty(),
        "linked spellings with no published inventory row: {missing:?}"
    );
}
