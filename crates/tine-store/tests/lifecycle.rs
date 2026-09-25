use std::path::{Path, PathBuf};
use std::sync::Arc;

use tine_store::model::Graph;
use tine_store::{Area, LoadError, OpenOptions, Refusal, Store, TxOutcome, Why};

struct Fixture(PathBuf);

impl Fixture {
    fn new(label: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let root = std::env::temp_dir().join(format!(
            "tine-lifecycle-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        ));
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("journals")).unwrap();
        Self(root)
    }

    fn root(&self) -> &Path {
        &self.0
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn compare_checked_open(root: &Path, approved: Option<&Path>) {
    let old = Graph::open_checked_with_assets(root, approved).map(|_| ());
    let new = Store::open(
        root,
        OpenOptions {
            approved_external_assets: approved.map(Path::to_path_buf),
            watch: Default::default(),
        },
    )
    .map(|(store, _, _)| {
        store.close();
    });
    assert_eq!(old.is_ok(), new.is_ok());
    if let (Err(old), Err(new)) = (old, new) {
        assert_eq!(old.to_string(), new.to_string());
    }
}

#[test]
fn checked_open_matches_legacy_layout_and_consent() {
    let ordinary = Fixture::new("ordinary");
    std::fs::write(ordinary.root().join("pages/A.md"), "- hello\n").unwrap();
    compare_checked_open(ordinary.root(), None);

    let missing_pages = Fixture::new("missing-pages");
    std::fs::remove_dir(missing_pages.root().join("pages")).unwrap();
    compare_checked_open(missing_pages.root(), None);

    let file = Fixture::new("not-folder");
    let file_path = file.root().join("a-file");
    std::fs::write(&file_path, "file").unwrap();
    assert!(matches!(
        Store::inspect(&file_path),
        Err(tine_store::OpenError::NotAFolder(_))
    ));
    assert!(matches!(
        Store::open(&file_path, OpenOptions::default()),
        Err(tine_store::OpenError::NotAFolder(_))
    ));

    #[cfg(unix)]
    {
        let linked_pages = Fixture::new("linked-pages");
        let outside = Fixture::new("outside-pages");
        std::fs::remove_dir(linked_pages.root().join("pages")).unwrap();
        std::os::unix::fs::symlink(
            outside.root().join("pages"),
            linked_pages.root().join("pages"),
        )
        .unwrap();
        compare_checked_open(linked_pages.root(), None);

        let external = Fixture::new("external-assets");
        let first = Fixture::new("first-assets");
        let second = Fixture::new("second-assets");
        std::fs::create_dir(first.root().join("target")).unwrap();
        std::fs::create_dir(second.root().join("target")).unwrap();
        std::os::unix::fs::symlink(first.root().join("target"), external.root().join("assets"))
            .unwrap();
        compare_checked_open(external.root(), None);
        compare_checked_open(external.root(), Some(&first.root().join("target")));
        std::fs::remove_file(external.root().join("assets")).unwrap();
        std::os::unix::fs::symlink(second.root().join("target"), external.root().join("assets"))
            .unwrap();
        compare_checked_open(external.root(), Some(&first.root().join("target")));
    }
}

#[test]
fn inspection_does_not_write() {
    let fixture = Fixture::new("inspect");
    let before: Vec<_> = std::fs::read_dir(fixture.root())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    let inspection = Store::inspect(fixture.root()).unwrap();
    let after: Vec<_> = std::fs::read_dir(fixture.root())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        inspection.root,
        std::fs::canonicalize(fixture.root()).unwrap()
    );
    assert!(inspection.external_assets.is_none());
    assert_eq!(before, after);
}

#[test]
fn unreadable_config_reports_problem_without_changing_legacy_defaults() {
    let fixture = Fixture::new("bad-config");
    std::fs::create_dir_all(fixture.root().join("logseq")).unwrap();
    std::fs::write(fixture.root().join("logseq/config.edn"), [0xff]).unwrap();
    let old = Graph::open_checked_with_assets(fixture.root(), None).unwrap();
    let (store, _, state) = Store::open(fixture.root(), OpenOptions::default()).unwrap();
    assert!(state.problem.is_some());
    assert_eq!(state.config.pages_dir, old.config.pages_dir);
    assert_eq!(
        store.config().problem.unwrap().kind,
        std::io::ErrorKind::InvalidData
    );
    store.close();
    assert_eq!(store.config().pages_dir, old.config.pages_dir);
}

#[test]
fn close_releases_load_waiter_and_refuses_commit() {
    let fixture = Fixture::new("close-loading");
    for i in 0..1200 {
        std::fs::write(fixture.root().join(format!("pages/{i}.md")), "- load\n").unwrap();
    }
    std::fs::write(fixture.root().join(".tine-test-pause-load"), "").unwrap();
    let (store, _, _) = Store::open(fixture.root(), OpenOptions::default()).unwrap();
    let store = Arc::new(store);
    let waiter = Arc::clone(&store);
    let (entered, started) = std::sync::mpsc::channel();
    let waiting = std::thread::spawn(move || {
        entered.send(()).unwrap();
        waiter.whole_graph().map(|_| ())
    });
    started.recv().unwrap();
    store.close();
    store.close();
    assert!(matches!(waiting.join().unwrap(), Err(LoadError::Closed)));
    let id = store.file_id(Area::Pages, "AfterClose.md").unwrap();
    let mut tx = store.transaction();
    tx.create(&id, tine_store::Content::Bytes(b"- forbidden\n".to_vec()));
    assert!(matches!(
        tx.commit(),
        TxOutcome::NotCommitted {
            why: Why::Refused(Refusal::Closed),
            ..
        }
    ));
    assert!(!fixture.root().join("pages/AfterClose.md").exists());
}

#[test]
fn whole_graph_taken_before_close_keeps_answering() {
    let fixture = Fixture::new("old-view");
    std::fs::write(fixture.root().join("pages/A.md"), "- hello\n").unwrap();
    let (store, _, _) = Store::open(fixture.root(), OpenOptions::default()).unwrap();
    let view = store.whole_graph().unwrap();
    store.close();
    assert!(view.inventory().0.iter().any(|entry| entry.name == "A"));
    assert!(matches!(store.whole_graph(), Err(LoadError::Closed)));
}

#[test]
fn same_root_scan_keeps_old_commands_open() {
    let fixture = Fixture::new("scan-load");
    std::fs::write(fixture.root().join("pages/A.md"), "- before\n").unwrap();
    let (store, _, _) = Store::open(fixture.root(), OpenOptions::default()).unwrap();
    store.scan_refresh().unwrap();
    assert!(store.whole_graph().is_ok());
    let id = store.file_id(Area::Pages, "AfterCancel.md").unwrap();
    let mut tx = store.transaction();
    tx.create(&id, tine_store::Content::Bytes(b"- after\n".to_vec()));
    assert!(matches!(tx.commit(), TxOutcome::Committed { .. }));
    store.close();
}

#[test]
fn journal_identity_stays_available_after_close_without_disk() {
    let fixture = Fixture::new("closed-journal-id");
    std::fs::write(fixture.root().join("journals/2026_09_25.md"), "- one\n").unwrap();
    let (store, _, _) = Store::open(fixture.root(), OpenOptions::default()).unwrap();
    let added = store.file_id(Area::Journals, "2026_09_26.md").unwrap();
    let mut tx = store.transaction();
    tx.create(&added, tine_store::Content::Bytes(b"- two\n".to_vec()));
    assert!(matches!(tx.commit(), TxOutcome::Committed { .. }));
    store.close();
    std::fs::remove_dir_all(fixture.root().join("journals")).unwrap();
    assert_eq!(
        store.journal_id(tine_store::Day(20260925)).as_str(),
        "journals/2026_09_25.md"
    );
    assert_eq!(
        store.journal_id(tine_store::Day(20260926)).as_str(),
        "journals/2026_09_26.md"
    );
}
