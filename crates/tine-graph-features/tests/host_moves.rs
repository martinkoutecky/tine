//! host-only-writer P3a: the two moves with bytes unchanged, the stray
//! rescue (`rename_file_to_page`, census W2) and the journal filename
//! migration (`migrate_journal_filenames`, W9), are host operations
//! (DESIGN §3.1). The move is planned from the source's bytes with no
//! fence; a source edited between the plan and the host's admission is
//! never moved as the planned bytes: the operation refuses `Changed`, and
//! the writer replans and moves the edited bytes. A target that appeared
//! since the plan is never overwritten.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use tine_graph_features::{journals, pages};
use tine_store::{Input, PageHost, PageId, Store};

const CONFIG: &str = "{:file/name-format :triple-lowbar}\n";

struct Hosted {
    _dir: tempfile::TempDir,
    root: PathBuf,
    store: Arc<Store>,
    host: PageHost,
}

fn hosted(files: &[(&str, &str)]) -> Hosted {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("graph");
    for sub in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
    fs::write(root.join("logseq/config.edn"), CONFIG).unwrap();
    for (rel, text) in files {
        fs::write(root.join(rel), text).unwrap();
    }
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    let host = PageHost::start_for_tests(&store, &dir.path().join("app")).unwrap();
    Hosted {
        _dir: dir,
        root,
        store,
        host,
    }
}

/// The text files under `area`, by name.
fn files(root: &Path, area: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = fs::read_dir(root.join(area))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, fs::read_to_string(&path).unwrap())
        })
        .collect();
    out.sort();
    out
}

/// Run `write` while another reservation holds `held`: the writer plans
/// from `held`'s current bytes and waits at the host's admission. Then an
/// external editor rewrites `held` to `edited`, and the reservation ends.
/// The writer's result.
fn edited_between_plan_and_admission(
    f: &Hosted,
    held: &str,
    edited: &str,
    write: impl Fn(&Store, &PageHost) -> io::Result<()> + Sync,
) -> io::Result<()> {
    let reservation = f
        .host
        .reserve(|| vec![PageId::from(held)], Input::Refuse)
        .expect("nothing unsaved");
    std::thread::scope(|scope| {
        let (done, finished) = mpsc::channel();
        let (store, host, write) = (&*f.store, &f.host, &write);
        scope.spawn(move || {
            let _ = done.send(write(store, host));
        });
        assert!(
            finished.recv_timeout(Duration::from_millis(500)).is_err(),
            "the writer finished while another reservation holds {held}"
        );
        fs::write(f.root.join(held), edited).unwrap();
        drop(reservation);
        finished
            .recv_timeout(Duration::from_secs(30))
            .expect("the writer finishes once the page is released")
    })
}

#[test]
fn a_rescue_with_a_host_moves_the_file_unchanged() {
    let _integration_guard = crate::integration_lock();
    let f = hosted(&[
        ("journals/Loose.md", "- loose\n"),
        ("pages/Taken.md", "- taken\n"),
    ]);
    pages::rename_file_to_page(&f.store, Some(&f.host), "journals/Loose.md", "Rescued").unwrap();
    assert_eq!(
        files(&f.root, "pages"),
        [
            ("Rescued.md".to_owned(), "- loose\n".to_owned()),
            ("Taken.md".to_owned(), "- taken\n".to_owned()),
        ]
    );
    assert!(files(&f.root, "journals").is_empty());
    // A name another page owns refuses and moves nothing.
    fs::write(f.root.join("journals/Other.md"), "- other\n").unwrap();
    f.store.refresh(tine_store::Depth::Stamps).unwrap();
    let refused = pages::rename_file_to_page(&f.store, Some(&f.host), "journals/Other.md", "Taken")
        .unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::AlreadyExists, "{refused}");
    assert_eq!(
        fs::read_to_string(f.root.join("pages/Taken.md")).unwrap(),
        "- taken\n"
    );
    assert_eq!(
        fs::read_to_string(f.root.join("journals/Other.md")).unwrap(),
        "- other\n"
    );
}

/// A sync-conflict source is not a page. Its destination still belongs to
/// the host, including the host's publication and edit-kind delivery.
#[test]
fn a_non_page_rescue_is_published_by_the_host() {
    let _integration_guard = crate::integration_lock();
    let src = "pages/A.sync-conflict-20261012-000000-DEVICE.md";
    let f = hosted(&[(src, "- rescued copy\n")]);
    assert!(f
        .store
        .as_page(&tine_store::FileId::from(src.to_owned()))
        .is_none());
    drop(f.host);
    let (sent, received) = mpsc::channel();
    let host = PageHost::start(
        &f.store,
        &f._dir.path().join("app"),
        "test-graph",
        move |page, kinds| {
            sent.send((page.to_owned(), kinds.to_vec())).unwrap();
        },
        |_| {},
    )
    .unwrap();
    pages::rename_file_to_page(&f.store, Some(&host), src, "Rescued").unwrap();
    let (page, kinds) = received
        .recv_timeout(Duration::from_secs(2))
        .expect("the destination must publish through the host");
    assert_eq!(page, "pages/Rescued.md");
    assert!(kinds.contains(&tine_store::EditKind::RenamePage));
    assert!(!f.root.join(src).exists());
    assert_eq!(
        fs::read_to_string(f.root.join("pages/Rescued.md")).unwrap(),
        "- rescued copy\n"
    );
}

#[test]
fn a_non_page_rescue_preserves_invalid_utf8_with_and_without_a_host() {
    let _integration_guard = crate::integration_lock();
    let src = "pages/A.sync-conflict-20261012-000000-DEVICE.md";
    let bytes = b"- imported \xff\n";
    for hosted_path in [false, true] {
        let f = hosted(&[]);
        fs::write(f.root.join(src), bytes).unwrap();
        f.store.refresh(tine_store::Depth::Bytes).unwrap();
        assert!(f
            .store
            .as_page(&tine_store::FileId::from(src.to_owned()))
            .is_none());
        pages::rename_file_to_page(&f.store, hosted_path.then_some(&f.host), src, "Rescued")
            .unwrap();
        assert_eq!(fs::read(f.root.join("pages/Rescued.md")).unwrap(), bytes);
        assert!(!f.root.join(src).exists());
    }
}

#[test]
fn a_rescue_planned_before_an_external_edit_moves_the_edited_bytes() {
    let _integration_guard = crate::integration_lock();
    let f = hosted(&[("journals/Loose.md", "- planned\n")]);
    edited_between_plan_and_admission(&f, "journals/Loose.md", "- edited\n", |store, host| {
        pages::rename_file_to_page(store, Some(host), "journals/Loose.md", "Rescued")
    })
    .unwrap();
    assert_eq!(
        files(&f.root, "pages"),
        [("Rescued.md".to_owned(), "- edited\n".to_owned())],
        "the rescue moved bytes the source no longer held"
    );
    assert!(files(&f.root, "journals").is_empty());
}

#[test]
fn a_journal_migration_with_a_host_moves_each_file_unchanged() {
    let _integration_guard = crate::integration_lock();
    let f = hosted(&[
        ("journals/Jun 19th, 2026.md", "- a\n"),
        ("journals/Jun 21st, 2026.md", "- b\n"),
    ]);
    let listed = journals::journal_filename_migrations(&f.store).unwrap();
    assert_eq!(listed.len(), 2, "{listed:?}");
    let result = journals::migrate_journal_filenames(&f.store, Some(&f.host), &listed).unwrap();
    assert_eq!(
        (result.migrated, result.skipped.len()),
        (2, 0),
        "{result:?}"
    );
    assert_eq!(
        files(&f.root, "journals"),
        [
            ("2026_06_19.md".to_owned(), "- a\n".to_owned()),
            ("2026_06_21.md".to_owned(), "- b\n".to_owned()),
        ]
    );
}

#[test]
fn a_journal_migration_planned_before_an_external_edit_moves_the_edited_bytes() {
    let _integration_guard = crate::integration_lock();
    let f = hosted(&[("journals/Jun 19th, 2026.md", "- planned\n")]);
    let listed = journals::journal_filename_migrations(&f.store).unwrap();
    edited_between_plan_and_admission(
        &f,
        "journals/Jun 19th, 2026.md",
        "- edited\n",
        |store, host| {
            let result = journals::migrate_journal_filenames(store, Some(host), &listed)?;
            assert_eq!(result.migrated, 1, "{:?}", result.skipped);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(
        files(&f.root, "journals"),
        [("2026_06_19.md".to_owned(), "- edited\n".to_owned())],
        "the migration moved bytes the source no longer held"
    );
}

/// A sync service delivers the target after the listing: the proposal is
/// skipped, and neither file changes.
#[test]
fn a_journal_target_that_appears_after_the_listing_is_never_overwritten() {
    let _integration_guard = crate::integration_lock();
    let f = hosted(&[("journals/Jun 19th, 2026.md", "- mine\n")]);
    let listed = journals::journal_filename_migrations(&f.store).unwrap();
    assert_eq!(listed.len(), 1, "{listed:?}");
    fs::write(f.root.join("journals/2026_06_19.md"), "- delivered\n").unwrap();
    let result = journals::migrate_journal_filenames(&f.store, Some(&f.host), &listed).unwrap();
    assert_eq!(result.migrated, 0);
    let skipped: Vec<&str> = result.skipped.iter().map(|s| s.file.as_str()).collect();
    assert_eq!(skipped, ["Jun 19th, 2026.md"], "{result:?}");
    assert_eq!(
        files(&f.root, "journals"),
        [
            ("2026_06_19.md".to_owned(), "- delivered\n".to_owned()),
            ("Jun 19th, 2026.md".to_owned(), "- mine\n".to_owned()),
        ]
    );
}
