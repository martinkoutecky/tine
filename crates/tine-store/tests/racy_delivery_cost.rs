//! REVIEW-3b-P2b R4(b), storage spec §5.4: the watcher's racy rehash is
//! linear in the files a delivery changed (a sync tool or an external
//! editor), not in the graph. Each delivered file is read a bounded number of
//! times per delivery, aged unrelated files are not read, and a settled
//! watcher reads nothing more. Measured on Linux: four opens per delivered
//! file (its reconciliation's whole read and two preamble opens, then the
//! racy follow-up's rehash once its stamp ages out of the 2 s window) and
//! one by-path stamp. Its own test binary: the counters are process-global.
use std::fs;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use tine_store::cost_counters::{self, Counts};
use tine_store::Store;

static CASE_LOCK: Mutex<()> = Mutex::new(());

/// Longer than the path-scoped diff's debounce plus the racy follow-up.
const QUIET: Duration = Duration::from_secs(4);

/// Every read of a graph file, on any thread.
fn reads(c: &Counts) -> u64 {
    c.full_reads + c.preamble_reads + c.hash_reads + c.watcher_reads
}

/// Wait until at least `least` reads were counted and none for `QUIET`.
fn settle(least: u64) -> Counts {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut last = reads(&cost_counters::snapshot());
    let mut quiet_since = Instant::now();
    while last < least || quiet_since.elapsed() < QUIET {
        assert!(
            Instant::now() < deadline,
            "the watcher never settled: {last} reads"
        );
        std::thread::sleep(Duration::from_millis(100));
        let now = reads(&cost_counters::snapshot());
        if now != last {
            last = now;
            quiet_since = Instant::now();
        }
    }
    cost_counters::snapshot()
}

/// An aged graph of `pages` pages; once the watcher settled, a delivery
/// rewrites `delivered` of them. Returns the delivery's settled counts and
/// the counts of a further quiet period with no delivery.
fn deliver(pages: usize, delivered: usize) -> (Counts, Counts) {
    let dir = tempfile::Builder::new()
        .prefix("racy-delivery-")
        .tempdir()
        .unwrap();
    let root = dir.path().join("graph");
    fs::create_dir_all(root.join("pages")).unwrap();
    let old = SystemTime::now() - Duration::from_secs(3600);
    let page = |index: usize| root.join(format!("pages/Page{index:05}.md"));
    for index in 0..pages {
        fs::write(page(index), format!("- page {index}\n")).unwrap();
        let file = fs::File::options().write(true).open(page(index)).unwrap();
        file.set_modified(old).unwrap();
    }
    let store = Store::open(&root, Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    settle(0);
    cost_counters::reset();
    for index in 0..delivered {
        fs::write(page(index), format!("- delivered {index}\n")).unwrap();
    }
    let delivery = settle(delivered as u64);
    cost_counters::reset();
    std::thread::sleep(QUIET);
    let after = cost_counters::snapshot();
    store.close();
    (delivery, after)
}

#[test]
fn a_deliverys_racy_rehash_is_linear_in_the_files_it_delivered() {
    let _integration_guard = crate::integration_lock();
    let _case = CASE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let small = deliver(60, 5);
    let large = deliver(600, 5);
    let many = deliver(600, 50);
    for (name, (delivery, after)) in [("60/5", small), ("600/5", large), ("600/50", many)] {
        eprintln!("R4(b) {name}: delivery={delivery:?}\nafter={after:?}");
    }
    for (delivered, (delivery, after)) in [(5, small), (5, large), (50, many)] {
        assert!(
            reads(&delivery) <= 4 * delivered + 4
                && delivery.watcher_stamps_by_path <= delivered + 4,
            "storage spec §5.4/R4(b): each delivered file is read a bounded number of times \
             per delivery (reconciliation, racy follow-up); exemplar watch/racy.rs: {delivery:?}"
        );
        assert_eq!(
            reads(&after),
            0,
            "§5.4: a settled racy file is not rehashed again: {after:?}"
        );
    }
    assert_eq!(
        reads(&large.0),
        reads(&small.0),
        "D-10/R4(b): the racy rehash does not read aged unrelated pages: {:?} vs {:?}",
        small.0,
        large.0
    );
}
