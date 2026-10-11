//! GH #406 (master 05a4b0001c5b, 13fce7f25764): a page rename's disk bill is
//! linear in what it rewrites. Master listed every ancestor directory on each
//! referrer write, so the write phase cost referrers x pages. og's contract:
//! directory enumerations per rename do not grow with the referrer count or
//! the page count, whole-file reads do not grow with the page count, and each
//! referrer costs one write and a bounded number of guard reads (I-13, I-25).
use std::fs;
use std::sync::Mutex;
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
    run(&host)
}

static CASE_LOCK: Mutex<()> = Mutex::new(());

/// `pages` unrelated pages spread over three folders, `referrers` pages that
/// link `[[Target]]`, and the target itself.
fn rename(pages_count: usize, referrers: usize) -> Counts {
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
    hosted(&store, |host| {
        pages::rename_page_expected(&store, host, "Target", "Renamed", None)
    })
    .unwrap();
    let counts = cost_counters::snapshot();
    assert!(root.join("pages/Renamed.md").exists());
    assert_eq!(
        fs::read_to_string(root.join("pages/Ref00000.md")).unwrap(),
        "- links [[Renamed]] 0\n"
    );
    store.close();
    counts
}

#[test]
fn rename_cost_is_linear_in_referrers_and_flat_in_graph_size() {
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
    // temporary file and its record in the operation's draft explosion
    // (STEP3 §4); the transaction path wrote the page alone.
    assert_eq!(
        many.files_written - few.files_written,
        2 * 28,
        "I-25: each extra referrer costs exactly two file writes (its page and its draft record)"
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
}

/// REVIEW-3b-P2b R5: a rename's publication runs on the page host, so the
/// measured subject is the host's publication bookkeeping
/// (`host_publication_probes`): reply slots and orders examined per page
/// publication, the caller's per-wake witness check, and index
/// publications. The transaction path's record probes this test counted
/// before (GH #623) are no longer on a rename's path.
#[test]
fn rename_publication_work_is_linear() {
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
    let _case = CASE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    for referrers in [8, 64] {
        let counts = rename(80, referrers);
        assert!(counts.parses <= referrers as u64 + 2,
            "GH #623/I-25: own reference rewrites must parse the new document once, without serializing/parsing the old one; exemplar model/transaction_publish.rs: {counts:?}");
    }
}

#[test]
fn rename_unit_cost_is_the_same_full_payload_on_one_and_sixty_blocks() {
    let _case = CASE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    for (blocks, bytes) in [(1, 552), (60, 6928)] {
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
        // counted where it is made: the operation draft, three page draft
        // records, the referrer and the destination page, and the source's
        // custody marker. The referrer's bytes travel in the operation draft
        // and in its draft record as base and payload.
        assert_eq!(cost.files_written, 7);
        assert_eq!(cost.bytes_written, bytes);
        // The draft worker may sync two retirements of the draft directory
        // together, so the count is 25 or 26.
        assert!(
            (25..=26).contains(&cost.fsyncs),
            "file, directory and draft syncs of one rename: {cost:?}"
        );
        assert_eq!(
            fs::read_to_string(root.path().join("pages/Ref.md")).unwrap(),
            "- links [[Renamed]] 0\n".repeat(blocks)
        );
        // The documented unit-cost values are pinned to this actual write path.
        let contract = include_str!("../../../docs/storage-contract.md");
        assert!(contract.contains("552/6,928 bytes"));
        store.close();
    }
}

/// One rename of `referrers` referrers among `pages_count` unrelated pages on
/// a host already started, measured from the call through the host's cleanup
/// (every draft retired, every page released): the counters are reset after
/// host startup, and the snapshot is taken once the host is quiescent. Checks
/// every referrer's bytes and the source's deletion. Also returns how long the
/// rename call itself took.
fn rename_through_cleanup(pages_count: usize, referrers: usize) -> (Counts, std::time::Duration) {
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
    pages::rename_page_expected(&store, &host, "Target", "Renamed", None).unwrap();
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
    (counts, call)
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
    let _case = CASE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    // Every size is measured before any is judged, so a failure shows the growth.
    let measured = [100u64, 200, 400].map(|referrers| {
        let (c, _) = rename_through_cleanup(80, referrers as usize);
        eprintln!("M4 rename host work, {referrers} referrers: {c:?}");
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
/// apart from the structural counts above. A 1000-referrer rename completes
/// inside its caller's bound (`OPERATION_WAIT`, 10 s): `rename_page_expected`
/// returns `TimedOut` past it. Release build only, so a loaded debug suite
/// cannot decide it; the executable gate is `scripts/rename-thousand-referrers.sh`.
#[test]
#[ignore = "release availability gate: scripts/rename-thousand-referrers.sh"]
fn a_thousand_referrer_rename_completes_inside_the_caller_bound() {
    let _case = CASE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    assert!(
        !cfg!(debug_assertions),
        "a release gate: run scripts/rename-thousand-referrers.sh"
    );
    let started = std::time::Instant::now();
    let (c, _) = rename_through_cleanup(80, 1000);
    eprintln!(
        "M4 1000 referrers: {:?} through cleanup: {c:?}",
        started.elapsed()
    );
}

/// Caller urgency: an operation's pages save without the 400 ms edit
/// debounce, so a small rename returns well inside it. Before, the debounce
/// floored every run at 400 ms; load only adds time, so the fastest of three
/// runs decides.
#[test]
fn a_small_rename_is_not_floored_by_the_edit_debounce() {
    let _case = CASE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let runs = [0; 3].map(|_| rename_through_cleanup(20, 2).1);
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
