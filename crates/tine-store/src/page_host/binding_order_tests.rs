//! A running rename's in-memory order and its caller's reply (STEP3-DESIGN
//! "Martin decision (2026-10-10 evening): rename ordering = option 2"),
//! against a real Store: dst, then the referrers, then the source's
//! deletion while Tine runs; every caller exit removes its slot (Finding B,
//! R2); a stop carries gated pages by their drafts (R3); a Discard of the
//! unwitnessed destination cancels in memory; D4 never chains.
use super::super::retained::{RenameRefusal, OPERATION_WAIT};
use super::*;
use crate::page_host::io::Phase;
use crate::RenameMap;

const OLD: &str = "pages/Old.md";
const NEW: &str = "pages/New.md";
const REF: &str = "pages/r.md";

fn graph() -> Live {
    Live::new(&[(OLD, "- old\n"), (REF, "- see [[Old]]\n")])
}

/// The rename `pages.rs` runs, on a fresh view (one attempt: nothing else
/// publishes in these tests).
fn rename(host: &PageHost, store: &Store) -> Result<(Vec<PageId>, Vec<PageId>), RenameRefusal> {
    let view = store.whole_graph().unwrap();
    host.rename(
        &PageId::from(OLD),
        &PageId::from(NEW),
        &[PageId::from(REF)],
        &RenameMap(vec![("Old".into(), "New".into())]),
        &view,
        &|_, _, _| None,
    )
}

fn clear_faults(live: &Live) {
    live.host
        .driver
        .shared
        .with_state(|state| state.progress.host.fs.faults.clear());
}

/// (running orders, caller slots).
fn census(host: &PageHost) -> (usize, usize) {
    host.driver.shared.with_state(|state| {
        let order = &state.progress.host.order;
        (order.view().count(), order.slots())
    })
}

fn until(what: &str, test: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !test() {
        assert!(Instant::now() < deadline, "never: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Option 2's in-run guarantee: while the destination's save keeps
/// failing, neither the referrer's rewrite nor the source's deletion
/// publishes, so the old file and every old link still resolve. The caller
/// hears no success; its slot is gone when it returns. Once the destination
/// publishes, the referrer and then the deletion follow and the order ends.
#[test]
fn a_rename_publishes_its_destination_before_the_referrer_and_the_deletion() {
    let live = graph();
    live.faults(Phase::PageTemp, 10_000);
    let renamed = rename(&live.host, &live.store);
    assert!(
        matches!(
            &renamed,
            Err(RenameRefusal::Unwritten(page)) if page.as_str() == NEW
        ) || renamed == Err(RenameRefusal::Uncertain),
        "a failing destination is never success: {renamed:?}"
    );
    assert_eq!(census(&live.host).1, 0, "R2: the caller removed its slot");
    assert_eq!(census(&live.host).0, 1, "the order runs on");
    // Give a gate-ignoring host every chance to publish the gated pages.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(live.disk(OLD), "- old\n", "the source waits for every page");
    assert_eq!(
        live.disk(REF),
        "- see [[Old]]\n",
        "the referrer waits for dst"
    );
    assert!(!live.root.join(NEW).exists());
    clear_faults(&live);
    until("the rename completes", || {
        !live.root.join(OLD).exists()
            && live.root.join(NEW).exists()
            && fs::read_to_string(live.root.join(REF)).unwrap() == "- see [[New]]\n"
    });
    until("the order ends", || census(&live.host) == (0, 0));
    assert_eq!(live.disk(NEW), "- old\n");
    live.host.stop();
}

/// R3: a stop while a rename is gated saves nothing gated. The destination
/// (never gated) gets its save attempt and is drafted when it fails; the
/// referrer and the source are carried by the operation's draft. The disk
/// keeps the old state; the stopped host's down ends the order (in memory).
#[test]
fn a_stop_carries_a_gated_rename_by_its_draft() {
    let live = graph();
    live.faults(Phase::PageTemp, 10_000);
    let renamed = rename(&live.host, &live.store);
    assert!(renamed.is_err(), "{renamed:?}");
    // No window request was admitted: the window consumed id 0.
    assert!(live.host.stop_begin(0, StopMode::Switch));
    assert_eq!(live.until_stop(), StopState::Ready);
    assert!(!live.drafts().is_empty(), "the rename's input is drafted");
    assert_eq!(live.disk(OLD), "- old\n");
    assert_eq!(live.disk(REF), "- see [[Old]]\n");
    assert!(!live.root.join(NEW).exists());
    assert_eq!(
        census(&live.host).1,
        0,
        "no caller slot outlives its caller"
    );
    let Live { host, .. } = live;
    assert!(host.stop_finish().is_ok());
}

/// In-memory cancellation: the window discards the unwitnessed destination
/// while the caller waits. The source takes its disk bytes again (it is
/// never trashed), the order ends, and the caller hears Superseded, not
/// success. The referrer's rewrite is then an ordinary save of a link to a
/// page that does not exist (plain Logseq).
#[test]
fn a_discard_of_the_unwitnessed_destination_cancels_the_rename() {
    let live = graph();
    live.faults(Phase::PageTemp, 10_000);
    let renamed = std::thread::scope(|scope| {
        let caller = scope.spawn(|| rename(&live.host, &live.store));
        until("the rename is admitted", || census(&live.host).0 == 1);
        let (key, page) = live.open(NEW);
        let id = live.id();
        live.host
            .discard(live.host.session(), id, &key, page.version)
            .unwrap();
        live.answer(&key, id);
        caller.join().unwrap()
    });
    assert_eq!(renamed, Err(RenameRefusal::Superseded(PageId::from(NEW))));
    assert_eq!(census(&live.host), (0, 0));
    clear_faults(&live);
    until("the referrer saves", || {
        fs::read_to_string(live.root.join(REF)).unwrap() == "- see [[New]]\n"
    });
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        live.disk(OLD),
        "- old\n",
        "a cancelled rename never trashes src"
    );
    assert!(!live.root.join(NEW).exists());
    live.host.stop();
}

/// D4, no chaining: an operation over a page of a running order waits
/// until that order ends, even over a clean, witnessed page. Here the window
/// sends stale input to the referrer, so it conflicts and the order (whose
/// source waits for it) keeps running after the destination published. A rename
/// caller not admitted within `OPERATION_WAIT` hears Busy and leaves no
/// slot; a deletion is Waiting at once.
#[test]
fn an_operation_over_a_running_order_is_busy_until_the_order_ends() {
    let live = graph();
    live.faults(Phase::PageTemp, 10_000);
    assert!(rename(&live.host, &live.store).is_err());
    let (key, _) = live.open(REF);
    // STALE (0): input on a version the window never saw becomes a conflict.
    let id = live.submit(&key, "- typed\n", 0, None).unwrap();
    live.answer(&key, id);
    until("the referrer conflicts", || {
        live.host.driver.shared.with_state(|state| {
            state
                .progress
                .host
                .pages
                .get(REF)
                .is_some_and(|p| p.conflict)
        })
    });
    clear_faults(&live);
    until("the destination publishes", || live.root.join(NEW).exists());
    // Witnessed: published and released (no window holds it), or clean.
    until("the destination is witnessed", || {
        live.host
            .driver
            .shared
            .with_state(|state| state.progress.host.pages.get(NEW).is_none_or(|p| p.clean()))
    });
    assert_eq!(
        live.host
            .delete(live.host.session(), &PageId::from(NEW), b"- old\n"),
        PageOperation::Waiting
    );
    let started = Instant::now();
    let view = live.store.whole_graph().unwrap();
    let again = live.host.rename(
        &PageId::from(NEW),
        &PageId::from("pages/Newer.md"),
        &[],
        &RenameMap(vec![("New".into(), "Newer".into())]),
        &view,
        &|_, _, _| None,
    );
    assert_eq!(again, Err(RenameRefusal::Busy));
    assert!(started.elapsed() >= OPERATION_WAIT);
    assert_eq!(
        census(&live.host),
        (1, 0),
        "the first order runs; no slot is left"
    );
    assert_eq!(
        live.disk(OLD),
        "- old\n",
        "the source still waits for the referrer"
    );
    assert!(!live.root.join("pages/Newer.md").exists());
    live.host.stop();
}

/// R2: a deletion's caller stops waiting once its window session is not
/// current (F1) and hears Uncertain, never Applied; its slot is gone, and
/// the deletion still completes once it can.
#[test]
fn a_deletion_wait_ends_uncertain_when_the_session_changes() {
    let live = Live::new(&[("pages/gone.md", "- gone\n")]);
    live.faults(Phase::TrashMove, 10_000);
    let session = live.host.session();
    let started = Instant::now();
    let deleted = std::thread::scope(|scope| {
        let caller = scope.spawn(|| {
            live.host
                .delete(session, &PageId::from("pages/gone.md"), b"- gone\n")
        });
        until("the deletion is admitted", || {
            live.host
                .driver
                .shared
                .with_state(|state| state.progress.host.order.slots() == 1)
        });
        live.host.window_reloaded();
        caller.join().unwrap()
    });
    assert_eq!(deleted, PageOperation::Uncertain);
    assert!(
        started.elapsed() < OPERATION_WAIT,
        "the reload ended the wait"
    );
    assert_eq!(census(&live.host), (0, 0));
    clear_faults(&live);
    until("the deletion completes", || {
        !live.root.join("pages/gone.md").exists()
    });
    live.host.stop();
}

/// R2: a caller waiting on a poisoned host hears Uncertain, and removing
/// its slot on the way out does not panic (an unwind would abort the app).
#[test]
fn a_caller_on_a_poisoned_host_hears_uncertain_and_leaves_cleanly() {
    let live = Live::new(&[("pages/gone.md", "- gone\n")]);
    live.faults(Phase::TrashMove, 10_000);
    let session = live.host.session();
    let shared = Arc::clone(&live.host.driver.shared);
    let deleted = std::thread::scope(|scope| {
        let caller = scope.spawn(|| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                live.host
                    .delete(session, &PageId::from("pages/gone.md"), b"- gone\n")
            }))
        });
        until("the deletion is admitted", || {
            shared.with_state(|state| state.progress.host.order.slots() == 1)
        });
        let poisoner = std::thread::spawn(move || {
            let _state = shared.state.lock().unwrap();
            panic!("R2 test: poison the host state");
        });
        assert!(poisoner.join().is_err());
        caller.join().unwrap()
    });
    assert_eq!(
        deleted.ok(),
        Some(PageOperation::Uncertain),
        "R2: a poisoned wait is Uncertain and its slot's drop does not panic"
    );
    // The host is failed by construction; its own teardown panics.
    drop(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || drop(live),
    )));
}

/// R1: an operation whose draft cannot be written changes nothing and says
/// so (DraftFailed: a full disk or an app-data disk error), installs no
/// order and leaves no slot.
#[test]
fn a_rename_whose_draft_fails_changes_nothing() {
    let live = graph();
    live.faults(Phase::DraftTemp, 10_000);
    assert_eq!(
        rename(&live.host, &live.store),
        Err(RenameRefusal::DraftFailed)
    );
    assert_eq!(census(&live.host), (0, 0));
    clear_faults(&live);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(live.disk(OLD), "- old\n");
    assert_eq!(live.disk(REF), "- see [[Old]]\n");
    assert!(!live.root.join(NEW).exists());
    live.host.stop();
}
