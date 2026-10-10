//! GH #623 (QF3b): on Windows every file open is scanned, so a rename's cost
//! is its opens. QF3 measured eight successful read opens per rewritten
//! referrer and a planner that built the whole-graph name inventory. og's
//! contract: the planner's name work does not grow with the page count, each
//! referrer's references are rewritten once (not again under the writer
//! lock), and each rewritten referrer is opened a constant four times: the
//! planner's read, the preflight base-revision stage, the final pre-rename
//! guard, and the publication read (I-13, I-25).
use std::fs;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};
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

/// `pages_count` unrelated pages, `referrers` pages linking `[[Target]]`, and
/// the target. Every file is backdated past the watcher's racy window so a
/// fresh fixture's freshness re-hash is not counted as rename work.
fn rename(pages_count: usize, referrers: usize) -> Counts {
    rename_then(pages_count, referrers, false)
}

/// The watcher's re-reads of the files a rename wrote land after the rename
/// returns: the path-scoped diff after its debounce, and the racy follow-up
/// about 2 s later (contract §5.4). Quiet for longer than that means settled.
const WATCHER_QUIET: Duration = Duration::from_secs(4);

/// As [`rename`]; with `settle`, the counts are taken only after the watcher
/// has gone quiet, so they include its re-reads.
fn rename_then(pages_count: usize, referrers: usize, settle: bool) -> Counts {
    let _case = CASE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let temp = tempfile::Builder::new()
        .prefix("rename-io-")
        .tempdir()
        .unwrap();
    let root = temp.path().to_path_buf();
    fs::create_dir_all(root.join("pages")).unwrap();
    let mut files = Vec::new();
    for index in 0..pages_count {
        files.push((
            format!("pages/Other{index:05}.md"),
            format!("- unrelated [[Other{}]] {index}\n", index + 1),
        ));
    }
    for index in 0..referrers {
        files.push((
            format!("pages/Ref{index:05}.md"),
            format!("- links [[Target]] {index}\n"),
        ));
    }
    files.push(("pages/Target.md".into(), "- the target\n".into()));
    let old = SystemTime::now() - Duration::from_secs(3600);
    for (path, text) in &files {
        let path = root.join(path);
        fs::write(&path, text).unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
    let store = std::sync::Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    cost_counters::reset();
    hosted(&store, |host| {
        pages::rename_page_expected(&store, host, "Target", "Renamed", None)
    })
    .unwrap();
    if settle {
        let watcher = |c: Counts| (c.watcher_hash_reads, c.watcher_stamps_by_path);
        let mut last = watcher(cost_counters::snapshot());
        let mut quiet_since = std::time::Instant::now();
        while quiet_since.elapsed() < WATCHER_QUIET {
            std::thread::sleep(Duration::from_millis(100));
            let now = watcher(cost_counters::snapshot());
            if now != last {
                last = now;
                quiet_since = std::time::Instant::now();
            }
        }
    }
    let counts = cost_counters::snapshot();
    assert!(root.join("pages/Renamed.md").exists());
    assert_eq!(
        fs::read_to_string(root.join("pages/Ref00001.md")).unwrap(),
        "- links [[Renamed]] 1\n"
    );
    store.close();
    counts
}

#[test]
fn rename_planner_name_work_is_flat_in_graph_size() {
    let small = rename(60, 2);
    let large = rename(600, 2);
    assert_eq!(
        large.name_inventory_entries, small.name_inventory_entries,
        "GH #623/I-13: the rename planner needs only the renamed page's own files, alias \
         collisions and referrers; building the whole-graph name inventory made planning \
         grow with the graph (exemplar store/inventory.rs page_files_at_or_under): \
         {small:?} vs {large:?}"
    );
}

#[test]
fn rename_rewrites_each_referrer_once_outside_the_writer() {
    let few = rename(60, 2);
    let many = rename(60, 30);
    assert_eq!(
        many.transaction_rewrites, few.transaction_rewrites,
        "GH #623/I-25: a referrer's rewrite is prepared once by the planner and handed to \
         preflight, which reuses it only when the staged bytes equal the prepared old bytes \
         (exemplar transaction/prepared.rs): {few:?} vs {many:?}"
    );
}

#[test]
fn rename_reads_each_referrer_once_per_guard() {
    let few = rename(60, 2);
    let many = rename(60, 30);
    assert!(
        many.full_reads - few.full_reads <= 3 * 28,
        "GH #623/I-25: each extra referrer costs three whole-file reads under the writer: \
         the preflight base-revision stage, the final pre-rename guard and the publication \
         read; a separate stage-2 verify of a changing rewrite repeated the pre-rename \
         guard's comparison (exemplar transaction/read_checks.rs): {few:?} vs {many:?}"
    );
    assert!(
        many.store_reads - few.store_reads <= 28,
        "GH #623/I-25: the planner reads each referrer once: {few:?} vs {many:?}"
    );
}

/// Counted on the publishing threads only: the watcher's settle re-reads of
/// the written files are an accepted cost off the publish path (contract
/// §5.4), bounded separately below. On Windows they landed inside the
/// measured window and were billed to publication.
#[test]
fn rename_publication_reopens_no_referrer() {
    let few = rename_then(60, 2, true);
    let many = rename_then(60, 30, true);
    assert_eq!(
        (many.preamble_reads + many.hash_reads) - (few.preamble_reads + few.hash_reads),
        0,
        "GH #623/I-25: publication derives the effective name and own stamp from the bytes \
         it already read; re-opening each written file for its preamble, the projection's \
         preamble and the own-write hash cost three opens per referrer \
         (exemplar transaction/publication.rs): {few:?} vs {many:?}"
    );
    assert!(
        many.stamps_by_path - few.stamps_by_path <= 28,
        "GH #623/I-25: one metadata stamp per written referrer (the own-write check); \
         the publication read's handle supplies the stamp it is compared with: \
         {few:?} vs {many:?}"
    );
    // The graph files the rename wrote: each referrer plus the renamed page.
    for (counts, written) in [(few, 2 + 1), (many, 30 + 1)] {
        assert!(
            counts.watcher_hash_reads <= written + 4
                && counts.watcher_stamps_by_path <= written + 4,
            "contract §5.4: once settled, the watcher re-reads each of the {written} graph \
             files the operation wrote at most once (its racy follow-up), plus a small \
             constant; a re-read that grows with the graph or repeats per file is not the \
             accepted cost \
             (exemplar watch/reconcile.rs racy::hash_settled): {counts:?}"
        );
    }
}
