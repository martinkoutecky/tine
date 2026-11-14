//! `RenameReport::renamed` (small-1109c r2): the rename reports every page
//! identity it moved, old name → the name it wrote, and the window's
//! navigation follows exactly that map: the page, its file-backed namespace
//! descendants, and the reference-only members whose references the rewrite
//! renamed. Renaming a page into a free name inside its own namespace moves
//! only that page, once.
use std::fs;
use std::path::PathBuf;
use tine_graph_features::pages;
use tine_store::Store;

/// Renames are the page host's (STEP3 §7): each runs through a host started
/// for it under its own app data, as the app's graph binding runs one.
fn renamed(files: &[(&str, &str)], old: &str, new: &str) -> (PathBuf, pages::RenameReport) {
    renamed_as(files, old, new, None)
}

fn renamed_as(
    files: &[(&str, &str)],
    old: &str,
    new: &str,
    merge_into: Option<&str>,
) -> (PathBuf, pages::RenameReport) {
    let root = crate::test_support::fresh_dir("tine-rename-identities-");
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
    let report =
        pages::rename_or_merge_page(&store, &host, old, new, None, merge_into, &[]).unwrap();
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

/// Optional wire capture lets the window regression consume this real report
/// from the baseline and corrected writer without starting a native window.
fn capture_report(label: &str, report: &pages::RenameReport) {
    if let Some(dir) = std::env::var_os("TINE_RENAME_REPORT_CAPTURE") {
        let dir = PathBuf::from(dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(format!("{label}.json")),
            serde_json::to_vec(report).unwrap(),
        )
        .unwrap();
    }
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

#[test]
fn a_skipped_conflicted_referrer_contributes_no_reference_only_names() {
    let _integration_guard = crate::integration_lock();
    let conflict = "- intro\n<<<<<<< HEAD\n- [[Old/ghost]]\n=======\n- other\n>>>>>>> branch\n";
    let (root, report) = renamed(
        &[("pages/Old.md", "- parent\n"), ("pages/Ref.md", conflict)],
        "Old",
        "New",
    );
    assert_eq!(report.skipped_conflicted_referrers, ["pages/Ref.md"]);
    capture_report("conflict", &report);
    assert_eq!(
        fs::read_to_string(root.join("pages/Ref.md")).unwrap(),
        conflict
    );
    assert_eq!(pairs(&report), [("Old", "New", Some("pages/Old.md"))]);
}

#[test]
fn every_applied_rewrite_target_contributes_its_reference_only_name() {
    let _integration_guard = crate::integration_lock();
    let md = "- {{renderer [[Old/macro]]}}\n- {{query (page [[Old/query]])}}\n- #Old/tag\n- [[Old/link]]\n";
    let org = "* [[Old/org][label]]\n* [[file:Old___file.org][label]]\n";
    let (root, report) = renamed(
        &[
            ("pages/Old.md", "- parent\n"),
            ("pages/Ref.md", md),
            ("pages/Org.org", org),
        ],
        "Old",
        "New",
    );
    capture_report("targets", &report);
    assert_eq!(
        fs::read_to_string(root.join("pages/Ref.md")).unwrap(),
        md.replace("Old/", "New/")
    );
    assert_eq!(
        fs::read_to_string(root.join("pages/Org.org")).unwrap(),
        org.replace("Old/", "New/").replace("Old___", "New___")
    );
    assert_eq!(
        pairs(&report),
        [
            ("Old", "New", Some("pages/Old.md")),
            ("Old/file", "New/file", None),
            ("Old/link", "New/link", None),
            ("Old/macro", "New/macro", None),
            ("Old/org", "New/org", None),
            ("Old/query", "New/query", None),
            ("Old/tag", "New/tag", None),
        ]
    );
}

#[test]
fn moved_and_merged_page_payloads_report_their_applied_targets() {
    let _integration_guard = crate::integration_lock();
    let body = "- {{renderer [[Old/source]]}}\n";
    let (root, report) = renamed(&[("pages/Old.md", body)], "Old", "New");
    assert_eq!(
        pairs(&report),
        [
            ("Old", "New", Some("pages/Old.md")),
            ("Old/source", "New/source", None)
        ]
    );
    assert_eq!(
        fs::read_to_string(root.join("pages/New.md")).unwrap(),
        body.replace("Old/", "New/")
    );
    let (root, report) = renamed_as(
        &[
            ("pages/Old.md", body),
            ("pages/New.md", "- {{query (page [[Old/survivor]])}}\n"),
        ],
        "Old",
        "New",
        Some("pages/New.md"),
    );
    assert_eq!(
        pairs(&report),
        [
            ("Old", "New", Some("pages/Old.md")),
            ("Old/source", "New/source", None),
            ("Old/survivor", "New/survivor", None),
        ]
    );
    let merged = fs::read_to_string(root.join("pages/New.md")).unwrap();
    assert!(merged.contains("{{renderer [[New/source]]}}"));
    assert!(merged.contains("{{query (page [[New/survivor]])}}"));
    assert!(!merged.contains("Old/"));
}
