//! One bad file must never stop the graph load (og-gh644 follow-up F1, queue
//! card yzg_T_Sw). Each test plants one page whose processing panics at a
//! named stage (a deterministic test-only fault hook, never in a shipped
//! build) among ordinary pages, opens the store as the app does, and checks
//! the outcome without an unbounded wait: a hang is a failure, not a timeout.
//!
//! - Page parse (`parse_page_content`): the store reaches Ready, the bad file
//!   is listed in `unreadable_files()` (what the window's
//!   `reportUnreadableFiles` toast shows), other pages open and answer
//!   backlinks and queries.
//! - Block projection (the lsdoc parse every graph index reads: facts, refs,
//!   aliases, search), of a block or of the page-property block: the same.
//!   Before the fix the projection ran first in the initial capture's
//!   graph-wide index build, outside the per-page isolation, killing the
//!   load thread with the store still Loading, so `whole_graph` never
//!   returned.
//! - A graph-wide index stage itself (pure Rust after the parse) that no row
//!   explains: the backstop. The load ends `Failed` promptly (reported,
//!   `refresh()` retries), pages still open, and the writer lock is not
//!   poisoned. A row that does explain it costs that row
//!   (`panic_isolation_tests.rs`).
//! - The same stage in a later publication's patch, which names its pages:
//!   only the panicking page is lost, a journal day's file included, and it
//!   stays retired until its bytes change.
use super::*;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::model::{TEST_INDEX_PANIC_SENTINEL, TEST_PAGE_PARSE_PANIC_SENTINEL};
use tine_core::doc::TEST_PROJECTION_PANIC_SENTINEL;

pub(super) fn graph_with_bad_page(tag: &str, bad: &str, fillers: usize) -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("tine-load-panic-{tag}-{unique}"));
    let pages = root.join("pages");
    fs::create_dir_all(&pages).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    fs::write(pages.join("Target.md"), "- the target page\n").unwrap();
    fs::write(
        pages.join("Good.md"),
        "- links to [[Target]]\n- status:: active\n",
    )
    .unwrap();
    for i in 0..fillers {
        fs::write(
            pages.join(format!("Filler {i:02}.md")),
            format!("- filler {i} mentions [[Target]]\n"),
        )
        .unwrap();
    }
    fs::write(pages.join("Bad.md"), bad).unwrap();
    root
}

/// A guarded save of an edited first block of `Good`.
pub(super) fn edit_good(
    store: &Arc<Store>,
    good: &PageRead,
) -> Result<crate::FileRev, crate::test_host_writer::SaveRefused> {
    let mut doc = good.doc.clone();
    doc.blocks[0].raw = "links to [[Target]], edited after the load".into();
    crate::test_host_writer::HostWriter::start(store).save(
        crate::EditKind::SaveBlock,
        &good.id,
        SaveBase::Existing(good.rev.clone()),
        &doc,
    )
}

/// The load's settled state, polled: `Ok(true)` Ready, `Err(_)` Failed or
/// Closed. Panics (test failure) when still Loading after the deadline.
pub(super) fn settled(store: &Store) -> Result<bool, LoadError> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match store.is_graph_ready() {
            Ok(false) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            Ok(false) => panic!("graph load still Loading after 60 s: the load thread died"),
            other => return other,
        }
    }
}

fn assert_ready_with_bad_page_unreadable(bad: &str, fillers: usize, tag: &str) {
    let root = graph_with_bad_page(tag, bad, fillers);
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    assert!(
        matches!(settled(&store), Ok(true)),
        "{tag}: the graph must reach Ready"
    );
    let view = store.whole_graph().unwrap();
    let bad_id = FileId::from("pages/Bad.md".to_string());
    assert!(
        view.unreadable_files().iter().any(|(id, _)| id == &bad_id),
        "{tag}: the bad file is an unreadable-file entry: {:?}",
        view.unreadable_files()
    );
    // Other pages open and are queryable.
    let good = store.page_named("Good", PageKind::Page).unwrap();
    assert!(good.is_some(), "{tag}: an ordinary page opens");
    let backlinks = view.backlinks("Target").unwrap();
    assert!(
        backlinks
            .iter()
            .any(|group| group.page.eq_ignore_ascii_case("good")),
        "{tag}: backlinks answer for the other pages"
    );
    assert_eq!(
        backlinks.len(),
        fillers + 1,
        "{tag}: every ordinary referrer is indexed"
    );
    match view.query("(property status active)", QueryDialect::Simple) {
        Ok(QueryResult::Simple(groups)) => assert!(
            groups
                .iter()
                .any(|group| group.page.eq_ignore_ascii_case("good")),
            "{tag}: a property query answers"
        ),
        Ok(_) => panic!("{tag}: a simple query answered an advanced result"),
        Err(error) => panic!("{tag}: property query failed: {error:?}"),
    }
    // Opening the bad page is refused for that page only, without a panic.
    let opened = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        store.page(&PageId::from("pages/Bad.md"))
    }));
    assert!(
        matches!(opened, Ok(Err(StoreError::Unparseable(_)))),
        "{tag}: opening the bad page is a per-page error: {:?}",
        opened.as_ref().map(|read| read.as_ref().err())
    );
    // The writer is usable afterwards: an own save writes and publishes.
    let good = store.page_named("Good", PageKind::Page).unwrap().unwrap();
    assert!(edit_good(&store, &good).is_ok(), "{tag}");
    assert!(fs::read_to_string(root.join("pages/Good.md"))
        .unwrap()
        .contains("edited after the load"));
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_page_parse_panic_leaves_one_unreadable_file_and_a_ready_graph() {
    for fillers in [2, 80] {
        assert_ready_with_bad_page_unreadable(
            &format!("- {TEST_PAGE_PARSE_PANIC_SENTINEL}\n"),
            fillers,
            &format!("parse-{fillers}"),
        );
    }
}

#[test]
fn a_block_projection_panic_leaves_one_unreadable_file_and_a_ready_graph() {
    for fillers in [2, 80] {
        assert_ready_with_bad_page_unreadable(
            &format!("- fine\n- {TEST_PROJECTION_PANIC_SENTINEL} [[Target]]\n"),
            fillers,
            &format!("projection-{fillers}"),
        );
    }
}

#[test]
fn a_page_property_projection_panic_leaves_one_unreadable_file_and_a_ready_graph() {
    for fillers in [2, 80] {
        assert_ready_with_bad_page_unreadable(
            &format!("note:: {TEST_PROJECTION_PANIC_SENTINEL}\n\n- body [[Target]]\n"),
            fillers,
            &format!("page-property-{fillers}"),
        );
    }
}

/// While it lives, every whole-graph index build of `root` panics and no row
/// reproduces it: what a load fails on now that a bad row costs only that row.
pub(super) struct NoRowExplains(PathBuf);

impl NoRowExplains {
    pub(super) fn arm(root: &Path) -> Self {
        let canonical = fs::canonicalize(root).unwrap();
        (crate::model::TEST_FULL_INDEX_PANIC_ROOTS.lock().unwrap()).push(canonical.clone());
        Self(canonical)
    }
}

impl Drop for NoRowExplains {
    fn drop(&mut self) {
        (crate::model::TEST_FULL_INDEX_PANIC_ROOTS.lock().unwrap()).retain(|r| *r != self.0);
    }
}

#[test]
fn an_index_build_panic_fails_the_load_instead_of_hanging_it() {
    let root = graph_with_bad_page("index", "- fine\n", 2);
    let no_row = NoRowExplains::arm(&root);
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    assert!(
        matches!(settled(&store), Err(LoadError::Failed { .. })),
        "an index-stage panic ends the load as Failed"
    );
    assert!(matches!(store.whole_graph(), Err(LoadError::Failed { .. })));
    // A retry (the window refreshes on every focus; Settings rescans)
    // rebuilds the same indexes and fails the same way, through the same
    // containment: still Failed, never a poisoned writer.
    for depth in [Depth::Stamps, Depth::Bytes, Depth::Rebuild] {
        let retried =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| store.refresh(depth)));
        assert!(
            matches!(retried, Ok(Err(LoadError::Failed { .. }))),
            "{depth:?}: the retry fails without unwinding"
        );
        assert!(matches!(store.whole_graph(), Err(LoadError::Failed { .. })));
    }
    // Pages still open from their files, and the writer lock is not poisoned.
    let good = store.page(&PageId::from("pages/Good.md")).unwrap();
    assert!(edit_good(&store, &good).is_ok());
    assert!(fs::read_to_string(root.join("pages/Good.md"))
        .unwrap()
        .contains("edited after the load"));
    drop((store, no_row));
    let _ = fs::remove_dir_all(&root);
}

/// After Ready, the later stages that read one page's parse: an external
/// edit (the watcher's and focus rescan's sync) and an own save. Each panics
/// a projection for that page only; the writer stays usable and the other
/// pages stay indexed.
pub(super) fn assert_rest_of_graph_answers(store: &Store, tag: &str, referrers: usize) {
    let view = store.whole_graph().unwrap();
    let backlinks = view.backlinks("Target").unwrap();
    assert_eq!(
        backlinks.len(),
        referrers,
        "{tag}: the other referrers stay indexed"
    );
    match view.query("(property status active)", QueryDialect::Simple) {
        Ok(QueryResult::Simple(groups)) => assert!(
            groups
                .iter()
                .any(|group| group.page.eq_ignore_ascii_case("good")),
            "{tag}: a property query answers"
        ),
        Ok(_) => panic!("{tag}: a simple query answered an advanced result"),
        Err(error) => panic!("{tag}: property query failed: {error:?}"),
    }
}

#[test]
fn an_external_edit_that_panics_a_projection_stays_one_unreadable_page() {
    for (tag, bad) in [
        (
            "external-block",
            format!("- {TEST_PROJECTION_PANIC_SENTINEL} [[Target]]\n"),
        ),
        (
            "external-page-property",
            format!("note:: {TEST_PROJECTION_PANIC_SENTINEL}\n\n- body [[Target]]\n"),
        ),
    ] {
        let root = graph_with_bad_page(tag, "- fine for now\n", 2);
        let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
        assert!(matches!(settled(&store), Ok(true)), "{tag}");
        // An external editor or sync client rewrites Filler 00.
        fs::write(root.join("pages/Filler 00.md"), &bad).unwrap();
        let refreshed =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| store.refresh(Depth::Bytes)));
        assert!(
            matches!(refreshed, Ok(Ok(_))),
            "{tag}: the rescan completes"
        );
        // The explicit refresh does not consume the watcher's notifications
        // or launch racy follow-up. A later retry of the bad projection can
        // invalidate the cache between the whole-graph check and page open;
        // a cold DTO read does not project page properties. Check the settled
        // result only after those independent observations have completed.
        assert!(
            store.watcher_drained_for_tests(Duration::from_secs(60)),
            "{tag}: the external edit's watcher observations never drained"
        );
        let view = store.whole_graph().unwrap();
        assert_eq!(
            view.unreadable_files()
                .iter()
                .map(|(id, _)| id.as_str())
                .collect::<Vec<_>>(),
            vec!["pages/Filler 00.md"],
            "{tag}: only the edited page is unreadable"
        );
        // Filler 00's reference is gone with its parse; the rest answer.
        assert_rest_of_graph_answers(&store, tag, 2);
        let opened = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            store.page(&PageId::from("pages/Filler 00.md"))
        }));
        assert!(
            matches!(opened, Ok(Err(StoreError::Unparseable(_)))),
            "{tag}: opening the edited page is a per-page error: {:?}",
            opened
                .as_ref()
                .map(|result| result.as_ref().map(|_| "readable page"))
        );
        let good = store.page_named("Good", PageKind::Page).unwrap().unwrap();
        assert!(edit_good(&store, &good).is_ok(), "{tag}");
        drop(store);
        let _ = fs::remove_dir_all(&root);
    }
}

#[test]
fn an_own_save_that_panics_a_projection_is_written_and_keeps_the_writer() {
    let root = graph_with_bad_page("own-save", "- fine for now\n", 2);
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    assert!(matches!(settled(&store), Ok(true)));
    let filler = store.page(&PageId::from("pages/Filler 01.md")).unwrap();
    let mut doc = filler.doc.clone();
    doc.blocks[0].raw = format!("{TEST_PROJECTION_PANIC_SENTINEL} [[Target]]");
    let saved = crate::test_host_writer::HostWriter::start(&store).save(
        crate::EditKind::SaveBlock,
        &filler.id,
        SaveBase::Existing(filler.rev.clone()),
        &doc,
    );
    assert!(
        saved.is_ok(),
        "the guarded write lands; only its indexing fails"
    );
    assert!(fs::read_to_string(root.join("pages/Filler 01.md"))
        .unwrap()
        .contains(TEST_PROJECTION_PANIC_SENTINEL));
    assert_rest_of_graph_answers(&store, "own-save", 2);
    let good = store.page_named("Good", PageKind::Page).unwrap().unwrap();
    assert!(edit_good(&store, &good).is_ok());
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

/// og-gh644 F3: a page move reads the moved page's own title (an lsdoc
/// outline parse) while it holds the writer lock. A panic there refuses that
/// one move and leaves the writer usable: before containment it unwound
/// through the writer guard, poisoning it, so every later save, read and
/// close of the store panicked too.
#[test]
fn a_title_read_panic_refuses_one_rename_and_keeps_the_writer() {
    use crate::model::TEST_TITLE_READ_PANIC_SENTINEL;
    let bad = format!("title:: Bad\n\n- {TEST_TITLE_READ_PANIC_SENTINEL}\n");
    let root = graph_with_bad_page("title-read", &bad, 2);
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    assert!(matches!(settled(&store), Ok(true)));
    let before = fs::read(root.join("pages/Bad.md")).unwrap();
    let map = crate::RenameMap(vec![("Bad".into(), "Renamed".into())], Vec::new());
    let committed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut tx = store.page_plan(Some(crate::EditKind::RenamePage));
        tx.move_file(
            &FileId::from("pages/Bad.md".to_string()),
            crate::FileRev::from_file(&root.join("pages/Bad.md")).unwrap(),
            &FileId::from("pages/Renamed.md".to_string()),
            Some(&map),
        );
        tx.plan()
    }));
    assert!(
        matches!(
            committed,
            Ok(Err(crate::Why::Refused(crate::Refusal::ReadOnly(_))))
        ),
        "the move is refused, not unwound"
    );
    assert_eq!(fs::read(root.join("pages/Bad.md")).unwrap(), before);
    assert!(!root.join("pages/Renamed.md").exists());
    let good = store.page_named("Good", PageKind::Page).unwrap().unwrap();
    assert!(edit_good(&store, &good).is_ok());
    assert_rest_of_graph_answers(&store, "title-read", 3);
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

/// The warm launch from a launch checkpoint (`checkpoint::launch_from`): a
/// page created while Tine was closed is parsed by the launch patch, and the
/// first publication then patches the served view's indexes with it. A
/// projection panic or an index-stage panic there leaves that one file
/// unreadable: the patch names its page, unlike a cold load's full build.
#[test]
fn a_checkpoint_launch_contains_a_bad_page_created_while_closed() {
    for (tag, rel, bad) in [
        (
            "checkpoint-projection",
            "pages/Bad.md",
            format!("- {TEST_PROJECTION_PANIC_SENTINEL} [[Target]]\n"),
        ),
        (
            "checkpoint-index",
            "pages/Bad.md",
            format!("- {TEST_INDEX_PANIC_SENTINEL}\n"),
        ),
        (
            "checkpoint-journal",
            JOURNAL,
            format!("- {TEST_INDEX_PANIC_SENTINEL}\n"),
        ),
    ] {
        let root = graph_with_bad_page(tag, "- fine for now\n", 2);
        fs::remove_file(root.join("pages/Bad.md")).unwrap();
        let cp = root.with_extension("checkpoint.bin");
        let options = || OpenOptions {
            watch: WatchMode::Poll,
            launch_checkpoint: Some(cp.clone()),
            ..Default::default()
        };
        let store = Arc::new(Store::open(&root, options()).unwrap().0);
        store.whole_graph_reconciled().unwrap();
        assert!(
            matches!(
                store.write_checkpoint_now(),
                Some(crate::CheckpointWrite::Written { .. })
            ),
            "{tag}: checkpoint written"
        );
        store.close();
        drop(store);
        fs::write(root.join(rel), &bad).unwrap();
        let store = Arc::new(Store::open(&root, options()).unwrap().0);
        let outcome = settled(&store);
        assert!(matches!(outcome, Ok(true)), "{tag}: {outcome:?}");
        let view = store.whole_graph().unwrap();
        assert!(
            (view.unreadable_files().iter()).any(|(id, _)| id.as_str() == rel),
            "{tag}: {:?}",
            view.unreadable_files()
        );
        assert_rest_of_graph_answers(&store, tag, 3);
        let good = store.page_named("Good", PageKind::Page).unwrap().unwrap();
        assert!(edit_good(&store, &good).is_ok(), "{tag}");
        drop(store);
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_file(&cp);
    }
}

/// The launch's own `Failed` path: a build-relevant config edit during the
/// launch diff rebuilds the indexes in its publication, and that build names
/// no page.
#[test]
fn a_checkpoint_launch_whose_diff_panics_with_no_page_to_blame_fails_and_recovers() {
    let root = graph_with_bad_page("checkpoint-failed", "- fine for now\n", 2);
    let cp = root.with_extension("checkpoint.bin");
    let options = || OpenOptions {
        watch: WatchMode::Poll,
        launch_checkpoint: Some(cp.clone()),
        ..Default::default()
    };
    let store = Store::open(&root, options()).unwrap().0;
    store.whole_graph_reconciled().unwrap();
    assert!(matches!(
        store.write_checkpoint_now(),
        Some(crate::CheckpointWrite::Written { .. })
    ));
    store.close();
    drop(store);
    let hold_diff = root.join(".tine-test-pause-launch-diff");
    fs::write(&hold_diff, "").unwrap();
    let store = Arc::new(Store::open(&root, options()).unwrap().0);
    let deadline = Instant::now() + Duration::from_secs(60);
    while !store.load.serving.load(Ordering::Acquire) {
        assert!(Instant::now() < deadline, "the checkpoint was not served");
        std::thread::sleep(Duration::from_millis(1));
    }
    fs::create_dir_all(root.join("logseq")).unwrap();
    fs::write(
        root.join("logseq/config.edn"),
        "{:property-pages/excludelist #{:unused}}\n",
    )
    .unwrap();
    let canonical = fs::canonicalize(&root).unwrap();
    crate::model::TEST_FULL_INDEX_PANIC_ROOTS
        .lock()
        .unwrap()
        .push(canonical.clone());
    fs::remove_file(&hold_diff).unwrap();
    let outcome = settled(&store);
    let poisoned = store.writer.is_poisoned();
    (crate::model::TEST_FULL_INDEX_PANIC_ROOTS.lock().unwrap()).retain(|r| *r != canonical);
    assert!(!poisoned, "the failed launch keeps the writer");
    assert!(
        matches!(outcome, Err(LoadError::Failed { .. })),
        "{outcome:?}"
    );
    let good = store.page(&PageId::from("pages/Good.md")).unwrap();
    assert!(edit_good(&store, &good).is_ok());
    assert!(store.refresh(Depth::Stamps).is_ok());
    assert!(matches!(settled(&store), Ok(true)), "a retry recovers it");
    assert_rest_of_graph_answers(&store, "checkpoint-failed", 3);
    drop(store);
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_file(&cp);
}

/// A Settings rescan (`Depth::Rebuild`) of a Ready graph rebuilds every
/// index from the whole cache (`publish_rebuilt`). An index-stage panic
/// there fails the graph instead of poisoning the writer, and the next
/// retry goes through the recovery publication's containment.
#[test]
fn a_rescan_whose_index_build_panics_fails_the_graph_and_keeps_the_writer() {
    let root = graph_with_bad_page("rescan-index", "- fine for now\n", 2);
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    assert!(matches!(settled(&store), Ok(true)));
    let canonical = fs::canonicalize(&root).unwrap();
    crate::model::TEST_FULL_INDEX_PANIC_ROOTS
        .lock()
        .unwrap()
        .push(canonical.clone());
    let rescanned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        store.refresh(Depth::Rebuild)
    }));
    assert!(
        matches!(rescanned, Ok(Err(LoadError::Failed { .. }))),
        "the rescan fails without unwinding: {rescanned:?}"
    );
    assert!(matches!(store.whole_graph(), Err(LoadError::Failed { .. })));
    let retried = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        store.refresh(Depth::Stamps)
    }));
    assert!(
        matches!(retried, Ok(Err(LoadError::Failed { .. }))),
        "{retried:?}"
    );
    let good = store.page(&PageId::from("pages/Good.md")).unwrap();
    assert!(edit_good(&store, &good).is_ok());
    assert!(fs::read_to_string(root.join("pages/Good.md"))
        .unwrap()
        .contains("edited after the load"));
    // The fault cleared, a retry recovers the graph.
    (crate::model::TEST_FULL_INDEX_PANIC_ROOTS.lock().unwrap()).retain(|r| *r != canonical);
    assert!(matches!(store.refresh(Depth::Stamps), Ok(_)));
    assert!(matches!(settled(&store), Ok(true)));
    assert_rest_of_graph_answers(&store, "rescan-index", 3);
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

/// Bytes a sync client delivered, keeping the source's old mtime (Syncthing,
/// `rsync -t`, `cp -p`, unzip): no racy rule protects them.
fn deliver_with_old_mtime(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(3600))
        .unwrap();
}

/// A load that failed at the index stage with its cache built, its watcher
/// stopped so that only the recovery's diff can observe a file.
fn failed_at_the_index_stage(tag: &str) -> (PathBuf, Arc<Store>, NoRowExplains) {
    let root = graph_with_bad_page(tag, "- fine\n", 2);
    let no_row = NoRowExplains::arm(&root);
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    assert!(matches!(settled(&store), Err(LoadError::Failed { .. })));
    assert!(store.graph.cache_built(), "the parse finished");
    store.watch.stop_background_for_test();
    (root, store, no_row)
}

/// The panic gone and the graph recovered: Good's first block now.
fn recovered_good(root: &Path, store: &Store, no_row: NoRowExplains) -> String {
    drop(no_row);
    store.refresh(Depth::Stamps).unwrap();
    let view = store.whole_graph().unwrap();
    let corpus = view.corpus(None, false);
    let good = corpus.pages.iter().find(|page| page.name == "Good");
    good.unwrap().document.roots[0].raw().to_owned()
}

#[test]
fn recovered_ready_diagnostics_include_timings() {
    let (root, store, no_row) = failed_at_the_index_stage("ready-diag");
    let failed = store.diagnostics()["launch"].clone();
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["loadStopped"], true);
    assert!(failed["readyMs"].is_null());
    drop(no_row);
    store.refresh(Depth::Stamps).unwrap();
    let launch = store.diagnostics()["launch"].clone();
    assert_eq!(launch["status"], "ready");
    assert!(
        launch["readyMs"].is_number(),
        "ready time recorded: {launch}"
    );
    assert!(launch["publishMs"].is_number());
    // A later failed rescan/recovery must keep the original launch endpoint.
    let no_row = NoRowExplains::arm(&root);
    assert!(store.refresh(Depth::Rebuild).is_err());
    drop(no_row);
    store.refresh(Depth::Stamps).unwrap();
    let recovered = store.diagnostics()["launch"].clone();
    assert_eq!(recovered["readyMs"], launch["readyMs"]);
    assert_eq!(recovered["publishMs"], launch["publishMs"]);
    store.close();
    drop(store);
    let _ = fs::remove_dir_all(root);
}

/// REG-FAILED-LOAD-RACED-OBSERVATION-KEPT, the reconcile: a raced observation
/// stamps the file with its current metadata and the published revision, so
/// recovery's diff must hash it, or its row stays on the old bytes for good.
/// A recovery that fails again consumes the race; the row it cached then
/// carries the delivery to the next one.
#[test]
fn a_raced_delivery_while_the_load_failed_reaches_the_index_after_recovery() {
    for second_failure in [false, true] {
        let (root, store, no_row) = failed_at_the_index_stage("raced-reconcile");
        let path = store.graph.root.join("pages/Good.md");
        let published = FileRev::from_file(&path).unwrap();
        deliver_with_old_mtime(&path, "- delivered\n");
        let id = FileId::from("pages/Good.md".to_owned());
        let raced = store
            .watch
            .note_own_observed(&[(id, Some(published))], &HashMap::new());
        assert!(raced.contains(&path));
        store.watch.reconcile_raced(&raced);
        if second_failure {
            assert!(
                store.refresh(Depth::Stamps).is_err(),
                "no row explains it yet"
            );
        }
        assert_eq!(recovered_good(&root, &store, no_row), "delivered");
        drop(store);
        let _ = fs::remove_dir_all(&root);
    }
}

/// REG-FAILED-LOAD-RACED-OBSERVATION-KEPT, the publication's early return.
#[test]
fn a_raced_publication_while_the_load_failed_reaches_the_index_after_recovery() {
    let (root, store, no_row) = failed_at_the_index_stage("raced-publication");
    let path = store.graph.root.join("pages/Good.md");
    let published = FileRev::from_file(&path).unwrap();
    deliver_with_old_mtime(&path, "- delivered\n");
    let id = FileId::from("pages/Good.md".to_owned());
    let files = vec![(id, ChangeKind::Modified, Some(published))];
    let (_, change) = store.publish_own(files, Default::default());
    assert!(change.is_none(), "nothing publishes before recovery");
    assert_eq!(recovered_good(&root, &store, no_row), "delivered");
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

/// REG-FAILED-LOAD-RACED-OBSERVATION-KEPT, a raced publication the load had
/// deferred when it failed: the failure drops the change, not its race.
#[test]
fn a_raced_deferred_publication_outlives_the_failed_load() {
    let root = graph_with_bad_page("raced-deferred", "- fine\n", 2);
    let no_row = NoRowExplains::arm(&root);
    fs::write(root.join(".tine-test-pause-load"), "").unwrap();
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.watch.stop_background_for_test();
    let installed: TestPause = Arc::new((Mutex::new((false, false)), Condvar::new()));
    *store.graph.warm_after_install_pause.lock().unwrap() = Some(Arc::clone(&installed));
    fs::remove_file(root.join(".tine-test-pause-load")).unwrap();
    {
        let (state, ready) = &*installed;
        let mut state = state.lock().unwrap();
        while !state.0 {
            state = ready.wait(state).unwrap();
        }
    }
    let path = store.graph.root.join("pages/Good.md");
    let published = FileRev::from_file(&path).unwrap();
    deliver_with_old_mtime(&path, "- delivered\n");
    let id = FileId::from("pages/Good.md".to_owned());
    let files = vec![(id, ChangeKind::Modified, Some(published))];
    {
        let _writer = store.writer.lock().unwrap();
        store.publish_own(files, Default::default());
    }
    assert!(store.changes.has_deferred_changes());
    {
        let (state, ready) = &*installed;
        state.lock().unwrap().1 = true;
        ready.notify_all();
    }
    assert!(matches!(settled(&store), Err(LoadError::Failed { .. })));
    assert_eq!(recovered_good(&root, &store, no_row), "delivered");
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

/// REG-FAILED-LOAD-RACED-OBSERVATION-KEPT beside index-stage containment: a
/// raced path whose row is retired for bad bytes, in a Ready graph and in one a
/// publication failed. Bytes that panicked stay contained without another
/// failure; new good bytes bring the page back; new bad bytes cost that page.
#[test]
fn a_raced_delivery_on_a_contained_page_stays_about_that_page() {
    let contained = format!("- {TEST_INDEX_PANIC_SENTINEL}\n");
    let other_bad = format!("- {TEST_INDEX_PANIC_SENTINEL} other\n");
    let good = "- delivered, mentions [[Target]]\n";
    for failed in [false, true] {
        for (bytes, unreadable, referrers) in
            [(&contained[..], 1, 2), (&other_bad[..], 1, 2), (good, 0, 3)]
        {
            let tag = format!("raced-contained-{failed}-{unreadable}");
            let (root, store) = ready_graph_without_watcher(&tag);
            let path = store.graph.root.join("pages/Filler 00.md");
            fs::write(&path, &contained).unwrap();
            store.refresh(Depth::Bytes).unwrap();
            assert_eq!(store.whole_graph().unwrap().unreadable_files().len(), 1);
            let canonical = fs::canonicalize(&root).unwrap();
            if failed {
                fs::create_dir_all(root.join("logseq")).unwrap();
                fs::write(
                    root.join("logseq/config.edn"),
                    "{:property-pages/excludelist #{:unused}}\n",
                )
                .unwrap();
                (crate::model::TEST_FULL_INDEX_PANIC_ROOTS.lock().unwrap()).push(canonical.clone());
                let refreshed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    store.refresh(Depth::Bytes)
                }));
                assert!(refreshed.is_ok(), "{tag}: the refresh must not unwind");
                assert!(matches!(store.whole_graph(), Err(LoadError::Failed { .. })));
            }
            let published = FileRev::from_bytes(b"- what the owner published\n");
            deliver_with_old_mtime(&path, bytes);
            let id = FileId::from("pages/Filler 00.md".to_owned());
            let raced = store
                .watch
                .note_own_observed(&[(id, Some(published))], &HashMap::new());
            assert!(raced.contains(&path), "{tag}");
            store.watch.reconcile_raced(&raced);
            (crate::model::TEST_FULL_INDEX_PANIC_ROOTS.lock().unwrap()).retain(|r| *r != canonical);
            assert!(!store.writer.is_poisoned(), "{tag}");
            store.refresh(Depth::Stamps).unwrap();
            let view = store.whole_graph().unwrap();
            assert_eq!(view.unreadable_files().len(), unreadable, "{tag}");
            assert_eq!(view.backlinks("Target").unwrap().len(), referrers, "{tag}");
            let saved = store.page(&PageId::from("pages/Filler 00.md")).unwrap();
            assert_eq!(saved.doc.blocks[0].raw, bytes[2..bytes.len() - 1], "{tag}");
            drop((view, store));
            let _ = fs::remove_dir_all(&root);
        }
    }
}

/// REG-CAPTURE-PATCHES-UNNAMED-ROWS: a host save while a failed rescan holds
/// the graph Failed changes the row and publishes nothing. The recovery's diff
/// then publishes what it found, another file, so the capture patches the
/// indexes of the generation before the failure: only `take_touched` adds the
/// saved row to that patch.
#[test]
fn a_save_while_a_rescan_failed_reaches_every_index_after_recovery() {
    let root = graph_with_bad_page("unnamed-row", "- fine for now\n", 2);
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    assert!(matches!(settled(&store), Ok(true)));
    let canonical = fs::canonicalize(&root).unwrap();
    (crate::model::TEST_FULL_INDEX_PANIC_ROOTS.lock().unwrap()).push(canonical.clone());
    let rescanned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        store.refresh(Depth::Rebuild)
    }));
    assert!(matches!(rescanned, Ok(Err(LoadError::Failed { .. }))));
    (crate::model::TEST_FULL_INDEX_PANIC_ROOTS.lock().unwrap()).retain(|r| *r != canonical);

    let writer = crate::test_host_writer::HostWriter::for_store(&store);
    let good = store.page(&PageId::from("pages/Good.md")).unwrap();
    let mut doc = good.doc.clone();
    doc.blocks[0].raw = "now links to [[Elsewhere]]".into();
    writer
        .save(
            crate::EditKind::SaveBlock,
            &good.id,
            SaveBase::Existing(good.rev.clone()),
            &doc,
        )
        .unwrap();
    writer.settle();
    fs::write(root.join("pages/Filler 00.md"), "- rewritten outside\n").unwrap();

    store.refresh(Depth::Stamps).unwrap();
    assert!(matches!(settled(&store), Ok(true)));
    let view = store.whole_graph().unwrap();
    let found = view.backlinks("Elsewhere").unwrap();
    assert!(
        (found.iter()).any(|group| group.page.eq_ignore_ascii_case("good")),
        "the pages root has the row, its reference index does not: {found:?}"
    );
    store.close();
    drop((view, writer, store));
    let _ = fs::remove_dir_all(&root);
}

/// A failed multi-row patch consumed its pending paths, but published nothing.
/// The watcher has already observed those bytes: only the failed-capture
/// restore carries their rows into the next successful patch.
#[test]
fn a_failed_patch_restores_its_pending_rows_for_recovery() {
    let root = graph_with_bad_page("failed-patch-restore", "- ordinary\n", 1);
    let old = std::time::SystemTime::now() - Duration::from_secs(3600);
    for entry in fs::read_dir(root.join("pages")).unwrap() {
        fs::File::options()
            .write(true)
            .open(entry.unwrap().path())
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
    let store = Store::open(&root, Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    store.stop_watcher_for_tests();
    let canonical = fs::canonicalize(&root).unwrap();
    let roots = &super::snapshot::TEST_PATCH_PANIC_ROOTS;
    roots.lock().unwrap().push(canonical.clone());
    for name in ["Good", "Bad"] {
        let path = root.join(format!("pages/{name}.md"));
        fs::write(&path, "- links to [[Recovered]]\n").unwrap();
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
    let before = store.changes.rev();
    let failed = store.refresh(Depth::Stamps).unwrap();
    roots.lock().unwrap().retain(|r| *r != canonical);
    assert_eq!(failed, before, "a failed capture publishes no revision");
    assert!(matches!(
        store.is_graph_ready(),
        Err(LoadError::Failed { .. })
    ));
    // A nonempty recovery delta must patch, not fall back to a full index
    // rebuild (which would mask the lost paths and make this mutant pass).
    fs::write(
        root.join("pages/Filler 00.md"),
        "- unrelated recovery trigger\n",
    )
    .unwrap();
    store.refresh(Depth::Stamps).unwrap();
    let view = store.whole_graph().unwrap();
    let mut pages: Vec<_> = view
        .backlinks("Recovered")
        .unwrap()
        .iter()
        .map(|group| group.page.clone())
        .collect();
    pages.sort();
    assert_eq!(
        pages,
        ["Bad", "Good"],
        "both changed rows reach the recovered index"
    );
    assert!(
        view.backlinks("Target").unwrap().is_empty(),
        "old links are retired"
    );
    store.close();
    drop((view, store));
    fs::remove_dir_all(root).unwrap();
}

/// The page cache's document for `rel`, for identity comparison: the same
/// `Arc` afterwards means that page was neither re-read nor re-parsed.
pub(super) fn cached_doc(store: &Store, rel: &str) -> Arc<tine_core::doc::Document> {
    let pages = store
        .graph
        .peek_pages()
        .expect("the page cache stays built");
    let (_, doc) = (pages.iter())
        .find(|(entry, _)| entry.rel_path_str() == rel)
        .unwrap_or_else(|| panic!("{rel} stays cached"));
    Arc::clone(doc)
}

/// Assert the one-bad-file outcome after `Filler 00` went bad: it alone is
/// unreadable and refuses its open; every other row is the same cached `Arc`;
/// the rest of the graph answers; the writer is usable.
fn assert_only_filler_00_lost(
    store: &Arc<Store>,
    root: &Path,
    tag: &str,
    others: &[(&str, Arc<tine_core::doc::Document>)],
) {
    let view = store.whole_graph().unwrap();
    assert_eq!(
        view.unreadable_files()
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        vec!["pages/Filler 00.md"],
        "{tag}: only the bad page is unreadable"
    );
    for (rel, before) in others {
        assert!(
            Arc::ptr_eq(before, &cached_doc(store, rel)),
            "{tag}: {rel} must not be re-read"
        );
    }
    assert_rest_of_graph_answers(store, tag, 2);
    let needle = "filler 1 mentions";
    let hits = store.graph.run_graph_search(needle, 0, 8, false).hits;
    assert!(
        !hits.is_empty(),
        "{tag}: search answers for the other pages"
    );
    // An index-stage panic leaves the page's parse good: it still opens, and
    // only its indexing is lost. Either way the open never unwinds, and no
    // other page pays for it.
    let opened = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        store.page(&PageId::from("pages/Filler 00.md"))
    }));
    assert!(
        matches!(opened, Ok(Ok(_)) | Ok(Err(StoreError::Unparseable(_)))),
        "{tag}: opening the bad page stays about that page"
    );
    let good = store.page_named("Good", PageKind::Page).unwrap().unwrap();
    assert!(edit_good(store, &good).is_ok(), "{tag}");
    assert!(fs::read_to_string(root.join("pages/Good.md"))
        .unwrap()
        .contains("edited after the load"));
}

pub(super) fn ready_graph_without_watcher(tag: &str) -> (PathBuf, Arc<Store>) {
    let root = graph_with_bad_page(tag, "- fine for now\n", 2);
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    assert!(matches!(settled(&store), Ok(true)), "{tag}");
    store.watch.stop_background_for_test();
    (root, store)
}

pub(super) fn others_cached(store: &Store) -> Vec<(&'static str, Arc<tine_core::doc::Document>)> {
    ["pages/Good.md", "pages/Target.md", "pages/Filler 01.md"]
        .map(|rel| (rel, cached_doc(store, rel)))
        .to_vec()
}

/// Queue card: "bad-page open wipes the page cache".
#[test]
fn opening_a_bad_page_keeps_every_other_page_cached() {
    let root = graph_with_bad_page("open-keeps-cache", "- fine for now\n", 2);
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    assert!(matches!(settled(&store), Ok(true)));
    let others = others_cached(&store);
    fs::write(
        root.join("pages/Filler 00.md"),
        format!("- {TEST_PROJECTION_PANIC_SENTINEL}\n"),
    )
    .unwrap();
    let opened = store.page(&PageId::from("pages/Filler 00.md"));
    assert!(matches!(opened, Err(StoreError::Unparseable(_))));
    // The file is fresh, so its stamp is racy: the watcher settles it.
    assert!(store.watcher_drained_for_tests(Duration::from_secs(60)));
    assert_only_filler_00_lost(&store, &root, "open", &others);
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

/// The transaction publication's sibling of the open above: the guarded write
/// lands, and only that one page's indexing is lost, not the page cache.
#[test]
fn an_own_save_that_panics_a_projection_keeps_every_other_page_cached() {
    let root = graph_with_bad_page("save-keeps-cache", "- fine for now\n", 2);
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    assert!(matches!(settled(&store), Ok(true)));
    let others = others_cached(&store);
    let filler = store.page(&PageId::from("pages/Filler 00.md")).unwrap();
    let mut doc = filler.doc.clone();
    doc.blocks[0].raw = TEST_PROJECTION_PANIC_SENTINEL.into();
    let saved = crate::test_host_writer::HostWriter::start(&store).save(
        crate::EditKind::SaveBlock,
        &filler.id,
        SaveBase::Existing(filler.rev.clone()),
        &doc,
    );
    assert!(saved.is_ok());
    // A held page's parse failure is its owner's, so it is reported once the
    // host releases the page; the file is fresh, so the watcher settles it.
    assert!(store.watcher_drained_for_tests(Duration::from_secs(60)));
    assert_only_filler_00_lost(&store, &root, "save-projection", &others);
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

/// Queue card: "incremental publication containment". A later publication
/// patches the indexes with the changed page (`Snapshot::capture`, under the
/// writer lock): an index-stage panic for that page, on each door that
/// publishes, costs that page only.
#[test]
fn an_external_edit_that_panics_an_index_stage_costs_that_page_only() {
    let (root, store) = ready_graph_without_watcher("external-index");
    let others = others_cached(&store);
    fs::write(
        root.join("pages/Filler 00.md"),
        format!("- {TEST_INDEX_PANIC_SENTINEL}\n"),
    )
    .unwrap();
    let refreshed =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| store.refresh(Depth::Bytes)));
    assert!(matches!(refreshed, Ok(Ok(_))), "the rescan completes");
    assert_only_filler_00_lost(&store, &root, "external-index", &others);
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn an_own_save_that_panics_an_index_stage_costs_that_page_only() {
    let (root, store) = ready_graph_without_watcher("save-index");
    let others = others_cached(&store);
    let filler = store.page(&PageId::from("pages/Filler 00.md")).unwrap();
    let mut doc = filler.doc.clone();
    doc.blocks[0].raw = TEST_INDEX_PANIC_SENTINEL.into();
    let saved = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::test_host_writer::HostWriter::start(&store).save(
            crate::EditKind::SaveBlock,
            &filler.id,
            SaveBase::Existing(filler.rev.clone()),
            &doc,
        )
    }));
    assert!(
        matches!(saved, Ok(Ok(_))),
        "the guarded write landed: it answers Saved, never a retry of a landed write"
    );
    assert!(fs::read_to_string(root.join("pages/Filler 00.md"))
        .unwrap()
        .contains(TEST_INDEX_PANIC_SENTINEL));
    assert_only_filler_00_lost(&store, &root, "save-index", &others);
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn opening_a_page_that_panics_an_index_stage_costs_that_page_only() {
    let (root, store) = ready_graph_without_watcher("open-index");
    let others = others_cached(&store);
    fs::write(
        root.join("pages/Filler 00.md"),
        format!("- {TEST_INDEX_PANIC_SENTINEL}\n"),
    )
    .unwrap();
    let opened = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        store.page(&PageId::from("pages/Filler 00.md"))
    }));
    assert!(opened.is_ok(), "the open must not unwind");
    assert_only_filler_00_lost(&store, &root, "open-index", &others);
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

/// The live watcher thread: the same edit delivered with no refresh call.
#[test]
fn the_watcher_survives_an_external_edit_that_panics_an_index_stage() {
    let root = graph_with_bad_page("watcher-index", "- fine for now\n", 2);
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    assert!(matches!(settled(&store), Ok(true)));
    fs::write(
        root.join("pages/Filler 00.md"),
        format!("- {TEST_INDEX_PANIC_SENTINEL}\n"),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    while !(store.whole_graph().unwrap().unreadable_files().iter())
        .any(|(id, _)| id.as_str() == "pages/Filler 00.md")
    {
        assert!(
            Instant::now() < deadline,
            "the watcher never reported the bad page"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(store.watcher_drained_for_tests(Duration::from_secs(60)));
    assert_rest_of_graph_answers(&store, "watcher-index", 2);
    // The watcher still delivers: a later edit of another page arrives.
    let good = store.page_named("Good", PageKind::Page).unwrap().unwrap();
    assert!(edit_good(&store, &good).is_ok());
    fs::write(
        root.join("pages/Target.md"),
        "- the target, edited outside\n",
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    while !cached_doc(&store, "pages/Target.md").roots[0]
        .raw()
        .contains("edited outside")
    {
        assert!(Instant::now() < deadline, "the watcher thread died");
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

/// One watcher batch, one bad page: the good page in it publishes.
#[test]
fn a_batch_with_one_page_that_panics_an_index_stage_publishes_the_others() {
    let (root, store) = ready_graph_without_watcher("batch-index");
    let others = others_cached(&store);
    let others: Vec<_> = (others.into_iter())
        .filter(|(rel, _)| *rel != "pages/Filler 01.md")
        .collect();
    fs::write(
        root.join("pages/Filler 00.md"),
        format!("- {TEST_INDEX_PANIC_SENTINEL}\n"),
    )
    .unwrap();
    fs::write(
        root.join("pages/Filler 01.md"),
        "- filler 1 mentions [[Target]] and [[Fresh]]\n",
    )
    .unwrap();
    store.refresh(Depth::Bytes).unwrap();
    assert_only_filler_00_lost(&store, &root, "batch", &others);
    let backlinks = store.whole_graph().unwrap().backlinks("Fresh").unwrap();
    assert_eq!(backlinks.len(), 1, "the batch's good page is patched in");
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

/// A repaired file comes back without a restart: the row is retired, not lost.
#[test]
fn a_repaired_page_returns_after_an_index_stage_panic() {
    let (root, store) = ready_graph_without_watcher("repair-index");
    let bad = root.join("pages/Filler 00.md");
    fs::write(&bad, format!("- {TEST_INDEX_PANIC_SENTINEL}\n")).unwrap();
    store.refresh(Depth::Bytes).unwrap();
    assert_eq!(store.whole_graph().unwrap().unreadable_files().len(), 1);
    fs::write(&bad, "- repaired, mentions [[Target]]\n").unwrap();
    store.refresh(Depth::Bytes).unwrap();
    let view = store.whole_graph().unwrap();
    assert!(view.unreadable_files().is_empty());
    assert_eq!(view.backlinks("Target").unwrap().len(), 3);
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

/// A held page's owner publishes `bytes`, as a window save does.
pub(super) fn publish_held(store: &Store, key: &str, bytes: String) {
    let _writer = store.writer.lock().unwrap();
    let bytes = Some(Arc::from(bytes.as_bytes()));
    assert!(store.graph.publish_owned(key, bytes, None).is_ok());
    let file = FileId::from(key.to_string());
    store.changes.publish(
        Origin::Own,
        vec![(file, ChangeKind::Modified, None)],
        false,
        Vec::new(),
    );
}

/// The window's saves publish a held page's row, which is its owner's: an
/// index-stage panic retires it all the same, and the owner's next publication
/// installs it again.
#[test]
fn a_held_page_that_panics_an_index_stage_costs_that_page_only() {
    let (root, store) = ready_graph_without_watcher("held-index");
    let others = others_cached(&store);
    let key = "pages/Filler 00.md";
    store.hold_page(&PageId::from(key)).unwrap();
    let publish = |bytes: String| publish_held(&store, key, bytes);
    publish(format!("- {TEST_INDEX_PANIC_SENTINEL}\n"));
    let view = store.whole_graph().unwrap();
    assert_eq!(
        view.unreadable_files().len(),
        1,
        "the held page is reported"
    );
    for (rel, before) in &others {
        assert!(Arc::ptr_eq(before, &cached_doc(&store, rel)), "{rel}");
    }
    assert_rest_of_graph_answers(&store, "held-index", 2);
    publish("- back, mentions [[Target]]\n".to_string());
    let view = store.whole_graph().unwrap();
    assert!(
        view.unreadable_files().is_empty(),
        "the owner's next bytes clear it"
    );
    assert_eq!(view.backlinks("Target").unwrap().len(), 3);
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

/// No page to blame: a build-relevant config edit rebuilds the indexes and
/// that build panics.
/// The graph fails and keeps the writer, and a retry recovers it.
#[test]
fn a_panic_no_page_explains_fails_the_graph_and_keeps_the_writer() {
    let (root, store) = ready_graph_without_watcher("config-index");
    let canonical = fs::canonicalize(&root).unwrap();
    fs::create_dir_all(root.join("logseq")).unwrap();
    fs::write(
        root.join("logseq/config.edn"),
        "{:property-pages/excludelist #{:unused}}\n",
    )
    .unwrap();
    crate::model::TEST_FULL_INDEX_PANIC_ROOTS
        .lock()
        .unwrap()
        .push(canonical.clone());
    let refreshed =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| store.refresh(Depth::Bytes)));
    let cleared = |store: &Store| {
        (crate::model::TEST_FULL_INDEX_PANIC_ROOTS.lock().unwrap()).retain(|r| *r != canonical);
        assert!(store.refresh(Depth::Stamps).is_ok());
    };
    assert!(refreshed.is_ok(), "the refresh must not unwind");
    let failed = matches!(store.whole_graph(), Err(LoadError::Failed { .. }));
    let good = store.page(&PageId::from("pages/Good.md")).unwrap();
    assert!(edit_good(&store, &good).is_ok());
    cleared(&store);
    assert!(failed, "the graph reports the failed publication");
    assert!(matches!(settled(&store), Ok(true)));
    assert_rest_of_graph_answers(&store, "config-index", 3);
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

// ---- The same doors for a journal -------------------------------------
//
// A journal day stays in the name index after its row is retired, so the
// publication's claimant pass finds a winner with no row and would load it
// again. These pin that it stays contained like a page.

const JOURNAL: &str = "journals/2026_01_05.md";

fn ready_graph_with_journal(tag: &str) -> (PathBuf, Arc<Store>) {
    let root = graph_with_bad_page(tag, "- fine for now\n", 2);
    fs::write(root.join(JOURNAL), "- journal fine\n").unwrap();
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    assert!(matches!(settled(&store), Ok(true)), "{tag}");
    store.watch.stop_background_for_test();
    (root, store)
}

fn assert_only_the_journal_lost(
    store: &Arc<Store>,
    root: &Path,
    tag: &str,
    others: &[(&str, Arc<tine_core::doc::Document>)],
) {
    let view = store.whole_graph().unwrap_or_else(|error| {
        panic!("{tag}: one bad journal must not fail the graph: {error:?}")
    });
    assert_eq!(
        (view.unreadable_files().iter())
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        vec![JOURNAL],
        "{tag}: only the bad journal is unreadable"
    );
    for (rel, before) in others {
        assert!(
            Arc::ptr_eq(before, &cached_doc(store, rel)),
            "{tag}: {rel} must not be re-read"
        );
    }
    assert_rest_of_graph_answers(store, tag, 3);
    assert!(
        store.page(&PageId::from(JOURNAL)).is_ok(),
        "{tag}: its parse is good, so the journal still opens"
    );
    let good = store.page_named("Good", PageKind::Page).unwrap().unwrap();
    assert!(edit_good(store, &good).is_ok(), "{tag}");
    assert!(fs::read_to_string(root.join("pages/Good.md"))
        .unwrap()
        .contains("edited after the load"));
}

#[test]
fn an_external_edit_that_panics_an_index_stage_costs_that_journal_only() {
    let (root, store) = ready_graph_with_journal("journal-external");
    let others = others_cached(&store);
    fs::write(
        root.join(JOURNAL),
        format!("- {TEST_INDEX_PANIC_SENTINEL}\n"),
    )
    .unwrap();
    assert!(store.refresh(Depth::Bytes).is_ok());
    assert_only_the_journal_lost(&store, &root, "journal-external", &others);
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn an_own_save_that_panics_an_index_stage_costs_that_journal_only() {
    let (root, store) = ready_graph_with_journal("journal-save");
    let others = others_cached(&store);
    let journal = store.page(&PageId::from(JOURNAL)).unwrap();
    let mut doc = journal.doc.clone();
    doc.blocks[0].raw = TEST_INDEX_PANIC_SENTINEL.into();
    let saved = crate::test_host_writer::HostWriter::start(&store).save(
        crate::EditKind::SaveBlock,
        &journal.id,
        SaveBase::Existing(journal.rev.clone()),
        &doc,
    );
    assert!(saved.is_ok());
    assert!(fs::read_to_string(root.join(JOURNAL))
        .unwrap()
        .contains(TEST_INDEX_PANIC_SENTINEL));
    assert_only_the_journal_lost(&store, &root, "journal-save", &others);
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn opening_a_journal_that_panics_an_index_stage_costs_that_journal_only() {
    let (root, store) = ready_graph_with_journal("journal-open");
    let others = others_cached(&store);
    fs::write(
        root.join(JOURNAL),
        format!("- {TEST_INDEX_PANIC_SENTINEL}\n"),
    )
    .unwrap();
    assert!(store.page(&PageId::from(JOURNAL)).is_ok());
    assert_only_the_journal_lost(&store, &root, "journal-open", &others);
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_watcher_survives_a_journal_edit_that_panics_an_index_stage() {
    let root = graph_with_bad_page("journal-watcher", "- fine for now\n", 2);
    fs::write(root.join(JOURNAL), "- journal fine\n").unwrap();
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    assert!(matches!(settled(&store), Ok(true)));
    fs::write(
        root.join(JOURNAL),
        format!("- {TEST_INDEX_PANIC_SENTINEL}\n"),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let view = store
            .whole_graph()
            .expect("one bad journal must not fail the graph");
        if (view.unreadable_files().iter()).any(|(id, _)| id.as_str() == JOURNAL) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the watcher never reported the bad journal"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(store.watcher_drained_for_tests(Duration::from_secs(60)));
    assert_rest_of_graph_answers(&store, "journal-watcher", 3);
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_batch_with_one_journal_that_panics_an_index_stage_publishes_the_others() {
    let (root, store) = ready_graph_with_journal("journal-batch");
    fs::write(
        root.join(JOURNAL),
        format!("- {TEST_INDEX_PANIC_SENTINEL}\n"),
    )
    .unwrap();
    fs::write(
        root.join("pages/Filler 01.md"),
        "- filler 1 mentions [[Target]] and [[Fresh]]\n",
    )
    .unwrap();
    store.refresh(Depth::Bytes).unwrap();
    let view = store.whole_graph().unwrap();
    assert_eq!(view.unreadable_files().len(), 1);
    assert_eq!(view.backlinks("Fresh").unwrap().len(), 1);
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_held_journal_that_panics_an_index_stage_costs_that_journal_only() {
    let (root, store) = ready_graph_with_journal("journal-held");
    let others = others_cached(&store);
    store.hold_page(&PageId::from(JOURNAL)).unwrap();
    let bad = format!("- {TEST_INDEX_PANIC_SENTINEL}\n");
    publish_held(&store, JOURNAL, bad.clone());
    // The owner publishing the same bytes again stays contained.
    publish_held(&store, JOURNAL, bad);
    let view = store
        .whole_graph()
        .expect("one bad held journal must not fail the graph");
    assert_eq!(view.unreadable_files().len(), 1);
    for (rel, before) in &others {
        assert!(Arc::ptr_eq(before, &cached_doc(&store, rel)), "{rel}");
    }
    assert_rest_of_graph_answers(&store, "journal-held", 3);
    publish_held(&store, JOURNAL, "- back, mentions [[Target]]\n".to_string());
    let view = store.whole_graph().unwrap();
    assert!(view.unreadable_files().is_empty(), "new bytes clear it");
    assert_eq!(view.backlinks("Target").unwrap().len(), 4);
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

// ---- One publication per containment -----------------------------------

/// Each read of a retired page used to install its row again, panic the
/// capture, retire it and publish an External change, which a window showing
/// a page no host holds answers with another read (`document/external.ts`).
/// Reading the same bytes now publishes nothing; new bytes are looked at anew.
#[test]
fn reading_a_contained_page_again_publishes_nothing() {
    for (tag, rel, name, kind, referrers) in [
        (
            "reread-page",
            "pages/Filler 00.md",
            "Filler 00",
            PageKind::Page,
            3,
        ),
        (
            "reread-journal",
            JOURNAL,
            "Jan 5th, 2026",
            PageKind::Journal,
            4,
        ),
    ] {
        let (root, store) = ready_graph_with_journal(tag);
        let id = PageId::from(rel);
        let bad = root.join(rel);
        fs::write(&bad, format!("- {TEST_INDEX_PANIC_SENTINEL}\n")).unwrap();
        let before = store.changes.rev();
        assert!(store.page(&id).is_ok(), "{tag}");
        let contained = store.changes.rev();
        assert_ne!(before, contained, "{tag}: the containment itself is told");
        for _ in 0..5 {
            let read = store.page(&id).unwrap();
            assert!(read.doc.blocks[0].raw.contains(TEST_INDEX_PANIC_SENTINEL));
            assert!(store.page_named(name, kind).unwrap().is_some(), "{tag}");
        }
        assert_eq!(
            contained,
            store.changes.rev(),
            "{tag}: reads publish nothing"
        );
        store.refresh(Depth::Bytes).unwrap();
        assert_eq!(contained, store.changes.rev(), "{tag}: nor does a rescan");
        let view = store.whole_graph().unwrap();
        assert_eq!(view.unreadable_files().len(), 1, "{tag}: it stays listed");
        // Other bad bytes are a new containment, told once more.
        fs::write(&bad, format!("- {TEST_INDEX_PANIC_SENTINEL} again\n")).unwrap();
        store.page(&id).unwrap();
        let again = store.changes.rev();
        assert_ne!(contained, again, "{tag}");
        store.page(&id).unwrap();
        assert_eq!(again, store.changes.rev(), "{tag}");
        // Repaired bytes come back without a restart.
        fs::write(&bad, "- repaired, mentions [[Target]]\n").unwrap();
        store.page(&id).unwrap();
        let view = store.whole_graph().unwrap();
        assert!(view.unreadable_files().is_empty(), "{tag}");
        assert_eq!(view.backlinks("Target").unwrap().len(), referrers, "{tag}");
        drop(store);
        let _ = fs::remove_dir_all(&root);
    }
}

/// A stale base on a retired page is still refused: retiring its row never
/// relaxes the guard, and the outside edit survives.
#[test]
fn a_contained_page_keeps_its_save_guard() {
    let (root, store) = ready_graph_with_journal("contained-guard");
    let id = PageId::from("pages/Filler 00.md");
    let bad = root.join("pages/Filler 00.md");
    fs::write(&bad, format!("- {TEST_INDEX_PANIC_SENTINEL}\n")).unwrap();
    let read = store.page(&id).unwrap();
    let outside = format!("- {TEST_INDEX_PANIC_SENTINEL} changed outside\n");
    fs::write(&bad, &outside).unwrap();
    let mut doc = read.doc.clone();
    doc.blocks[0].raw = "my stale edit".into();
    let stale = crate::test_host_writer::HostWriter::start(&store).save(
        crate::EditKind::SaveBlock,
        &id,
        SaveBase::Existing(read.rev.clone()),
        &doc,
    );
    assert!(stale.is_err(), "stale base admitted");
    assert_eq!(fs::read_to_string(&bad).unwrap(), outside);
    drop(store);
    let _ = fs::remove_dir_all(&root);
}
