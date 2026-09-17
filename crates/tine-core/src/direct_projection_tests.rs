#[test]
fn direct_query_producer_has_no_saved_edit_or_answer_cache_protocol() {
    let source = include_str!("direct_projection.rs");
    let production = source.split("#[cfg(test)]\nmod tests {").next().unwrap();
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

#[path = "direct_projection_diagnostic_543.rs"]
mod diagnostic_543;

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
    // Current snapshot reads cannot admit a partial warm image. This
    // models the stream interval between replacement and ordering turns.
    projection.shared.pending.lock().unwrap().warm_stream = Some(graph.cache_generation());
    let partial = projection.open_current_query_job(RegistrySensitivity::Insensitive);
    projection.shared.pending.lock().unwrap().warm_stream = None;
    assert!(matches!(partial, QueryJobOpen::NotReady));
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
    projection.enqueue_full(generation + 2, pages, revisions, Arc::new(changed));
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
fn abandoning_a_warm_stream_asks_for_a_complete_inventory() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("abandon-asks-full");
    let database = root.join("private/projection.sqlite");
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database).unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let projection = graph.direct_projection_test().unwrap();
    let generation = graph.cache_generation();
    {
        let mut pending = projection.shared.pending.lock().unwrap();
        assert!(
            !pending.needs_full,
            "a converged projection owes no inventory before the stream is abandoned"
        );
        pending.warm_stream = Some(generation);
    }
    assert!(
        !projection.abandon_warm_stream(generation),
        "no full snapshot superseded this stream, so the caller still owes the fallback"
    );
    assert!(
        projection.shared.pending.lock().unwrap().needs_full,
        "an abandoned warm stream is what asks for the complete source inventory"
    );
    assert!(projection.close_and_wait_for_worker(Duration::from_secs(3)));
    std::fs::remove_dir_all(root).unwrap();
}

/// A turn deferred for want of a complete inventory is NOT a failure and
/// must not reach the always-on channel; a real write failure still must.
///
/// Both halves run here because the counter only proves the split when it
/// is shown to move for one input and not the other. Without the second
/// half a reporter that never printed anything would pass.
#[test]
fn a_deferred_turn_is_silent_while_a_write_failure_is_reported() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("deferred-turn-silent");
    let database = root.join("private/projection.sqlite");
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database.clone()).unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    let projection = graph.direct_projection_test().unwrap();

    // Exactly the state `abandon_warm_stream` leaves behind (pinned by
    // `abandoning_a_warm_stream_asks_for_a_complete_inventory`).
    projection.shared.pending.lock().unwrap().needs_full = true;
    let before = reported_projection_failures_test();

    let entry = graph.list_pages().into_iter().next().unwrap();
    let mut page = graph.load_page(&entry).unwrap();
    let baseline = page.rev.clone();
    page.blocks[0].raw = "TODO the delta that races a cold open".into();
    graph.save_page(&page, baseline.as_deref()).unwrap();

    let started = Instant::now();
    while !projection.shared.worker_failed.load(Ordering::Acquire)
        && started.elapsed() < Duration::from_secs(3)
    {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        projection.shared.worker_failed.load(Ordering::Acquire),
        "the deferred turn still withdraws readiness: only a full inventory may publish it"
    );
    assert_eq!(
        reported_projection_failures_test(),
        before,
        "the cold-open handoff must not print the always-on failure family; \
             it printed once per delta turn until 2026-09-10 and a cold open under \
             sync traffic emitted it hundreds of times while queries answered fine"
    );

    // The same channel must still fire for a genuine write failure.
    crate::direct_projection::recover_until_ready(&graph);
    let writer = rusqlite::Connection::open(&database).unwrap();
    writer.execute_batch("DROP TABLE block_text").unwrap();
    drop(writer);
    let mut page = graph.load_page(&entry).unwrap();
    let baseline = page.rev.clone();
    page.blocks[0].raw = "TODO a delta that cannot lower".into();
    graph.save_page(&page, baseline.as_deref()).unwrap();
    let started = Instant::now();
    while reported_projection_failures_test() == before
        && started.elapsed() < Duration::from_secs(5)
    {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        reported_projection_failures_test() > before,
        "a real materialization failure must still reach the always-on channel"
    );
    assert!(projection.close_and_wait_for_worker(Duration::from_secs(3)));
    std::fs::remove_dir_all(root).unwrap();
}

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
    let entry = graph.list_pages().into_iter().next().unwrap();
    let mut page = graph.load_page(&entry).unwrap();
    let baseline = page.rev.clone();
    let writer = rusqlite::Connection::open(&database).unwrap();
    writer.execute_batch("DROP TABLE block_text").unwrap();
    drop(writer);
    page.blocks[0].raw = "TODO acknowledged source survives failed projection".into();
    graph.save_page(&page, baseline.as_deref()).unwrap();
    let started = Instant::now();
    while !projection.shared.worker_failed.load(Ordering::Acquire)
        && started.elapsed() < Duration::from_secs(3)
    {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(projection.shared.worker_failed.load(Ordering::Acquire));
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
    // R6: the cold open streamed a fresh parse (structural ids), so the
    // page holds no live-id claim; only a live save adds one.
    assert!(
        !job.session_pages.contains(&page_id("pages/source.md")),
        "a streamed structural lowering claims no live ids"
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
    let ids = |graph: &Graph, names: &[&str]| -> HashSet<[u8; 16]> {
        graph
            .list_pages()
            .into_iter()
            .filter(|entry| names.contains(&entry.name.as_str()))
            .map(|entry| page_id(&entry.rel_path))
            .collect()
    };

    {
        let graph = Graph::open(&root);
        graph.attach_direct_projection(database.clone()).unwrap();
        graph.warm_cache();
        wait_ready(&graph);
        let projection = graph.direct_projection_test().unwrap();
        // R6: a cold open streams fresh parses (structural ids), so no
        // page holds a live-id claim yet.
        assert!(projection.session_pages_test().is_empty());

        let two = ids(&graph, &["two"]);
        graph.delete_page("two", PageKind::Page).unwrap();
        wait_ready(&graph);
        let after_delete = projection.session_pages_test();
        assert!(after_delete.is_empty());
        assert!(after_delete.is_disjoint(&two));

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
        assert_eq!(*projection.session_pages_test(), ids(&graph, &["one"]));
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
        assert_eq!(lowerings(), 1, "only the externally written page relowers");
        let projection = graph.direct_projection_test().unwrap();
        // R6: the relowering is a warm-stream parse — structural ids, so
        // the page holds no live-id claim either (the derived id and the
        // stored id coincide). Only a live save adds a page.
        assert!(
            projection.session_pages_test().is_empty(),
            "a reused row keeps an earlier session's id and a structural relowering claims none"
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
fn direct_projection_matches_fuzzy_search_and_virtual_reference_names() {
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
    let oracle = crate::query::search_cancellable(&graph, "cly", 20, || false);
    graph
        .attach_direct_projection(root.join("private/projection.sqlite"))
        .unwrap();
    wait_ready(&graph);

    let selected = graph.search("cly", 20).unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].page, "one");
    assert_eq!(
        signature(&graph.search("cly", 20).unwrap()),
        signature(&oracle)
    );
    assert_eq!(graph.direct_projection_fuzzy_candidate_reads_test(), 0);
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

    let fuzzy_reads = graph.direct_projection_fuzzy_candidate_reads_test();
    let name_reads = graph.direct_projection_referenced_name_reads_test();
    graph.direct_projection_mark_stale_test();
    assert_eq!(
        signature(&graph.search("cly", 20).unwrap()),
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
        graph.direct_projection_fuzzy_candidate_reads_test(),
        fuzzy_reads,
        "a stale generation must use the parser fallback"
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
    assert!(!graph.search("ecf", 20).unwrap().is_empty());
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

#[test]
fn direct_projection_matches_parser_reference_family_and_stale_fallback() {
    let _serial = serialize_projection_tests();
    let root = scratch("reference-family-parity");
    let target_id = "11111111-2222-4333-8444-555555555555";
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("pages/target.md"),
        format!("alias:: Alias Target\n\n- target\n  id:: {target_id}\n"),
    )
    .unwrap();
    std::fs::write(
            root.join("pages/referrer.md"),
            format!(
                "- [[Alias Target]] and plain Alias Target and (({target_id})) (({target_id}))\n- another (({target_id}))\n"
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
        .any(|(alias, owner, _)| alias == "changed alias" && owner == "target"));
    assert!(!changed_aliases
        .iter()
        .any(|(alias, _, _)| alias == "alias target"));

    graph.delete_page("target", PageKind::Page).unwrap();
    wait_ready(&graph);
    assert!(!graph
        .page_aliases_with_owners()
        .iter()
        .any(|(alias, _, _)| alias == "changed alias"));
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
        let candidates = reader
            .reference_candidate_pages(&[crate::refs::page_key("target")], ReferenceKind::Plain);
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
        let walked = reader.reference_candidate_pages(&names, ReferenceKind::Explicit);
        let indexed = reader.reference_candidate_pages_indexed(&names, ReferenceKind::Explicit);
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
    assert!(!crate::query::search_cancellable(&graph, "cly", 20, || false).is_empty());
    assert!(matches!(
        graph.search("cly", 20),
        Err(crate::query::QueryExecutionError::Unavailable(
            crate::query::QueryUnavailableReason::ProjectionUnavailable
        ))
    ));
    let names = graph
        .referenced_page_names()
        .into_iter()
        .map(|name| crate::refs::page_key(&name))
        .collect::<std::collections::BTreeSet<_>>();
    assert!(names.contains("inline only"));
    assert!(names.contains("alias only"));
    assert_eq!(graph.direct_projection_fuzzy_candidate_reads_test(), 0);
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
        query_page_order: None,
        identity: DeltaIdentity::Live,
    };
    let position = |pending: &PendingProjection, name: &str| match &pending.deltas
        [&format!("pages/{name}.md")]
        .1
    {
        PageDelta::Replace {
            query_page_order, ..
        } => query_page_order.expect("a delta outside a warm stream carries its position"),
        _ => panic!("replacement expected"),
    };
    let mut pending = PendingProjection::default();
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
            1,
            "one changed page must produce one SQL delta"
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

/// **F11.** The parse config travels inside each queued work item, so two
/// replacements coalesced into one worker turn are each lowered and stamped
/// under the config they were queued with -- never under whichever config
/// the last enqueue happened to leave beside the queue, and never under a
/// default that absence could stand in for.
///
/// The stamp is what reconciliation compares, so a page carrying another
/// page's config digest is a page whose rows answer a question the config
/// no longer asks and which no later reopen will notice (J7, D-1).
#[test]
fn each_queued_page_lowers_under_the_config_it_was_queued_with() {
    let _serial = serialize_projection_tests();
    let root = scratch("per-item-parse-config");
    std::fs::create_dir_all(&root).unwrap();
    let mut database = open_projection_database(&root.join("projection.sqlite")).unwrap();

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
                    query_page_order: Some(u64::from(rel_path == "beta.md")),
                    identity: DeltaIdentity::Live,
                },
            ),
        )
    };
    let deltas = BTreeMap::from([
        queued("alpha.md", &default_config),
        queued("beta.md", &edited_config),
    ]);
    apply_pending(&mut database, None, None, deltas).unwrap();

    let stamped = |alpha: &Arc<ParseConfig>, beta: &Arc<ParseConfig>| {
        database
            .source_delta(&[
                PhysicalGraphProjectionSourceRevision {
                    page_id: page_id("alpha.md"),
                    revision: projection_source_revision("sha256:alpha.md", alpha.digest()),
                },
                PhysicalGraphProjectionSourceRevision {
                    page_id: page_id("beta.md"),
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
        "dropping the parsed cache clears the set"
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
            "Its descriptor wrapper does: Direct\nblock answers carry `query_page_order.position` and\n`query_block_results.preorder` and end with `ORDER BY` on those columns",
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
            "otherwise recovery validates a\ncomplete source inventory from bytes and streams bounded per-page replacements",
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
    assert!(graph.direct_projection_fuzzy_candidate_reads_test() > 0);
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

// ----- R6: parsed independence (warm reuse, streaming cold init, session
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
    assert_eq!(graph.warm_stream_parses_test(), 0);
    let _projection = graph.direct_projection_test().unwrap();
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

    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(database.parent().unwrap());
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
    graph.invalidate_cache();
    assert!(graph.warm_cache_cancellable(|| false));
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
        !graph.has_parsed_cache_test(),
        "registry reads retain no Documents"
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
    let damage = rusqlite::Connection::open(&database).unwrap();
    damage.execute("PRAGMA foreign_keys = OFF", []).unwrap();
    damage.execute("DELETE FROM blocks", []).unwrap();
    drop(damage);
    let projection = graph.direct_projection_test().unwrap();
    // The old row adapter accepted this impossible owner: pin the failure
    // scenario independently of the new visitor's implementation.
    assert!(!projection
        .property_owner_rows(graph.cache_generation())
        .unwrap()
        .0
        .is_empty());
    let QueryJobOpen::Job(mut job) = projection.open_query_job(graph.cache_generation()) else {
        panic!("the schema still opens before corrupt ownership is inspected");
    };
    assert!(matches!(
        job.read_registry(&graph.config.parse_config()),
        Err(QueryExecutionError::Unavailable(
            QueryUnavailableReason::InvalidSnapshot
        ))
    ));
    assert!(!graph.has_parsed_cache_test());
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
    assert!(!graph.has_parsed_cache_test());
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
    assert!(!graph.has_parsed_cache_test());
}

/// **R6 §1, cold.** A fresh projection streams its build: every page is
/// lowered, no parsed cache is retained, and never more than
/// `WARM_STREAM_HIGH_WATER` documents wait in the queue.
#[test]
fn cold_open_streams_without_retaining_the_graph() {
    let _serial = serialize_projection_tests();
    let root = scratch("cold-stream");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let pages = 3 * WARM_STREAM_HIGH_WATER + 7;
    for i in 0..pages {
        std::fs::write(
            root.join(format!("pages/p{i:03}.md")),
            format!("- TODO task {i}\n- DONE done {i}\n"),
        )
        .unwrap();
    }
    let database = scratch("cold-stream-db").join("projection.sqlite");
    reset_lowerings(&root);
    MAX_PENDING_DELTAS.store(0, Ordering::Relaxed);
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database.clone()).unwrap();
    assert!(graph.warm_cache_cancellable(|| false));
    wait_ready(&graph);
    assert_eq!(lowerings(), pages as u64);
    assert_eq!(graph.warm_stream_parses_test(), pages);
    assert!(
        !graph.has_parsed_cache_test(),
        "a cold open must stream, not pin the graph"
    );
    assert!(
        MAX_PENDING_DELTAS.load(Ordering::Relaxed) <= WARM_STREAM_HIGH_WATER as u64,
        "the stream ran ahead of the worker: {} queued deltas",
        MAX_PENDING_DELTAS.load(Ordering::Relaxed)
    );
    let oracle = Graph::open(&root);
    assert_eq!(
        signature(
            &graph
                .run_query_bounded("(task TODO)", 1_000, 8_000_000)
                .expect("the ready projection answers the public bounded route")
                .groups
        ),
        signature(
            &crate::query::run_query_bounded(&oracle, "(task TODO)", 1_000, 8_000_000).groups
        )
    );
    assert!(!graph.has_parsed_cache_test());
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(database.parent().unwrap());
}

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

/// **R6 §1, one external edit between sessions.** Exactly that page is
/// parsed and relowered; the parsed cache is never built.
#[test]
fn an_external_edit_between_sessions_relowers_one_page_without_a_cache() {
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

    reset_lowerings(&root);
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database.clone()).unwrap();
    assert!(graph.warm_cache_cancellable(|| false));
    wait_ready(&graph);
    assert_eq!(lowerings(), 1);
    assert_eq!(graph.warm_stream_parses_test(), 1);
    assert_eq!(graph.page_build_parses_test(), 0);
    assert!(!graph.has_parsed_cache_test());
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

/// **R6 §1, damaged file.** A projection whose schema is damaged is
/// recreated and its rebuild streams like a cold open — no parsed cache.
#[test]
fn a_damaged_projection_streams_its_rebuild() {
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
    assert_eq!(graph.warm_stream_parses_test(), 5);
    assert!(!graph.has_parsed_cache_test());
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
    assert!(!graph.has_parsed_cache_test());
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
    // sync_file's parsed-cache adapter is a no-op without that cache;
    // streamed reconciliation owns the absent-cache path.
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
fn failed_projection_recovery_does_not_build_a_parsed_graph() {
    let _serial = serialize_projection_tests();
    let root = r6_graph("streamed-damage-recovery");
    let database = scratch("streamed-damage-recovery-db").join("projection.sqlite");
    let graph = Graph::open(&root);
    graph.attach_direct_projection(database.clone()).unwrap();
    graph.warm_cache();
    wait_ready(&graph);
    assert!(!graph.has_parsed_cache_test());
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
    assert!(
        !graph.has_parsed_cache_test(),
        "projection repair must stream source pages without retaining the parsed graph"
    );
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
    assert!(!graph.has_parsed_cache_test());
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
    let one = page_id(&entry.rel_path);
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

    // Losing this disposable source stamp forces a streamed replacement
    // from unchanged bytes, exercising recovery's identity provenance.
    let writer = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(
        writer
            .execute(
                "DELETE FROM direct_source_revisions WHERE page_id = ?1",
                rusqlite::params![one.as_slice()]
            )
            .unwrap(),
        1
    );
    drop(writer);
    let parses = graph.warm_stream_parses_test();
    graph.invalidate_cache();
    assert!(
        projection.session_pages_test().contains(&one),
        "dropping the parsed cache must retain compatible live IDs"
    );
    graph.warm_cache();
    wait_ready(&graph);
    assert_eq!(graph.warm_stream_parses_test(), parses + 1);
    assert!(!graph.has_parsed_cache_test());
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
        worker_finished: AtomicBool::new(false),
        worker_resources: Mutex::new(Some(Vec::new())),
        validated: AtomicBool::new(false),
        after_sql_commit: Mutex::new(None),
        repairs_in_flight: AtomicUsize::new(0),
        #[cfg(test)]
        capture_thread: Mutex::new(None),
        #[cfg(test)]
        indexed_reads: AtomicU64::new(0),
        statement_reads: AtomicU64::new(0),
        registry_capture_attempts: AtomicU64::new(0),
        inject_read_failure: AtomicBool::new(false),
        fallback_reads: AtomicU64::new(0),
        referenced_name_reads: AtomicU64::new(0),
        fuzzy_candidate_reads: AtomicU64::new(0),
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
    let entry = graph.list_pages().into_iter().next().unwrap();
    let id = page_id(&entry.rel_path);
    assert!(!projection.session_pages_test().contains(&id));
    // Build the edit fixture without enqueueing load_page's ordinary
    // watcher delta on the producer whose first identity transition this
    // test pauses. The source bytes and deterministic starting IDs agree.
    let mut page = Graph::open(&root).load_page(&entry).unwrap();
    let baseline = page.rev.clone();
    page.blocks[0].raw = "TODO producer identity sentinel".into();
    let (paused, observed) = std::sync::mpsc::channel();
    let (resume, resumed) = std::sync::mpsc::channel();
    *projection.shared.after_sql_commit.lock().unwrap() = Some(Box::new(move || {
        paused.send(()).unwrap();
        resumed.recv().unwrap();
    }));
    graph.save_page(&page, baseline.as_deref()).unwrap();
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
fn a_structural_relower_drops_the_session_identity() {
    let shared = empty_projection_shared();
    let a = page_id("pages/a.md");
    let b = page_id("pages/b.md");
    shared.record_session_pages(&AppliedPages {
        lowered: vec![a, b],
        ..AppliedPages::default()
    });
    assert_eq!(
        **shared.session_pages.lock().unwrap(),
        HashSet::from([a, b])
    );
    shared.record_session_pages(&AppliedPages {
        relowered_structurally: vec![a],
        ..AppliedPages::default()
    });
    assert_eq!(**shared.session_pages.lock().unwrap(), HashSet::from([b]));
}

/// **R6 §2.** Backlinks on a warm, cache-less graph hydrate exactly the
/// projection's candidates from disk and build no whole-graph cache.
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
    let parses = graph.page_build_parses_test() + graph.warm_stream_parses_test();
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
        let candidates = graph
            .reference_candidate_pages(&[crate::refs::page_key(target)], ReferenceKind::Explicit);
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

/// Manual cost probe for GH #543: how much of one search's per-block work is
/// `canonical_fold` (which the projection ALREADY stores as
/// `blocks.query_visible_folded`) versus the relevance scan itself. The answer
/// decides the fix: if folding dominates, passing the stored fold through
/// `QueryRankPrograms::bind_pair` is a semantics-free win; if the scan
/// dominates, the fix must instead reduce how many blocks are ranked at all.
///
/// `block_relevance` is module-private, so this times fold-only and
/// fold+relevance (`rank_block_text`) and reports the difference rather than
/// claiming to measure the scan directly.
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
