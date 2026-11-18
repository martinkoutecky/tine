//! GH #978: of two files claiming one name, the window deletes exactly the one
//! it shows (`pages::delete_page_file`), at the revision it loaded, through the
//! page host's guarded deletion. A delete by name still refuses the twins.
use std::fs;
use std::sync::Arc;

use tine_core::concord_queue::ConflictSource;
use tine_core::model::PageKind;
use tine_graph_features::{conflicts, pages};
use tine_store::{FileRev, PageHost, PageOperation, Resolved, Store};

const WINNER: &str = "pages/XX1.md";
const SUBFOLDER: &str = "pages/笔记集/XX1.md";
const WINNER_BYTES: &str = "- top-level copy\n";
const SUBFOLDER_BYTES: &str = "tags:: 笔记\n\n- 子文件夹里的副本\n";

fn twins() -> (tempfile::TempDir, Arc<Store>) {
    let root = tempfile::tempdir().unwrap();
    for dir in ["pages/笔记集", "journals", "logseq"] {
        fs::create_dir_all(root.path().join(dir)).unwrap();
    }
    fs::write(root.path().join(WINNER), WINNER_BYTES).unwrap();
    fs::write(root.path().join(SUBFOLDER), SUBFOLDER_BYTES).unwrap();
    let store = Arc::new(Store::open(root.path(), Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    (root, store)
}

/// A fresh host and window session per call, as the window's command has.
fn with_session<T>(store: &Arc<Store>, run: impl FnOnce(&PageHost, u64) -> T) -> T {
    let app_data = tempfile::tempdir().unwrap();
    let host = PageHost::start_for_tests(store, app_data.path()).unwrap();
    let session = serde_json::to_value(host.window_reloaded()).unwrap()["session"]
        .as_u64()
        .unwrap();
    run(&host, session)
}

fn rev_of(root: &tempfile::TempDir, path: &str) -> String {
    FileRev::from_bytes(&fs::read(root.path().join(path)).unwrap()).into()
}

fn claimants(store: &Store) -> Vec<String> {
    match store.whole_graph().unwrap().resolve("XX1", false) {
        Resolved::Existing { id, others } => std::iter::once(id)
            .chain(others)
            .map(|id| id.as_str().to_owned())
            .collect(),
        _ => Vec::new(),
    }
}

fn name_conflicts(store: &Store) -> usize {
    conflicts::conflict_inventory(store)
        .unwrap()
        .queue
        .iter()
        .filter(|object| object.source == ConflictSource::DuplicateName)
        .count()
}

#[test]
fn deleting_one_twin_file_keeps_the_other_byte_identical_and_clears_the_conflict() {
    let _integration_guard = crate::integration_lock();
    for (deleted, kept, kept_bytes) in [
        (SUBFOLDER, WINNER, WINNER_BYTES),
        (WINNER, SUBFOLDER, SUBFOLDER_BYTES),
    ] {
        let (root, store) = twins();
        assert_eq!(claimants(&store), [WINNER, SUBFOLDER]);
        assert_eq!(name_conflicts(&store), 1);
        let kept_rev = rev_of(&root, kept);
        let base = rev_of(&root, deleted);
        let outcome = with_session(&store, |host, session| {
            pages::delete_page_file(&store, host, session, deleted, &base)
        })
        .unwrap();
        assert_eq!(outcome, PageOperation::Applied, "{deleted}");
        assert!(!root.path().join(deleted).exists(), "{deleted}");
        assert_eq!(
            fs::read_to_string(root.path().join(kept)).unwrap(),
            kept_bytes
        );
        assert_eq!(
            rev_of(&root, kept),
            kept_rev,
            "nothing writes the kept file"
        );
        // The published answer, a rescan and a restart agree: one claimant.
        assert_eq!(claimants(&store), [kept]);
        assert_eq!(name_conflicts(&store), 0);
        store.refresh(tine_store::Depth::Stamps).unwrap();
        assert_eq!(claimants(&store), [kept]);
        drop(store);
        let reopened = Arc::new(Store::open(root.path(), Default::default()).unwrap().0);
        assert_eq!(claimants(&reopened), [kept]);
        assert_eq!(name_conflicts(&reopened), 0);
        assert_eq!(rev_of(&root, kept), kept_rev);
    }
}

#[test]
fn a_delete_by_name_still_refuses_twins_and_writes_nothing() {
    let _integration_guard = crate::integration_lock();
    let (root, store) = twins();
    for expected in [None, Some(WINNER), Some(SUBFOLDER)] {
        let error = with_session(&store, |host, session| {
            pages::delete_page_expected(&store, host, session, "XX1", PageKind::Page, expected)
        })
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(error.to_string().contains("“pages/笔记集/XX1.md”"));
    }
    assert_eq!(
        fs::read_to_string(root.path().join(WINNER)).unwrap(),
        WINNER_BYTES
    );
    assert_eq!(
        fs::read_to_string(root.path().join(SUBFOLDER)).unwrap(),
        SUBFOLDER_BYTES
    );
    assert_eq!(claimants(&store), [WINNER, SUBFOLDER]);
}

#[test]
fn a_file_delete_at_a_stale_revision_or_outside_the_pages_refuses_and_writes_nothing() {
    let _integration_guard = crate::integration_lock();
    let (root, store) = twins();
    let loaded = rev_of(&root, SUBFOLDER);
    // An external editor or a sync delivery changes the file after the window read it.
    let external = "- edited elsewhere after the window loaded it\n";
    fs::write(root.path().join(SUBFOLDER), external).unwrap();
    let error = with_session(&store, |host, session| {
        pages::delete_page_file(&store, host, session, SUBFOLDER, &loaded)
    })
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    assert_eq!(
        fs::read_to_string(root.path().join(SUBFOLDER)).unwrap(),
        external
    );
    for path in ["../XX1.md", "logseq/config.edn", "pages/笔记集"] {
        assert!(
            with_session(&store, |host, session| {
                pages::delete_page_file(&store, host, session, path, &loaded)
            })
            .is_err(),
            "{path}"
        );
    }
    assert_eq!(
        fs::read_to_string(root.path().join(WINNER)).unwrap(),
        WINNER_BYTES
    );
    assert!(!root.path().join("logseq/.tine-trash").exists());
}

/// A twin outside the configured pages and journals folders (the graph root, another folder) is a page file too:
/// "Delete file" offered for it removes exactly that file (merge review of 384b3ad42, finding 1).
#[test]
fn a_twin_outside_the_pages_folder_is_deleted_by_its_path() {
    let _integration_guard = crate::integration_lock();
    for outside in [
        "XX1.md",
        "external/XX1.md",
        "external/deep/XX1.org",
        "笔记集/XX1.markdown",
    ] {
        let root = tempfile::tempdir().unwrap();
        for dir in ["pages", "journals", "logseq"] {
            fs::create_dir_all(root.path().join(dir)).unwrap();
        }
        let file = root.path().join(outside);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(root.path().join(WINNER), WINNER_BYTES).unwrap();
        fs::write(&file, "- outside copy\n").unwrap();
        let store = Arc::new(Store::open(root.path(), Default::default()).unwrap().0);
        store.whole_graph().unwrap();
        assert_eq!(name_conflicts(&store), 1, "{outside} is a claimant");
        let kept_rev = rev_of(&root, WINNER);
        let base = rev_of(&root, outside);
        let outcome = with_session(&store, |host, session| {
            pages::delete_page_file(&store, host, session, outside, &base)
        })
        .unwrap();
        assert_eq!(outcome, PageOperation::Applied, "{outside}");
        assert!(!file.exists(), "{outside}");
        assert_eq!(
            rev_of(&root, WINNER),
            kept_rev,
            "nothing writes the kept file"
        );
        assert_eq!(name_conflicts(&store), 0, "{outside}");
    }
}
