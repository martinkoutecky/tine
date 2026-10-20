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
        // Filler 00's reference is gone with its parse; the rest answer.
        assert_rest_of_graph_answers(&store, tag, 2);
        let opened = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            store.page(&PageId::from("pages/Filler 00.md"))
        }));
        assert!(
            matches!(opened, Ok(Err(StoreError::Unparseable(_)))),
            "{tag}: opening the edited page is a per-page error"
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
        let mut tx = store.transaction(Some(crate::EditKind::RenamePage));
        tx.move_file(
            &FileId::from("pages/Bad.md".to_string()),
            crate::FileRev::from_file(&root.join("pages/Bad.md")).unwrap(),
            &FileId::from("pages/Renamed.md".to_string()),
            Some(&map),
        );
        tx.commit()
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
/// first publication then builds the indexes. A projection panic there
/// leaves that one file unreadable; an index-stage panic ends the load
/// Failed instead of leaving it Loading.
#[test]
fn a_checkpoint_launch_contains_a_bad_page_created_while_closed() {
    for (tag, bad, ready) in [
        (
            "checkpoint-projection",
            format!("- {TEST_PROJECTION_PANIC_SENTINEL} [[Target]]\n"),
            true,
        ),
        (
            "checkpoint-index",
            format!("- {TEST_INDEX_PANIC_SENTINEL}\n"),
            false,
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
        fs::write(root.join("pages/Bad.md"), &bad).unwrap();
        let store = Store::open(&root, options()).unwrap().0;
        let outcome = settled(&store);
        if ready {
            assert!(matches!(outcome, Ok(true)), "{tag}: {outcome:?}");
            let view = store.whole_graph().unwrap();
            assert!(
                (view.unreadable_files().iter()).any(|(id, _)| id.as_str() == "pages/Bad.md"),
                "{tag}: {:?}",
                view.unreadable_files()
            );
            assert_rest_of_graph_answers(&store, tag, 3);
        } else {
            assert!(
                matches!(outcome, Err(LoadError::Failed { .. })),
                "{tag}: {outcome:?}"
            );
        }
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
