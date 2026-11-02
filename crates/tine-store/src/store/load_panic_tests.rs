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
//! - A graph-wide index stage itself (pure Rust after the parse): the
//!   backstop. The load ends `Failed` promptly (reported, `refresh()`
//!   retries), pages still open, and the writer lock is not poisoned.
//! - The same stage in a later publication's patch, which names its pages:
//!   only the panicking page is lost, a journal day's file included, and it
//!   stays retired until its bytes change.
use super::*;
use std::time::{Duration, Instant};

use crate::model::{TEST_INDEX_PANIC_SENTINEL, TEST_PAGE_PARSE_PANIC_SENTINEL};
use tine_core::doc::TEST_PROJECTION_PANIC_SENTINEL;

fn graph_with_bad_page(tag: &str, bad: &str, fillers: usize) -> PathBuf {
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
fn edit_good(store: &Store, good: &PageRead) -> SaveOutcome {
    let mut doc = good.doc.clone();
    doc.blocks[0].raw = "links to [[Target]], edited after the load".into();
    store.save(
        crate::EditKind::SaveBlock,
        &good.id,
        SaveBase::Existing(good.rev.clone()),
        &doc,
    )
}

/// The load's settled state, polled: `Ok(true)` Ready, `Err(_)` Failed or
/// Closed. Panics (test failure) when still Loading after the deadline.
fn settled(store: &Store) -> Result<bool, LoadError> {
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
    let store = Store::open(&root, Default::default()).unwrap().0;
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
    assert!(
        matches!(edit_good(&store, &good), SaveOutcome::Saved(_)),
        "{tag}"
    );
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

#[test]
fn an_index_build_panic_fails_the_load_instead_of_hanging_it() {
    let root = graph_with_bad_page("index", &format!("- {TEST_INDEX_PANIC_SENTINEL}\n"), 2);
    let store = Store::open(&root, Default::default()).unwrap().0;
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
    assert!(matches!(edit_good(&store, &good), SaveOutcome::Saved(_)));
    assert!(fs::read_to_string(root.join("pages/Good.md"))
        .unwrap()
        .contains("edited after the load"));
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

/// After Ready, the later stages that read one page's parse: an external
/// edit (the watcher's and focus rescan's sync) and an own save. Each panics
/// a projection for that page only; the writer stays usable and the other
/// pages stay indexed.
fn assert_rest_of_graph_answers(store: &Store, tag: &str, referrers: usize) {
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
        let store = Store::open(&root, Default::default()).unwrap().0;
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
        assert!(
            matches!(edit_good(&store, &good), SaveOutcome::Saved(_)),
            "{tag}"
        );
        drop(store);
        let _ = fs::remove_dir_all(&root);
    }
}

#[test]
fn an_own_save_that_panics_a_projection_is_written_and_keeps_the_writer() {
    let root = graph_with_bad_page("own-save", "- fine for now\n", 2);
    let store = Store::open(&root, Default::default()).unwrap().0;
    assert!(matches!(settled(&store), Ok(true)));
    let filler = store.page(&PageId::from("pages/Filler 01.md")).unwrap();
    let mut doc = filler.doc.clone();
    doc.blocks[0].raw = format!("{TEST_PROJECTION_PANIC_SENTINEL} [[Target]]");
    let saved = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        store.save(
            crate::EditKind::SaveBlock,
            &filler.id,
            SaveBase::Existing(filler.rev.clone()),
            &doc,
        )
    }));
    assert!(
        matches!(saved, Ok(SaveOutcome::Saved(_))),
        "the guarded write lands; only its indexing fails"
    );
    assert!(fs::read_to_string(root.join("pages/Filler 01.md"))
        .unwrap()
        .contains(TEST_PROJECTION_PANIC_SENTINEL));
    assert_rest_of_graph_answers(&store, "own-save", 2);
    let good = store.page_named("Good", PageKind::Page).unwrap().unwrap();
    assert!(matches!(edit_good(&store, &good), SaveOutcome::Saved(_)));
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
    let store = Store::open(&root, Default::default()).unwrap().0;
    assert!(matches!(settled(&store), Ok(true)));
    let before = fs::read(root.join("pages/Bad.md")).unwrap();
    let map = crate::RenameMap(vec![("Bad".into(), "Renamed".into())]);
    let committed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut tx = store.page_plan(Some(crate::EditKind::RenamePage));
        tx.move_file(
            &FileId::from("pages/Bad.md".to_string()),
            crate::FileRev::from_file(&root.join("pages/Bad.md")).unwrap(),
            &FileId::from("pages/Renamed.md".to_string()),
            Some(&map),
        );
        tx.commit_internal()
    }));
    assert!(
        matches!(
            committed,
            Ok(crate::TxOutcome::NotCommitted {
                why: crate::Why::Refused(crate::Refusal::ReadOnly(_)),
                ..
            })
        ),
        "the move is refused, not unwound"
    );
    assert_eq!(fs::read(root.join("pages/Bad.md")).unwrap(), before);
    assert!(!root.join("pages/Renamed.md").exists());
    let good = store.page_named("Good", PageKind::Page).unwrap().unwrap();
    assert!(matches!(edit_good(&store, &good), SaveOutcome::Saved(_)));
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
        let store = Store::open(&root, options()).unwrap().0;
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
        let store = Store::open(&root, options()).unwrap().0;
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
        assert!(
            matches!(edit_good(&store, &good), SaveOutcome::Saved(_)),
            "{tag}"
        );
        drop(store);
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_file(&cp);
    }
}

/// The launch's own `Failed` path: a config file edited during the launch diff
/// rebuilds the indexes in its publication, and that build names no page.
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
    let store = Store::open(&root, options()).unwrap().0;
    let deadline = Instant::now() + Duration::from_secs(60);
    while !store.load.serving.load(Ordering::Acquire) {
        assert!(Instant::now() < deadline, "the checkpoint was not served");
        std::thread::sleep(Duration::from_millis(1));
    }
    fs::create_dir_all(root.join("logseq")).unwrap();
    fs::write(
        root.join("logseq/config.edn"),
        "{:ref/linked-references-collapsed-threshold 7}\n",
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
    assert!(matches!(edit_good(&store, &good), SaveOutcome::Saved(_)));
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
    let store = Store::open(&root, Default::default()).unwrap().0;
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
    assert!(matches!(edit_good(&store, &good), SaveOutcome::Saved(_)));
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

/// The page cache's document for `rel`, for identity comparison: the same
/// `Arc` afterwards means that page was neither re-read nor re-parsed.
fn cached_doc(store: &Store, rel: &str) -> Arc<tine_core::doc::Document> {
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
    store: &Store,
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
    assert!(
        matches!(edit_good(store, &good), SaveOutcome::Saved(_)),
        "{tag}"
    );
    assert!(fs::read_to_string(root.join("pages/Good.md"))
        .unwrap()
        .contains("edited after the load"));
}

fn ready_graph_without_watcher(tag: &str) -> (PathBuf, Store) {
    let root = graph_with_bad_page(tag, "- fine for now\n", 2);
    let store = Store::open(&root, Default::default()).unwrap().0;
    assert!(matches!(settled(&store), Ok(true)), "{tag}");
    store.watch.stop_background_for_test();
    (root, store)
}

fn others_cached(store: &Store) -> Vec<(&'static str, Arc<tine_core::doc::Document>)> {
    ["pages/Good.md", "pages/Target.md", "pages/Filler 01.md"]
        .map(|rel| (rel, cached_doc(store, rel)))
        .to_vec()
}

/// Queue card: "bad-page open wipes the page cache".
#[test]
fn opening_a_bad_page_keeps_every_other_page_cached() {
    let root = graph_with_bad_page("open-keeps-cache", "- fine for now\n", 2);
    let store = Store::open(&root, Default::default()).unwrap().0;
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
    let (root, store) = ready_graph_without_watcher("save-keeps-cache");
    let others = others_cached(&store);
    let filler = store.page(&PageId::from("pages/Filler 00.md")).unwrap();
    let mut doc = filler.doc.clone();
    doc.blocks[0].raw = TEST_PROJECTION_PANIC_SENTINEL.into();
    let saved = store.save(
        crate::EditKind::SaveBlock,
        &filler.id,
        SaveBase::Existing(filler.rev.clone()),
        &doc,
    );
    assert!(matches!(saved, SaveOutcome::Saved(_)));
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
        store.save(
            crate::EditKind::SaveBlock,
            &filler.id,
            SaveBase::Existing(filler.rev.clone()),
            &doc,
        )
    }));
    assert!(
        matches!(saved, Ok(SaveOutcome::Saved(_))),
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
    let store = Store::open(&root, Default::default()).unwrap().0;
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
    assert!(matches!(edit_good(&store, &good), SaveOutcome::Saved(_)));
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
fn publish_held(store: &Store, key: &str, bytes: String) {
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

/// No page to blame: a config edit rebuilds the indexes and that build panics.
/// The graph fails and keeps the writer, and a retry recovers it.
#[test]
fn a_panic_no_page_explains_fails_the_graph_and_keeps_the_writer() {
    let (root, store) = ready_graph_without_watcher("config-index");
    let canonical = fs::canonicalize(&root).unwrap();
    fs::create_dir_all(root.join("logseq")).unwrap();
    fs::write(
        root.join("logseq/config.edn"),
        "{:ref/linked-references-collapsed-threshold 7}\n",
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
    assert!(matches!(edit_good(&store, &good), SaveOutcome::Saved(_)));
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

fn ready_graph_with_journal(tag: &str) -> (PathBuf, Store) {
    let root = graph_with_bad_page(tag, "- fine for now\n", 2);
    fs::write(root.join(JOURNAL), "- journal fine\n").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    assert!(matches!(settled(&store), Ok(true)), "{tag}");
    store.watch.stop_background_for_test();
    (root, store)
}

fn assert_only_the_journal_lost(
    store: &Store,
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
    assert!(
        matches!(edit_good(store, &good), SaveOutcome::Saved(_)),
        "{tag}"
    );
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
    let saved = store.save(
        crate::EditKind::SaveBlock,
        &journal.id,
        SaveBase::Existing(journal.rev.clone()),
        &doc,
    );
    assert!(matches!(saved, SaveOutcome::Saved(_)));
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
    let store = Store::open(&root, Default::default()).unwrap().0;
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
    let stale = store.save(
        crate::EditKind::SaveBlock,
        &id,
        SaveBase::Existing(read.rev.clone()),
        &doc,
    );
    assert!(
        !matches!(stale, SaveOutcome::Saved(_)),
        "stale base admitted"
    );
    assert_eq!(fs::read_to_string(&bad).unwrap(), outside);
    drop(store);
    let _ = fs::remove_dir_all(&root);
}
