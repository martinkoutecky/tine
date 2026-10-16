//! GH #406 (master 05a4b0001c5b, 13fce7f25764): a page rename's disk bill is
//! linear in what it rewrites. Master listed every ancestor directory on each
//! referrer write, so the write phase cost referrers x pages. og's contract:
//! directory enumerations per rename do not grow with the referrer count or
//! the page count, whole-file reads do not grow with the page count, and each
//! referrer costs one write and a bounded number of guard reads (I-13, I-25).
use std::fs;
use std::sync::Mutex;
use std::time::Duration;
use tine_graph_features::pages;
use tine_store::cost_counters::{self, Counts};
use tine_store::Store;

/// Renames are the page host's (STEP3 §7): each runs through a host started
/// for it under its own app data, as the app's graph binding runs one.
fn hosted<T>(
    store: &std::sync::Arc<tine_store::Store>,
    run: impl FnOnce(&tine_store::PageHost) -> T,
) -> T {
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(store, app_data.path()).unwrap();
    let result = run(&host);
    // The measurement boundary (contract §5.4, REVIEW-3b-P2b-2 N3).
    assert!(
        host.quiesce_for_tests(Duration::from_secs(60)),
        "the watcher or the host never settled"
    );
    result
}

static CASE_LOCK: Mutex<()> = Mutex::new(());

/// `pages` unrelated pages spread over three folders, `referrers` pages that
/// link `[[Target]]`, and the target itself.
fn rename(pages_count: usize, referrers: usize) -> Counts {
    rename_timed(pages_count, referrers).0
}

/// [`rename`], plus the wall time of the rename call alone (no fixture
/// setup, no host start or quiesce).
fn rename_timed(pages_count: usize, referrers: usize) -> (Counts, Duration) {
    // Self-deleting: dropped after `store` (declared later), and on a panic too.
    let temp = tempfile::Builder::new()
        .prefix("rename-cost-")
        .tempdir()
        .unwrap();
    let root = temp.path().to_path_buf();
    for dir in ["pages/a", "pages/b", "pages/c", "journals"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    for index in 0..pages_count {
        let folder = ["a", "b", "c"][index % 3];
        fs::write(
            root.join(format!("pages/{folder}/Other{index:05}.md")),
            format!("- unrelated {index}\n"),
        )
        .unwrap();
    }
    for index in 0..referrers {
        fs::write(
            root.join(format!("pages/Ref{index:05}.md")),
            format!("- links [[Target]] {index}\n"),
        )
        .unwrap();
    }
    fs::write(root.join("pages/Target.md"), "- the target\n").unwrap();
    let store = std::sync::Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    cost_counters::reset();
    let elapsed = hosted(&store, |host| {
        let start = std::time::Instant::now();
        pages::rename_page_expected(&store, host, "Target", "Renamed", None).unwrap();
        start.elapsed()
    });
    let counts = cost_counters::snapshot();
    assert!(root.join("pages/Renamed.md").exists());
    assert_eq!(
        fs::read_to_string(root.join("pages/Ref00000.md")).unwrap(),
        "- links [[Renamed]] 0\n"
    );
    store.close();
    (counts, elapsed)
}

#[test]
fn rename_cost_is_linear_in_referrers_and_flat_in_graph_size() {
    let _integration_guard = crate::integration_lock();
    let _case = CASE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let few = rename(60, 2);
    let many = rename(60, 30);
    let large = rename(600, 2);
    eprintln!("GH #406 rename cost: 60p/2r={few:?}\n60p/30r={many:?}\n600p/2r={large:?}");
    assert_eq!(
        many.readdir, few.readdir,
        "GH #406/I-13: directory enumerations per rename must not grow with referrers \
         (master listed pages/ once per referrer write); exemplar transaction.rs Transaction::apply"
    );
    assert_eq!(
        large.readdir, few.readdir,
        "GH #406/I-13: directory enumerations per rename must not grow with the page count"
    );
    assert_eq!(
        large.full_reads, few.full_reads,
        "I-13: a rename must not re-read unrelated pages"
    );
    // Re-measured on the host's instrumented I/O (Q-P2b-2): the page's
    // temporary file only. Its draft record stays in the operation's one
    // draft file, which retires whole after the last publication
    // (RENAME-COST L1); exploding it into a draft file per page doubled the
    // writes and cost three more syncs per page.
    assert_eq!(
        many.files_written - few.files_written,
        28,
        "I-25 (RENAME-COST): each extra referrer costs exactly one file write, its page; \
         its draft record travels in the operation's draft file (exemplar \
         page_host/draft_worker.rs retire_whole): {few:?} vs {many:?}"
    );
    assert!(
        many.fsyncs - few.fsyncs <= 2 * 28,
        "I-25 (RENAME-COST): each extra referrer costs at most two syncs, its page file \
         and the graph directory; no per-page draft copy or draft removal is synced \
         (exemplar page_host/draft_worker.rs retire_whole): {few:?} vs {many:?}"
    );
    // Three guarded reads per rewritten referrer, each a base-revision or
    // publication check on the audited save path: preflight stage, the check
    // just before the rename, and the post-commit publication read that tells
    // own bytes from an external editor's (GH #623 QF3b removed a stage-2
    // verify that repeated the pre-rename check). None scales with the graph.
    assert!(
        many.full_reads - few.full_reads <= 3 * 28,
        "I-25: each extra referrer costs at most three whole-file reads (stage, \
         pre-rename check, publication)"
    );
    for counts in [few, many, large] {
        assert!(
            counts.observation_reads <= counts.files_written + 4,
            "contract §5.4 (O5): once quiesced, the host's re-reads of held pages the watcher \
             forwarded (the echoes of the operation's own writes) are at most the files it \
             wrote plus a small constant, billed apart from the operation's own reads \
             (exemplar page_host/driver.rs step_unblocked): {counts:?}"
        );
    }
}

/// REVIEW-3b-P2b R5: a rename's publication runs on the page host, so the
/// measured subject is the host's publication bookkeeping
/// (`host_publication_probes`): reply slots and orders examined per page
/// publication, the caller's per-wake witness check, and index
/// publications. The transaction path's record probes this test counted
/// before (GH #623) are no longer on a rename's path.
#[test]
fn rename_publication_work_is_linear() {
    let _integration_guard = crate::integration_lock();
    let _case = CASE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    for referrers in [8, 64] {
        let counts = rename(80, referrers);
        assert!(
            counts.host_publication_probes <= 12 * (referrers as u64 + 2),
            "GH #623/I-25/R5: publishing a rename's pages examines O(pages written), not \
             O(referrers²): the caller waiting for its witnesses reads the host's stuck \
             pages per wake, never every answered page (exemplar \
             page_host/binding_retained.rs settle): {counts:?}"
        );
    }
}

#[test]
fn rename_publication_parses_only_changed_documents() {
    let _integration_guard = crate::integration_lock();
    let _case = CASE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    for referrers in [8, 64] {
        let counts = rename(80, referrers);
        assert!(counts.parses <= referrers as u64 + 2,
            "GH #623/I-25: own reference rewrites must parse the new document once, without serializing/parsing the old one; exemplar model/transaction_publish.rs: {counts:?}");
    }
}

#[test]
fn rename_unit_cost_is_the_same_full_payload_on_one_and_sixty_blocks() {
    let _integration_guard = crate::integration_lock();
    let _case = CASE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    for (blocks, bytes) in [(1, 300), (60, 4137)] {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("pages")).unwrap();
        fs::write(root.path().join("pages/Target.md"), "- target\n").unwrap();
        fs::write(
            root.path().join("pages/Ref.md"),
            "- links [[Target]] 0\n".repeat(blocks),
        )
        .unwrap();
        let store = std::sync::Arc::new(Store::open(root.path(), Default::default()).unwrap().0);
        store.whole_graph().unwrap();
        cost_counters::reset();
        hosted(&store, |host| {
            pages::rename_page_expected(&store, host, "Target", "Renamed", None)
        })
        .unwrap();
        let cost = cost_counters::snapshot();
        // Measured on the host's instrumented I/O (Q-P2b-2), every write
        // counted where it is made: the operation draft, the referrer and the
        // destination page, and the source's custody marker. The referrer's
        // bytes travel in the operation draft as base and payload; its draft
        // record retires with that file, uncopied (RENAME-COST L1).
        assert_eq!(cost.files_written, 4);
        assert_eq!(cost.bytes_written, bytes);
        // RO3 (a) (Martin 2026-10-11): 17 -> 16, the source's trash payload
        // is no longer file-synced.
        assert_eq!(
            cost.fsyncs, 16,
            "file, directory and draft syncs of one rename: {cost:?}"
        );
        assert_eq!(
            fs::read_to_string(root.path().join("pages/Ref.md")).unwrap(),
            "- links [[Renamed]] 0\n".repeat(blocks)
        );
        // The documented unit-cost values are pinned to this actual write path.
        let contract = include_str!("../../../docs/storage-contract.md");
        assert!(contract.contains("300/4,137 bytes"));
        store.close();
    }
}

/// One rename of `referrers` referrers among `pages_count` unrelated pages on
/// a host already started, measured from the call through the host's cleanup
/// (every draft retired, every page released): the counters are reset after
/// host startup, and the snapshot is taken once the host is quiescent. Checks
/// every referrer's bytes and the source's deletion. Also returns how long the
/// rename call itself took and what its caller heard: past `OPERATION_WAIT`
/// the caller hears `TimedOut` while the host finishes the rename, so the
/// work and bytes are checked either way (REVIEW-progress-plan R5).
fn rename_through_cleanup(
    pages_count: usize,
    referrers: usize,
) -> (Counts, std::time::Duration, std::io::Result<()>) {
    let temp = tempfile::Builder::new()
        .prefix("rename-work-")
        .tempdir()
        .unwrap();
    let root = temp.path().join("graph");
    for dir in ["pages/a", "pages/b", "pages/c"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    for index in 0..pages_count {
        let folder = ["a", "b", "c"][index % 3];
        fs::write(
            root.join(format!("pages/{folder}/Other{index:05}.md")),
            format!("- unrelated {index}\n"),
        )
        .unwrap();
    }
    for index in 0..referrers {
        fs::write(
            root.join(format!("pages/Ref{index:05}.md")),
            format!("- links [[Target]] {index}\n"),
        )
        .unwrap();
    }
    fs::write(root.join("pages/Target.md"), "- the target\n").unwrap();
    let store = std::sync::Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    let host = tine_store::PageHost::start_for_tests(&store, &temp.path().join("app")).unwrap();
    let started = std::time::Instant::now();
    while !host.quiescent_for_tests() {
        assert!(
            started.elapsed().as_secs() < 30,
            "the host never settled after launch"
        );
        std::thread::yield_now();
    }
    cost_counters::reset();
    let called = std::time::Instant::now();
    let heard = pages::rename_page_expected(&store, &host, "Target", "Renamed", None);
    let call = called.elapsed();
    let renamed = std::time::Instant::now();
    while !host.quiescent_for_tests() {
        assert!(
            renamed.elapsed().as_secs() < 60,
            "the host never finished the rename's cleanup"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let counts = cost_counters::snapshot();
    // REVIEW-rename-cost: a quiescent host retired the operation vehicle,
    // so fewer writes cannot come from a draft that was never removed.
    let drafts = temp.path().join("app/drafts-v2");
    assert!(drafts.is_dir(), "the host keeps its drafts at {drafts:?}");
    let vehicles: Vec<_> = walk_files(&drafts);
    assert!(
        vehicles.is_empty(),
        "drafts left after the rename: {vehicles:?}"
    );
    assert!(
        !root.join("pages/Target.md").exists(),
        "the source is deleted"
    );
    assert_eq!(
        fs::read_to_string(root.join("pages/Renamed.md")).unwrap(),
        "- the target\n"
    );
    for index in 0..referrers {
        assert_eq!(
            fs::read_to_string(root.join(format!("pages/Ref{index:05}.md"))).unwrap(),
            format!("- links [[Renamed]] {index}\n"),
            "referrer {index} is rewritten"
        );
    }
    drop(host);
    store.close();
    (counts, call, heard)
}

/// M4 (STEP3-DESIGN "Manager decisions on the P2b-fix report", REVIEW-progress-
/// plan B3/R5): a host step does work proportional to what changed, so a
/// rename's host work through its cleanup is linear in its referrers, per
/// primitive, at three sizes, against fixed per-referrer constants measured
/// on the reviewed implementation. Before M4 every reconcile walked every held
/// page and rebuilt the whole logical draft map, the operation worker copied
/// and relocked its whole page set on every phase, and its tidy scanned every
/// vehicle per page: O(referrers²).
#[test]
fn rename_host_work_is_linear() {
    let _integration_guard = crate::integration_lock();
    let _case = CASE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    // Every size is measured before any is judged, so a failure shows the growth.
    let measured = [100u64, 200, 400].map(|referrers| {
        let (c, call, heard) = rename_through_cleanup(80, referrers as usize);
        eprintln!("M4 rename host work, {referrers} referrers ({call:?}, {heard:?}): {c:?}");
        // The caller's bound is the availability gate's subject, not this one's.
        if let Err(error) = heard {
            assert_eq!(error.kind(), std::io::ErrorKind::TimedOut, "{error}");
        }
        (referrers, c)
    });
    for (referrers, c) in measured {
        let primitives = [
            (
                "host_reconcile_visits",
                c.host_reconcile_visits,
                RECONCILE_PER_REFERRER,
            ),
            (
                "host_draft_record_visits",
                c.host_draft_record_visits,
                DRAFT_RECORDS_PER_REFERRER,
            ),
            (
                "host_candidate_visits",
                c.host_candidate_visits,
                CANDIDATES_PER_REFERRER,
            ),
            (
                "host_worker_key_visits",
                c.host_worker_key_visits,
                WORKER_KEYS_PER_REFERRER,
            ),
            ("host_book_visits", c.host_book_visits, BOOK_PER_REFERRER),
        ];
        for (name, count, per_referrer) in primitives {
            assert!(
                count > 0,
                "{name} is instrumented on a rename's path: {c:?}"
            );
            assert!(
                count <= per_referrer * (referrers + 2) + HOST_WORK_FLOOR,
                "M4/I-14/D-10: a rename's {name} must be linear in its pages, at most \
                 {per_referrer} per page plus {HOST_WORK_FLOOR}; got {count} at {referrers} \
                 referrers. A host step visits what changed since the last step, never every \
                 held page or draft (exemplar page_host/progress.rs reconcile): {c:?}"
            );
        }
    }
}

/// M4 (REVIEW-progress-plan R5): the availability half of the proof, kept
/// apart from the structural counts above. A rename of 250 and of 1000
/// referrers is answered success, and off Windows inside 10 s (the caller's
/// `OPERATION_WAIT`). The caller's bound is on a stall, not the operation
/// (storage contract, page host binding): on Windows each save's
/// write-through rename makes a thousand referrers outlast 10 s while they
/// keep publishing, an accepted cost (Martin 2026-10-12, journal "Windows
/// rename cost: keep write-through"), so there only success is asserted and
/// the timing printed. Release build only, so a loaded debug suite cannot
/// decide it; the executable gate is `scripts/rename-thousand-referrers.sh`
/// (Linux) and the `scope=windows` CI job (G1), which print the timings.
fn release_rename_gate(referrers: usize) {
    let _case = CASE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    assert!(
        !cfg!(debug_assertions),
        "a release gate: run scripts/rename-thousand-referrers.sh"
    );
    let started = std::time::Instant::now();
    let (c, call, heard) = rename_through_cleanup(80, referrers);
    eprintln!(
        "M4 release gate, {referrers} referrers: caller {call:?} (heard {heard:?}), {:?} through \
         cleanup: {c:?}",
        started.elapsed()
    );
    heard.expect("the caller hears success: its bound is on a stall, not the operation");
    if !cfg!(windows) {
        assert!(
            call < std::time::Duration::from_secs(10),
            "M4: off Windows a rename of {referrers} referrers answers inside OPERATION_WAIT: {call:?}"
        );
    }
}

#[test]
#[ignore = "release availability gate: scripts/rename-thousand-referrers.sh"]
fn a_thousand_referrer_rename_completes_inside_the_caller_bound() {
    let _integration_guard = crate::integration_lock();
    release_rename_gate(1000);
}

#[test]
#[ignore = "release availability gate: scripts/rename-thousand-referrers.sh"]
fn a_250_referrer_rename_completes_inside_the_caller_bound() {
    let _integration_guard = crate::integration_lock();
    release_rename_gate(250);
}

/// Caller urgency: an operation's pages save without the 400 ms edit
/// debounce, so a small rename returns well inside it. Before, the debounce
/// floored every run at 400 ms; load only adds time, so the fastest of three
/// runs decides.
#[test]
fn a_small_rename_is_not_floored_by_the_edit_debounce() {
    let _integration_guard = crate::integration_lock();
    let _case = CASE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let runs = [0; 3].map(|_| {
        let (_, call, heard) = rename_through_cleanup(20, 2);
        heard.unwrap();
        call
    });
    eprintln!("2-referrer rename calls: {runs:?}");
    let fastest = runs.into_iter().min().unwrap();
    assert!(
        fastest < std::time::Duration::from_millis(400),
        "an operation's caller waits for its pages' saves, which skip the edit \
         debounce (exemplar page_host/progress.rs reconcile): fastest {fastest:?}"
    );
}

// Fixed from the reviewed measurement (RECEIPT "M4": 27, 7, 8, 106 and 24
// per page at 100, 200 and 400 referrers), about 2x headroom.
const RECONCILE_PER_REFERRER: u64 = 60;
const DRAFT_RECORDS_PER_REFERRER: u64 = 16;
const CANDIDATES_PER_REFERRER: u64 = 20;
const WORKER_KEYS_PER_REFERRER: u64 = 220;
const BOOK_PER_REFERRER: u64 = 50;
const HOST_WORK_FLOOR: u64 = 500;

fn walk_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(walk_files(&path));
        } else {
            files.push(path);
        }
    }
    files
}

/// G1 timing probe, not a gate: a rename's wall time on this platform's
/// filesystem (hosted Windows decides card B's priority, Martin 2026-10-11).
#[test]
#[ignore = "timing probe: cargo test -p tine-graph-features --test integration rename_cost::probe_rename_wall_time -- --ignored --nocapture"]
fn probe_rename_wall_time() {
    let _integration_guard = crate::integration_lock();
    let _case = CASE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    for (pages_count, referrers) in [(60, 2), (60, 30), (600, 30)] {
        let mut times: Vec<_> = (0..5)
            .map(|_| rename_timed(pages_count, referrers).1.as_secs_f64() * 1000.0)
            .collect();
        times.sort_by(f64::total_cmp);
        eprintln!(
            "G1 rename {pages_count}p/{referrers}r: median {:.1} ms, min {:.1}, max {:.1}",
            times[2], times[0], times[4]
        );
    }
}
