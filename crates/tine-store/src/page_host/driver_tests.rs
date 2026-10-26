//! STEP3 §1/§14: the driver thread, its lock protocol and its wakeups.
use super::io::{HostIo, Phase};
use super::model_fs::{Fault, ModelFs};
use super::tests::{draft, edit, host, open, risk, saved, text};
use super::*;
use crate::page_host::binding::{Delivery, Indexing, Publication};
use crate::page_host::driver::{Deadline, Driver, Owner, Sink, State, Wait};
use crate::page_host::progress::{Clock, Progress};
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Clone, Default)]
struct SharedClock(Arc<AtomicU64>);
impl Clock for SharedClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

enum Delivered {
    Mail(PageKey, Mail),
}

struct Channel(mpsc::Sender<Delivered>);
/// Indexes every publication at once.
impl Sink for Channel {
    fn deliver(
        &mut self,
        delivery: Delivery,
        _: &dyn Fn(&str) -> Owner,
        _: bool,
    ) -> Result<Vec<(Publication, Indexing)>, Delivery> {
        for (page, mail, _) in delivery.mail {
            let _ = self.0.send(Delivered::Mail(page, mail));
        }
        let indexed = delivery.publications.into_iter();
        Ok(indexed.map(|p| (p, Indexing::Indexed)).collect())
    }
}

type Test = Driver<ModelFs, SharedClock>;

fn spawn(host: Host<ModelFs>) -> (Test, SharedClock, mpsc::Receiver<Delivered>) {
    let clock = SharedClock::default();
    let (send, receive) = mpsc::channel();
    (
        Driver::spawn(host, clock.clone(), Channel(send)),
        clock,
        receive,
    )
}

fn admit(d: &Test, page: &str, kind: RequestKind) -> u64 {
    d.shared.with_state_for_tests(|s| {
        let host = &mut s.progress.host;
        let id = host.last_admitted + 1;
        let request = Request {
            id,
            generation: host.generation,
            page: page.into(),
            kind,
        };
        assert_eq!(host.admit(request), Disposition::Applied);
        id
    })
}

/// The answer mail for `id`, waiting at most five seconds.
fn answered(receive: &mpsc::Receiver<Delivered>, id: u64) -> (PageKey, Mail) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match receive.recv_timeout(left).expect("driver answered in time") {
            Delivered::Mail(page, mail) if mail.answer.as_ref().is_some_and(|a| a.id == id) => {
                return (page, mail)
            }
            _ => {}
        }
    }
}

fn polls(d: &Test) -> u64 {
    d.shared.state_for_tests().polls
}

/// Wait until the driver has gone to sleep (no poll for 50 ms).
fn settle(d: &Test) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last = polls(d);
    loop {
        std::thread::sleep(Duration::from_millis(50));
        let now = polls(d);
        if now == last {
            return;
        }
        assert!(Instant::now() < deadline, "the driver never went idle");
        last = now;
    }
}

#[test]
fn plan_lock_revalidate_never_holds_state_while_waiting_for_a_path_lock() {
    let h = host();
    let lock_a = h.locks["a.md"].clone();
    let (mut d, _, receive) = spawn(h);
    // A transaction holds a.md's path lock (writer → paths).
    let transaction = lock_a.lock().unwrap();
    let first = admit(&d, "a.md", RequestKind::Open);
    // The driver dequeues, plans the Open and blocks on a.md's path lock.
    let blocked = Instant::now() + Duration::from_secs(5);
    while d.shared.state_for_tests().progress.host.applying.is_none() {
        assert!(
            Instant::now() < blocked,
            "the driver never planned the Open"
        );
        std::thread::yield_now();
    }
    std::thread::sleep(Duration::from_millis(50));
    // Admission still takes the state mutex at once: the blocked driver
    // holds no state lock while it waits for the path lock.
    let started = Instant::now();
    let second = admit(&d, "b.md", RequestKind::Open);
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(d.shared.state_for_tests().progress.host.pages.is_empty());
    drop(transaction);
    assert_eq!(answered(&receive, first).0, "a.md");
    assert_eq!(answered(&receive, second).0, "b.md");
    d.join();
}
#[test]
fn every_admission_wakes_a_sleeping_driver() {
    let (mut d, _, receive) = spawn(host());
    for round in 0..200 {
        let page = if round % 2 == 0 { "a.md" } else { "b.md" };
        let kind = if round % 4 < 2 {
            RequestKind::Open
        } else {
            RequestKind::Close
        };
        let id = admit(&d, page, kind.clone());
        if kind == RequestKind::Open {
            answered(&receive, id);
        } else {
            // Close has no answer: wait until it is applied.
            let deadline = Instant::now() + Duration::from_secs(5);
            while d.shared.state_for_tests().progress.host.last_applied < id {
                assert!(Instant::now() < deadline, "lost wakeup at round {round}");
                std::thread::yield_now();
            }
        }
    }
    d.join();
}
struct ManualClock(Cell<u64>);
impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.0.get()
    }
}

#[test]
fn due_saves_are_served_round_robin_from_the_last_started_page() {
    let mut p = Progress::new(host(), ManualClock(Cell::new(0)));
    p.with_host(|h| {
        for key in ["a.md", "b.md", "c.md"] {
            open(h, key);
            edit(h, key, "mine");
        }
    });
    // Every save fails at its temp, so each page stays due after its try.
    p.host.fs.inject(Phase::PageTemp, [Fault::Before; 8]);
    let mut started = vec![];
    for now in [1_000, 1_100, 1_200, 1_400, 1_600, 1_800] {
        p.clock.0.set(now);
        for _ in 0..8 {
            let job = p.host.job.as_ref().map(|j| j.page.clone());
            if p.poll() == Disposition::Disabled {
                break;
            }
            if let (None, Some(job)) = (job, p.host.job.as_ref()) {
                started.push(job.page.clone());
            }
        }
    }
    // Key order from the start would retry a.md as soon as it is due again,
    // ahead of b.md and c.md; rotation from the last started page serves
    // each due page in turn.
    assert_eq!(&started[..4], ["a.md", "b.md", "c.md", "a.md"]);
}

#[test]
fn production_draft_sync_moves_only_the_synced_entry() {
    let dir = tempfile::tempdir().unwrap();
    let (graph, app) = (dir.path().join("graph"), dir.path().join("app"));
    std::fs::create_dir_all(&graph).unwrap();
    let trash = graph.join("logseq/.tine-trash/pages");
    let mut fs = production::ProductionIo::attach(&graph, &app, "g", &trash);
    for n in 0..20 {
        let record = drafts::Record {
            page: format!("p{n}.md"),
            wseq: n + 1,
            version: n + 1,
            base: Base::Known(None),
            bytes: text("x"),
        };
        let name = drafts::page_name(&record.page);
        fs.draft_temp(&name, &drafts::encode(&[record])).unwrap();
        fs.draft_rename(&name).unwrap();
    }
    fs.draft_sync().unwrap();
    assert_eq!(fs.draft_changes().len(), 20);
    let record = drafts::Record {
        page: "new.md".into(),
        wseq: 99,
        version: 99,
        base: Base::Known(None),
        bytes: text("y"),
    };
    let name = drafts::page_name("new.md");
    fs.draft_temp(&name, &drafts::encode(std::slice::from_ref(&record)))
        .unwrap();
    fs.draft_rename(&name).unwrap();
    fs.draft_sync().unwrap();
    let changes = fs.draft_changes();
    assert_eq!(changes.len(), 1, "one entry moved into the durable census");
    assert_eq!(changes[0].0, name);
    assert_eq!(fs.draft_files(true).len(), 21);
    fs.draft_unlink(&name).unwrap();
    assert_eq!(fs.draft_files(true).len(), 21, "unsynced removal");
    fs.draft_sync().unwrap();
    assert_eq!(fs.draft_changes(), vec![(name, None)]);
    assert_eq!(fs.draft_files(true).len(), 20);
}

#[test]
fn the_draft_index_follows_the_durable_census_without_rescans() {
    let mut h = host();
    open(&mut h, "a.md");
    edit(&mut h, "a.md", "mine");
    risk(&mut h, "a.md");
    draft(&mut h, "a.md");
    // The index holds the one vehicle; logical_drafts asserts (under test)
    // that it equals the scan of the durable census.
    assert_eq!(h.drafts.len(), 1);
    assert_eq!(h.logical_drafts()["a.md"].bytes, text("mine"));
    saved(&mut h, "a.md");
    draft(&mut h, "a.md");
    assert!(h.drafts.is_empty());
    assert!(h.logical_drafts().is_empty());
}
/// A wait whose caller's deadline falls while the driver is asking it
/// (the admission's own last ask, made at that deadline). `last` runs as
/// the ask that spans the deadline; earlier asks re-ask just before it.
fn ask_across_the_deadline<R: Send + 'static>(
    last: impl FnOnce() -> Wait<R> + Send + 'static,
) -> Result<R, super::driver::Unanswered> {
    let (mut d, ..) = spawn(host());
    let deadline = Instant::now() + Duration::from_millis(200);
    let mut last = Some(last);
    let answer = d.shared.locked_wait(Deadline::At(deadline), move |_| {
        if Instant::now() + Duration::from_millis(100) < deadline {
            return Wait::Until(Some(deadline - Duration::from_millis(50)));
        }
        // Mid-ask at the caller's deadline.
        std::thread::sleep(deadline + Duration::from_millis(100) - Instant::now());
        match last.take() {
            Some(last) => last(),
            None => Wait::Until(None),
        }
    });
    d.join();
    answer
}

/// M6 (CI 38436208770): an ask that spans the caller's deadline and does
/// not admit drops the request, so it never takes effect: the caller hears
/// Withdrawn (Busy), not Late (Uncertain, "keeps trying to finish it").
#[test]
fn an_unadmitted_ask_across_the_deadline_is_withdrawn_not_late() {
    let answer = ask_across_the_deadline(|| Wait::<()>::Until(None));
    assert_eq!(answer, Err(super::driver::Unanswered::Withdrawn));
}

/// The same ask answering takes effect, so the caller gets that answer.
#[test]
fn an_answer_given_across_the_deadline_reaches_the_caller() {
    assert_eq!(ask_across_the_deadline(|| Wait::Ready(7)), Ok(7));
}

#[test]
fn a_lock_retry_that_passes_while_the_driver_plans_its_sleep_still_wakes_it() {
    let mut h = host();
    h.test.submits_wait = true;
    open(&mut h, "a.md");
    let lock_a = h.locks["a.md"].clone();
    let (d, clock, _receive) = spawn(h);
    let version = d.shared.state_for_tests().progress.host.pages["a.md"].version;
    let id = admit(
        &d,
        "a.md",
        RequestKind::Submit {
            bytes: text("mine"),
            version,
            resolve: None,
        },
    );
    let applied = Instant::now() + Duration::from_secs(5);
    while d.shared.state_for_tests().progress.host.last_applied < id {
        assert!(Instant::now() < applied, "the edit was never applied");
        std::thread::yield_now();
    }
    // The driver thread is preempted after each idle step for longer than
    // the first path-lock backoff (10 ms): the retry instant its step waited
    // for has passed by the time it plans its sleep.
    d.shared.plan_pause.store(30, Ordering::SeqCst);
    let other = lock_a.lock().unwrap();
    let started = Instant::now() + Duration::from_secs(5);
    while d.shared.state_for_tests().progress.host.job.is_none() {
        assert!(Instant::now() < started, "the save never started");
        clock.0.fetch_add(2_000, Ordering::SeqCst);
        d.shared.with_state_for_tests(|_| {});
        std::thread::sleep(Duration::from_millis(1));
    }
    // Nothing else wakes the driver from here: only its own lock retry.
    std::thread::sleep(Duration::from_millis(200));
    drop(other);
    let saved = Instant::now() + Duration::from_secs(3);
    loop {
        let state = d.shared.state_for_tests();
        let host = &state.progress.host;
        if host.job.is_none() && host.fs.files["graph/a.md"].as_ref() == b"mine" {
            break;
        }
        drop(state);
        assert!(
            Instant::now() < saved,
            "the lock retry was lost: the save never resumed"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_blocked_dirty_page_contributes_no_deadline_and_the_driver_sleeps() {
    for blocked_by in ["conflict"] {
        let mut h = host();
        open(&mut h, "a.md");
        edit(&mut h, "a.md", "mine");
        if blocked_by == "conflict" {
            h.fs.external("a.md", text("theirs"), true);
            assert_eq!(h.observe("a.md"), Disposition::Applied);
            assert!(h.pages["a.md"].conflict);
        }
        let (mut d, clock, _receive) = spawn(h);
        // Long past every save deadline.
        clock.0.store(60_000, Ordering::SeqCst);
        d.shared.with_state_for_tests(|_| {});
        std::thread::sleep(Duration::from_millis(100));
        let state = d.shared.state_for_tests();
        // A draft for the conflicted page is the only timed work left.
        let next = state.progress.next_deadline();
        let worker = state.progress.host.worker.is_some();
        drop(state);
        assert!(next.is_none() || worker, "{blocked_by}: {next:?}");
        let before = polls(&d);
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            polls(&d) - before <= 2,
            "{blocked_by}: the driver spun on an expired deadline"
        );
        d.join();
    }
}

#[test]
fn a_spelling_move_while_the_driver_waits_replans_on_the_new_lock() {
    let mut h = host();
    open(&mut h, "a.md");
    let lock_a = h.locks["a.md"].clone();
    let lock_b = Arc::new(Mutex::new(()));
    let (mut d, _, _receive) = spawn(h);
    settle(&d);
    // The spelling move's transaction holds both spellings' locks.
    let old = lock_a.lock().unwrap();
    let new = lock_b.lock().unwrap();
    let before = polls(&d);
    d.shared.with_state_for_tests(|s| {
        s.progress.host.fs.external("a.md", text("theirs"), true);
        s.observe.insert("a.md".into(), Default::default());
    });
    let blocked = Instant::now() + Duration::from_secs(5);
    while d
        .shared
        .state_for_tests()
        .progress
        .host
        .lock_request
        .is_some()
        || polls(&d) < before + 1
    {
        assert!(Instant::now() < blocked);
        std::thread::yield_now();
    }
    std::thread::sleep(Duration::from_millis(50));
    // Under its reservation the writer moves the entry and respells the key,
    // then releases the reservation and the old spelling's lock.
    d.shared.with_state_for_tests(|s| {
        s.progress.host.fs.spell("a.md", "A.md");
        s.progress.host.locks.insert("a.md".into(), lock_b.clone());
    });
    drop(old);
    std::thread::sleep(Duration::from_millis(150));
    let state = d.shared.state_for_tests();
    assert_eq!(
        state.progress.host.pages["a.md"].buf,
        text("A"),
        "the observation ran under the old spelling's lock"
    );
    assert_eq!(state.progress.host.fs.spelling("a.md"), "A.md");
    drop(state);
    drop(new);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let state = d.shared.state_for_tests();
        if state.observe.is_empty() {
            assert_eq!(state.progress.host.pages["a.md"].buf, text("theirs"));
            break;
        }
        drop(state);
        assert!(Instant::now() < deadline, "the new lock never ran the step");
        std::thread::sleep(Duration::from_millis(5));
    }
    d.join();
}
