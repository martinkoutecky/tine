//! GH #540: the recovery panel's kept originals at the binding boundary
//! (`PageHost::kept_originals`, `kept_text`, `restore_kept`), against a real
//! Store and the driver thread. Each test keeps an original the way a user
//! does (a rename cancelled after its source's removal published, then a
//! process death) and starts a live host on what that left.
use super::*;
use crate::page_host::io::{RestoreOutcome, RestoreRefusal};

const SOURCE: &str = "pages/a.md";
const RECEIVER: &str = "pages/c.md";
const REFERRER: &str = "pages/b.md";
/// Windows line ends: a restore puts the original back byte for byte.
const ORIGINAL: &str = "- old\r\n- two\r\n";

/// A live host on a graph whose rename of `pages/a.md` (text `original`)
/// to `pages/c.md` was cancelled with a's removal published; `extra` files
/// are on disk before the host starts.
fn kept(original: &str, extra: &[(&str, &str)]) -> Live {
    let dir = sandbox();
    let (graph, app) = (dir.path().join("graph"), dir.path().join("app"));
    for area in ["pages", "journals"] {
        fs::create_dir_all(graph.join(area)).unwrap();
    }
    fs::create_dir_all(&app).unwrap();
    fs::write(graph.join(SOURCE), original).unwrap();
    fs::write(graph.join(REFERRER), "- see [[a]]\n").unwrap();
    crate::page_host::production_tests::keep_an_original(
        &graph,
        &app,
        "test-graph",
        [SOURCE, RECEIVER, REFERRER],
    );
    assert!(!graph.join(SOURCE).exists(), "the original is not on disk");
    for (path, body) in extra {
        fs::write(graph.join(path), body).unwrap();
    }
    Live::start_in(dir, false, crate::WatchMode::Notify, DraftOwnership::Owned)
}

/// The one row the panel lists.
fn listed(live: &Live) -> io::KeptOriginal {
    let rows = live.host.kept_originals(live.host.session()).unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    rows.into_iter().next().unwrap()
}

fn restore(live: &Live, row: &io::KeptOriginal) -> RestoreOutcome {
    live.host.restore_kept(live.host.session(), &row.key, row.origin)
}

/// A refusal changes nothing: the slot stays empty (or keeps its file)
/// and the original stays listed.
fn unchanged(live: &Live, before: &io::KeptOriginal, slot: Option<&str>) {
    let on_disk = fs::read_to_string(live.root.join(SOURCE)).ok();
    assert_eq!(on_disk.as_deref(), slot, "the slot is untouched");
    let after = listed(live);
    assert_eq!((after.key, after.origin), (before.key.clone(), before.origin));
    assert_eq!(live.host.draft_status().kept, 1);
}

fn until(what: &str, test: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !test() {
        assert!(Instant::now() < deadline, "never: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Acceptance 1 and 15: the list names the kept original, Restore puts its
/// bytes back exactly (CRLF kept), the index has it when Restored is
/// reported, the pair is released (no row, no count), and the write is
/// recorded as a page creation (OG-RULES Rule 8).
#[test]
fn restore_puts_the_kept_original_back_byte_for_byte() {
    let live = kept(ORIGINAL, &[]);
    let row = listed(&live);
    assert_eq!(
        (row.key.as_str(), row.path.as_str(), row.name.as_deref(), row.len),
        (SOURCE, SOURCE, Some("a"), ORIGINAL.len())
    );
    assert_eq!(live.host.draft_status().kept, 1, "the launch count");
    let text = live.host.kept_text(live.host.session(), &row.key, row.origin);
    assert_eq!(text.as_deref(), Some(ORIGINAL), "Copy original");
    assert_eq!(restore(&live, &row), RestoreOutcome::Restored);
    assert_eq!(fs::read(live.root.join(SOURCE)).unwrap(), ORIGINAL.as_bytes());
    assert_eq!(
        live.indexed(SOURCE),
        Some(content_rev(ORIGINAL)),
        "Restored only once the index has it"
    );
    assert!(live.host.kept_originals(live.host.session()).unwrap().is_empty());
    assert_eq!(live.host.draft_status().kept, 0);
    until("the write's kind is published", || {
        live.kinds.lock().unwrap().iter().any(|(key, _)| key == SOURCE)
    });
    let kinds = live.kinds.lock().unwrap().clone();
    let taken: Vec<_> = kinds.iter().filter(|(key, _)| key == SOURCE).collect();
    assert_eq!(taken, [&(SOURCE.to_string(), vec![EditKind::CreatePage])]);
    // A second Restore of the same row finds nothing kept.
    assert_eq!(restore(&live, &row), RestoreOutcome::Refused(RestoreRefusal::NotKept));
    live.host.stop();
}

/// Acceptance: a window holding the page refuses (it shows the page, and
/// its own save path owns what it writes), and so does a window holding the
/// page the original's text names (`title:: Named`, slot `pages/Named.md`).
#[test]
fn restore_refuses_while_a_window_holds_the_page_or_its_name() {
    let live = kept(ORIGINAL, &[]);
    let row = listed(&live);
    let (key, _) = live.open(SOURCE);
    let held = RestoreRefusal::Held { name: "a".into() };
    assert_eq!(restore(&live, &row), RestoreOutcome::Refused(held));
    unchanged(&live, &row, None);
    live.close(&key);
    live.host.stop();

    let live = kept("title:: Named\n\n- old\n", &[]);
    let row = listed(&live);
    assert_eq!(row.name.as_deref(), Some("Named"));
    live.open("pages/Named.md");
    let held = RestoreRefusal::Held { name: "Named".into() };
    assert_eq!(restore(&live, &row), RestoreOutcome::Refused(held));
    unchanged(&live, &row, None);
    assert!(!live.root.join("pages/Named.md").exists());
    live.host.stop();
}

/// Amendment 2: admission re-checks under the page's path lock. Restore
/// waits for a lock another writer holds; meanwhile a window opens the page;
/// once the lock is free the step sees the window and refuses.
#[test]
fn restore_rechecks_a_window_after_waiting_for_the_path_lock() {
    let live = kept(ORIGINAL, &[]);
    let row = listed(&live);
    let lock = live.store.graph.page_lock(&live.root.join(SOURCE));
    let outcome = std::thread::scope(|scope| {
        let held = lock.lock().unwrap();
        let (host, session, row) = (&live.host, live.host.session(), &row);
        let caller = scope.spawn(move || host.restore_kept(session, &row.key, row.origin));
        // Restore is parked on the lock (its first step ran unheld).
        std::thread::sleep(Duration::from_millis(300));
        let generation = live.host.session();
        live.host
            .open(generation, live.id(), &PageId::from(SOURCE), "a")
            .unwrap();
        drop(held);
        caller.join().unwrap()
    });
    let held = RestoreRefusal::Held { name: "a".into() };
    assert_eq!(outcome, RestoreOutcome::Refused(held));
    unchanged(&live, &row, None);
    live.host.stop();
}

/// Acceptance: a file at the slot, another file that is this page, and a
/// readable file claiming the page's name each refuse, changing nothing.
#[test]
fn restore_refuses_an_occupied_slot_a_twin_and_a_name_claimant() {
    let live = kept(ORIGINAL, &[(SOURCE, "- someone else\n")]);
    let row = listed(&live);
    assert_eq!(restore(&live, &row), RestoreOutcome::Refused(RestoreRefusal::Occupied));
    unchanged(&live, &row, Some("- someone else\n"));
    live.host.stop();

    let live = kept(ORIGINAL, &[("pages/a.org", "* a\n")]);
    let row = listed(&live);
    let twin = RestoreRefusal::Twin { file: "pages/a.org".into() };
    assert_eq!(restore(&live, &row), RestoreOutcome::Refused(twin));
    unchanged(&live, &row, None);
    live.host.stop();

    let live = kept(ORIGINAL, &[("pages/x.md", "title:: a\n\n- x\n")]);
    let row = listed(&live);
    let twin = RestoreRefusal::Twin { file: "pages/x.md".into() };
    assert_eq!(restore(&live, &row), RestoreOutcome::Refused(twin));
    unchanged(&live, &row, None);
    live.host.stop();
}

/// Amendment 5 and acceptance: a row belongs to the session that listed
/// it, a stale row (another origin) or a key that is not kept finds nothing,
/// and a reloaded window's old rows refuse; none changes anything.
#[test]
fn restore_refuses_stale_rows_and_other_sessions() {
    let live = kept(ORIGINAL, &[]);
    let row = listed(&live);
    let session = live.host.session();
    let not_kept = RestoreOutcome::Refused(RestoreRefusal::NotKept);
    assert_eq!(live.host.restore_kept(session, &row.key, row.origin + 1), not_kept);
    assert_eq!(live.host.restore_kept(session, RECEIVER, row.origin), not_kept);
    assert_eq!(live.host.kept_originals(session + 1), None);
    assert_eq!(live.host.kept_text(session + 1, &row.key, row.origin), None);
    let other = live.host.restore_kept(session + 1, &row.key, row.origin);
    assert_eq!(other, RestoreOutcome::Refused(RestoreRefusal::Unavailable));
    unchanged(&live, &row, None);
    // A window reload: the old window's rows answer nothing; the new
    // session lists the same original again.
    live.host.window_reloaded();
    assert_ne!(live.host.session(), session);
    let stale = live.host.restore_kept(session, &row.key, row.origin);
    assert_eq!(stale, RestoreOutcome::Refused(RestoreRefusal::Unavailable));
    unchanged(&live, &row, None);
    live.host.stop();
}

/// Amendment 1: Restored is reported only on the host's own publication.
/// The save cannot finish; a window opens the page and takes Use Disk (the
/// slot is empty) before any witness. The caller hears Superseded, not
/// Restored, and nothing is written once the faults clear.
#[test]
fn a_use_disk_before_the_witness_supersedes_the_restore() {
    let live = kept(ORIGINAL, &[]);
    let row = listed(&live);
    live.faults(Phase::PageTemp, 10_000);
    let outcome = std::thread::scope(|scope| {
        let (host, session, row) = (&live.host, live.host.session(), &row);
        let caller = scope.spawn(move || host.restore_kept(session, &row.key, row.origin));
        until("the restore is admitted", || {
            let read = live.host.driver.shared.read(|state| state.progress.host.order.slots());
            read.unwrap() == 1
        });
        let (key, _) = live.open(SOURCE);
        let id = live.id();
        live.host.discard(live.host.session(), id, &key).unwrap();
        live.answer(&key, id);
        caller.join().unwrap()
    });
    assert_eq!(outcome, RestoreOutcome::Superseded);
    live.host
        .driver
        .shared
        .with_state_for_tests(|state| state.progress.host.fs.faults.clear());
    std::thread::sleep(Duration::from_millis(500));
    assert!(!live.root.join(SOURCE).exists(), "Use Disk's empty slot stands");
    live.host.stop();
}

/// Amendment 4: a Restore whose save cannot finish is Unwritten, once.
/// `held` says whether a window holds the page by then (its editor reports
/// the failing save, so the panel adds no toast). When the fault clears,
/// the save the host kept finishes.
#[test]
fn an_unwritable_restore_is_unwritten_and_says_whether_a_window_holds_it() {
    for late_open in [false, true] {
        let live = kept(ORIGINAL, &[]);
        let row = listed(&live);
        live.faults(Phase::PageTemp, 10_000);
        let outcome = std::thread::scope(|scope| {
            let (host, session, row) = (&live.host, live.host.session(), &row);
        let caller = scope.spawn(move || host.restore_kept(session, &row.key, row.origin));
            if late_open {
                until("the restore is admitted", || {
                    let shared = &live.host.driver.shared;
                    shared.read(|state| state.progress.host.order.slots()).unwrap() == 1
                });
                live.open(SOURCE);
            }
            caller.join().unwrap()
        });
        assert_eq!(outcome, RestoreOutcome::Unwritten { held: late_open });
        assert!(!live.root.join(SOURCE).exists());
        live.host
            .driver
            .shared
            .with_state_for_tests(|state| state.progress.host.fs.faults.clear());
        live.until_disk(SOURCE, ORIGINAL);
        live.host.stop();
    }
}
