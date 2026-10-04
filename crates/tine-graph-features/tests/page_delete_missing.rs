//! GH #620: deletion is idempotent after an honest external-file removal.
use std::fs;
use tine_core::model::PageKind;
use tine_graph_features::pages;
use tine_store::Store;

fn fixture(files: &[(&str, &str)]) -> (tempfile::TempDir, Store) {
    let root = tempfile::tempdir().unwrap();
    for dir in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.path().join(dir)).unwrap();
    }
    for (path, bytes) in files {
        fs::write(root.path().join(path), bytes).unwrap();
    }
    let store = Store::open(root.path(), Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    (root, store)
}

#[test]
fn delete_after_external_removal_accepts_the_displayed_path_without_writes() {
    for (name, path, kind) in [
        (
            "Example/Namespace",
            "pages/Example%2FNamespace.md",
            PageKind::Page,
        ),
        ("Oct 1st, 2026", "journals/2026_10_01.md", PageKind::Journal),
    ] {
        let (root, store) = fixture(&[(path, "- displayed body\n"), ("pages/Kept.md", "- kept\n")]);
        let revision = pages::get_page(&store, name, kind).unwrap().unwrap().rev;
        fs::remove_file(root.path().join(path)).unwrap();
        store.scan_refresh().unwrap();
        pages::delete_page_expected(&store, name, kind, Some(path), Some(&revision)).unwrap();
        // Repeated Delete and restart are equally harmless; no trash or page is created.
        pages::delete_page_expected(&store, name, kind, Some(path), None).unwrap();
        drop(store);
        let reopened = Store::open(root.path(), Default::default()).unwrap().0;
        pages::delete_page_expected(&reopened, name, kind, Some(path), None).unwrap();
        assert!(!root.path().join(path).exists());
        assert!(!root.path().join("logseq/.tine-trash").exists());
        assert_eq!(
            fs::read(root.path().join("pages/Kept.md")).unwrap(),
            b"- kept\n"
        );
        assert!(pages::get_page(&reopened, name, kind).unwrap().is_none());
    }
}

#[test]
fn absent_identity_does_not_trash_a_live_file_with_a_changed_title() {
    let bytes = "title:: Someone Else\n\n- external editor content\n";
    let (root, store) = fixture(&[("pages/Old.md", bytes)]);
    let error =
        pages::delete_page_expected(&store, "Old", PageKind::Page, Some("pages/Old.md"), None)
            .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    assert_eq!(
        fs::read_to_string(root.path().join("pages/Old.md")).unwrap(),
        bytes
    );
}

#[test]
fn removed_file_does_not_authorize_deleting_a_replacement_claimant() {
    let (root, store) = fixture(&[("pages/Replacement.md", "title:: Old\n\n- replacement\n")]);
    assert!(
        pages::delete_page_expected(&store, "Old", PageKind::Page, Some("pages/Old.md"), None)
            .is_err()
    );
    assert_eq!(
        fs::read(root.path().join("pages/Replacement.md")).unwrap(),
        b"title:: Old\n\n- replacement\n"
    );
}

#[test]
fn absent_delete_still_refuses_invalid_paths_and_ambiguous_twins() {
    let (root, store) = fixture(&[("pages/Old.md", "- one\n"), ("pages/Old.org", "* two\n")]);
    assert!(pages::delete_page_expected(
        &store,
        "Absent",
        PageKind::Page,
        Some("../Outside.md"),
        None
    )
    .is_err());
    assert!(
        pages::delete_page_expected(&store, "Old", PageKind::Page, Some("pages/Old.md"), None)
            .is_err()
    );
    assert_eq!(
        fs::read(root.path().join("pages/Old.md")).unwrap(),
        b"- one\n"
    );
    assert_eq!(
        fs::read(root.path().join("pages/Old.org")).unwrap(),
        b"* two\n"
    );
}
