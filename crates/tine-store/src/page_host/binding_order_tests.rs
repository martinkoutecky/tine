//! A running rename's in-memory order and its caller's reply (STEP3-DESIGN
//! "Martin decision (2026-10-10 evening): rename ordering = option 2"),
//! against a real Store: dst, then the referrers, then the source's
//! deletion while Tine runs; every caller exit removes its slot (Finding B,
//! R2); a stop carries gated pages by their drafts (R3); a Discard of the
//! unwitnessed destination cancels in memory; D4 never chains.
use super::super::retained::{RenameRefusal, ReserveRefusal, OPERATION_WAIT};
use super::*;
use crate::page_host::io::Phase;
use crate::RenameMap;

const OLD: &str = "pages/Old.md";
const NEW: &str = "pages/New.md";
const REF: &str = "pages/r.md";

fn graph() -> Live {
    Live::new(&[(OLD, "- old\n"), (REF, "- see [[Old]]\n")])
}

/// The rename `pages.rs` runs: Old to New over the referrer.
fn rename(host: &PageHost, store: &Store) -> Result<(Vec<PageId>, Vec<PageId>), RenameRefusal> {
    replanned(host, store, OLD, NEW, &[REF], ("Old", "New"))
}

/// A host rename as `pages.rs` drives it: planned in a fresh view, and
/// replanned in a new one when a publication made it stale (B2: `Refused`
/// is `pages.rs`'s replan). A save's publication bumps the graph revision
/// after the host already shows the page saved, so a view taken right
/// after a save can be stale by the time the host checks it.
fn replanned(
    host: &PageHost,
    store: &Store,
    source: &str,
    target: &str,
    referrers: &[&str],
    (old, new): (&str, &str),
) -> Result<(Vec<PageId>, Vec<PageId>), RenameRefusal> {
    let referrers: Vec<PageId> = referrers.iter().map(|page| PageId::from(*page)).collect();
    let map = RenameMap(vec![(old.into(), new.into())]);
    for _ in 0..20 {
        let view = store.whole_graph().unwrap();
        match host.rename(
            &PageId::from(source),
            &PageId::from(target),
            &referrers,
            &map,
            &view,
            Arc::new(|_: &PageId, _: &[u8], _| None),
        ) {
            Err(RenameRefusal::Refused) => continue,
            done => return done,
        }
    }
    Err(RenameRefusal::Refused)
}

fn clear_faults(live: &Live) {
    live.host
        .driver
        .shared
        .with_state_for_tests(|state| state.progress.host.fs.faults.clear());
}

/// (running orders, caller slots).
/// Read through the driver's queue, after every notice sent before it (a
/// slot's or a reservation's release).
fn census(host: &PageHost) -> (usize, usize) {
    let read = host.driver.shared.read(|state| {
        let order = &state.progress.host.order;
        (order.view().count(), order.slots())
    });
    read.unwrap()
}

/// A writer parked in the flush pause, undone on drop: faults cleared, the
/// pause resumed, then `_held` released. A failed `until` inside a thread
/// scope then fails the test instead of hanging the scope's join on the
/// parked writer (seen on hosted Windows, run 38130589757 attempt 2).
/// Poison-tolerant: it also runs while a failed assertion unwinds.
struct Unpark<'a, H> {
    live: &'a Live,
    pause: crate::store::TestPause,
    _held: H,
}

impl<H> Drop for Unpark<'_, H> {
    fn drop(&mut self) {
        let mut state = self.live.host.driver.shared.state_for_tests();
        state.progress.host.fs.faults.clear();
        drop(state);
        let (state, resumed) = &*self.pause;
        state.lock().unwrap_or_else(|poison| poison.into_inner()).1 = true;
        resumed.notify_all();
    }
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
        let (key, _) = live.open(NEW);
        let id = live.id();
        live.host.discard(live.host.session(), id, &key).unwrap();
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
        live.host.driver.shared.with_state_for_tests(|state| {
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
        live.host.driver.shared.with_state_for_tests(|state| {
            state.progress.host.pages.get(NEW).is_none_or(|p| p.clean())
        })
    });
    assert_eq!(
        live.host
            .delete(live.host.session(), &PageId::from(NEW), b"- old\n"),
        PageOperation::Waiting
    );
    let started = Instant::now();
    let again = replanned(
        &live.host,
        &live.store,
        NEW,
        "pages/Newer.md",
        &[],
        ("New", "Newer"),
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
                .with_state_for_tests(|state| state.progress.host.order.slots() == 1)
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
/// The poison lands under the same lock that first sees the slot: the
/// operation's saves skip the debounce (Caller urgency), so a poison that
/// came later could meet a caller already told `Pending` by its third
/// failed trash move (100 + 300 ms of backoff after the first). The failed
/// host is torn down before any assertion (`drop_failed`): a failure's
/// unwind through its panicking teardown would abort the test binary.
#[test]
fn a_caller_on_a_poisoned_host_hears_uncertain_and_leaves_cleanly() {
    let live = Live::new(&[("pages/gone.md", "- gone\n")]);
    live.faults(Phase::TrashMove, 10_000);
    let session = live.host.session();
    let shared = Arc::clone(&live.host.driver.shared);
    let (deleted, poisoned) = std::thread::scope(|scope| {
        let caller = scope.spawn(|| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                live.host
                    .delete(session, &PageId::from("pages/gone.md"), b"- gone\n")
            }))
        });
        let poisoner = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(20);
            while Instant::now() < deadline {
                let state = shared.state_for_tests();
                if state.progress.host.order.slots() == 1 {
                    panic!("R2 test: poison the host state once the deletion is admitted");
                }
                drop(state);
                std::thread::yield_now();
            }
        });
        let poisoned = poisoner.join().is_err();
        (caller.join().unwrap(), poisoned)
    });
    drop_failed(live);
    assert!(poisoned, "never: the deletion is admitted");
    assert_eq!(
        deleted.ok(),
        Some(PageOperation::Uncertain),
        "R2: a poisoned wait is Uncertain and its slot's drop does not panic"
    );
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

/// Reserve `pages` for a retained writer on another thread; the result
/// arrives on the returned channel (true: reserved, false: refused).
fn reserving<'s>(
    scope: &'s std::thread::Scope<'s, '_>,
    host: &'s PageHost,
    pages: &'static [&'static str],
    input: Input,
) -> mpsc::Receiver<bool> {
    let (sender, result) = mpsc::channel();
    scope.spawn(move || {
        let reserved = host.reserve(|| pages.iter().map(|p| PageId::from(*p)).collect(), input);
        let _ = sender.send(reserved.is_ok());
    });
    result
}

const OTHER: &str = "pages/other.md";

/// REVIEW-3b-P2b R2 (D4, no chaining): a retained writer's reservation over
/// a page of a running order waits, holding no lock, until that order ends,
/// even over its clean, witnessed destination; unrelated pages stay
/// available. Here the referrer conflicts, so the order outlives the
/// destination's publication; the window taking disk for the referrer lets
/// the source's deletion finish, the order ends and the reservation is made
/// (inside `OPERATION_WAIT`; past it the writer is refused, M6).
#[test]
fn a_retained_writer_over_a_running_order_waits_until_it_completes() {
    let live = Live::new(&[
        (OLD, "- old\n"),
        (REF, "- see [[Old]]\n"),
        (OTHER, "- other\n"),
    ]);
    live.faults(Phase::PageTemp, 10_000);
    assert!(rename(&live.host, &live.store).is_err());
    let (key, _) = live.open(REF);
    let id = live.submit(&key, "- typed\n", 0, None).unwrap();
    live.answer(&key, id);
    clear_faults(&live);
    until("the destination is witnessed", || {
        live.root.join(NEW).exists()
            && live.host.driver.shared.with_state_for_tests(|state| {
                state.progress.host.pages.get(NEW).is_none_or(|p| p.clean())
                    && state
                        .progress
                        .host
                        .pages
                        .get(REF)
                        .is_some_and(|p| p.conflict)
            })
    });
    assert_eq!(census(&live.host).0, 1, "the order runs on");
    std::thread::scope(|scope| {
        let waiting = reserving(scope, &live.host, &[NEW], Input::Refuse);
        assert_eq!(
            waiting.recv_timeout(Duration::from_millis(500)),
            Err(mpsc::RecvTimeoutError::Timeout),
            "R2: a retained writer over the running order's destination waits"
        );
        let unrelated = live
            .host
            .reserve(|| vec![PageId::from(OTHER)], Input::Refuse);
        assert!(unrelated.is_ok(), "an unrelated writer stays available");
        drop(unrelated);
        let id = live.id();
        live.host.discard(live.host.session(), id, &key).unwrap();
        live.answer(&key, id);
        assert_eq!(
            waiting.recv_timeout(Duration::from_secs(20)),
            Ok(true),
            "the order's completion lets the writer in"
        );
    });
    assert_eq!(census(&live.host), (0, 0));
    assert!(!live.root.join(OLD).exists(), "the rename completed");
    live.host.stop();
}

/// R2: the same wait ends when the window cancels the order by discarding
/// its unwitnessed destination. The writer over the order's source (its
/// gated deletion) then finds the source back at its disk bytes, clean.
#[test]
fn a_retained_writer_over_a_running_order_waits_until_it_is_cancelled() {
    let live = Live::new(&[
        (OLD, "- old\n"),
        (REF, "- see [[Old]]\n"),
        (OTHER, "- other\n"),
    ]);
    live.faults(Phase::PageTemp, 10_000);
    assert!(rename(&live.host, &live.store).is_err());
    std::thread::scope(|scope| {
        let waiting = reserving(scope, &live.host, &[OLD], Input::Refuse);
        assert_eq!(
            waiting.recv_timeout(Duration::from_millis(500)),
            Err(mpsc::RecvTimeoutError::Timeout),
            "R2: a retained writer over the running order's source waits"
        );
        let unrelated = live
            .host
            .reserve(|| vec![PageId::from(OTHER)], Input::Refuse);
        assert!(unrelated.is_ok(), "an unrelated writer stays available");
        drop(unrelated);
        clear_faults(&live);
        let (key, _) = live.open(NEW);
        let id = live.id();
        live.host.discard(live.host.session(), id, &key).unwrap();
        live.answer(&key, id);
        assert_eq!(
            waiting.recv_timeout(Duration::from_secs(20)),
            Ok(true),
            "the order's cancellation lets the writer in"
        );
    });
    assert_eq!(census(&live.host), (0, 0));
    assert_eq!(
        live.disk(OLD),
        "- old\n",
        "a cancelled rename never trashes src"
    );
    live.host.stop();
}

/// M6 (STEP3-DESIGN "Manager decisions on REVIEW-progress-plan", R4
/// interpretation 2): the bound also covers a rename admitted but not yet
/// applied. Its operation draft cannot be made durable (an app-data disk
/// error), so its worker keeps retrying with no order installed and its
/// pages busy. A writer over one of them is refused `Busy` once
/// `OPERATION_WAIT` has passed since its start, with nothing reserved; an
/// unrelated writer is not held up. Once the disk recovers the operation
/// ends (nothing changed) and the same writer reserves. A references-only
/// rename runs as the same caller operation worker.
#[test]
fn a_retained_writer_over_an_unapplied_rename_is_told_busy_after_the_bound() {
    let live = Live::new(&[
        (OLD, "- old\n"),
        (REF, "- see [[Old]]\n"),
        (OTHER, "- other\n"),
    ]);
    live.faults(Phase::DraftSync, 10_000);
    let worker = || {
        live.host
            .driver
            .shared
            .with_state_for_tests(|state| state.progress.host.worker.is_some())
    };
    std::thread::scope(|scope| {
        let renamed = scope.spawn(|| rename(&live.host, &live.store));
        until("the rename is admitted", worker);
        let (sender, refused) = mpsc::channel();
        let started = Instant::now();
        let host = &live.host;
        scope.spawn(move || {
            let reserved = host.reserve(|| vec![PageId::from(REF)], Input::Refuse);
            let _ = sender.send(reserved.err());
        });
        let unrelated = live
            .host
            .reserve(|| vec![PageId::from(OTHER)], Input::Refuse);
        assert!(unrelated.is_ok(), "an unrelated writer stays available");
        drop(unrelated);
        let outcome = refused.recv_timeout(OPERATION_WAIT + Duration::from_secs(3));
        if outcome.is_err() {
            // Unblock a writer that still waits (the pre-M6 fence).
            clear_faults(&live);
        }
        assert_eq!(
            outcome,
            Ok(Some(ReserveRefusal::Busy)),
            "M6: a writer over an admitted rename's pages is refused Busy within its bound"
        );
        assert!(started.elapsed() >= OPERATION_WAIT);
        live.host.driver.shared.with_state_for_tests(|state| {
            assert!(
                state.progress.host.retained.is_empty(),
                "nothing is reserved"
            );
            assert!(
                state.progress.host.worker.is_some(),
                "the rename still runs"
            );
        });
        assert_eq!(census(&live.host).0, 0, "no order is installed yet");
        clear_faults(&live);
        assert!(
            renamed.join().unwrap().is_err(),
            "the caller hears no success"
        );
    });
    until("the operation ends", || !worker());
    assert_eq!(live.disk(OLD), "- old\n");
    assert_eq!(live.disk(REF), "- see [[Old]]\n");
    let reservation = live.host.reserve(|| vec![PageId::from(REF)], Input::Refuse);
    assert!(
        reservation.is_ok(),
        "the writer reserves once the operation ended"
    );
    drop(reservation);
    live.host.stop();
}

/// M6: the operations the retained writer's bound covers. A full rename and
/// a references-only rename (the source has no file) are renames while
/// their operation worker runs; a deletion writes no page and is not, so a
/// writer over it still waits for it.
#[test]
fn the_writer_bound_covers_full_and_references_only_renames_not_a_deletion() {
    let observe = |pages: &[(&str, &str)], run: &(dyn Fn(&PageHost, &Store) + Sync)| -> bool {
        let live = Live::new(pages);
        live.faults(Phase::DraftSync, 10_000);
        let flag = || {
            live.host.driver.shared.with_state_for_tests(|state| {
                state
                    .progress
                    .host
                    .worker
                    .as_ref()
                    .map(|worker| worker.rename)
            })
        };
        let rename = std::thread::scope(|scope| {
            let (host, store) = (&live.host, &*live.store);
            let running = scope.spawn(move || run(host, store));
            until("the operation is admitted", || flag().is_some());
            let rename = flag().unwrap();
            clear_faults(&live);
            running.join().unwrap();
            rename
        });
        live.host.stop();
        rename
    };
    let renamed = |host: &PageHost, store: &Store| drop(rename(host, store));
    assert!(observe(
        &[(OLD, "- old\n"), (REF, "- see [[Old]]\n")],
        &renamed
    ));
    assert!(
        observe(&[(REF, "- see [[Old]]\n")], &renamed),
        "references only"
    );
    let deleted = |host: &PageHost, _: &Store| {
        let _ = host.delete(host.session(), &PageId::from(OLD), b"- old\n");
    };
    assert!(!observe(&[(OLD, "- old\n")], &deleted), "a deletion");
}

/// REVIEW-3b-P2b-2 N1: the flush shares the writer's deadline. A flush-first
/// writer finds Old's edit unsaved and waits for its save holding nothing;
/// the edit saves, and a rename over Old is admitted whose draft keeps
/// failing. Old is now that rename's input, so the writer is refused Busy
/// once `OPERATION_WAIT` has passed since it started, nothing reserved.
#[test]
fn a_flush_handed_to_a_rename_is_told_busy_after_the_bound() {
    let live = graph();
    live.submits_wait();
    let (key, page) = live.open(OLD);
    live.faults(Phase::PageTemp, 10_000);
    let id = live.submit(&key, "- edited\n", page.version, None).unwrap();
    live.answer(&key, id);
    let pause: crate::store::TestPause = Default::default();
    *live.host.driver.shared.flush_pause.lock().unwrap() = Some(Arc::clone(&pause));
    let worker = || {
        live.host
            .driver
            .shared
            .with_state_for_tests(|state| state.progress.host.worker.is_some())
    };
    std::thread::scope(|scope| {
        let _unpark = Unpark {
            live: &live,
            pause: Arc::clone(&pause),
            _held: (),
        };
        let (sender, refused) = mpsc::channel();
        let started = Instant::now();
        let host = &live.host;
        scope.spawn(move || {
            let reserved = host.reserve(|| vec![PageId::from(OLD)], Input::Flush);
            let _ = sender.send(reserved.err());
        });
        until("the writer waits in its flush", || {
            pause.0.lock().unwrap().0
        });
        clear_faults(&live);
        live.until_disk(OLD, "- edited\n");
        until("the edit is saved", || {
            live.host.driver.shared.with_state_for_tests(|state| {
                matches!(state.progress.host.logical(&key), Logical::Settled(page) if page.clean())
            })
        });
        live.faults(Phase::DraftSync, 10_000);
        let renamed = scope.spawn(|| rename(&live.host, &live.store));
        until("the rename is admitted", worker);
        pause.0.lock().unwrap().1 = true;
        pause.1.notify_all();
        let outcome = refused.recv_timeout(OPERATION_WAIT + Duration::from_secs(3));
        if outcome.is_err() {
            // Unblock a flush that still waits (before N1).
            clear_faults(&live);
        }
        assert_eq!(
            outcome,
            Ok(Some(ReserveRefusal::Busy)),
            "N1: a flush over an admitted rename's page is refused Busy within the bound"
        );
        assert!(started.elapsed() >= OPERATION_WAIT);
        live.host.driver.shared.with_state_for_tests(|state| {
            assert!(
                state.progress.host.retained.is_empty(),
                "nothing is reserved"
            );
        });
        clear_faults(&live);
        let _ = renamed.join().unwrap();
    });
    until("the operation ends", || !worker());
    live.host.stop();
}

/// REVIEW-3b-P2b-3 (Codex's mixed-set plant): a writer over clean Old and
/// dirty Other flushes Other, whose save waits on its path lock held by
/// another honest writer; meanwhile a rename over Old is admitted whose
/// drafts keep failing. The rename holds a page of the writer's set, so the
/// writer is refused Busy by its deadline though the page it waits to see
/// saved is not the rename's: the dependency is the whole set, not the
/// unsaved part.
#[test]
fn a_flush_over_a_mixed_set_is_told_busy_when_a_rename_holds_its_clean_page() {
    let live = Live::new(&[
        (OLD, "- old\n"),
        (REF, "- see [[Old]]\n"),
        (OTHER, "- other\n"),
    ]);
    live.submits_wait();
    let (other, page) = live.open(OTHER);
    let id = live
        .submit(&other, "- edited\n", page.version, None)
        .unwrap();
    live.answer(&other, id);
    // Inside the edit debounce: the save finds the path lock held.
    let lock = live.store.graph.page_lock(&live.root.join(&other));
    let held = lock.lock().unwrap();
    assert_eq!(live.disk(&other), "- other\n", "held before Other's save");
    let pause: crate::store::TestPause = Default::default();
    *live.host.driver.shared.flush_pause.lock().unwrap() = Some(Arc::clone(&pause));
    let worker = || {
        live.host
            .driver
            .shared
            .with_state_for_tests(|state| state.progress.host.worker.is_some())
    };
    std::thread::scope(|scope| {
        let unpark = Unpark {
            live: &live,
            pause: Arc::clone(&pause),
            _held: held,
        };
        let (sender, refused) = mpsc::channel();
        let started = Instant::now();
        let host = &live.host;
        scope.spawn(move || {
            let pages = || vec![PageId::from(OLD), PageId::from(OTHER)];
            let _ = sender.send(host.reserve(pages, Input::Flush).err());
        });
        until("the writer waits in its flush", || {
            pause.0.lock().unwrap().0
        });
        live.faults(Phase::DraftSync, 10_000);
        let renamed = scope.spawn(|| rename(&live.host, &live.store));
        until("the rename is admitted", worker);
        pause.0.lock().unwrap().1 = true;
        pause.1.notify_all();
        let outcome = refused.recv_timeout(OPERATION_WAIT + Duration::from_secs(3));
        let retained = live
            .host
            .driver
            .shared
            .with_state_for_tests(|state| state.progress.host.retained.is_empty());
        drop(unpark);
        assert_eq!(
            outcome,
            Ok(Some(ReserveRefusal::Busy)),
            "a flush waiting on an unrelated page is refused Busy by its deadline \
             while a rename holds a page of its set"
        );
        assert!(started.elapsed() >= OPERATION_WAIT);
        assert!(retained, "nothing is reserved");
        let _ = renamed.join().unwrap();
    });
    until("the operation ends", || !worker());
    live.host.stop();
}

/// REVIEW-3b-P2b-2 N1: a writer entering a poisoned host (one that
/// panicked under its lock) is refused Busy, never a panic.
#[test]
fn a_writer_entering_a_poisoned_host_is_told_busy() {
    let poisoned = graph();
    let shared = Arc::clone(&poisoned.host.driver.shared);
    let poisoner = std::thread::spawn(move || {
        let _state = shared.state_for_tests();
        panic!("N1 test: poison the host state");
    });
    assert!(poisoner.join().is_err());
    let reserved = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        poisoned
            .host
            .reserve(|| vec![PageId::from(OLD)], Input::Flush)
            .err()
    }));
    drop_failed(poisoned);
    assert_eq!(
        reserved.ok(),
        Some(Some(ReserveRefusal::Busy)),
        "a poisoned host is a refused writer, not a panic"
    );
}

/// REVIEW-3b-P2b-2 N1: a writer entering a host whose driver ended (nothing
/// will answer it) is refused Busy with nothing reserved.
#[test]
fn a_writer_entering_an_ended_driver_is_told_busy() {
    let mut ended = graph();
    ended.host.driver.join();
    assert!(ended.host.driver.shared.ended());
    let reserved = ended
        .host
        .reserve(|| vec![PageId::from(OLD)], Input::Flush)
        .err();
    assert_eq!(
        reserved,
        Some(ReserveRefusal::Busy),
        "an ended driver admits no writer"
    );
    ended.host.driver.shared.with_state_for_tests(|state| {
        assert!(
            state.progress.host.retained.is_empty(),
            "nothing is reserved"
        );
    });
}

/// REVIEW-3b-P2b-2 N1: a host that goes down between the writer's fence
/// and its input check refuses Busy, and the fenced pages are released.
#[test]
fn a_host_down_after_the_fence_refuses_busy_and_releases_the_pages() {
    let live = graph();
    let discovered = std::cell::Cell::new(0);
    let reserved = live
        .host
        .reserve(
            || {
                discovered.set(discovered.get() + 1);
                if discovered.get() == 2 {
                    live.host
                        .driver
                        .shared
                        .with_state_for_tests(|state| state.progress.host.stop());
                }
                vec![PageId::from(OLD)]
            },
            Input::Flush,
        )
        .err();
    assert_eq!(discovered.get(), 2, "the fence ran, then the rediscovery");
    assert_eq!(
        reserved,
        Some(ReserveRefusal::Busy),
        "a down host admits no writer"
    );
    live.host.driver.shared.with_state_for_tests(|state| {
        assert!(
            state.progress.host.retained.is_empty(),
            "the fenced pages are released"
        );
    });
    live.host.stop();
}

/// `PageHost`'s production methods, by name, as token text: every `impl
/// PageHost` in the binding's production files.
fn page_host_methods() -> BTreeMap<String, String> {
    use quote::ToTokens;
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/page_host");
    let mut methods = BTreeMap::new();
    for entry in fs::read_dir(&dir).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        if !name.starts_with("binding") || name.contains("test") || !name.ends_with(".rs") {
            continue;
        }
        let source = fs::read_to_string(dir.join(&name)).unwrap();
        for item in syn::parse_file(&source)
            .expect("Rust source must parse")
            .items
        {
            let syn::Item::Impl(block) = item else {
                continue;
            };
            if block.self_ty.to_token_stream().to_string() != "PageHost" {
                continue;
            }
            for item in block.items {
                if let syn::ImplItem::Fn(function) = item {
                    let body = function.block.to_token_stream().to_string();
                    methods.insert(function.sig.ident.to_string(), body);
                }
            }
        }
    }
    methods
}

/// The forms the reserve path must not use, and why. No caller thread can
/// lock the host state (it is private to `driver.rs`); what is left to
/// police is how a writer treats an unanswered request and where it waits.
/// A `read` or `call` is not a wait on host progress: the driver answers it
/// between steps and never waits for a lock another thread holds (the
/// store writer included: a reservation's writer may hold it), so it costs
/// at most one step, as the state lock it replaced did. Pinned by
/// `binding_tests.rs`
/// `the_driver_answers_while_another_thread_holds_the_store_writer`.
const UNCHECKED: &[(&str, &str)] = &[
    (
        "unwrap ()",
        "a request's answer unwrapped: a driver that ended panics the writer",
    ),
    (
        "expect (",
        "a request's answer expected: a driver that ended panics the writer",
    ),
    (". locked_wait (", "a path-lock wait outside `bounded`"),
    (". locked_step (", "a path-lock wait outside `bounded`"),
    (
        ". wait (",
        "a host wait outside `bounded`: no writer deadline, no down check",
    ),
];

/// The one function on the reserve path that waits on the host.
const SANCTIONED_WAIT: &str = "bounded";

/// Every function `reserve` reaches through `self.` calls, and each
/// unchecked form one of them uses.
fn reserve_path_violations(methods: &BTreeMap<String, String>) -> (BTreeSet<String>, Vec<String>) {
    let mut path = BTreeSet::new();
    let mut todo = vec!["reserve".to_string()];
    while let Some(name) = todo.pop() {
        if !path.insert(name.clone()) {
            continue;
        }
        let body = &methods[&name];
        todo.extend(
            methods
                .keys()
                .filter(|callee| body.contains(&format!("self . {callee} (")))
                .cloned(),
        );
    }
    let mut violations = Vec::new();
    for name in &path {
        for (form, why) in UNCHECKED {
            let sanctioned = name == SANCTIONED_WAIT && *form == ". wait (";
            if !sanctioned && methods[name].contains(form) {
                violations.push(format!("{name} uses `{form}`: {why}"));
            }
        }
    }
    (path, violations)
}

/// REVIEW-3b-P2b-2 N1, shape: every wait on a retained writer's
/// reservation path goes through `bounded`, which carries the writer's
/// deadline, and no unanswered request panics the writer. Exemplar:
/// `binding_retained.rs` `PageHost::flush`.
/// Scope: it catches honest mistakes written in the house style, not
/// adversarial spellings. Known limit (REVIEW-3b-P2b-3 N1): it follows only
/// `self.method(..)` calls among the binding files' `PageHost` methods, so a
/// helper reached another way (`Self::f(self)`, a `Shared` or `Graph`
/// helper) is not checked. Retired with `reserve` itself in the next storage
/// step, when the page host becomes the only page writer.
#[test]
fn the_reserve_path_has_no_unchecked_lock_or_bare_wait() {
    let methods = page_host_methods();
    let (path, violations) = reserve_path_violations(&methods);
    for reached in [
        "bounded",
        "flush",
        "register_all",
        "try_identify",
        "try_register",
    ] {
        assert!(
            path.contains(reached),
            "the reserve path reaches {reached}: {path:?}"
        );
    }
    assert!(
        violations.is_empty(),
        "M6/N1: the reserve path waits only through `bounded` (the writer's deadline; \
         ended/down refuse Busy); exemplar binding_retained.rs flush:\n{}",
        violations.join("\n")
    );
    // The guard's own necessity: each unchecked form is caught.
    let mutants = [
        (
            "flush",
            "{ let _ = self . driver . shared . read (| s | s . book . session) . unwrap () ; }",
        ),
        (
            "flush",
            "{ self . driver . shared . wait (Deadline :: Never , step) }",
        ),
        (
            "bounded",
            "{ self . driver . shared . locked_wait (deadline , step) }",
        ),
        (
            "reserve",
            "{ self . try_identify (page) . expect (\"ended\") }",
        ),
        (
            "register_all",
            "{ self . try_register (key , spelling) . unwrap () }",
        ),
    ];
    for (function, added) in mutants {
        let mut planted = methods.clone();
        planted.get_mut(function).unwrap().push_str(added);
        assert!(
            !reserve_path_violations(&planted).1.is_empty(),
            "the guard misses {added} in {function}"
        );
    }
}
