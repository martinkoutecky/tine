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

/// I-25 (host-only-writer S2): a namespace rename's unit cost. `Ns` with
/// two children and one referrer, on 1-block and 60-block pages: files and
/// bytes written and fsyncs, measured on the host's instrumented I/O from
/// the call through the host's cleanup.
#[test]
fn namespace_rename_unit_cost_on_one_and_sixty_blocks() {
    let _integration_guard = crate::integration_lock();
    let _case = CASE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    for blocks in [1, 60] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("graph");
        fs::create_dir_all(root.join("pages")).unwrap();
        let body = |text: &str| format!("- {text}\n").repeat(blocks);
        fs::write(root.join("pages/Ns.md"), body("ns")).unwrap();
        fs::write(root.join("pages/Ns%2Fa.md"), body("a")).unwrap();
        fs::write(root.join("pages/Ns%2Fb.md"), body("b")).unwrap();
        fs::write(root.join("pages/Ref.md"), body("links [[Ns/a]]")).unwrap();
        let store = std::sync::Arc::new(Store::open(&root, Default::default()).unwrap().0);
        store.whole_graph().unwrap();
        let host = tine_store::PageHost::start_for_tests(&store, &temp.path().join("app")).unwrap();
        let started = std::time::Instant::now();
        while !host.quiescent_for_tests() {
            assert!(started.elapsed().as_secs() < 30);
            std::thread::yield_now();
        }
        let source_bytes: usize = ["Ns.md", "Ns%2Fa.md", "Ns%2Fb.md", "Ref.md"]
            .iter()
            .map(|name| fs::read(root.join("pages").join(name)).unwrap().len())
            .sum();
        cost_counters::reset();
        pages::rename_page_expected(&store, &host, "Ns", "Space", None).unwrap();
        let renamed = std::time::Instant::now();
        while !host.quiescent_for_tests() {
            assert!(renamed.elapsed().as_secs() < 60);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let cost = cost_counters::snapshot();
        eprintln!("S2 namespace rename, {blocks} blocks, {source_bytes} source bytes: {cost:?}");
        assert_eq!(
            fs::read_to_string(root.join("pages/Space%2Fa.md")).unwrap(),
            body("a")
        );
        assert_eq!(
            fs::read_to_string(root.join("pages/Ref.md")).unwrap(),
            body("links [[Space/a]]")
        );
        assert!(!root.join("pages/Ns%2Fb.md").exists());
        store.close();
    }
}

#[test]
#[ignore]
fn namespace_rename_cost_scale_probe() {
    let _integration_guard = crate::integration_lock();
    let _case = CASE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    for children in [2usize, 22] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("graph");
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::write(root.join("pages/Ns.md"), "- ns\n").unwrap();
        for i in 0..children {
            fs::write(root.join(format!("pages/Ns%2Fc{i:03}.md")), "- child\n").unwrap();
        }
        fs::write(root.join("pages/Ref.md"), "- links [[Ns/c000]]\n").unwrap();
        let store = std::sync::Arc::new(Store::open(&root, Default::default()).unwrap().0);
        store.whole_graph().unwrap();
        let host = tine_store::PageHost::start_for_tests(&store, &temp.path().join("app")).unwrap();
        while !host.quiescent_for_tests() {
            std::thread::yield_now();
        }
        cost_counters::reset();
        let t = std::time::Instant::now();
        pages::rename_page_expected(&store, &host, "Ns", "Space", None).unwrap();
        let call = t.elapsed();
        while !host.quiescent_for_tests() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let c = cost_counters::snapshot();
        eprintln!(
            "SCALE children={children} call={call:?} files={} bytes={} fsyncs={}",
            c.files_written, c.bytes_written, c.fsyncs
        );
        assert!(root.join("pages/Space%2Fc001.md").exists());
        store.close();
    }
}

// ---------------------------------------------------------------------------
// PROBE ONLY (branch probe/s2-windows-timing, never merged): S2 Windows
// timing of host page operations with the trash-custody marker on and off,
// and the namespace rename on the pre-W1 transaction path.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum ProbeOp {
    Delete,
    Namespace,
    NamespaceTx,
    Single,
}

const PROBE_CHILDREN: usize = 20;

struct ProbeSample {
    total_ms: f64,
    call_ms: f64,
    fsyncs: u64,
    file_syncs: u64,
    files_written: u64,
}

fn probe_wait_quiescent(host: &tine_store::PageHost, what: &str) {
    let started = std::time::Instant::now();
    while !host.quiescent_for_tests() {
        assert!(
            started.elapsed().as_secs() < 120,
            "{what}: host never settled"
        );
        std::thread::sleep(Duration::from_micros(250));
    }
}

fn probe_one(op: ProbeOp, marker: bool) -> ProbeSample {
    use std::sync::atomic::Ordering::Relaxed;
    let temp = tempfile::Builder::new()
        .prefix("s2-timing-")
        .tempdir()
        .unwrap();
    let root = temp.path().join("graph");
    // An existing trash chain, as in any graph that has deleted a page once:
    // the first deletion's uncounted chain creation is not the steady state.
    fs::create_dir_all(root.join("logseq/.tine-trash/pages")).unwrap();
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    match op {
        ProbeOp::Delete => {
            fs::write(root.join("pages/Gone.md"), "- gone\n").unwrap();
            fs::write(root.join("pages/Keep.md"), "- keep\n").unwrap();
        }
        ProbeOp::Namespace | ProbeOp::NamespaceTx => {
            fs::write(root.join("pages/Ns.md"), "- ns\n").unwrap();
            for i in 0..PROBE_CHILDREN {
                fs::write(
                    root.join(format!("pages/Ns%2Fc{i:03}.md")),
                    format!("- child {i}\n"),
                )
                .unwrap();
            }
            fs::write(root.join("pages/Ref.md"), "- links [[Ns/c000]]\n").unwrap();
        }
        ProbeOp::Single => {
            fs::write(root.join("pages/Target.md"), "- the target\n").unwrap();
            fs::write(root.join("pages/Ref.md"), "- links [[Target]]\n").unwrap();
        }
    }
    let store = std::sync::Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    let host = tine_store::PageHost::start_for_tests(&store, &temp.path().join("app")).unwrap();
    probe_wait_quiescent(&host, "launch");
    let session = serde_json::to_value(host.window_reloaded()).unwrap()["session"]
        .as_u64()
        .unwrap();
    probe_wait_quiescent(&host, "reload");
    cost_counters::PROBE_NO_MARKER.store(!marker, Relaxed);
    cost_counters::PROBE_TX_RENAME.store(op == ProbeOp::NamespaceTx, Relaxed);
    cost_counters::reset();
    cost_counters::PROBE_FILE_SYNCS.store(0, Relaxed);
    let started = std::time::Instant::now();
    match op {
        ProbeOp::Delete => {
            pages::delete_page_expected(
                &store,
                &host,
                session,
                "Gone",
                tine_core::model::PageKind::Page,
                None,
            )
            .unwrap();
        }
        ProbeOp::Namespace | ProbeOp::NamespaceTx => {
            pages::rename_page_expected(&store, &host, "Ns", "Space", None).unwrap();
        }
        ProbeOp::Single => {
            pages::rename_page_expected(&store, &host, "Target", "Renamed", None).unwrap();
        }
    }
    let call = started.elapsed();
    probe_wait_quiescent(&host, "operation");
    let total = started.elapsed();
    let counts = cost_counters::snapshot();
    let file_syncs = cost_counters::PROBE_FILE_SYNCS.load(Relaxed);
    cost_counters::PROBE_NO_MARKER.store(false, Relaxed);
    cost_counters::PROBE_TX_RENAME.store(false, Relaxed);
    // The operation did what it says.
    match op {
        ProbeOp::Delete => assert!(!root.join("pages/Gone.md").exists()),
        ProbeOp::Namespace | ProbeOp::NamespaceTx => {
            assert!(!root.join("pages/Ns.md").exists());
            for i in 0..PROBE_CHILDREN {
                assert!(!root.join(format!("pages/Ns%2Fc{i:03}.md")).exists());
                assert_eq!(
                    fs::read_to_string(root.join(format!("pages/Space%2Fc{i:03}.md"))).unwrap(),
                    format!("- child {i}\n")
                );
            }
            assert_eq!(
                fs::read_to_string(root.join("pages/Ref.md")).unwrap(),
                "- links [[Space/c000]]\n"
            );
        }
        ProbeOp::Single => {
            assert!(!root.join("pages/Target.md").exists());
            assert_eq!(
                fs::read_to_string(root.join("pages/Ref.md")).unwrap(),
                "- links [[Renamed]]\n"
            );
        }
    }
    let markers: Vec<_> = walk_files(&temp.path().join("app"))
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "tcm"))
        .collect();
    assert!(markers.is_empty(), "markers left: {markers:?}");
    drop(host);
    store.close();
    ProbeSample {
        total_ms: total.as_secs_f64() * 1000.0,
        call_ms: call.as_secs_f64() * 1000.0,
        fsyncs: counts.fsyncs,
        file_syncs,
        files_written: counts.files_written,
    }
}

#[test]
#[ignore = "S2 timing probe: cargo test --release -p tine-graph-features --test integration rename_cost::probe_s2_windows_timing -- --ignored --exact --nocapture"]
fn probe_s2_windows_timing() {
    let _integration_guard = crate::integration_lock();
    let _case = CASE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let warmup: usize = std::env::var("S2_PROBE_WARMUP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    let runs: usize = std::env::var("S2_PROBE_RUNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(21);
    let configs = [
        (ProbeOp::Delete, true),
        (ProbeOp::Delete, false),
        (ProbeOp::Single, true),
        (ProbeOp::Single, false),
        (ProbeOp::Namespace, true),
        (ProbeOp::Namespace, false),
        (ProbeOp::NamespaceTx, true),
    ];
    // Timer granularity of the quiescence poll on this platform.
    let mut sleeps: Vec<f64> = (0..50)
        .map(|_| {
            let t = std::time::Instant::now();
            std::thread::sleep(Duration::from_micros(250));
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    sleeps.sort_by(f64::total_cmp);
    eprintln!(
        "S2T|meta|os={}|temp={}|poll_sleep_250us_median_ms={:.3}|warmup={warmup}|runs={runs}|children={PROBE_CHILDREN}",
        std::env::consts::OS,
        std::env::temp_dir().display(),
        sleeps[sleeps.len() / 2]
    );
    let mut samples: Vec<Vec<ProbeSample>> = configs.iter().map(|_| Vec::new()).collect();
    for iteration in 0..warmup + runs {
        // Rotate the order each iteration so no configuration always runs
        // first or right after another.
        for k in 0..configs.len() {
            let index = (k + iteration) % configs.len();
            let (op, marker) = configs[index];
            let sample = probe_one(op, marker);
            eprintln!(
                "S2T|raw|{op:?}|marker={}|iter={iteration}|total_ms={:.3}|call_ms={:.3}|fsyncs={}|file_syncs={}|files={}",
                if marker { "on" } else { "off" },
                sample.total_ms,
                sample.call_ms,
                sample.fsyncs,
                sample.file_syncs,
                sample.files_written
            );
            if iteration >= warmup {
                samples[index].push(sample);
            }
        }
    }
    let rank = |sorted: &[f64], q: f64| {
        let n = sorted.len();
        sorted[((q * n as f64).ceil() as usize).clamp(1, n) - 1]
    };
    for ((op, marker), list) in configs.iter().zip(&samples) {
        let mut total: Vec<f64> = list.iter().map(|s| s.total_ms).collect();
        let mut call: Vec<f64> = list.iter().map(|s| s.call_ms).collect();
        total.sort_by(f64::total_cmp);
        call.sort_by(f64::total_cmp);
        let fsyncs: std::collections::BTreeSet<u64> = list.iter().map(|s| s.fsyncs).collect();
        let files: std::collections::BTreeSet<u64> = list.iter().map(|s| s.file_syncs).collect();
        let written: std::collections::BTreeSet<u64> =
            list.iter().map(|s| s.files_written).collect();
        eprintln!(
            "S2T|summary|{op:?}|marker={}|n={}|median_ms={:.3}|p90_ms={:.3}|min_ms={:.3}|max_ms={:.3}|call_median_ms={:.3}|call_p90_ms={:.3}|fsyncs={fsyncs:?}|file_syncs={files:?}|files_written={written:?}",
            if *marker { "on" } else { "off" },
            total.len(),
            rank(&total, 0.5),
            rank(&total, 0.9),
            total[0],
            total[total.len() - 1],
            rank(&call, 0.5),
            rank(&call, 0.9),
        );
    }
}
