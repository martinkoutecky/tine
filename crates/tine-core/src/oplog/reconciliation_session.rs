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
        OperationalPhase,
    },
    reconciliation_import::{
        execute_stable_scan_import, ReconciliationImportBlockReason, ReconciliationImportBlocked,
        ReconciliationImportOutcome,
    },
    reconciliation_scan::{
        scan_graph_text, GraphTextScanLimits, JoinedAuthenticatedExpectedPathSource,
        ReconciliationCompletionOutcome, ReconciliationJob, ReconciliationLease,
        ReconciliationScheduler, ReconciliationSchedulerLimits, ReconciliationSchedulerStatus,
        ReconciliationTrigger, ReconciliationWork,
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
        let mut dispatch = LiveReconciliationSessionDispatch { dependencies };
        self.step_with(&mut dispatch)
    }

    /// Resume the exact retained post-publication continuation once.
    pub(crate) fn resume(
        &mut self,
        continuation: ReconciliationPendingContinuation,
        dependencies: ReconciliationSessionDependencies<'_>,
    ) -> Result<ReconciliationSessionStep, ReconciliationSessionError> {
        let mut dispatch = LiveReconciliationSessionDispatch { dependencies };
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
            Err(error) if error.phase() == OperationalPhase::Bindings => {
                ReconciliationSessionDispatchOutcome::RetryFull
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
                // An unavailable baseline/expected source is never a clean
                // outcome. It asks the bounded scheduler for a fresh full scan.
                Err(_) => return ReconciliationSessionDispatchOutcome::RetryFull,
            };
            let source = JoinedAuthenticatedExpectedPathSource::new(engine, projection);
            match scan_graph_text(graph, &source, GraphTextScanLimits::default()) {
                Ok(scan) => scan,
                // A stable scan failure (including uncertainty) has no
                // authority to infer exact operations; retry the full work.
                Err(_) => return ReconciliationSessionDispatchOutcome::RetryFull,
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
            ReconciliationImportOutcome::Blocked(ReconciliationImportBlocked::Discovery(
                discovery,
            )) if discovery.reason
                == ReconciliationImportBlockReason::ExpectedAuthorityUnavailable =>
            {
                ReconciliationSessionDispatchOutcome::RetryFull
            }
            ReconciliationImportOutcome::Blocked(
                ReconciliationImportBlocked::CoordinatorError(error),
            ) if error.phase() == OperationalPhase::Bindings => {
                ReconciliationSessionDispatchOutcome::RetryFull
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
        _arrive: &mut dyn FnMut(ReconciliationTrigger),
    ) -> ReconciliationSessionDispatchOutcome<Self::Continuation> {
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
    use std::collections::{BTreeSet, VecDeque};

    use super::*;

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
