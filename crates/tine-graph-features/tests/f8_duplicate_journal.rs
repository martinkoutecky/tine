//! og family 8 (Concord): a duplicate journal day is a conflict-queue object
//! resolved at the page with an implicit row-by-row merge. Ported in meaning
//! from master 9dc54e4a7 (`model_tests.rs` duplicate-day tests).

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use tine_core::concord_queue::{ConflictObject, ConflictSource};
use tine_core::model::PageKind;
use tine_core::sync_diff::{DiffRow, RowKind};
use tine_graph_features::conflicts::{self, ConflictQueue};
use tine_store::{FileId, Store};

const KEEPER: &str = "journals/2026_06_26.md";
const STRAY: &str = "journals/Friday, 26-06-2026.md";

#[test]
fn f8_cross_kind_feed_masks_inactive_journal() {
    let _guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    for folder in ["pages", "journals"] {
        fs::create_dir(dir.path().join(folder)).unwrap();
    }
    let ordinary = "title:: Nov 6th, 2026\n- ordinary opening file\n";
    let journal = "- HIDDEN_JOURNAL_SENTINEL\n";
    fs::write(dir.path().join("pages/A.md"), ordinary).unwrap();
    fs::write(dir.path().join("journals/2026_11_06.md"), journal).unwrap();
    // This also exercises referenced-day insertion when the physical journal
    // is masked, rather than only file-backed feed membership.
    fs::write(dir.path().join("pages/Refs.md"), "- [[Nov 6th, 2026]]\n").unwrap();
    let options = || tine_store::OpenOptions {
        watch: tine_store::WatchMode::Poll,
        ..Default::default()
    };
    let store = Store::open(dir.path(), options()).unwrap().0;
    store.whole_graph().unwrap();
    let check = |store: &Store, visible: bool| {
        let view = store.whole_graph().unwrap();
        assert_eq!(view.corpus(None, false).pages.len(), 3);
        assert_eq!(
            view.corpus(None, true).pages.len(),
            if visible { 3 } else { 2 }
        );
        let mut answers = Vec::new();
        for include_references in [false, true] {
            for before in [None, Some(20261107)] {
                let feed = tine_graph_features::journals::feed_page_through(
                    store,
                    10,
                    before,
                    None,
                    include_references,
                )
                .unwrap();
                let bodies: Vec<_> = feed.pages.iter().map(|page| &page.doc.blocks).collect();
                let has_journal = format!("{bodies:?}").contains("HIDDEN_JOURNAL_SENTINEL");
                assert_eq!(
                    has_journal, visible,
                    "references={include_references}, before={before:?}"
                );
                assert!(feed
                    .pages
                    .iter()
                    .all(|page| page.doc.kind == PageKind::Journal));
                if !visible {
                    assert!(
                        feed.pages.is_empty(),
                        "an ordinary owner is not a virtual journal"
                    );
                }
                answers.push(
                    serde_json::to_value(
                        feed.pages
                            .into_iter()
                            .map(|page| page.doc)
                            .collect::<Vec<_>>(),
                    )
                    .unwrap(),
                );
            }
        }
        let physical = store
            .page(&tine_store::PageId::from(
                "journals/2026_11_06.md".to_owned(),
            ))
            .unwrap();
        assert_eq!(physical.doc.blocks[0].raw, "HIDDEN_JOURNAL_SENTINEL");
        assert_eq!(
            fs::read_to_string(dir.path().join("journals/2026_11_06.md")).unwrap(),
            journal
        );
        answers
    };
    check(&store, false);
    let conflicts = ConflictQueue::default().inventory(&store).unwrap();
    assert!(conflicts.queue.iter().any(|object| {
        object.source == ConflictSource::DuplicateName
            && object
                .sides
                .iter()
                .any(|side| side.kind == Some(PageKind::Journal))
    }));
    for (text, visible) in [
        ("title:: Elsewhere\n- ordinary opening file\n", true),
        (ordinary, false),
    ] {
        fs::write(dir.path().join("pages/A.md"), text).unwrap();
        store.refresh(tine_store::Depth::Bytes).unwrap();
        let incremental = check(&store, visible);
        let fresh = Store::open(dir.path(), options()).unwrap().0;
        fresh.whole_graph().unwrap();
        assert_eq!(incremental, check(&fresh, visible));
        fresh.close();
    }
    store.close();
}

#[test]
fn f8_cross_kind_loading_feed_masks_inactive_journal() {
    let _guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    for folder in ["pages", "journals"] {
        fs::create_dir(dir.path().join(folder)).unwrap();
    }
    fs::write(
        dir.path().join("pages/A.md"),
        "title:: Nov 6th, 2026\n- ordinary\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("journals/2026_11_06.md"),
        "- HIDDEN_JOURNAL_SENTINEL\n",
    )
    .unwrap();
    let pause = dir.path().join(".tine-test-pause-load");
    fs::write(&pause, "").unwrap();
    let store = std::sync::Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    let (send, receive) = std::sync::mpsc::channel();
    let reader = std::sync::Arc::clone(&store);
    let thread = std::thread::spawn(move || {
        send.send(tine_graph_features::journals::feed_page_through(
            &reader, 10, None, None, false,
        ))
        .unwrap();
    });
    let early = receive.recv_timeout(std::time::Duration::from_secs(5));
    let loading = !store.is_graph_ready().unwrap();
    fs::remove_file(pause).unwrap();
    thread.join().unwrap();
    assert!(loading);
    let feed = early
        .expect("file-only feed must not wait for Ready")
        .unwrap();
    assert!(feed.references_pending);
    assert!(
        feed.pages.is_empty(),
        "the title-only ordinary owner masks the journal before Ready"
    );
    store.close();
}

#[test]
fn name_conflicts_list_ordinary_and_cross_kind_files_without_writing() {
    let _guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    for folder in ["pages", "journals"] {
        fs::create_dir(dir.path().join(folder)).unwrap();
    }
    let first = "#+title: Nov 6th, 2026\n* first\n";
    let second = "title:: Nov 6th, 2026\n- second\n";
    let journal = "- journal\n";
    for (path, text) in [
        ("pages/A.org", first),
        ("pages/Z.md", second),
        ("journals/2026_11_06.md", journal),
        ("journals/2026-11-06.md", "- another journal\n"),
    ] {
        fs::write(dir.path().join(path), text).unwrap();
    }
    let store = Store::open(
        dir.path(),
        tine_store::OpenOptions {
            watch: tine_store::WatchMode::Poll,
            ..Default::default()
        },
    )
    .unwrap()
    .0;
    let queue = ConflictQueue::default();
    let check = |count| {
        let inventory = queue.inventory(&store).unwrap();
        assert_eq!(inventory.queue.len(), 1);
        let object = inventory
            .queue
            .iter()
            .find(|o| o.source == ConflictSource::DuplicateName)
            .unwrap();
        assert_eq!(object.sides.len(), count);
        assert_eq!(object.page_path, "pages/A.org");
        assert_eq!(object.sides[0].kind, Some(PageKind::Page));
        assert!(object
            .sides
            .iter()
            .any(|side| side.kind == Some(PageKind::Journal)));
    };
    check(4);
    let days = tine_graph_features::journals::journal_conflicts(&store).unwrap();
    assert!(days[0].files.iter().all(|file| !file.canonical));
    fs::remove_file(dir.path().join("pages/Z.md")).unwrap();
    store.refresh(tine_store::Depth::Stamps).unwrap();
    queue
        .refresh_files(&store, &[FileId::from("pages/Z.md".to_owned())])
        .unwrap();
    check(3);
    assert_eq!(
        fs::read_to_string(dir.path().join("pages/A.org")).unwrap(),
        first
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("journals/2026_11_06.md")).unwrap(),
        journal
    );
    // Ordinary-only events can restore or mask the journal conflict too.
    fs::remove_file(dir.path().join("pages/A.org")).unwrap();
    store.refresh(tine_store::Depth::Stamps).unwrap();
    queue
        .refresh_files(&store, &[FileId::from("pages/A.org".to_owned())])
        .unwrap();
    let promoted = queue.inventory(&store).unwrap();
    assert_eq!(promoted.queue.len(), 1);
    assert_eq!(promoted.queue[0].source, ConflictSource::DuplicateJournal);
    assert_eq!(promoted.queue[0].page_path, "journals/2026-11-06.md");
    fs::write(dir.path().join("pages/A.org"), first).unwrap();
    store.refresh(tine_store::Depth::Stamps).unwrap();
    queue
        .refresh_files(&store, &[FileId::from("pages/A.org".to_owned())])
        .unwrap();
    check(3);
    store.close();
}

fn graph(label: &str) -> PathBuf {
    let root = crate::test_support::fresh_dir(&format!("tine-f8dj-{label}-"));
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    fs::write(
        root.join("logseq/config.edn"),
        "{:journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
    )
    .unwrap();
    root
}

fn duplicate_day(label: &str) -> PathBuf {
    let root = graph(label);
    fs::write(root.join(KEEPER), "- shared line\n- only in canonical\n").unwrap();
    fs::write(root.join(STRAY), "- shared line\n- only in stray\n").unwrap();
    root
}

fn open(root: &Path) -> std::sync::Arc<Store> {
    std::sync::Arc::new(Store::open(root, Default::default()).unwrap().0)
}

fn day(queue: &[ConflictObject]) -> Option<&ConflictObject> {
    queue
        .iter()
        .find(|o| o.source == ConflictSource::DuplicateJournal)
}

fn trashed(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for path in entries.flatten().map(|entry| entry.path()) {
        if path.is_dir() {
            out.extend(trashed(&path));
        } else if let Ok(text) = fs::read_to_string(&path) {
            out.push(text);
        }
    }
    out
}

fn keep_both(rows: &[DiffRow], out: &mut HashMap<String, String>) {
    for row in rows {
        if row.kind != RowKind::Unchanged {
            out.insert(row.id.clone(), "both".to_string());
        }
        keep_both(&row.children, out);
    }
}

#[test]
fn a_duplicate_journal_day_is_a_resolvable_queue_object_with_stable_id() {
    let _integration_guard = crate::integration_lock();
    let root = duplicate_day("queue");
    let queue = conflicts::conflict_inventory(&open(&root)).unwrap().queue;
    let object = day(&queue).expect("the duplicate day is a queue object");
    assert_eq!(object.id, format!("journal:{KEEPER}"));
    assert_eq!(object.page_name, "Friday, 26-06-2026");
    assert_eq!(object.page_path, KEEPER);
    assert_eq!(object.kind, PageKind::Journal);
    let labels: Vec<_> = object.sides.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(labels, ["2026_06_26.md", "Friday, 26-06-2026.md"]);
    assert!(
        object.block_conflicts.is_some_and(|rows| rows > 0),
        "merge is implicit: real rows, got {:?}",
        object.block_conflicts
    );
    let again = conflicts::conflict_inventory(&open(&root)).unwrap().queue;
    assert_eq!(day(&again).map(|o| o.id.clone()), Some(object.id.clone()));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn resolving_keep_both_folds_the_stray_in_trashes_it_and_leaves_the_queue() {
    let _integration_guard = crate::integration_lock();
    let root = duplicate_day("resolve");
    let store = open(&root);
    let host = crate::test_host(&store);
    let queue = ConflictQueue::default();
    assert!(day(&queue.inventory(&store).unwrap().queue).is_some());
    let diff = conflicts::duplicate_journal_diff(&store, KEEPER, STRAY)
        .unwrap()
        .expect("a same-format pair diffs");
    assert!(diff.merge_base_rev.is_none(), "no common ancestor: 2-way");
    let mut decisions = HashMap::new();
    keep_both(&diff.rows, &mut decisions);
    conflicts::resolve_duplicate_journal_day(
        &store,
        &host,
        KEEPER,
        STRAY,
        &decisions,
        &diff.base_rev,
        &diff.conflict_rev,
        "union",
    )
    .unwrap();
    let kept = fs::read_to_string(root.join(KEEPER)).unwrap();
    assert!(
        kept.contains("only in canonical") && kept.contains("only in stray"),
        "{kept:?}"
    );
    assert_eq!(
        kept.matches("shared line").count(),
        1,
        "the overlap is not doubled: {kept:?}"
    );
    assert!(!root.join(STRAY).exists(), "the stray left the graph");
    assert!(
        trashed(&root.join("logseq/.tine-trash"))
            .contains(&"- shared line\n- only in stray\n".to_string()),
        "the stray's bytes are recoverable in trash"
    );
    // The settle a command performs re-derives the day without a full walk.
    queue
        .refresh_files(
            &store,
            &[
                FileId::from(KEEPER.to_owned()),
                FileId::from(STRAY.to_owned()),
            ],
        )
        .unwrap();
    assert!(day(&queue.inventory(&store).unwrap().queue).is_none());
    assert!(day(&conflicts::conflict_inventory(&open(&root)).unwrap().queue).is_none());
    drop(host);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn resolving_refuses_files_of_different_days_and_a_non_canonical_target() {
    let _integration_guard = crate::integration_lock();
    let root = duplicate_day("guard");
    fs::write(root.join("journals/2026_06_24.md"), "- other day\n").unwrap();
    let store = open(&root);
    let host = crate::test_host(&store);
    let other = "journals/2026_06_24.md";
    let refuse = |canonical: &str, stray: &str| {
        conflicts::resolve_duplicate_journal_day(
            &store,
            &host,
            canonical,
            stray,
            &HashMap::new(),
            "whatever",
            "whatever",
            "union",
        )
        .unwrap_err()
        .kind()
    };
    assert_eq!(refuse(KEEPER, other), io::ErrorKind::InvalidInput);
    assert_eq!(refuse(other, KEEPER), io::ErrorKind::NotFound);
    assert_eq!(
        refuse(STRAY, KEEPER),
        io::ErrorKind::InvalidInput,
        "stray as target"
    );
    assert_eq!(refuse(KEEPER, KEEPER), io::ErrorKind::InvalidInput);
    assert_eq!(
        fs::read_to_string(root.join(other)).unwrap(),
        "- other day\n"
    );
    assert_eq!(
        fs::read_to_string(root.join(KEEPER)).unwrap(),
        "- shared line\n- only in canonical\n"
    );
    assert!(root.join(STRAY).exists());
    drop(host);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_stale_review_writes_nothing() {
    let _integration_guard = crate::integration_lock();
    let root = duplicate_day("stale");
    let store = open(&root);
    let host = crate::test_host(&store);
    let diff = conflicts::duplicate_journal_diff(&store, KEEPER, STRAY)
        .unwrap()
        .unwrap();
    fs::write(
        root.join(STRAY),
        "- shared line\n- edited after the review\n",
    )
    .unwrap();
    let mut decisions = HashMap::new();
    keep_both(&diff.rows, &mut decisions);
    let error = conflicts::resolve_duplicate_journal_day(
        &store,
        &host,
        KEEPER,
        STRAY,
        &decisions,
        &diff.base_rev,
        &diff.conflict_rev,
        "union",
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(
        fs::read_to_string(root.join(KEEPER)).unwrap(),
        "- shared line\n- only in canonical\n"
    );
    assert!(root.join(STRAY).exists());
    drop(host);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_cross_format_day_lists_its_files_offers_no_rows_and_refuses_a_fold() {
    let _integration_guard = crate::integration_lock();
    let root = graph("cross-format");
    fs::write(root.join(KEEPER), "- markdown\n").unwrap();
    fs::write(root.join("journals/Friday, 26-06-2026.org"), "* org\n").unwrap();
    let store = open(&root);
    let host = crate::test_host(&store);
    let queue = conflicts::conflict_inventory(&store).unwrap().queue;
    let object = day(&queue).expect("still a queue object");
    assert_eq!(object.sides.len(), 2, "both files are listed");
    assert!(
        object.block_conflicts.is_none(),
        "no row choice on an unmergeable pair"
    );
    let org = "journals/Friday, 26-06-2026.org";
    assert!(conflicts::duplicate_journal_diff(&store, KEEPER, org)
        .unwrap()
        .is_none());
    let error = conflicts::resolve_duplicate_journal_day(
        &store,
        &host,
        KEEPER,
        org,
        &HashMap::new(),
        "x",
        "y",
        "union",
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(root.join(org).exists());
    drop(host);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn an_external_stray_enters_the_open_queue_on_its_change_event() {
    let _integration_guard = crate::integration_lock();
    let root = graph("external");
    fs::write(root.join(KEEPER), "- mine\n").unwrap();
    let store = open(&root);
    let queue = ConflictQueue::default();
    assert!(day(&queue.inventory(&store).unwrap().queue).is_none());
    fs::write(root.join(STRAY), "- theirs\n").unwrap();
    store.refresh(tine_store::Depth::Stamps).unwrap();
    assert!(queue
        .refresh_files(&store, &[FileId::from(STRAY.to_owned())])
        .unwrap());
    let object = day(&queue.inventory(&store).unwrap().queue).map(|o| o.id.clone());
    assert_eq!(object, Some(format!("journal:{KEEPER}")));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_journal_title_format_change_that_reveals_a_twin_queues_the_day() {
    let _integration_guard = crate::integration_lock();
    let root = graph("reveal");
    fs::write(root.join("logseq/config.edn"), "{}\n").unwrap();
    fs::write(root.join(KEEPER), "- mine\n").unwrap();
    fs::write(root.join(STRAY), "- theirs\n").unwrap();
    let store = open(&root);
    let queue = ConflictQueue::default();
    assert!(
        day(&queue.inventory(&store).unwrap().queue).is_none(),
        "under the default format the title-named file is an ordinary page"
    );
    // The Settings path, observed through the change feed exactly as the
    // watcher hook (`concord_observe`) sees it: an own-origin config change.
    let changes = store.subscribe();
    tine_graph_features::config::set_journal_page_title_format(&store, "EEEE, dd-MM-yyyy").unwrap();
    let mut changed = false;
    while let Ok(Some(change)) = changes.try_recv() {
        changed |= queue.refresh_change(&store, &change).unwrap();
    }
    assert!(changed, "the config change re-derives the duplicate days");
    let queued = day(&queue.inventory(&store).unwrap().queue).map(|o| o.page_name.clone());
    assert_eq!(queued.as_deref(), Some("Friday, 26-06-2026"));
    let _ = fs::remove_dir_all(&root);
}

/// The in-page resolver mounts on the page whose file is the object's
/// `page_path`; opening the day by its title must read that keeper file.
#[test]
fn opening_the_day_by_title_reads_the_file_the_queue_object_names() {
    let _integration_guard = crate::integration_lock();
    let root = duplicate_day("open");
    let store = open(&root);
    store.refresh(tine_store::Depth::Stamps).unwrap();
    let queue = conflicts::conflict_inventory(&store).unwrap().queue;
    let object = day(&queue).expect("queued");
    let read = tine_graph_features::pages::get_page(&store, &object.page_name, PageKind::Journal)
        .ok()
        .flatten()
        .expect("the day opens");
    assert_eq!(read.id.as_str(), object.page_path);
    let _ = fs::remove_dir_all(&root);
}

/// An Org graph whose day has a date-stem file and a title-named stray, the
/// shape the store's former test-only copy of `journal_conflicts` was tested
/// on; its assertions now run against the production listing.
fn org_title_format_graph(label: &str) -> PathBuf {
    let root = graph(label);
    fs::write(
        root.join("logseq/config.edn"),
        "{:preferred-format \"Org\"\n :journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
    )
    .unwrap();
    root
}

/// Ported from `tine-store` model.rs `journal_conflicts_reports_duplicate_days`.
#[test]
fn journal_conflicts_reports_duplicate_days() {
    let _integration_guard = crate::integration_lock();
    let root = org_title_format_graph("dup-days");
    fs::write(
        root.join("journals/2026_06_26.org"),
        "* canonical content\n",
    )
    .unwrap();
    fs::write(
        root.join("journals/Friday, 26-06-2026.org"),
        "* stray content\n",
    )
    .unwrap();
    // A clean day with one file: not a conflict.
    fs::write(root.join("journals/2026_06_24.org"), "* fine\n").unwrap();
    let store = open(&root);
    store.whole_graph().unwrap();
    let conflicts = tine_graph_features::journals::journal_conflicts(&store).unwrap();
    assert_eq!(
        conflicts.len(),
        1,
        "exactly one conflicted day: {conflicts:?}"
    );
    let c = &conflicts[0];
    assert_eq!(c.title, "Friday, 26-06-2026");
    assert_eq!(c.files.len(), 2);
    // The date-stem file sorts first and is flagged; preview is the body line.
    assert_eq!(c.files[0].name, "2026_06_26.org");
    assert!(c.files[0].canonical);
    assert_eq!(c.files[0].preview, "canonical content");
    assert!(!c.files[1].canonical);
    assert_eq!(c.files[1].name, "Friday, 26-06-2026.org");
    let _ = fs::remove_dir_all(&root);
}

/// Ported from `tine-store` model.rs
/// `journal_conflicts_reports_nested_duplicate_days`.
#[test]
fn journal_conflicts_reports_nested_duplicate_days() {
    let _integration_guard = crate::integration_lock();
    let root = org_title_format_graph("dup-days-nested");
    fs::create_dir_all(root.join("journals/archive")).unwrap();
    fs::write(
        root.join("journals/archive/2026_06_26.org"),
        "* canonical nested\n",
    )
    .unwrap();
    fs::write(
        root.join("journals/archive/Friday, 26-06-2026.org"),
        "* stray nested\n",
    )
    .unwrap();
    let store = open(&root);
    store.whole_graph().unwrap();
    let conflicts = tine_graph_features::journals::journal_conflicts(&store).unwrap();
    assert_eq!(
        conflicts.len(),
        1,
        "nested duplicate day is surfaced: {conflicts:?}"
    );
    let files = &conflicts[0].files;
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].path, "journals/archive/2026_06_26.org");
    assert_eq!(files[1].path, "journals/archive/Friday, 26-06-2026.org");
    let _ = fs::remove_dir_all(&root);
}

/// Ported from `tine-store` model.rs
/// `journal_conflicts_expose_a_routable_path_per_file`.
#[test]
fn journal_conflicts_expose_a_routable_path_per_file() {
    let _integration_guard = crate::integration_lock();
    let root = org_title_format_graph("dup-days-path");
    fs::write(root.join("journals/2026_06_26.org"), "* canonical body\n").unwrap();
    fs::write(
        root.join("journals/Friday, 26-06-2026.org"),
        "* stray body\n",
    )
    .unwrap();
    let store = open(&root);
    store.whole_graph().unwrap();
    let conflicts = tine_graph_features::journals::journal_conflicts(&store).unwrap();
    assert_eq!(conflicts.len(), 1, "one duplicated day");
    let files = &conflicts[0].files;
    assert_eq!(files.len(), 2);
    assert!(files[0].canonical);
    assert_eq!(files[0].path, "journals/2026_06_26.org");
    assert_eq!(files[1].path, "journals/Friday, 26-06-2026.org");
    for f in files {
        assert!(
            store
                .page(&tine_store::PageId::from(f.path.clone()))
                .is_ok(),
            "conflict path resolves: {}",
            f.path
        );
    }
    let _ = fs::remove_dir_all(&root);
}

/// CM step 2 OW-2 (UI-CM-DUPLICATE-DAY-KEEPER, PQ-1): with `2026_10_26.md`
/// (the stem the default format writes) and `2026-10-26.md` (an ISO stem,
/// also a date name), the day opens `2026_10_26.md`. The duplicate-day list
/// must name that file as the one keeper, so Settings offers Merge for the
/// other and the conflict-queue merge keeps the page the user sees.
#[test]
fn the_duplicate_day_keeper_is_the_file_the_day_opens() {
    let _integration_guard = crate::integration_lock();
    let root = crate::test_support::fresh_dir("tine-f8dj-ow2-");
    for dir in ["pages", "journals", "assets"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    let opened_file = "journals/2026-10-26.md";
    let iso_file = "journals/2026_10_26.md";
    fs::write(
        root.join(opened_file),
        "- shared line\n- only in underscore\n",
    )
    .unwrap();
    fs::write(root.join(iso_file), "- shared line\n- only in iso\n").unwrap();
    let store = open(&root);
    let host = crate::test_host(&store);
    store.whole_graph().unwrap();
    let conflicts = tine_graph_features::journals::journal_conflicts(&store).unwrap();
    assert_eq!(conflicts.len(), 1, "{conflicts:?}");
    let day = &conflicts[0];
    let opened = store
        .page_named(&day.title, PageKind::Journal)
        .unwrap()
        .expect("the day opens");
    assert_eq!(opened.id.as_str(), opened_file, "claimant order");
    let files: Vec<(&str, bool)> = day
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.canonical))
        .collect();
    assert_eq!(
        files,
        vec![(opened_file, true), (iso_file, false)],
        "the keeper is the file the day opens, and the only canonical one"
    );
    // The merge keeps the opened file and folds the other into it.
    let diff = conflicts::duplicate_journal_diff(&store, opened_file, iso_file)
        .unwrap()
        .expect("a same-format pair diffs");
    let mut decisions = HashMap::new();
    keep_both(&diff.rows, &mut decisions);
    conflicts::resolve_duplicate_journal_day(
        &store,
        &host,
        opened_file,
        iso_file,
        &decisions,
        &diff.base_rev,
        &diff.conflict_rev,
        "union",
    )
    .unwrap();
    let kept = fs::read_to_string(root.join(opened_file)).unwrap();
    assert!(
        kept.contains("only in underscore") && kept.contains("only in iso"),
        "{kept:?}"
    );
    assert!(
        !root.join(iso_file).exists(),
        "the other file left the graph"
    );
    drop(host);
    let _ = fs::remove_dir_all(&root);
}
