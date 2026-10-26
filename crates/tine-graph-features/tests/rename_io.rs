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
    let result = run(&host);
    // The measurement boundary (contract §5.4, REVIEW-3b-P2b-2 N3).
    assert!(
        host.quiesce_for_tests(Duration::from_secs(60)),
        "the watcher or the host never settled"
    );
    result
}

static CASE_LOCK: Mutex<()> = Mutex::new(());

/// `pages_count` unrelated pages, `referrers` pages linking `[[Target]]`, and
/// the target. Every file is backdated past the watcher's racy window so a
/// fresh fixture's freshness re-hash is not counted as rename work.
fn rename(pages_count: usize, referrers: usize) -> Counts {
    rename_then(pages_count, referrers, false)
}

/// As [`rename`]; with `settle`, the counts are taken only after the watcher
/// drained once more after the host stopped, so they include its re-reads.
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
        // After the host stopped: the watcher's re-reads of what the rename
        // wrote (contract §5.4, REVIEW-3b-P2b-2 N3).
        assert!(
            store.watcher_drained_for_tests(Duration::from_secs(60)),
            "the watcher never drained"
        );
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
    let _integration_guard = crate::integration_lock();
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
    let _integration_guard = crate::integration_lock();
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
    let _integration_guard = crate::integration_lock();
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

/// Counted on the publishing threads only: the watcher's settle re-reads of
/// the written files are an accepted cost off the publish path (contract
/// §5.4), bounded separately below. On Windows they landed inside the
/// measured window and were billed to publication.
#[test]
fn rename_publication_reopens_no_referrer() {
    let _integration_guard = crate::integration_lock();
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
    // Linear in the files the operation wrote (`files_written`: each page
    // and its draft record), never in the graph. Measured on 31 written
    // pages: Linux re-reads each once (the racy follow-up), Windows up to
    // twice (the echo event's path-scoped diff, then the racy follow-up;
    // FOLLOWUPS K5). The per-request drain acknowledgement (8f2183c6d)
    // waits for that second read, so the bound counts it.
    let per_file = if cfg!(windows) { 2 } else { 1 };
    for counts in [few, many] {
        let bound = per_file * counts.files_written + 4;
        assert!(
            counts.watcher_reads <= bound && counts.watcher_stamps_by_path <= bound,
            "contract §5.4: once settled, the watcher's re-reads and re-stamps of what an \
             operation wrote are at most the files it wrote plus a small constant; a \
             re-read that grows with the graph is not the accepted cost \
             (exemplar watch/reconcile.rs racy::hash_settled): {counts:?}"
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
    let _integration_guard = crate::integration_lock();
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
