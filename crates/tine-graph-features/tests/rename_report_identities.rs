//! `RenameReport::renamed` (small-1109c r2): the rename reports every page
//! identity it moved, old name → the name it wrote, and the window's
//! navigation follows exactly that map: the page, its file-backed namespace
//! descendants, and the reference-only members whose references the rewrite
//! renamed. Renaming a page into a free name inside its own namespace moves
//! only that page, once.
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tine_graph_features::pages;
use tine_store::Store;

/// Renames are the page host's (STEP3 §7): each runs through a host started
/// for it under its own app data, as the app's graph binding runs one.
fn renamed(files: &[(&str, &str)], old: &str, new: &str) -> (PathBuf, pages::RenameReport) {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-rename-identities-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    for dir in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    fs::write(
        root.join("logseq/config.edn"),
        "{:file/name-format :triple-lowbar}\n",
    )
    .unwrap();
    for (rel, body) in files {
        fs::write(root.join(rel), body).unwrap();
    }
    let store = std::sync::Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
    let report = pages::rename_or_merge_page(&store, &host, old, new, None, None, &[]).unwrap();
    (root, report)
}

fn pairs(report: &pages::RenameReport) -> Vec<(&str, &str, Option<&str>)> {
    let mut pairs: Vec<_> = report
        .renamed
        .iter()
        .map(|page| (page.from.as_str(), page.to.as_str(), page.path.as_deref()))
        .collect();
    pairs.sort_unstable();
    pairs
}

#[test]
fn the_report_names_every_identity_the_rename_moved() {
    let _integration_guard = crate::integration_lock();
    let (root, report) = renamed(
        &[
            ("pages/Old.md", "- parent\n"),
            ("pages/Old___child.md", "- child\n"),
            ("pages/Ref.md", "- [[Old]] and [[Old/ghost]]\n"),
            ("pages/Deep.md", "- only [[Old/a/deep]]\n"),
        ],
        "Old",
        "New",
    );
    assert_eq!(
        pairs(&report),
        [
            ("Old", "New", Some("pages/Old.md")),
            ("Old/a", "New/a", None),
            ("Old/a/deep", "New/a/deep", None),
            ("Old/child", "New/child", Some("pages/Old___child.md")),
            ("Old/ghost", "New/ghost", None),
        ]
    );
    // `Old/ghost` and `Old/a/deep` have no file, but every reference to them
    // (and to the fileless namespace parent `Old/a`) now names the new page.
    assert_eq!(
        fs::read_to_string(root.join("pages/Ref.md")).unwrap(),
        "- [[New]] and [[New/ghost]]\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("pages/Deep.md")).unwrap(),
        "- only [[New/a/deep]]\n"
    );
    let wire = serde_json::to_value(&report).unwrap();
    assert_eq!(
        wire["renamed"][0],
        serde_json::json!({"from": "Old", "to": "New", "path": "pages/Old.md"})
    );
}

#[test]
fn renaming_into_a_free_name_inside_its_own_namespace_moves_one_page_once() {
    let _integration_guard = crate::integration_lock();
    let (root, report) = renamed(&[("pages/Foo.md", "- foo\n")], "Foo", "Foo/bar");
    assert_eq!(pairs(&report), [("Foo", "Foo/bar", Some("pages/Foo.md"))]);
    assert!(root.join("pages/Foo___bar.md").exists());
}

#[test]
fn a_reference_only_page_reports_its_rename_without_a_file() {
    let _integration_guard = crate::integration_lock();
    let (_root, report) = renamed(&[("pages/Ref.md", "- [[Ghost]]\n")], "Ghost", "Spirit");
    assert_eq!(pairs(&report), [("Ghost", "Spirit", None)]);
}
