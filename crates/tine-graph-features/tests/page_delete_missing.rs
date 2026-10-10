//! GH #620: deletion is idempotent after an honest external-file removal.
//! The identity checks run before the page host's deletion and bind it to
//! the bytes they read (`pages::delete_page_expected`, Q-P2b-5).
use std::fs;
use std::sync::Arc;
use tine_core::model::PageKind;
use tine_graph_features::pages;
use tine_store::{PageHost, PageId, PageOperation, Store};

fn fixture(files: &[(&str, &str)]) -> (tempfile::TempDir, Arc<Store>) {
    let root = tempfile::tempdir().unwrap();
    for dir in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.path().join(dir)).unwrap();
    }
    for (path, bytes) in files {
        fs::write(root.path().join(path), bytes).unwrap();
    }
    let store = Arc::new(Store::open(root.path(), Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    (root, store)
}

/// The window's `page_delete`: a fresh host and session per call, as after a
/// restart. Returns the operation the checks handed the host, or their error.
fn delete(
    store: &Arc<Store>,
    name: &str,
    kind: PageKind,
    expected_path: Option<&str>,
) -> std::io::Result<PageOperation> {
    let app_data = tempfile::tempdir().unwrap();
    let host = PageHost::start_for_tests(store, app_data.path()).unwrap();
    let session = serde_json::to_value(host.window_reloaded()).unwrap()["session"]
        .as_u64()
        .unwrap();
    pages::delete_page_expected(store, &host, session, name, kind, expected_path)
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
        fs::remove_file(root.path().join(path)).unwrap();
        store.refresh(tine_store::Depth::Stamps).unwrap();
        assert_eq!(
            delete(&store, name, kind, Some(path)).unwrap(),
            PageOperation::Applied
        );
        // Repeated Delete and restart are equally harmless; no trash or page is created.
        assert_eq!(
            delete(&store, name, kind, Some(path)).unwrap(),
            PageOperation::Applied
        );
        drop(store);
        let reopened = Arc::new(Store::open(root.path(), Default::default()).unwrap().0);
        assert_eq!(
            delete(&reopened, name, kind, Some(path)).unwrap(),
            PageOperation::Applied
        );
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
    let error = delete(&store, "Old", PageKind::Page, Some("pages/Old.md")).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    assert_eq!(
        fs::read_to_string(root.path().join("pages/Old.md")).unwrap(),
        bytes
    );
}

#[test]
fn removed_file_does_not_authorize_deleting_a_replacement_claimant() {
    let (root, store) = fixture(&[("pages/Replacement.md", "title:: Old\n\n- replacement\n")]);
    assert!(delete(&store, "Old", PageKind::Page, Some("pages/Old.md")).is_err());
    assert_eq!(
        fs::read(root.path().join("pages/Replacement.md")).unwrap(),
        b"title:: Old\n\n- replacement\n"
    );
}

#[test]
fn absent_delete_still_refuses_invalid_paths_and_ambiguous_twins() {
    let (root, store) = fixture(&[("pages/Old.md", "- one\n"), ("pages/Old.org", "* two\n")]);
    assert!(delete(&store, "Absent", PageKind::Page, Some("../Outside.md")).is_err());
    assert!(delete(&store, "Old", PageKind::Page, Some("pages/Old.md")).is_err());
    assert_eq!(
        fs::read(root.path().join("pages/Old.md")).unwrap(),
        b"- one\n"
    );
    assert_eq!(
        fs::read(root.path().join("pages/Old.org")).unwrap(),
        b"* two\n"
    );
}

#[test]
fn a_delete_outside_the_graph_refuses_without_writes() {
    let (root, store) = fixture(&[("pages/Old.md", "- one\n")]);
    let app_data = tempfile::tempdir().unwrap();
    let name = format!(
        "{}-Outside.md",
        root.path().file_name().unwrap().to_string_lossy()
    );
    let outside = root.path().parent().unwrap().join(&name);
    fs::write(&outside, "- outside\n").unwrap();
    // The host's own refusal (docs/storage-contract.md
    // `tine-store::page_host::delete`), past the identity checks.
    let host = PageHost::start_for_tests(&store, app_data.path()).unwrap();
    let session = serde_json::to_value(host.window_reloaded()).unwrap()["session"]
        .as_u64()
        .unwrap();
    let deleted = host.delete(
        session,
        &PageId::from(format!("../{name}").as_str()),
        b"- outside\n",
    );
    let kept = fs::read(&outside);
    let _ = fs::remove_file(&outside);
    assert_eq!(deleted, PageOperation::Refused);
    assert_eq!(kept.unwrap(), b"- outside\n");
    assert_eq!(
        fs::read(root.path().join("pages/Old.md")).unwrap(),
        b"- one\n"
    );
}
