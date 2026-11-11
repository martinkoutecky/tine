//! GH #623 (QF3b): on Windows every file open is scanned, so a rename's cost
//! is its opens. QF3 measured eight successful read opens per rewritten
//! referrer and a planner that built the whole-graph name inventory. og's
//! contract: the planner's name work does not grow with the page count, each
//! referrer's references are rewritten once outside the writer. R5/D4R keeps
//! five opens: selection, PagePlan preflight, host admission load, final
//! pre-rename guard and publication. Staged-byte preflight is deferred.
use std::fs;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};
use tine_graph_features::pages;
use tine_store::cost_counters::{self, Counts};
use tine_store::Store;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Watcher {
    Running,
    Drained,
    Stopped,
}

/// Renames are the page host's (STEP3 §7): each runs through a host started
/// for it under its own app data, as the app's graph binding runs one.
fn hosted<T>(
    store: &std::sync::Arc<tine_store::Store>,
    watcher: Watcher,
    run: impl FnOnce(&tine_store::PageHost) -> T,
) -> T {
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(store, app_data.path()).unwrap();
    let result = run(&host);
    if watcher != Watcher::Stopped {
        // The live-watcher boundary (contract §5.4, REVIEW-3b-P2b-2 N3).
        assert!(
            host.quiesce_for_tests(Duration::from_secs(60)),
            "the watcher or the host never settled"
        );
    }
    // Dropping the host joins its driver and returns its held pages to the
    // watcher synchronously, including with the background watcher stopped.
    result
}

static CASE_LOCK: Mutex<()> = Mutex::new(());

/// `pages_count` unrelated pages, `referrers` pages linking `[[Target]]`, and
/// the target. Every file is backdated past the watcher's racy window so a
/// fresh fixture's freshness re-hash is not counted as rename work.
fn rename(pages_count: usize, referrers: usize) -> Counts {
    rename_then(pages_count, referrers, Watcher::Running)
}

/// As [`rename`]; `Drained` includes a further watcher drain after the host
/// stops; `Stopped` isolates publication from independent watcher echoes.
fn rename_then(pages_count: usize, referrers: usize, watcher: Watcher) -> Counts {
    rename_measured(pages_count, referrers, watcher).0
}

fn rename_measured(
    pages_count: usize,
    referrers: usize,
    watcher: Watcher,
) -> (Counts, Vec<cost_counters::WatcherReadCycle>) {
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
            format!("- links [[Target]] [[Target/ghost]] {index}\n"),
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
    if watcher == Watcher::Stopped {
        store.stop_watcher_for_tests();
    }
    cost_counters::reset();
    cost_counters::start_watcher_read_trace();
    let report = hosted(&store, watcher, |host| {
        pages::rename_or_merge_page(&store, host, "Target", "Renamed", None, None, &[])
    })
    .unwrap();
    assert!(report.renamed.iter().any(|page| page.from == "Target/ghost"
        && page.to == "Renamed/ghost"
        && page.path.is_none()));
    if watcher == Watcher::Drained {
        // After the host stopped: the watcher's re-reads of what the rename
        // wrote (contract §5.4, REVIEW-3b-P2b-2 N3).
        assert!(
            store.watcher_drained_for_tests(Duration::from_secs(60)),
            "the watcher never drained"
        );
    }
    let counts = cost_counters::snapshot();
    let cycles = cost_counters::take_watcher_read_trace();
    assert!(root.join("pages/Renamed.md").exists());
    assert_eq!(
        fs::read_to_string(root.join("pages/Ref00001.md")).unwrap(),
        "- links [[Renamed]] [[Renamed/ghost]] 1\n"
    );
    store.close();
    (counts, cycles)
}

#[test]
fn rename_planner_name_work_is_flat_in_graph_size() {
    let _integration_guard = crate::exclusive_integration_lock();
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
    let _integration_guard = crate::exclusive_integration_lock();
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
    let _integration_guard = crate::exclusive_integration_lock();
    let few = rename(60, 2);
    let many = rename(60, 30);
    assert!(
        many.full_reads - few.full_reads <= 4 * 28,
        "GH #623/I-25/R5: each extra referrer costs four whole-file reads under the writer: \
         PagePlan staging, host admission load, final pre-rename guard and publication \
         read; a separate stage-2 verify of a changing rewrite repeated the pre-rename \
         guard's comparison (exemplar transaction/read_checks.rs): {few:?} vs {many:?}"
    );
    assert!(
        many.store_reads - few.store_reads <= 28,
        "GH #623/I-25: the planner reads each referrer once: {few:?} vs {many:?}"
    );
    for counts in [few, many] {
        assert!(
            counts.observation_reads <= counts.files_written + 4,
            "contract §5.4 (O5): once quiesced, the host's re-reads of held pages the watcher \
             forwarded (the echoes of the operation's own writes) are at most the files it \
             wrote plus a small constant, billed apart from the operation's own reads \
             (exemplar page_host/driver.rs step_unblocked): {counts:?}"
        );
    }
}

/// Isolate publication: a forwarded removal echo makes the host stamp the
/// absent source in `Bridge::index`'s unchanged-observation branch. That is
/// off the publishing path, but it is not on the watcher thread and has no
/// successful read to bill as `observation_reads` (contract §5.4).
#[test]
fn rename_publication_reopens_no_referrer() {
    let _integration_guard = crate::exclusive_integration_lock();
    let few = rename_then(60, 2, Watcher::Stopped);
    let many = rename_then(60, 30, Watcher::Stopped);
    eprintln!("rename publication: few={few:?}; many={many:?}");
    assert_eq!(
        (many.preamble_reads + many.hash_reads) - (few.preamble_reads + few.hash_reads),
        0,
        "GH #623/I-25: publication derives the effective name and own stamp from the bytes \
         it already read; re-opening each written file for its preamble, the projection's \
         preamble and the own-write hash cost three opens per referrer \
         (exemplar transaction/publication.rs): {few:?} vs {many:?}"
    );
    assert!(
        // M3-1 adds exactly one post-directory-sync identity probe per save.
        // Keep the own-write check at its later publication boundary as well.
        many.stamps_by_path - few.stamps_by_path <= 2 * 28,
        "GH #623/I-25: two metadata stamps per written referrer (post-sync identity \
         and the own-write check); no additional payload open or hash; \
         the publication read's handle supplies the stamp it is compared with: \
         {few:?} vs {many:?}"
    );
}

/// Keep the live watcher's cost independent of the publication comparison.
#[test]
fn rename_watcher_echo_cost_is_linear_in_written_files() {
    let _integration_guard = crate::exclusive_integration_lock();
    let few = rename_measured(60, 2, Watcher::Drained);
    let many = rename_measured(60, 30, Watcher::Drained);
    // Linear in the files the operation wrote (`files_written`: each page
    // and its draft record), never in the graph. A written page is read at
    // most twice: by the cycle its echo lands in (path-scoped), and once
    // more by the one full diff the operation's directory event
    // (`Create(Folder)`, the trash) forces while its own-write stamp is still
    // racy (§5.4). Windows always splits the two (FOLLOWUPS K5); Linux does
    // when the echoes straddle two watcher cycles, as under host load
    // (REG-LOAD-FLAKY-RENAME-WATCHER-READS: 59 of 33 written at load ~14; one
    // read each when one cycle takes them all). The per-request drain
    // acknowledgement (8f2183c6d) waits for that second read, so the bound
    // counts it on every platform.
    let per_file = 2;
    // What the bound guards: a re-read that grows with the graph. The same
    // rename in a graph ten times larger stays inside the same bound, which
    // a single graph-wide re-read (631 pages) would exceed ninefold.
    let large = rename_measured(600, 30, Watcher::Drained);
    for (pages_count, (counts, cycles)) in [(60, few), (60, many), (600, large)] {
        let bound = per_file * counts.files_written + 4;
        // Printed on success too: the hosted Windows flake is intermittent,
        // and the per-pass split of a passing run shows how close it runs.
        eprintln!(
            "rename_io watcher trace: unrelated pages={pages_count} reads={} stamps={} \
             bound={bound} cycles={cycles:?}",
            counts.watcher_reads, counts.watcher_stamps_by_path
        );
        assert!(
            counts.watcher_reads <= bound && counts.watcher_stamps_by_path <= bound,
            "contract §5.4: once settled, the watcher's re-reads and re-stamps of what an \
             operation wrote are at most twice the files it wrote plus a small constant; a \
             re-read that grows with the graph is not the accepted cost \
             (exemplar watch/reconcile.rs racy::hash_settled), \
             unrelated pages={pages_count}, bound={bound}: {counts:?}; \
             cycle reads=[initial hash, racy rehash, metadata retry, other]: {cycles:#?}"
        );
    }
}

/// O5: a planted echo of a held page (its own bytes written again, as an
/// operation's save echoes) is forwarded by the watcher, and the host's
/// re-read of it bills as `observation_reads`, not as the operation's
/// `full_reads`. On Windows such an echo of a rename's own write landed in
/// the rename's measured window and broke its publishing bound.
#[test]
fn a_forwarded_echo_bills_as_an_observation_read() {
    let _integration_guard = crate::exclusive_integration_lock();
    let _case = CASE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("graph");
    fs::create_dir_all(root.join("pages")).unwrap();
    let held = root.join("pages/Held.md");
    fs::write(&held, "- held\n").unwrap();
    let old = SystemTime::now() - Duration::from_secs(3600);
    fs::File::options()
        .write(true)
        .open(&held)
        .unwrap()
        .set_modified(old)
        .unwrap();
    let store = std::sync::Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    let (sender, mail) = std::sync::mpsc::channel::<serde_json::Value>();
    let host = tine_store::PageHost::start(
        &store,
        &temp.path().join("app"),
        "graph",
        |_, _| {},
        move |m| {
            let _ = sender.send(serde_json::to_value(m).unwrap());
        },
    )
    .unwrap();
    let reloaded = serde_json::to_value(host.window_reloaded()).unwrap();
    let session = reloaded["session"].as_u64().unwrap();
    let id = reloaded["nextId"].as_u64().unwrap();
    let opened = host
        .open(
            session,
            id,
            &tine_store::PageId::from("pages/Held.md"),
            "Held",
        )
        .unwrap();
    let key = serde_json::to_value(&opened).unwrap()["key"]
        .as_str()
        .unwrap()
        .to_owned();
    loop {
        let m = mail.recv_timeout(Duration::from_secs(20)).unwrap();
        if m["key"] == key.as_str() && m["answer"]["id"] == id {
            break;
        }
    }
    cost_counters::reset();
    fs::write(&held, "- held\n").unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while cost_counters::snapshot().observation_reads == 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "O5: the host's read of a forwarded echo is an observation read: {:?}",
            cost_counters::snapshot()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let counts = cost_counters::snapshot();
    assert_eq!(
        counts.full_reads, 0,
        "O5: an echo's re-read is not billed to the operation: {counts:?}"
    );
    drop(host);
    store.close();
}
