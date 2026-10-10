//! og family 8 (Concord): a duplicate journal day is a conflict-queue object
//! resolved at the page with an implicit row-by-row merge. Ported in meaning
//! from master 9dc54e4a7 (`model_tests.rs` duplicate-day tests).

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use tine_core::concord_queue::{ConflictObject, ConflictSource};
use tine_core::model::PageKind;
use tine_core::sync_diff::{DiffRow, RowKind};
use tine_graph_features::conflicts::{self, ConflictQueue};
use tine_store::{FileId, Store};

const KEEPER: &str = "journals/2026_06_26.md";
const STRAY: &str = "journals/Friday, 26-06-2026.md";

fn graph(label: &str) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-f8dj-{label}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
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

fn open(root: &Path) -> Store {
    Store::open(root, Default::default()).unwrap().0
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
    let root = duplicate_day("resolve");
    let store = open(&root);
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
        None,
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
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn resolving_refuses_files_of_different_days_and_a_non_canonical_target() {
    let root = duplicate_day("guard");
    fs::write(root.join("journals/2026_06_24.md"), "- other day\n").unwrap();
    let store = open(&root);
    let other = "journals/2026_06_24.md";
    let refuse = |canonical: &str, stray: &str| {
        conflicts::resolve_duplicate_journal_day(
            &store,
            None,
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
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_stale_review_writes_nothing() {
    let root = duplicate_day("stale");
    let store = open(&root);
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
        None,
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
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_cross_format_day_lists_its_files_offers_no_rows_and_refuses_a_fold() {
    let root = graph("cross-format");
    fs::write(root.join(KEEPER), "- markdown\n").unwrap();
    fs::write(root.join("journals/Friday, 26-06-2026.org"), "* org\n").unwrap();
    let store = open(&root);
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
        None,
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
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn an_external_stray_enters_the_open_queue_on_its_change_event() {
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
