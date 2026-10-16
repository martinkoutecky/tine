//! (G) as amended 2026-10-11, extended from page opens to claims: a page an
//! operation takes on keeps its index row when the host read the row's bytes.
//! Before, a delete or rename published each page it took on as removed (the
//! claim) and then re-added at its pre-operation bytes (the host's load), so
//! every referrer cost an extra snapshot and inventory change, and the Concord
//! ledger recorded a base for a page that was being deleted.
use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tine_core::model::PageKind;
use tine_graph_features::pages;
use tine_store::{ChangeKind, FileRev, PageHost, Store};

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

#[test]
fn an_operation_publishes_no_pre_operation_bytes_of_the_pages_it_takes_on() {
    let _integration_guard = crate::integration_lock();
    const GONE: &str = "- gone\n";
    const REF: &str = "- see [[Target]]\n";
    let (root, store) = fixture(&[
        ("pages/Gone.md", GONE),
        ("pages/Target.md", "- target\n"),
        ("pages/Ref.md", REF),
    ]);
    let changes = store.subscribe();
    let app_data = tempfile::tempdir().unwrap();
    let host = PageHost::start_for_tests(&store, app_data.path()).unwrap();
    let session = serde_json::to_value(host.window_reloaded()).unwrap()["session"]
        .as_u64()
        .unwrap();
    pages::delete_page_expected(&store, &host, session, "Gone", PageKind::Page, None).unwrap();
    pages::rename_page_expected(&store, &host, "Target", "Renamed", None).unwrap();

    // Every published file change until both operations are visible.
    let mut seen = Vec::new();
    let done = |seen: &Vec<(String, ChangeKind, Option<FileRev>)>| {
        seen.iter()
            .any(|(id, kind, _)| id == "pages/Gone.md" && *kind == ChangeKind::Removed)
            && seen
                .iter()
                .any(|(id, kind, _)| id == "pages/Ref.md" && *kind == ChangeKind::Modified)
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(&seen) && Instant::now() < deadline {
        match changes.try_recv().unwrap() {
            Some(change) => seen.extend(
                change
                    .files
                    .into_iter()
                    .map(|(id, kind, rev)| (id.as_str().to_owned(), kind, rev)),
            ),
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    assert!(done(&seen), "operations not published: {seen:?}");
    assert!(!root.path().join("pages/Gone.md").exists());
    assert!(fs::read_to_string(root.path().join("pages/Ref.md"))
        .unwrap()
        .contains("[[Renamed]]"));

    let gone: Vec<_> = seen
        .iter()
        .filter(|(id, ..)| id == "pages/Gone.md")
        .collect();
    assert!(
        gone.iter().all(|(_, kind, _)| *kind == ChangeKind::Removed),
        "the deleted page was re-published before its removal: {gone:?}"
    );
    let stale = Some(FileRev::from_bytes(REF.as_bytes()));
    let referrer: Vec<_> = seen
        .iter()
        .filter(|(id, ..)| id == "pages/Ref.md")
        .collect();
    assert!(
        referrer.iter().all(|(_, _, rev)| *rev != stale),
        "the referrer was re-published at its pre-rename bytes: {referrer:?}"
    );
    drop(host);
    store.close();
}
