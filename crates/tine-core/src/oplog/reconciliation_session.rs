//! Headless, single-endpoint execution of inactive reconciliation jobs.
//!
//! A session owns only one bounded scheduler and, after publication, the
//! coordinator continuation for that scheduler's active lease. Graph truth,
//! enrollment, filesystem authority, and lifecycle policy remain injected by
//! the synchronous caller for every step.

use std::collections::BTreeSet;

use crate::model::Graph;

use super::{
    operational_coordinator::{
        FailedClosedOperationalCoordinator, OperationalCoordinator, OperationalCoordinatorState,
    },
    reconciliation_import::{execute_stable_scan_import, ReconciliationImportOutcome},
    reconciliation_scan::{
        scan_graph_text, GraphTextScanFailureClass, GraphTextScanLimits,
        JoinedAuthenticatedExpectedPathSource, ReconciliationCompletionOutcome, ReconciliationJob,
        ReconciliationLease, ReconciliationScheduler, ReconciliationSchedulerLimits,
        ReconciliationSchedulerStatus, ReconciliationTrigger, ReconciliationWork,
    },
    ManagedPath, ProjectionReceiptStore, ShardedHotEngine, SqliteFrontier, TailOverlay,
};

/// Exact enrolled dependencies supplied for one synchronous session step.
///
/// The session never retains any of these references. In particular, it does
/// not cache a graph, projection receipt, engine, database, tail, scan, or
/// expected-path source between calls.
pub(crate) struct ReconciliationSessionDependencies<'a> {
    pub(crate) graph: &'a Graph,
    pub(crate) receipts: &'a ProjectionReceiptStore,
    pub(crate) engine: &'a mut ShardedHotEngine,
    pub(crate) database: &'a mut SqliteFrontier,
    pub(crate) tail: &'a mut TailOverlay,
}

/// Opaque identity for a post-publication continuation retained by a session.
///
/// It cannot be forged outside this module and does not grant a scheduler
/// completion capability. The durable coordinator continuation stays owned by
/// its session until `resume` finishes it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReconciliationPendingContinuation {
    lease: ReconciliationLease,
}

/// Observable result of exactly one selected reconciliation job.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReconciliationSessionStep {
    Idle,
    Noop,
    Complete,
    Blocked,
    RetryFull,
    Pending(ReconciliationPendingContinuation),
}

/// A rejected session action never settles or replans an active lease.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReconciliationSessionError {
    PendingContinuation(ReconciliationPendingContinuation),
    StaleOrForeignContinuation,
}

struct PendingContinuation<C> {
    token: ReconciliationPendingContinuation,
    continuation: C,
}

/// One headless reconciliation session for exactly one enrolled endpoint.
///
/// The type has no public export and intentionally exposes no lease-complete,
/// cancellation, shutdown, enrollment, or persistence operation. A published
/// failed-closed continuation therefore cannot be discarded through this API.
pub(crate) struct ReconciliationSession<C = FailedClosedOperationalCoordinator> {
    scheduler: ReconciliationScheduler,
    pending: Option<PendingContinuation<C>>,
}

impl<C> ReconciliationSession<C> {
    pub(crate) fn new(limits: ReconciliationSchedulerLimits) -> Self {
        Self {
            scheduler: ReconciliationScheduler::new(limits),
            pending: None,
        }
    }

    /// Coalesce a bounded discovery hint, including hints that arrive while a
    /// job or a published continuation is active.
    pub(crate) fn trigger(&mut self, trigger: ReconciliationTrigger) {
        self.scheduler.trigger(trigger);
    }

    pub(crate) fn status(&self) -> ReconciliationSchedulerStatus {
        self.scheduler.status()
    }

    fn step_with<D>(
        &mut self,
        dispatch: &mut D,
    ) -> Result<ReconciliationSessionStep, ReconciliationSessionError>
    where
        D: ReconciliationSessionDispatch<Continuation = C>,
    {
        if let Some(pending) = &self.pending {
            return Err(ReconciliationSessionError::PendingContinuation(
                pending.token,
            ));
        }
        let Some(job) = self.scheduler.next() else {
            return Ok(ReconciliationSessionStep::Idle);
        };
        let outcome = {
            let mut arrive = |trigger| self.scheduler.trigger(trigger);
            dispatch.dispatch(job.work(), &mut arrive)
        };
        self.settle_job(job, outcome)
    }

    fn resume_with<D>(
        &mut self,
        continuation: ReconciliationPendingContinuation,
        dispatch: &mut D,
    ) -> Result<ReconciliationSessionStep, ReconciliationSessionError>
    where
        D: ReconciliationSessionDispatch<Continuation = C>,
    {
        let Some(pending) = self.pending.as_ref() else {
            return Err(ReconciliationSessionError::StaleOrForeignContinuation);
        };
        if pending.token != continuation {
            return Err(ReconciliationSessionError::StaleOrForeignContinuation);
        }
        let pending = self
            .pending
            .take()
            .expect("checked reconciliation continuation disappeared");
        let outcome = dispatch.resume(pending.continuation);
        self.settle_continuation(pending.token, outcome)
    }

    fn settle_job(
        &mut self,
        job: ReconciliationJob,
        outcome: ReconciliationSessionDispatchOutcome<C>,
    ) -> Result<ReconciliationSessionStep, ReconciliationSessionError> {
        match outcome {
            ReconciliationSessionDispatchOutcome::FailedClosed(continuation) => {
                let token = ReconciliationPendingContinuation { lease: job.lease() };
                self.pending = Some(PendingContinuation {
                    token,
                    continuation,
                });
                Ok(ReconciliationSessionStep::Pending(token))
            }
            outcome => self.settle_lease(job.lease(), outcome),
        }
    }

    fn settle_continuation(
        &mut self,
        token: ReconciliationPendingContinuation,
        outcome: ReconciliationSessionDispatchOutcome<C>,
    ) -> Result<ReconciliationSessionStep, ReconciliationSessionError> {
        match outcome {
            ReconciliationSessionDispatchOutcome::FailedClosed(continuation) => {
                self.pending = Some(PendingContinuation {
                    token,
                    continuation,
                });
                Ok(ReconciliationSessionStep::Pending(token))
            }
            outcome => self.settle_lease(token.lease, outcome),
        }
    }

    fn settle_lease(
        &mut self,
        lease: ReconciliationLease,
        outcome: ReconciliationSessionDispatchOutcome<C>,
    ) -> Result<ReconciliationSessionStep, ReconciliationSessionError> {
        let (completion, step) = match outcome {
            ReconciliationSessionDispatchOutcome::Noop => (
                ReconciliationCompletionOutcome::Noop,
                ReconciliationSessionStep::Noop,
            ),
            ReconciliationSessionDispatchOutcome::Complete => (
                ReconciliationCompletionOutcome::Complete,
                ReconciliationSessionStep::Complete,
            ),
            // An ordinary coordinator error is deliberately classified as
            // blocked, never as a clean no-op or completion.
            ReconciliationSessionDispatchOutcome::Blocked => (
                ReconciliationCompletionOutcome::Blocked,
                ReconciliationSessionStep::Blocked,
            ),
            ReconciliationSessionDispatchOutcome::RetryFull => (
                ReconciliationCompletionOutcome::Retry,
                ReconciliationSessionStep::RetryFull,
            ),
            ReconciliationSessionDispatchOutcome::FailedClosed(_) => {
                unreachable!("failed-closed continuations are retained before lease settlement")
            }
        };
        self.scheduler
            .complete(lease, completion)
            .expect("session owns the exact active reconciliation lease");
        Ok(step)
    }
}

impl ReconciliationSession<FailedClosedOperationalCoordinator> {
    /// Execute one selected job with freshly injected live dependencies.
    pub(crate) fn step(
        &mut self,
        dependencies: ReconciliationSessionDependencies<'_>,
    ) -> Result<ReconciliationSessionStep, ReconciliationSessionError> {
        let mut dispatch = LiveReconciliationSessionDispatch {
            dependencies,
            #[cfg(test)]
            before_second_scan_pass: None,
            #[cfg(test)]
            arrival_before_dispatch: None,
        };
        self.step_with(&mut dispatch)
    }

    /// Resume the exact retained post-publication continuation once.
    pub(crate) fn resume(
        &mut self,
        continuation: ReconciliationPendingContinuation,
        dependencies: ReconciliationSessionDependencies<'_>,
    ) -> Result<ReconciliationSessionStep, ReconciliationSessionError> {
        let mut dispatch = LiveReconciliationSessionDispatch {
            dependencies,
            #[cfg(test)]
            before_second_scan_pass: None,
            #[cfg(test)]
            arrival_before_dispatch: None,
        };
        self.resume_with(continuation, &mut dispatch)
    }
}

enum ReconciliationSessionDispatchOutcome<C> {
    Noop,
    Complete,
    Blocked,
    RetryFull,
    FailedClosed(C),
}

/// Private generic seam: production dispatch performs the authoritative scan
/// and coordinator calls, while module tests prove lease and continuation
/// behavior without manufacturing graph or publication state.
trait ReconciliationSessionDispatch {
    type Continuation;

    fn dispatch(
        &mut self,
        work: &ReconciliationWork,
        arrive: &mut dyn FnMut(ReconciliationTrigger),
    ) -> ReconciliationSessionDispatchOutcome<Self::Continuation>;

    fn resume(
        &mut self,
        continuation: Self::Continuation,
    ) -> ReconciliationSessionDispatchOutcome<Self::Continuation>;
}

struct LiveReconciliationSessionDispatch<'a> {
    dependencies: ReconciliationSessionDependencies<'a>,
    #[cfg(test)]
    before_second_scan_pass: Option<Box<dyn FnMut() + 'a>>,
    #[cfg(test)]
    arrival_before_dispatch: Option<ReconciliationTrigger>,
}

impl LiveReconciliationSessionDispatch<'_> {
    fn execute_targeted(
        &mut self,
        paths: &BTreeSet<ManagedPath>,
    ) -> ReconciliationSessionDispatchOutcome<FailedClosedOperationalCoordinator> {
        let requested_paths = paths.iter().map(ManagedPath::as_str).collect::<Vec<_>>();
        let ReconciliationSessionDependencies {
            graph,
            receipts,
            engine,
            database,
            tail,
        } = &mut self.dependencies;
        match OperationalCoordinator::execute(
            graph,
            receipts,
            engine,
            database,
            tail,
            &requested_paths,
        ) {
            Ok(OperationalCoordinatorState::Noop) => ReconciliationSessionDispatchOutcome::Noop,
            Ok(OperationalCoordinatorState::Complete(_)) => {
                ReconciliationSessionDispatchOutcome::Complete
            }
            Ok(OperationalCoordinatorState::Blocked(_)) => {
                ReconciliationSessionDispatchOutcome::Blocked
            }
            Err(_) => ReconciliationSessionDispatchOutcome::Blocked,
            Ok(OperationalCoordinatorState::FailedClosed(continuation)) => {
                ReconciliationSessionDispatchOutcome::FailedClosed(continuation)
            }
        }
    }

    fn execute_full_scan(
        &mut self,
    ) -> ReconciliationSessionDispatchOutcome<FailedClosedOperationalCoordinator> {
        let scan = {
            let ReconciliationSessionDependencies { graph, engine, .. } = &mut self.dependencies;
            let projection = match engine.projection_work_index() {
                Ok(projection) => projection,
                // A projection work index error is an authoritative failure,
                // not evidence that rerunning the same scan can make progress.
                Err(_) => return ReconciliationSessionDispatchOutcome::Blocked,
            };
            let source = JoinedAuthenticatedExpectedPathSource::new(engine, projection);
            #[cfg(test)]
            let result = if let Some(hook) = self.before_second_scan_pass.as_mut() {
                super::reconciliation_scan::scan_graph_text_with_hook(
                    graph,
                    &source,
                    GraphTextScanLimits::default(),
                    || {
                        hook();
                        Ok(())
                    },
                )
            } else {
                scan_graph_text(graph, &source, GraphTextScanLimits::default())
            };
            #[cfg(not(test))]
            let result = scan_graph_text(graph, &source, GraphTextScanLimits::default());
            match result {
                Ok(scan) => scan,
                Err(error) => {
                    return match error.class {
                        // The two-pass scanner established that its observed
                        // epoch moved. One coalesced fresh scan is meaningful.
                        GraphTextScanFailureClass::UnstableEpoch => {
                            ReconciliationSessionDispatchOutcome::RetryFull
                        }
                        // Bounds, unsafe filesystems, and unavailable/corrupt
                        // expected authority are terminal for this lease.
                        GraphTextScanFailureClass::Blocked => {
                            ReconciliationSessionDispatchOutcome::Blocked
                        }
                    };
                }
            }
        };
        let ReconciliationSessionDependencies {
            graph,
            receipts,
            engine,
            database,
            tail,
        } = &mut self.dependencies;
        match execute_stable_scan_import(scan, graph, receipts, engine, database, tail) {
            ReconciliationImportOutcome::Noop => ReconciliationSessionDispatchOutcome::Noop,
            ReconciliationImportOutcome::Complete(_) => {
                ReconciliationSessionDispatchOutcome::Complete
            }
            ReconciliationImportOutcome::Blocked(_) => {
                ReconciliationSessionDispatchOutcome::Blocked
            }
            ReconciliationImportOutcome::RetryFull(_) => {
                ReconciliationSessionDispatchOutcome::RetryFull
            }
            ReconciliationImportOutcome::FailedClosed(continuation) => {
                ReconciliationSessionDispatchOutcome::FailedClosed(continuation)
            }
        }
    }
}

impl ReconciliationSessionDispatch for LiveReconciliationSessionDispatch<'_> {
    type Continuation = FailedClosedOperationalCoordinator;

    fn dispatch(
        &mut self,
        work: &ReconciliationWork,
        arrive: &mut dyn FnMut(ReconciliationTrigger),
    ) -> ReconciliationSessionDispatchOutcome<Self::Continuation> {
        #[cfg(test)]
        if let Some(trigger) = self.arrival_before_dispatch.take() {
            arrive(trigger);
        }
        match work {
            ReconciliationWork::ProjectionPreconditionMismatch { paths }
            | ReconciliationWork::WatcherPaths { paths } => self.execute_targeted(paths),
            ReconciliationWork::FullScan(_) => self.execute_full_scan(),
        }
    }

    fn resume(
        &mut self,
        continuation: Self::Continuation,
    ) -> ReconciliationSessionDispatchOutcome<Self::Continuation> {
        let ReconciliationSessionDependencies {
            graph,
            receipts,
            engine,
            database,
            tail,
        } = &mut self.dependencies;
        match continuation.retry(graph, receipts, engine, database, tail) {
            OperationalCoordinatorState::Complete(_) => {
                ReconciliationSessionDispatchOutcome::Complete
            }
            OperationalCoordinatorState::FailedClosed(continuation) => {
                ReconciliationSessionDispatchOutcome::FailedClosed(continuation)
            }
            // `retry` only returns Complete or FailedClosed today. Preserve
            // safety if that implementation grows another terminal state.
            OperationalCoordinatorState::Blocked(_) | OperationalCoordinatorState::Noop => {
                ReconciliationSessionDispatchOutcome::Blocked
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeSet, VecDeque},
        fs,
        path::{Path, PathBuf},
    };

    use super::*;
    use crate::{
        model::Graph,
        oplog::{
            write_projection_exact, ApplicationRuntimeRoot, AuthorBatch, BatchId, CrdtPeerId,
            DeviceId, DocumentId, LineageDigest, LogicalPageName, ManagedTextKind, ObjectStore,
            OperationTransaction, PageId, ProjectionClaim, ProjectionEndpointBinding,
            ProjectionEndpointId, RebuildSource, SemanticOperation, SessionId, WorkspaceId,
        },
    };
    use uuid::Uuid;

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "tine-reconciliation-session-{label}-{}",
                Uuid::new_v4()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct LiveFixture {
        _root: TestRoot,
        graph_root: PathBuf,
        graph: Graph,
        receipts: ProjectionReceiptStore,
        engine: ShardedHotEngine,
        database: SqliteFrontier,
        tail: TailOverlay,
        path: String,
    }

    impl LiveFixture {
        fn new(label: &str, complete_projection: bool) -> Self {
            let root = TestRoot::new(label);
            let graph_root = root.path().join("graph");
            fs::create_dir_all(&graph_root).unwrap();
            let graph = Graph::open(&graph_root);
            let workspace_id = WorkspaceId::from_uuid(Uuid::from_u128(101));
            let endpoint = ProjectionEndpointBinding::enroll_graph(
                &graph,
                ProjectionEndpointId::from_uuid(Uuid::from_u128(102)),
                DeviceId::from_uuid(Uuid::from_u128(103)),
            )
            .unwrap();
            let receipts = ProjectionReceiptStore::open_for_endpoint(
                &root.path().join("receipts"),
                workspace_id,
                endpoint,
            )
            .unwrap();
            let lineage = LineageDigest::of(label.as_bytes());
            let catalog = DocumentId::from_uuid(Uuid::from_u128(104));
            let page_id = PageId::from_uuid(Uuid::from_u128(105));
            let path = "pages/live.md".to_owned();
            let transaction = OperationTransaction::new(vec![SemanticOperation::CreatePage {
                page_id,
                home_document_id: DocumentId::from_uuid(Uuid::from_u128(106)),
                name: LogicalPageName::parse("Live Session").unwrap(),
                path: ManagedPath::parse(&path).unwrap(),
                kind: ManagedTextKind::Page,
            }])
            .unwrap();
            let author = ShardedHotEngine::new(workspace_id, lineage, catalog);
            let bootstrap = author
                .prepare_bootstrap_transaction(
                    AuthorBatch {
                        batch_id: BatchId::from_uuid(Uuid::from_u128(107)),
                        author_device_id: DeviceId::from_uuid(Uuid::from_u128(108)),
                        author_session_id: SessionId::from_uuid(Uuid::from_u128(109)),
                        crdt_peer_id: CrdtPeerId::from_u64(110),
                    },
                    &transaction,
                )
                .unwrap();
            let archive_root = root.path().join("archive");
            ObjectStore::open(&archive_root, workspace_id)
                .unwrap()
                .publish_prepared(&bootstrap)
                .unwrap();
            let mut engine = ShardedHotEngine::with_enrolled_projection(
                ObjectStore::open(&archive_root, workspace_id).unwrap(),
                lineage,
                catalog,
                &graph,
                &receipts,
            );
            engine
                .stage_archive_batch(bootstrap.manifest().batch_id())
                .unwrap();
            if complete_projection {
                write_projection_exact(&graph, &receipts, &engine, page_id, None).unwrap();
            }
            let archive = ObjectStore::open(&archive_root, workspace_id).unwrap();
            let runtime =
                ApplicationRuntimeRoot::open_for_test(&root.path().join("runtime")).unwrap();
            let database_path = root.path().join("sqlite/materialized.sqlite3");
            let source = RebuildSource::new(&engine, &archive).unwrap();
            let database = SqliteFrontier::open_or_rebuild(
                &database_path,
                &runtime,
                ProjectionClaim::current(workspace_id, lineage),
                source,
            )
            .unwrap()
            .database;
            let source = RebuildSource::new(&engine, &archive).unwrap();
            let tail = TailOverlay::from_durable(&database, &source).unwrap();
            Self {
                _root: root,
                graph_root,
                graph,
                receipts,
                engine,
                database,
                tail,
                path,
            }
        }

        fn dependencies(&mut self) -> ReconciliationSessionDependencies<'_> {
            ReconciliationSessionDependencies {
                graph: &self.graph,
                receipts: &self.receipts,
                engine: &mut self.engine,
                database: &mut self.database,
                tail: &mut self.tail,
            }
        }
    }

    fn live_session() -> ReconciliationSession {
        ReconciliationSession::new(ReconciliationSchedulerLimits::default())
    }

    fn assert_blocked_and_idle(session: &mut ReconciliationSession, fixture: &mut LiveFixture) {
        assert_eq!(
            session.status().last_completion,
            Some(ReconciliationCompletionOutcome::Blocked)
        );
        assert!(session.status().blocked.is_some());
        assert!(!session.status().active);
        assert!(!session.status().pending);
        assert_eq!(
            session.step(fixture.dependencies()),
            Ok(ReconciliationSessionStep::Idle)
        );
    }

    #[test]
    fn live_full_scan_missing_expected_authority_blocks_without_retry() {
        let mut fixture = LiveFixture::new("missing-expected-authority", false);
        let mut session = live_session();
        session.trigger(ReconciliationTrigger::Explicit);

        assert_eq!(
            session.step(fixture.dependencies()),
            Ok(ReconciliationSessionStep::Blocked)
        );
        assert_blocked_and_idle(&mut session, &mut fixture);
    }

    #[test]
    fn live_full_scan_unenrolled_projection_index_blocks_without_retry() {
        let mut fixture = LiveFixture::new("unenrolled-projection-index", true);
        let mut unenrolled = ShardedHotEngine::new(
            WorkspaceId::from_uuid(Uuid::from_u128(201)),
            LineageDigest::of(b"unenrolled-projection-index"),
            DocumentId::from_uuid(Uuid::from_u128(202)),
        );
        let mut session = live_session();
        session.trigger(ReconciliationTrigger::Explicit);

        assert_eq!(
            session.step(ReconciliationSessionDependencies {
                graph: &fixture.graph,
                receipts: &fixture.receipts,
                engine: &mut unenrolled,
                database: &mut fixture.database,
                tail: &mut fixture.tail,
            }),
            Ok(ReconciliationSessionStep::Blocked)
        );
        assert_eq!(
            session.status().last_completion,
            Some(ReconciliationCompletionOutcome::Blocked)
        );
        assert!(session.status().blocked.is_some());
        assert!(!session.status().active);
        assert!(!session.status().pending);
        assert_eq!(
            session.step(ReconciliationSessionDependencies {
                graph: &fixture.graph,
                receipts: &fixture.receipts,
                engine: &mut unenrolled,
                database: &mut fixture.database,
                tail: &mut fixture.tail,
            }),
            Ok(ReconciliationSessionStep::Idle)
        );
    }

    #[cfg(unix)]
    #[test]
    fn live_full_scan_unsafe_filesystem_blocks_without_retry() {
        use std::os::unix::fs::symlink;

        let mut fixture = LiveFixture::new("unsafe-filesystem", true);
        symlink(
            fixture.graph_root.join(&fixture.path),
            fixture.graph_root.join("pages/unsafe-link.md"),
        )
        .unwrap();
        let mut session = live_session();
        session.trigger(ReconciliationTrigger::Explicit);

        assert_eq!(
            session.step(fixture.dependencies()),
            Ok(ReconciliationSessionStep::Blocked)
        );
        assert_blocked_and_idle(&mut session, &mut fixture);
    }

    #[test]
    fn live_unstable_scan_race_queues_one_full_retry() {
        let mut fixture = LiveFixture::new("unstable-scan-race", true);
        let mutation = fixture.graph_root.join(&fixture.path);
        let mut session = live_session();
        session.trigger(ReconciliationTrigger::Explicit);
        let mut dispatch = LiveReconciliationSessionDispatch {
            dependencies: fixture.dependencies(),
            before_second_scan_pass: Some(Box::new(move || {
                fs::write(&mutation, b"- changed during scan\n").unwrap();
            })),
            arrival_before_dispatch: None,
        };

        assert_eq!(
            session.step_with(&mut dispatch),
            Ok(ReconciliationSessionStep::RetryFull)
        );
        drop(dispatch);
        assert_eq!(
            session.status().last_completion,
            Some(ReconciliationCompletionOutcome::Retry)
        );
        assert!(!session.status().active);
        assert!(session.status().pending);

        assert!(matches!(
            session.step(fixture.dependencies()),
            Ok(ReconciliationSessionStep::Complete)
                | Ok(ReconciliationSessionStep::Noop)
                | Ok(ReconciliationSessionStep::Blocked)
        ));
        assert!(!session.status().pending);
        assert_eq!(
            session.step(fixture.dependencies()),
            Ok(ReconciliationSessionStep::Idle)
        );
    }

    #[test]
    fn live_failure_keeps_queued_precondition_ahead_of_full_retry() {
        let mut fixture = LiveFixture::new("queued-precondition", true);
        let mutation = fixture.graph_root.join(&fixture.path);
        let precondition = paths(&[&fixture.path]);
        let mut session = live_session();
        session.trigger(ReconciliationTrigger::Explicit);
        let mut dispatch = LiveReconciliationSessionDispatch {
            dependencies: fixture.dependencies(),
            before_second_scan_pass: Some(Box::new(move || {
                fs::write(&mutation, b"- changed during scan\n").unwrap();
            })),
            arrival_before_dispatch: Some(ReconciliationTrigger::ProjectionPreconditionMismatch(
                precondition.clone(),
            )),
        };

        assert_eq!(
            session.step_with(&mut dispatch),
            Ok(ReconciliationSessionStep::RetryFull)
        );
        drop(dispatch);
        assert!(session.status().pending);

        let precondition_job = session
            .scheduler
            .next()
            .expect("queued projection precondition must remain pending");
        assert_eq!(
            precondition_job.work(),
            &ReconciliationWork::ProjectionPreconditionMismatch {
                paths: precondition
            }
        );
        session
            .scheduler
            .complete(
                precondition_job.lease(),
                ReconciliationCompletionOutcome::Blocked,
            )
            .unwrap();
        let retry_job = session
            .scheduler
            .next()
            .expect("unstable scan retry must remain after the urgent precondition");
        assert!(matches!(retry_job.work(), ReconciliationWork::FullScan(_)));
        session
            .scheduler
            .complete(retry_job.lease(), ReconciliationCompletionOutcome::Blocked)
            .unwrap();
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct FakeContinuation(u64);

    #[derive(Clone, Copy)]
    enum FakeDispatchResult {
        Noop,
        Complete,
        Blocked,
        RetryFull,
        FailedClosed(u64),
    }

    #[derive(Clone, Copy)]
    enum FakeResumeResult {
        Complete,
        FailedClosed(u64),
    }

    struct FakeDispatch {
        dispatch_results: VecDeque<FakeDispatchResult>,
        resume_results: VecDeque<FakeResumeResult>,
        arrivals: VecDeque<ReconciliationTrigger>,
        calls: Vec<ReconciliationWork>,
        resumed: Vec<u64>,
    }

    impl FakeDispatch {
        fn with_dispatch(results: impl IntoIterator<Item = FakeDispatchResult>) -> Self {
            Self {
                dispatch_results: results.into_iter().collect(),
                resume_results: VecDeque::new(),
                arrivals: VecDeque::new(),
                calls: Vec::new(),
                resumed: Vec::new(),
            }
        }
    }

    impl ReconciliationSessionDispatch for FakeDispatch {
        type Continuation = FakeContinuation;

        fn dispatch(
            &mut self,
            work: &ReconciliationWork,
            arrive: &mut dyn FnMut(ReconciliationTrigger),
        ) -> ReconciliationSessionDispatchOutcome<Self::Continuation> {
            self.calls.push(work.clone());
            for trigger in std::mem::take(&mut self.arrivals) {
                arrive(trigger);
            }
            match self
                .dispatch_results
                .pop_front()
                .expect("fixture must supply a dispatch result")
            {
                FakeDispatchResult::Noop => ReconciliationSessionDispatchOutcome::Noop,
                FakeDispatchResult::Complete => ReconciliationSessionDispatchOutcome::Complete,
                FakeDispatchResult::Blocked => ReconciliationSessionDispatchOutcome::Blocked,
                FakeDispatchResult::RetryFull => ReconciliationSessionDispatchOutcome::RetryFull,
                FakeDispatchResult::FailedClosed(identity) => {
                    ReconciliationSessionDispatchOutcome::FailedClosed(FakeContinuation(identity))
                }
            }
        }

        fn resume(
            &mut self,
            continuation: Self::Continuation,
        ) -> ReconciliationSessionDispatchOutcome<Self::Continuation> {
            self.resumed.push(continuation.0);
            match self
                .resume_results
                .pop_front()
                .expect("fixture must supply a resume result")
            {
                FakeResumeResult::Complete => ReconciliationSessionDispatchOutcome::Complete,
                FakeResumeResult::FailedClosed(identity) => {
                    ReconciliationSessionDispatchOutcome::FailedClosed(FakeContinuation(identity))
                }
            }
        }
    }

    fn paths(paths: &[&str]) -> BTreeSet<ManagedPath> {
        paths
            .iter()
            .map(|path| ManagedPath::parse(*path).unwrap())
            .collect()
    }

    fn session() -> ReconciliationSession<FakeContinuation> {
        ReconciliationSession::new(ReconciliationSchedulerLimits::default())
    }

    #[test]
    fn session_dispatches_targeted_work_exactly_once() {
        let mut session = session();
        session.trigger(ReconciliationTrigger::ProjectionPreconditionMismatch(
            paths(&["managed/nested/a.md", "journals/nonstandard/b.org"]),
        ));
        let mut dispatch = FakeDispatch::with_dispatch([FakeDispatchResult::Complete]);

        assert_eq!(
            session.step_with(&mut dispatch),
            Ok(ReconciliationSessionStep::Complete)
        );
        assert_eq!(dispatch.calls.len(), 1);
        assert_eq!(
            dispatch.calls,
            vec![ReconciliationWork::ProjectionPreconditionMismatch {
                paths: paths(&["journals/nonstandard/b.org", "managed/nested/a.md"]),
            }]
        );
        assert_eq!(
            session.status().last_completion,
            Some(ReconciliationCompletionOutcome::Complete)
        );
    }

    #[test]
    fn session_dispatches_full_work_exactly_once() {
        let mut session = session();
        session.trigger(ReconciliationTrigger::Explicit);
        let mut dispatch = FakeDispatch::with_dispatch([FakeDispatchResult::Noop]);

        assert_eq!(
            session.step_with(&mut dispatch),
            Ok(ReconciliationSessionStep::Noop)
        );
        assert_eq!(dispatch.calls.len(), 1);
        assert!(matches!(dispatch.calls[0], ReconciliationWork::FullScan(_)));
    }

    #[test]
    fn session_retry_full_maps_to_a_safe_full_scan() {
        let mut session = session();
        session.trigger(ReconciliationTrigger::WatcherPaths(paths(&[
            "pages/moved.md",
        ])));
        let mut dispatch = FakeDispatch::with_dispatch([FakeDispatchResult::RetryFull]);

        assert_eq!(
            session.step_with(&mut dispatch),
            Ok(ReconciliationSessionStep::RetryFull)
        );
        assert_eq!(
            session.status().last_completion,
            Some(ReconciliationCompletionOutcome::Retry)
        );

        dispatch
            .dispatch_results
            .push_back(FakeDispatchResult::Complete);
        assert_eq!(
            session.step_with(&mut dispatch),
            Ok(ReconciliationSessionStep::Complete)
        );
        assert!(matches!(dispatch.calls[1], ReconciliationWork::FullScan(_)));
    }

    #[test]
    fn session_blocked_completion_remains_observable() {
        let mut session = session();
        session.trigger(ReconciliationTrigger::WatcherPaths(paths(&[
            "pages/blocked.md",
        ])));
        let mut dispatch = FakeDispatch::with_dispatch([FakeDispatchResult::Blocked]);

        assert_eq!(
            session.step_with(&mut dispatch),
            Ok(ReconciliationSessionStep::Blocked)
        );
        assert_eq!(
            session.status().last_completion,
            Some(ReconciliationCompletionOutcome::Blocked)
        );
        assert!(session.status().blocked.is_some());
    }

    #[test]
    fn session_coalesces_full_triggers_while_idle() {
        let mut session = session();
        session.trigger(ReconciliationTrigger::Explicit);
        session.trigger(ReconciliationTrigger::Startup);
        session.trigger(ReconciliationTrigger::WatcherUncertain);
        let mut dispatch = FakeDispatch::with_dispatch([FakeDispatchResult::Complete]);

        assert_eq!(
            session.step_with(&mut dispatch),
            Ok(ReconciliationSessionStep::Complete)
        );
        let ReconciliationWork::FullScan(reasons) = &dispatch.calls[0] else {
            panic!("expected one coalesced full scan");
        };
        assert_eq!(
            reasons.reasons,
            BTreeSet::from([
                super::super::reconciliation_scan::ReconciliationFullScanReason::Explicit,
                super::super::reconciliation_scan::ReconciliationFullScanReason::Startup,
                super::super::reconciliation_scan::ReconciliationFullScanReason::WatcherUncertain,
            ])
        );
    }

    #[test]
    fn session_retains_triggers_arriving_during_a_step() {
        let mut session = session();
        session.trigger(ReconciliationTrigger::WatcherPaths(paths(&[
            "pages/first.md",
        ])));
        let mut dispatch = FakeDispatch::with_dispatch([
            FakeDispatchResult::Complete,
            FakeDispatchResult::Complete,
        ]);
        dispatch
            .arrivals
            .push_back(ReconciliationTrigger::ProjectionPreconditionMismatch(
                paths(&["pages/arrived.md"]),
            ));

        assert_eq!(
            session.step_with(&mut dispatch),
            Ok(ReconciliationSessionStep::Complete)
        );
        assert!(session.status().pending);
        assert_eq!(
            session.step_with(&mut dispatch),
            Ok(ReconciliationSessionStep::Complete)
        );
        assert_eq!(
            dispatch.calls[1],
            ReconciliationWork::ProjectionPreconditionMismatch {
                paths: paths(&["pages/arrived.md"]),
            }
        );
    }

    #[test]
    fn session_failed_closed_holds_lease_without_replanning_and_resumes_same_identity() {
        let mut session = session();
        session.trigger(ReconciliationTrigger::WatcherPaths(paths(&[
            "pages/published.md",
        ])));
        let mut dispatch = FakeDispatch::with_dispatch([FakeDispatchResult::FailedClosed(41)]);
        dispatch.resume_results.extend([
            FakeResumeResult::FailedClosed(41),
            FakeResumeResult::Complete,
        ]);

        let Ok(ReconciliationSessionStep::Pending(token)) = session.step_with(&mut dispatch) else {
            panic!("expected a retained failed-closed continuation");
        };
        session.trigger(ReconciliationTrigger::WatcherPaths(paths(&[
            "pages/later.md",
        ])));
        assert!(session.status().active);
        assert!(session.status().pending);
        assert_eq!(
            session.step_with(&mut dispatch),
            Err(ReconciliationSessionError::PendingContinuation(token))
        );
        assert_eq!(dispatch.calls.len(), 1);
        assert_eq!(
            session.resume_with(token, &mut dispatch),
            Ok(ReconciliationSessionStep::Pending(token))
        );
        assert!(session.status().active);
        assert_eq!(dispatch.resumed, vec![41]);
        assert_eq!(
            session.resume_with(token, &mut dispatch),
            Ok(ReconciliationSessionStep::Complete)
        );
        assert_eq!(dispatch.resumed, vec![41, 41]);
        assert!(!session.status().active);
        assert!(session.status().pending);
    }

    #[test]
    fn session_rejects_stale_foreign_and_double_resume_tokens() {
        let mut first = session();
        let mut second = session();
        first.trigger(ReconciliationTrigger::Explicit);
        second.trigger(ReconciliationTrigger::Explicit);
        let mut first_dispatch = FakeDispatch::with_dispatch([FakeDispatchResult::FailedClosed(1)]);
        first_dispatch
            .resume_results
            .push_back(FakeResumeResult::Complete);
        let mut second_dispatch =
            FakeDispatch::with_dispatch([FakeDispatchResult::FailedClosed(2)]);

        let Ok(ReconciliationSessionStep::Pending(first_token)) =
            first.step_with(&mut first_dispatch)
        else {
            panic!("expected first continuation");
        };
        let Ok(ReconciliationSessionStep::Pending(second_token)) =
            second.step_with(&mut second_dispatch)
        else {
            panic!("expected second continuation");
        };
        assert_ne!(first_token, second_token);
        assert_eq!(
            first.resume_with(second_token, &mut first_dispatch),
            Err(ReconciliationSessionError::StaleOrForeignContinuation)
        );
        assert_eq!(
            first.resume_with(first_token, &mut first_dispatch),
            Ok(ReconciliationSessionStep::Complete)
        );
        assert_eq!(
            first.resume_with(first_token, &mut first_dispatch),
            Err(ReconciliationSessionError::StaleOrForeignContinuation)
        );
    }

    #[test]
    fn sessions_are_independent_per_endpoint() {
        let mut first = session();
        let mut second = session();
        first.trigger(ReconciliationTrigger::WatcherPaths(paths(&[
            "pages/first.md",
        ])));
        second.trigger(ReconciliationTrigger::Explicit);
        let mut first_dispatch = FakeDispatch::with_dispatch([FakeDispatchResult::Complete]);
        let mut second_dispatch = FakeDispatch::with_dispatch([FakeDispatchResult::Complete]);

        assert_eq!(
            first.step_with(&mut first_dispatch),
            Ok(ReconciliationSessionStep::Complete)
        );
        assert_eq!(
            second.step_with(&mut second_dispatch),
            Ok(ReconciliationSessionStep::Complete)
        );
        assert!(matches!(
            first_dispatch.calls[0],
            ReconciliationWork::WatcherPaths { .. }
        ));
        assert!(matches!(
            second_dispatch.calls[0],
            ReconciliationWork::FullScan(_)
        ));
    }
}
