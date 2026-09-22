#[test]
fn direct_query_producer_has_no_saved_edit_or_answer_cache_protocol() {
    let production = crate::test_support::rust_module_production_source("direct_projection.rs");
    for forbidden in [
        "PhysicalProjectionQueryProgress",
        "PhysicalProjectionQueryTarget",
        "saved_query_target",
        "QueryResultMemo",
        "SharedResultMemo",
    ] {
        assert!(
            !production.contains(forbidden),
            "Direct queries must use current SQLite and existing reader lifecycle: {forbidden}"
        );
    }
}

use super::*;
use crate::model::Graph;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

static PROJECTION_TEST_LOCK: Mutex<()> = Mutex::new(());

/// Serialize the projection tests without letting one failure hide the
/// rest. This mutex guards no invariant — it only stops two tests driving
/// the same global projection worker at once — so a panicking test must not
/// poison it. It used to be taken with `.unwrap()`, and a single failing
/// test then turned 39 unrelated tests into `PoisonError` noise, which is
/// exactly the shape that makes a suite unreadable by name.
fn serialize_projection_tests() -> std::sync::MutexGuard<'static, ()> {
    PROJECTION_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn scratch(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("tine-direct-projection-{tag}-{}", Uuid::new_v4()))
}

fn reset_lowerings(root: &Path) {
    *PHYSICAL_PAGE_LOWERINGS.lock().unwrap() = (Some(root.to_path_buf()), 0);
}

fn lowerings() -> u64 {
    PHYSICAL_PAGE_LOWERINGS.lock().unwrap().1
}

fn signature(groups: &[crate::model::RefGroup]) -> Vec<(String, Vec<(String, String)>)> {
    groups
        .iter()
        .map(|group| {
            (
                group.page.clone(),
                group
                    .blocks
                    .iter()
                    .map(|block| (block.id.clone(), block.raw.clone()))
                    .collect(),
            )
        })
        .collect()
}

/// Run `attempt` until it answers, retrying ONLY typed readiness.
///
/// RET2's public Direct route answers from SQL or returns a typed error;
/// `NotReady` is the one error a caller may retry, and this is the same
/// signal `src/queryReadiness.ts` loops on. `Unavailable` and `Cancelled`
/// fail the fixture immediately.
fn when_ready<T>(mut attempt: impl FnMut() -> Result<T, crate::query::QueryExecutionError>) -> T {
    let started = Instant::now();
    loop {
        match attempt() {
            Ok(answer) => return answer,
            Err(crate::query::QueryExecutionError::NotReady(reason)) => {
                assert!(
                    started.elapsed() < Duration::from_secs(15),
                    "the query index never became ready ({})",
                    reason.as_str()
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(other) => panic!("the public query route refused: {other}"),
        }
    }
}

/// Reopening a projection database without releasing the previous worker is
/// a race, so no fixture in this crate may do it.
///
/// This is a source scan because the defect is invisible at runtime on a
/// fast machine: the tests that took down the Linux release selection on
/// 2026-09-09 pass locally in 0.05s and failed on a loaded hosted runner
/// after their full readiness deadline, reporting `cache_generation=0` and
/// `Resource temporarily unavailable (os error 11)`.
///
/// It walks the WHOLE crate, deliberately. The first version of this guard
/// scanned only `direct_projection.rs`, the file whose fixtures had failed --
/// and the very next CI run failed on
/// `the_public_ir_route_answers_a_warm_reopen_without_parsing`, the same
/// shape one module over, which a file-scoped scan could never see.
#[test]
fn every_fixture_that_reopens_a_projection_database_releases_the_previous_worker() {
    // Deliberate exception: this test IS the two-live-owners scenario, and
    // its second instance must meet a held lease.
    const TWO_OWNERS: &str = "concurrent_graph_instance_cannot_replace_ready_projection_facts";
    let source_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");

    fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("the crate source tree is readable") {
            let path = entry.expect("a readable directory entry").path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    rust_files(&source_root, &mut files);
    files.sort();
    assert!(
        files.len() > 10,
        "the scan found {} source files, so it is not looking at the crate",
        files.len()
    );

    let mut offenders: Vec<String> = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).expect("a readable source file");
        let mut current = String::from("<file scope>");
        let (mut attaches, mut releases) = (0usize, 0usize);
        for line in text.lines().chain(std::iter::once("fn <end of file>(")) {
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed
                .strip_prefix("fn ")
                .or_else(|| trimmed.strip_prefix("pub(crate) fn "))
                .or_else(|| trimmed.strip_prefix("pub fn "))
            {
                if attaches >= 2 && releases == 0 && current != TWO_OWNERS {
                    offenders.push(format!("{}::{current}", file.display()));
                }
                current = rest
                    .split(['(', '<'])
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                attaches = 0;
                releases = 0;
            }
            if line.contains("attach_direct_projection(database") {
                attaches += 1;
            }
            if line.contains("release_projection(&") {
                releases += 1;
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "these fixtures reattach the same projection database without releasing the \
             previous worker, so the reopen races its exclusive writer lease and fails only \
             under load: {offenders:#?}. Call release_projection(&graph) as the last statement \
             of the first session; a sleep is a guess, not a handoff."
    );
}

/// Every fixture drives projection recovery through `recover_until_ready`.
///
/// One call to `direct_projection_recover_after_failed_read` is permitted
/// to do nothing at all: it latches `pending.rebuild`, and the worker takes
/// that flag only alongside a `full` or `warm` payload, so a payload turn
/// that fails leaves the rebuild latched with nothing to carry it. The app
/// recovers because the user's NEXT query calls recovery again. A fixture
/// that calls it once and then waits is asserting a convergence guarantee
/// the contract does not make, and it fails about one run in twenty on a
/// loaded machine -- which is what took down the Linux release selection on
/// 2026-09-10.
///
/// Crate-wide on purpose. The lease-handoff guard above learned this the
/// expensive way the same night: scoped to the file whose fixtures had
/// failed, it could not see the identical defect one module over.
#[test]
fn every_fixture_drives_projection_recovery_through_the_retrying_helper() {
    const CALL: &str = ".direct_projection_recover_after_failed_read()";
    const HELPER: &str = "recover_until_ready";
    let source_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");

    fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("the crate source tree is readable") {
            let path = entry.expect("a readable directory entry").path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    rust_files(&source_root, &mut files);
    files.sort();
    assert!(
        files.len() > 10,
        "the scan found {} source files, so it is not looking at the crate",
        files.len()
    );

    let mut offenders: Vec<String> = Vec::new();
    let mut helper_calls = 0usize;
    for file in &files {
        let text = std::fs::read_to_string(file).expect("a readable source file");
        let mut current = String::from("<file scope>");
        for line in text.lines() {
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed
                .strip_prefix("fn ")
                .or_else(|| trimmed.strip_prefix("pub(crate) fn "))
                .or_else(|| trimmed.strip_prefix("pub fn "))
            {
                current = rest
                    .split(['(', '<'])
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
            }
            if trimmed.starts_with("///") || trimmed.starts_with("//") {
                continue;
            }
            if !line.contains(CALL) {
                continue;
            }
            // Production calls it on `self` exactly once per query and
            // turns a repair that did not take into an error the next
            // query retries; that is the contract, not a defect.
            if line.contains("self.direct_projection_recover_after_failed_read()") {
                continue;
            }
            if current == HELPER {
                helper_calls += 1;
                continue;
            }
            // This scan names the call it forbids, so it matches itself.
            if current == "every_fixture_drives_projection_recovery_through_the_retrying_helper" {
                continue;
            }
            offenders.push(format!("{}::{current}", file.display()));
        }
    }

    assert_eq!(
        helper_calls, 1,
        "recover_until_ready must be the single fixture entry point that retries recovery"
    );
    assert!(
        offenders.is_empty(),
        "these fixtures call projection recovery directly instead of \
             recover_until_ready(&graph), so they assume one attempt converges and fail \
             about one run in twenty under load: {offenders:#?}"
    );
}

fn wait_ready(graph: &Graph) {
    let started = Instant::now();
    while !graph.direct_projection_ready_test() {
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "Direct Files projection did not converge: cache_generation={} {}",
            graph.cache_generation(),
            graph
                .direct_projection_test()
                .map(|projection| projection.debug_state_test())
                .unwrap_or_else(|| "no projection".to_owned())
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn query_job_closes_snapshot_before_releasing_capacity() {
    let path = std::env::temp_dir().join(format!("tine-query-drop-{}.sqlite", Uuid::new_v4()));
    let writer = rusqlite::Connection::open(&path).unwrap();
    writer.busy_timeout(Duration::ZERO).unwrap();
    writer.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE payload(value TEXT); INSERT INTO payload VALUES ('before');").unwrap();
    let snapshot = PhysicalProjectionQuerySnapshot::open_direct(&path, || Ok(())).unwrap();
    let owner = Arc::new(crate::query_jobs::QueryJobOwner::new(1));
    let slot = match owner.acquire_owned_at_within(owner.capture_epoch(), QUERY_JOB_WAIT) {
        crate::query_jobs::OwnedAdmission::Slot(slot) => slot,
        _ => panic!("query admission"),
    };
    assert!(slot.register(snapshot.cancellation()));
    let job = DirectQueryJob {
        snapshot,
        _slot: slot,
        session_pages: Arc::new(HashSet::new()),
        config: Arc::new(ParseConfig::default()),
        query_revision: 0,
        registry: None,
        registry_owner: Arc::new(Mutex::new(None)),
    };
    writer
        .execute("UPDATE payload SET value='after'", [])
        .unwrap();
    let checkpoint = || {
        writer
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap()
    };
    assert_eq!(checkpoint(), 1, "fixture must retain a real WAL snapshot");
    let (releasing, resume) = owner.pause_next_release_for_test();
    let busy_at_release = {
        let dropper = std::thread::spawn(move || drop(job));
        releasing.recv_timeout(Duration::from_secs(3)).unwrap();
        let busy = checkpoint();
        // Resume before asserting: the old declaration order must fail,
        // not leave the scope waiting forever for its paused dropper.
        resume.send(()).unwrap();
        dropper.join().unwrap();
        busy
    };
    assert_eq!(owner.active(), 0);
    drop(writer);
    let _ = std::fs::remove_file(path);
    assert_eq!(
        busy_at_release, 0,
        "slot release must follow SQLite transaction release"
    );
}

#[test]
fn public_query_replacement_returns_readiness_for_a_new_request() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("public-target-replacement-retry");
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let projection = graph.direct_projection_test().unwrap();
    let old_epoch = projection.query_epoch();
    graph.direct_projection_inject_read_failure_test();
    let first = graph.run_query_bounded("(task TODO)", 100, 1 << 20);
    assert!(
        matches!(first, Err(crate::query::QueryExecutionError::NotReady(_))),
        "replacement must reach the existing automatic readiness retry"
    );
    wait_ready(&graph);
    assert_eq!(projection.active_query_jobs_test(), 0);
    assert_ne!(projection.query_epoch(), old_epoch);
    let result = graph
        .run_query_bounded("(task TODO)", 100, 1 << 20)
        .unwrap();
    assert_eq!(result.total, 3);
    assert!(projection.close_and_wait_for_worker(Duration::from_secs(3)));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn current_snapshot_capture_runs_before_later_queued_save() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("target-capture-before-later-save");
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let projection = graph.direct_projection_test().unwrap();
    // A producer-queued capture drains the preceding publication before
    // registering this test's observer; wait_ready alone sees readiness
    // before the final notification instructions execute.
    let QueryJobOpen::Job(initial) =
        projection.open_current_query_job(RegistrySensitivity::Insensitive)
    else {
        panic!("initial complete image");
    };
    drop(initial);
    let (commit_wake, commits) = std::sync::mpsc::channel();
    let before_revision = projection.observe_commits(commit_wake.clone());
    let entry = graph.list_pages().into_iter().next().unwrap();
    let (paused, observed) = std::sync::mpsc::channel();
    let (resume, resumed) = std::sync::mpsc::channel();
    // Load and settle BEFORE arming the pause. `load_page` assigns runtime
    // ids and enqueues its own projection turn, and the hook is one-shot:
    // armed any earlier it is consumed by the load's turn, so the worker
    // pauses before save B exists and the queued capture below is served
    // against the pre-B image. That is what made this test red -- the
    // capture was measured against the wrong turn, not against a save.
    let mut page = graph.load_page(&entry).unwrap();
    let baseline = page.rev.clone();
    page.blocks[0].raw = "TODO save B".into();
    wait_ready(&graph);
    *BEFORE_APPLY_PENDING.lock().unwrap() = Some(Box::new(move || {
        paused.send(()).unwrap();
        resumed.recv().unwrap();
    }));
    graph.save_page(&page, baseline.as_deref()).unwrap();
    observed.recv_timeout(Duration::from_secs(3)).unwrap();
    let query_projection = Arc::clone(&projection);
    let query = std::thread::spawn(move || {
        query_projection.open_current_query_job(RegistrySensitivity::Insensitive)
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    while projection
        .shared
        .pending
        .lock()
        .unwrap()
        .captures
        .is_empty()
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(1));
    }
    let capture_queued = !projection
        .shared
        .pending
        .lock()
        .unwrap()
        .captures
        .is_empty();
    let mut page = graph.load_page(&entry).unwrap();
    let baseline = page.rev.clone();
    page.blocks[0].raw = "TODO save C".into();
    graph.save_page(&page, baseline.as_deref()).unwrap();
    resume.send(()).unwrap();
    let result = query.join().unwrap();
    wait_ready(&graph);
    assert!(
        capture_queued,
        "current read must enter the bounded capture queue"
    );
    let QueryJobOpen::Job(mut job) = result else {
        panic!("queued C must not reject the earlier coherent query");
    };
    let QueryJobOpen::Job(mut current) =
        projection.open_current_query_job(RegistrySensitivity::Insensitive)
    else {
        panic!("final C snapshot");
    };
    assert!(
        job.query_revision < current.query_revision,
        "capture must occur between B and C"
    );
    for (snapshot, expected) in [
        (&mut job.snapshot, "TODO save B"),
        (&mut current.snapshot, "TODO save C"),
    ] {
        let mut matching = 0;
        snapshot
            .visit_projection_query(
                "SELECT block_id FROM block_text WHERE content = ?1",
                &[tine_storage::sqlite::PhysicalQueryValue::Text(
                    expected.into(),
                )],
                |_| {
                    matching += 1;
                    Ok(std::ops::ControlFlow::Continue(()))
                },
            )
            .unwrap();
        assert_eq!(
            matching, 1,
            "each snapshot must retain its exact semantic result"
        );
    }
    assert!(!job.is_cancelled());
    let notified = commits.recv_timeout(Duration::from_secs(1)).is_ok();
    let published = projection.observe_commits(commit_wake);
    assert!(
        notified,
        "a later committed image must wake the application watcher"
    );
    assert!(published > before_revision);
    assert!(
        published >= before_revision + 2,
        "both B and C commits publish invalidations"
    );
    drop(current);
    drop(job);
    assert!(projection.close_and_wait_for_worker(Duration::from_secs(3)));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn current_snapshot_needs_no_saved_target_and_stays_coherent_across_edits() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("target-acquired-image");
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let projection = graph.direct_projection_test().unwrap();
    let QueryJobOpen::Job(initial) = projection.open_query_job(graph.cache_generation()) else {
        panic!("initial snapshot");
    };
    let initial_revision = initial.query_revision;
    drop(initial);
    let entry = graph.list_pages().into_iter().next().unwrap();
    let mut page = graph.load_page(&entry).unwrap();
    let baseline = page.rev.clone();
    page.blocks[0].raw = "TODO newer acquired image".into();
    graph.save_page(&page, baseline.as_deref()).unwrap();
    wait_ready(&graph);
    projection.mark_stale();
    assert!(!projection.ready_at(graph.cache_generation()));
    let QueryJobOpen::Job(mut job) =
        projection.open_current_query_job(RegistrySensitivity::Insensitive)
    else {
        panic!("a complete current image needs no saved target");
    };
    assert!(job.query_revision > initial_revision);
    assert_eq!(job.query_revision, job.snapshot.query_revision().unwrap());
    assert_eq!(job.config.digest(), graph.config.parse_config().digest());
    assert!(job.registry.is_none());
    let acquired_revision = job.query_revision;
    assert_eq!(
        projection.active_query_jobs_test(),
        1,
        "only the held job retains capacity"
    );
    let mut page = graph.load_page(&entry).unwrap();
    let baseline = page.rev.clone();
    page.blocks[0].raw = "TODO after snapshot".into();
    graph.save_page(&page, baseline.as_deref()).unwrap();
    wait_ready(&graph);
    assert!(!job.is_cancelled());
    assert_eq!(job.snapshot.query_revision().unwrap(), acquired_revision);
    let QueryJobOpen::Job(current) = projection.open_query_job(graph.cache_generation()) else {
        panic!("new image job");
    };
    assert!(current.query_revision > acquired_revision);
    drop(current);
    drop(job);
    assert_eq!(projection.active_query_jobs_test(), 0);
    assert!(projection.close_and_wait_for_worker(Duration::from_secs(3)));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn current_snapshot_requires_initialization_but_not_source_freshness() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("current-initialization");
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    let projection = graph.direct_projection_test().unwrap();
    assert!(matches!(
        projection.open_current_query_job(RegistrySensitivity::Insensitive),
        QueryJobOpen::NotReady
    ));
    graph.warm_cache();
    wait_ready(&graph);
    projection.mark_stale();
    let QueryJobOpen::Job(job) =
        projection.open_current_query_job(RegistrySensitivity::Insensitive)
    else {
        panic!("a complete image remains available while source freshness is unknown");
    };
    drop(job);
    assert!(projection.close_and_wait_for_worker(Duration::from_secs(3)));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn same_config_inventory_preserves_jobs_but_changed_config_cancels_them() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("target-config-lifecycle");
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let projection = graph.direct_projection_test().unwrap();
    let wait_generation = |generation| {
        let started = Instant::now();
        while !projection.ready_at(generation) && started.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            projection.ready_at(generation),
            "producer did not complete inventory"
        );
    };
    let mut pages = Vec::new();
    let mut revisions = HashMap::new();
    for entry in graph.list_pages() {
        revisions.insert(
            entry.path.clone(),
            graph.load_page(&entry).unwrap().rev.unwrap(),
        );
        let mut document = crate::doc::parse(&std::fs::read_to_string(&entry.path).unwrap());
        crate::model::assign_doc_runtime_ids(&mut document.roots, &entry.rel_path);
        pages.push((entry, Arc::new(document)));
    }
    let pages = Arc::new(pages);
    let revisions = Arc::new(revisions);
    let config = Arc::clone(
        &projection
            .shared
            .committed_registry
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .config,
    );
    let generation = graph.cache_generation();
    // The `wait_ready` above is stale by this point: the inventory loop
    // reads every page, and each read advances the cache generation, so the
    // projection is mid-reconciliation at `generation` and correctly refuses
    // a strict-generation job. Readiness here is a precondition of the test,
    // not the property under test -- which is what an ordinary inventory
    // reconciliation does to an OPEN job.
    wait_generation(generation);
    let QueryJobOpen::Job(job) = projection.open_query_job(generation) else {
        panic!("initial job");
    };
    let original = projection.query_epoch();
    projection.enqueue_full(
        generation + 1,
        Arc::clone(&pages),
        Arc::clone(&revisions),
        Arc::clone(&config),
        true,
    );
    wait_generation(generation + 1);
    let ordinary_cancelled = job.is_cancelled();
    drop(job);
    assert!(
        !ordinary_cancelled,
        "ordinary inventory reconciliation is not replacement"
    );
    assert_eq!(projection.query_epoch(), original);

    let QueryJobOpen::Job(job) = projection.open_query_job(generation + 1) else {
        panic!("current job");
    };
    let mut changed = (*config).clone();
    changed
        .hidden_properties
        .push("target-config-sentinel".into());
    projection.enqueue_full(generation + 2, pages, revisions, Arc::new(changed), true);
    let started = Instant::now();
    while !job.is_cancelled() && started.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(1));
    }
    let config_cancelled = job.is_cancelled();
    drop(job);
    assert!(
        config_cancelled,
        "config replacement must interrupt existing jobs before writing"
    );
    assert_ne!(projection.query_epoch(), original);
    wait_generation(generation + 2);
    assert!(projection.close_and_wait_for_worker(Duration::from_secs(3)));
    std::fs::remove_dir_all(root).unwrap();
}

/// The cold-open handoff sets `needs_full`. This is the step that makes the
/// next delta-only turn refuse, so it is the step the alarming log line was
/// really reporting.
#[test]
fn current_snapshot_write_failure_recovers_from_authoritative_source() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("target-write-failure");
    let database = root.join("private/projection.sqlite");
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database.clone()).unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let projection = graph.direct_projection_test().unwrap();
    let reported_before = reported_projection_failures_test();
    let entry = graph.list_pages().into_iter().next().unwrap();
    let mut page = graph.load_page(&entry).unwrap();
    let baseline = page.rev.clone();
    let writer = rusqlite::Connection::open(&database).unwrap();
    writer.execute_batch("DROP TABLE block_text").unwrap();
    drop(writer);
    page.blocks[0].raw = "TODO acknowledged source survives failed projection".into();
    graph.save_page(&page, baseline.as_deref()).unwrap();
    let started = Instant::now();
    while (!projection.shared.worker_failed.load(Ordering::Acquire)
        || reported_projection_failures_test() <= reported_before)
        && started.elapsed() < Duration::from_secs(3)
    {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(projection.shared.worker_failed.load(Ordering::Acquire));
    assert!(
        reported_projection_failures_test() > reported_before,
        "a genuine serving-writer failure reaches the always-on failure family"
    );
    assert!(matches!(
        projection.open_current_query_job(RegistrySensitivity::Insensitive),
        QueryJobOpen::NotReady
    ));
    crate::direct_projection::recover_until_ready(&graph);
    let result = graph
        .run_query_bounded("(task TODO)", 100, 1_000_000)
        .unwrap();
    assert!(result
        .groups
        .iter()
        .flat_map(|group| &group.blocks)
        .any(|block| block.raw == page.blocks[0].raw));
    assert!(projection.close_and_wait_for_worker(Duration::from_secs(3)));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn direct_projection_matches_parser_tasks_and_tracks_replace_delete() {
    let _serial = serialize_projection_tests();
    let root = scratch("task-parity");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(
        root.join("pages/tasks.md"),
        "- TODO [#A] parent\n\t- TODO child\n- TODO other\n  SCHEDULED: <2026-08-13 Thu>\n",
    )
    .unwrap();
    std::fs::write(root.join("pages/org.org"), "* TODO [#B] org task\n").unwrap();

    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);

    for query in [
        "(task TODO)",
        "(and (task TODO) (priority A))",
        "(and (task TODO) (scheduled))",
        "(and (task TODO) (sort-by priority desc))",
    ] {
        let oracle = crate::query::run_query_bounded(&graph, query, 100, 1_000_000);
        let indexed = graph
            .run_query_bounded(query, 100, 1_000_000)
            .expect("the ready projection answers the public bounded route");
        assert_eq!(
            signature(&indexed.groups),
            signature(&oracle.groups),
            "{query}"
        );
        assert_eq!(
            (indexed.total, indexed.exceeded),
            (oracle.total, oracle.exceeded)
        );
    }
    // R3: every user query above was answered by the dispatched statement.
    // Every invocation acquires its own coherent snapshot, including
    // repeated filters and queries that differ only in view directives.
    assert_eq!(graph.direct_projection_fallback_reads_test(), 0);
    assert!(graph.direct_projection_statement_reads_test() >= 3);
    let statement_reads = graph.direct_projection_statement_reads_test();
    let repeated = graph
        .run_query_bounded("(task TODO)", 100, 1_000_000)
        .expect("the ready projection answers the public bounded route");
    assert_eq!(
        signature(&repeated.groups),
        signature(&crate::query::run_query_bounded(&graph, "(task TODO)", 100, 1_000_000).groups)
    );
    assert_eq!(
        graph.direct_projection_statement_reads_test(),
        statement_reads + 1,
        "a repeated query must acquire and validate its own snapshot"
    );

    let entry = graph
        .list_pages()
        .into_iter()
        .find(|entry| entry.name == "tasks")
        .unwrap();
    let mut page = graph.load_page(&entry).unwrap();
    let baseline = page.rev.clone();
    page.blocks[0].raw = "DONE [#A] parent".into();
    graph.save_page(&page, baseline.as_deref()).unwrap();
    wait_ready(&graph);
    for query in ["(task TODO)", "(task DONE)"] {
        let oracle = crate::query::run_query_bounded(&graph, query, 100, 1_000_000);
        let indexed = graph
            .run_query_bounded(query, 100, 1_000_000)
            .expect("the ready projection answers the public bounded route");
        assert_eq!(
            signature(&indexed.groups),
            signature(&oracle.groups),
            "{query}"
        );
    }

    graph.delete_page("org", PageKind::Page).unwrap();
    wait_ready(&graph);
    let oracle = crate::query::run_query_bounded(&graph, "(task TODO)", 100, 1_000_000);
    let indexed = graph
        .run_query_bounded("(task TODO)", 100, 1_000_000)
        .expect("the ready projection answers the public bounded route");
    assert_eq!(signature(&indexed.groups), signature(&oracle.groups));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn b4_page_ref_and_property_facets_record_indexed_reads() {
    let _serial = serialize_projection_tests();
    let root = scratch("b4-indexed-reads");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("pages/source.md"),
        "category:: work\ntags:: work\n\n- TODO points to [[Target]]\n  status:: active\n",
    )
    .unwrap();
    std::fs::write(root.join("pages/target.md"), "- target\n").unwrap();
    std::fs::write(root.join("pages/Project___Child.md"), "- namespace child\n").unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(root.join("journals/2026_09_03.md"), "- journal block\n").unwrap();

    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);

    let indexed_before = graph.direct_projection_indexed_reads_test();
    let statements_before = graph.direct_projection_statement_reads_test();
    for query in [
        "(page-ref Target)",
        "(and (page-ref Target) \"points\")",
        "(and \"points\" (page-ref Target))",
    ] {
        let oracle = crate::query::run_query_bounded(&graph, query, 100, 1_000_000);
        let indexed = graph
            .run_query_bounded(query, 100, 1_000_000)
            .expect("the ready projection answers the public bounded route");
        assert_eq!(signature(&indexed.groups), signature(&oracle.groups));
        assert_eq!(
            (indexed.total, indexed.exceeded),
            (oracle.total, oracle.exceeded)
        );
    }
    assert_eq!(
        graph.property_facets(),
        crate::query::property_facets(&graph)
    );
    assert_eq!(
        graph.autocomplete_property_facets_bounded(100, 1_000_000),
        crate::query::autocomplete_property_facets_bounded(&graph, 100, 1_000_000)
    );
    // R3: a user query is answered by the dispatched statement from its own
    // read snapshot — it no longer passes through the indexed page reads.
    assert!(
        graph.direct_projection_statement_reads_test() >= statements_before + 3,
        "PageRef queries must be answered by the dispatched statement"
    );
    assert!(
        graph.direct_projection_indexed_reads_test() >= indexed_before + 2,
        "both property-facet entry points must use the generation-bound SQLite read"
    );

    // **SPEC §5.9's ready shape.** When the projection is ready and the
    // statement lowers, the STATEMENT answers: exactly one dispatched read,
    // no walk, no fallback — and the same answer the walk gives, including
    // `total` and `exceeded`. This replaces the candidate-plan route, which
    // selected a page SUPERSET and then walked it; the statement selects the
    // answer. There is no cost test in front of this and no fourth route:
    // `(journal)` and `"points"` below are deliberately in the list because
    // one is unselective and the other is an unbounded content predicate,
    // and §5.9 routes both to the statement anyway.
    for (query, expected_captures) in [
        ("(and (task TODO) (page source))", 1),
        ("(property status active)", 1),
        ("(page-property category work)", 1),
        ("(page source)", 1),
        ("(namespace Project)", 1),
        ("(journal)", 1),
        ("(and (property status active) (page source))", 1),
        ("(or (page source) (page Target))", 1),
        ("\"points\"", 1),
    ] {
        let oracle = crate::query::run_query_bounded(&graph, query, 100, 1_000_000);
        let statements_before = graph.direct_projection_statement_reads_test();
        let fallback_before = graph.direct_projection_fallback_reads_test();
        graph.reset_direct_projection_candidate_probe_test();
        let actual = graph
            .run_query_bounded(query, 100, 1_000_000)
            .expect("the ready projection answers the public bounded route");
        assert_eq!(
            signature(&actual.groups),
            signature(&oracle.groups),
            "{query}"
        );
        assert_eq!(
            (actual.total, actual.exceeded),
            (oracle.total, oracle.exceeded),
            "{query}"
        );
        assert_eq!(
            graph.direct_projection_statement_reads_test(),
            statements_before + expected_captures,
            "{query}: results and property metadata share one owned snapshot"
        );
        assert_eq!(
            crate::query::full_graph_query_evaluations(),
            0,
            "{query}: production invocation entered the forbidden full-graph evaluator"
        );
        assert_eq!(
            graph.direct_projection_fallback_reads_test(),
            fallback_before,
            "{query}: ready dispatch fell back"
        );
    }

    let statements_before = graph.direct_projection_statement_reads_test();
    let fallback_before = graph.direct_projection_fallback_reads_test();
    graph.reset_direct_projection_candidate_probe_test();
    let empty = graph
        .run_query_bounded("(", 100, 1_000_000)
        .expect("the ready projection answers the public bounded route");
    assert!(empty.groups.is_empty());
    assert_eq!(
        crate::query::full_graph_query_evaluations(),
        0,
        "a refused source must not enter the graph evaluator"
    );
    assert_eq!(
        graph.direct_projection_statement_reads_test(),
        statements_before,
        "a refused source must not run a statement"
    );
    assert_eq!(
        graph.direct_projection_fallback_reads_test(),
        fallback_before,
        "a refused source must not record fallback access"
    );

    // Property-facet navigation keeps its existing parser fallback.
    // Live queries may read a complete committed image even while source
    // readiness is stale; an incomplete recovery image owes readiness.
    let fallback_query = "(and (page-ref Target) (not (page Missing)))";
    let oracle = crate::query::run_query_bounded(&graph, fallback_query, 100, 1_000_000);
    assert!(
        oracle.total > 0,
        "the stale-state query must have a real answer"
    );

    let fallback_before = graph.direct_projection_fallback_reads_test();
    graph.direct_projection_mark_stale_test();
    assert_eq!(
        graph.property_facets(),
        crate::query::property_facets(&graph)
    );
    assert!(
        graph.direct_projection_fallback_reads_test() >= fallback_before + 1,
        "a stale facet read must still record a parser fallback"
    );

    let fallback_before = graph.direct_projection_fallback_reads_test();
    let walks_before = crate::query::full_graph_query_evaluations();
    // The worker may already have caught up, so accept either verdict --
    // but ONLY the two the route is allowed to give.
    match graph.run_query_bounded(fallback_query, 100, 1_000_000) {
        Ok(answer) => assert_eq!(signature(&answer.groups), signature(&oracle.groups)),
        Err(crate::query::QueryExecutionError::NotReady(_)) => {}
        other => panic!("a query needs a coherent SQL answer or readiness, got {other:?}"),
    }
    let fallback = when_ready(|| graph.run_query_bounded(fallback_query, 100, 1_000_000));
    assert_eq!(signature(&fallback.groups), signature(&oracle.groups));
    assert_eq!(
        crate::query::full_graph_query_evaluations(),
        walks_before,
        "a stale public query must not traverse the graph"
    );
    assert_eq!(
        graph.direct_projection_fallback_reads_test(),
        fallback_before,
        "the public query route has no fallback left to record"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_damaged_query_table_is_rebuilt_without_a_source_edit() {
    let _serial = serialize_projection_tests();
    let root = scratch("damaged-query-table");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages/source.md"), "- TODO links [[Target]]\n").unwrap();
    let path = root.join("private/projection.sqlite");
    let graph = Graph::open(&root);
    graph.attach_direct_projection(path.clone()).unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    // Persistently unavailable projection data, unlike a one-shot seam
    // error on a healthy file. Recovery must actually rebuild the cache.
    let damaged = rusqlite::Connection::open(&path).unwrap();
    damaged.execute("DROP TABLE block_path_refs", []).unwrap();
    drop(damaged);
    let query = "(page-ref Target)";
    let oracle = crate::query::run_query_bounded(&graph, query, 100, 1_000_000);
    // RET2: a damaged table is a failed read, so the route repairs once and
    // retries the SAME statement. The repair is asynchronous, so the public
    // answer may be `NotReady(Recovering)` until the rebuild lands — which
    // is the signal the frontend retries on, and never a walked answer.
    let answer = when_ready(|| graph.run_query_bounded(query, 100, 1_000_000));
    assert_eq!(signature(&answer.groups), signature(&oracle.groups));
    wait_ready(&graph);
    let statements_before = graph.direct_projection_statement_reads_test();
    // A different memo key must reach the repaired SQL table.
    let next = "(and (page-ref Target) (task TODO))";
    let oracle = crate::query::run_query_bounded(&graph, next, 100, 1_000_000);
    let answer = graph
        .run_query_bounded(next, 100, 1_000_000)
        .expect("the ready projection answers the public bounded route");
    assert_eq!(signature(&answer.groups), signature(&oracle.groups));
    assert_eq!(
        graph.direct_projection_statement_reads_test(),
        statements_before + 1
    );
    drop(graph);
    let _ = std::fs::remove_dir_all(root);
}

/// R3 (§2B, D-3): a rebuild under a LIVE query job. The worker must
/// interrupt and drain every owned snapshot before it resets the
/// disposable file, and a job admitted before the drain can neither run
/// its statement nor outlive it. In-scope scenario: a torn projection
/// rebuilt while a query is reading it. Also pins the admission answers a
/// dispatch maps: a stale generation is `NotReady` and an unopenable file
/// is `Failed`, and neither consumes a slot.
#[test]
fn a_rebuild_drains_a_live_query_job_before_touching_the_file() {
    let _serial = serialize_projection_tests();
    let root = scratch("rebuild-drains-job");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages/source.md"), "- TODO links [[Target]]\n").unwrap();
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let projection = graph.direct_projection_test().unwrap();
    let generation = graph.cache_generation();

    assert!(matches!(
        projection.open_query_job(generation + 1),
        QueryJobOpen::NotReady
    ));
    assert_eq!(projection.active_query_jobs_test(), 0);
    projection.inject_next_statement_failure();
    assert!(matches!(
        projection.open_query_job(generation),
        QueryJobOpen::Failed
    ));
    assert_eq!(projection.active_query_jobs_test(), 0);

    let QueryJobOpen::Job(mut job) = projection.open_query_job(generation) else {
        panic!("a ready projection admits a job at its generation");
    };
    assert_eq!(projection.active_query_jobs_test(), 1);
    assert!(!job.is_cancelled());
    assert!(
        job.session_pages.contains("pages/source.md"),
        "the captured fresh-build snapshot publishes its live ids"
    );
    let mut rows = 0usize;
    job.snapshot
        .visit_projection_query("SELECT block_id FROM blocks", &[], |_| {
            rows += 1;
            Ok(std::ops::ControlFlow::Continue(()))
        })
        .unwrap();
    assert_eq!(rows, 1, "the owned snapshot reads the projection");

    // Streaming repair waits for reset before producing replacement pages.
    // Hold the reader on this thread while a separate caller requests
    // recovery, as in production (a failed query releases its own job
    // before requesting repair). Otherwise this fixture waits on itself.
    std::thread::scope(|scope| {
        let repair = scope.spawn(|| graph.direct_projection_recover_after_failed_read_test());
        let started = Instant::now();
        while !job.is_cancelled() {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "the rebuild must cancel the live job"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        // The worker waits for the drain: the slot is still held, the
        // projection is not ready, and nothing has been able to reset the file
        // under the pinned read transaction.
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(projection.active_query_jobs_test(), 1);
        assert!(!graph.direct_projection_ready_test());
        let interrupted = job.snapshot.visit_projection_query(
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 200000) \
             SELECT count(*) FROM c",
            &[],
            |_| Ok(std::ops::ControlFlow::Continue(())),
        );
        assert!(
            interrupted.is_err(),
            "a cancelled snapshot cannot run a statement"
        );
        assert!(matches!(
            projection.open_query_job(generation),
            QueryJobOpen::NotReady
        ));

        drop(job);
        repair.join().unwrap();
    });
    wait_ready(&graph);
    assert_eq!(projection.active_query_jobs_test(), 0);
    let generation = graph.cache_generation();
    assert!(matches!(
        projection.open_query_job(generation),
        QueryJobOpen::Job(_)
    ));
    drop(projection);
    drop(graph);
    let _ = std::fs::remove_dir_all(root);
}

/// R3 identity policy: the projection names exactly the pages THIS process
/// lowered — a full snapshot's replacements, each live delta's page, minus
/// deletions — and a warm reopen that reuses the file (R1) starts from the
/// empty set, because those rows' stored ids came from an earlier session
/// and must be answered structurally.
#[test]
fn lowering_measurement_excludes_other_graphs() {
    let _serial = serialize_projection_tests();
    let root = scratch("measurement-owner");
    let other = scratch("measurement-other");
    reset_lowerings(&root);
    for graph_root in [&root, &other] {
        std::fs::create_dir_all(graph_root.join("pages")).unwrap();
        std::fs::write(graph_root.join("pages/one.md"), "- TODO one\n").unwrap();
        let graph = Graph::open(graph_root);
        graph
            .attach_direct_projection(graph_root.join("projection.sqlite"))
            .unwrap();
        graph.warm_cache();
        wait_ready(&graph);
    }
    assert_eq!(lowerings(), 1, "only the measured graph contributes");
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(other);
}

#[test]
fn session_pages_name_exactly_the_pages_this_process_lowered() {
    let _serial = serialize_projection_tests();
    let root = scratch("session-pages");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages/one.md"), "- TODO one\n").unwrap();
    std::fs::write(root.join("pages/two.md"), "- DONE two\n").unwrap();
    let database = scratch("session-pages-db").join("projection.sqlite");
    let ids = |graph: &Graph, names: &[&str]| -> HashSet<String> {
        graph
            .list_pages()
            .into_iter()
            .filter(|entry| names.contains(&entry.name.as_str()))
            .map(|entry| entry.rel_path)
            .collect()
    };

    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        let projection = graph.direct_projection_test().unwrap();
        let before_delete = projection.session_pages_test();
        assert_eq!(*before_delete, ids(&graph, &["one", "two"]));

        let two = ids(&graph, &["two"]);
        graph.delete_page("two", PageKind::Page).unwrap();
        wait_ready(&graph);
        let after_delete = projection.session_pages_test();
        assert_eq!(*after_delete, ids(&graph, &["one"]));
        assert!(after_delete.is_disjoint(&two));
        assert!(!Arc::ptr_eq(&before_delete, &after_delete));
        assert!(
            !before_delete.is_disjoint(&two),
            "copy-on-write must preserve the captured pre-delete membership"
        );

        let entry = graph
            .list_pages()
            .into_iter()
            .find(|entry| entry.name == "one")
            .unwrap();
        let mut page = graph.load_page(&entry).unwrap();
        let baseline = page.rev.clone();
        page.blocks[0].raw = "DONE one".into();
        graph.save_page(&page, baseline.as_deref()).unwrap();
        wait_ready(&graph);
        let after_existing_page_edit = projection.session_pages_test();
        assert_eq!(*after_existing_page_edit, ids(&graph, &["one"]));
        assert!(
            Arc::ptr_eq(&after_delete, &after_existing_page_edit),
            "lowering a page already in this session must reuse the membership Arc"
        );
        release_projection(&graph);
    }
    std::thread::sleep(Duration::from_millis(20));

    std::fs::write(root.join("pages/two.md"), "- DONE two again\n").unwrap();
    reset_lowerings(&root);
    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        assert_eq!(
            lowerings(),
            2,
            "a stale warm builds one complete fresh image"
        );
        let projection = graph.direct_projection_test().unwrap();
        assert_eq!(
            *projection.session_pages_test(),
            ids(&graph, &["one", "two"])
        );
    }
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(database.parent().unwrap());
}

/// **RET2's cancellation shape.** A cancelled snapshot means the caller's
/// own work was withdrawn — the graph is closing, or a drain superseded the
/// request. It is the ONE §5.9 state that must NOT be repaired and must NOT
/// be retried: repairing would schedule a rebuild nobody asked for, and
/// retrying would race the very drain that cancelled the first attempt.
///
/// The fixture matches on the typed error rather than on a bare `is_err`,
/// because `Cancelled` and `Unavailable` differ in exactly the way the
/// frontend acts on: one is silent, the other is shown.
#[test]
fn a_cancelled_query_job_refuses_without_repairing_or_walking() {
    let _serial = serialize_projection_tests();
    let root = scratch("cancelled-query-job");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("pages/tasks.md"),
        "- TODO ship it\n  status:: active\n",
    )
    .unwrap();

    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);

    // Prime the SAME query: even a cache hit must acquire its image and
    // respect cancellation before it can return the shared result.
    let warm = when_ready(|| graph.run_query_bounded("(task TODO)", 100, 1_000_000));
    assert!(warm.total > 0, "the warm-up query must match something");

    // Non-vacuity for the cancelled query itself, from the independent
    // oracle: the refusal below is not a correct empty answer.
    let oracle = crate::query::run_query_bounded(&graph, "(task TODO)", 100, 1_000_000);
    assert!(
        oracle.total > 0,
        "the cancelled query must have a real answer"
    );

    let projection = graph.direct_projection_test().expect("a projection");
    let generation_before = graph.cache_generation();
    let walks_before = crate::query::full_graph_query_evaluations();
    projection.close_query_jobs_test();

    let refused = graph.run_query_bounded("(task TODO)", 100, 1_000_000);
    assert!(
        matches!(refused, Err(crate::query::QueryExecutionError::Cancelled)),
        "a cancelled job is reported as cancelled, not repaired away: {refused:?}"
    );
    assert_eq!(
        crate::query::full_graph_query_evaluations(),
        walks_before,
        "a cancelled query must not traverse the graph"
    );
    // A repair would have marked the projection stale and enqueued a full
    // rebuild. Readiness at the same generation is what proves neither
    // happened.
    assert_eq!(graph.cache_generation(), generation_before);
    assert!(
        graph.direct_projection_ready_test(),
        "cancellation must not schedule a rebuild of a healthy projection"
    );

    let _ = std::fs::remove_dir_all(root);
}

/// A live read may use the committed image while an ordinary edit is
/// still projecting. A subsequent read observes the completed projection.
#[test]
fn a_projection_behind_edits_serves_coherent_sql_then_refreshes() {
    let _serial = serialize_projection_tests();
    let root = scratch("behind-parsed-cache");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages/tasks.md"), "- TODO ship it\n").unwrap();

    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);

    // Non-vacuity: there IS a matching TODO, and the ready projection finds
    // it, so an unready refusal below is not a correct empty answer.
    let answered = when_ready(|| graph.run_query_bounded("(task TODO)", 100, 1_000_000));
    assert!(
        answered.total > 0,
        "the fixture must have something to match"
    );

    // Move the parsed cache ahead of the projection WITHOUT waiting: the
    // save bumps the cache generation and the worker has not caught up.
    let walks_before = crate::query::full_graph_query_evaluations();
    let mut page = graph.load_named("tasks", PageKind::Page).unwrap().unwrap();
    page.blocks[0].raw = "TODO ship it soon".into();
    graph.save_page(&page, page.rev.as_deref()).unwrap();

    // Either committed version is valid; freshness does not gate a read.
    let answer = graph
        .run_query_bounded("(task TODO)", 100, 1_000_000)
        .unwrap();
    assert_eq!(answer.total, 1);
    assert_eq!(answer.groups.len(), 1);
    assert_eq!(answer.groups[0].blocks.len(), 1);
    assert!(matches!(
        answer.groups[0].blocks[0].raw.as_str(),
        "TODO ship it" | "TODO ship it soon"
    ));
    assert_eq!(
        crate::query::full_graph_query_evaluations(),
        walks_before,
        "an unready projection must not traverse the graph"
    );

    // Synchronize the test with producer completion, not query success:
    // a successful current read deliberately makes no freshness promise.
    wait_ready(&graph);
    let after = when_ready(|| graph.run_query_bounded("(task TODO)", 100, 1_000_000));
    assert_eq!(after.total, 1);
    assert_eq!(after.groups[0].blocks[0].raw, "TODO ship it soon");

    let _ = std::fs::remove_dir_all(root);
}

/// **SPEC §5.9's failed-read shape, RET2.** A read that was ATTEMPTED and
/// did not answer owes a repair and a RETRY OF THE SAME STATEMENT — not a
/// walk. The walk is gone from the public route, so the obligation the old
/// shape discharged by falling back is now discharged by
/// `direct_projection_recover_after_failed_read` followed by a second SQL
/// attempt inside the same public call.
///
/// **In-scope scenario** (AGENTS §5): a torn or truncated projection file
/// after a crash or power loss, a disk error, or a projection whose page set
/// has drifted from the parsed cache. The projection is disposable derived
/// state (D-3), so the answer is still recovery and not refusal — the user
/// gets the RIGHT rows, from SQL, without a user edit and without the graph
/// ever being traversed.
#[test]
fn a_failed_statement_read_repairs_and_retries_the_same_statement() {
    let _serial = serialize_projection_tests();
    let root = scratch("failed-read-recovers");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("pages/source.md"),
        "- TODO points to [[Target]]\n  status:: active\n",
    )
    .unwrap();
    std::fs::write(root.join("pages/target.md"), "- target\n").unwrap();

    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);

    let query = "(page-ref Target)";
    let oracle = crate::query::run_query_bounded(&graph, query, 100, 1_000_000);
    assert!(oracle.total > 0, "the fixture must have something to match");
    // The counter that used to prove the failed read: the query route has
    // no fallback left to count, so it must NOT move here. It still counts
    // the property-facet fallback, which RET2 did not touch.
    let fallbacks_before = graph.direct_projection_fallback_reads_test();
    graph.reset_direct_projection_candidate_probe_test();
    graph.direct_projection_inject_read_failure_test();

    let walks_before = crate::query::full_graph_query_evaluations();
    let statements_before_failure = graph.direct_projection_statement_reads_test();

    let answered = when_ready(|| graph.run_query_bounded(query, 100, 1_000_000));
    assert_eq!(
        signature(&answered.groups),
        signature(&oracle.groups),
        "a failed read must be answered by a repaired SQL retry, not refused"
    );
    assert_eq!(
        crate::query::full_graph_query_evaluations(),
        walks_before,
        "the public route never walks the graph, not even to survive a failed read"
    );
    assert_eq!(
        graph.direct_projection_fallback_reads_test(),
        fallbacks_before,
        "the query route has no fallback left to take"
    );
    assert!(
        graph.direct_projection_statement_reads_test() > statements_before_failure,
        "the retry must go through the statement seam again"
    );

    // The recovery obligation: `mark_stale` alone would only clear `ready`
    // and strand the projection. The full-snapshot enqueue is scheduled from
    // the already-parsed cache, so it needs no reparse, no disk read, and no
    // user action — `ready` comes back on its own.
    wait_ready(&graph);
    // A different query exercises result construction after recovery;
    // a same-image cache hit still acquires a snapshot but skips payload.
    let after = "(property status active)";
    let after_oracle = crate::query::run_query_bounded(&graph, after, 100, 1_000_000);
    let statements_before = graph.direct_projection_statement_reads_test();
    let fallbacks_before = graph.direct_projection_fallback_reads_test();
    let recovered = when_ready(|| graph.run_query_bounded(after, 100, 1_000_000));
    assert_eq!(
        signature(&recovered.groups),
        signature(&after_oracle.groups)
    );
    assert_eq!(
        graph.direct_projection_statement_reads_test(),
        statements_before + 1,
        "property metadata and result must share one acquired SQL snapshot"
    );
    assert_eq!(
        graph.direct_projection_fallback_reads_test(),
        fallbacks_before,
        "the recovered projection must not fall back"
    );

    let _ = std::fs::remove_dir_all(root);
}

/// **SPEC §5.3's base order and hydration, together.**
///
/// The selection relation is unordered; its descriptor wrapper orders by
/// persisted page position and block preorder. `signature` compares the
/// ordered page list and each page's ordered block list, so a set-equivalent
/// result with changed traversal order fails this gate.
///
/// Two visible-order paths are covered because they are different paths and
/// a feed-only repro misses real bugs: a routed NAMED page (nested blocks,
/// document order within the page) and the JOURNAL feed (kind rank, journal
/// before page at the same display name).
///
/// Result payload comes from SQLite for admitted rows. No query result page
/// is loaded as a `Document` (I-13, I-15).
#[test]
fn the_dispatched_result_reproduces_the_walks_order_and_loads_only_result_pages() {
    let _serial = serialize_projection_tests();
    let root = scratch("dispatch-order");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    // A routed named page with NESTED matches, so within-page document order
    // is observable: `tree/filter-top-level-blocks` keeps the outer match and
    // the grandchild, and the ordered comparison sees which comes first.
    std::fs::write(
        root.join("pages/Alpha.md"),
        "- TODO alpha one\n\t- plain middle\n\t\t- TODO alpha three\n- TODO alpha four\n",
    )
    .unwrap();
    std::fs::write(root.join("pages/Beta.md"), "- TODO beta one\n").unwrap();
    // Never matches: it must not be hydrated.
    std::fs::write(root.join("pages/Gamma.md"), "- ordinary prose\n").unwrap();
    std::fs::write(
        root.join("journals/2026_06_28.md"),
        "- TODO journal one\n- TODO journal two\n",
    )
    .unwrap();
    std::fs::write(
        root.join("journals/2026_06_29.md"),
        "- TODO journal three\n",
    )
    .unwrap();

    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);

    let feed = "(task TODO)";
    let oracle = crate::query::run_query_bounded(&graph, feed, 100, 1_000_000);
    graph.reset_direct_projection_candidate_probe_test();
    let dispatched = graph
        .run_query_bounded(feed, 100, 1_000_000)
        .expect("the ready projection answers the public bounded route");
    assert_eq!(
        signature(&dispatched.groups),
        signature(&oracle.groups),
        "the journal feed must match the walk INCLUDING order"
    );
    // The fixture has to be able to fail: more than one page, and a page with
    // more than one block, or the ordered comparison proves nothing.
    assert!(
        dispatched.groups.len() >= 4,
        "fixture must span several pages: {:?}",
        dispatched
            .groups
            .iter()
            .map(|g| &g.page)
            .collect::<Vec<_>>()
    );
    assert!(
        dispatched.groups.iter().any(|g| g.blocks.len() > 1),
        "fixture must have a page with several ordered matches"
    );
    assert_eq!(
        crate::query::full_graph_query_evaluations(),
        0,
        "the feed must not enter the whole-graph evaluator"
    );
    // R3 (I-13, I-15): the answer is constructed from the projection alone.
    // NO parsed document is loaded for a dispatched query — not the result
    // pages, and not `Gamma`, which matches nothing.
    let hydrated = graph.direct_projection_hydrated_pages_test();
    assert!(
        hydrated.is_empty(),
        "a dispatched query loads no page document: {hydrated:?}"
    );

    // The routed named-page path: the same query scoped to one page, whose
    // within-page order is document order and not any projection column.
    let routed = "(and (task TODO) (page Alpha))";
    let routed_oracle = crate::query::run_query_bounded(&graph, routed, 100, 1_000_000);
    graph.reset_direct_projection_candidate_probe_test();
    let routed_dispatched = graph
        .run_query_bounded(routed, 100, 1_000_000)
        .expect("the ready projection answers the public bounded route");
    assert_eq!(
        signature(&routed_dispatched.groups),
        signature(&routed_oracle.groups),
        "a routed named page must match the walk INCLUDING order"
    );
    assert_eq!(
        routed_dispatched.groups.len(),
        1,
        "the routed query names exactly one page"
    );
    assert_eq!(
        routed_dispatched.groups[0].blocks.len(),
        3,
        "Alpha contributes the outer match, its grandchild and its sibling, \
             in document order"
    );
    assert!(
        graph.direct_projection_hydrated_pages_test().is_empty(),
        "a routed one-page result loads no page document either"
    );

    let _ = std::fs::remove_dir_all(root);
}

/// **SPEC §5.9: an unselective shape takes the statement too — there is no
/// fourth route.**
///
/// This fixture exists because of the route it USED to prove. The candidate
/// plan materialized a page SUPERSET and then walked it, which on `(journal)`
/// meant materializing most of the graph and running 7–10× slower than the
/// walk; a candidate-count hatch abandoned the projection on exactly that
/// shape. §5.9 removes the reason for the hatch rather than the hatch's
/// symptom: the statement selects the ANSWER, so an unselective shape costs
/// what its answer costs and there is nothing to abandon.
///
/// The obligation the hatch protected is kept as an assertion, not as a
/// route: on the unselective shape the dispatched path must load exactly the
/// RESULT's pages and must not enter the whole-graph evaluator.
#[test]
fn an_unselective_shape_answers_through_the_statement_without_a_candidate_superset() {
    let _serial = serialize_projection_tests();
    let root = scratch("b4-candidate-cutoff");
    std::fs::create_dir_all(root.join("journals")).unwrap();
    // 50 real journal dates, comfortably past the 32-page small-graph
    // floor. Two months, because a date that does not exist (2026-09-31)
    // is not a journal and would not become a candidate.
    for (month, days) in [(9, 30), (10, 20)] {
        for day in 1..=days {
            std::fs::write(
                root.join(format!("journals/2026_{month:02}_{day:02}.md")),
                "- journal block\n",
            )
            .unwrap();
        }
    }
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("pages/source.md"),
        "- TODO points to [[Target]]\n  status:: active\n",
    )
    .unwrap();
    std::fs::write(root.join("pages/target.md"), "- target\n").unwrap();

    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);

    // The shape the retired hatch existed for: `(journal)` matches most of
    // the graph. The fixture is pinned through the ANSWER's own page count
    // so a fixture that stopped being unselective fails here rather than
    // silently testing nothing — the candidate lowering that used to pin it
    // is gone along with the hatch and the walk it fell back to.
    let unselective = "(journal)";

    // Run the oracle BEFORE resetting the probes, so the oracle's own walk
    // is not counted as the production invocation's route evidence.
    let oracle = crate::query::run_query_bounded(&graph, unselective, 500, 4_000_000);
    assert!(
        oracle.groups.len() > 32,
        "fixture must exceed the old cutoff; got {} pages",
        oracle.groups.len()
    );
    let statements_before = graph.direct_projection_statement_reads_test();
    let fallback_before = graph.direct_projection_fallback_reads_test();
    graph.reset_direct_projection_candidate_probe_test();
    let dispatched = graph
        .run_query_bounded(unselective, 500, 4_000_000)
        .expect("the ready projection answers the public bounded route");

    assert_eq!(
        signature(&dispatched.groups),
        signature(&oracle.groups),
        "the statement must answer an unselective shape identically"
    );
    assert_eq!(
        (dispatched.total, dispatched.exceeded),
        (oracle.total, oracle.exceeded),
        "the statement must reproduce the walk's bound outcome"
    );
    assert_eq!(
        graph.direct_projection_statement_reads_test(),
        statements_before + 1,
        "an unselective shape is still answered by exactly one statement"
    );
    assert_eq!(
        graph.direct_projection_fallback_reads_test(),
        fallback_before,
        "a ready projection must not fall back on an unselective shape"
    );
    assert_eq!(
        crate::query::full_graph_query_evaluations(),
        0,
        "an unselective shape must not enter the whole-graph evaluator"
    );
    // **The I-13/I-15 obligation the hatch used to buy with a route.** The
    // dispatched path loads exactly the pages the RESULT names — here every
    // journal, because every journal matches — and never a superset. The
    // number that mattered was "pages materialized that the answer does not
    // contain", and it is zero by construction now.
    assert_eq!(
        dispatched.groups.len(),
        oracle.groups.len(),
        "the dispatched result must name the walk's pages"
    );

    // Same graph, same readiness: a selective shape is the same one route.
    let selective = "(page-ref Target)";
    let selective_oracle = crate::query::run_query_bounded(&graph, selective, 500, 4_000_000);
    let statements_before = graph.direct_projection_statement_reads_test();
    let fallback_before = graph.direct_projection_fallback_reads_test();
    graph.reset_direct_projection_candidate_probe_test();
    let routed = graph
        .run_query_bounded(selective, 500, 4_000_000)
        .expect("the ready projection answers the public bounded route");
    assert_eq!(
        signature(&routed.groups),
        signature(&selective_oracle.groups)
    );
    assert_eq!(
        graph.direct_projection_statement_reads_test(),
        statements_before + 1,
        "a selective shape is answered by exactly one statement"
    );
    assert_eq!(
        graph.direct_projection_fallback_reads_test(),
        fallback_before,
        "a selective shape must not fall back"
    );
    assert_eq!(
        crate::query::full_graph_query_evaluations(),
        0,
        "a selective shape must not enter the full-graph evaluator"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
#[ignore = "manual B4 corpus gate; set TINE_B4_QUERY_CORPUS"]
fn b4_corpus_page_ref_and_facets_match_oracle_with_route_evidence() {
    fn copy_tree(source: &Path, target: &Path) {
        std::fs::create_dir_all(target).unwrap();
        for entry in std::fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            let destination = target.join(entry.file_name());
            if kind.is_dir() {
                copy_tree(&entry.path(), &destination);
            } else if kind.is_file() {
                std::fs::copy(entry.path(), destination).unwrap();
            }
        }
    }

    let _serial = serialize_projection_tests();
    let source = PathBuf::from(
        std::env::var("TINE_B4_QUERY_CORPUS").expect("TINE_B4_QUERY_CORPUS is required"),
    );
    let root = scratch("b4-corpus");
    if source.is_dir() {
        copy_tree(&source, &root);
    } else {
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::copy(&source, root.join("pages/corpus-fixture.md")).unwrap();
    }
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
            root.join("pages/B4 Indexed Source.md"),
            "b4-page-facet:: yes\ntags:: b4-tag\n\n- TODO synthetic [[B4 Indexed Target]]\n  b4-facet:: yes\n",
        )
        .unwrap();
    std::fs::write(
        root.join("pages/B4___Namespace.md"),
        "- synthetic namespace\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(root.join("journals/2026_09_03.md"), "- synthetic journal\n").unwrap();
    std::fs::write(
        root.join("pages/B4 Indexed Target.md"),
        "- synthetic target\n",
    )
    .unwrap();

    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join(".b4-private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);

    // RET2 deleted the candidate-count escape hatch and the parser walk it
    // handed the query back to, so there are no longer two sides to assert.
    // What the real corpus still proves, and no synthetic fixture does at
    // scale, is that EVERY shape below — including `(journal)`, whose
    // candidate set is the size of the journal directory — is answered by
    // one statement, equals the parser oracle row for row, and never
    // reaches the full-graph evaluator.
    let graph_page_count = graph.with_pages(|pages| pages.len());
    let mut answered = 0usize;
    for (query, expected_captures) in [
        ("(page-ref \"B4 Indexed Target\")", 1),
        ("(and (task TODO) (page \"B4 Indexed Source\"))", 1),
        ("(property b4-facet yes)", 1),
        ("(page-property b4-page-facet yes)", 1),
        ("(page \"B4 Indexed Source\")", 1),
        ("(namespace B4)", 1),
        ("(journal)", 1),
        (
            "(and (property b4-facet yes) (page \"B4 Indexed Source\"))",
            1,
        ),
        (
            "(or (page \"B4 Indexed Source\") (page \"B4 Indexed Target\"))",
            1,
        ),
    ] {
        let oracle = crate::query::run_query_bounded(&graph, query, 20_000, 32 * 1024 * 1024);
        let statements_before = graph.direct_projection_statement_reads_test();
        let fallback_before = graph.direct_projection_fallback_reads_test();
        graph.reset_direct_projection_candidate_probe_test();
        let indexed = when_ready(|| graph.run_query_bounded(query, 20_000, 32 * 1024 * 1024));
        assert_eq!(
            signature(&indexed.groups),
            signature(&oracle.groups),
            "{query}: the answer must equal the parser oracle"
        );
        assert_eq!(
            (indexed.total, indexed.exceeded),
            (oracle.total, oracle.exceeded)
        );
        answered += 1;
        assert_eq!(
            graph.direct_projection_statement_reads_test(),
            statements_before + expected_captures,
            "{query}: results and property metadata share one owned snapshot"
        );
        assert_eq!(
            graph.direct_projection_fallback_reads_test(),
            fallback_before,
            "{query}: the query route has no fallback left to take"
        );
        assert_eq!(
            crate::query::full_graph_query_evaluations(),
            0,
            "{query}: the public route must not enter the full-graph evaluator"
        );
        assert!(
            graph.direct_projection_hydrated_pages_test().len() <= indexed.groups.len(),
            "{query}: a dispatched query hydrates only the pages its result names"
        );
    }
    // Non-vacuity: a corpus that answered nothing proves nothing.
    assert_eq!(
        answered, 9,
        "every shape must have been answered (pages={graph_page_count})"
    );
    assert!(
        graph.property_facets() == crate::query::property_facets(&graph),
        "corpus query-builder facets differ from the parser oracle"
    );
    assert!(
        graph.autocomplete_property_facets_bounded(20_000, 32 * 1024 * 1024)
            == crate::query::autocomplete_property_facets_bounded(&graph, 20_000, 32 * 1024 * 1024,),
        "corpus autocomplete facets differ from the parser oracle"
    );
    // A stale projection owes readiness, not a walked answer; the worker
    // catches up on its own and the same statement then answers.
    let fallback_before = graph.direct_projection_fallback_reads_test();
    let walks_before = crate::query::full_graph_query_evaluations();
    graph.direct_projection_mark_stale_test();
    let stale_query = "(and (page-ref \"B4 Indexed Target\") \"synthetic\")";
    let oracle = crate::query::run_query_bounded(&graph, stale_query, 20_000, 32 * 1024 * 1024);
    let recovered = when_ready(|| graph.run_query_bounded(stale_query, 20_000, 32 * 1024 * 1024));
    assert!(
        signature(&recovered.groups) == signature(&oracle.groups),
        "corpus stale recovery differs from the parser oracle"
    );
    assert_eq!(
        crate::query::full_graph_query_evaluations(),
        walks_before,
        "a stale public query must not traverse the graph"
    );
    assert_eq!(
        graph.direct_projection_fallback_reads_test(),
        fallback_before,
        "the query route has no fallback left to record"
    );

    let pages = graph.with_pages(|pages| pages.len());
    println!(
        "b4_corpus_gate pages={pages} indexed_reads={} fallback_reads={}",
        graph.direct_projection_indexed_reads_test(),
        graph.direct_projection_fallback_reads_test()
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn direct_projection_matches_literal_search_and_virtual_reference_names() {
    let _serial = serialize_projection_tests();
    let root = scratch("search-reference-parity");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
            root.join("pages/one.md"),
            "tags:: Page Tag, [[Property Page]]\nalias:: Alias Page\nquoted:: untouched\n\n- Characteristically useful [[Inline Page]]\n  aliases:: #Block Alias\n- c% literal\n",
        )
        .unwrap();
    std::fs::write(root.join("pages/two.md"), "- unrelated content\n").unwrap();
    let graph = Graph::open(&root);
    graph.warm_cache();
    let oracle = crate::query::search_cancellable(&graph, "characteristically", 20, || false);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    wait_ready(&graph);

    let selected = graph.search("characteristically", 20).unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].page, "one");
    assert_eq!(
        signature(&graph.search("characteristically", 20).unwrap()),
        signature(&oracle)
    );
    let names = graph
        .referenced_page_names()
        .into_iter()
        .map(|name| crate::refs::page_key(&name))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        names,
        [
            "page tag",
            "property page",
            "alias page",
            "inline page",
            "block",
            "block alias",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    );
    assert!(graph.direct_projection_referenced_name_reads_test() > 0);

    let name_reads = graph.direct_projection_referenced_name_reads_test();
    graph.direct_projection_mark_stale_test();
    assert_eq!(
        signature(&graph.search("characteristically", 20).unwrap()),
        signature(&oracle)
    );
    assert_eq!(
        graph
            .referenced_page_names()
            .into_iter()
            .map(|name| crate::refs::page_key(&name))
            .collect::<std::collections::BTreeSet<_>>(),
        names
    );
    assert_eq!(
        graph.direct_projection_referenced_name_reads_test(),
        name_reads,
        "a stale generation must not read reference names from SQLite"
    );

    let entry = graph
        .list_pages()
        .into_iter()
        .find(|entry| entry.name == "one")
        .unwrap();
    let mut page = graph.load_page(&entry).unwrap();
    let baseline = page.rev.clone();
    page.blocks[0].raw = "Nothing matching [[Replacement Page]]".into();
    graph.save_page(&page, baseline.as_deref()).unwrap();
    wait_ready(&graph);
    assert!(graph.search("cly", 20).unwrap().is_empty());
    let names = graph
        .referenced_page_names()
        .into_iter()
        .map(|name| crate::refs::page_key(&name))
        .collect::<std::collections::BTreeSet<_>>();
    assert!(names.contains("replacement page"));
    assert!(!names.contains("inline page"));

    std::fs::write(
        root.join("pages/one.md"),
        "tags:: External Tag\n\n- Externally changed fuzzy [[External Page]]\n",
    )
    .unwrap();
    graph.sync_file_checked(&root.join("pages/one.md")).unwrap();
    wait_ready(&graph);
    assert!(!graph.search("externally changed", 20).unwrap().is_empty());
    let names = graph
        .referenced_page_names()
        .into_iter()
        .map(|name| crate::refs::page_key(&name))
        .collect::<std::collections::BTreeSet<_>>();
    assert!(names.contains("external tag"));
    assert!(names.contains("external page"));
    assert!(!names.contains("replacement page"));

    let _ = std::fs::remove_dir_all(root);
}

/// Q9: `[[ ]]` autocomplete and the frontend's own reference-name IPC both ask
/// for this set, and the projection answers it by draining one row per (source
/// page, referenced name) pair. On a 10,000-page graph that is 110,000 rows to
/// yield 10,010 names — 1.29 s of the 1.41 s each autocomplete keystroke cost,
/// paid again on every single call because nothing memoized it.
#[test]
fn referenced_names_are_read_once_per_cache_generation() {
    let _serial = serialize_projection_tests();
    let root = scratch("referenced-names-memo");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages/one.md"), "- links to [[Inline Page]]\n").unwrap();
    let graph = Graph::open(&root);
    graph.warm_cache();
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    wait_ready(&graph);

    let keys = |graph: &Graph| {
        graph
            .referenced_page_names()
            .into_iter()
            .map(|name| crate::refs::page_key(&name))
            .collect::<std::collections::BTreeSet<_>>()
    };

    let first = keys(&graph);
    assert!(first.contains("inline page"));
    let reads = graph.direct_projection_referenced_name_reads_test();
    assert!(reads > 0, "the first call must read the projection");

    // The property the memo exists for: a second call at the same cache
    // generation answers identically WITHOUT draining SQLite again.
    assert_eq!(keys(&graph), first);
    assert_eq!(
        graph.direct_projection_referenced_name_reads_test(),
        reads,
        "a second call at one cache generation must not re-drain the projection"
    );

    // ...and the memo must not outlive the edit that invalidates it: a newly
    // linked page has to reach autocomplete, and the replaced one has to leave.
    let entry = graph
        .list_pages()
        .into_iter()
        .find(|entry| entry.name == "one")
        .unwrap();
    let mut page = graph.load_page(&entry).unwrap();
    let baseline = page.rev.clone();
    page.blocks[0].raw = "links to [[Replacement Page]]".into();
    graph.save_page(&page, baseline.as_deref()).unwrap();
    wait_ready(&graph);

    let after = keys(&graph);
    assert!(
        after.contains("replacement page"),
        "a page linked after the memo was filled must still be offered"
    );
    assert!(!after.contains("inline page"));
    assert!(
        graph.direct_projection_referenced_name_reads_test() > reads,
        "a new cache generation must re-read the projection"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn direct_projection_matches_parser_reference_family_and_stale_fallback() {
    let _serial = serialize_projection_tests();
    let root = scratch("reference-family-parity");
    let target_id = "11111111-2222-4333-8444-555555555555";
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("pages/target.md"),
        format!("alias:: Alias Target, TT\n\n- target\n  id:: {target_id}\n"),
    )
    .unwrap();
    std::fs::write(
            root.join("pages/referrer.md"),
            format!(
                "- [[Alias Target]] and plain Alias Target and plain TT and (({target_id})) (({target_id}))\n- another (({target_id}))\n"
            ),
        )
        .unwrap();
    std::fs::write(root.join("pages/unrelated.md"), "- unrelated\n").unwrap();

    let graph = Graph::open(&root);
    graph.warm_cache();
    let parser_aliases = crate::query::page_aliases_with_owners(&graph);
    let parser_backlinks = crate::query::backlinks(&graph, "target");
    let parser_unlinked = crate::query::unlinked_refs(&graph, "target");
    let parser_referrers = crate::query::block_referrers(&graph, target_id);
    let parser_resolved = crate::query::resolve_block(&graph, target_id);
    let parser_counts = graph.block_ref_counts().unwrap();

    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    wait_ready(&graph);

    assert_eq!(graph.page_aliases_with_owners(), parser_aliases);
    let explicit_candidates = graph.reference_candidate_pages(
        &[
            crate::refs::page_key("target"),
            crate::refs::page_key("Alias Target"),
        ],
        "target",
        ReferenceKind::Explicit,
    );
    assert!(explicit_candidates.indexed);
    assert!(explicit_candidates.pages.len() < explicit_candidates.full_page_count);
    assert_eq!(
        signature(&crate::query::backlinks(&graph, "target")),
        signature(&parser_backlinks)
    );
    assert_eq!(
        signature(&crate::query::unlinked_refs(&graph, "target")),
        signature(&parser_unlinked)
    );
    assert_eq!(
        signature(
            &graph
                .unlinked_refs_bounded_indexed("target", 100, 4 * 1024 * 1024)
                .expect("interactive title/alias windows answer")
                .groups
        ),
        signature(&parser_unlinked),
        "the independently planned long and short alias needles union before exact dedupe"
    );
    assert_eq!(
        signature(&crate::query::block_referrers(&graph, target_id)),
        signature(&parser_referrers)
    );
    assert_eq!(
        crate::query::resolve_block(&graph, target_id)
            .as_ref()
            .map(|group| signature(std::slice::from_ref(group))),
        parser_resolved
            .as_ref()
            .map(|group| signature(std::slice::from_ref(group)))
    );
    assert_eq!(
        graph.block_ref_counts().unwrap().as_ref(),
        parser_counts.as_ref()
    );
    assert_eq!(graph.block_ref_counts().unwrap().get(target_id), Some(&2));

    let custom_path = root.join("pages/custom.md");
    std::fs::write(&custom_path, "- custom identity\n  id:: not-a-uuid\n").unwrap();
    assert!(graph.sync_file(&custom_path).is_some());
    wait_ready(&graph);
    assert_eq!(
        crate::query::resolve_block(&graph, "not-a-uuid")
            .and_then(|group| group.blocks.into_iter().next())
            .map(|block| block.raw),
        Some("custom identity\nid:: not-a-uuid".to_string())
    );

    graph.direct_projection_mark_stale_test();
    assert_eq!(graph.page_aliases_with_owners(), parser_aliases);
    assert_eq!(
        signature(&crate::query::backlinks(&graph, "target")),
        signature(&parser_backlinks)
    );
    assert_eq!(
        signature(&crate::query::block_referrers(&graph, target_id)),
        signature(&parser_referrers)
    );
    assert_eq!(
        graph.block_ref_counts().unwrap().as_ref(),
        parser_counts.as_ref()
    );

    let target_path = root.join("pages/target.md");
    std::fs::write(
        &target_path,
        format!("alias:: Changed Alias\n\n- target\n  id:: {target_id}\n"),
    )
    .unwrap();
    assert!(graph.sync_file(&target_path).is_some());
    wait_ready(&graph);
    let changed_aliases = graph.page_aliases_with_owners();
    assert!(changed_aliases
        .iter()
        .any(|(alias, owner, _)| alias == "Changed Alias" && owner == "target"));
    assert!(!changed_aliases
        .iter()
        .any(|(alias, _, _)| alias == "Alias Target"));

    graph.delete_page("target", PageKind::Page).unwrap();
    wait_ready(&graph);
    assert!(!graph
        .page_aliases_with_owners()
        .iter()
        .any(|(alias, _, _)| alias == "Changed Alias"));
    let _ = std::fs::remove_dir_all(root);
}

/// GH #400. An ordinary edit has already published its parsed page and
/// queued the exact one-page SQLite delta. A reference read which overlaps
/// that short worker turn must not immediately turn into a whole-graph
/// parser scan. Waiting for this already-running bounded delta preserves the
/// same semantics and avoids the reported multi-second fallback.
#[test]
fn a_timed_out_close_retains_resources_until_the_writer_really_exits() {
    let _serial = serialize_projection_tests();
    let root = scratch("worker-resource-lifetime");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages/source.md"), "- before\n").unwrap();
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/query.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let projection = graph.direct_projection_test().unwrap();
    let resource = Arc::new(());
    let weak = Arc::downgrade(&resource);
    projection.retain_worker_resource(resource);
    let (paused_tx, paused_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    *BEFORE_APPLY_PENDING.lock().unwrap() = Some(Box::new(move || {
        paused_tx.send(()).unwrap();
        resume_rx.recv().unwrap();
    }));
    let entry = graph
        .list_pages()
        .into_iter()
        .find(|entry| entry.name == "source")
        .unwrap();
    let mut page = graph.load_page(&entry).unwrap();
    let revision = page.rev.clone();
    page.blocks[0].raw = "after".into();
    graph.save_page(&page, revision.as_deref()).unwrap();
    paused_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let closed = projection.close_and_wait_for_worker(Duration::ZERO);
    let retained = weak.upgrade().is_some();
    // Release the real worker even if an assertion below fails.
    resume_tx.send(()).unwrap();
    assert!(!closed, "the paused writer cannot have finished");
    assert!(
        retained,
        "a wait timeout must not destroy the writer's resources"
    );
    assert!(projection.close_and_wait_for_worker(Duration::from_secs(5)));
    assert!(
        weak.upgrade().is_none(),
        "the exited writer must release resources"
    );
    // Registration after exit must not retain a resource forever.
    let late = Arc::new(());
    let late_weak = Arc::downgrade(&late);
    projection.retain_worker_resource(late);
    assert!(late_weak.upgrade().is_none());
    drop(graph);
    drop(projection);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn reference_lookup_waits_for_an_inflight_one_page_projection_delta() {
    let _serial = serialize_projection_tests();
    let root = scratch("reference-delta-handoff");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages/target.md"), "- target\n").unwrap();
    std::fs::write(root.join("pages/source.md"), "- unrelated\n").unwrap();

    let graph = Arc::new(Graph::open(&root));
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);

    let (worker_paused_tx, worker_paused_rx) = mpsc::channel();
    let (release_worker_tx, release_worker_rx) = mpsc::channel();
    *BEFORE_APPLY_PENDING.lock().unwrap() = Some(Box::new(move || {
        worker_paused_tx.send(()).unwrap();
        release_worker_rx.recv().unwrap();
    }));

    let entry = graph
        .list_pages()
        .into_iter()
        .find(|entry| entry.name == "source")
        .unwrap();
    let mut page = graph.load_page(&entry).unwrap();
    let baseline = page.rev.clone();
    page.blocks[0].raw = "plain target mention".into();
    graph.save_page(&page, baseline.as_deref()).unwrap();
    worker_paused_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("the one-page projection delta reached the worker");

    let reader = Arc::clone(&graph);
    let (result_tx, result_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let candidates = reader.reference_candidate_pages(
            &[crate::refs::page_key("target")],
            "target",
            ReferenceKind::Plain,
        );
        result_tx.send(candidates.indexed).unwrap();
    });

    match result_rx.recv_timeout(Duration::from_millis(100)) {
        Err(mpsc::RecvTimeoutError::Timeout) => {}
        result => {
            let _ = release_worker_tx.send(());
            panic!(
                    "reference lookup escaped to parser fallback before its queued delta completed: {result:?}"
                );
        }
    }
    release_worker_tx.send(()).unwrap();
    assert_eq!(
        result_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        true,
        "the converged lookup must use current indexed candidates"
    );

    let _ = std::fs::remove_dir_all(root);
}

/// A reference read that finds the projection mid-turn has two possible
/// answers, and they are not equally good. Parsing every page in the graph
/// is the OLD one: it is correct, but during the cold-open window it is the
/// whole graph, once per panel, to produce rows the index would have served
/// a moment later. Reporting the state is the new one, because the panel has
/// the same readiness retry a query block has.
///
/// Both policies are asserted here on one held worker, so the difference
/// between them is the subject of the test rather than a claim about it.
#[test]
fn a_working_projection_refuses_an_indexed_reference_read_instead_of_parsing_every_page() {
    let _serial = serialize_projection_tests();
    let root = scratch("reference-working-refusal");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages/target.md"), "- target\n").unwrap();
    for index in 0..6 {
        std::fs::write(
            root.join(format!("pages/source{index}.md")),
            "- unrelated\n",
        )
        .unwrap();
    }

    let graph = Arc::new(Graph::open(&root));
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);

    let (worker_paused_tx, worker_paused_rx) = mpsc::channel();
    let (release_worker_tx, release_worker_rx) = mpsc::channel();
    *BEFORE_APPLY_PENDING.lock().unwrap() = Some(Box::new(move || {
        worker_paused_tx.send(()).unwrap();
        release_worker_rx.recv().unwrap();
    }));

    let entry = graph
        .list_pages()
        .into_iter()
        .find(|entry| entry.name == "source0")
        .unwrap();
    let mut page = graph.load_page(&entry).unwrap();
    let baseline = page.rev.clone();
    page.blocks[0].raw = "[[target]]".into();
    graph.save_page(&page, baseline.as_deref()).unwrap();
    worker_paused_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the one-page projection delta reached the worker");

    let names = [crate::refs::page_key("target")];
    let reader = Arc::clone(&graph);
    let (result_tx, result_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let walked = reader.reference_candidate_pages(&names, "target", ReferenceKind::Explicit);
        let indexed =
            reader.reference_candidate_pages_indexed(&names, "target", ReferenceKind::Explicit);
        result_tx
            .send((
                walked.indexed,
                walked.pages.len(),
                walked.full_page_count,
                indexed.err(),
            ))
            .unwrap();
    });

    let (walk_indexed, walked_pages, full_pages, refusal) = result_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("neither policy may block on the held worker");
    // Release the real worker before any assertion can unwind.
    release_worker_tx.send(()).unwrap();

    assert!(
        !walk_indexed && walked_pages == full_pages && full_pages == 7,
        "the walking policy still answers by parsing every page \
             (indexed={walk_indexed} pages={walked_pages} of {full_pages})"
    );
    assert!(
        matches!(
            refusal,
            Some(crate::query::QueryExecutionError::NotReady(_))
        ),
        "the panel policy must report the working projection, not parse \
             every page: {refusal:?}"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn interactive_plain_reference_recency_is_independent_per_title_and_alias() {
    let _serial = serialize_projection_tests();
    let root = scratch("plain-reference-independent-windows");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let mut body = String::new();
    for ordinal in 0..350 {
        body.push_str(&format!("- primary needle occurrence {ordinal}\n"));
    }
    for ordinal in 0..350 {
        body.push_str(&format!("- authored alias occurrence {ordinal}\n"));
    }
    std::fs::write(root.join("pages/source.md"), body).unwrap();

    let graph = Graph::open(&root);
    graph.warm_cache();
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    wait_ready(&graph);

    let candidates = graph
        .reference_candidate_pages_indexed(
            &[
                crate::refs::page_key("primary needle"),
                crate::refs::page_key("authored alias"),
            ],
            "target owner",
            ReferenceKind::Plain,
        )
        .expect("interactive plain-reference candidates");
    assert_eq!(
        candidates
            .blocks
            .as_ref()
            .map(std::collections::HashSet::len),
        Some(600),
        "each resolved spelling must receive its own 300-match verified window before union"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn interactive_plain_reference_window_verifies_raw_page_preambles() {
    let _serial = serialize_projection_tests();
    let root = scratch("plain-reference-raw-page-preamble");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("pages/plain target source.md"),
        "note:: target\n\n- body\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pages/explicit target source.md"),
        "note:: [[target]]\n\n- body\n",
    )
    .unwrap();
    std::fs::write(root.join("pages/target.md"), "- owner\n").unwrap();
    std::fs::write(
        root.join("pages/plain go source.md"),
        "note:: go\n\n- body\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pages/explicit go source.md"),
        "note:: [[go]]\n\n- body\n",
    )
    .unwrap();
    std::fs::write(root.join("pages/go.md"), "- owner\n").unwrap();

    let graph = Graph::open(&root);
    graph.warm_cache();
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    wait_ready(&graph);

    let groups = graph
        .unlinked_refs_bounded_indexed("target", 100, 4 * 1024 * 1024)
        .expect("interactive raw-preamble reference read")
        .groups;
    assert!(groups
        .iter()
        .any(|group| group.page == "plain target source"));
    assert!(
        groups
            .iter()
            .all(|group| group.page != "explicit target source"),
        "explicit page-property syntax is not an unlinked occurrence: {groups:?}"
    );
    let short_groups = graph
        .unlinked_refs_bounded_indexed("go", 100, 4 * 1024 * 1024)
        .expect("short-scan raw-preamble reference read")
        .groups;
    assert!(short_groups
        .iter()
        .any(|group| group.page == "plain go source"));
    assert!(
        short_groups
            .iter()
            .all(|group| group.page != "explicit go source"),
        "short scans retain the same plain-vs-explicit semantics: {short_groups:?}"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn interactive_plain_reference_window_counts_verified_page_and_block_owners_together() {
    let _serial = serialize_projection_tests();
    let root = scratch("plain-reference-mixed-owner-window");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let window = crate::query::candidate::INTERACTIVE_VERIFIED_WINDOW;
    // These pages satisfy every target trigram but not the exact occurrence.
    // Their paths sort after the real hits, so their entity ids are newer and
    // the candidate cursor must pass more than W false positives first.
    for ordinal in 0..=window {
        std::fs::write(
            root.join("pages")
                .join(format!("zz-false-target-{ordinal:04}.md")),
            "note:: tar arg rge get\n",
        )
        .unwrap();
    }
    // More than W page-level exact matches prove page owners participate in
    // the same bound instead of being appended as an unbounded side channel.
    for ordinal in 0..=window {
        std::fs::write(
            root.join("pages")
                .join(format!("aa-hit-target-{ordinal:04}.md")),
            "note:: target\n",
        )
        .unwrap();
    }
    let block_matches = 25;
    let mut block_source = String::new();
    for ordinal in 0..block_matches {
        block_source.push_str(&format!("- target block {ordinal}\n"));
    }
    std::fs::write(root.join("pages/zz-block-target-source.md"), block_source).unwrap();

    let graph = Graph::open(&root);
    graph.warm_cache();
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    wait_ready(&graph);

    let candidates = graph
        .reference_candidate_pages_indexed(
            &[crate::refs::page_key("target")],
            "target owner",
            ReferenceKind::Plain,
        )
        .expect("mixed page/block verified window");
    assert_eq!(
        candidates.pages.len(),
        window - block_matches + 1,
        "25 block entities share one owner path; the remaining window slots are page entities"
    );
    assert!(candidates
        .pages
        .iter()
        .all(|(entry, _)| entry.name.starts_with("aa-hit-")
            || entry.name == "zz-block-target-source"));
    assert!(candidates
        .pages
        .iter()
        .all(|(entry, _)| !entry.name.starts_with("zz-false-")));
    assert_eq!(
        candidates
            .blocks
            .as_ref()
            .map(std::collections::HashSet::len),
        Some(block_matches)
    );
    assert_eq!(
        candidates
            .page_owners
            .as_ref()
            .map(std::collections::HashSet::len),
        Some(window - block_matches),
        "only page entities that survived the shared page/block window admit preambles"
    );
    let answer = graph
        .unlinked_refs_bounded_indexed("target", window + 10, 16 * 1024 * 1024)
        .expect("mixed page/block reference answer");
    assert_eq!(
        answer
            .groups
            .iter()
            .map(|group| group.blocks.len())
            .sum::<usize>(),
        window,
        "surviving page preambles and blocks must both reach public evidence"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn interactive_short_plain_reference_scan_bounds_exact_callback_work() {
    let _serial = serialize_projection_tests();
    let root = scratch("plain-reference-short-scan-work");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let window = crate::query::candidate::INTERACTIVE_VERIFIED_WINDOW;
    // Each source contributes one qualifying page-preamble entity and one
    // qualifying block entity: more than 2W exact matches for a short needle
    // whose candidate plan must scan rather than use FTS.
    for ordinal in 0..=window {
        std::fs::write(
            root.join("pages").join(format!("source-{ordinal:04}.md")),
            "note:: go\n\n- go\n",
        )
        .unwrap();
    }
    std::fs::write(root.join("pages/go.md"), "- owner\n").unwrap();

    let graph = Graph::open(&root);
    graph.warm_cache();
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    wait_ready(&graph);

    reset_plain_reference_query_instrumentation();
    graph
        .reference_candidate_pages_indexed(
            &[crate::refs::page_key("go")],
            "go",
            ReferenceKind::Plain,
        )
        .expect("short-scan interactive candidates");
    let (callbacks, plans) = plain_reference_query_instrumentation();
    assert_eq!(
        callbacks,
        window + 1,
        "a >2W qualifying scan must do exactly W plus one merge-lookahead callback; plans={plans:?}"
    );
    assert!(
        plans
            .iter()
            .flatten()
            .any(|step| step.contains("MERGE (UNION ALL)")),
        "the production scan statement must be a mergeable top-level compound: {plans:?}"
    );
    assert!(
        plans
            .iter()
            .flatten()
            .all(|step| !step.contains("USE TEMP B-TREE FOR ORDER BY")),
        "the scan must not materialize the whole union before LIMIT: {plans:?}"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn interactive_indexed_plain_reference_keeps_exact_callback_work_bounded() {
    let _serial = serialize_projection_tests();
    let root = scratch("plain-reference-indexed-work");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let window = crate::query::candidate::INTERACTIVE_VERIFIED_WINDOW;
    let mut source = String::new();
    for ordinal in 0..=(window * 2) {
        source.push_str(&format!("- indexed target {ordinal}\n"));
    }
    std::fs::write(root.join("pages/source.md"), source).unwrap();
    std::fs::write(root.join("pages/indexed target.md"), "- owner\n").unwrap();

    let graph = Graph::open(&root);
    graph.warm_cache();
    let projection_path = root.join("private/projection.sqlite");
    graph
        .attach_direct_projection(projection_path.clone())
        .unwrap();
    wait_ready(&graph);
    let analyzer = rusqlite::Connection::open(&projection_path).expect("projection opens");
    analyzer
        .execute_batch("ANALYZE")
        .expect("indexed unlinked fixture is analyzed");
    let statistics: i64 = analyzer
        .query_row("SELECT COUNT(*) FROM sqlite_stat1", [], |row| row.get(0))
        .expect("sqlite_stat1 remains populated");
    assert!(statistics > 0, "the plan fixture needs planner statistics");
    drop(analyzer);

    reset_plain_reference_query_instrumentation();
    graph
        .reference_candidate_pages_indexed(
            &[crate::refs::page_key("indexed target")],
            "indexed target",
            ReferenceKind::Plain,
        )
        .expect("indexed interactive candidates");
    let (callbacks, plans) = plain_reference_query_instrumentation();
    assert_eq!(
        callbacks, window,
        "the indexed candidate cursor must stop exact work at W; plans={plans:?}"
    );
    assert!(
        plans
            .iter()
            .flatten()
            .any(|step| step.contains("search_fts") && step.contains("M1")),
        "the indexed unlinked cursor must actually use the FTS match plan: {plans:?}"
    );
    assert!(
        plans
            .iter()
            .flatten()
            .all(|step| !step.contains("USE TEMP B-TREE FOR ORDER BY")),
        "the indexed candidate cursor must stream FTS rowids before LIMIT: {plans:?}"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn interactive_block_owner_does_not_admit_unwindowed_page_preamble() {
    let _serial = serialize_projection_tests();
    let root = scratch("plain-reference-block-owner-preamble");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let window = crate::query::candidate::INTERACTIVE_VERIFIED_WINDOW;
    let mut source = String::from("note:: target\n\n");
    for ordinal in 0..window {
        source.push_str(&format!("- target body {ordinal}\n"));
    }
    std::fs::write(root.join("pages/source.md"), source).unwrap();
    std::fs::write(root.join("pages/target.md"), "- owner\n").unwrap();

    let graph = Graph::open(&root);
    graph.warm_cache();
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    wait_ready(&graph);

    let answer = graph
        .unlinked_refs_bounded_indexed("target", window + 10, 16 * 1024 * 1024)
        .expect("interactive reference answer");
    let rows = answer
        .groups
        .iter()
        .map(|group| group.blocks.len())
        .sum::<usize>();
    assert_eq!(
        rows, window,
        "a path admitted by W selected blocks must not also admit its unselected preamble"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn interactive_plain_reference_excludes_self_before_window_admission() {
    let _serial = serialize_projection_tests();
    let root = scratch("plain-reference-self-before-window");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("pages/aaa-other.md"),
        "- older eligible target mention\n",
    )
    .unwrap();
    let mut own = String::from("title:: target\n\n");
    for ordinal in 0..=crate::query::candidate::INTERACTIVE_VERIFIED_WINDOW {
        own.push_str(&format!("- newer self-page target mention {ordinal}\n"));
    }
    std::fs::write(root.join("pages/zzz-self.md"), own).unwrap();

    let graph = Graph::open(&root);
    graph.warm_cache();
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    wait_ready(&graph);

    let groups = graph
        .unlinked_refs_bounded_indexed("target", 100, 4 * 1024 * 1024)
        .expect("self-excluded interactive reference read")
        .groups;
    assert_eq!(
        groups.len(),
        1,
        "the self page must not consume W: {groups:?}"
    );
    assert_eq!(groups[0].page, "aaa-other");
    assert_eq!(groups[0].blocks.len(), 1);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn reference_wait_is_zero_cost_when_no_projection_work_exists() {
    let _serial = serialize_projection_tests();
    let root = scratch("reference-no-work-wait");
    let projection = DirectProjection::start(root.join("projection.sqlite")).unwrap();
    let started = Instant::now();
    assert!(!projection.wait_for_reference_generation(1));
    assert!(
        started.elapsed() < Duration::from_millis(50),
        "an unavailable projection must fall back immediately"
    );
    drop(projection);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn direct_projection_preserves_external_uuid_ambiguity_for_parser_resolution() {
    let _serial = serialize_projection_tests();
    let root = scratch("external-uuid-ambiguity");
    let target_id = "11111111-2222-4333-8444-555555555555";
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("pages/alpha.md"),
        format!("- alpha claimant\n  id:: {target_id}\n"),
    )
    .unwrap();
    std::fs::write(
        root.join("pages/beta.md"),
        format!("- beta claimant\n  id:: {target_id}\n"),
    )
    .unwrap();

    let graph = Graph::open(&root);
    graph.warm_cache();
    let parser_resolution = crate::query::resolve_block(&graph, target_id)
        .map(|group| signature(std::slice::from_ref(&group)));
    let projection_path = root.join("private/projection.sqlite");
    graph
        .attach_direct_projection(projection_path.clone())
        .unwrap();
    wait_ready(&graph);

    let database = PhysicalGraphProjectionDatabase::open_read_only(&projection_path).unwrap();
    let claim = Uuid::parse_str(target_id).unwrap().into_bytes();
    assert_eq!(
        database
            .read()
            .blocks_by_logseq_uuid(claim, 2)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        crate::query::resolve_block(&graph, target_id)
            .map(|group| signature(std::slice::from_ref(&group))),
        parser_resolution,
        "SQLite must not choose one external UUID owner from an ambiguous graph"
    );
    drop(database);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn reference_family_has_no_second_in_memory_semantic_index() {
    let model = crate::test_support::model_module_source();
    for removed in [
        "alias_cache",
        "reference_candidate_index",
        "block_ref_count_cache",
        "block_index: RwLock",
    ] {
        assert!(
            !model.contains(removed),
            "Direct Files reference family reintroduced {removed} beside SQLite"
        );
    }
}

#[test]
fn direct_projection_fuzzy_candidates_preserve_parser_corpus_semantics() {
    let _serial = serialize_projection_tests();
    let root = scratch("search-corpus-parity");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
            root.join("pages/search.md"),
            "- Characteristically useful\n  - descendant Needle\n- Café and cafe\u{301}\n- 100% under_score back\\slash\n- MixedCASE\n- x a y b z\n",
        )
        .unwrap();
    std::fs::write(
        root.join("pages/other.md"),
        "- Another characteristically useful result\n",
    )
    .unwrap();
    let cases = [
        ("", 20),
        ("   ", 20),
        ("cly", 20),
        ("needle", 20),
        ("CAFÉ", 20),
        ("cafe\u{301}", 20),
        ("%", 20),
        ("_", 20),
        ("\\", 20),
        ("mixedcase", 20),
        ("xyz", 20),
        ("cly", 1),
    ];
    let oracle_graph = Graph::open(&root);
    oracle_graph.warm_cache();
    let oracle = cases
        .iter()
        .map(|(query, limit)| {
            signature(&crate::query::search_cancellable(
                &oracle_graph,
                query,
                *limit,
                || false,
            ))
        })
        .collect::<Vec<_>>();
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    assert!(
        graph.warm_cache_cancellable(|| false),
        "corpus cache failed to warm: {:?}",
        graph.page_index_failures()
    );
    wait_ready(&graph);
    for ((query, limit), expected) in cases.into_iter().zip(oracle) {
        assert_eq!(
            signature(&graph.search(query, limit).unwrap()),
            expected,
            "{query:?}"
        );
    }
    let cancellation_checks = std::cell::Cell::new(0);
    assert!(crate::query::search_cancellable(&graph, "cly", 20, || {
        cancellation_checks.set(cancellation_checks.get() + 1);
        cancellation_checks.get() > 1
    })
    .is_empty());

    graph.rename_page("search", "renamed search").unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    assert_eq!(
        signature(&graph.search("needle", 20).unwrap()),
        signature(&crate::query::search_cancellable(
            &graph,
            "needle",
            20,
            || false
        ))
    );
    graph.delete_page("renamed search", PageKind::Page).unwrap();
    wait_ready(&graph);
    assert!(graph.search("needle", 20).unwrap().is_empty());

    let _ = std::fs::remove_dir_all(root);
}

/// **RET2's missing-projection shape.** A projection that could not be
/// created at all is not a reason to walk the graph: the public query route
/// refuses with a typed `Unavailable(ProjectionUnavailable)`, which is the
/// bounded signal the frontend surfaces instead of a spinner that never
/// ends. Search shares the typed refusal; the reference-name inventory
/// keeps its navigation semantics unchanged, which
/// is what makes this a query-route claim and not a graph-wide one.
#[test]
fn unavailable_projection_refuses_the_public_query_and_keeps_other_semantics() {
    let _serial = serialize_projection_tests();
    let root = scratch("fallback");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("pages/tasks.md"),
        "- TODO Characteristically readable [[Inline Only]]\n  alias:: #Alias Only\n",
    )
    .unwrap();
    let blocked_parent = root.join("not-a-directory");
    std::fs::write(&blocked_parent, b"ordinary file").unwrap();

    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(blocked_parent.join("projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    std::thread::sleep(Duration::from_millis(30));
    // Non-vacuity: the graph DOES hold a matching TODO, so a refusal here
    // cannot be confused with a correct empty answer.
    let oracle = crate::query::run_query_bounded(&graph, "(task TODO)", 100, 1_000_000);
    assert!(oracle.total > 0, "the fixture must have something to match");

    let walks_before = crate::query::full_graph_query_evaluations();
    let refused = graph.run_query_bounded("(task TODO)", 100, 1_000_000);
    assert!(
        matches!(
            refused,
            Err(crate::query::QueryExecutionError::Unavailable(
                crate::query::QueryUnavailableReason::ProjectionUnavailable
            ))
        ),
        "a projection that could not be created refuses, it does not walk: {refused:?}"
    );
    assert_eq!(
        crate::query::full_graph_query_evaluations(),
        walks_before,
        "the refusal must not traverse the graph"
    );
    assert_eq!(graph.direct_projection_indexed_reads_test(), 0);
    assert!(
        !crate::query::search_cancellable(&graph, "characteristically", 20, || false).is_empty()
    );
    assert!(!graph.search("characteristically", 20).unwrap().is_empty());
    let names = graph
        .referenced_page_names()
        .into_iter()
        .map(|name| crate::refs::page_key(&name))
        .collect::<std::collections::BTreeSet<_>>();
    assert!(names.contains("inline only"));
    assert!(names.contains("alias only"));
    assert_eq!(graph.direct_projection_referenced_name_reads_test(), 0);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn concurrent_graph_instance_cannot_replace_ready_projection_facts() {
    let _serial = serialize_projection_tests();
    let root = scratch("single-writer");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages/tasks.md"), "- TODO one\n").unwrap();
    let database = scratch("single-writer-db").join("projection.sqlite");

    let owner = Graph::open(&root);
    owner.attach_direct_projection(database.clone()).unwrap();
    owner.warm_cache();
    wait_ready(&owner);

    let fallback = Graph::open(&root);
    fallback.attach_direct_projection(database.clone()).unwrap();
    fallback.warm_cache();
    std::thread::sleep(Duration::from_millis(50));
    assert!(
        !fallback.direct_projection_ready_test(),
        "a second graph instance must not publish into the first instance's ready database"
    );
    // Non-vacuity: the graph matches, so the second instance's refusal is
    // not a correct empty answer wearing a hat.
    let oracle = crate::query::run_query_bounded(&fallback, "(task TODO)", 100, 1_000_000);
    assert!(oracle.total > 0, "the fixture must have something to match");
    let walks_before = crate::query::full_graph_query_evaluations();
    let refused = fallback.run_query_bounded("(task TODO)", 100, 1_000_000);
    assert!(
        refused.is_err(),
        "an instance that cannot publish into the owner's database refuses \
             rather than walking: {refused:?}"
    );
    assert_eq!(
        crate::query::full_graph_query_evaluations(),
        walks_before,
        "the refusal must not traverse the graph"
    );
    assert_eq!(fallback.direct_projection_indexed_reads_test(), 0);

    let owner_oracle = crate::query::run_query_bounded(&owner, "(task TODO)", 100, 1_000_000);
    let owner_actual = when_ready(|| owner.run_query_bounded("(task TODO)", 100, 1_000_000));
    assert_eq!(
        signature(&owner_actual.groups),
        signature(&owner_oracle.groups)
    );
    assert!(owner.direct_projection_indexed_reads_test() > 0);

    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(database.parent().unwrap());
}

#[test]
fn coalesced_edits_keep_first_insertion_page_order_and_readds_append() {
    let entry = |name: &str| PageEntry {
        name: name.into(),
        kind: PageKind::Page,
        date_key: None,
        rel_path: format!("pages/{name}.md"),
        path: PathBuf::from(format!("pages/{name}.md")),
    };
    let replacement = |name: &str| PageDelta::Replace {
        entry: entry(name),
        document: Arc::new(crate::doc::parse("- text")),
        revision: "exact-revision".into(),
        parse_config: Arc::new(ParseConfig::default()),
        page_position: None,
    };
    let position = |pending: &PendingProjection, name: &str| match &pending.deltas
        [&format!("pages/{name}.md")]
        .1
    {
        PageDelta::Replace { page_position, .. } => {
            page_position.expect("an ordinary delta carries its position")
        }
        _ => panic!("replacement expected"),
    };
    let mut pending = PendingProjection::default();
    // Before an inventory seeds the order, a delta carries no position:
    // storage keeps a stored page's own (GH #550).
    pending.record_delta(1, replacement("unseeded"));
    assert!(matches!(
        &pending.deltas["pages/unseeded.md"].1,
        PageDelta::Replace {
            page_position: None,
            ..
        }
    ));
    pending.deltas.clear();
    pending.seed_page_order(std::iter::empty::<&str>());
    pending.record_delta(1, replacement("z-first"));
    pending.record_delta(2, replacement("a-second"));
    pending.record_delta(3, replacement("z-first"));
    assert_eq!(position(&pending, "z-first"), 0);
    assert_eq!(position(&pending, "a-second"), 1);
    assert_eq!(pending.deltas.len(), 2, "first page edit is coalesced");
    pending.record_delta(
        4,
        PageDelta::Delete {
            entry: entry("z-first"),
        },
    );
    pending.record_delta(5, replacement("z-first"));
    assert_eq!(position(&pending, "a-second"), 1);
    assert_eq!(position(&pending, "z-first"), 2);
}

/// GH #543: the first query after a reopen must not throw the persisted
/// projection away.
///
/// The app issues queries before the background warm reaches the projection --
/// the journal feed, backlinks, and Ctrl-K all run while `warm_cache_async` is
/// still sleeping its 250 ms. Such a query finds the worker idle, unvalidated
/// and not yet failed, which `progress_at` reported as `Stale`; recovery then
/// latched `pending.rebuild`, and the warm it scheduled rode in on that flag,
/// so the worker `reset()` the database and re-lowered EVERY page. On a small
/// fixture that is invisible; on a real graph it is the whole index rebuilt on
/// every launch, which is why search stayed on "Indexing -- waiting for search
/// to be ready..." for minutes and the app wrote continuously while idle.
///
/// A projection that has not failed is repaired by validating it, never by
/// erasing it.
#[test]
fn a_query_before_the_warm_keeps_a_clean_reopen_clean() {
    let _serial = serialize_projection_tests();
    let root = scratch("reopen-query-first");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages/one.md"), "- TODO one\n").unwrap();
    std::fs::write(root.join("pages/two.md"), "- DONE two\n").unwrap();
    let database = scratch("reopen-query-first-db").join("projection.sqlite");

    reset_lowerings(&root);
    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        assert_eq!(lowerings(), 2, "the first open lowers both pages");
        release_projection(&graph);
    }
    std::thread::sleep(Duration::from_millis(20));

    reset_lowerings(&root);
    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        // Exactly what the app does: a query arrives before the warm.
        let _ = graph.run_query_bounded("(task TODO)", 100, 1_000_000);
        graph.warm_cache();
        wait_ready(&graph);
        assert_eq!(
            lowerings(),
            0,
            "a query before the warm must not re-lower an unchanged graph"
        );
        assert_eq!(
            signature(
                &graph
                    .run_query_bounded("(task TODO)", 100, 1_000_000)
                    .expect("the ready projection answers the public bounded route")
                    .groups
            ),
            signature(
                &crate::query::run_query_bounded(&graph, "(task TODO)", 100, 1_000_000).groups
            )
        );
        release_projection(&graph);
    }

    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(database.parent().unwrap());
}

#[test]
fn a_repair_in_flight_reports_work_in_progress_not_a_stale_projection() {
    let _serial = serialize_projection_tests();
    let root = scratch("repair-in-flight");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages/one.md"), "- TODO one\n").unwrap();
    let database = scratch("repair-in-flight-db").join("projection.sqlite");

    let graph = Graph::open(&root);
    graph.attach_direct_projection(database.clone()).unwrap();
    let projection = graph.direct_projection_test().unwrap();
    let generation = graph.cache_generation();
    assert!(
        matches!(
            projection.progress_at(generation),
            ProjectionProgress::Stale
        ),
        "a fresh unvalidated projection with an empty queue is idle and stale"
    );

    {
        // A repair spends seconds computing its payload on a real graph
        // before it enqueues anything. Only one repair holds
        // `projection_recovery` at a time, so the query that loses that race
        // must still see work in progress and retry -- otherwise it reports
        // the terminal "the query index could not be read" over a projection
        // the thread beside it is repairing perfectly well.
        let _repair = projection.begin_repair();
        assert!(
            matches!(
                projection.progress_at(generation),
                ProjectionProgress::Working(crate::query::QueryReadinessReason::Recovering)
            ),
            "a repair that has not enqueued its payload yet is still progress"
        );
    }

    assert!(
        matches!(
            projection.progress_at(generation),
            ProjectionProgress::Stale
        ),
        "the marker lasts exactly one repair attempt"
    );

    drop(projection);
    release_projection(&graph);
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(database.parent().unwrap());
}

#[test]
fn clean_reopen_reuses_sqlite_and_external_edit_relowers_only_one_page() {
    let _serial = serialize_projection_tests();
    let root = scratch("reopen-revisions");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages/one.md"), "- TODO one\n").unwrap();
    std::fs::write(root.join("pages/two.md"), "- DONE two\n").unwrap();
    let database = scratch("reopen-revisions-db").join("projection.sqlite");

    reset_lowerings(&root);
    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        assert_eq!(lowerings(), 2);
        release_projection(&graph);
    }
    std::thread::sleep(Duration::from_millis(20));

    reset_lowerings(&root);
    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        assert_eq!(lowerings(), 0, "unchanged pages must stay inside SQLite");
        release_projection(&graph);
    }
    std::thread::sleep(Duration::from_millis(20));

    std::fs::write(root.join("pages/one.md"), "- TODO one changed\n").unwrap();
    reset_lowerings(&root);
    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        assert_eq!(
            lowerings(),
            2,
            "a stale warm publishes one complete fresh image"
        );
        assert_eq!(
            signature(
                &graph
                    .run_query_bounded("(task TODO)", 100, 1_000_000)
                    .expect("the ready projection answers the public bounded route")
                    .groups
            ),
            signature(
                &crate::query::run_query_bounded(&graph, "(task TODO)", 100, 1_000_000).groups
            )
        );
    }

    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(database.parent().unwrap());
}

#[test]
fn extractor_version_participates_in_disposable_source_revision() {
    let source = "sha256:unchanged-source";
    let digest = ParseConfig::default().digest();
    let projected = projection_source_revision(source, digest);
    let hex = digest
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(
        projected,
        format!("direct-facts-v2:{hex}:sha256:unchanged-source")
    );
    assert_ne!(projected, source);
}

/// Guard 4, Direct Files half (§5.8 J7). Reconciliation compares only
/// source revisions, so a config edit that changes no file byte must still
/// change the revision it compares -- otherwise every unchanged page keeps
/// rows derived under the old config forever.
#[test]
fn a_parse_config_change_moves_every_source_revision() {
    let source = "sha256:unchanged-source";
    let mut edited = ParseConfig::default();
    edited.separated_by_commas.push("authors".to_owned());
    assert_ne!(ParseConfig::default().digest(), edited.digest());
    assert_ne!(
        projection_source_revision(source, ParseConfig::default().digest()),
        projection_source_revision(source, edited.digest()),
    );
}

/// A staged build's connection is disposable. The resting cache policy must
/// be applied to the WAL/NORMAL writer reopened after publication, and must
/// remain the writer that serves ordinary edit turns.
#[test]
fn a_fresh_build_restores_the_serving_writer_cache_budget() {
    use crate::projection_budget::RESTING_CACHE_FLOOR_BYTES;

    let _serial = serialize_projection_tests();
    let root = r6_graph("serving-writer-cache-budget");
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let projection = graph.direct_projection_test().unwrap();
    assert_eq!(
        projection.serving_writer_cache_budget_test(),
        RESTING_CACHE_FLOOR_BYTES,
        "the reopened serving writer, not the discarded stage, owns the resting budget"
    );

    let entry = graph.list_pages().into_iter().next().unwrap();
    let mut page = graph.load_page(&entry).unwrap();
    let baseline = page.rev.clone();
    page.blocks[0].raw.push_str(" after publication");
    graph.save_page(&page, baseline.as_deref()).unwrap();
    wait_ready(&graph);
    assert_eq!(
        projection.serving_writer_cache_budget_test(),
        RESTING_CACHE_FLOOR_BYTES,
        "ordinary edit turns continue on the budgeted serving writer"
    );
    assert!(projection.close_and_wait_for_worker(Duration::from_secs(3)));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn ordinary_graph_edits_never_run_whole_image_health_checks() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("edit-health-check-locality");
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let projection = graph.direct_projection_test().unwrap();

    let other_root = r6_graph("edit-health-check-isolation-other");
    let other_graph = Graph::open(&other_root);
    other_graph
        .attach_direct_projection(other_root.join("private/projection.sqlite"))
        .unwrap();
    other_graph.warm_cache();
    wait_ready(&other_graph);
    let other_projection = other_graph.direct_projection_test().unwrap();

    projection.reset_projection_health_checks_test();
    other_projection.reset_projection_health_checks_test();

    let (pages, revisions, config) = parsed_snapshot(&other_graph);
    other_projection.enqueue_full(
        other_graph.cache_generation(),
        pages,
        revisions,
        config,
        true,
    );
    wait_ready(&other_graph);
    assert_eq!(
        other_projection.projection_health_checks_test(),
        1,
        "the other Graph performed its complete-image health check"
    );
    assert_eq!(
        projection.projection_health_checks_test(),
        0,
        "another Graph's complete-image health check must not affect this projection's receipt"
    );

    let entries = graph.list_pages();
    for (index, entry) in entries.iter().take(4).enumerate() {
        let mut page = graph.load_page(entry).unwrap();
        let baseline = page.rev.clone();
        page.blocks[0]
            .raw
            .push_str(&format!(" locality edit {index}"));
        graph.save_page(&page, baseline.as_deref()).unwrap();
        wait_ready(&graph);
        assert_eq!(
            projection.projection_health_checks_test(),
            0,
            "one-page and repeated Graph edits must not quick-check the database"
        );
    }

    let (pages, revisions, config) = parsed_snapshot(&graph);
    projection.enqueue_full(graph.cache_generation(), pages, revisions, config, true);
    wait_ready(&graph);
    assert_eq!(
        projection.projection_health_checks_test(),
        1,
        "a complete-image reuse boundary checks integrity exactly once"
    );
    assert!(other_projection.close_and_wait_for_worker(Duration::from_secs(3)));
    assert!(projection.close_and_wait_for_worker(Duration::from_secs(3)));
    std::fs::remove_dir_all(other_root).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

/// **F11.** The parse config travels inside each queued work item, so two
/// replacements coalesced into one worker turn are each lowered and stamped
/// under the config they were queued with -- never under whichever config
/// the last enqueue happened to leave beside the queue, and never under a
/// default that absence could stand in for.
///
/// GH #543: a whole-graph build sizes the writer's SQLite page cache to the
/// text it projects and hands the memory back after the commit, and a build
/// into an empty projection builds its secondary indexes once after the rows.
/// Fail-before: the writer stayed at SQLite's ~2 MiB default (`cache_size
/// = -2000`) through and after the build, and every apply maintained all 35
/// indexes per row.
#[test]
fn each_queued_page_lowers_under_the_config_it_was_queued_with() {
    let _serial = serialize_projection_tests();
    let root = scratch("per-item-parse-config");
    std::fs::create_dir_all(&root).unwrap();
    let mut database =
        PhysicalGraphProjectionDatabase::open_writable(&root.join("projection.sqlite")).unwrap();
    database.initialize_schema().unwrap();

    let default_config = Arc::new(ParseConfig::default());
    let edited_config = Arc::new({
        let mut edited = ParseConfig::default();
        edited.separated_by_commas.push("authors".to_owned());
        edited
    });
    assert_ne!(default_config.digest(), edited_config.digest());

    let queued = |rel_path: &str, parse_config: &Arc<ParseConfig>| {
        (
            rel_path.to_owned(),
            (
                1_u64,
                PageDelta::Replace {
                    entry: PageEntry {
                        name: rel_path.trim_end_matches(".md").to_owned(),
                        kind: PageKind::Page,
                        date_key: None,
                        rel_path: rel_path.to_owned(),
                        path: root.join(rel_path),
                    },
                    document: Arc::new({
                        let mut document = crate::doc::parse("- authors:: ada, grace\n");
                        crate::model::assign_doc_runtime_ids(&mut document.roots, rel_path);
                        document
                    }),
                    revision: format!("sha256:{rel_path}"),
                    parse_config: Arc::clone(parse_config),
                    page_position: Some(u64::from(rel_path == "beta.md")),
                },
            ),
        )
    };
    let deltas = BTreeMap::from([
        queued("alpha.md", &default_config),
        queued("beta.md", &edited_config),
    ]);
    apply_deltas(&mut database, deltas).unwrap();

    let stamped = |alpha: &Arc<ParseConfig>, beta: &Arc<ParseConfig>| {
        database
            .source_delta(&[
                PhysicalGraphProjectionSourceRevision {
                    path: "alpha.md".into(),
                    revision: projection_source_revision("sha256:alpha.md", alpha.digest()),
                },
                PhysicalGraphProjectionSourceRevision {
                    path: "beta.md".into(),
                    revision: projection_source_revision("sha256:beta.md", beta.digest()),
                },
            ])
            .unwrap()
            .replacements
    };
    assert!(
        stamped(&default_config, &edited_config).is_empty(),
        "each page must carry the digest of the config it was queued with"
    );
    // Not vacuous: the two stamps really are distinct, so the assertion
    // above could have failed.
    assert_eq!(
        stamped(&edited_config, &default_config).len(),
        2,
        "swapping the two configs must make both pages stale"
    );
    drop(database);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn storage_contract_names_the_generation_bound_cutover() {
    fn contains_words(haystack: &str, needle: &str) -> bool {
        let normalize = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
        let needle = normalize(needle);
        normalize(haystack).contains(needle.as_str())
    }

    let contract = include_str!("../../../docs/storage-sync-contract.md");
    assert!(contains_words(
        contract,
        "direct-files-projections/<canonical-graph-path-digest>.sqlite"
    ));
    assert!(contains_words(contract, "shared\nproperty-facet rows"));
    assert!(contains_words(
        contract,
        "The production\nquery route selects results through SQL."
    ));
    assert!(contains_words(contract, "literal fuzzy-search candidate"));
    assert!(contains_words(contract, "referenced-page\ninventory"));
    assert!(contains_words(
        contract,
        "retains no separate semantic memo"
    ));
    assert!(contains_words(
        contract,
        "exact current graph cache generation"
    ));
    assert!(contains_words(contract, "Direct fact-extractor version"));
    assert!(contains_words(
        contract,
        "app-private graph-fact projection contains no authority state"
    ));
    assert!(contains_words(
        contract,
        "bounded batches stream through one storage-owned fresh-build transaction"
    ));
    assert!(contains_words(
        contract,
        "creates the secondary indexes exactly once, then applies the captured tail delta"
    ));
    assert!(contains_words(
        contract,
        "Only the resulting closed finalized-stage token can invoke publication"
    ));
    assert!(contains_words(contract, "clean\nreopen lowers none"));
    assert!(contains_words(
        contract,
        "memo of already-shaped frontend result DTOs remains Tine-native"
    ));
    assert!(contains_words(contract, "grants no\n   authority"));
    // R3: the owned-snapshot job contract this file implements.
    assert!(contains_words(
        contract,
        "capacity is acquired before\nthe snapshot"
    ));
    assert!(contains_words(
        contract,
        "the worker drains every job before a rebuild touches the file"
    ));
    assert!(contains_words(
        contract,
        "Cancellation is\na typed dispatch answer, not a failed read"
    ));
    assert!(contains_words(
        contract,
        "Result identity follows who lowered the row"
    ));
    // R6: warm validation and the session-identity ownership rule.
    assert!(contains_words(
        contract,
        "Warm validation from bytes, never from a parsed graph"
    ));
    assert!(contains_words(
        contract,
        "a delta alone never publishes an\ninventory"
    ));
    assert!(contains_words(
        contract,
        "Parsed-cache eviction does not erase the\nsession identity owner"
    ));

    // The routing rule is asserted inside its own section, not anywhere in
    // the document: a whole-document `contains` passes with the sentence
    // parked under an unrelated heading, which is exactly how a contract
    // stops describing the subsystem it claims to describe.
    let heading = "### 1.3 Direct Files disposable graph projection";
    let start = contract.find(heading).expect("Direct projection section");
    let body = &contract[start + heading.len()..];
    let section = body
        .find("\n## ")
        .map_or(body, |end| &body[..end])
        .to_owned();
    // RET2's Direct Files route, and the lifecycle facts a reader has to be
    // able to check without reading the code: which reads a query performs,
    // how every non-answer is classified, what one repair owes, and what a
    // cached result is keyed by.
    for sentence in [
            // One SQL route. The no-walk clause is pinned because weakening it
            // into an availability fallback would restore the retired engine.
            "The Direct public-query route has no production tree-walk fallback.",
            "semantically refused source returns its existing empty or unsupported-report\nanswer before any job or snapshot",
            "`@block`, `@page` and Explain reads enter `dispatch_direct_query`",
            "a refresh enters the same dispatcher\nand query-job owner",
            "There is no cost test and no selectivity hatch in\nfront of this route.",
            "None of these\nbranches evaluates the parsed graph or fabricates an empty success.",
            // Snapshot, metadata and ordered result reads (I-13, I-15).
            "Capacity is acquired before SQLite opens\nthe owned snapshot.",
            "using the actual storage query revision and parse config",
            "This owner is separate from the editor registry",
            "Text-only edits with unchanged metadata reuse the registry without a full or per-key scan",
            "All turn writes and identity publication precede cache revision publication",
            "validating its actual revision even on cache hits",
            "Older coherent reads cannot overwrite newer publications or clear newer dirty keys",
            "Ready query selection\nand result construction load NO `Document`, read NO source text and consult no\nparsed graph.",
            "Recovery source-inventory work is counted separately.",
            "Its descriptor wrapper does: Direct\nblock answers carry `pages.position` and\n`blocks.preorder` and end with `ORDER BY` on those columns",
            "Missing Direct order metadata\nfails the read",
            "Page results carry physical graph-relative `path`",
            "the complete saved sort and `COUNT(*) OVER()` before its row limit",
            "only admitted owners receive property payload reads, in batches of 128",
            "`matched_total`\nreports the complete SQL match count, independent of limits and sampling",
            "remembered once per generation,\nnever once per query",
            // Typed non-answers and the one-repair obligation.
            "Capacity pressure is\n`NotReady(Busy)`.",
            "cancellation is `Cancelled` and schedules no\nrepair",
            "gets at most one\nbounded repair and one SQL retry",
            "a torn or truncated projection file after a crash or power\nloss, a disk error, a resource limit, or a projection whose page set has drifted\nfrom the current graph generation",
            "otherwise recovery validates a\ncomplete source inventory from bytes and then captures one parsed snapshot for\nan unpublished bounded-batch reconstruction",
            "Clearing readiness\nalone would strand the projection until another edit.",
            "Cancellation is excluded\nfrom repair",
            // Answer ownership belongs to the read operation, not a producer cache.
            "Direct simple and advanced queries\nconstruct an operation-scoped answer from their acquired SQLite snapshots.",
            "The producer retains\nno query answers and manages no answer-cache invalidation.",
            // Navigation and Friendly remain separately scoped migration work.
            "Friendly graph\nsearch likewise still ranks and produces evidence from parser-projected blocks",
            "they do not authorize a fallback from the\nsimple, advanced, page, registry, or Explain public-query dispatch",
            // The candidate planner and its selectivity cutoff are oracle-only.
            "Production queries construct no candidate-page plan and apply no\nselectivity cutoff.",
            "semantic empty refusal before any snapshot or registry acquisition",
            "Candidate types and lowering remain test-only\nfor the independent oracle.",
        ] {
            assert!(
                contains_words(&section, sentence),
                "§1.3 must state the required Direct Files query semantics: {sentence}"
            );
        }
}

#[test]
#[ignore = "manual storage packet receipt; set TINE_DIRECT_PROJECTION_CORPUS"]
fn real_corpus_projection_converges_and_matches_task_query() {
    let _serial = serialize_projection_tests();
    let root = PathBuf::from(
        std::env::var("TINE_DIRECT_PROJECTION_CORPUS")
            .expect("TINE_DIRECT_PROJECTION_CORPUS is required"),
    );
    let database = scratch("real-corpus").join("projection.sqlite");
    let oracle_graph = Graph::open(&root);
    oracle_graph.warm_cache();
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database.clone()).unwrap();
    let started = Instant::now();
    graph.warm_cache();
    let warm = started.elapsed();
    wait_ready(&graph);
    let converged = started.elapsed();
    let oracle_started = Instant::now();
    let oracle = crate::query::run_query_bounded(&oracle_graph, "(task TODO)", 20_000, 32 << 20);
    let oracle_elapsed = oracle_started.elapsed();
    let query_started = Instant::now();
    let indexed = graph
        .run_query_bounded("(task TODO)", 20_000, 32 << 20)
        .expect("the ready projection answers the public bounded route");
    let indexed_elapsed = query_started.elapsed();
    assert_eq!(signature(&indexed.groups), signature(&oracle.groups));
    let indexed_reads = graph.direct_projection_indexed_reads_test();
    let memo_started = Instant::now();
    let repeated = graph
        .run_query_bounded("(task TODO)", 20_000, 32 << 20)
        .expect("the ready projection answers the public bounded route");
    let memo_elapsed = memo_started.elapsed();
    assert_eq!(signature(&repeated.groups), signature(&oracle.groups));
    assert_eq!(graph.direct_projection_indexed_reads_test(), indexed_reads);
    let mut fuzzy_indexed = Duration::ZERO;
    let mut fuzzy_oracle = Duration::ZERO;
    for value in ["a", "todo", "http", "2026", "%", "_", "é"] {
        let indexed_started = Instant::now();
        let indexed_search = graph.search(value, 5_000).unwrap();
        fuzzy_indexed += indexed_started.elapsed();
        let oracle_started = Instant::now();
        let oracle_search = crate::query::search_cancellable(&oracle_graph, value, 5_000, || false);
        fuzzy_oracle += oracle_started.elapsed();
        assert_eq!(
            signature(&indexed_search),
            signature(&oracle_search),
            "real-corpus fuzzy search diverged for a bounded probe"
        );
    }
    eprintln!(
        "direct projection fuzzy receipt: indexed_total_ms={} oracle_total_ms={}",
        fuzzy_indexed.as_millis(),
        fuzzy_oracle.as_millis(),
    );
    let normalize_names = |mut names: Vec<String>| {
        names.sort_by_key(|name| crate::refs::page_key(name));
        names
    };
    assert_eq!(
        normalize_names(graph.referenced_page_names()),
        normalize_names(oracle_graph.referenced_page_names()),
        "real-corpus referenced-page inventory diverged"
    );
    assert!(graph.direct_projection_referenced_name_reads_test() > 0);
    let task_candidates = PhysicalGraphProjectionDatabase::open_read_only(&database)
        .unwrap()
        .read()
        .task_candidate_blocks_after("TODO", None, 10_000)
        .unwrap()
        .len();
    eprintln!(
            "direct projection receipt: warm_ms={} projection_total_ms={} oracle_query_us={} indexed_query_us={} repeated_query_us={} pages={} task_candidates={}",
            warm.as_millis(),
            converged.as_millis(),
            oracle_elapsed.as_micros(),
            indexed_elapsed.as_micros(),
            memo_elapsed.as_micros(),
            graph.list_pages().len(),
            task_candidates,
        );
}

// ----- R6: parsed independence (warm reuse, staged cold init, session
// identity). Each test below was RED at 5684c8cb (necessity receipt in
// tine-agents/evidence/qe/r6/).

fn r6_graph(tag: &str) -> PathBuf {
    let root = scratch(tag);
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(root.join("pages/one.md"), "- TODO one [[target]]\n").unwrap();
    std::fs::write(root.join("pages/two.md"), "- DONE two\n").unwrap();
    std::fs::write(
        root.join("pages/target.md"),
        "- target\n  status:: active\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pages/titled.md"),
        "title:: Titled Page\n\n- TODO titled\n",
    )
    .unwrap();
    std::fs::write(root.join("journals/2026_09_06.md"), "- TODO today\n").unwrap();
    root
}

/// `enqueue_warm` hands back the attempt id its verdict is keyed by; a test
/// that queues a validation must wait on that exact id.
fn require_warm(attempt: Option<u64>) -> u64 {
    attempt.expect("the warm validation was refused by the queue")
}

/// The walk order of a warm's sources and retained pages, in that order.
fn walk_order(sources: &[(PageEntry, String)], retained: &[PageEntry]) -> Vec<String> {
    sources
        .iter()
        .map(|(entry, _)| entry.rel_path.clone())
        .chain(retained.iter().map(|entry| entry.rel_path.clone()))
        .collect()
}

fn warm_sources(graph: &Graph) -> (Vec<(PageEntry, String)>, u64) {
    let mut text_bytes = 0u64;
    let sources = graph
        .walk_entries_test()
        .into_iter()
        .map(|entry| {
            let content = std::fs::read_to_string(&entry.path).unwrap();
            text_bytes += content.len() as u64;
            let revision = crate::model::content_rev(&content);
            (entry, revision)
        })
        .collect();
    (sources, text_bytes)
}

fn parsed_snapshot(graph: &Graph) -> (PageSnapshot, PageRevisions, Arc<ParseConfig>) {
    let mut pages = Vec::new();
    let mut revisions = HashMap::new();
    for entry in graph.walk_entries_test() {
        let content = std::fs::read_to_string(&entry.path).unwrap();
        revisions.insert(entry.path.clone(), crate::model::content_rev(&content));
        let mut document = crate::doc::parse(&content);
        crate::model::assign_doc_runtime_ids(&mut document.roots, &entry.rel_path);
        pages.push((entry, Arc::new(document)));
    }
    (
        Arc::new(pages),
        Arc::new(revisions),
        Arc::new(graph.config.parse_config()),
    )
}

fn projection_contains(database: &Path, needle: &str) -> bool {
    let connection = rusqlite::Connection::open(database).unwrap();
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM block_text WHERE instr(content, ?1) != 0)",
            rusqlite::params![needle],
            |row| row.get(0),
        )
        .unwrap()
}

fn entry_signature(entries: &[PageEntry]) -> Vec<(String, String, Option<i64>, String)> {
    let mut signature = entries
        .iter()
        .map(|entry| {
            (
                entry.rel_path.clone(),
                entry.name.clone(),
                format!("{:?}", entry.kind),
                entry.date_key,
            )
        })
        .map(|(rel_path, name, kind, date_key)| (rel_path, name, date_key, kind))
        .collect::<Vec<_>>();
    signature.sort();
    signature
}

/// **R6 §1.** An unchanged reopen validates the projection from file
/// bytes alone: nothing is parsed, nothing is lowered, no parsed cache
/// exists, and every startup consumer — the dispatched query, aliases,
/// block-ref counts, the property registry, `list_pages` — answers from
/// SQL with the cache still absent.
fn friendly_committed_fixture(tag: &str) -> Graph {
    let root = scratch(tag);
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages/Tasks.md"), "- needle\n").unwrap();
    let database = scratch(&format!("{tag}-db")).join("projection.sqlite");
    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        release_projection(&graph);
    }
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database).unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    assert!(!graph.has_parsed_cache_test());
    std::fs::remove_file(root.join("pages/Tasks.md")).unwrap();
    graph
}

#[test]
fn public_friendly_reads_committed_blocks_without_source_documents() {
    let _serial = serialize_projection_tests();
    let graph = friendly_committed_fixture("friendly-current-main");
    let answer = graph.run_graph_search("needle", 0, 10, false).unwrap();
    assert_eq!(answer.hits.len(), 1, "read the committed SQLite block");
    assert!(!graph.has_parsed_cache_test());
}

#[test]
fn public_block_search_reads_committed_blocks_without_source_documents() {
    let _serial = serialize_projection_tests();
    let graph = friendly_committed_fixture("literal-current-main");
    let answer = graph.search("needle", 10).unwrap();
    assert_eq!(answer.len(), 1, "read the committed SQLite group");
    assert_eq!(answer[0].blocks.len(), 1);
    assert!(!graph.has_parsed_cache_test());
}

#[test]
fn public_friendly_unavailable_is_typed_not_empty() {
    use crate::query::{QueryExecutionError, QueryReadinessReason, QueryUnavailableReason};
    let _serial = serialize_projection_tests();
    let missing = Graph::open(scratch("friendly-no-projection"));
    assert!(matches!(
        missing.run_graph_search("needle", 0, 10, false),
        Err(QueryExecutionError::Unavailable(
            QueryUnavailableReason::ProjectionUnavailable
        ))
    ));
    assert!(matches!(
        missing.search("needle", 10),
        Err(QueryExecutionError::Unavailable(
            QueryUnavailableReason::ProjectionUnavailable
        ))
    ));
    let graph = friendly_committed_fixture("friendly-busy");
    let projection = graph.direct_projection_test().unwrap();
    let QueryJobOpen::Job(first) =
        projection.open_current_query_job(RegistrySensitivity::Insensitive)
    else {
        panic!("first job")
    };
    let QueryJobOpen::Job(second) =
        projection.open_current_query_job(RegistrySensitivity::Insensitive)
    else {
        panic!("second job")
    };
    assert!(matches!(
        graph.run_graph_search("needle", 0, 10, false),
        Err(QueryExecutionError::NotReady(QueryReadinessReason::Busy))
    ));
    drop(first);
    drop(second);
    assert_eq!(graph.search("needle", 10).unwrap().len(), 1);
}

#[test]
fn friendly_latest_lane_cancels_rank_payload_and_ancestor_work() {
    let _serial = serialize_projection_tests();
    let graph = Arc::new(friendly_committed_fixture("friendly-rank-supersession"));
    let newer = Arc::clone(&graph);
    crate::query::friendly::set_before_friendly_rank_hook(Some(Box::new(move || {
        assert_eq!(
            newer.search_latest("shared", "needle", 10).unwrap().len(),
            1
        );
    })));
    let old = graph
        .run_graph_search_latest("shared", "needle", 0, 10, false)
        .unwrap();
    assert!(old.cancelled);
    assert!(old.hits.is_empty());
    let newer = Arc::clone(&graph);
    crate::query::friendly::set_before_friendly_rank_hook(Some(Box::new(move || {
        assert_eq!(
            newer
                .run_graph_search_latest("shared", "needle", 0, 10, false)
                .unwrap()
                .hits
                .len(),
            1
        );
    })));
    assert!(graph
        .search_latest("shared", "needle", 10)
        .unwrap()
        .is_empty());
    assert_eq!(
        graph
            .direct_projection_test()
            .unwrap()
            .active_query_jobs_test(),
        0
    );
}

#[test]
fn friendly_independent_lanes_and_edits_do_not_cancel() {
    let _serial = serialize_projection_tests();
    let graph = Arc::new(friendly_committed_fixture("friendly-independent-lanes"));
    let other = Arc::clone(&graph);
    crate::query::friendly::set_before_friendly_rank_hook(Some(Box::new(move || {
        assert_eq!(
            other.search_latest("picker", "needle", 10).unwrap().len(),
            1
        );
    })));
    let answer = graph
        .run_graph_search_latest("switcher", "needle", 0, 10, false)
        .unwrap();
    assert!(!answer.cancelled);
    assert_eq!(answer.hits.len(), 1);
    assert_eq!(
        graph
            .direct_projection_test()
            .unwrap()
            .active_query_jobs_test(),
        0
    );
}

#[test]
fn public_export_reads_committed_subtrees_without_source_documents() {
    let _serial = serialize_projection_tests();
    let root = scratch("export-current-main");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("pages/Tasks.md"),
        "- TODO export root\n\t- exported child\n",
    )
    .unwrap();
    let database = scratch("export-current-main-db").join("projection.sqlite");
    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        release_projection(&graph);
    }
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database).unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    assert!(!graph.has_parsed_cache_test());
    // A later source change has not reached the ordinary projection worker.
    // The export must still answer the coherent committed image it opens.
    std::fs::remove_file(root.join("pages/Tasks.md")).unwrap();
    let specs = [crate::query::QueryExportSpec {
        key: "todo".into(),
        query: "(task TODO)".into(),
        advanced: false,
        simple_dialect: None,
        current_page: None,
    }];
    let answer = graph
        .export_query_subtrees(&specs, 4, 8, 32, 100_000)
        .expect("current main export");
    assert_eq!(answer.results.len(), 1);
    assert_eq!(
        answer.results[0].groups.len(),
        1,
        "export must use the committed SQLite image"
    );
    assert_eq!(answer.results[0].groups[0].blocks.len(), 1);
    assert_eq!(answer.results[0].groups[0].blocks[0].children.len(), 1);
    assert!(
        !graph.has_parsed_cache_test(),
        "export must not hydrate parsed pages"
    );
}

#[test]
fn public_ir_query_on_warm_reopen_uses_sql_without_parsed_cache() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("public-ir-warm");
    let database = scratch("public-ir-warm-db").join("projection.sqlite");
    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        release_projection(&graph);
    }
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database).unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    assert!(!graph.has_parsed_cache_test());
    let (query, view) = crate::query::parse_query_text(
        "(task TODO)",
        crate::query::QueryDialect::Og,
        crate::date::JournalDate::today(),
    );
    let before = graph.direct_projection_statement_reads_test();
    let result = crate::query::run_query_result_ir(
        &graph,
        &query,
        &view,
        crate::query::ir::Bounds {
            max_rows: 100,
            max_bytes: 1_000_000,
        },
        &crate::query::ir::ExecutionContext::default(),
    )
    .expect("the ready projection answers the public IR route");
    assert!(
        result.total > 0,
        "fixture must exercise actual result construction"
    );
    assert_eq!(
        graph.direct_projection_statement_reads_test(),
        before + 1,
        "public query_run must execute SQLite, not the oracle"
    );
    assert!(
        !graph.has_parsed_cache_test(),
        "query_run must not hydrate the graph"
    );
}

#[test]
fn bl1_loaded_runtime_id_can_miss_sql_without_a_parsed_cache() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("bl1-runtime-contract");
    let database = scratch("bl1-runtime-contract-db").join("projection.sqlite");
    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        let mut page = graph.load_by_path("pages/one.md").unwrap().unwrap();
        let baseline = page.rev.clone();
        // A frontend-created runtime identity is persisted in the disposable
        // projection, but not in Markdown as an id:: property.
        let mut inserted = page.blocks[0].clone();
        inserted.id = Uuid::new_v4().to_string();
        inserted.raw = "TODO inserted first".into();
        page.blocks.insert(0, inserted);
        graph.save_page(&page, baseline.as_deref()).unwrap();
        wait_ready(&graph);
        release_projection(&graph);
    }
    std::thread::sleep(Duration::from_millis(20));

    let graph = Graph::open(&root);
    graph.attach_direct_projection(database).unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let page = graph.load_by_path("pages/one.md").unwrap().unwrap();
    let id = &page.blocks[1].id;
    assert!(!graph.has_parsed_cache_test());
    assert_eq!(graph.page_build_parses_test(), 0);
    let projection = graph.direct_projection_test().unwrap();
    assert!(!projection.session_pages_test().contains("pages/one.md"));
    assert_eq!(
        projection.block_page_hint(graph.cache_generation(), id),
        Some(None),
        "the frontend holds a runtime id absent from the current SQL image"
    );
    let resolved = graph.resolve_block(id).expect("today's parser resolves it");
    assert_eq!(resolved.blocks[0].raw, page.blocks[1].raw);
    assert_eq!(graph.page_build_parses_test(), 0);
    assert!(graph.on_demand_parses_test() <= 1);
    release_projection(&graph);
}

#[test]
fn warm_reopen_parses_nothing_and_answers_from_sql() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("warm-reopen");
    let database = scratch("warm-reopen-db").join("projection.sqlite");
    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        release_projection(&graph);
    }
    std::thread::sleep(Duration::from_millis(20));

    reset_lowerings(&root);
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database.clone()).unwrap();
    assert!(graph.warm_cache_cancellable(|| false));
    wait_ready(&graph);
    assert_eq!(lowerings(), 0, "unchanged pages stay inside SQLite");
    assert_eq!(
        graph.page_build_parses_test(),
        0,
        "a warm reopen parses nothing"
    );
    assert_eq!(graph.page_build_parses_test(), 0);
    let projection = graph.direct_projection_test().unwrap();
    assert!(
        !graph.has_parsed_cache_test(),
        "readiness must not require the whole-graph parsed cache"
    );

    let oracle = Graph::open(&root);
    let statements = graph.direct_projection_statement_reads_test();
    let fallbacks = graph.direct_projection_fallback_reads_test();
    let indexed = graph
        .run_query_bounded("(task TODO)", 100, 1_000_000)
        .expect("the ready projection answers the public bounded route");
    assert_eq!(
        signature(&indexed.groups),
        signature(&crate::query::run_query_bounded(&oracle, "(task TODO)", 100, 1_000_000).groups)
    );
    assert_eq!(
        graph.direct_projection_statement_reads_test(),
        statements + 1
    );
    assert_eq!(graph.direct_projection_fallback_reads_test(), fallbacks);

    assert_eq!(
        graph.page_aliases_with_owners(),
        crate::query::page_aliases_with_owners(&oracle)
    );
    assert_eq!(
        *graph.block_ref_counts().unwrap(),
        *oracle.block_ref_counts().unwrap()
    );
    assert_eq!(
        entry_signature(&graph.list_pages()),
        entry_signature(&oracle.list_pages())
    );
    assert_eq!(graph.page_build_parses_test(), 0);
    assert!(
        !graph.has_parsed_cache_test(),
        "no startup consumer may force the whole-graph parse"
    );

    let QueryJobOpen::Job(job) =
        projection.open_current_query_job(RegistrySensitivity::Insensitive)
    else {
        panic!("the clean SQL image admits a pinned reader");
    };
    let epoch = projection.query_epoch();
    assert_eq!(graph.with_pages(|pages| pages.len()), 5);
    assert!(graph.has_parsed_cache_test());
    assert!(graph.direct_projection_ready_test());
    assert!(!job.is_cancelled());
    assert_eq!(projection.query_epoch(), epoch);
    drop(job);

    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(database.parent().unwrap());
}

const BL1_ID: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";

fn bl1_fixture(tag: &str) -> PathBuf {
    let root = r6_graph(tag);
    std::fs::write(
        root.join("pages/target.md"),
        "- target\n  status:: active\n  id:: local-id\n",
    )
    .unwrap();
    std::fs::write(root.join("pages/one.md"), format!(
        "icon:: 🌲\nalias:: forest\n\n- TODO target\n  id:: {BL1_ID}\n  status:: active\n  - child\n    id:: bbbbbbbb-bbbb-cccc-dddd-eeeeeeeeeeee\n- parent template\n  template:: Parent\n  - child template\n    template:: Nested\n    id:: cccccccc-bbbb-cccc-dddd-eeeeeeeeeeee\n- omitted parent\n  template:: Children\n  template-including-parent:: false\n  - inserted child\n    id:: dddddddd-bbbb-cccc-dddd-eeeeeeeeeeee\n")).unwrap();
    std::fs::write(
        root.join("pages/two.md"),
        format!(
            "- ancestor\n  - exact (({BL1_ID}))\n- uppercase (({}))\n",
            BL1_ID.to_uppercase()
        ),
    )
    .unwrap();
    std::fs::write(root.join("journals/2026_09_07.md"), "").unwrap();
    std::fs::write(root.join("journals/2026_09_08.md"), "-\n  status:: empty\n").unwrap();
    std::fs::write(
        root.join("journals/2026_09_09.md"),
        "title:: Sep 10th, 2026\n\n- renamed content\n",
    )
    .unwrap();
    root
}

fn bl1_answer(graph: &Graph, surface: usize) -> serde_json::Value {
    match surface {
        0 => serde_json::to_value(graph.page_icons(&[
            "one".into(),
            "forest".into(),
            "missing".into(),
        ]))
        .unwrap(),
        1 => {
            let ids = vec![
                BL1_ID.into(),
                BL1_ID.into(),
                BL1_ID.to_uppercase(),
                "local-id".into(),
                "missing".into(),
            ];
            // The parser's cross-page hinted HashMap order is unspecified;
            // exercise truncation with duplicate inputs for one physical row.
            let bounded = crate::query::resolve_blocks_bounded(graph, &ids[..3], 1, usize::MAX);
            serde_json::json!([
                graph.resolve_block(BL1_ID),
                graph.resolve_blocks(&ids),
                bounded,
                graph.preview_block(BL1_ID, 8),
                graph.preview_block_with_budget(BL1_ID, 1, 4096)
            ])
        }
        2 => {
            let result = graph.block_referrers_bounded(BL1_ID, 10, 10000);
            serde_json::json!([*result.groups, result.total, result.exceeded])
        }
        3 => serde_json::json!([
            graph.property_facets_bounded(100, 100000),
            graph.autocomplete_property_facets_bounded(100, 100000)
        ]),
        4 => {
            let mut templates = graph.templates();
            templates.sort_by(|a, b| a.name.cmp(&b.name));
            assert_eq!(templates.len(), 3);
            serde_json::to_value(templates).unwrap()
        }
        5 => {
            let mut days = graph.journal_content_days();
            days.sort();
            serde_json::to_value(days).unwrap()
        }
        _ => unreachable!(),
    }
}

#[test]
fn bl1_six_surfaces_answer_from_sql_after_warm_reopen() {
    let _serial = serialize_projection_tests();
    let root = bl1_fixture("bl1-six-ready");
    let database = scratch("bl1-six-ready-db").join("projection.sqlite");
    let oracle = Graph::open(&root);
    let expected = (0..6)
        .map(|surface| bl1_answer(&oracle, surface))
        .collect::<Vec<_>>();
    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        release_projection(&graph);
    }
    let mut violations = Vec::new();
    for (surface, expected) in expected.iter().enumerate() {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        let answer = bl1_answer(&graph, surface);
        let evidence = (
            surface,
            answer == *expected,
            graph.page_build_parses_test(),
            graph.on_demand_parses_test(),
            graph.has_parsed_cache_test(),
        );
        eprintln!(
            "BL1 ready (surface, oracle_equal, whole_parses, on_demand, cache): {evidence:?}"
        );
        if evidence != (surface, true, 0, 0, false) {
            violations.push(evidence);
        }
        release_projection(&graph);
    }
    assert!(violations.is_empty(), "{violations:?}");
}

/// Opening a page publishes it and moves the generation. A derived read that
/// saw the move answers again from the index instead of parsing the graph.
#[test]
fn bl1_six_surfaces_follow_a_page_opened_during_the_read() {
    let _serial = serialize_projection_tests();
    let root = bl1_fixture("bl1-six-moved");
    // A page no derived read touches, so opening it always publishes.
    std::fs::write(root.join("pages/untouched.md"), "- plain\n").unwrap();
    let database = scratch("bl1-six-moved-db").join("projection.sqlite");
    let oracle = Graph::open(&root);
    let expected = (0..6)
        .map(|surface| bl1_answer(&oracle, surface))
        .collect::<Vec<_>>();
    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        release_projection(&graph);
    }
    let mut violations = Vec::new();
    for (surface, expected) in expected.iter().enumerate() {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        graph.open_page_during_next_derived_read_test(Some(root.join("pages/untouched.md")));
        let generation = graph.cache_generation();
        let answer = bl1_answer(&graph, surface);
        let evidence = (
            surface,
            answer == *expected,
            graph.page_build_parses_test(),
            graph.has_parsed_cache_test(),
        );
        let moved = graph.cache_generation() != generation;
        eprintln!(
            "BL1 moved (surface, oracle_equal, whole_parses, cache): {evidence:?} moved={moved}"
        );
        if evidence != (surface, true, 0, false) || !moved {
            violations.push(evidence);
        }
        graph.open_page_during_next_derived_read_test(None);
        release_projection(&graph);
    }
    assert!(violations.is_empty(), "{violations:?}");
}

/// GH #543 (I-13): every index-backed derived read goes through
/// `Graph::indexed_read`, which follows a generation move instead of falling
/// back to the whole-graph parser. A reader calling `wait_for_derived_read`
/// directly re-creates the fallback on every page open during a read. Imitate
/// `direct_projection_page_aliases_with_owners` in `model/direct_query.rs`.
#[test]
fn derived_reads_follow_generation_moves_through_one_front_door() {
    let model = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/model");
    let mut callers = Vec::new();
    for entry in std::fs::read_dir(&model).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|ext| ext == "rs") {
            let source = std::fs::read_to_string(&path).unwrap();
            for (line, text) in source.lines().enumerate() {
                if text.contains("self.wait_for_derived_read(") {
                    callers.push(format!(
                        "{}:{}",
                        path.file_name().unwrap().to_string_lossy(),
                        line + 1
                    ));
                }
            }
        }
    }
    assert_eq!(
        callers.len(),
        1,
        "only Graph::derived_reader (model/derived_reads.rs) may call wait_for_derived_read; use Graph::indexed_read: {callers:?}"
    );
    assert!(callers[0].starts_with("derived_reads.rs:"), "{callers:?}");
}

#[test]
fn bl1_six_surfaces_wait_during_warm_without_parsing() {
    let _serial = serialize_projection_tests();
    let root = bl1_fixture("bl1-six-warming");
    let database = scratch("bl1-six-warming-db").join("projection.sqlite");
    let oracle = Graph::open(&root);
    let expected = (0..6)
        .map(|surface| bl1_answer(&oracle, surface))
        .collect::<Vec<_>>();
    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        release_projection(&graph);
    }
    let graph = Arc::new(Graph::open(&root));
    graph.attach_direct_projection(database).unwrap();
    let pause = graph.pause_next_warm_after_read_test();
    let warm = {
        let graph = Arc::clone(&graph);
        std::thread::spawn(move || graph.warm_cache())
    };
    pause.reached.wait();
    let reads = (0..6)
        .map(|surface| {
            let graph = Arc::clone(&graph);
            std::thread::spawn(move || bl1_answer(&graph, surface))
        })
        .collect::<Vec<_>>();
    std::thread::sleep(Duration::from_millis(600));
    let premature = reads.iter().any(|read| read.is_finished());
    let parses = graph.page_build_parses_test();
    pause.release.wait();
    warm.join().unwrap();
    for (surface, read) in reads.into_iter().enumerate() {
        assert_eq!(read.join().unwrap(), expected[surface], "surface {surface}");
    }
    assert!(!premature, "every surface must wait for warm validation");
    assert_eq!(parses, 0);
    assert_eq!(graph.page_build_parses_test(), 0);
    assert_eq!(graph.on_demand_parses_test(), 0);
    release_projection(&graph);
}

#[test]
fn bl1_sql_preview_runtime_ids_remain_locatable() {
    let _serial = serialize_projection_tests();
    let root = bl1_fixture("bl1-preview-runtime");
    let page_path = root.join("pages/one.md");
    let source = std::fs::read_to_string(&page_path).unwrap();
    std::fs::write(
        &page_path,
        source.replace("    id:: bbbbbbbb-bbbb-cccc-dddd-eeeeeeeeeeee\n", ""),
    )
    .unwrap();
    let database = scratch("bl1-preview-runtime-db").join("projection.sqlite");
    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        let mut page = graph.load_by_path("pages/one.md").unwrap().unwrap();
        let baseline = page.rev.clone();
        let mut inserted = page.blocks[0].clone();
        inserted.id = Uuid::new_v4().to_string();
        inserted.raw = "inserted before target".into();
        inserted.children.clear();
        page.blocks.insert(0, inserted);
        graph.save_page(&page, baseline.as_deref()).unwrap();
        wait_ready(&graph);
        release_projection(&graph);
    }
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database).unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let preview = graph.preview_block(BL1_ID, 8).unwrap();
    assert_eq!(graph.on_demand_parses_test(), 0);
    let child = &preview.group.blocks[0].children[0];
    assert!(!graph
        .direct_projection_test()
        .unwrap()
        .session_pages_test()
        .contains("pages/one.md"));
    let resolved = graph.resolve_block(&child.id).unwrap();
    assert_eq!(resolved.blocks[0].raw, child.raw);
    assert_eq!(graph.page_build_parses_test(), 0);
    assert_eq!(graph.on_demand_parses_test(), 1);
    // Exact revision failure cannot resurrect the old runtime id or trigger a
    // graph walk, even before a watcher has reported the changed file.
    std::fs::write(root.join("pages/one.md"), "- externally replaced\n").unwrap();
    assert!(graph.resolve_block(&child.id).is_none());
    assert_eq!(graph.page_build_parses_test(), 0);
    assert_eq!(graph.on_demand_parses_test(), 1);
    release_projection(&graph);
}

#[test]
fn query_registry_snapshot_preserves_old_reads_without_publishing_over_new_edits() {
    let _serial = serialize_projection_tests();
    let root = scratch("registry-snapshot");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let source = root.join("pages/Source.md");
    std::fs::write(&source, "score:: 1\n- TODO task\n  score:: 2\n").unwrap();
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let projection = graph.direct_projection_test().unwrap();
    let old_generation = graph.cache_generation();
    let QueryJobOpen::Job(mut old_job) = projection.open_query_job(old_generation) else {
        panic!("initial snapshot must be ready");
    };
    let config = graph.config.parse_config();
    let old = old_job.read_registry(&config).unwrap();
    assert!(!old.rows().is_empty());
    let legacy = graph.property_registry();
    assert!(
        old.rows_equal(&legacy),
        "the shared inference producer must agree"
    );

    std::fs::write(&source, "score:: word\n- TODO task\n  score:: another\n").unwrap();
    assert!(graph.sync_file_checked(&source).unwrap().is_some());
    wait_ready(&graph);
    let new_generation = graph.cache_generation();
    assert_ne!(old_generation, new_generation);
    let QueryJobOpen::Job(mut new_job) = projection.open_query_job(new_generation) else {
        panic!("updated snapshot must be ready");
    };
    let new = graph.query_property_registry_at(&mut new_job).unwrap();
    assert!(!old.rows_equal(&new), "the property type changed");
    let retained = graph.query_property_registry_at(&mut old_job).unwrap();
    assert!(
        old.rows_equal(&retained),
        "later edits preserve the acquired read"
    );
    drop(new_job);
    let QueryJobOpen::Job(mut current_job) = projection.open_query_job(new_generation) else {
        panic!("current snapshot must remain ready");
    };
    assert!(
        Arc::ptr_eq(&new, &current_job.read_registry(&config).unwrap()),
        "old readers cannot overwrite the committed registry cache"
    );
    assert!(
        graph.has_parsed_cache_test(),
        "the fresh-build snapshot remains the graph's captured parsed cache"
    );
}

#[test]
fn query_registry_live_text_edits_reuse_cache_and_property_edits_patch_it() {
    let _serial = serialize_projection_tests();
    let root = scratch("registry-live-delta");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages/Source.md"), "score:: 1\n- TODO task\n").unwrap();
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let projection = graph.direct_projection_test().unwrap();
    let config = graph.config.parse_config();
    let read = || {
        let QueryJobOpen::Job(mut job) = projection.open_query_job(graph.cache_generation()) else {
            panic!("ready query snapshot");
        };
        let registry = job.read_registry(&config).unwrap();
        (job, registry)
    };
    let (job, initial) = read();
    drop(job);
    assert!(
        Arc::ptr_eq(
            &initial,
            &graph
                .query_property_registry_current(graph.cache_generation())
                .unwrap()
        ),
        "the public registry is the committed owner's"
    );
    let entry = graph
        .list_pages()
        .into_iter()
        .find(|entry| entry.name == "Source")
        .unwrap();
    let mut page = graph.load_page(&entry).unwrap();
    let baseline = page.rev.clone();
    page.blocks[0].raw = "DONE task".into();
    graph.save_page(&page, baseline.as_deref()).unwrap();
    wait_ready(&graph);
    registry_sql::reset_statement_count();
    let (job, text_edit) = read();
    assert!(Arc::ptr_eq(&initial, &text_edit));
    assert_eq!(
        registry_sql::statement_count(),
        0,
        "text-only edits perform no full or per-key registry SQL scan"
    );
    drop(job);

    let mut page = graph.load_page(&entry).unwrap();
    let baseline = page.rev.clone();
    page.pre_block = Some("score:: word\n".into());
    graph.save_page(&page, baseline.as_deref()).unwrap();
    wait_ready(&graph);
    registry_sql::reset_statement_count();
    let (mut job, property_edit) = read();
    assert_eq!(
        registry_sql::statement_count(),
        2,
        "one key patch reads properties and declarations"
    );
    assert!(!initial.rows_equal(&property_edit));
    let full = registry_sql::read_registry(&mut job.snapshot, &config).unwrap();
    assert!(property_edit.rows_equal(&full));
    assert!(property_edit.generation() > initial.generation());
}

#[test]
fn query_registry_snapshot_rejects_orphaned_property_owners() {
    use crate::query::{QueryExecutionError, QueryUnavailableReason};
    let _serial = serialize_projection_tests();
    let root = scratch("registry-orphan");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages/Source.md"), "- TODO task\n  score:: 2\n").unwrap();
    let graph = Graph::open(&root);
    let database = root.join("private/projection.sqlite");
    graph.attach_direct_projection(database.clone()).unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let had_cache = graph.has_parsed_cache_test();
    let damage = rusqlite::Connection::open(&database).unwrap();
    damage.execute("PRAGMA foreign_keys = OFF", []).unwrap();
    damage.execute("DELETE FROM blocks", []).unwrap();
    drop(damage);
    let projection = graph.direct_projection_test().unwrap();
    // The typed physical cursor rejects the impossible owner before the
    // registry adapter can publish a partial row stream.
    assert!(
        projection
            .property_owner_rows(graph.cache_generation())
            .is_none(),
        "an orphaned integer owner must fail the physical registry read"
    );
    let QueryJobOpen::Job(mut job) = projection.open_query_job(graph.cache_generation()) else {
        panic!("the schema still opens before corrupt ownership is inspected");
    };
    assert!(matches!(
        job.read_registry(&graph.config.parse_config()),
        Err(QueryExecutionError::Unavailable(
            QueryUnavailableReason::InvalidSnapshot
        ))
    ));
    assert_eq!(graph.has_parsed_cache_test(), had_cache);
}

/// The not-ready autocomplete fallback reads a page's own properties with
/// the projection's grammar: an Org page's drawer is offered before
/// readiness exactly as after it, and a `key::` line inside a Markdown
/// preamble's code fence is content in both answers.
#[test]
fn autocomplete_fallback_reads_page_properties_like_the_projection() {
    let _serial = serialize_projection_tests();
    let root = scratch("autocomplete-page-properties");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    for (name, text) in [
        ("Plan.org", ":PROPERTIES:\n:owner: alice\n:END:\n* task\n"),
        ("Notes.md", "status:: draft\n\n- block\n"),
        ("Fence.md", "```\nfenced:: content\n```\n\n- block\n"),
    ] {
        std::fs::write(root.join("pages").join(name), text).unwrap();
    }
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);

    let ready = graph.autocomplete_property_facets_bounded(100, 1_000_000);
    assert!(
        ready.0.iter().any(|(key, _)| key == "owner"),
        "the projection reads the Org page drawer: {ready:?}"
    );
    assert_eq!(
        crate::query::autocomplete_property_facets_bounded(&graph, 100, 1_000_000),
        ready,
        "the not-ready fallback must offer what the ready projection offers"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn task_query_does_not_refresh_property_registry() {
    let _serial = serialize_projection_tests();
    let root = scratch("task-no-registry");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages/Source.md"), "- TODO task\n  score:: 2\n").unwrap();
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let had_cache = graph.has_parsed_cache_test();
    take_registry_read_attempts();
    let result = graph
        .run_query_bounded("(task TODO)", 100, 1_000_000)
        .expect("the ready projection answers the public bounded route");
    assert_eq!(result.total, 1);
    assert_eq!(
        take_registry_read_attempts(),
        0,
        "a task query must not scan property metadata to check its memo"
    );
    assert_eq!(graph.has_parsed_cache_test(), had_cache);
}

#[test]
fn task_query_skips_registry_capture_after_many_dirty_property_keys() {
    let _serial = serialize_projection_tests();
    let root = scratch("task-no-registry-capture");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages/Source.md"), "seed:: 1\n- TODO task\n").unwrap();
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let projection = graph.direct_projection_test().unwrap();

    let initial = graph
        .query_registry_snapshot_ready()
        .expect("the first public registry read publishes a base");
    assert!(initial.rows.iter().any(|row| row.normalized_name == "seed"));
    projection.take_registry_capture_attempts();

    let entry = graph
        .list_pages()
        .into_iter()
        .find(|entry| entry.name == "Source")
        .unwrap();
    let mut page = graph.load_page(&entry).unwrap();
    let baseline = page.rev.clone();
    page.pre_block = Some(
        (0..64)
            .map(|index| format!("key-{index:03}:: {index}"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    );
    graph.save_page(&page, baseline.as_deref()).unwrap();
    wait_ready(&graph);
    projection.take_registry_capture_attempts();
    let had_cache = graph.has_parsed_cache_test();

    let result = graph
        .run_query_bounded("(task TODO)", 100, 1_000_000)
        .expect("the non-property public query remains available");
    assert_eq!(result.total, 1);
    assert_eq!(
        projection.take_registry_capture_attempts(),
        0,
        "a non-property query must not clone accumulated registry dirtiness"
    );

    let QueryJobOpen::Job(mut registry_free) =
        projection.open_query_job_for(graph.cache_generation(), RegistrySensitivity::Insensitive)
    else {
        panic!("the ready projection admits a registry-free job");
    };
    assert!(matches!(
        registry_free.read_registry(&graph.config.parse_config()),
        Err(crate::query::QueryExecutionError::Unavailable(
            crate::query::QueryUnavailableReason::InvalidSnapshot
        ))
    ));
    drop(registry_free);

    let current = graph
        .query_registry_snapshot_ready()
        .expect("the public registry still captures and patches the same snapshot");
    assert_eq!(projection.take_registry_capture_attempts(), 1);
    let changed = current
        .rows
        .iter()
        .find(|row| row.normalized_name == "key-031")
        .expect("the patched registry contains the new property key");
    assert_eq!(
        changed.observed_type,
        crate::query::ir::ObservedType::Number
    );
    assert!(!current.rows.iter().any(|row| row.normalized_name == "seed"));
    assert_eq!(graph.has_parsed_cache_test(), had_cache);
}

/// **R6 §1, cold.** A fresh projection streams its build: every page is
/// lowered, no parsed cache is retained, and never more than
/// `WARM_STREAM_HIGH_WATER` documents wait in the queue.
#[test]
fn publication_reader_uses_capture_ids_without_changing_live_editor_ids() {
    let _serial = serialize_projection_tests();
    let root = scratch("publication-capture-ids");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let path = root.join("pages/Source.md");
    std::fs::write(&path, "- TODO keep me\n").unwrap();
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let entry = graph.list_pages().into_iter().next().unwrap();
    let mut page = graph.load_page(&entry).unwrap();
    let old_id = page.blocks[0].id.clone();
    let baseline = page.rev.clone();
    let mut inserted = page.blocks[0].clone();
    inserted.id = uuid::Uuid::new_v4().to_string();
    inserted.raw = "Inserted heading".into();
    inserted.marker = None;
    page.blocks.insert(0, inserted);
    graph.save_page(&page, baseline.as_deref()).unwrap();
    wait_ready(&graph);
    assert_eq!(graph.load_page(&entry).unwrap().blocks[1].id, old_id);
    let had_cache = graph.has_parsed_cache_test();
    let content = std::fs::read_to_string(&path).unwrap();
    let mut captured = crate::doc::parse(&content);
    crate::model::assign_doc_runtime_ids(&mut captured.roots, &entry.rel_path);
    let captured_id = captured.roots[1].uuid.clone();
    assert_ne!(
        captured_id, old_id,
        "the structural edit makes identity policies distinguishable"
    );
    let sources = vec![(entry.clone(), crate::model::content_rev(&content))];
    let (query, view) = crate::query::parse_query_text(
        "(task TODO)",
        crate::query::QueryDialect::Og,
        crate::date::JournalDate::today(),
    );
    let result = graph
        .with_publication_query_reader(&sources, |reader| {
            reader
                .run(
                    &query,
                    &view,
                    crate::query::ir::Bounds {
                        max_rows: 100,
                        max_bytes: 1_000_000,
                    },
                    &crate::query::ir::ExecutionContext::none(),
                )
                .map_err(std::io::Error::other)
        })
        .unwrap();
    let crate::query::ir::QueryRows::Block { groups } = result.rows else {
        panic!("block publication result")
    };
    assert_eq!(groups[0].blocks[0].id, captured_id);
    assert_eq!(graph.load_page(&entry).unwrap().blocks[1].id, old_id);
    assert_eq!(graph.has_parsed_cache_test(), had_cache);

    let mut mismatched = sources;
    mismatched[0].1 = crate::model::content_rev("different captured bytes");
    let called = std::cell::Cell::new(false);
    let refused = graph
        .with_publication_query_reader(&mismatched, |_| {
            called.set(true);
            Ok(())
        })
        .unwrap_err();
    assert_eq!(refused.kind(), std::io::ErrorKind::WouldBlock);
    assert!(
        !called.get(),
        "source mismatch cannot enter the publication writer"
    );
}

#[test]
fn publication_sources_are_compared_inside_the_owned_main_snapshot() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("publication-source-snapshot");
    let database = scratch("publication-source-snapshot-db").join("projection.sqlite");
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database.clone()).unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let sources = graph
        .list_pages()
        .into_iter()
        .map(|entry| {
            let revision =
                crate::model::content_rev(&std::fs::read_to_string(&entry.path).unwrap());
            (entry, revision)
        })
        .collect::<Vec<_>>();
    let config = graph.config.parse_config();
    let projection = graph.direct_projection_test().unwrap();
    let QueryJobOpen::Job(mut old) =
        projection.open_current_query_job(RegistrySensitivity::Insensitive)
    else {
        panic!("old publication image");
    };
    assert!(old.publication_sources_match(&sources, &config).unwrap());
    let mut other_config = config.clone();
    other_config
        .separated_by_commas
        .push("publication-fixture".into());
    assert!(!old
        .publication_sources_match(&sources, &other_config)
        .unwrap());
    assert!(!old
        .publication_sources_match(&sources[1..], &config)
        .unwrap());
    let mut extra = sources.clone();
    extra[0].0.rel_path = "pages/not-captured.md".into();
    assert!(!old.publication_sources_match(&extra, &config).unwrap());

    let path = root.join("pages/two.md");
    let updated = "- TODO changed after publication capture\n";
    std::fs::write(&path, updated).unwrap();
    assert!(graph.sync_file(&path).is_some());
    wait_ready(&graph);
    let mut fresh = sources.clone();
    fresh
        .iter_mut()
        .find(|(entry, _)| entry.path == path)
        .unwrap()
        .1 = crate::model::content_rev(updated);
    assert!(
        old.publication_sources_match(&sources, &config).unwrap(),
        "a later commit cannot mix the captured image"
    );
    assert!(!old.publication_sources_match(&fresh, &config).unwrap());
    let QueryJobOpen::Job(mut current) =
        projection.open_current_query_job(RegistrySensitivity::Insensitive)
    else {
        panic!("current publication image");
    };
    assert!(current.publication_sources_match(&fresh, &config).unwrap());
    assert!(!current
        .publication_sources_match(&sources, &config)
        .unwrap());
    current.snapshot.cancellation().cancel();
    assert!(matches!(
        current.publication_sources_match(&fresh, &config),
        Err(crate::query::results::ResultReadError::Cancelled)
    ));
    drop(current);
    drop(old);

    let writer = rusqlite::Connection::open(&database).unwrap();
    writer
        .execute("DELETE FROM direct_source_revisions", [])
        .unwrap();
    drop(writer);
    let QueryJobOpen::Job(mut damaged) =
        projection.open_current_query_job(RegistrySensitivity::Insensitive)
    else {
        panic!("damaged source-metadata image");
    };
    assert!(matches!(
        damaged.publication_sources_match(&fresh, &config),
        Err(crate::query::results::ResultReadError::Corrupt(_))
    ));
}

#[test]
fn external_watcher_edit_after_warm_reopen_enqueues_a_page_delta() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("external-watcher-no-cache");
    let database = scratch("external-watcher-no-cache-db").join("projection.sqlite");
    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        release_projection(&graph);
    }
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database).unwrap();
    assert!(graph.warm_cache_cancellable(|| false));
    wait_ready(&graph);
    assert!(!graph.has_parsed_cache_test());
    reset_lowerings(&root);

    let path = root.join("pages/two.md");
    std::fs::write(&path, "- TODO external watcher changed\n").unwrap();
    assert!(
        graph.sync_file(&path).is_some(),
        "the watcher admits external bytes even without a parsed cache"
    );
    wait_ready(&graph);
    let answer = graph
        .run_query_bounded("(task TODO)", 100, 1_000_000)
        .expect("ordinary delta progression updates the current main query");
    assert!(answer
        .groups
        .iter()
        .flat_map(|g| &g.blocks)
        .any(|block| block.raw.contains("external watcher changed")));
    assert_eq!(lowerings(), 1, "only the externally edited page is lowered");
    assert!(
        !graph.has_parsed_cache_test(),
        "the watcher does not warm the graph"
    );
    assert!(
        graph.sync_file(&path).is_none(),
        "unchanged bytes remain a no-op"
    );
    assert_eq!(lowerings(), 1);
    let added = root.join("pages/External-new.md");
    std::fs::write(&added, "- TODO external watcher created\n").unwrap();
    assert!(graph.sync_file(&added).is_some());
    wait_ready(&graph);
    let answer = graph
        .run_query_bounded("(task TODO)", 100, 1_000_000)
        .unwrap();
    assert!(answer
        .groups
        .iter()
        .flat_map(|g| &g.blocks)
        .any(|block| block.raw.contains("external watcher created")));
    assert_eq!(lowerings(), 2);
    std::fs::remove_file(&added).unwrap();
    graph.forget_file(&added);
    wait_ready(&graph);
    let answer = graph
        .run_query_bounded("(task TODO)", 100, 1_000_000)
        .unwrap();
    assert!(!answer
        .groups
        .iter()
        .flat_map(|g| &g.blocks)
        .any(|block| block.raw.contains("external watcher created")));
    assert!(!graph.has_parsed_cache_test());
}

/// **R6 §1, pages changed between sessions** (Martin, 2026-09-22). Syncthing
/// delivers an edit, a journal written on the phone, a deletion while Tine is
/// closed: exactly those pages are parsed and relowered in one transaction
/// and the image is validated again. It used to be rebuilt from a parse of
/// the whole graph, 20 s on a 10,000-page graph for one changed page.
#[test]
fn pages_changed_between_sessions_are_repaired_page_by_page() {
    let _serial = serialize_projection_tests();
    for case in ["edited", "added", "deleted", "all-three"] {
        let root = r6_graph(&format!("between-sessions-{case}"));
        for index in 0..12 {
            std::fs::write(
                root.join("pages").join(format!("bulk-{index:02}.md")),
                format!("- TODO bulk {index}\n"),
            )
            .unwrap();
        }
        let database = scratch(&format!("between-sessions-{case}-db")).join("projection.sqlite");
        {
            let graph = Graph::open(&root);
            graph.attach_direct_projection(database.clone()).unwrap();
            graph.warm_cache();
            wait_ready(&graph);
            release_projection(&graph);
        }
        std::thread::sleep(Duration::from_millis(20));
        let edit = matches!(case, "edited" | "all-three");
        let add = matches!(case, "added" | "all-three");
        let delete = matches!(case, "deleted" | "all-three");
        if edit {
            std::fs::write(root.join("pages/two.md"), "- TODO two changed\n").unwrap();
        }
        if add {
            // Sorts into the middle of the walk, not after it.
            std::fs::write(
                root.join("pages/bulk-05a.md"),
                "- TODO added while closed\n",
            )
            .unwrap();
        }
        if delete {
            std::fs::remove_file(root.join("pages/bulk-03.md")).unwrap();
        }

        reset_lowerings(&root);
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        assert!(graph.warm_cache_cancellable(|| false));
        wait_ready(&graph);
        let changed = usize::from(edit) + usize::from(add);
        assert_eq!(
            graph.page_build_parses_test(),
            0,
            "{case}: a whole-graph pass ran"
        );
        assert!(!graph.has_parsed_cache_test(), "{case}");
        assert_eq!(graph.warm_repair_parses_test(), changed, "{case}");
        assert_eq!(
            lowerings(),
            changed as u64,
            "{case}: only the changed pages are relowered"
        );
        let expect = |graph: &Graph| {
            let oracle = Graph::open(&root);
            assert_eq!(
                signature(
                    &graph
                        .run_query_bounded("(task TODO)", 100, 1_000_000)
                        .expect("the ready projection answers")
                        .groups
                ),
                signature(
                    &crate::query::run_query_bounded(&oracle, "(task TODO)", 100, 1_000_000).groups
                ),
                "{case}: the index must answer what the files say"
            );
        };
        expect(&graph);

        // Positions stay coherent: an ordinary edit to a page walked after
        // the added or deleted one lands without a failure or a rebuild.
        let failures = reported_projection_failures_test();
        let later = root.join("pages/bulk-11.md");
        std::fs::write(&later, "- TODO bulk 11 edited after the repair\n").unwrap();
        graph
            .load_page(&graph.entry_for_path(&later).unwrap())
            .unwrap();
        wait_ready(&graph);
        assert_eq!(reported_projection_failures_test(), failures, "{case}");
        expect(&graph);
        release_projection(&graph);
        drop(graph);

        // And the repaired image reopens clean.
        std::thread::sleep(Duration::from_millis(20));
        reset_lowerings(&root);
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        assert!(graph.warm_cache_cancellable(|| false));
        wait_ready(&graph);
        assert_eq!(graph.page_build_parses_test(), 0, "{case}: reopen");
        assert_eq!(graph.warm_repair_parses_test(), 0, "{case}: reopen");
        assert_eq!(lowerings(), 0, "{case}: reopen");
        release_projection(&graph);
        drop(graph);
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(database.parent().unwrap());
    }
}

/// Past a quarter of the graph, a stale image is rebuilt from one complete
/// parsed snapshot instead of being rewritten in place.
#[test]
fn many_pages_changed_between_sessions_build_one_complete_fresh_image() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("external-edit-stream");
    let database = scratch("external-edit-stream-db").join("projection.sqlite");
    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        release_projection(&graph);
    }
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(root.join("pages/two.md"), "- TODO two changed\n").unwrap();
    std::fs::write(root.join("pages/one.md"), "- TODO one changed\n").unwrap();

    reset_lowerings(&root);
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database.clone()).unwrap();
    assert!(graph.warm_cache_cancellable(|| false));
    wait_ready(&graph);
    assert_eq!(lowerings(), 5);
    assert_eq!(graph.page_build_parses_test(), 5);
    assert_eq!(graph.warm_repair_parses_test(), 0);
    assert!(graph.has_parsed_cache_test());
    let oracle = Graph::open(&root);
    let indexed = graph
        .run_query_bounded("(task TODO)", 100, 1_000_000)
        .expect("the ready projection answers the public bounded route");
    assert_eq!(
        signature(&indexed.groups),
        signature(&crate::query::run_query_bounded(&oracle, "(task TODO)", 100, 1_000_000).groups)
    );
    assert!(indexed.groups.iter().any(|group| group
        .blocks
        .iter()
        .any(|block| block.raw.contains("two changed"))));
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(database.parent().unwrap());
}

/// A projection whose schema is damaged is replaced from one complete parsed
/// snapshot; no damaged active file is repaired in place.
#[test]
fn a_damaged_projection_gets_one_complete_fresh_build() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("damaged-stream");
    let database = scratch("damaged-stream-db").join("projection.sqlite");
    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        release_projection(&graph);
    }
    std::thread::sleep(Duration::from_millis(20));
    // Schema damage (a dropped fact table) is the in-scope shape: the open
    // route recreates the file, so the warm meets an empty projection.
    let damaged = rusqlite::Connection::open(&database).unwrap();
    damaged.execute("DROP TABLE block_path_refs", []).unwrap();
    drop(damaged);

    reset_lowerings(&root);
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database.clone()).unwrap();
    assert!(graph.warm_cache_cancellable(|| false));
    wait_ready(&graph);
    assert_eq!(
        lowerings(),
        5,
        "every page is relowered into the recreated file"
    );
    assert_eq!(graph.page_build_parses_test(), 5);
    assert!(graph.has_parsed_cache_test());
    let oracle = Graph::open(&root);
    assert_eq!(
        signature(
            &graph
                .run_query_bounded("(task TODO)", 100, 1_000_000)
                .expect("the ready projection answers the public bounded route")
                .groups
        ),
        signature(&crate::query::run_query_bounded(&oracle, "(task TODO)", 100, 1_000_000).groups)
    );
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(database.parent().unwrap());
}

#[test]
fn edited_page_reload_and_sql_keep_the_same_session_ids() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("edited-session-reload");
    let database = scratch("edited-session-reload-db").join("projection.sqlite");
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database).unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let entry = graph
        .list_pages()
        .into_iter()
        .find(|entry| entry.name == "one")
        .unwrap();
    let mut page = graph.load_page(&entry).unwrap();
    let baseline = page.rev.clone();
    let kept = page.blocks[0].clone();
    let mut inserted = kept.clone();
    inserted.id = Uuid::new_v4().to_string();
    inserted.raw = "TODO inserted first".into();
    page.blocks.insert(0, inserted);
    graph.save_page(&page, baseline.as_deref()).unwrap();
    wait_ready(&graph);
    assert!(graph.has_parsed_cache_test());
    let live = graph
        .run_query_bounded("(task TODO)", 100, 1_000_000)
        .expect("the ready projection answers the public bounded route");
    let query_id = &live
        .groups
        .iter()
        .flat_map(|group| &group.blocks)
        .find(|block| block.raw == kept.raw)
        .unwrap()
        .id;
    assert_eq!(query_id, &kept.id);
    let reloaded = graph.load_by_path(&entry.rel_path).unwrap().unwrap();
    let reloaded_id = &reloaded
        .blocks
        .iter()
        .find(|block| block.raw == kept.raw)
        .unwrap()
        .id;
    assert_eq!(
        reloaded_id, query_id,
        "reloading an unchanged edited page must retain the IDs SQLite exposes"
    );

    // An incompatible external revision must use that revision's parser
    // identities, even if it happens to have the same tree shape.
    let changed = std::fs::read_to_string(&entry.path)
        .unwrap()
        .replace("inserted first", "external first");
    std::fs::write(&entry.path, changed).unwrap();
    graph.sync_file_checked(&entry.path).unwrap();
    // Force a captured full snapshot for the incompatible external revision.
    graph.invalidate_cache();
    graph.warm_cache();
    wait_ready(&graph);
    let external = graph.load_by_path(&entry.rel_path).unwrap().unwrap();
    let external_id = &external
        .blocks
        .iter()
        .find(|block| block.raw == kept.raw)
        .unwrap()
        .id;
    assert_ne!(
        external_id, &kept.id,
        "incompatible source does not reuse the old map"
    );
    let sql = graph
        .run_query_bounded("(task TODO)", 100, 1_000_000)
        .expect("the ready projection answers the public bounded route");
    assert_eq!(
        &sql.groups
            .iter()
            .flat_map(|group| &group.blocks)
            .find(|block| block.raw == kept.raw)
            .unwrap()
            .id,
        external_id
    );
}

#[test]
fn failed_projection_recovery_uses_one_captured_parsed_snapshot() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("streamed-damage-recovery");
    let database = scratch("streamed-damage-recovery-db").join("projection.sqlite");
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database.clone()).unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    assert!(graph.has_parsed_cache_test());
    assert!(graph
        .create_markdown_page_if_absent("aaa-added", "- TODO appended\n")
        .unwrap());
    wait_ready(&graph);
    let query = "(and (task TODO) (not (journal)))";
    // Admission happens before output sorting. A new page appends to this
    // session even when its filename sorts before the existing pages.
    let expected = graph
        .run_query_bounded(query, 1, 1_000_000)
        .expect("the ready projection answers the public bounded route");
    assert_ne!(expected.groups[0].page, "aaa-added");
    // Page creation may have warmed another feature's cache. Evict it
    // before repair so this gate measures recovery's own source ownership.
    graph.invalidate_cache();
    assert!(!graph.has_parsed_cache_test());
    let writer = rusqlite::Connection::open(&database).unwrap();
    writer.execute_batch("DROP TABLE block_text").unwrap();
    drop(writer);
    crate::direct_projection::recover_until_ready(&graph);
    assert!(graph.has_parsed_cache_test());
    let before = graph.direct_projection_statement_reads_test();
    let actual = graph
        .run_query_bounded(query, 1, 1_000_000)
        .expect("the ready projection answers the public bounded route");
    assert_eq!(graph.direct_projection_statement_reads_test(), before + 1);
    assert_eq!(signature(&actual.groups), signature(&expected.groups));
}

#[test]
fn failed_projection_writer_can_recover_from_source_inventory() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("failed-writer-stream-recovery");
    let database = scratch("failed-writer-stream-recovery-db").join("projection.sqlite");
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database.clone()).unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let entry = graph
        .list_pages()
        .into_iter()
        .find(|entry| entry.name == "one")
        .unwrap();
    let mut page = graph.load_page(&entry).unwrap();
    let baseline = page.rev.clone();
    page.blocks[0].raw = "TODO repaired from acknowledged edit".into();
    let kept_id = page.blocks[0].id.clone();
    let writer = rusqlite::Connection::open(&database).unwrap();
    writer.execute_batch("DROP TABLE block_text").unwrap();
    drop(writer);
    graph.save_page(&page, baseline.as_deref()).unwrap();
    let projection = graph.direct_projection_test().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !projection.shared.worker_failed.load(Ordering::Acquire) {
        assert!(
            Instant::now() < deadline,
            "damaged projection must fail its edit turn"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    crate::direct_projection::recover_until_ready(&graph);
    assert!(graph.has_parsed_cache_test());
    let result = graph
        .run_query_bounded("(task TODO)", 100, 1_000_000)
        .expect("the ready projection answers the public bounded route");
    let block = result
        .groups
        .iter()
        .flat_map(|group| &group.blocks)
        .find(|block| block.id == kept_id)
        .unwrap();
    assert_eq!(block.raw, page.blocks[0].raw);
    assert!(!projection.shared.worker_failed.load(Ordering::Acquire));
}

/// Cache eviction does not change an unchanged page's session identities.
/// The compact session owner survives without retaining parsed documents.
#[test]
fn session_identity_survives_parsed_page_eviction() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("identity-eviction");
    let database = scratch("identity-eviction-db").join("projection.sqlite");
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database.clone()).unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let projection = graph.direct_projection_test().unwrap();

    let entry = graph
        .list_pages()
        .into_iter()
        .find(|entry| entry.name == "one")
        .unwrap();
    let one = entry.rel_path.clone();
    let mut page = graph.load_page(&entry).unwrap();
    let baseline = page.rev.clone();
    let kept = page.blocks[0].clone();
    let mut inserted = kept.clone();
    inserted.id = Uuid::new_v4().to_string();
    inserted.raw = "TODO inserted first".into();
    page.blocks.insert(0, inserted);
    graph.save_page(&page, baseline.as_deref()).unwrap();
    wait_ready(&graph);
    assert!(projection.session_pages_test().contains(&one));
    let live = graph
        .run_query_bounded("(task TODO)", 100, 1_000_000)
        .expect("the ready projection answers the public bounded route");
    let live_id = live
        .groups
        .iter()
        .flat_map(|group| group.blocks.iter())
        .find(|block| block.raw == "TODO one [[target]]")
        .map(|block| block.id.clone())
        .expect("the moved block answers");
    assert_eq!(
        live_id, kept.id,
        "a live save answers with the preserved id"
    );

    // Losing this disposable source stamp forces a page repair from
    // unchanged bytes, exercising recovery's identity provenance.
    let writer = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(
        writer
            .execute(
                "DELETE FROM direct_source_revisions WHERE path = ?1",
                rusqlite::params![one]
            )
            .unwrap(),
        1
    );
    drop(writer);
    let parses = graph.page_build_parses_test();
    let repairs = graph.warm_repair_parses_test();
    graph.invalidate_cache();
    assert!(
        projection.session_pages_test().contains(&one),
        "dropping the parsed cache must retain compatible live IDs"
    );
    graph.warm_cache();
    wait_ready(&graph);
    assert_eq!(
        graph.page_build_parses_test(),
        parses,
        "one lost stamp is repaired page by page, not by a whole-graph pass"
    );
    assert_eq!(graph.warm_repair_parses_test(), repairs + 1);
    let statements = graph.direct_projection_statement_reads_test();
    let after = graph
        .run_query_bounded("(task TODO)", 100, 1_000_000)
        .expect("the ready projection answers the public bounded route");
    assert_eq!(
        graph.direct_projection_statement_reads_test(),
        statements + 1
    );
    assert_eq!(
        signature(&after.groups),
        signature(&live.groups),
        "cache eviction preserves the same session's complete result"
    );
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(database.parent().unwrap());
}

fn empty_projection_shared() -> ProjectionShared {
    ProjectionShared {
        path: PathBuf::from("unused"),
        pending: Mutex::new(PendingProjection::default()),
        changed: Condvar::new(),
        ready: AtomicBool::new(false),
        ready_generation: AtomicU64::new(0),
        commit_notification: AtomicU64::new(0),
        commit_waker: Mutex::new(None),
        reader: Mutex::new(None),
        query_jobs: Arc::new(QueryJobOwner::new(DEFAULT_QUERY_JOB_CAPACITY)),
        session_pages: Mutex::new(Arc::new(HashSet::new())),
        committed_registry: Arc::new(Mutex::new(None)),
        worker_available: AtomicBool::new(true),
        worker_failed: AtomicBool::new(false),
        worker_busy: AtomicBool::new(false),
        worker_building: AtomicBool::new(false),
        worker_finished: AtomicBool::new(false),
        worker_resources: Mutex::new(Some(Vec::new())),
        validated: AtomicBool::new(false),
        after_sql_commit: Mutex::new(None),
        after_fresh_build_batch: Mutex::new(None),
        before_fresh_publication: Mutex::new(None),
        after_fresh_publication: Mutex::new(None),
        before_shared_reader_admission: Mutex::new(None),
        after_shared_reader_admission: Mutex::new(None),
        before_shared_reader_drain_lock: Mutex::new(None),
        after_shared_reader_drain_lock: Mutex::new(None),
        serving_writer_cache_budget: AtomicU64::new(0),
        projection_health_checks: AtomicU64::new(0),
        repairs_in_flight: AtomicUsize::new(0),
        warms_in_flight: AtomicUsize::new(0),
        build_progress: Default::default(),
        #[cfg(test)]
        capture_thread: Mutex::new(None),
        #[cfg(test)]
        indexed_reads: AtomicU64::new(0),
        statement_reads: AtomicU64::new(0),
        registry_capture_attempts: AtomicU64::new(0),
        inject_read_failure: AtomicBool::new(false),
        fallback_reads: AtomicU64::new(0),
        referenced_name_reads: AtomicU64::new(0),
    }
}

#[test]
fn query_job_queued_captures_release_before_reset_or_close_waits() {
    for close in [false, true] {
        let shared = empty_projection_shared();
        let epoch = shared.query_jobs.capture_epoch();
        let mut results = Vec::new();
        for _ in 0..DEFAULT_QUERY_JOB_CAPACITY {
            let OwnedAdmission::Slot(slot) = shared
                .query_jobs
                .acquire_owned_at_within(epoch, Duration::ZERO)
            else {
                panic!("fixture admission");
            };
            let (reply, result) = std::sync::mpsc::sync_channel(1);
            shared
                .pending
                .lock()
                .unwrap()
                .captures
                .push(PendingQueryCapture {
                    requirement: QueryCaptureRequirement::StrictGeneration(0),
                    registry_sensitivity: RegistrySensitivity::Required,
                    slot,
                    reply,
                });
            results.push(result);
        }
        assert!(matches!(
            shared
                .query_jobs
                .acquire_owned_at_within(epoch, Duration::ZERO),
            OwnedAdmission::Busy
        ));
        let fence = shared.cancel_queued_captures(close);
        // Assert before waiting: a regression fails instead of deadlocking.
        assert_eq!(shared.query_jobs.active(), 0);
        assert!(shared.pending.lock().unwrap().captures.is_empty());
        for result in results {
            assert!(matches!(
                result.try_recv().unwrap(),
                QueryJobOpen::Cancelled
            ));
        }
        shared.query_jobs.wait_for_drain(fence);
        assert!(matches!(
            shared
                .query_jobs
                .acquire_owned_at_within(epoch, Duration::ZERO),
            OwnedAdmission::Cancelled
        ));
        let next = shared
            .query_jobs
            .acquire_owned_at_within(shared.query_jobs.capture_epoch(), Duration::ZERO);
        if close {
            assert!(matches!(next, OwnedAdmission::Cancelled));
        } else {
            assert!(matches!(next, OwnedAdmission::Slot(_)));
        }
    }
}

#[test]
fn query_job_capture_waits_for_post_commit_identity_publication() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("producer-identity-boundary");
    let database = root.join("private/projection.sqlite");
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database.clone()).unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let projection = graph.direct_projection_test().unwrap();
    // A producer-queued capture drains the preceding publication before
    // registering this test's observer; wait_ready alone sees readiness
    // before the final notification instructions execute.
    let QueryJobOpen::Job(initial) =
        projection.open_current_query_job(RegistrySensitivity::Insensitive)
    else {
        panic!("initial complete image");
    };
    drop(initial);
    let (commit_wake, commits) = std::sync::mpsc::channel();
    projection.observe_commits(commit_wake);
    let id = "pages/identity-new.md".to_owned();
    assert!(!projection.session_pages_test().contains(&id));
    let (paused, observed) = std::sync::mpsc::channel();
    let (resume, resumed) = std::sync::mpsc::channel();
    *projection.shared.after_sql_commit.lock().unwrap() = Some(Box::new(move || {
        paused.send(()).unwrap();
        resumed.recv().unwrap();
    }));
    assert!(graph
        .create_markdown_page_if_absent("identity-new", "- TODO producer identity sentinel\n",)
        .unwrap());
    observed.recv_timeout(Duration::from_secs(3)).unwrap();
    // The new SQL is committed but its identity policy has not published.
    let connection = rusqlite::Connection::open(&database).unwrap();
    let committed: i64 = connection
        .query_row(
            "SELECT count(*) FROM block_text WHERE content = 'TODO producer identity sentinel'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let identity_pending = !projection.session_pages_test().contains(&id);
    let notification_pending = matches!(
        commits.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    );
    let owner = &projection.shared.query_jobs;
    let OwnedAdmission::Slot(slot) =
        owner.acquire_owned_at_within(owner.capture_epoch(), Duration::ZERO)
    else {
        panic!("capture admission");
    };
    let (reply, result) = std::sync::mpsc::sync_channel(1);
    projection
        .shared
        .pending
        .lock()
        .unwrap()
        .captures
        .push(PendingQueryCapture {
            requirement: QueryCaptureRequirement::StrictGeneration(graph.cache_generation()),
            registry_sensitivity: RegistrySensitivity::Required,
            slot,
            reply,
        });
    projection.shared.changed.notify_all();
    let remained_queued = matches!(result.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty));
    // Always release the barrier before assertions or graph destruction.
    resume.send(()).unwrap();
    let captured = result.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(committed, 1);
    assert!(identity_pending);
    assert!(
        notification_pending,
        "raw SQL commit must not publish incoherent identity"
    );
    commits.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(remained_queued);
    let QueryJobOpen::Job(mut job) = captured else {
        panic!("published capture");
    };
    assert!(job.session_pages.contains(&id));
    let mut matching = 0;
    job.snapshot
        .visit_projection_query(
            "SELECT block_id FROM block_text WHERE content = 'TODO producer identity sentinel'",
            &[],
            |_| {
                matching += 1;
                Ok(std::ops::ControlFlow::Continue(()))
            },
        )
        .unwrap();
    assert_eq!(matching, 1);
    drop(job);
    drop(connection);
    drop(projection);
    drop(graph);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn query_job_snapshot_is_captured_on_the_projection_worker() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("producer-capture");
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let projection = graph.direct_projection_test().unwrap();
    let (observed, result) = std::sync::mpsc::channel();
    *projection.shared.capture_thread.lock().unwrap() = Some(observed);
    let QueryJobOpen::Job(mut job) = projection.open_query_job(graph.cache_generation()) else {
        panic!("ready capture");
    };
    assert_ne!(
        result.try_recv().unwrap(),
        std::thread::current().id(),
        "foreground snapshot acquisition bypassed producer serialization"
    );
    let mut rows = 0;
    job.snapshot
        .visit_projection_query("SELECT block_id FROM blocks", &[], |_| {
            rows += 1;
            Ok(std::ops::ControlFlow::Continue(()))
        })
        .unwrap();
    assert!(rows > 0);
    drop(job);
    drop(projection);
    drop(graph);
    let _ = std::fs::remove_dir_all(root);
}

/// **R6 §3, unit.** A structural relowering removes the page from the
/// session set exactly as a deletion does.
#[test]
fn reference_hydration_without_a_cache_parses_only_candidates() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("hydration");
    let database = scratch("hydration-db").join("projection.sqlite");
    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        release_projection(&graph);
    }
    std::thread::sleep(Duration::from_millis(20));
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database.clone()).unwrap();
    assert!(graph.warm_cache_cancellable(|| false));
    wait_ready(&graph);
    graph.reset_direct_projection_candidate_probe_test();
    let backlinks = crate::query::backlinks(&graph, "target");
    let oracle = Graph::open(&root);
    assert_eq!(
        signature(&backlinks),
        signature(&crate::query::backlinks(&oracle, "target"))
    );
    assert_eq!(
        graph.direct_projection_hydrated_pages_test(),
        vec![PathBuf::from("pages/one.md")]
    );
    assert_eq!(graph.on_demand_parses_test(), 1);
    assert_eq!(graph.page_build_parses_test(), 0);
    assert!(!graph.has_parsed_cache_test());
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(database.parent().unwrap());
}

/// **R6 §2.** `list_pages` is served from the projection inventory when
/// it is ready: no parse, no cache, and the same effective entries as the
/// cold whole-graph listing — a `title::` page and a journal's sort key
/// included.
#[test]
fn list_pages_is_served_from_the_projection_when_ready() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("list-pages");
    let database = scratch("list-pages-db").join("projection.sqlite");
    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        release_projection(&graph);
    }
    std::thread::sleep(Duration::from_millis(20));
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database.clone()).unwrap();
    assert!(graph.warm_cache_cancellable(|| false));
    wait_ready(&graph);
    let listed = graph.list_pages();
    assert_eq!(graph.page_build_parses_test(), 0);
    assert!(!graph.has_parsed_cache_test());
    let oracle = Graph::open(&root);
    assert_eq!(
        entry_signature(&listed),
        entry_signature(&oracle.list_pages())
    );
    assert!(listed
        .iter()
        .any(|entry| entry.name == "Titled Page" && entry.rel_path == "pages/titled.md"));
    assert!(listed.iter().any(|entry| {
        entry.kind == PageKind::Journal
            && entry.date_key == Some(20260906)
            && entry.rel_path == "journals/2026_09_06.md"
    }));
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(database.parent().unwrap());
}

#[test]
#[ignore = "manual storage packet receipt; set TINE_DIRECT_PROJECTION_CORPUS"]
fn real_corpus_clean_reopen_reuses_projected_pages() {
    let _serial = serialize_projection_tests();
    let root = PathBuf::from(
        std::env::var("TINE_DIRECT_PROJECTION_CORPUS")
            .expect("TINE_DIRECT_PROJECTION_CORPUS is required"),
    );
    let database = scratch("real-corpus-reopen").join("projection.sqlite");
    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        release_projection(&graph);
    }
    std::thread::sleep(Duration::from_millis(20));

    reset_lowerings(&root);
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database.clone()).unwrap();
    let started = Instant::now();
    graph.warm_cache();
    let warm = started.elapsed();
    wait_ready(&graph);
    let converged = started.elapsed();
    // R6: the warm validated from bytes alone — no page parsed, no cache.
    let parses = graph.page_build_parses_test();
    assert_eq!(parses, 0, "clean reopen must not parse any page");
    assert!(!graph.has_parsed_cache_test());
    let query_started = Instant::now();
    let indexed = graph
        .run_query_bounded("(task TODO)", 20_000, 32 << 20)
        .expect("the ready projection answers the public bounded route");
    let indexed_elapsed = query_started.elapsed();
    let oracle = crate::query::run_query_bounded(&graph, "(task TODO)", 20_000, 32 << 20);
    assert_eq!(signature(&indexed.groups), signature(&oracle.groups));
    assert_eq!(
        lowerings(),
        0,
        "clean reopen must not lower unchanged pages"
    );
    eprintln!(
            "direct projection clean-reopen receipt: warm_ms={} warm_validate_ms={} projection_total_ms={} projection_tail_ms={} indexed_query_us={} pages_lowered={} pages_parsed={}",
            warm.as_millis(),
            warm.as_millis(),
            converged.as_millis(),
            converged.saturating_sub(warm).as_millis(),
            indexed_elapsed.as_micros(),
            lowerings(),
            parses,
        );
    let _ = std::fs::remove_dir_all(database.parent().unwrap());
}

#[test]
#[ignore = "manual storage packet receipt; set TINE_DIRECT_PROJECTION_CORPUS"]
fn real_corpus_reference_family_matches_parser_oracle() {
    let _serial = serialize_projection_tests();
    let root = PathBuf::from(
        std::env::var("TINE_DIRECT_PROJECTION_CORPUS")
            .expect("TINE_DIRECT_PROJECTION_CORPUS is required"),
    );
    let database = scratch("real-corpus-reference-family").join("projection.sqlite");
    let oracle = Graph::open(&root);
    oracle.warm_cache();
    let aliases = crate::query::page_aliases_with_owners(&oracle);
    let alias_target = aliases.first().map(|(alias, _, _)| alias.clone());
    let oracle_backlinks = alias_target
        .as_deref()
        .map(|target| crate::query::backlinks(&oracle, target));
    let oracle_unlinked_started = Instant::now();
    let oracle_unlinked = alias_target
        .as_deref()
        .map(|target| crate::query::unlinked_refs(&oracle, target));
    let oracle_unlinked_elapsed = oracle_unlinked_started.elapsed();
    let oracle_count_started = Instant::now();
    let oracle_counts = oracle.block_ref_counts().unwrap();
    let oracle_count_elapsed = oracle_count_started.elapsed();
    let block_claim = oracle.with_pages(|pages| {
        pages.iter().find_map(|(_, document)| {
            let mut claim = None;
            fn visit(blocks: &[DocBlock], claim: &mut Option<String>) {
                for block in blocks {
                    if claim.is_none() {
                        *claim = block.projection().block_refs.first().cloned();
                    }
                    visit(&block.children, claim);
                }
            }
            visit(&document.roots, &mut claim);
            claim
        })
    });
    let oracle_referrers = block_claim
        .as_deref()
        .map(|claim| crate::query::block_referrers(&oracle, claim));
    let oracle_resolved = block_claim
        .as_deref()
        .and_then(|claim| crate::query::resolve_block(&oracle, claim));

    let graph = Graph::open(&root);
    graph.attach_direct_projection(database.clone()).unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    assert_eq!(graph.page_aliases_with_owners(), aliases);
    let projected_count_started = Instant::now();
    let projected_counts = graph.block_ref_counts().unwrap();
    let projected_count_elapsed = projected_count_started.elapsed();
    assert_eq!(projected_counts.as_ref(), oracle_counts.as_ref());
    eprintln!(
        "real-corpus-reference counts={} parser_count_us={} sqlite_count_us={}",
        projected_counts.len(),
        oracle_count_elapsed.as_micros(),
        projected_count_elapsed.as_micros(),
    );
    if let Some(target) = alias_target.as_deref() {
        let indexed_unlinked_started = Instant::now();
        let indexed_unlinked = crate::query::unlinked_refs(&graph, target);
        let indexed_unlinked_elapsed = indexed_unlinked_started.elapsed();
        assert_eq!(
            signature(&crate::query::backlinks(&graph, target)),
            signature(oracle_backlinks.as_deref().unwrap())
        );
        assert_eq!(
            signature(&indexed_unlinked),
            signature(oracle_unlinked.as_deref().unwrap())
        );
        let candidates = graph.reference_candidate_pages(
            &[crate::refs::page_key(target)],
            target,
            ReferenceKind::Explicit,
        );
        assert!(candidates.indexed);
        eprintln!(
                "real-corpus-reference explicit_candidates={} full_pages={} parser_unlinked_us={} indexed_unlinked_us={}",
                candidates.pages.len(),
                candidates.full_page_count,
                oracle_unlinked_elapsed.as_micros(),
                indexed_unlinked_elapsed.as_micros(),
            );
    }
    if let Some(claim) = block_claim.as_deref() {
        assert_eq!(
            signature(&crate::query::block_referrers(&graph, claim)),
            signature(oracle_referrers.as_deref().unwrap())
        );
        assert_eq!(
            crate::query::resolve_block(&graph, claim)
                .as_ref()
                .map(|group| signature(std::slice::from_ref(group))),
            oracle_resolved
                .as_ref()
                .map(|group| signature(std::slice::from_ref(group)))
        );
    }
    let _ = std::fs::remove_dir_all(database.parent().unwrap());
}

/// Child half of the two `retired_class_c_*` probes. Emits BOTH retired
/// class-(c) reports, each with its own planted marker, through the exact
/// production reporter and the exact error types the call sites hand it.
#[test]
#[ignore = "child process for the retired class-(c) stderr probe"]
fn w4_i5b_projection_failure_marker_child() {
    if std::env::var("TINE_I5B_SET_FLAG").as_deref() == Ok("1") {
        crate::backend_error::set_runtime_debug_diagnostics(true);
    }
    // Exactly what `open_projection_database` returns: a free-form
    // `MaterializationError` payload.
    report_projection_failure(
        "disabled: its database could not be opened",
        &tine_storage::sqlite::MaterializationError::Sqlite(
            "planted-open-marker-Zq7Page".to_owned(),
        ),
    );
    // Exactly what `apply_pending` returns: a `String` naming the
    // graph-relative page it was projecting.
    report_projection_failure(
        PROJECTION_UPDATE_FAILURE,
        &"parsed page has no exact source revision: pages/planted-apply-marker-Zq7Page.md"
            .to_owned(),
    );
}

fn projection_failure_child_stderr(set_flag: &str) -> String {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "direct_projection::tests::w4_i5b_projection_failure_marker_child",
            "--nocapture",
        ])
        .env_remove("TINE_DEBUG")
        .env("TINE_I5B_SET_FLAG", set_flag)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "projection-failure child failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// I-5, retired class-(c) row `direct_projection.rs` "projection database
/// could not be opened": the always-on line carried a free-form
/// `MaterializationError` payload.
#[test]
fn retired_class_c_projection_database_open_emits_no_planted_marker() {
    let marker = "planted-open-marker-Zq7Page";
    assert!(
        !projection_failure_child_stderr("0").contains(marker),
        "I-5: the always-on projection-open failure still carried its error prose. \
             The always-on line names the failure family only; the detail belongs behind \
             `runtime_debug_diagnostics_enabled()` (I-9 keeps the family, not the prose)."
    );
    assert!(
        projection_failure_child_stderr("1").contains(marker),
        "the directed debug channel must still carry the detail, or this probe proves \
             nothing about where the prose went"
    );
}

/// I-5, retired class-(c) row `direct_projection.rs` "projection is stale;
/// using parser fallback": `apply_pending` formats the graph-relative page
/// path into the error this line used to print always-on.
#[test]
fn retired_class_c_projection_apply_failure_emits_no_planted_marker() {
    let marker = "planted-apply-marker-Zq7Page";
    let ordinary = projection_failure_child_stderr("0");
    assert!(ordinary.contains(PROJECTION_UPDATE_FAILURE));
    assert!(!ordinary.contains("using parser fallback"));
    assert!(
        source_of_this_file().contains("parsed page has no exact source revision: {}"),
        "non-vacuity: this probe exists because `apply_pending` names the page it was \
             projecting in its error string. If that error no longer does, re-derive the row's \
             class before relaxing the probe."
    );
    assert!(
        !ordinary.contains(marker),
        "I-5: the always-on parser-fallback line still carried the graph-relative page \
             path from `apply_pending`. The always-on line names the failure family only."
    );
    assert!(
        projection_failure_child_stderr("1").contains(marker),
        "the directed debug channel must still carry the detail, or this probe proves \
             nothing about where the prose went"
    );
}

fn source_of_this_file() -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/direct_projection.rs"))
        .unwrap()
}

/// Manual scale probe for GH #543 ("Search is stuck on 'Indexing — waiting for
/// search to be ready…'"). Search readiness IS Direct-projection readiness
/// (RET2 made the public Direct query route projection-only, with no walk arm),
/// so the reporter's stuck search is the projection never publishing readiness.
/// `wait_ready` above allows 15 s; the recorded product ceiling is 10 s and
/// linear. This probe REPORTS rather than asserts, because the number is the
/// finding — a pass/fail would hide whether a big graph is slow or never
/// converges at all.
#[test]
#[ignore = "manual scale probe: set TINE_WARM_SCALE_GRAPH to a graph directory"]
fn warm_scale_probe_reports_time_to_projection_ready() {
    let Some(source) = std::env::var_os("TINE_WARM_SCALE_GRAPH") else {
        eprintln!("skipped: set TINE_WARM_SCALE_GRAPH to a graph directory");
        return;
    };
    let _serial = serialize_projection_tests();
    let root = scratch("warm-scale-probe");
    probe_copy_tree(std::path::Path::new(&source), &root);
    let count = |dir: &str| {
        std::fs::read_dir(root.join(dir))
            .map(|entries| entries.count())
            .unwrap_or(0)
    };
    let (pages, journals) = (count("pages"), count("journals"));

    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();

    let started = Instant::now();
    graph.warm_cache();
    let warmed = started.elapsed();

    let budget_secs: u64 = std::env::var("TINE_WARM_SCALE_BUDGET_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(600);
    let budget = Duration::from_secs(budget_secs);
    let state = || {
        graph
            .direct_projection_test()
            .map(|projection| projection.debug_state_test())
            .unwrap_or_else(|| "no projection".to_owned())
    };

    let mut ready_at = None;
    let mut last_report = Instant::now();
    while started.elapsed() < budget {
        if graph.direct_projection_ready_test() {
            ready_at = Some(started.elapsed());
            break;
        }
        if last_report.elapsed() >= Duration::from_secs(5) {
            last_report = Instant::now();
            println!("WARM-SCALE t={:?} {}", started.elapsed(), state());
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    // Several successive queries, because one timing cannot separate a one-time
    // index materialization from the steady-state per-query cost, and the fix
    // for those two is not the same.
    let mut timings = Vec::new();
    let mut answer = String::new();
    for (i, token) in ["zqx1", "zqx1", "zqx500", "outline", "Topic-00042"]
        .into_iter()
        .enumerate()
    {
        let searched = Instant::now();
        let outcome = match graph.search(token, 50) {
            Ok(groups) => format!("ok({} groups)", groups.len()),
            Err(crate::query::QueryExecutionError::NotReady(reason)) => {
                format!("NotReady({})", reason.as_str())
            }
            Err(_) => "other-error".to_owned(),
        };
        let took = searched.elapsed();
        println!("WARM-SCALE SEARCH #{i} token={token} took={took:?} {outcome}");
        timings.push(format!("{token}={took:?}"));
        if i == 0 {
            answer = outcome;
        }
    }
    println!(
        "WARM-SCALE RESULT pages={pages} journals={journals} warm_cache={warmed:?} \
         ready_at={ready_at:?} budget={budget_secs}s search={answer} searches=[{}] state={}",
        timings.join(" "),
        state()
    );
    if std::env::var_os("TINE_WARM_SCALE_KEEP").is_some() {
        println!("WARM-SCALE KEEP root={}", root.display());
    } else {
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// GH #543: a query landing while the open-path warm is still reading page
/// bytes (seconds on a 10k-page graph, tens on Windows) must be told
/// `Indexing` and retry, not read the projection as idle and start a repair —
/// its repair ran a SECOND warm validation on the query thread, racing the
/// open path's: whichever lost fell to the whole-graph parse and superseded
/// the stream with a one-transaction snapshot, whichever won streamed the
/// graph on the query thread. The Windows verify probe measured that as a
/// 10k-page cold open going from 150 s to 587 s with 35 GB written and the
/// switcher frozen at "Indexing 240 of 10,000".
#[test]
fn a_query_during_the_warm_inventory_read_retries_instead_of_repairing() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("query-during-warm-read");
    let graph = Arc::new(Graph::open(&root));
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    let pause = graph.pause_next_warm_validation_test();
    let warm = {
        let graph = Arc::clone(&graph);
        std::thread::spawn(move || graph.warm_cache())
    };
    // The warm has announced itself and is about to read the inventory.
    pause.reached.wait();
    let during = graph.search_latest("switcher", "target", 50);
    assert!(
        matches!(
            during,
            Err(crate::query::QueryExecutionError::NotReady(
                crate::query::QueryReadinessReason::Indexing
            ))
        ),
        "a query during the warm's inventory read must report Indexing, got {during:?}"
    );
    assert!(
        !graph.has_parsed_cache_test(),
        "the query must not have parsed the graph beside the warm"
    );
    pause.release.wait();
    warm.join().unwrap();
    wait_ready(&graph);
    let groups = when_ready(|| graph.search_latest("switcher", "target", 50));
    assert!(
        !groups.is_empty(),
        "the same query answers once the warm has converged"
    );
    let projection = graph.direct_projection_test().unwrap();
    assert!(projection.close_and_wait_for_worker(Duration::from_secs(3)));
    drop(projection);
    drop(graph);
    std::fs::remove_dir_all(root).unwrap();
}

/// GH #543: the build's progress label must not go dark while the warm is
/// still converging the graph.
///
/// Its sibling above pins that a QUERY landing in the warm's inventory read is
/// told `Indexing`. `index_progress` answers the same question — is a warm
/// converging this graph? — for the label beside that query, and answered it
/// differently: it tested only the projection's pending QUEUE, and a warm that
/// has announced itself queues nothing until the inventory read finishes.
/// `progress_at` already counts `warms_in_flight` for exactly this reason
/// (`Working(Reason::Indexing)`); `index_progress` did not, so the two
/// disagreed for the whole of the build's most expensive phase — seconds on a
/// 10k-page graph, tens on Windows, and re-run from the top on every drift
/// retry.
///
/// The user-visible harm is not only the missing count. `QuickSwitcher.tsx`
/// reads a non-null → null transition as "the build finished" and re-runs the
/// query, so every drift retry fired a spurious search at a machine already
/// saturated by the rebuild the user is waiting for, while the line above it
/// still read "Indexing — waiting for search to be ready…".
#[test]
#[ignore = "manual read probe: set TINE_READ_PROBE_GRAPH to a graph directory"]
fn read_latency_probe_reports_per_surface_timings() {
    let Some(source) = std::env::var_os("TINE_READ_PROBE_GRAPH") else {
        eprintln!("skipped: set TINE_READ_PROBE_GRAPH to a graph directory");
        return;
    };
    let _serial = serialize_projection_tests();
    let root = scratch("read-latency-probe");
    let source = std::path::PathBuf::from(&source);
    probe_copy_tree(&source, &root);
    // Reuse a prebuilt index when the source carries one: the probe measures
    // READ latency, and rebuilding a 10,000-page index per run would dominate
    // the comparison it exists to make.
    if source.join("private").is_dir() {
        std::fs::create_dir_all(root.join("private")).unwrap();
        for entry in std::fs::read_dir(source.join("private")).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_file() {
                std::fs::copy(entry.path(), root.join("private").join(entry.file_name())).unwrap();
            }
        }
    }
    let label = std::env::var("TINE_READ_PROBE_LABEL").unwrap_or_else(|_| "probe".to_owned());
    let repeats: usize = std::env::var("TINE_READ_PROBE_REPEATS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(3);

    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    let warm_started = Instant::now();
    graph.warm_cache();
    let budget = Duration::from_secs(
        std::env::var("TINE_READ_PROBE_BUDGET_SECS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(900),
    );
    let waited = Instant::now();
    while !graph.direct_projection_ready_test() {
        assert!(
            waited.elapsed() < budget,
            "the probe graph never became ready: {}",
            graph
                .direct_projection_test()
                .map(|projection| projection.debug_state_test())
                .unwrap_or_else(|| "no projection".to_owned())
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    println!(
        "PROBE {label} ready_ms={} parsed_cache={}",
        warm_started.elapsed().as_millis(),
        graph.has_parsed_cache_test()
    );

    let measure = |surface: &str, needle: &str, mut run: Box<dyn FnMut() -> usize + '_>| {
        let mut timings = Vec::with_capacity(repeats);
        let mut hits = Vec::with_capacity(repeats);
        for _ in 0..repeats {
            let started = Instant::now();
            let count = run();
            timings.push(started.elapsed().as_millis());
            hits.push(count);
        }
        assert!(
            hits.windows(2).all(|pair| pair[0] == pair[1]),
            "{surface}/{needle} answered differently across repeats: {hits:?}"
        );
        timings.sort_unstable();
        println!(
            "PROBE {label} surface={surface} needle={needle:?} median_ms={} runs_ms={timings:?} hits={}",
            timings[repeats / 2],
            hits[0]
        );
    };

    // Zero-hit and every-block needles bracket the candidate bound: one should
    // become trivial, the other cannot, and neither may regress.
    for needle in [
        "zqx1",
        "sentinel543",
        "outline",
        // The MIDDLE of the selectivity range, which the rest of this matrix
        // misses: every other needle here matches either nothing or nearly
        // every block, and a candidate bound can only help in between. On this
        // fixture the trigram index admits ~120 blocks for this one.
        "4242",
        "block 4242",
        "Topic 4242",
        "你好",
        "b17",
    ] {
        // Ctrl-K itself. `QuickSwitcher.tsx` calls `runGraphSearch` with
        // PAGE_POOL/BLOCK_POOL = 100 in the "quick-switch" lane, which compiles
        // the block branch as Contains/Phrase. `search_latest` below is a
        // DIFFERENT surface — the in-editor `((…))` block picker — and compiles
        // Fuzzy. Measuring only that one is what made an earlier before/after
        // pair show no change: the bound cannot apply to Fuzzy at all.
        measure(
            "graph_search",
            needle,
            Box::new(|| {
                graph
                    .run_graph_search_latest("quick-switch", needle, 100, 100, false)
                    .map(|execution| execution.hits.len())
                    .unwrap_or(usize::MAX)
            }),
        );
        measure(
            "search",
            needle,
            Box::new(|| {
                graph
                    .search_latest("read-probe", needle, 100)
                    .map(|groups| groups.len())
                    .unwrap_or(usize::MAX)
            }),
        );
        measure(
            "quick_switch",
            needle,
            Box::new(|| graph.quick_switch(needle, 50).len()),
        );
    }
    measure("templates", "-", Box::new(|| graph.templates().len()));
    let names = (0..50)
        .map(|page| format!("Topic {page} 你好"))
        .collect::<Vec<_>>();
    measure(
        "page_icons",
        "50 names",
        Box::new(|| graph.page_icons(&names).len()),
    );

    // Q9's historical quick-switch producer probes remain separate from the
    // dictionary-backed executor. They detect a regression that reconnects a
    // whole-graph inventory/alias/reference read ahead of needle planning.
    measure(
        "producer:list_pages",
        "-",
        Box::new(|| graph.list_pages().len()),
    );
    measure(
        "producer:aliases",
        "-",
        Box::new(|| graph.page_aliases_with_owners().len()),
    );
    measure(
        "producer:referenced",
        "-",
        Box::new(|| graph.referenced_page_names().len()),
    );

    if std::env::var_os("TINE_READ_PROBE_KEEP").is_some() {
        println!("PROBE {label} KEEP root={}", root.display());
    } else {
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// GH #543: the projection lifecycle is the one part of a long build nobody
/// could observe. The Windows verify probe could say only that a 10,000-page
/// cold open took 139 s with the switcher stuck on "Indexing 0 of 10,001";
/// the app's debug log stopped at "Direct Files publish" and the next 105 s
/// were unrecorded. These lines are that record — and they must stay on the
/// opt-in channel, because they name the graph's page counts and timings.
#[test]
fn projection_lifecycle_diagnostics_follow_the_debug_flag() {
    struct RestoreFlag;
    impl Drop for RestoreFlag {
        fn drop(&mut self) {
            crate::backend_error::set_runtime_debug_diagnostics(false);
        }
    }
    let _serial = serialize_projection_tests();
    let _restore = RestoreFlag;

    crate::backend_error::set_runtime_debug_diagnostics(false);
    let silent_root = r6_graph("projection-diag-off");
    let silent = Graph::open(&silent_root);
    silent
        .attach_direct_projection(silent_root.join("private/projection.sqlite"))
        .unwrap();
    let before = crate::direct_projection::projection_diag_lines_test();
    silent.warm_cache();
    wait_ready(&silent);
    assert_eq!(
        crate::direct_projection::projection_diag_lines_test(),
        before,
        "a run without TINE_DEBUG must emit no projection lifecycle line"
    );

    crate::backend_error::set_runtime_debug_diagnostics(true);
    let loud_root = r6_graph("projection-diag-on");
    let loud = Graph::open(&loud_root);
    loud.attach_direct_projection(loud_root.join("private/projection.sqlite"))
        .unwrap();
    let before = crate::direct_projection::projection_diag_lines_test();
    loud.warm_cache();
    wait_ready(&loud);
    assert!(
        crate::direct_projection::projection_diag_lines_test() > before,
        "a run under TINE_DEBUG must record the warm announcing, queueing and converging"
    );

    for (graph, root) in [(silent, silent_root), (loud, loud_root)] {
        let projection = graph.direct_projection_test().unwrap();
        assert!(projection.close_and_wait_for_worker(Duration::from_secs(3)));
        std::fs::remove_dir_all(root).unwrap();
    }
}

/// Manual rank-cost probe retained from GH #543. The historical switcher poll
/// and partial-build progress API are retired; this now isolates only the
/// per-query ranking cost over a supplied block corpus.
#[test]
#[ignore = "manual cost probe: set TINE_RANK_CORPUS to a NUL-separated block corpus"]
fn rank_cost_split_probe() {
    let Some(path) = std::env::var_os("TINE_RANK_CORPUS") else {
        eprintln!("skipped: set TINE_RANK_CORPUS to a NUL-separated block corpus");
        return;
    };
    let corpus = std::fs::read_to_string(path).unwrap();
    let texts: Vec<&str> = corpus.split('\0').collect();
    let bytes: usize = texts.iter().map(|text| text.len()).sum();
    let plan = crate::query_plan::QueryPlan::block_search_literal("zqx1", 50);
    let branch = &plan.branches[0];

    let started = Instant::now();
    let mut folded_len = 0usize;
    for text in &texts {
        folded_len += crate::search_query::canonical_fold(text).len();
    }
    let fold_only = started.elapsed();

    let started = Instant::now();
    let mut ranked = 0usize;
    for text in &texts {
        if crate::query_plan::rank_block_text(&plan, branch, text).is_some() {
            ranked += 1;
        }
    }
    let fold_and_rank = started.elapsed();

    // Price the alternative design: if the rank program took a PAIR
    // (visible, stored fold) through `framed_pair_sql`, SQL would concatenate
    // both texts per row. That must be cheaper than the fold it replaces, or the
    // pair design buys nothing.
    let started = Instant::now();
    let mut framed_len = 0usize;
    for text in &texts {
        let folded = text; // same bytes on this corpus; the SHAPE is what is priced
        framed_len += format!("{}:{}{}", text.len(), text, folded).len();
    }
    let framing_only = started.elapsed();
    println!("RANK-COST-FRAMING framed_len={framed_len} framing_only={framing_only:?}");

    println!(
        "RANK-COST blocks={} bytes={bytes} folded_len={folded_len} ranked={ranked} \
         fold_only={fold_only:?} fold_and_rank={fold_and_rank:?} relevance_only~={:?} \
         fold_share={:.0}%",
        texts.len(),
        fold_and_rank.saturating_sub(fold_only),
        100.0 * fold_only.as_secs_f64() / fold_and_rank.as_secs_f64().max(f64::MIN_POSITIVE),
    );
}

fn probe_copy_tree(source: &std::path::Path, target: &std::path::Path) {
    std::fs::create_dir_all(target).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let to = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            probe_copy_tree(&entry.path(), &to);
        } else {
            std::fs::copy(entry.path(), to).unwrap();
        }
    }
}

/// GH #543: `rebuild` is an obligation. Repair sets it before computing a
/// payload, and the worker consumes it ONLY beside a full snapshot or a warm
/// inventory — so a repair that fails to enqueue one leaves a flag nobody can
/// discharge. `has_work` does not count it, so the worker sleeps; every capture
/// is refused; `progress_at` says `Working(Recovering)` forever. The reporter
/// sees "Rebuilding the search index…" on an idle process, with no filesystem
/// activity needed to get there.
#[test]
fn a_reset_projection_does_not_answer_from_the_rows_it_just_erased() {
    let root = scratch("gh543-reset-validated");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages/a.md"), "- sentinel543 original\n").unwrap();
    let graph = Arc::new(Graph::open(&root));
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    let projection = graph.direct_projection_test().unwrap();
    graph.warm_cache();
    assert!(graph.direct_projection_ready_test());
    assert!(graph.has_parsed_cache_test());

    // Drift the generation as the recovery warm commits, so its replacement
    // stream abandons: the reset has happened, the refill has not.
    {
        let graph = Arc::clone(&graph);
        *projection.shared.after_sql_commit.lock().unwrap() =
            Some(Box::new(move || graph.drift_generation_test()));
    }
    projection.inject_next_statement_failure();
    let _ = graph.search("sentinel543", 50);
    *projection.shared.after_sql_commit.lock().unwrap() = None;

    // A real edit arrives through the real reconciliation path.
    let path = graph.walk_entries_test()[0].path.clone();
    std::fs::write(&path, "- newest543\n").unwrap();
    assert!(graph.sync_file_checked(&path).unwrap().is_some());

    // Either search reports that it is rebuilding, or it finds the new text.
    // What it must never do is answer "no results" from an erased index.
    let mut found = false;
    for _ in 0..600 {
        match graph.search("newest543", 50) {
            Ok(groups) if !groups.is_empty() => {
                found = true;
                break;
            }
            Ok(_) => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(crate::query::QueryExecutionError::NotReady(_)) => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("unexpected error {error}"),
        }
    }
    assert!(
        found,
        "the edit never became searchable: {}",
        projection.debug_state_test()
    );
}

#[test]
fn a_torn_projection_is_replaced_only_by_a_complete_fresh_build() {
    let root = scratch("gh543-torn-header");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages/a.md"), "- fresh-build sentinel\n").unwrap();
    let path = root.join("private/projection.sqlite");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let torn = vec![0x55; 4096];
    std::fs::write(&path, &torn).unwrap();

    let projection = DirectProjection::start(path.clone()).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), torn);
    assert!(projection.worker_available());
    assert!(projection.close_and_wait_for_worker(Duration::from_secs(5)));

    let graph = Graph::open(&root);
    graph.attach_direct_projection(path.clone()).unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    assert!(std::fs::read(&path)
        .unwrap()
        .starts_with(b"SQLite format 3\0"));
    assert!(!graph.search("fresh-build sentinel", 50).unwrap().is_empty());
}

/// GH #543 (re-audit A2-B1): the stolen verdict the idle escape could NOT
/// catch. A warm descheduled before reading the slot used to take a LATER
/// warm's verdict; that later warm then waited for a producer that had already
/// run, and the earlier one acted on a verdict computed for a generation it
/// does not carry. The escape hatch cannot fire for the robbed waiter, because
/// its own stream is open — so nothing released it, and it holds the
/// process-wide warm mutex while every later graph warm queues behind it.
#[test]
fn a_warm_does_not_take_a_later_attempts_verdict() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("gh543-stolen-verdict");
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let projection = graph.direct_projection_test().unwrap();
    let config = Arc::new(graph.config.parse_config());

    // Attempt A announces a warm and is descheduled before reading its verdict.
    let (sources, bytes) = warm_sources(&graph);
    let order = walk_order(&sources, &[]);
    let attempt_a = require_warm(projection.enqueue_warm(
        graph.cache_generation(),
        sources,
        Vec::new(),
        Vec::new(),
        order,
        Arc::clone(&config),
        bytes,
    ));
    for _ in 0..500 {
        if projection
            .shared
            .pending
            .lock()
            .unwrap()
            .warm_outcome
            .is_some()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        projection
            .shared
            .pending
            .lock()
            .unwrap()
            .warm_outcome
            .is_some(),
        "the first warm's verdict never landed: {}",
        projection.debug_state_test()
    );

    // The source changes underneath it and attempt B is admitted, clearing the
    // slot and taking over the stream.
    let path = graph.walk_entries_test()[0].path.clone();
    std::fs::write(&path, "- gh543 stolen verdict sentinel\n").unwrap();
    graph.drift_generation_test();
    for _ in 0..500 {
        let idle = {
            let pending = projection.shared.pending.lock().unwrap();
            !pending.has_work()
        };
        if idle && !projection.shared.worker_busy.load(Ordering::Acquire) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let (sources, bytes) = warm_sources(&graph);
    let order = walk_order(&sources, &[]);
    let attempt_b = require_warm(projection.enqueue_warm(
        graph.cache_generation(),
        sources,
        Vec::new(),
        Vec::new(),
        order,
        config,
        bytes,
    ));
    assert_ne!(attempt_a, attempt_b);

    // A wakes. B's verdict is not A's to take.
    let outcome_a = projection.wait_warm_outcome(attempt_a);
    assert!(
        matches!(outcome_a, WarmOutcome::Superseded),
        "the first warm took a verdict computed for another attempt: {outcome_a:?}"
    );

    // And B still receives its own, on a thread so a regression times out here
    // rather than hanging the suite.
    let (tx, rx) = std::sync::mpsc::channel();
    let waiter = Arc::clone(&projection);
    let joined = std::thread::spawn(move || {
        let _ = tx.send(waiter.wait_warm_outcome(attempt_b));
    });
    let outcome_b = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the second warm never received its own verdict");
    joined.join().unwrap();
    assert!(
        !matches!(outcome_b, WarmOutcome::Superseded | WarmOutcome::Failed),
        "the second warm lost its own verdict: {outcome_b:?} {}",
        projection.debug_state_test()
    );
    assert!(projection.close_and_wait_for_worker(Duration::from_secs(5)));
}

/// GH #543 (audit finding F1): `warm_outcome` is a single slot with no attempt
/// key, so a warm that is descheduled before it reads the slot can find its
/// outcome already taken by a second warm. Such a waiter must not block
/// forever — it holds the process-wide warm mutex, so every later graph warm
/// queues behind it and the app stops indexing altogether.
#[test]
fn a_warm_waiter_with_no_producer_does_not_block_forever() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("gh543-lost-outcome");
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let projection = graph.direct_projection_test().unwrap();

    // The state a loser of that race is left in: nothing queued, an idle
    // worker, and an empty outcome slot.
    let attempt = {
        let pending = projection.shared.pending.lock().unwrap();
        assert!(pending.warm.is_none());
        assert!(pending.warm_outcome.is_none());
        // The CURRENT attempt id: this waiter has not been superseded, so only
        // the idle-worker escape can release it.
        pending.warm_attempt
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let waiter = Arc::clone(&projection);
    let joined = std::thread::spawn(move || {
        let _ = tx.send(waiter.wait_warm_outcome(attempt));
    });
    let outcome = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("wait_warm_outcome blocked with nothing left to produce an outcome");
    joined.join().unwrap();
    assert!(
        matches!(outcome, WarmOutcome::Superseded),
        "expected the waiter to be told another attempt owns readiness, got {outcome:?}"
    );
}

/// GH #543 (audit finding F4): a page the walk cannot READ is not a page that
/// is GONE. Such a page carries no revision, so it is absent from the warm's
/// `sources` — and validation used to read that absence as a deletion and drop
/// every row it had, while reporting `Clean`. A live page then vanished from
/// search over a transient disk error or a sharing violation, and stayed gone
/// until something else touched it.
#[test]
fn a_page_the_walk_could_not_read_keeps_the_rows_it_already_had() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("gh543-retained");
    std::fs::write(
        root.join("pages/unreadable.md"),
        "- gh543 retained sentinel\n",
    )
    .unwrap();
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    assert!(
        !graph
            .search("gh543 retained sentinel", 50)
            .unwrap()
            .is_empty(),
        "the page was never indexed, so this test would prove nothing"
    );
    let projection = graph.direct_projection_test().unwrap();

    // A second walk in which that one page's bytes could not be read: it is
    // absent from `sources` for want of a revision, and named `retained`.
    let (all, bytes) = warm_sources(&graph);
    let (unreadable, sources): (Vec<_>, Vec<_>) = all
        .into_iter()
        .partition(|(entry, _)| entry.rel_path.ends_with("unreadable.md"));
    let retained = unreadable
        .into_iter()
        .map(|(entry, _)| entry)
        .collect::<Vec<_>>();
    assert_eq!(retained.len(), 1, "fixture did not contain the page");
    let config = Arc::new(graph.config.parse_config());
    let generation = graph.cache_generation();
    let order = walk_order(&sources, &retained);
    let attempt = require_warm(projection.enqueue_warm(
        generation,
        sources,
        retained,
        Vec::new(),
        order,
        config,
        bytes,
    ));
    let outcome = projection.wait_warm_outcome(attempt);
    wait_ready(&graph);

    assert!(
        !graph
            .search("gh543 retained sentinel", 50)
            .unwrap()
            .is_empty(),
        "an unreadable page's rows were deleted as if the page were gone \
         (warm outcome {outcome:?}): {}",
        projection.debug_state_test()
    );
}

/// GH #543: a page the warm could not read keeps its place in the page order.
/// Appending it after the pages that were read shifted every page walked
/// after it by one, so the next update to any of them claimed a position
/// another stored page still held (`UNIQUE constraint failed:
/// pages.position`), and that failure rebuilt the whole index.
#[test]
fn a_page_the_walk_could_not_read_keeps_its_place_for_later_updates() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("gh543-retained-order");
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let projection = graph.direct_projection_test().unwrap();

    // Every page but the last is followed by one: retain the first.
    let (all, bytes) = warm_sources(&graph);
    let order = walk_order(&all, &[]);
    let retained = vec![all[0].0.clone()];
    let later = all.last().unwrap().0.clone();
    let sources = all.into_iter().skip(1).collect::<Vec<_>>();
    let config = Arc::new(graph.config.parse_config());
    let attempt = require_warm(projection.enqueue_warm(
        graph.cache_generation(),
        sources,
        retained,
        Vec::new(),
        order,
        config,
        bytes,
    ));
    assert!(matches!(
        projection.wait_warm_outcome(attempt),
        WarmOutcome::Clean
    ));
    wait_ready(&graph);

    let failures = reported_projection_failures_test();
    std::fs::write(&later.path, "- gh543 later page edited\n").unwrap();
    graph.load_page(&later).expect("the page opens");
    wait_ready(&graph);
    assert_eq!(
        reported_projection_failures_test(),
        failures,
        "the update to a page walked after the unreadable one failed: {}",
        projection.debug_state_test()
    );
    assert!(
        !graph
            .search("gh543 later page edited", 50)
            .unwrap()
            .is_empty(),
        "the edit is not indexed: {}",
        projection.debug_state_test()
    );
}

#[test]
fn an_incomplete_stale_snapshot_keeps_the_old_coherent_image_until_reconstruction_succeeds() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("incomplete-stale-snapshot");
    let unreadable_path = root.join("pages/unreadable.md");
    std::fs::write(&unreadable_path, "- retained unreadable sentinel\n").unwrap();
    let database = root.join("private/projection.sqlite");
    let first = Graph::open(&root);
    first.attach_direct_projection(database.clone()).unwrap();
    first.warm_cache();
    wait_ready(&first);
    assert!(projection_contains(
        &database,
        "retained unreadable sentinel"
    ));
    assert!(projection_contains(&database, "TODO one"));
    let unreadable_entry = first
        .list_pages()
        .into_iter()
        .find(|entry| entry.path == unreadable_path)
        .unwrap();
    release_projection(&first);
    drop(first);

    std::fs::write(root.join("pages/one.md"), "- TODO readable replacement\n").unwrap();
    std::fs::write(&unreadable_path, [0xff, 0xfe, 0xfd]).unwrap();

    // A real Graph open captures an incomplete parsed snapshot: the healthy
    // older SQL image stays serviceable, but readiness is withheld until the
    // unreadable page can join a complete source inventory.
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database.clone()).unwrap();
    graph.warm_cache();
    let projection = graph.direct_projection_test().unwrap();
    let reported = reported_projection_failures_test();
    let started = Instant::now();
    loop {
        let pending = projection.shared.pending.lock().unwrap();
        if !pending.has_work() && !projection.shared.worker_busy.load(Ordering::Acquire) {
            break;
        }
        drop(pending);
        assert!(started.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!projection.worker_failed());
    assert_eq!(reported_projection_failures_test(), reported);
    assert!(projection_contains(
        &database,
        "retained unreadable sentinel"
    ));
    assert!(projection_contains(&database, "TODO one"));
    assert!(!projection_contains(&database, "readable replacement"));

    // Restoring and loading the failed page reconciles the parsed cache through
    // the normal Graph path. Its subsequent save may race that full capture;
    // the queue must coalesce both and converge without a test-only enqueue.
    std::fs::write(&unreadable_path, "- restored through Graph\n").unwrap();
    let mut page = graph.load_page(&unreadable_entry).unwrap();
    let baseline = page.rev.clone();
    page.blocks[0].raw = "restored and saved through Graph".into();
    graph.save_page(&page, baseline.as_deref()).unwrap();
    wait_ready(&graph);
    assert!(projection_contains(
        &database,
        "restored and saved through Graph"
    ));
    assert!(projection_contains(&database, "readable replacement"));
    assert!(projection.close_and_wait_for_worker(Duration::from_secs(3)));
    std::fs::remove_dir_all(root).unwrap();
}

/// GH #543 (re-audit A2-N2): `retained` keeps an unreadable page's EXISTING
/// rows — but on a COLD or just-reset index that page has no rows at all, and
/// naming it in the order inventory broke the storage order reconciler's
/// exact-cover requirement. The worker read that bookkeeping mismatch as a
/// failed projection turn: `worker_failed`, readiness revoked, a full rebuild
/// demanded. One unreadable file failed indexing for the whole graph.
#[test]
fn an_unreadable_page_with_no_old_rows_does_not_fail_the_readable_graph() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("unreadable-cold-snapshot");
    std::fs::write(
        root.join("pages/unreadable.md"),
        "- unreadable cold sentinel\n",
    )
    .unwrap();
    let database = root.join("private/projection.sqlite");
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database.clone()).unwrap();
    let (pages, revisions, config) = parsed_snapshot(&graph);
    let omitted = pages
        .iter()
        .find(|(entry, _)| entry.rel_path.ends_with("unreadable.md"))
        .map(|(entry, _)| entry.path.clone())
        .unwrap();
    let pages = Arc::new(
        pages
            .iter()
            .filter(|(entry, _)| entry.path != omitted)
            .cloned()
            .collect::<Vec<_>>(),
    );
    let mut revisions = (*revisions).clone();
    revisions.remove(&omitted);
    let projection = graph.direct_projection_test().unwrap();
    projection.enqueue_full(
        graph.cache_generation(),
        pages,
        Arc::new(revisions),
        config,
        false,
    );
    wait_ready(&graph);
    assert!(projection.worker_available() && !projection.worker_failed());
    assert!(projection_contains(&database, "TODO one"));
    assert!(!projection_contains(&database, "unreadable cold sentinel"));
    assert!(projection.close_and_wait_for_worker(Duration::from_secs(3)));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_full_snapshot_older_than_the_queue_does_not_roll_it_back() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("gh543-stale-full");
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let projection = graph.direct_projection_test().unwrap();

    // A complete parsed snapshot of the graph as it is now.
    let mut pages = Vec::new();
    let mut revisions = HashMap::new();
    for entry in graph.list_pages() {
        revisions.insert(
            entry.path.clone(),
            graph.load_page(&entry).unwrap().rev.unwrap(),
        );
        let mut document = crate::doc::parse(&std::fs::read_to_string(&entry.path).unwrap());
        crate::model::assign_doc_runtime_ids(&mut document.roots, &entry.rel_path);
        pages.push((entry, Arc::new(document)));
    }
    let pages = Arc::new(pages);
    let revisions = Arc::new(revisions);
    let config = Arc::clone(
        &projection
            .shared
            .committed_registry
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .config,
    );
    let stale_generation = graph.cache_generation();

    // A save lands while that snapshot is in the caller's hands.
    let entry = graph.list_pages().into_iter().next().unwrap();
    let mut page = graph.load_page(&entry).unwrap();
    let baseline = page.rev.clone();
    page.blocks[0].raw = "gh543 stale full sentinel".into();
    graph.save_page(&page, baseline.as_deref()).unwrap();
    let saved_generation = graph.cache_generation();
    assert!(saved_generation > stale_generation);

    // The delayed snapshot arrives, naming the generation it was built for.
    projection.enqueue_full(stale_generation, pages, revisions, config, true);
    // Read the queue OUT of the lock: a panic while holding it wedges the
    // worker on the poisoned mutex and the failure shows up as a hang.
    let latest = projection.shared.pending.lock().unwrap().latest_generation;
    assert!(
        latest >= saved_generation,
        "the queue was rolled back from generation {saved_generation} to {latest}: {}",
        projection.debug_state_test()
    );

    // And the user-visible consequence: the saved text is searchable.
    let mut found = false;
    for _ in 0..600 {
        if graph
            .search("gh543 stale full sentinel", 50)
            .map(|groups| !groups.is_empty())
            .unwrap_or(false)
        {
            found = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        found,
        "the save was lost to a stale snapshot: {}",
        projection.debug_state_test()
    );
}

/// GH #543 (re-audit A2-N1): the rebuild EXCEPTION to the rule above. A reset
/// repair must apply its payload — refusing it leaves an emptied index with
/// nothing queued to fill it — but the payload is a parsed snapshot of an
/// older generation, not a fresh read of disk. Clearing the queue beside it
/// erased a save that had already committed, and the stored revision then
/// asserted the old rows were current, so no later comparison corrected it.
#[test]
fn a_rebuild_does_not_discard_a_save_newer_than_its_snapshot() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("gh543-stale-full-rebuild");
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let projection = graph.direct_projection_test().unwrap();

    let mut pages = Vec::new();
    let mut revisions = HashMap::new();
    for entry in graph.list_pages() {
        revisions.insert(
            entry.path.clone(),
            graph.load_page(&entry).unwrap().rev.unwrap(),
        );
        let mut document = crate::doc::parse(&std::fs::read_to_string(&entry.path).unwrap());
        crate::model::assign_doc_runtime_ids(&mut document.roots, &entry.rel_path);
        pages.push((entry, Arc::new(document)));
    }
    let pages = Arc::new(pages);
    let revisions = Arc::new(revisions);
    let config = Arc::clone(
        &projection
            .shared
            .committed_registry
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .config,
    );
    let stale_generation = graph.cache_generation();

    // The save lands while the repair holds its snapshot.
    let entry = graph.list_pages().into_iter().next().unwrap();
    let mut page = graph.load_page(&entry).unwrap();
    let baseline = page.rev.clone();
    page.blocks[0].raw = "gh543 rebuild sentinel".into();
    graph.save_page(&page, baseline.as_deref()).unwrap();
    let saved_generation = graph.cache_generation();
    assert!(saved_generation > stale_generation);

    // Drain the save BEFORE the stale snapshot arrives. This is the case the
    // queue cannot repair by replaying its own deltas: the worker takes a
    // delta the instant it starts its turn, so by now the save is in neither
    // the snapshot nor the queue, and only refusing the snapshot keeps it
    // (third audit A3-N1). Without this wait the interleaving was reached
    // barely half the time and the test passed by luck.
    let mut drained = false;
    for _ in 0..600 {
        let quiet = {
            let pending = projection.shared.pending.lock().unwrap();
            pending.deltas.is_empty() && pending.full.is_none()
        };
        if quiet
            && !projection
                .shared
                .worker_busy
                .load(std::sync::atomic::Ordering::Acquire)
            && graph
                .search("gh543 rebuild sentinel", 50)
                .map(|groups| !groups.is_empty())
                .unwrap_or(false)
        {
            drained = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        drained,
        "the save never reached the index, so the drained case was never set up: {}",
        projection.debug_state_test()
    );

    // A reset repair whose snapshot predates that save. The payload is
    // refused, and the rebuild obligation must go with it rather than latch.
    projection.request_rebuild();
    projection.enqueue_full(stale_generation, pages, revisions, config, true);
    assert!(
        !projection.shared.pending.lock().unwrap().rebuild,
        "the refused payload left the rebuild latched with nothing to ride in on: {}",
        projection.debug_state_test()
    );
    let latest = projection.shared.pending.lock().unwrap().latest_generation;
    assert!(
        latest >= saved_generation,
        "the rebuild rolled the queue back from generation {saved_generation} to {latest}: {}",
        projection.debug_state_test()
    );

    let mut found = false;
    for _ in 0..600 {
        if graph
            .search("gh543 rebuild sentinel", 50)
            .map(|groups| !groups.is_empty())
            .unwrap_or(false)
        {
            found = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        found,
        "the save was erased by the rebuild's older snapshot: {}",
        projection.debug_state_test()
    );
}

/// Fresh replacement has one storage-owned publication crossing and no active
/// database reset: an unpublished image is the only build target.
#[test]
fn a_fresh_build_has_one_atomic_publication_and_no_active_reset() {
    fn visit(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    visit(&root, &mut files);
    files.sort();

    let resets = files
        .iter()
        .filter(|path| !path.to_string_lossy().ends_with("_tests.rs"))
        .flat_map(|path| {
            let source = std::fs::read_to_string(path).unwrap();
            source
                .lines()
                .enumerate()
                .filter(|(_, line)| line.contains("database.reset("))
                .map(|(index, _)| {
                    format!(
                        "{}:{}",
                        path.file_name().unwrap().to_string_lossy(),
                        index + 1
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    assert!(
        resets.is_empty(),
        "active projection reset returned: {resets:?}"
    );
    let publication_sites = files
        .iter()
        .filter(|path| !path.to_string_lossy().ends_with("_tests.rs"))
        .flat_map(|path| {
            let source = std::fs::read_to_string(path).unwrap();
            source
                .lines()
                .enumerate()
                .filter(|(_, line)| line.contains(".publish_replace_single_writer("))
                .map(|(index, _)| {
                    format!(
                        "{}:{}",
                        path.file_name().unwrap().to_string_lossy(),
                        index + 1
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        publication_sites.len(),
        1,
        "publication sites: {publication_sites:?}"
    );
    assert!(publication_sites[0].starts_with("direct_projection.rs:"));

    let raw_publication_sites = files
        .iter()
        .filter(|path| !path.to_string_lossy().ends_with("_tests.rs"))
        .flat_map(|path| {
            let source = std::fs::read_to_string(path).unwrap();
            source
                .lines()
                .enumerate()
                .filter(|(_, line)| line.contains("replace_from_staged_regular_single_writer"))
                .map(|(index, _)| {
                    format!(
                        "{}:{}",
                        path.file_name().unwrap().to_string_lossy(),
                        index + 1
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert!(
        raw_publication_sites.is_empty(),
        "core bypasses the finalized fresh-stage token: {raw_publication_sites:?}"
    );
}

#[test]
fn owned_stage_cleanup_is_exact_and_runs_on_open() {
    let _serial = serialize_projection_tests();
    let root = scratch("owned-stage-cleanup");
    let private = root.join("private");
    std::fs::create_dir_all(&private).unwrap();
    let owned = ".projection.sqlite.tine-projection-build-0123456789abcdef0123456789abcdef";
    for suffix in ["", "-wal", "-shm", "-journal"] {
        std::fs::write(private.join(format!("{owned}{suffix}")), b"stale").unwrap();
    }
    let unrelated = [
        ".projection.sqlite.tine-projection-build-not-a-uuid",
        ".projection.sqlite.tine-projection-build-0123456789abcdef0123456789abcdef.keep",
        ".projection.sqlite.tine-projection-build-notes",
    ];
    for name in unrelated {
        std::fs::write(private.join(name), b"keep").unwrap();
    }

    let projection = DirectProjection::start(private.join("projection.sqlite")).unwrap();
    let started = Instant::now();
    while private.join(owned).exists() && started.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(1));
    }
    for suffix in ["", "-wal", "-shm", "-journal"] {
        assert!(!private.join(format!("{owned}{suffix}")).exists());
    }
    for name in unrelated {
        assert!(
            private.join(name).exists(),
            "unrelated prefix match was removed: {name}"
        );
    }
    assert!(projection.close_and_wait_for_worker(Duration::from_secs(3)));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cancellation_before_publication_keeps_the_old_image_and_discards_the_stage() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("cancel-before-publication");
    let database = root.join("private/projection.sqlite");
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database.clone()).unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    assert!(projection_contains(&database, "TODO one"));

    std::fs::write(root.join("pages/one.md"), "- TODO before-boundary-new\n").unwrap();
    let (pages, revisions, config) = parsed_snapshot(&graph);
    let projection = graph.direct_projection_test().unwrap();
    let reached = Arc::new(std::sync::Barrier::new(2));
    let release = Arc::new(std::sync::Barrier::new(2));
    *projection.shared.before_fresh_publication.lock().unwrap() = Some(Box::new({
        let reached = Arc::clone(&reached);
        let release = Arc::clone(&release);
        move || {
            reached.wait();
            release.wait();
            Ok(())
        }
    }));
    projection.request_rebuild();
    projection.enqueue_full(graph.cache_generation() + 1, pages, revisions, config, true);
    reached.wait();
    let closing = Arc::clone(&projection);
    let closed =
        std::thread::spawn(move || closing.close_and_wait_for_worker(Duration::from_secs(10)));
    while !projection.shared.pending.lock().unwrap().stop {
        std::thread::sleep(Duration::from_millis(1));
    }
    release.wait();
    assert!(closed.join().unwrap());
    assert!(projection_contains(&database, "TODO one"));
    assert!(!projection_contains(&database, "before-boundary-new"));
    let entries = std::fs::read_dir(database.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(entries
        .iter()
        .all(|name| !name.contains(PROJECTION_STAGE_MARKER)));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cancellation_between_fresh_build_batches_discards_the_partial_stage() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("cancel-between-build-batches");
    let database = root.join("private/projection.sqlite");
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database.clone()).unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    for index in 0..80 {
        std::fs::write(
            root.join("pages").join(format!("batch-{index:03}.md")),
            format!("- TODO staged-batch-{index:03}\n"),
        )
        .unwrap();
    }
    let (pages, revisions, config) = parsed_snapshot(&graph);
    let projection = graph.direct_projection_test().unwrap();
    let reached = Arc::new(std::sync::Barrier::new(2));
    let release = Arc::new(std::sync::Barrier::new(2));
    *projection.shared.after_fresh_build_batch.lock().unwrap() = Some(Box::new({
        let reached = Arc::clone(&reached);
        let release = Arc::clone(&release);
        move || {
            reached.wait();
            release.wait();
        }
    }));
    projection.request_rebuild();
    projection.enqueue_full(graph.cache_generation() + 1, pages, revisions, config, true);
    reached.wait();
    let closing = Arc::clone(&projection);
    let closed =
        std::thread::spawn(move || closing.close_and_wait_for_worker(Duration::from_secs(10)));
    while !projection.shared.pending.lock().unwrap().stop {
        std::thread::sleep(Duration::from_millis(1));
    }
    release.wait();
    assert!(closed.join().unwrap());
    assert!(projection_contains(&database, "TODO one"));
    assert!(!projection_contains(&database, "staged-batch-079"));
    let entries = std::fs::read_dir(database.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(entries
        .iter()
        .all(|name| !name.contains(PROJECTION_STAGE_MARKER)));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn edit_delete_and_rename_during_staged_build_reconcile_after_publication() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("mutations-during-staged-build");
    for index in 0..40 {
        std::fs::write(
            root.join("pages").join(format!("bulk-{index:03}.md")),
            format!("- TODO bulk-{index:03}\n"),
        )
        .unwrap();
    }
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let (pages, revisions, config) = parsed_snapshot(&graph);
    let projection = graph.direct_projection_test().unwrap();
    let reached = Arc::new(std::sync::Barrier::new(2));
    let release = Arc::new(std::sync::Barrier::new(2));
    *projection.shared.after_fresh_build_batch.lock().unwrap() = Some(Box::new({
        let reached = Arc::clone(&reached);
        let release = Arc::clone(&release);
        move || {
            reached.wait();
            release.wait();
        }
    }));
    projection.request_rebuild();
    projection.enqueue_full(graph.cache_generation(), pages, revisions, config, true);
    reached.wait();

    let one = graph
        .list_pages()
        .into_iter()
        .find(|entry| entry.name == "one")
        .unwrap();
    let mut page = graph.load_page(&one).unwrap();
    let baseline = page.rev.clone();
    page.blocks[0].raw = "TODO edit during staged build".into();
    graph.save_page(&page, baseline.as_deref()).unwrap();
    graph.delete_page("two", PageKind::Page).unwrap();
    graph.rename_page("target", "renamed target").unwrap();
    release.wait();
    wait_ready(&graph);

    assert!(!graph
        .search("edit during staged build", 50)
        .unwrap()
        .is_empty());
    assert!(graph.search("DONE two", 50).unwrap().is_empty());
    let names = graph
        .list_pages()
        .into_iter()
        .map(|entry| entry.name)
        .collect::<Vec<_>>();
    assert!(names.iter().any(|name| name == "renamed target"));
    assert!(names.iter().all(|name| name != "target"));
    assert!(projection.close_and_wait_for_worker(Duration::from_secs(3)));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cancellation_after_publication_keeps_the_installed_complete_image() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("cancel-after-publication");
    let database = root.join("private/projection.sqlite");
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database.clone()).unwrap();
    graph.warm_cache();
    wait_ready(&graph);

    std::fs::write(root.join("pages/one.md"), "- TODO after-boundary-new\n").unwrap();
    let (pages, revisions, config) = parsed_snapshot(&graph);
    let projection = graph.direct_projection_test().unwrap();
    let reached = Arc::new(std::sync::Barrier::new(2));
    let release = Arc::new(std::sync::Barrier::new(2));
    *projection.shared.after_fresh_publication.lock().unwrap() = Some(Box::new({
        let reached = Arc::clone(&reached);
        let release = Arc::clone(&release);
        move || {
            reached.wait();
            release.wait();
            Ok(())
        }
    }));
    projection.request_rebuild();
    projection.enqueue_full(graph.cache_generation() + 1, pages, revisions, config, true);
    reached.wait();
    let closing = Arc::clone(&projection);
    let closed =
        std::thread::spawn(move || closing.close_and_wait_for_worker(Duration::from_secs(10)));
    while !projection.shared.pending.lock().unwrap().stop {
        std::thread::sleep(Duration::from_millis(1));
    }
    release.wait();
    assert!(closed.join().unwrap());
    assert!(projection_contains(&database, "after-boundary-new"));
    assert!(
        !graph.direct_projection_ready_test(),
        "a stopped owner publishes no readiness"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn held_alias_reader_is_drained_before_fresh_publication() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("held-alias-reader-drain");
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let projection = graph.direct_projection_test().unwrap();
    let generation = graph.cache_generation();
    let (pages, revisions, config) = parsed_snapshot(&graph);

    let reached = Arc::new(std::sync::Barrier::new(2));
    let release = Arc::new(std::sync::Barrier::new(2));
    *projection
        .shared
        .after_shared_reader_admission
        .lock()
        .unwrap() = Some(Box::new({
        let reached = Arc::clone(&reached);
        let release = Arc::clone(&release);
        move || {
            reached.wait();
            release.wait();
        }
    }));
    let alias_projection = Arc::clone(&projection);
    let alias_read =
        std::thread::spawn(move || alias_projection.page_aliases_with_owners(generation));
    reached.wait();

    let (before_tx, before_rx) = mpsc::channel();
    let (after_tx, after_rx) = mpsc::channel();
    *projection
        .shared
        .before_shared_reader_drain_lock
        .lock()
        .unwrap() = Some(Box::new(move || before_tx.send(()).unwrap()));
    *projection
        .shared
        .after_shared_reader_drain_lock
        .lock()
        .unwrap() = Some(Box::new(move || after_tx.send(()).unwrap()));
    projection.request_rebuild();
    projection.enqueue_full(generation, pages, revisions, config, true);
    before_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        after_rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "publication acquired the reader drain while the alias snapshot was held"
    );

    release.wait();
    let _ = alias_read.join().unwrap();
    after_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    wait_ready(&graph);
    assert!(projection.close_and_wait_for_worker(Duration::from_secs(3)));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn delayed_shared_reader_rechecks_readiness_before_opening_a_connection() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("delayed-shared-reader-admission");
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let projection = graph.direct_projection_test().unwrap();
    let generation = graph.cache_generation();
    let (pages, revisions, config) = parsed_snapshot(&graph);

    let reached = Arc::new(std::sync::Barrier::new(2));
    let release = Arc::new(std::sync::Barrier::new(2));
    *projection
        .shared
        .before_shared_reader_admission
        .lock()
        .unwrap() = Some(Box::new({
        let reached = Arc::clone(&reached);
        let release = Arc::clone(&release);
        move || {
            reached.wait();
            release.wait();
        }
    }));
    let delayed_projection = Arc::clone(&projection);
    let delayed = std::thread::spawn(move || delayed_projection.referenced_page_names(generation));
    reached.wait();

    let publication_reached = Arc::new(std::sync::Barrier::new(2));
    let publication_release = Arc::new(std::sync::Barrier::new(2));
    *projection.shared.before_fresh_publication.lock().unwrap() = Some(Box::new({
        let publication_reached = Arc::clone(&publication_reached);
        let publication_release = Arc::clone(&publication_release);
        move || {
            publication_reached.wait();
            publication_release.wait();
            Ok(())
        }
    }));
    projection.request_rebuild();
    projection.enqueue_full(generation, pages, revisions, config, true);
    publication_reached.wait();

    release.wait();
    assert!(delayed.join().unwrap().is_none());
    assert!(
        projection.shared.reader.lock().unwrap().is_none(),
        "a caller delayed across readiness withdrawal reopened the old destination"
    );
    publication_release.wait();
    wait_ready(&graph);
    assert!(projection.close_and_wait_for_worker(Duration::from_secs(3)));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn failures_on_either_side_of_publication_preserve_the_boundary_image() {
    let _serial = serialize_projection_tests();
    for after_publication in [false, true] {
        let tag = if after_publication {
            "failure-after-publication"
        } else {
            "failure-before-publication"
        };
        let root = r6_graph(tag);
        let database = root.join("private/projection.sqlite");
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        std::fs::write(root.join("pages/one.md"), format!("- TODO {tag}-new\n")).unwrap();
        let (pages, revisions, config) = parsed_snapshot(&graph);
        let projection = graph.direct_projection_test().unwrap();
        let injected = Box::new(|| Err("injected publication-boundary failure".to_owned()));
        if after_publication {
            *projection.shared.after_fresh_publication.lock().unwrap() = Some(injected);
        } else {
            *projection.shared.before_fresh_publication.lock().unwrap() = Some(injected);
        }
        projection.request_rebuild();
        projection.enqueue_full(graph.cache_generation() + 1, pages, revisions, config, true);
        let started = Instant::now();
        while !projection.worker_failed() && started.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            projection.worker_failed(),
            "injected failure was not observed"
        );
        assert_eq!(
            projection_contains(&database, tag),
            after_publication,
            "the destination must be old before the boundary and newly installed after it"
        );
        assert_eq!(
            projection_contains(&database, "TODO one"),
            !after_publication,
            "publication boundary selected the wrong coherent image"
        );
        let entries = std::fs::read_dir(database.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(entries
            .iter()
            .all(|name| !name.contains(PROJECTION_STAGE_MARKER)));
        assert!(projection.close_and_wait_for_worker(Duration::from_secs(3)));
        std::fs::remove_dir_all(root).unwrap();
    }
}

/// Two publications deliberately write a file's own parsed-cache slot while
/// leaving the day's NAME to the canonical file: the path-pinned save
/// (`save_path.rs`) and the shadow branch of `sync_file_content`. Both are safe
/// only because the page cache's `by_name` map is not how a page is found by
/// name — `find_entry` builds its own index over `list_pages` and prefers the
/// date-stem file (`lookup.rs`). That is an architectural claim those two
/// comments rest on, so it is checked rather than asserted: `by_name` may be
/// written, never read, outside this cache's own construction. (The seventh
/// audit was right that an earlier version of those comments named the wrong
/// mechanism — `or_insert` in vector order, which does not decide public
/// lookup — and a wrong comment is what mistrains the next reader.)
#[test]
fn a_page_cache_by_name_map_is_not_a_lookup() {
    fn visit(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    visit(&root, &mut files);
    files.sort();

    let mut sites = Vec::new();
    for path in files
        .iter()
        .filter(|path| !path.to_string_lossy().ends_with("_tests.rs"))
    {
        let source = std::fs::read_to_string(path).unwrap();
        for (index, line) in source.lines().enumerate() {
            if !line.contains(".by_name") {
                continue;
            }
            // Writing it is how the map is built; reading it would make the
            // vector's order decide which file a name opens.
            let writes = line.contains(".by_name.entry(") || line.contains(".by_name.insert(");
            if !writes {
                sites.push(format!(
                    "{}:{}",
                    path.file_name().unwrap().to_string_lossy(),
                    index + 1
                ));
            }
        }
    }

    assert!(
        sites.is_empty(),
        "the page cache's `by_name` map is written, never read: a page is found \
         by name through `find_entry`, which builds its own index and prefers \
         the date-stem file, and that is what lets a duplicate day's stray be \
         saved and reconciled into its own slot without taking the day from the \
         canonical file (GH #543). Reading `by_name` would hand that decision \
         to whatever order the cache vector happens to be in. Found: {sites:?}"
    );
}

/// A file that leaves the graph is retired BY PATH, with the entry the caller
/// already holds. `cache_remove(name, kind, None)` retires only what it can
/// find, and with a cold parsed cache and no current page-list memo it finds
/// nothing — so it falls through to marking the index stale with no producer
/// queued, and search goes on answering from a file that is now in the trash
/// (sixth audit A6-N3, where the PDF highlight migration did exactly this).
/// The rule is checkable, so it is checked here rather than written in a
/// comment: a function that MOVES graph text may not retire by name without
/// handing over the entry it moved. Blessed exemplars:
/// `journals.rs::trash_journal_file` (`cache_remove_path`) and
/// `page_rename.rs::delete_page_expected` (`cache_remove(.., removed)`).
#[test]
fn a_moved_file_is_never_retired_by_a_lookup_that_can_come_back_empty() {
    fn visit(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/model");
    let mut files = Vec::new();
    visit(&root, &mut files);
    files.sort();

    let mut offenders = Vec::new();
    for path in files
        .iter()
        .filter(|path| !path.to_string_lossy().ends_with("_tests.rs"))
    {
        let source = std::fs::read_to_string(path).unwrap();
        // Impl-level functions start at four spaces; that is enough structure to
        // attribute a call to the function that makes it.
        let starts: Vec<usize> = source
            .lines()
            .enumerate()
            .filter(|(_, line)| {
                line.starts_with("    fn ")
                    || line.starts_with("    pub fn ")
                    || line.starts_with("    pub(super) fn ")
                    || line.starts_with("    pub(crate) fn ")
            })
            .map(|(index, _)| index)
            .collect();
        let lines: Vec<&str> = source.lines().collect();
        for (position, start) in starts.iter().enumerate() {
            let end = starts.get(position + 1).copied().unwrap_or(lines.len());
            let body = lines[*start..end].join("\n");
            if !body.contains("graph_text_move_") {
                continue;
            }
            // The third argument is the entry the caller holds. Written out, a
            // retirement that hands over nothing reads as `, None)`.
            if body.contains("self.cache_remove(") && body.contains("None,\n") {
                offenders.push(format!(
                    "{}:{}",
                    path.file_name().unwrap().to_string_lossy(),
                    start + 1
                ));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "a function that moves graph text retires the file it moved by PATH \
         (`cache_remove_path`), or by name WITH the entry it holds — never \
         `cache_remove(name, kind, None)`, which retires nothing when the \
         parsed cache is cold and leaves search answering from a trashed file \
         with no producer queued to correct it (GH #543, sixth audit A6-N3). \
         Imitate `journals.rs::trash_journal_file`. Offenders: {offenders:?}"
    );
}

/// A rename changes the page SET, so the committed image stops matching the
/// inventory this session validated — and nothing queues a producer for it.
/// Partial admission would then let search keep ANSWERING from that image,
/// with the renamed page's old name and path, indefinitely: a successful
/// answer never reaches the repair that a refusal starts, so nothing ever
/// converged it (third audit A3-F1). A delete has always published its own
/// delta; a rename published none.
#[test]
fn a_rename_converges_search_instead_of_answering_with_the_old_name() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("gh543-rename-converges");
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let projection = graph.direct_projection_test().unwrap();

    let names = |graph: &Graph| {
        graph.search("target", 20).map(|groups| {
            let mut names = groups
                .iter()
                .map(|group| group.page.clone())
                .collect::<Vec<_>>();
            names.sort();
            names.dedup();
            names
        })
    };
    assert!(
        names(&graph).unwrap().iter().any(|name| name == "target"),
        "the fixture never indexed the page this test renames"
    );

    let before = projection.shared.pending.lock().unwrap().latest_generation;

    // No `warm_cache` after this: the point is that the rename itself leaves a
    // producer behind, so the projection converges on its own.
    graph.rename_page("target", "renamed target").unwrap();

    assert!(
        projection.shared.pending.lock().unwrap().latest_generation > before,
        "the rename queued nothing, so nothing will ever correct the index: {}",
        projection.debug_state_test()
    );

    let mut converged = false;
    for _ in 0..600 {
        if names(&graph)
            .map(|names| names.iter().any(|name| name == "renamed target"))
            .unwrap_or(false)
        {
            converged = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        converged,
        "search never learned the new page name: {}",
        projection.debug_state_test()
    );

    let _ = std::fs::remove_dir_all(root);
}

/// The page-set family: a merge retires its source, a rescue retires the old
/// path and publishes the new one. Each of these moved a physical page out of
/// the live graph and told the index only that *something* had changed, so the
/// index kept rows for a file that no longer existed — with nothing queued, no
/// progress shown, and searches answering from it indefinitely, because a
/// successful answer never reaches the repair a refusal would start (fourth
/// audit A4-N1). A rename had the same hole (A3-F1); this is the family it
/// belonged to.
#[test]
fn a_merge_and_a_rescue_tell_the_index_what_they_changed() {
    let _serial = serialize_projection_tests();
    let root = scratch("gh543-page-set-family");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(root.join("pages/source.md"), "- zebrafish source\n").unwrap();
    std::fs::write(root.join("pages/destination.md"), "- destination body\n").unwrap();
    std::fs::write(root.join("pages/stray.md"), "- narwhal stray\n").unwrap();
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let projection = graph.direct_projection_test().unwrap();

    let pages = |token: &str| {
        graph.search(token, 20).map(|groups| {
            let mut names = groups
                .iter()
                .map(|group| group.page.clone())
                .collect::<Vec<_>>();
            names.sort();
            names.dedup();
            names
        })
    };
    let settle = |want: &str, token: &str| {
        for _ in 0..600 {
            if pages(token)
                .map(|names| names == vec![want.to_owned()])
                .unwrap_or(false)
            {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    };

    assert_eq!(pages("zebrafish").unwrap(), vec!["source".to_owned()]);
    let before = projection.shared.pending.lock().unwrap().latest_generation;
    graph
        .merge_pages("pages/source.md", "pages/destination.md")
        .unwrap();
    assert!(
        projection.shared.pending.lock().unwrap().latest_generation > before,
        "the merge queued nothing to retire the source it trashed: {}",
        projection.debug_state_test()
    );
    assert!(
        !root.join("pages/source.md").exists(),
        "the fixture did not actually merge"
    );
    assert!(
        settle("destination", "zebrafish"),
        "search still offers the merged-away page: {:?} — {}",
        pages("zebrafish"),
        projection.debug_state_test()
    );

    assert_eq!(pages("narwhal").unwrap(), vec!["stray".to_owned()]);
    let before = projection.shared.pending.lock().unwrap().latest_generation;
    graph
        .rename_file_to_page("pages/stray.md", "rescued")
        .unwrap();
    assert!(
        projection.shared.pending.lock().unwrap().latest_generation > before,
        "the rescue queued nothing, so nothing will ever correct the index: {}",
        projection.debug_state_test()
    );
    assert!(
        settle("rescued", "narwhal"),
        "search still offers the rescued page under its old path: {:?} — {}",
        pages("narwhal"),
        projection.debug_state_test()
    );

    let _ = std::fs::remove_dir_all(root);
}

/// A mutation that changes the page SET publishes its whole change through ONE
/// call. Published a delta at a time, the queue empties between them and the
/// worker announces readiness for a generation that is only half enqueued — a
/// rename's `Delete` drained on its own left the index `ready` over a graph
/// missing the page entirely (fourth audit A4-N2). The one-page save and delete
/// paths keep their own single-delta entry points; these three do not.
///
/// This scan proves the SPELLING inside the mutation file, not that the whole
/// publication is atomic: it cannot see a publication these functions reach
/// through `write_page -> cache_upsert`, which is exactly how a merge used to
/// announce its destination while its source was still queued (fifth audit
/// A5-N4). That property is behavioural, and lives in
/// `a_merged_away_source_cannot_be_resurrected_by_a_repair` and
/// `a_failed_merge_puts_the_source_back_in_the_index_too`, where a merge's
/// retirement is published before the destination write and rolled back with
/// it.
#[test]
fn a_page_set_mutation_publishes_through_one_front_door() {
    for file in ["model/page_rename.rs", "model/pages_merge.rs"] {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join(file),
        )
        .unwrap();
        assert!(
            source.contains("direct_projection_publish_page_set("),
            "{file} changes the page set but never tells the index what it changed"
        );
        for single in [
            "direct_projection_enqueue_replace(",
            "direct_projection_enqueue_delete(",
        ] {
            assert!(
                !source.contains(single),
                "{file} publishes a page-set change one delta at a time via \
                 `{single}`. Use `direct_projection_publish_page_set`, which takes \
                 the queue lock once: otherwise the worker can drain half the \
                 change and publish readiness over an incomplete generation \
                 (GH #543, fourth audit A4-N2)."
            );
        }
    }
}

/// A duplicate journal day: the canonical date-named file and a title-named
/// stray for the same day, both indexed, with `token` present only in the stray.
fn duplicate_day_projection_graph(tag: &str, token: &str) -> PathBuf {
    let root = scratch(tag);
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("logseq/config.edn"),
        "{:journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("journals/2026_06_26.md"),
        "- shared line\n- only in canonical\n",
    )
    .unwrap();
    std::fs::write(
        root.join("journals/Friday, 26-06-2026.md"),
        format!("- shared line\n- {token} only in stray\n"),
    )
    .unwrap();
    root
}

fn attached_graph(root: &Path) -> Graph {
    let graph = Graph::open(root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    graph
}

/// How many indexed BLOCKS carry `token`. Counting groups is not enough for a
/// duplicate journal day: the stray and the canonical file answer under the
/// same journal title, so a ghost row hides inside one group.
fn hits(graph: &Graph, token: &str) -> usize {
    graph
        .search(token, 50)
        .map(|groups| groups.iter().map(|group| group.blocks.len()).sum())
        .unwrap_or(usize::MAX)
}

fn settle_hits(graph: &Graph, token: &str, want: usize) -> bool {
    for _ in 0..600 {
        if hits(graph, token) == want {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn a_move_whose_durability_fails_after_the_rename_still_converges(operation: &str) {
    let _serial = serialize_projection_tests();
    let root = duplicate_day_projection_graph(&format!("move-durability-{operation}"), "quokka");
    std::fs::write(root.join("pages/source.md"), "- numbat source\n").unwrap();
    std::fs::write(root.join("pages/destination.md"), "- destination body\n").unwrap();
    let graph = attached_graph(&root);
    graph.with_pages(|_| ());
    wait_ready(&graph);
    crate::model::fail_graph_text_directory_sync_after_mutation();
    let outcome = match operation {
        "merge" => graph.merge_pages("pages/source.md", "pages/destination.md"),
        "rescue" => graph.rename_file_to_page("pages/source.md", "rescued"),
        "delete" => graph.delete_page("source", PageKind::Page),
        "journal-trash" => graph.trash_journal_file("Friday, 26-06-2026.md"),
        _ => unreachable!(),
    };
    assert!(outcome.is_err(), "durability failure must reach the caller");
    let retired = if operation == "journal-trash" {
        "journals/Friday, 26-06-2026.md"
    } else {
        "pages/source.md"
    };
    assert!(
        !root.join(retired).exists(),
        "the rename must have landed: {outcome:?}"
    );
    let token = if operation == "journal-trash" {
        "quokka"
    } else {
        "numbat"
    };
    if operation == "rescue" {
        assert!(root.join("pages/rescued.md").is_file());
        for _ in 0..600 {
            if graph
                .search(token, 20)
                .unwrap()
                .iter()
                .any(|g| g.page == "rescued")
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    } else {
        assert!(
            settle_hits(&graph, token, 0),
            "{operation}: retired source still searchable: {}",
            graph.direct_projection_test().unwrap().debug_state_test()
        );
    }
    wait_ready(&graph);
    for _ in 0..5 {
        let groups = graph.search(token, 20).unwrap();
        let names: Vec<_> = groups.iter().map(|g| g.page.as_str()).collect();
        assert_eq!(
            names,
            if operation == "rescue" {
                vec!["rescued"]
            } else {
                vec![]
            },
            "{operation}: search must describe the live paths"
        );
    }
    graph.with_pages(|pages| {
        assert!(
            !pages.iter().any(|(entry, _)| entry.rel_path == retired),
            "parsed cache must not resurrect the retired source"
        )
    });
}

#[test]
fn a_merge_whose_durability_fails_after_the_move_stops_serving_the_source() {
    a_move_whose_durability_fails_after_the_rename_still_converges("merge");
}
#[test]
fn a_rescue_whose_durability_fails_after_the_move_answers_at_the_new_name() {
    a_move_whose_durability_fails_after_the_rename_still_converges("rescue");
}
#[test]
fn a_delete_whose_durability_fails_after_the_move_stops_serving_the_page() {
    a_move_whose_durability_fails_after_the_rename_still_converges("delete");
}
#[test]
fn a_journal_trash_whose_durability_fails_after_the_move_stops_serving_the_file() {
    a_move_whose_durability_fails_after_the_rename_still_converges("journal-trash");
}

/// Folding a duplicate day's stray into the canonical file moves the stray to
/// the trash. Nothing retired its rows, so the token the merge had just folded
/// in was served TWICE — once from the live canonical file and once from a file
/// that no longer exists — with the index `ready`, nothing queued and no
/// progress shown. A successful answer never reaches the repair a refusal would
/// start, so it never went away (GH #543, fifth audit A5-N1).
#[test]
fn a_resolved_duplicate_day_retires_the_stray_it_trashed() {
    let _serial = serialize_projection_tests();
    let root = duplicate_day_projection_graph("gh543-duplicate-day-resolve", "zebrafish");
    let graph = attached_graph(&root);
    assert_eq!(hits(&graph, "zebrafish"), 1, "the stray starts indexed");

    let diff = graph
        .duplicate_journal_diff("journals/2026_06_26.md", "journals/Friday, 26-06-2026.md")
        .unwrap()
        .expect("a same-format pair diffs");
    fn keep_both(
        rows: &[crate::sync_diff::DiffRow],
        out: &mut std::collections::HashMap<String, String>,
    ) {
        for row in rows {
            if row.kind != crate::sync_diff::RowKind::Unchanged {
                out.insert(row.id.clone(), "both".to_string());
            }
            keep_both(&row.children, out);
        }
    }
    let mut decisions = std::collections::HashMap::new();
    keep_both(&diff.rows, &mut decisions);
    graph
        .resolve_duplicate_journal_day(
            "journals/2026_06_26.md",
            "journals/Friday, 26-06-2026.md",
            &decisions,
            &diff.base_rev,
            &diff.conflict_rev,
            "union",
        )
        .unwrap();
    assert!(
        !root.join("journals/Friday, 26-06-2026.md").exists(),
        "the fixture did not actually resolve the day"
    );
    // The fold is the precondition the ghost hides behind: without it the
    // token would still exist exactly once and the count below would pass
    // while the stray's rows survived.
    assert!(
        std::fs::read_to_string(root.join("journals/2026_06_26.md"))
            .unwrap()
            .contains("zebrafish"),
        "the fixture did not fold the stray's line into the canonical file"
    );

    let projection = graph.direct_projection_test().unwrap();
    assert!(
        settle_hits(&graph, "zebrafish", 1),
        "the trashed stray is still answering beside the file it was folded \
         into: {} block(s) — {}",
        hits(&graph, "zebrafish"),
        projection.debug_state_test()
    );
    let _ = std::fs::remove_dir_all(root);
}

/// Trashing one file of a duplicate day is the other half of the same
/// affordance, and it published nothing at all: no generation advance, no
/// delta, no invalidation. Search went on finding the trashed file's text.
#[test]
fn a_trashed_journal_file_leaves_search() {
    let _serial = serialize_projection_tests();
    let root = duplicate_day_projection_graph("gh543-journal-trash", "narwhal");
    let graph = attached_graph(&root);
    assert_eq!(hits(&graph, "narwhal"), 1, "the stray starts indexed");

    graph.trash_journal_file("Friday, 26-06-2026.md").unwrap();
    assert!(
        !root.join("journals/Friday, 26-06-2026.md").exists(),
        "the fixture did not actually trash the file"
    );

    let projection = graph.direct_projection_test().unwrap();
    assert!(
        settle_hits(&graph, "narwhal", 0),
        "search still answers from the trashed journal file: {} block(s) — {}",
        hits(&graph, "narwhal"),
        projection.debug_state_test()
    );
    let _ = std::fs::remove_dir_all(root);
}

/// Migrating a title-named journal to the graph's filename format MOVES the
/// file. The logical page and its text are unchanged, so search still answers —
/// but from rows keyed to a path that no longer exists, with nothing queued to
/// correct them.
#[test]
fn a_journal_filename_migration_republishes_the_file_it_moved() {
    let _serial = serialize_projection_tests();
    let root = scratch("gh543-journal-migration");
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("logseq/config.edn"),
        "{:journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("journals/Friday, 26-06-2026.md"),
        "- okapi only here\n",
    )
    .unwrap();
    let graph = attached_graph(&root);
    assert_eq!(hits(&graph, "okapi"), 1, "the journal starts indexed");

    assert_eq!(graph.migrate_journal_filenames_checked().unwrap(), 1);
    assert!(root.join("journals/2026_06_26.md").exists());
    assert!(!root.join("journals/Friday, 26-06-2026.md").exists());

    // The stale row is keyed by the retired PATH, and the logical page and its
    // text are unchanged — so the migration alone shows nothing. It surfaces on
    // the next ordinary edit: that save publishes the NEW path, the retired
    // path's row keeps answering beside it, and the graph now has two rows for
    // one file.
    let mut page = graph
        .load_by_path("journals/2026_06_26.md")
        .unwrap()
        .expect("the migrated journal loads at its new path");
    let baseline = page.rev.clone();
    page.blocks[0].raw = "tapir only here".into();
    graph.save_page(&page, baseline.as_deref()).unwrap();

    let projection = graph.direct_projection_test().unwrap();
    assert!(
        settle_hits(&graph, "tapir", 1),
        "the saved text never reached the index: {} block(s) — {}",
        hits(&graph, "tapir"),
        projection.debug_state_test()
    );
    assert_eq!(
        hits(&graph, "okapi"),
        0,
        "the index still answers from the migrated-away path: {}",
        projection.debug_state_test()
    );
    let _ = std::fs::remove_dir_all(root);
}

/// `graph_text_move_noreplace` renames FIRST and then does fallible durability
/// work, so an `Err` from it does not mean the file stayed put. The migration
/// filtered its moves on `.is_ok()`, so a rename that committed and then failed
/// to flush its directory was recorded as not having happened: nothing was
/// published, the retired path went on describing the file in search, and the
/// call reported `Ok(0)` — "nothing migrated" — for a file that had moved
/// (GH #543, seventh audit A7-N3).
#[test]
fn a_migration_publishes_a_move_whose_durability_step_failed() {
    let _serial = serialize_projection_tests();
    let root = scratch("gh543-journal-migration-partial");
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("logseq/config.edn"),
        "{:journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("journals/Friday, 26-06-2026.md"),
        "- okapi only here\n",
    )
    .unwrap();
    let graph = attached_graph(&root);
    assert_eq!(hits(&graph, "okapi"), 1, "the journal starts indexed");

    crate::model::fail_next_projection_directory_sync();
    let outcome = graph.migrate_journal_filenames_checked();
    assert!(
        root.join("journals/2026_06_26.md").is_file()
            && !root.join("journals/Friday, 26-06-2026.md").exists(),
        "the fixture must leave the file MOVED with the call having failed: \
         outcome={outcome:?}"
    );

    // The retired path is the observable: nothing published means search keeps
    // describing it, and the next ordinary edit of the day then publishes the
    // new path beside the ghost, so one file answers twice.
    let mut page = graph
        .load_by_path("journals/2026_06_26.md")
        .unwrap()
        .expect("the migrated journal loads at its new path");
    let baseline = page.rev.clone();
    page.blocks[0].raw = "tapir only here".into();
    graph.save_page(&page, baseline.as_deref()).unwrap();

    let projection = graph.direct_projection_test().unwrap();
    assert!(
        settle_hits(&graph, "tapir", 1),
        "the saved text never reached the index: {} block(s) — {}",
        hits(&graph, "tapir"),
        projection.debug_state_test()
    );
    assert_eq!(
        hits(&graph, "okapi"),
        0,
        "the migration moved the file and published nothing, so search still \
         answers from the path it moved away from: {}",
        projection.debug_state_test()
    );
    let _ = std::fs::remove_dir_all(root);
}

/// A path-pinned save — a duplicate day's stray, which deliberately never
/// enters the `(kind,name)` parsed cache — wrote its bytes and published
/// nothing. The projection's rows are keyed by PATH, so that exclusion had
/// silently become "this file's index is never updated again": search kept
/// serving the pre-save text, with the index idle, validated and ready
/// (GH #543, fifth audit A5-N2).
#[test]
fn a_path_pinned_save_reaches_search() {
    let _serial = serialize_projection_tests();
    let root = duplicate_day_projection_graph("gh543-pinned-save", "wombat");
    let graph = attached_graph(&root);
    assert_eq!(hits(&graph, "wombat"), 1, "the stray starts indexed");

    let mut page = graph
        .load_by_path("journals/Friday, 26-06-2026.md")
        .unwrap()
        .expect("the stray loads by path");
    let baseline = page.rev.clone();
    let edited = page
        .blocks
        .iter()
        .position(|block| block.raw.contains("wombat"))
        .expect("the token block");
    page.blocks[edited].raw = "kiwifruit only in stray".into();
    graph.save_page(&page, baseline.as_deref()).unwrap();
    assert!(
        std::fs::read_to_string(root.join("journals/Friday, 26-06-2026.md"))
            .unwrap()
            .contains("kiwifruit"),
        "the fixture did not actually save"
    );

    let projection = graph.direct_projection_test().unwrap();
    assert!(
        settle_hits(&graph, "kiwifruit", 1),
        "the saved text never reached the index: {} block(s) — {}",
        hits(&graph, "kiwifruit"),
        projection.debug_state_test()
    );
    assert_eq!(
        hits(&graph, "wombat"),
        0,
        "the index still serves the pre-save text of a file that was saved"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// Deleting the merged-away source's ROWS is not the same as retiring the page.
/// The parsed whole-graph cache is an authoritative snapshot producer: a query
/// read failure makes it reset the index and republish that snapshot at the
/// CURRENT generation, so a source left in the cache walks straight back into
/// search — past the older-generation guard, which sees nothing wrong
/// (GH #543, fifth audit A5-N3).
#[test]
fn a_merged_away_source_cannot_be_resurrected_by_a_repair() {
    let _serial = serialize_projection_tests();
    let root = scratch("gh543-merge-repair-resurrection");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(root.join("pages/source.md"), "- pangolin source\n").unwrap();
    std::fs::write(root.join("pages/destination.md"), "- destination body\n").unwrap();
    let graph = attached_graph(&root);
    // The optional parsed snapshot exists — this is the producer under test.
    graph.with_pages(|pages| assert_eq!(pages.len(), 2));

    graph
        .merge_pages("pages/source.md", "pages/destination.md")
        .unwrap();
    let names = |graph: &Graph| {
        graph
            .search("pangolin", 20)
            .map(|groups| {
                let mut names = groups
                    .iter()
                    .map(|group| group.page.clone())
                    .collect::<Vec<_>>();
                names.sort();
                names.dedup();
                names
            })
            .unwrap_or_default()
    };
    let settled = (0..600).any(|_| {
        if names(&graph) == vec!["destination".to_owned()] {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
        false
    });
    assert!(
        settled,
        "the merge itself did not converge: {:?}",
        names(&graph)
    );

    graph.direct_projection_inject_read_failure_test();
    let projection = graph.direct_projection_test().unwrap();
    for _ in 0..300 {
        assert_ne!(
            names(&graph),
            vec!["destination".to_owned(), "source".to_owned()],
            "a repair republished the merged-away source from the parsed cache: {}",
            projection.debug_state_test()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        names(&graph),
        vec!["destination".to_owned()],
        "search lost the merged text: {}",
        projection.debug_state_test()
    );
    let _ = std::fs::remove_dir_all(root);
}

/// Publishing a pinned save's rows is not enough if nothing else agrees the
/// save happened. The parsed whole-graph snapshot still held the file's old
/// document at the SAME generation, so one query read error rebuilt the index
/// from it and undid the save; and an older captured snapshot was still
/// accepted, because its generation had not been outrun (GH #543, sixth audit
/// A6-N1).
#[test]
fn a_pinned_save_cannot_be_undone_by_another_producer() {
    let _serial = serialize_projection_tests();
    let root = duplicate_day_projection_graph("gh543-pinned-save-producers", "dugong");
    let graph = attached_graph(&root);
    // The optional parsed snapshot exists — it is one of the producers here.
    graph.with_pages(|pages| assert_eq!(pages.len(), 2));
    let before = graph.cache_generation();

    let mut page = graph
        .load_by_path("journals/Friday, 26-06-2026.md")
        .unwrap()
        .expect("the stray loads by path");
    let baseline = page.rev.clone();
    let edited = page
        .blocks
        .iter()
        .position(|block| block.raw.contains("dugong"))
        .expect("the token block");
    page.blocks[edited].raw = "quokka only in stray".into();
    graph.save_page(&page, baseline.as_deref()).unwrap();
    assert!(
        settle_hits(&graph, "quokka", 1),
        "the saved text never reached the index: {} block(s)",
        hits(&graph, "quokka")
    );
    // Producers are admitted by generation. A publication that leaves the
    // generation where it found it cannot outrank a snapshot captured before
    // the save.
    assert!(
        graph.cache_generation() > before,
        "a pinned save that changed bytes must advance the generation"
    );

    let projection = graph.direct_projection_test().unwrap();
    graph.direct_projection_inject_read_failure_test();
    for _ in 0..300 {
        // A refusal (`usize::MAX`) is the index declining to answer while it
        // repairs — designed behaviour, not a resurrection. What must never
        // happen is the overwritten text coming back as an ANSWER.
        let dugong = hits(&graph, "dugong");
        assert!(
            dugong == 0 || dugong == usize::MAX,
            "a repair rebuilt the index from a snapshot that never heard about \
             the save: {dugong} block(s), {}",
            projection.debug_state_test()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        settle_hits(&graph, "quokka", 1),
        "the repaired index lost the saved text: {} block(s), {}",
        hits(&graph, "quokka"),
        projection.debug_state_test()
    );
    let _ = std::fs::remove_dir_all(root);
}

/// An external editor or a sync tool writing to one file of a duplicate
/// journal day is reconciled — and was then dropped on the floor, by the same
/// conflation the pinned save had: the file is deliberately kept out of the
/// by-name cache, and the index was only ever told about changes from there
/// (GH #543, sixth audit A6-N2).
#[test]
fn an_external_edit_to_a_duplicate_days_stray_reaches_search() {
    let _serial = serialize_projection_tests();
    let root = duplicate_day_projection_graph("gh543-external-shadow-edit", "caracal");
    let graph = attached_graph(&root);
    assert_eq!(hits(&graph, "caracal"), 1, "the stray starts indexed");

    let stray = root.join("journals/Friday, 26-06-2026.md");
    std::fs::write(&stray, "- shared line\n- axolotl only in stray\n").unwrap();
    graph.sync_file_checked(&stray).unwrap();

    let projection = graph.direct_projection_test().unwrap();
    assert!(
        settle_hits(&graph, "axolotl", 1),
        "the externally delivered edit never reached the index: {} block(s) — {}",
        hits(&graph, "axolotl"),
        projection.debug_state_test()
    );
    assert_eq!(
        hits(&graph, "caracal"),
        0,
        "the index still answers with the text the file had before the edit: {}",
        projection.debug_state_test()
    );

    // A redelivery of the SAME bytes publishes nothing (seventh audit A7-N2).
    // Sync tools redeliver routinely and Tine's own save of this file echoes
    // back through here; the ordinary path suppresses both by comparing the
    // recorded disk revision, and the shadow branch returned before reaching
    // that comparison. Every needless publication bumps the generation, which
    // invalidates every memoized whole-graph result.
    let settled = graph.cache_generation();
    for _ in 0..5 {
        graph.sync_file_checked(&stray).unwrap();
    }
    assert_eq!(
        graph.cache_generation(),
        settled,
        "unchanged redeliveries of the stray each published a replacement and \
         bumped the generation: {}",
        projection.debug_state_test()
    );
    assert_eq!(
        hits(&graph, "axolotl"),
        1,
        "the redeliveries disturbed the index: {}",
        projection.debug_state_test()
    );
    let _ = std::fs::remove_dir_all(root);
}

fn pdf_highlight(id: &str, page: i64, text: &str) -> crate::pdf::Highlight {
    let rect = crate::pdf::Rect {
        top: 1.0,
        left: 2.0,
        width: 3.0,
        height: 4.0,
        source_width: None,
        source_height: None,
    };
    crate::pdf::Highlight {
        id: id.into(),
        page,
        position: crate::pdf::Position {
            page,
            bounding: rect.clone(),
            rects: vec![rect],
        },
        color: "yellow".into(),
        text: Some(text.to_owned()),
        image: None,
    }
}

/// Migrating a legacy PDF highlight page to its OG-compatible key trashes the
/// old page and calls the blessed by-NAME removal — which, with no parsed
/// whole-graph cache to name the file from, removed nothing and queued
/// nothing. Search answered from both pages, one of them already in the trash
/// (GH #543, sixth audit A6-N3).
#[test]
fn a_migrated_legacy_highlight_page_leaves_search() {
    let _serial = serialize_projection_tests();
    let root = scratch("gh543-pdf-legacy-migration");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("assets")).unwrap();
    let pdf = "My Paper.pdf";
    let legacy_key = crate::pdf::legacy_asset_key(pdf);
    let highlight = pdf_highlight(
        "11111111-1111-1111-1111-111111111111",
        3,
        "pangolin legacy text",
    );
    std::fs::write(
        root.join("assets").join(format!("{legacy_key}.edn")),
        crate::pdf::write_highlights(std::slice::from_ref(&highlight), ""),
    )
    .unwrap();
    std::fs::write(
        root.join("pages").join(format!("hls__{legacy_key}.md")),
        crate::doc::serialize(&crate::pdf::hls_page_document(
            pdf,
            "My Paper",
            std::slice::from_ref(&highlight),
        )),
    )
    .unwrap();
    // Deliberately NO parsed whole-graph cache: the ordinary projection warm is
    // the only index, which is exactly the session the by-name removal cannot
    // resolve a path in.
    let graph = Graph::open(&root);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    // A query with no parsed cache is what drives the projection's own warm.
    for _ in 0..1500 {
        if graph.direct_projection_ready_test() {
            break;
        }
        let _ = graph.search("pangolin", 5);
        std::thread::sleep(Duration::from_millis(10));
    }
    wait_ready(&graph);
    assert_eq!(
        hits(&graph, "pangolin"),
        1,
        "the legacy page starts indexed"
    );

    graph
        .write_highlights(pdf, "My Paper", std::slice::from_ref(&highlight), &[])
        .unwrap();
    assert!(
        !root
            .join("pages")
            .join(format!("hls__{legacy_key}.md"))
            .exists(),
        "the fixture did not actually migrate the legacy page"
    );

    let projection = graph.direct_projection_test().unwrap();
    assert!(
        settle_hits(&graph, "pangolin", 1),
        "search answers from both the migrated page and the trashed one: {} \
         block(s) — {}",
        hits(&graph, "pangolin"),
        projection.debug_state_test()
    );
    let _ = std::fs::remove_dir_all(root);
}

/// GH #550: the Journals feed loads its first pages before the warm has
/// validated the reopened projection. Those reads must not turn a clean reopen
/// into a full rebuild -- on a phone that rebuild was the whole 40 s open,
/// repeated on every launch.
#[test]
fn gh550_journal_feed_before_the_warm_keeps_a_clean_reopen_clean() {
    let _serial = serialize_projection_tests();
    let root = scratch("gh550-feed-first");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    for (day, text) in [
        // A future-dated journal: stored first, but never in today's feed.
        // The feed's cold session therefore numbers its pages from 0 while
        // the image holds that future day at position 0.
        ("2027_01_04", "- planned\n"),
        ("2026_09_19", "- TODO older\n"),
        ("2026_09_20", "- yesterday\n  - child\n"),
        ("2026_09_21", "- today [[one]]\n"),
    ] {
        std::fs::write(root.join(format!("journals/{day}.md")), text).unwrap();
    }
    std::fs::write(root.join("pages/one.md"), "- TODO one\n").unwrap();
    std::fs::write(root.join("pages/two.md"), "- DONE two\n").unwrap();
    // Enough ordinary pages that the image stores the journals at positions
    // the feed's cold session would not guess.
    for page in 0..12 {
        std::fs::write(root.join(format!("pages/aa-{page:02}.md")), "- filler\n").unwrap();
    }
    let database = scratch("gh550-feed-first-db").join("projection.sqlite");

    reset_lowerings(&root);
    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        assert_eq!(lowerings(), 18, "the first open lowers every page");
        release_projection(&graph);
    }
    std::thread::sleep(Duration::from_millis(20));

    reset_lowerings(&root);
    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        // Exactly what the app does: the feed loads its window first.
        let cutoff = crate::date::JournalDate {
            year: 2026,
            month: 9,
            day: 21,
        };
        let feed = graph.feed_journals_desc_through(cutoff);
        assert_eq!(
            feed.len(),
            3,
            "the feed sees every journal through the cutoff"
        );
        for entry in &feed {
            graph.load_page(entry).expect("the feed loads its journal");
        }
        // ...and one ordinary page. The cold session numbers these four
        // reads 0..=3, and the image cannot hold all four at exactly those
        // positions (it has a fourth journal and walks each folder whole), so
        // at least one number is already taken by a page this change does not
        // touch: the `UNIQUE constraint failed: pages.position` of GH #550.
        let two = graph
            .entry_for_path(&root.join("pages/two.md"))
            .expect("the page has an entry");
        graph.load_page(&two).expect("an ordinary page loads");
        // Let the worker take the feed's deltas before the warm, as it does
        // on a slow phone, so this does not depend on who wins the race.
        assert!(
            graph.direct_projection_test().unwrap().wait_drained_test(),
            "the feed's deltas must not fail against the reopened image"
        );
        graph.warm_cache();
        wait_ready(&graph);
        assert_eq!(
            lowerings(),
            0,
            "feed reads before the warm must not re-lower an unchanged graph"
        );
        assert_eq!(
            signature(
                &graph
                    .run_query_bounded("(task TODO)", 100, 1_000_000)
                    .expect("the ready projection answers the public bounded route")
                    .groups
            ),
            signature(
                &crate::query::run_query_bounded(&graph, "(task TODO)", 100, 1_000_000).groups
            )
        );
        release_projection(&graph);
    }

    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(database.parent().unwrap());
}

/// GH #543 (indexing audit IT-02): creating a page needs proof that no other
/// file already owns its name. On a reopened graph there is no parsed cache,
/// so creation parsed the whole graph on its own -- during the launch check a
/// second whole-graph pass ahead of it, and after a clean check on the first
/// creation of every session. The ready index holds every page's effective
/// name, so creation now takes its proof from there (waiting for the check
/// like any other graph-wide read) and parses nothing. The proof still
/// refuses a name another file owns through `title::`.
#[test]
fn gh543_creating_a_page_during_the_launch_check_parses_nothing() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("gh543-create-during-warm");
    std::fs::write(
        root.join("pages/aaa claims.md"),
        "title:: Taken\n\n- owner\n",
    )
    .unwrap();
    let database = scratch("gh543-create-during-warm-db").join("projection.sqlite");
    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        release_projection(&graph);
    }
    let graph = Arc::new(Graph::open(&root));
    graph.attach_direct_projection(database).unwrap();
    let pause = graph.pause_next_warm_after_read_test();
    let warmer = {
        let graph = Arc::clone(&graph);
        std::thread::spawn(move || graph.warm_cache())
    };
    pause.reached.wait();
    let creator = {
        let graph = Arc::clone(&graph);
        std::thread::spawn(move || {
            let created = graph.save_page(
                &crate::vocab::markdown_page_dto("Created", "Created", "- TODO new\n").unwrap(),
                None,
            );
            let taken = graph.save_page(
                &crate::vocab::markdown_page_dto("Taken", "Taken", "- second owner\n").unwrap(),
                None,
            );
            (created.map(|_| ()), taken.map(|_| ()))
        })
    };
    std::thread::sleep(Duration::from_millis(100));
    pause.release.wait();
    warmer.join().unwrap();
    let (created, taken) = creator.join().unwrap();
    wait_ready(&graph);

    created.expect("a free name is created");
    let refusal = taken.expect_err("a name another file owns is refused");
    assert_eq!(
        refusal.kind(),
        std::io::ErrorKind::AlreadyExists,
        "{refusal}"
    );
    assert!(!root.join("pages/Taken.md").exists());
    assert_eq!(
        graph.page_build_parses_test(),
        0,
        "creation ran its own whole-graph pass"
    );
    assert!(!graph.has_parsed_cache_test());
    let todos = graph
        .run_query_bounded("(task TODO)", 100, 1_000_000)
        .expect("the ready projection answers")
        .groups;
    assert!(
        todos
            .iter()
            .flat_map(|group| group.blocks.iter())
            .any(|block| block.raw == "TODO new"),
        "the created page is indexed"
    );
    release_projection(&graph);
    let _ = std::fs::remove_dir_all(root);
}

/// GH #543 (indexing audit IT-05): opening a named page -- a restored tab, the
/// configured home page, a favourite, a link -- waited for the whole launch
/// check, because resolving a name went through the graph-wide page list.
/// A page whose file is named for it now opens from that file while the check
/// runs, and resolves to the same file before and after it: when another file
/// claims the same name through `title::`, the file named for the page wins
/// in both paths. Names no file is named for still wait for the list.
#[test]
fn gh543_a_named_page_opens_while_the_launch_check_runs() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("gh543-named-open");
    // Sorts before `target.md` and claims the same name.
    std::fs::write(
        root.join("pages/aaa claims target.md"),
        "title:: target\n\n- the stray\n",
    )
    .unwrap();
    std::fs::write(root.join("pages/area%2Ftopic.md"), "- namespaced\n").unwrap();
    let database = scratch("gh543-named-open-db").join("projection.sqlite");
    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        release_projection(&graph);
    }
    let graph = Arc::new(Graph::open(&root));
    graph.attach_direct_projection(database).unwrap();
    let pause = graph.pause_next_warm_after_read_test();
    let warmer = {
        let graph = Arc::clone(&graph);
        std::thread::spawn(move || graph.warm_cache())
    };
    pause.reached.wait();
    let open_during = |name: &'static str| {
        let (send, receive) = std::sync::mpsc::channel();
        let graph = Arc::clone(&graph);
        let reader =
            std::thread::spawn(move || send.send(graph.load_named(name, PageKind::Page)).unwrap());
        let opened = receive
            .recv_timeout(Duration::from_secs(2))
            .unwrap_or_else(|_| panic!("{name}: opening waited for the paused launch check"));
        reader.join().unwrap();
        opened.unwrap().map(|page| page.path)
    };
    let target_during = open_during("target");
    let namespaced_during = open_during("area/topic");
    pause.release.wait();
    warmer.join().unwrap();
    wait_ready(&graph);

    assert_eq!(target_during, Some("pages/target.md".to_owned()));
    assert_eq!(namespaced_during, Some("pages/area%2Ftopic.md".to_owned()));
    let after = |name| {
        graph
            .load_named(name, PageKind::Page)
            .unwrap()
            .map(|page| page.path)
    };
    assert_eq!(
        after("target"),
        target_during,
        "the same file after the check"
    );
    // Every other name lookup (links, the save path) resolves the same file.
    assert_eq!(
        graph
            .find_entry("target", PageKind::Page)
            .map(|entry| entry.rel_path),
        target_during
    );
    assert_eq!(after("area/topic"), namespaced_during);
    // Title-named and renamed files resolve through the list, as before.
    assert_eq!(after("Titled Page"), Some("pages/titled.md".to_owned()));
    assert_eq!(after("titled"), None, "a title:: renames the file's page");
    release_projection(&graph);
    let _ = std::fs::remove_dir_all(root);
}

/// GH #550 siblings: a launch read of a page that changed since the image was
/// written applies in place, and one of a page the image has never seen waits
/// for the warm, which repairs that one page (GH #543). Neither may fail the
/// projection, lower the page twice, or rebuild the graph, and both must
/// answer queries from the current bytes.
#[test]
fn gh550_launch_reads_of_changed_and_new_pages_settle_without_failing() {
    let _serial = serialize_projection_tests();
    for (case, edit, expected_lowerings) in [
        ("changed", "journals/2026_09_20.md", 1),
        ("new", "journals/2026_09_21.md", 1),
    ] {
        let root = scratch(&format!("gh550-launch-{case}"));
        std::fs::create_dir_all(root.join("journals")).unwrap();
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(root.join("journals/2026_09_19.md"), "- older\n").unwrap();
        std::fs::write(root.join("journals/2026_09_20.md"), "- yesterday\n").unwrap();
        std::fs::write(root.join("pages/one.md"), "- TODO one\n").unwrap();
        let database = scratch(&format!("gh550-launch-{case}-db")).join("projection.sqlite");
        {
            let graph = Graph::open(&root);
            graph.attach_direct_projection(database.clone()).unwrap();
            graph.warm_cache();
            wait_ready(&graph);
            release_projection(&graph);
        }
        std::thread::sleep(Duration::from_millis(20));
        // Another device delivers this between launches.
        std::fs::write(root.join(edit), "- TODO delivered by sync\n").unwrap();

        reset_lowerings(&root);
        {
            let graph = Graph::open(&root);
            graph.attach_direct_projection(database.clone()).unwrap();
            let entry = graph
                .entry_for_path(&root.join(edit))
                .expect("the delivered journal has an entry");
            graph.load_page(&entry).expect("the feed loads it");
            assert!(
                graph.direct_projection_test().unwrap().wait_drained_test(),
                "{case}: a launch read must not fail the projection"
            );
            graph.warm_cache();
            wait_ready(&graph);
            assert_eq!(lowerings(), expected_lowerings, "{case}");
            let projected = graph
                .run_query_bounded("(task TODO)", 100, 1_000_000)
                .expect("the ready projection answers the public bounded route")
                .groups;
            assert_eq!(
                signature(&projected),
                signature(
                    &crate::query::run_query_bounded(&graph, "(task TODO)", 100, 1_000_000).groups
                ),
                "{case}"
            );
            assert_eq!(projected.len(), 2, "{case}: the delivered TODO is indexed");
            release_projection(&graph);
        }
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(database.parent().unwrap());
    }
}

/// GH #543: opening a page while the warm is reading the graph must not throw
/// that read away. A launch opens today's journal during the warm, and opening
/// publishes the page even when its bytes are unchanged. On a 10,000-page
/// Windows graph the warm's read took 5.6 s; the publication moved the cache
/// generation, the warm abandoned on drift, and every warm reopen fell back to
/// parsing the whole graph (16 s to a working search on the verify runner).
/// A real edit or a new page is published as an update, and the warm keeps
/// its read and applies that update after validating (audit IT-03): throwing
/// the read away for one changed page parsed all 10,000.
#[test]
fn gh543_a_page_opened_during_the_warm_read_keeps_the_warm() {
    let _serial = serialize_projection_tests();
    for case in ["unchanged", "edited", "created", "created-taken"] {
        let root = scratch(&format!("gh543-open-during-warm-{case}"));
        std::fs::create_dir_all(root.join("journals")).unwrap();
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(root.join("journals/2026_09_21.md"), "- DONE today\n").unwrap();
        for page in 0..8 {
            std::fs::write(
                root.join(format!("pages/p-{page}.md")),
                format!("- TODO page {page}\n"),
            )
            .unwrap();
        }
        let database =
            scratch(&format!("gh543-open-during-warm-{case}-db")).join("projection.sqlite");
        {
            let graph = Graph::open(&root);
            graph.attach_direct_projection(database.clone()).unwrap();
            graph.warm_cache();
            wait_ready(&graph);
            release_projection(&graph);
        }
        std::thread::sleep(Duration::from_millis(20));

        let graph = Arc::new(Graph::open(&root));
        graph.attach_direct_projection(database.clone()).unwrap();
        let pause = graph.pause_next_warm_after_read_test();
        let warm = {
            let graph = Arc::clone(&graph);
            std::thread::spawn(move || graph.warm_cache())
        };
        // The warm has read every page, today's journal included.
        pause.reached.wait();
        let opened = match case {
            // A page the warm never saw: a warm that let this move pass would
            // validate an inventory without it and drop its rows.
            "created" | "created-taken" => {
                let created = root.join("pages/created.md");
                std::fs::write(&created, "- TODO created after the warm read\n").unwrap();
                created
            }
            "edited" => {
                let journal = root.join("journals/2026_09_21.md");
                std::fs::write(&journal, "- TODO today, edited after the warm read it\n").unwrap();
                journal
            }
            _ => root.join("journals/2026_09_21.md"),
        };
        let entry = graph
            .entry_for_path(&opened)
            .expect("the page has an entry");
        graph.load_page(&entry).expect("the page opens");
        if case == "created-taken" {
            // The worker takes the new page's update before the warm hands
            // over: with no inventory yet it cannot place a page the image
            // does not hold.
            let projection = graph.direct_projection_test().unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while !projection.shared.pending.lock().unwrap().deltas.is_empty()
                || projection.shared.worker_busy.load(Ordering::Acquire)
            {
                assert!(
                    Instant::now() < deadline,
                    "the worker never took the update"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        pause.release.wait();
        warm.join().unwrap();
        wait_ready(&graph);

        assert!(
            !graph.has_parsed_cache_test() && graph.page_build_parses_test() == 0,
            "{case}: a page opened during the warm read threw the warm away \
             and parsed the whole graph ({} parses)",
            graph.page_build_parses_test()
        );
        let projected = graph
            .run_query_bounded("(task TODO)", 100, 1_000_000)
            .expect("the ready projection answers the public bounded route")
            .groups;
        assert_eq!(
            signature(&projected),
            signature(
                &crate::query::run_query_bounded(graph.as_ref(), "(task TODO)", 100, 1_000_000)
                    .groups
            ),
            "{case}: the index must answer what the files say"
        );
        assert_eq!(
            projected.len(),
            if case == "unchanged" { 8 } else { 9 },
            "{case}: the TODO written during the warm is indexed exactly when it exists"
        );
        release_projection(graph.as_ref());
        drop(graph);

        // The image the update left behind is current: the next reopen
        // validates it without parsing and still answers the edit.
        std::thread::sleep(Duration::from_millis(20));
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        assert!(
            !graph.has_parsed_cache_test() && graph.page_build_parses_test() == 0,
            "{case}: the next reopen parsed the whole graph ({} parses)",
            graph.page_build_parses_test()
        );
        assert_eq!(
            graph
                .run_query_bounded("(task TODO)", 100, 1_000_000)
                .expect("the reopened projection answers")
                .groups
                .len(),
            if case == "unchanged" { 8 } else { 9 },
            "{case}: the reopened index still holds the page written during the warm"
        );
        release_projection(&graph);
        drop(graph);
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(database.parent().unwrap());
    }
}

/// GH #543 (audit IT-03): a warm that reads the generation of an edit made
/// during it must not publish readiness before that edit's update is queued.
/// The update used to be queued after the cache lock was released: in that
/// gap the warm kept its read, left the edited page to an update that did not
/// exist yet, and search at the new generation answered the old text.
#[test]
fn gh543_a_warm_never_publishes_an_edit_before_its_update_is_queued() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("gh543-warm-edit-gap");
    let database = scratch("gh543-warm-edit-gap-db").join("projection.sqlite");
    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        release_projection(&graph);
    }
    std::thread::sleep(Duration::from_millis(20));

    let graph = Arc::new(Graph::open(&root));
    graph.attach_direct_projection(database.clone()).unwrap();
    let read_done = graph.pause_next_warm_after_read_test();
    let warm = {
        let graph = Arc::clone(&graph);
        std::thread::spawn(move || graph.warm_cache())
    };
    read_done.reached.wait();
    let edited = root.join("pages/two.md");
    std::fs::write(&edited, "- TODO gh543 edited during the warm\n").unwrap();
    let published = graph.pause_next_page_publication_test();
    let opener = {
        let graph = Arc::clone(&graph);
        let entry = graph.entry_for_path(&edited).expect("entry");
        std::thread::spawn(move || graph.load_page(&entry).expect("the page opens"))
    };
    // The edit's generation is observable; the opener has not returned.
    published.reached.wait();
    read_done.release.wait();
    warm.join().unwrap();
    let generation = graph.cache_generation();
    let projection = graph.direct_projection_test().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut stale = None;
    while Instant::now() < deadline && stale.is_none() {
        if projection.ready_at(generation) {
            let hits = graph
                .search("gh543 edited during the warm", 50)
                .unwrap_or_default();
            stale = Some(hits.is_empty());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    published.release.wait();
    opener.join().unwrap();
    wait_ready(&graph);
    assert_ne!(
        stale,
        Some(true),
        "readiness was published at the edit's generation while search still \
         answered the old text: {}",
        projection.debug_state_test()
    );
    assert!(
        !graph
            .search("gh543 edited during the warm", 50)
            .unwrap()
            .is_empty(),
        "the edit is not indexed after its update: {}",
        projection.debug_state_test()
    );
    assert_eq!(
        graph.page_build_parses_test(),
        0,
        "the warm was thrown away"
    );
    release_projection(graph.as_ref());
    drop(graph);
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(database.parent().unwrap());
}

/// GH #543 (audit IT-03): pages opened at launch publish updates the index
/// worker may not have taken yet when the warm check finishes reading. The
/// warm used to be refused for any queued update and the whole graph parsed;
/// the worker validates the warm and applies the updates after it in one turn.
#[test]
fn gh543_a_warm_is_queued_beside_pending_page_updates() {
    let _serial = serialize_projection_tests();
    let root = scratch("gh543-warm-beside-deltas");
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("journals/2026_09_21.md"), "- DONE today\n").unwrap();
    for page in 0..8 {
        std::fs::write(
            root.join(format!("pages/p-{page}.md")),
            format!("- TODO page {page}\n"),
        )
        .unwrap();
    }
    let database = scratch("gh543-warm-beside-deltas-db").join("projection.sqlite");
    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        release_projection(&graph);
    }
    std::thread::sleep(Duration::from_millis(20));

    let graph = Arc::new(Graph::open(&root));
    graph.attach_direct_projection(database.clone()).unwrap();
    let projection = graph.direct_projection_test().unwrap();
    let pause = graph.pause_next_warm_after_read_test();
    let warm = {
        let graph = Arc::clone(&graph);
        std::thread::spawn(move || graph.warm_cache())
    };
    pause.reached.wait();
    // Hold the worker inside the turn for the first opened page, so the
    // second page's update is still queued when the warm is handed over.
    let (paused, observed) = std::sync::mpsc::channel();
    let (resume, resumed) = std::sync::mpsc::channel::<()>();
    *BEFORE_APPLY_PENDING.lock().unwrap() = Some(Box::new(move || {
        paused.send(()).unwrap();
        resumed.recv().unwrap();
    }));
    let open = |path: &str| {
        let entry = graph.entry_for_path(&root.join(path)).expect("entry");
        graph.load_page(&entry).expect("the page opens");
    };
    open("journals/2026_09_21.md");
    observed.recv_timeout(Duration::from_secs(3)).unwrap();
    open("pages/p-0.md");
    assert!(
        !projection.shared.pending.lock().unwrap().deltas.is_empty(),
        "the second update must still be queued: {}",
        projection.debug_state_test()
    );
    pause.release.wait();
    // Let the warm reach its hand-over before the worker moves on.
    let deadline = Instant::now() + Duration::from_secs(5);
    while projection.shared.pending.lock().unwrap().warm.is_none()
        && graph.page_build_parses_test() == 0
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(1));
    }
    resume.send(()).unwrap();
    warm.join().unwrap();
    wait_ready(&graph);

    assert!(
        !graph.has_parsed_cache_test() && graph.page_build_parses_test() == 0,
        "queued page updates made the warm fall back to parsing the whole graph: {} parses",
        graph.page_build_parses_test()
    );
    let projected = graph
        .run_query_bounded("(task TODO)", 100, 1_000_000)
        .expect("the ready projection answers")
        .groups;
    assert_eq!(
        projected.len(),
        8,
        "the index still answers what the files say"
    );
    release_projection(graph.as_ref());
    drop(graph);
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(database.parent().unwrap());
}

/// GH #543 progress bar: each whole-graph pass reports how many pages it has
/// finished, and the report ends once search is answered by a current index.
#[test]
fn gh543_indexing_progress_counts_each_whole_graph_pass() {
    use crate::indexing_progress::{IndexingPhase, IndexingProgress};
    let _serial = serialize_projection_tests();
    let root = r6_graph("gh543-indexing-progress");
    for index in 0..80 {
        std::fs::write(
            root.join("pages").join(format!("bulk-{index:03}.md")),
            format!("- TODO bulk-{index:03}\n"),
        )
        .unwrap();
    }
    let database = root.join("private/projection.sqlite");
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database.clone()).unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    assert_eq!(
        graph.indexing_progress(),
        None,
        "a current index reports nothing"
    );

    // A fresh build counts the pages it has written.
    let (pages, revisions, config) = parsed_snapshot(&graph);
    let total = pages.len() as u64;
    let projection = graph.direct_projection_test().unwrap();
    let reached = Arc::new(std::sync::Barrier::new(2));
    let release = Arc::new(std::sync::Barrier::new(2));
    *projection.shared.after_fresh_build_batch.lock().unwrap() = Some(Box::new({
        let reached = Arc::clone(&reached);
        let release = Arc::clone(&release);
        move || {
            reached.wait();
            release.wait();
        }
    }));
    projection.request_rebuild();
    projection.enqueue_full(graph.cache_generation() + 1, pages, revisions, config, true);
    reached.wait();
    assert_eq!(
        graph.indexing_progress(),
        Some(IndexingProgress {
            phase: IndexingPhase::Indexing,
            done: 32,
            total,
        })
    );
    release.wait();
    drop(projection);
    release_projection(&graph);
    drop(graph);
    std::thread::sleep(Duration::from_millis(20));

    // A warm reopen counts the pages it has checked.
    let graph = Arc::new(Graph::open(&root));
    graph.attach_direct_projection(database).unwrap();
    let pause = graph.pause_next_warm_after_read_test();
    let warm = {
        let graph = Arc::clone(&graph);
        std::thread::spawn(move || graph.warm_cache())
    };
    pause.reached.wait();
    let checking = graph.indexing_progress().expect("the check is reported");
    assert_eq!(checking.phase, IndexingPhase::Checking);
    assert!(checking.total >= 80 && checking.done == checking.total);
    pause.release.wait();
    warm.join().unwrap();
    wait_ready(&graph);
    assert_eq!(graph.indexing_progress(), None);
    release_projection(&graph);
    std::fs::remove_dir_all(root).unwrap();
}

/// GH #543: on a warm reopen the frontend asks for the page list and aliases
/// while the warm is still reading files. Those reads used to give up after a
/// short wait and parse every page; the parse queued a full snapshot that
/// outranked the warm. They now wait for the warm and read the index.
#[test]
fn gh543_whole_graph_reads_during_the_warm_read_wait_instead_of_parsing() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("gh543-derived-reads-wait");
    for index in 0..40 {
        std::fs::write(
            root.join("pages").join(format!("bulk-{index:03}.md")),
            format!("alias:: bulk alias {index}\n\n- TODO bulk-{index:03}\n"),
        )
        .unwrap();
    }
    let database = scratch("gh543-derived-reads-wait-db").join("projection.sqlite");
    let (expected_pages, expected_aliases) = {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        let answer = (graph.list_pages().len(), graph.page_aliases().len());
        release_projection(&graph);
        answer
    };
    assert!(expected_aliases >= 40);
    std::thread::sleep(Duration::from_millis(20));

    let graph = Arc::new(Graph::open(&root));
    graph.attach_direct_projection(database).unwrap();
    let pause = graph.pause_next_warm_after_read_test();
    let warm = {
        let graph = Arc::clone(&graph);
        std::thread::spawn(move || graph.warm_cache())
    };
    pause.reached.wait();
    let reads = {
        let graph = Arc::clone(&graph);
        std::thread::spawn(move || (graph.list_pages().len(), graph.page_aliases().len()))
    };
    // Longer than the short delta wait: the reads must still be waiting.
    std::thread::sleep(Duration::from_millis(600));
    assert!(
        !graph.has_parsed_cache_test() && graph.page_build_parses_test() == 0,
        "a whole-graph read during the warm read parsed every page instead of waiting"
    );
    pause.release.wait();
    warm.join().unwrap();
    assert_eq!(reads.join().unwrap(), (expected_pages, expected_aliases));
    assert_eq!(graph.page_build_parses_test(), 0);
    release_projection(&graph);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn gh543_a_page_list_before_the_scheduled_warm_starts_waits_for_it() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("gh543-scheduled-warm");
    for index in 0..40 {
        std::fs::write(
            root.join("pages").join(format!("bulk-{index:03}.md")),
            format!("- bulk-{index:03}\n"),
        )
        .unwrap();
    }
    let database = scratch("gh543-scheduled-warm-db").join("projection.sqlite");
    let expected_pages = {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        let answer = graph.list_pages().len();
        release_projection(&graph);
        answer
    };
    std::thread::sleep(Duration::from_millis(20));

    // The app schedules the warm, then delays its thread so the first paint
    // goes first; that paint lists pages inside the delay.
    let graph = Arc::new(Graph::open(&root));
    graph.attach_direct_projection(database).unwrap();
    let announcement = graph.announce_launch_warm();
    let reads = {
        let graph = Arc::clone(&graph);
        std::thread::spawn(move || graph.list_pages().len())
    };
    // The same paint opens a page, which publishes it and moves the
    // generation under the waiting list.
    let before = graph.cache_generation();
    let opened = graph
        .entry_for_path(&root.join("pages/bulk-000.md"))
        .expect("the page has an entry");
    graph.load_page(&opened).expect("the page opens");
    assert_ne!(
        graph.cache_generation(),
        before,
        "opening a page is expected to move the generation"
    );
    std::thread::sleep(Duration::from_millis(600));
    assert!(
        !graph.has_parsed_cache_test() && graph.page_build_parses_test() == 0,
        "a page list before the scheduled warm started parsed every page instead of waiting"
    );
    let warm = {
        let graph = Arc::clone(&graph);
        std::thread::spawn(move || {
            let _announcement = announcement;
            graph.warm_cache()
        })
    };
    warm.join().unwrap();
    assert_eq!(reads.join().unwrap(), expected_pages);
    assert_eq!(graph.page_build_parses_test(), 0);
    release_projection(&graph);
    std::fs::remove_dir_all(root).unwrap();
}
