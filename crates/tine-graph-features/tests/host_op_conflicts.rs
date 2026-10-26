//! The Concord folds as host operations (host-only-writer P3 lane c, DESIGN
//! §3.1, §3.4): a sync copy's or a duplicate journal day's fold writes the
//! winner (and deletes a duplicate day's stray) as one `HostOp` planned
//! from the bytes it read, so a page that changed after the plan is never
//! overwritten, a sync copy (no page) is trashed only after the winner
//! applied, and a kill between the winner's write and the stray's deletion
//! completes at relaunch instead of leaving the stray to fold in twice.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use tine_core::sync_diff::{DiffRow, RowKind};
use tine_graph_features::conflicts;
use tine_store::{FaultPoint, Input, PageHost, PageId, Store};

const CONFIG: &str = "{:journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n";
const WINNER: &str = "pages/Foo.md";
const COPY: &str = "pages/Foo.sync-conflict-20260705-120000-ABCDEFG.md";
const KEEPER: &str = "journals/2026_06_26.md";
const STRAY: &str = "journals/Friday, 26-06-2026.md";
const COPY_TEXT: &str = "- shared\n- theirs\n";
const KEEPER_TEXT: &str = "- shared line\n- only in canonical\n";
const STRAY_TEXT: &str = "- shared line\n- only in stray\n";

fn graph(label: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-host-op-conflicts-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    fs::write(root.join("logseq/config.edn"), CONFIG).unwrap();
    fs::write(root.join(WINNER), "- shared\n- mine\n").unwrap();
    fs::write(root.join(COPY), COPY_TEXT).unwrap();
    fs::write(root.join(KEEPER), KEEPER_TEXT).unwrap();
    fs::write(root.join(STRAY), STRAY_TEXT).unwrap();
    root
}

fn open(root: &Path) -> Arc<Store> {
    let store = Arc::new(Store::open(root, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    store
}

/// The graph's page and journal files (trash locations carry stamps).
fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    for area in ["pages", "journals"] {
        for entry in fs::read_dir(root.join(area)).unwrap() {
            let path = entry.unwrap().path();
            let rel = format!("{area}/{}", path.file_name().unwrap().to_string_lossy());
            out.insert(rel, fs::read(&path).unwrap());
        }
    }
    out
}

/// The texts of every file under `dir`, recursively.
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
            out.insert(row.id.clone(), "both".to_owned());
        }
        keep_both(&row.children, out);
    }
}

#[derive(Clone, Copy, Debug)]
enum Fold {
    SyncCopy,
    DuplicateDay,
}

impl Fold {
    fn pair(self) -> (&'static str, &'static str) {
        match self {
            Fold::SyncCopy => (WINNER, COPY),
            Fold::DuplicateDay => (KEEPER, STRAY),
        }
    }

    /// The folded file's text, as `graph` writes it.
    fn other_text(self) -> &'static str {
        match self {
            Fold::SyncCopy => COPY_TEXT,
            Fold::DuplicateDay => STRAY_TEXT,
        }
    }

    /// The fold of the pair as reviewed now: keep both sides of every row.
    fn review(self, store: &Store) -> (HashMap<String, String>, String, String) {
        let (winner, other) = self.pair();
        let diff = match self {
            Fold::SyncCopy => conflicts::sync_conflict_diff(store, winner, other, &[]),
            Fold::DuplicateDay => conflicts::duplicate_journal_diff(store, winner, other),
        }
        .unwrap()
        .expect("the pair diffs");
        let mut decisions = HashMap::new();
        keep_both(&diff.rows, &mut decisions);
        (decisions, diff.base_rev, diff.conflict_rev)
    }

    fn run(
        self,
        store: &Store,
        host: Option<&PageHost>,
        (decisions, base, other): &(HashMap<String, String>, String, String),
    ) -> std::io::Result<()> {
        let (winner, copy) = self.pair();
        match self {
            Fold::SyncCopy => conflicts::resolve_sync_conflict(
                store,
                host,
                winner,
                copy,
                decisions,
                base,
                other,
                None,
                &[],
                "union",
            ),
            Fold::DuplicateDay => conflicts::resolve_duplicate_journal_day(
                store, host, winner, copy, decisions, base, other, "union",
            ),
        }
    }
}

/// A fold with a host leaves the graph byte-for-byte as the plain fold does,
/// and the folded file's bytes stay recoverable in graph trash.
#[test]
fn a_fold_with_a_host_writes_what_the_plain_fold_writes() {
    let _integration_guard = crate::integration_lock();
    for fold in [Fold::SyncCopy, Fold::DuplicateDay] {
        let plain = graph("plain");
        let store = open(&plain);
        let review = fold.review(&store);
        fold.run(&store, None, &review).unwrap();
        store.close();
        let hosted = graph("hosted");
        let app_data = tempfile::tempdir().unwrap();
        let store = open(&hosted);
        let host = PageHost::start_for_tests(&store, app_data.path()).unwrap();
        let review = fold.review(&store);
        fold.run(&store, Some(&host), &review).unwrap();
        drop(host);
        store.close();
        assert_eq!(
            tree(&hosted),
            tree(&plain),
            "{fold:?}: a fold's bytes changed under a host"
        );
        let (_, other) = fold.pair();
        assert!(!hosted.join(other).exists(), "{fold:?}: folded file kept");
        let text = fs::read_to_string(plain.join(fold.pair().0)).unwrap();
        assert!(
            text.contains("theirs") || text.contains("only in stray"),
            "{fold:?}: the other side was not folded in: {text:?}"
        );
        assert!(
            trashed(&hosted.join("logseq/.tine-trash")).contains(&fold.other_text().to_owned()),
            "{fold:?}: the folded file's bytes are not recoverable in trash"
        );
        let _ = fs::remove_dir_all(plain);
        let _ = fs::remove_dir_all(hosted);
    }
}

/// While a reservation holds the winner, the fold (with a host) starts and
/// waits; `change` then edits the graph on disk, as an external editor or a
/// sync service would after the review, and the reservation is released.
/// The fold's result, and the graph after it.
fn fold_changed_meanwhile(
    fold: Fold,
    change: impl FnOnce(&Path),
) -> (std::io::Result<()>, PathBuf) {
    let root = graph("raced");
    let app_data = tempfile::tempdir().unwrap();
    let store = open(&root);
    let host = PageHost::start_for_tests(&store, app_data.path()).unwrap();
    let review = fold.review(&store);
    let held = host
        .reserve(|| vec![PageId::from(fold.pair().0)], Input::Refuse)
        .expect("nothing unsaved");
    let result = std::thread::scope(|scope| {
        let (done, finished) = mpsc::channel();
        let (store, host, review) = (&store, &host, &review);
        scope.spawn(move || {
            let _ = done.send(fold.run(store, Some(host), review));
        });
        // The fold reads and plans now; it cannot apply while the winner
        // is held.
        std::thread::sleep(Duration::from_millis(300));
        change(&root);
        drop(held);
        finished
            .recv_timeout(Duration::from_secs(30))
            .expect("the fold finishes once the winner is released")
    });
    drop(host);
    store.close();
    (result, root)
}

/// The optimistic admission (DESIGN §3.1): a page that changed after the
/// fold read it is never overwritten; the fold replans, sees the change
/// against the review, and refuses as before, writing nothing.
#[test]
fn a_page_changed_after_the_plan_is_never_overwritten() {
    let _integration_guard = crate::integration_lock();
    for fold in [Fold::SyncCopy, Fold::DuplicateDay] {
        let (winner, other) = fold.pair();
        let (result, root) = fold_changed_meanwhile(fold, |root| {
            fs::write(root.join(winner), "- external edit\n").unwrap();
        });
        let error = result.expect_err("a fold over a changed winner refuses");
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists, "{error}");
        assert_eq!(error.to_string(), "winner changed on disk");
        assert_eq!(fs::read(root.join(winner)).unwrap(), b"- external edit\n");
        assert_eq!(
            fs::read_to_string(root.join(other)).unwrap(),
            fold.other_text()
        );
        let _ = fs::remove_dir_all(root);
    }
    // A duplicate day's stray is a page of the operation too.
    let (result, root) = fold_changed_meanwhile(Fold::DuplicateDay, |root| {
        fs::write(root.join(STRAY), "- stray edited\n").unwrap();
    });
    let error = result.expect_err("a fold over a changed stray refuses");
    assert_eq!(error.to_string(), "conflict copy changed on disk");
    assert_eq!(fs::read(root.join(KEEPER)).unwrap(), KEEPER_TEXT.as_bytes());
    assert_eq!(fs::read(root.join(STRAY)).unwrap(), b"- stray edited\n");
    let _ = fs::remove_dir_all(root);
}

/// A sync copy is no page, so it is trashed after the winner's operation
/// applied (DESIGN §3.4, post), guarded by the bytes the fold read: a copy
/// that changed after the review is never trashed, and the error says so.
#[test]
fn a_sync_copy_changed_after_the_plan_is_never_trashed() {
    let _integration_guard = crate::integration_lock();
    let (result, root) = fold_changed_meanwhile(Fold::SyncCopy, |root| {
        fs::write(root.join(COPY), "- theirs, delivered again\n").unwrap();
    });
    let error = result.expect_err("the changed copy is reported");
    // The winner's write completed: the error says so (P3 review #7).
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists, "{error}");
    assert_eq!(
        error.to_string(),
        "The merge was saved, but the conflict copy changed on disk and was kept"
    );
    assert_eq!(
        fs::read(root.join(COPY)).unwrap(),
        b"- theirs, delivered again\n"
    );
    // The winner took the copy the user reviewed (the operation applied
    // before the post step found the copy changed): the reported conflict
    // re-reviews the merged winner against the copy's new bytes.
    let winner = fs::read_to_string(root.join(WINNER)).unwrap();
    assert!(
        winner.contains("- mine") && winner.contains("- theirs\n"),
        "{winner:?}"
    );
    let _ = fs::remove_dir_all(root);
}

/// The child of `a_duplicate_day_fold_killed_before_the_strays_deletion_*`:
/// folds the day with a host and is killed after the keeper's write, just
/// before the stray's removal, whichever path does it (the host's trash
/// move, or the second step of a store transaction).
#[test]
fn crash_duplicate_day_fold_worker() {
    let _integration_guard = crate::integration_lock();
    let (Ok(root), Ok(app_data)) = (
        std::env::var("TINE_FOLD_CRASH_ROOT"),
        std::env::var("TINE_FOLD_CRASH_APP_DATA"),
    ) else {
        return;
    };
    use tine_store::host_faults::{abort_before, Phase};
    let store = open(Path::new(&root));
    let review = Fold::DuplicateDay.review(&store);
    abort_before(Phase::TrashMove, 0);
    store.inject_fault(FaultPoint::AbortAfterStep(0));
    let host = PageHost::start_for_tests(&store, Path::new(&app_data)).unwrap();
    let result = Fold::DuplicateDay.run(&store, Some(&host), &review);
    panic!("the fold was not killed before the stray's removal: {result:?}");
}

/// DESIGN §3.7 (CENSUS W6 crash): the keeper's write and the stray's
/// deletion are one operation draft, so a kill between them completes at
/// relaunch: the stray leaves the graph and is never left to fold in again.
#[test]
fn a_duplicate_day_fold_killed_before_the_strays_deletion_completes_at_relaunch() {
    let _integration_guard = crate::integration_lock();
    let root = graph("crash");
    let app_data = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "host_op_conflicts::crash_duplicate_day_fold_worker",
            "--nocapture",
        ])
        .env("TINE_FOLD_CRASH_ROOT", &root)
        .env("TINE_FOLD_CRASH_APP_DATA", app_data.path())
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "the child must be killed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let folded = fs::read_to_string(root.join(KEEPER)).unwrap();
    assert!(folded.contains("only in stray"), "{folded:?}");
    assert!(
        root.join(STRAY).exists(),
        "killed before the stray's removal"
    );
    let store = open(&root);
    let host = PageHost::start_for_tests(&store, app_data.path()).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while root.join(STRAY).exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the relaunch never removed the stray: a rerun would fold it in twice"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(fs::read_to_string(root.join(KEEPER)).unwrap(), folded);
    assert!(
        trashed(&root.join("logseq/.tine-trash")).contains(&STRAY_TEXT.to_owned()),
        "the stray's bytes are recoverable in trash"
    );
    drop(host);
    store.close();
    let _ = fs::remove_dir_all(root);
}

/// W7 needs no page fence: every name `trash_sync_conflict` accepts (a
/// Syncthing, Dropbox or Seafile conflict copy of a page or journal) is no
/// page by the store's definition, `Store::as_page`; and the copy is
/// trashed recoverably with a host.
#[test]
fn a_sync_copy_is_never_a_page() {
    let _integration_guard = crate::integration_lock();
    let root = graph("copies");
    let copies = [
        COPY,
        "pages/Foo (Martin's conflicted copy 2026-07-05).md",
        "pages/Foo (SFConflict martin@example.com 2026-07-05-12-00-00).md",
        "journals/2026_06_26.sync-conflict-20260705-120000-ABCDEFG.org",
    ];
    for copy in copies {
        fs::write(root.join(copy), "- a copy\n").unwrap();
    }
    let app_data = tempfile::tempdir().unwrap();
    let store = open(&root);
    let host = PageHost::start_for_tests(&store, app_data.path()).unwrap();
    for copy in copies {
        let (area, name) = copy.split_once('/').unwrap();
        let area = match area {
            "pages" => tine_store::Area::Pages,
            _ => tine_store::Area::Journals,
        };
        let file = store.file_id(area, name).unwrap();
        assert_eq!(store.as_page(&file), None, "{copy} is a page");
        conflicts::trash_sync_conflict(&store, Some(&host), copy).unwrap();
        assert!(!root.join(copy).exists(), "{copy} was not trashed");
    }
    // A page is not a sync copy: refused, untouched.
    assert!(conflicts::trash_sync_conflict(&store, Some(&host), WINNER).is_err());
    assert!(root.join(WINNER).exists());
    drop(host);
    store.close();
    let _ = fs::remove_dir_all(root);
}

const MARKED: &str = "pages/Marked.md";
const MARKED_TEXT: &str = "- before\n<<<<<<< ours\n- mine\n=======\n- theirs\n>>>>>>> theirs\n";

/// The review of `MARKED` now: keep both sides of every changed row.
fn marker_review(store: &Store) -> (HashMap<String, String>, String) {
    let diff = conflicts::vcs_marker_conflict_diff(store, MARKED)
        .unwrap()
        .expect("markers");
    let mut decisions = HashMap::new();
    keep_both(&diff.diff.rows, &mut decisions);
    (decisions, diff.diff.base_rev)
}

fn resolve_markers(
    store: &Store,
    host: Option<&PageHost>,
    (decisions, base): &(HashMap<String, String>, String),
) -> std::io::Result<()> {
    conflicts::resolve_vcs_marker_conflict(store, host, MARKED, decisions, base, "union")
}

/// W8 (P3 combine): a marker resolution with a host writes what the plain
/// one writes, and the marker bytes are in conflict trash, staged before the
/// operation (DESIGN §3.4 pre).
#[test]
fn a_marker_resolution_with_a_host_writes_what_the_plain_one_writes() {
    let _integration_guard = crate::integration_lock();
    let plain = graph("markers-plain");
    fs::write(plain.join(MARKED), MARKED_TEXT).unwrap();
    let store = open(&plain);
    let review = marker_review(&store);
    resolve_markers(&store, None, &review).unwrap();
    store.close();
    let hosted = graph("markers-hosted");
    fs::write(hosted.join(MARKED), MARKED_TEXT).unwrap();
    let app_data = tempfile::tempdir().unwrap();
    let store = open(&hosted);
    let host = PageHost::start_for_tests(&store, app_data.path()).unwrap();
    let review = marker_review(&store);
    resolve_markers(&store, Some(&host), &review).unwrap();
    drop(host);
    store.close();
    assert_eq!(
        tree(&hosted),
        tree(&plain),
        "a resolution's bytes changed under a host"
    );
    let resolved = fs::read_to_string(hosted.join(MARKED)).unwrap();
    assert!(!resolved.contains("<<<<<<<"), "{resolved:?}");
    assert!(
        resolved.contains("- mine") && resolved.contains("- theirs"),
        "{resolved:?}"
    );
    assert_eq!(
        trashed(&hosted.join("logseq/.tine-trash/conflicts")),
        vec![MARKED_TEXT.to_owned()],
        "the marker bytes are not recoverable in conflict trash"
    );
    let _ = fs::remove_dir_all(plain);
    let _ = fs::remove_dir_all(hosted);
}

/// W8: a marker page edited after the resolution planned is never
/// overwritten. The operation is refused as changed on disk; the staged copy
/// of the reviewed bytes stays in trash (DESIGN §3.4: harmless).
#[test]
fn a_marker_page_changed_after_the_plan_is_never_overwritten() {
    let _integration_guard = crate::integration_lock();
    let root = graph("markers-raced");
    fs::write(root.join(MARKED), MARKED_TEXT).unwrap();
    let app_data = tempfile::tempdir().unwrap();
    let store = open(&root);
    let host = PageHost::start_for_tests(&store, app_data.path()).unwrap();
    let review = marker_review(&store);
    let held = host
        .reserve(|| vec![PageId::from(MARKED)], Input::Refuse)
        .expect("nothing unsaved");
    let result = std::thread::scope(|scope| {
        let (done, finished) = mpsc::channel();
        let (store, host, review) = (&store, &host, &review);
        scope.spawn(move || {
            let _ = done.send(resolve_markers(store, Some(host), review));
        });
        // The resolution reads, stages and plans now; it cannot apply while
        // the page is held.
        std::thread::sleep(Duration::from_millis(300));
        fs::write(root.join(MARKED), "- resolved by hand\n").unwrap();
        drop(held);
        finished
            .recv_timeout(Duration::from_secs(30))
            .expect("the resolution finishes once the page is released")
    });
    drop(host);
    store.close();
    let error = result.expect_err("a resolution over a changed page refuses");
    assert_eq!(error.to_string(), "file changed on disk");
    assert_eq!(
        fs::read(root.join(MARKED)).unwrap(),
        b"- resolved by hand\n"
    );
    assert_eq!(
        trashed(&root.join("logseq/.tine-trash/conflicts")),
        vec![MARKED_TEXT.to_owned()]
    );
    let _ = fs::remove_dir_all(root);
}
