//! Actor-owned exact external Markdown/Org feed execution.
//!
//! This module is the production core seam between normalized platform watcher
//! observations and the sparse-oplog reconciliation path. It deliberately has
//! crate visibility only. A later runtime actor may own an
//! [`ExactExternalFeedState`] while retaining the sole promoted runtime,
//! `LocalActive` authority, and watcher queue for the root.
//!
//! The move-only state binds bounded work and continuations to that actor's
//! exact storage and opaque queue identities. Each step borrows the actor-owned
//! runtime/authority pair and uses only its watcher intake/drain/ack/abandon
//! APIs. Queue epochs are acknowledged only after admitted reconciliation,
//! exact Graph feed/cache publication, and initial catch-up publication reach
//! terminal `Noop` or `Complete`. Every coarser or incomplete result keeps the
//! epoch owed. Uncertainty rebuilds the complete exact Graph index at the queue
//! fence before complete reconciliation; it never guesses a managed path.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::fmt;

use crate::model::{
    Graph, GraphTextExactFeedFailure, GraphTextExactFeedLease, GraphTextExactFeedPathClass,
};

use super::{
    hot_engine::ProjectionStorageBinding,
    local_active::{
        ExternalImportAdmission, LocalActiveAuthority, PromotedLocalRuntime, RuntimePromotionError,
        RuntimeRevocation,
    },
    reconciliation_baseline::{BaselineTimestamp, ReconciliationBaseline},
    reconciliation_scan::{ReconciliationSchedulerLimits, ReconciliationTrigger},
    reconciliation_session::{
        ReconciliationPendingContinuation, ReconciliationSession,
        ReconciliationSessionDependencies, ReconciliationSessionStep,
        ReconciliationTerminalChangedPaths,
    },
    watcher_queue::{
        WatcherDrainError, WatcherEnqueueError, WatcherEpoch, WatcherObservation,
        WatcherSettlementError,
    },
    ManagedPath, ProjectionReceiptStore,
};

const EXACT_FEED_MAXIMUM_PATHS: usize = 256;
const EXACT_FEED_MAXIMUM_PATH_BYTES: usize = 64 * 1024;

/// Construction failed before a platform-facing observer existed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExactExternalFeedOpenError {
    detail: String,
}

impl ExactExternalFeedOpenError {
    fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }

    pub(crate) fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for ExactExternalFeedOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for ExactExternalFeedOpenError {}

/// Watcher intake was refused without consuming or acknowledging another
/// queue's work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ExactExternalFeedObserveError {
    ForeignActor,
    Terminal,
    Queue(WatcherEnqueueError),
}

impl fmt::Display for ExactExternalFeedObserveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ForeignActor => {
                formatter.write_str("exact external feed observation has a foreign runtime")
            }
            Self::Terminal => formatter.write_str("exact external feed owner is terminal"),
            Self::Queue(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ExactExternalFeedObserveError {}

/// Permanent stop reason for one owner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ExactExternalFeedTerminal {
    WorkspaceAuthorityRevoked(RuntimeRevocation),
    RuntimeAuthority(String),
    GraphFeed(String),
    Queue(String),
}

impl fmt::Display for ExactExternalFeedTerminal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WorkspaceAuthorityRevoked(revocation) => revocation.fmt(formatter),
            Self::RuntimeAuthority(detail) => write!(
                formatter,
                "exact external feed runtime authority is terminal: {detail}"
            ),
            Self::GraphFeed(detail) => {
                write!(formatter, "exact external Graph feed is terminal: {detail}")
            }
            Self::Queue(detail) => {
                write!(
                    formatter,
                    "exact external watcher queue is terminal: {detail}"
                )
            }
        }
    }
}

/// Result of one bounded actor turn.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ExactExternalFeedDrain {
    Idle,
    /// The caller did not present the exact actor-owned runtime/authority pair.
    /// No watcher queue or feed state was touched.
    ForeignActor,
    /// The promoted runtime did not adopt a clean `Safe` handoff and does not
    /// currently own an in-progress or completed startup full-scan recovery
    /// catch-up. Work remains queued and no external import authority is
    /// granted.
    RecoveryBlocked(&'static str),
    /// A durable coordinator continuation or required follow-up full scan is
    /// retained by this owner. The queue epoch remains in flight and unacked.
    Recovering,
    /// The complete scan could not yet reach one stable admitted result. The
    /// same queue epoch remains in flight and unacked.
    RetryFull,
    /// Reconciliation reached a stable blocked result. The queue epoch remains
    /// unacknowledged, and a published continuation remains retained when the
    /// detail names retry exhaustion.
    Blocked(String),
    /// A retryable, pre-ack operation failed. The queue epoch remains owed.
    Failed(String),
    AdmittedNoop {
        epoch: u64,
    },
    AdmittedComplete {
        epoch: u64,
        batch_id: Option<super::BatchId>,
    },
    Terminal(ExactExternalFeedTerminal),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ActiveDrainScope {
    Exact(BTreeSet<ManagedPath>),
    FullScan,
}

struct ActiveDrain {
    epoch: WatcherEpoch,
    scope: ActiveDrainScope,
    continuation: Option<ReconciliationPendingContinuation>,
    rebase_before_step: bool,
    retry_rebase: bool,
    retry_rebases: u8,
}

/// Move-only bounded exact-feed state for one actor-owned promoted runtime.
///
/// There is intentionally no `Clone`, no runtime/authority/queue constructor,
/// and no accessor for the feed lease, continuation, or baseline. Watcher
/// intake and settlement always pass through a borrowed runtime.
pub(crate) struct ExactExternalFeedState {
    binding: ProjectionStorageBinding,
    actor_session_id: super::SessionId,
    actor_verification_digest: super::ContentDigest,
    watcher_queue_anchor: WatcherEpoch,
    lease: GraphTextExactFeedLease,
    reconciliation: ReconciliationSession,
    baseline: ReconciliationBaseline,
    active: Option<ActiveDrain>,
    /// The lease is armed during fast runtime open, but its graph-wide index is
    /// intentionally not built until the first held uncertainty epoch.
    initial_index_build_pending: bool,
    feed_sequence: u64,
    caught_up_published: bool,
    terminal: Option<ExactExternalFeedTerminal>,
    #[cfg(test)]
    initial_build_count: u64,
    #[cfg(test)]
    rebase_count: u64,
    #[cfg(test)]
    before_second_scan_pass: Option<Box<dyn FnMut()>>,
}

impl fmt::Debug for ExactExternalFeedState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExactExternalFeedState")
            .field("terminal", &self.terminal)
            .field("feed_sequence", &self.feed_sequence)
            .finish_non_exhaustive()
    }
}

impl ExactExternalFeedState {
    /// Bind bounded feed state to one borrowed actor runtime.
    ///
    /// Every fresh owner arms its exact feed and seeds one uncertainty epoch,
    /// but does no graph-wide enumeration during runtime open. Its first
    /// admitted drain builds the initial index at that held epoch's fence and
    /// fully reconciles before it can publish caught-up authority or acknowledge
    /// the queue. This closes both the initial-build/watch-install race and the
    /// process-crash loss of an in-memory queue without treating a prior `Safe`
    /// handoff as proof that the closed graph was unchanged.
    pub(crate) fn open(
        graph: &Graph,
        receipts: &ProjectionReceiptStore,
        runtime: &PromotedLocalRuntime,
        baseline: ReconciliationBaseline,
    ) -> Result<Self, ExactExternalFeedOpenError> {
        let binding = validate_open_binding(graph, receipts, runtime, &baseline)?;
        let lease = graph
            .arm_graph_text_exact_feed(0)
            .map_err(|error| ExactExternalFeedOpenError::new(error.to_string()))?;

        // This is not optional bookkeeping. A new process cannot reconstruct
        // the predecessor's in-memory watcher epoch, so it always owes one
        // complete scan before any exact epoch may be acknowledged.
        let intake = runtime
            .watcher_handle()
            .enqueue(binding, [WatcherObservation::RescanRequired])
            .map_err(|error| ExactExternalFeedOpenError::new(error.to_string()))?;

        Ok(Self {
            binding,
            actor_session_id: runtime.session_id(),
            actor_verification_digest: runtime.verification_digest(),
            watcher_queue_anchor: intake.epoch,
            lease,
            reconciliation: ReconciliationSession::new(ReconciliationSchedulerLimits {
                maximum_watcher_paths: EXACT_FEED_MAXIMUM_PATHS,
                maximum_watcher_path_bytes: EXACT_FEED_MAXIMUM_PATH_BYTES,
                maximum_precondition_paths: EXACT_FEED_MAXIMUM_PATHS,
                maximum_precondition_path_bytes: EXACT_FEED_MAXIMUM_PATH_BYTES,
                maximum_full_scan_reasons: 8,
            }),
            baseline,
            active: None,
            initial_index_build_pending: true,
            feed_sequence: 0,
            caught_up_published: false,
            terminal: None,
            #[cfg(test)]
            initial_build_count: 0,
            #[cfg(test)]
            rebase_count: 0,
            #[cfg(test)]
            before_second_scan_pass: None,
        })
    }

    pub(crate) fn terminal(&self) -> Option<&ExactExternalFeedTerminal> {
        self.terminal.as_ref()
    }

    /// Whether the only retained watcher work is the uncertainty seeded by
    /// this fresh owner. A clean-handoff reader may use the already-authenticated
    /// managed projection while this scan remains owed; any real watcher input
    /// advances the queue and closes that narrow read-only fast path.
    pub(crate) fn only_startup_catch_up_pending(&self, runtime: &PromotedLocalRuntime) -> bool {
        let queue = runtime.watcher_status();
        queue.pending
            && queue.latest_enqueue == self.watcher_queue_anchor
            && queue.acknowledged.sequence() < self.watcher_queue_anchor.sequence()
            && queue.drain_in_flight.is_none()
    }

    /// Submit normalized watcher observations through the existing bounded
    /// watcher queue.
    ///
    /// Exact paths are reclassified against the retained Graph scope before
    /// intake. An excluded path is conservatively made uncertain; a
    /// configuration mutation is terminal because its scope can be recovered
    /// only by opening a fresh Graph/runtime owner.
    pub(crate) fn observe(
        &mut self,
        graph: &Graph,
        runtime: &PromotedLocalRuntime,
        observations: impl IntoIterator<Item = WatcherObservation>,
    ) -> Result<(), ExactExternalFeedObserveError> {
        if self.terminal.is_some() {
            return Err(ExactExternalFeedObserveError::Terminal);
        }
        if !self.matches_actor_runtime(runtime) {
            return Err(ExactExternalFeedObserveError::ForeignActor);
        }
        // Classification stays lazy: the watcher queue stops polling as soon
        // as uncertainty or overflow subsumes the rest of the batch. This
        // adapter therefore cannot materialize an arbitrarily large callback
        // merely to validate it.
        let configuration_mutated = Cell::new(false);
        let classification_error = RefCell::new(None::<String>);
        let normalized = observations
            .into_iter()
            .map(|observation| match observation {
                WatcherObservation::ManagedPath(path) => {
                    match graph.classify_graph_text_exact_feed_path(path.as_str()) {
                        Ok(GraphTextExactFeedPathClass::RetainedFile) => {
                            WatcherObservation::ManagedPath(path)
                        }
                        Ok(GraphTextExactFeedPathClass::Excluded) => {
                            WatcherObservation::UnknownPath
                        }
                        Ok(GraphTextExactFeedPathClass::Configuration) => {
                            configuration_mutated.set(true);
                            WatcherObservation::UnknownPath
                        }
                        Err(error) => {
                            *classification_error.borrow_mut() = Some(error.to_string());
                            WatcherObservation::UnknownPath
                        }
                    }
                }
                uncertain => uncertain,
            });
        let enqueue = self
            .runtime_watcher_handle(runtime)
            .enqueue(self.binding, normalized)
            .map_err(ExactExternalFeedObserveError::Queue);
        if let Some(detail) = classification_error.into_inner() {
            let _ = graph.poison_graph_text_exact_feed(
                &self.lease,
                GraphTextExactFeedFailure::RootMutation,
                &detail,
            );
            self.terminal = Some(ExactExternalFeedTerminal::GraphFeed(detail));
            return Err(ExactExternalFeedObserveError::Terminal);
        }
        if configuration_mutated.get() {
            let detail = "graph-text configuration changed; a fresh runtime reopen is required";
            let _ = graph.poison_graph_text_exact_feed(
                &self.lease,
                GraphTextExactFeedFailure::ScopeOrConfigMutation,
                detail,
            );
            self.terminal = Some(ExactExternalFeedTerminal::GraphFeed(detail.to_owned()));
            return Err(ExactExternalFeedObserveError::Terminal);
        }
        enqueue.map(|_| ())
    }

    /// Drive at most one queue epoch toward terminal admission.
    ///
    /// A continuation/follow-up scan may require another actor turn, but there
    /// is never more than one queue drain in flight and no nonterminal result
    /// acknowledges it.
    pub(crate) fn drain_one(
        &mut self,
        graph: &Graph,
        receipts: &ProjectionReceiptStore,
        authority: &mut LocalActiveAuthority,
        runtime: &mut PromotedLocalRuntime,
        observed_at: BaselineTimestamp,
    ) -> ExactExternalFeedDrain {
        if let Some(terminal) = &self.terminal {
            return ExactExternalFeedDrain::Terminal(terminal.clone());
        }
        if !self.matches_actor_runtime(runtime) || !runtime.owns_local_active_authority(authority) {
            return ExactExternalFeedDrain::ForeignActor;
        }
        if let Some(revocation) = runtime.workspace_authority_revocation() {
            return self.stop_revoked(graph, runtime, revocation);
        }
        if let Err(result) =
            self.admit_automatic_external_import_recovery(graph, authority, runtime)
        {
            return result;
        }
        if let Err(error) = validate_live_binding(graph, receipts, runtime, self.binding) {
            return self.stop_runtime(graph, runtime, error.detail);
        }

        if self.active.is_none() {
            let drain = match runtime.begin_watcher_drain() {
                Ok(Some(drain)) => drain,
                Ok(None) => return ExactExternalFeedDrain::Idle,
                Err(error) => return self.handle_drain_error(graph, runtime, error),
            };
            let scope = match drain.trigger() {
                ReconciliationTrigger::WatcherPaths(paths) => {
                    ActiveDrainScope::Exact(paths.clone())
                }
                ReconciliationTrigger::WatcherUncertain => ActiveDrainScope::FullScan,
                _ => {
                    let detail =
                        "watcher queue produced a non-watcher reconciliation trigger".to_owned();
                    let _ = runtime.abandon_watcher_drain(drain.epoch());
                    return self.stop_queue(graph, runtime, detail);
                }
            };
            if self.initial_index_build_pending && !matches!(scope, ActiveDrainScope::FullScan) {
                let detail =
                    "initial exact-feed catch-up lost its owed full-scan uncertainty".to_owned();
                let _ = runtime.abandon_watcher_drain(drain.epoch());
                return self.stop_queue(graph, runtime, detail);
            }
            self.active = Some(ActiveDrain {
                epoch: drain.epoch(),
                rebase_before_step: matches!(scope, ActiveDrainScope::FullScan),
                retry_rebase: false,
                retry_rebases: 0,
                scope,
                continuation: None,
            });
        }

        if self
            .active
            .as_ref()
            .is_some_and(|active| active.rebase_before_step)
        {
            let epoch = self
                .active
                .as_ref()
                .expect("active drain disappeared")
                .epoch;
            let initial_build = self.initial_index_build_pending;
            let rebuilt = if initial_build {
                graph.build_graph_text_exact_feed_at_fence(&self.lease, epoch.sequence())
            } else {
                graph.rebase_graph_text_exact_feed_at_fence(&self.lease, epoch.sequence())
            };
            match rebuilt {
                Ok(()) => {
                    self.feed_sequence = epoch.sequence();
                    self.initial_index_build_pending = false;
                    #[cfg(test)]
                    {
                        if initial_build {
                            self.initial_build_count += 1;
                        } else {
                            self.rebase_count += 1;
                        }
                    }
                    let active = self.active.as_mut().expect("active drain disappeared");
                    active.rebase_before_step = false;
                    if active.retry_rebase {
                        active.retry_rebases = active.retry_rebases.saturating_add(1);
                        active.retry_rebase = false;
                    }
                }
                Err(error) => {
                    if let Some(revocation) = runtime.workspace_authority_revocation() {
                        return self.stop_revoked(graph, runtime, revocation);
                    }
                    if !self.lease.is_terminal() {
                        return ExactExternalFeedDrain::Failed(error.to_string());
                    }
                    return self.stop_graph(
                        graph,
                        runtime,
                        GraphTextExactFeedFailure::BackendError,
                        &error.to_string(),
                    );
                }
            }
        }

        let is_new_job = self
            .active
            .as_ref()
            .is_some_and(|active| active.continuation.is_none())
            && !self.reconciliation.status().active;
        if is_new_job {
            let trigger = match &self
                .active
                .as_ref()
                .expect("active drain disappeared")
                .scope
            {
                ActiveDrainScope::Exact(paths) => {
                    ReconciliationTrigger::WatcherPaths(paths.clone())
                }
                ActiveDrainScope::FullScan => ReconciliationTrigger::WatcherUncertain,
            };
            self.reconciliation.trigger(trigger);
        }

        let continuation = self
            .active
            .as_mut()
            .expect("active drain disappeared")
            .continuation
            .take();
        let step = match self.execute_reconciliation(
            graph,
            receipts,
            authority,
            runtime,
            observed_at,
            continuation,
        ) {
            Ok(step) => step,
            Err(ExecuteReconciliationError::Revoked(revocation)) => {
                return self.stop_revoked(graph, runtime, revocation);
            }
            Err(ExecuteReconciliationError::Runtime(detail)) => {
                return self.stop_runtime(graph, runtime, detail);
            }
        };
        match step {
            ReconciliationSessionStep::Pending(continuation) => {
                self.active
                    .as_mut()
                    .expect("active drain disappeared")
                    .continuation = Some(continuation);
                ExactExternalFeedDrain::Recovering
            }
            ReconciliationSessionStep::PublishedBlocked(continuation) => {
                let detail = self
                    .reconciliation
                    .published_blocked_detail(continuation)
                    .expect("published blocked step must retain exact failure evidence")
                    .to_owned();
                self.active
                    .as_mut()
                    .expect("active drain disappeared")
                    .continuation = Some(continuation);
                ExactExternalFeedDrain::Blocked(detail)
            }
            ReconciliationSessionStep::RetryFull => {
                let active = self.active.as_mut().expect("active drain disappeared");
                if matches!(active.scope, ActiveDrainScope::Exact(_)) {
                    // A targeted scan which asks for RetryFull has made its
                    // exact hint insufficient. Collapse this same queue epoch
                    // to a full rebase.
                    active.scope = ActiveDrainScope::FullScan;
                    active.rebase_before_step = true;
                    active.retry_rebase = true;
                } else if active.retry_rebases == 0 {
                    // Refresh once for the two-pass race. Further instability
                    // gets a bounded retry cycle below instead of repeatedly
                    // rebuilding the complete graph-wide index in one watcher
                    // turn.
                    active.rebase_before_step = true;
                    active.retry_rebase = true;
                } else {
                    // Retain and re-arm this exact fenced epoch, but yield a
                    // failed cycle so the platform retry schedule applies
                    // backoff before another graph-wide rebase. No epoch is
                    // acknowledged and no late-path race is discarded.
                    active.rebase_before_step = true;
                    active.retry_rebase = false;
                    active.retry_rebases = 0;
                    return ExactExternalFeedDrain::Failed(
                        "continuously unstable full-scan epoch retained for bounded retry".into(),
                    );
                }
                ExactExternalFeedDrain::RetryFull
            }
            ReconciliationSessionStep::Blocked => {
                self.reconciliation.take_terminal_changed_paths();
                let detail = self
                    .reconciliation
                    .take_terminal_blocked_detail()
                    .unwrap_or_else(|| "exact external reconciliation is blocked".to_owned());
                self.abandon_active(runtime);
                ExactExternalFeedDrain::Blocked(detail)
            }
            ReconciliationSessionStep::Idle => {
                let detail =
                    "active exact-feed drain reached an idle reconciliation session".to_owned();
                self.stop_runtime(graph, runtime, detail)
            }
            ReconciliationSessionStep::Noop | ReconciliationSessionStep::Complete => {
                let Some(changed_paths) = self.reconciliation.take_terminal_changed_paths() else {
                    // A stable full scan may require one post-drain confirmation
                    // scan. The intermediate semantic outcome is not terminal
                    // for the queue epoch and therefore cannot be acknowledged.
                    // The exact index was already rebuilt at this epoch's fence;
                    // the confirmation pass must not publish a second rebuild
                    // or fold a later watcher epoch into this one.
                    return ExactExternalFeedDrain::Recovering;
                };
                self.finish_terminal(graph, authority, runtime, step, changed_paths)
            }
        }
    }

    fn admit_automatic_external_import_recovery(
        &mut self,
        graph: &Graph,
        authority: &mut LocalActiveAuthority,
        runtime: &mut PromotedLocalRuntime,
    ) -> Result<(), ExactExternalFeedDrain> {
        match runtime.automatic_external_import() {
            ExternalImportAdmission::Allowed => Ok(()),
            ExternalImportAdmission::Blocked(_) if self.recovery_catch_up_complete(runtime) => {
                runtime
                    .complete_automatic_external_import_recovery(authority, graph)
                    .map_err(Self::recovery_completion_failed)
            }
            ExternalImportAdmission::Blocked(_)
                if self.recovery_catch_up_owed_or_in_progress(runtime) =>
            {
                Ok(())
            }
            ExternalImportAdmission::Blocked(reason) => {
                Err(ExactExternalFeedDrain::RecoveryBlocked(reason))
            }
        }
    }

    fn recovery_catch_up_owed_or_in_progress(&self, runtime: &PromotedLocalRuntime) -> bool {
        !self.recovery_catch_up_complete(runtime)
    }

    fn recovery_catch_up_complete(&self, runtime: &PromotedLocalRuntime) -> bool {
        let queue = runtime.watcher_status();
        self.caught_up_published
            && !self.initial_index_build_pending
            && self.active.is_none()
            // Caught-up publication is not recovery completion on its own: an
            // after-terminal-before-ack failure leaves this held startup epoch
            // owed even though publication succeeded.
            && queue.acknowledged.sequence() >= self.watcher_queue_anchor.sequence()
            // Do not open automatic import while any later watcher work is
            // still retained, in flight, or deferred behind this actor turn.
            && !queue.pending
            && queue.drain_in_flight.is_none()
            && !queue.deferred
            && !queue.sequence_exhausted
    }

    fn recovery_completion_failed(error: RuntimePromotionError) -> ExactExternalFeedDrain {
        ExactExternalFeedDrain::Failed(format!(
            "automatic external import recovery catch-up could not be authenticated: {error}"
        ))
    }

    fn finish_terminal(
        &mut self,
        graph: &Graph,
        authority: &mut LocalActiveAuthority,
        runtime: &mut PromotedLocalRuntime,
        step: ReconciliationSessionStep,
        changed_paths: ReconciliationTerminalChangedPaths,
    ) -> ExactExternalFeedDrain {
        let (epoch, scope) = {
            let active = self.active.as_ref().expect("terminal drain disappeared");
            (active.epoch, active.scope.clone())
        };
        match (&scope, changed_paths.complete_scan()) {
            (ActiveDrainScope::FullScan, true) => {
                if !changed_paths.exact_paths().is_empty() || self.feed_sequence != epoch.sequence()
                {
                    return self.stop_runtime(
                        graph,
                        runtime,
                        "terminal full reconciliation did not match its exact feed fence",
                    );
                }
            }
            (ActiveDrainScope::Exact(expected), false)
                if expected == changed_paths.exact_paths() =>
            {
                if self.feed_sequence < epoch.sequence() {
                    let Some(first_sequence) = self.feed_sequence.checked_add(1) else {
                        return self.stop_graph(
                            graph,
                            runtime,
                            GraphTextExactFeedFailure::SequenceDiscontinuity,
                            "exact feed sequence exhausted",
                        );
                    };
                    let batch = match self.lease.batch(
                        first_sequence,
                        epoch.sequence(),
                        changed_paths
                            .exact_paths()
                            .iter()
                            .map(|path| path.as_str().to_owned()),
                    ) {
                        Ok(batch) => batch,
                        Err(error) => {
                            return self.stop_graph(
                                graph,
                                runtime,
                                GraphTextExactFeedFailure::UnsupportedOrAmbiguousEvent,
                                &error.to_string(),
                            );
                        }
                    };
                    if let Err(error) = graph.apply_graph_text_exact_feed_batch(&self.lease, batch)
                    {
                        return self.stop_graph(
                            graph,
                            runtime,
                            GraphTextExactFeedFailure::BackendError,
                            &error.to_string(),
                        );
                    }
                    self.feed_sequence = epoch.sequence();
                } else if self.feed_sequence != epoch.sequence() {
                    return self.stop_graph(
                        graph,
                        runtime,
                        GraphTextExactFeedFailure::SequenceDiscontinuity,
                        "exact queue epoch moved behind the Graph feed",
                    );
                }
            }
            _ => {
                return self.stop_runtime(
                    graph,
                    runtime,
                    "terminal reconciliation changed-path report differs from its queue epoch",
                );
            }
        }

        if !self.caught_up_published {
            if let Err(error) =
                graph.publish_graph_text_exact_feed_caught_up(&self.lease, self.feed_sequence)
            {
                return self.stop_graph(
                    graph,
                    runtime,
                    GraphTextExactFeedFailure::BackendError,
                    &error.to_string(),
                );
            }
            self.caught_up_published = true;
        }

        if let Err(error) = exact_feed_after_terminal_before_ack_hook() {
            let detail = error.to_string();
            self.abandon_active(runtime);
            return ExactExternalFeedDrain::Failed(detail);
        }

        if let Err(error) = runtime.acknowledge_watcher_drain(epoch) {
            return self.handle_settlement_error(graph, runtime, error);
        }
        self.active = None;
        if let Err(result) =
            self.admit_automatic_external_import_recovery(graph, authority, runtime)
        {
            return result;
        }
        let completed_batch = self.reconciliation.take_completed_batch();
        match step {
            ReconciliationSessionStep::Noop if completed_batch.is_some() => {
                ExactExternalFeedDrain::AdmittedComplete {
                    epoch: epoch.sequence(),
                    batch_id: completed_batch,
                }
            }
            ReconciliationSessionStep::Noop => ExactExternalFeedDrain::AdmittedNoop {
                epoch: epoch.sequence(),
            },
            ReconciliationSessionStep::Complete => ExactExternalFeedDrain::AdmittedComplete {
                epoch: epoch.sequence(),
                batch_id: completed_batch,
            },
            _ => unreachable!("finish_terminal accepts only admitted terminal outcomes"),
        }
    }

    fn abandon_active(&mut self, runtime: &PromotedLocalRuntime) {
        let Some(active) = self.active.take() else {
            return;
        };
        let _ = runtime.abandon_watcher_drain(active.epoch);
    }

    fn handle_drain_error(
        &mut self,
        graph: &Graph,
        runtime: &PromotedLocalRuntime,
        error: WatcherDrainError,
    ) -> ExactExternalFeedDrain {
        match error {
            WatcherDrainError::DrainInFlight(_) => self.stop_queue(
                graph,
                runtime,
                "watcher queue has an unowned drain in flight".to_owned(),
            ),
            WatcherDrainError::ForeignBinding => {
                self.stop_queue(graph, runtime, "watcher queue binding changed".to_owned())
            }
            WatcherDrainError::Quiescing => {
                ExactExternalFeedDrain::Failed("watcher queue is quiescing".to_owned())
            }
        }
    }

    fn handle_settlement_error(
        &mut self,
        graph: &Graph,
        runtime: &PromotedLocalRuntime,
        error: WatcherSettlementError,
    ) -> ExactExternalFeedDrain {
        self.stop_queue(
            graph,
            runtime,
            format!("terminal queue acknowledgement was refused: {error}"),
        )
    }

    fn stop_revoked(
        &mut self,
        graph: &Graph,
        runtime: &PromotedLocalRuntime,
        revocation: RuntimeRevocation,
    ) -> ExactExternalFeedDrain {
        self.abandon_active(runtime);
        let detail = revocation.to_string();
        let _ = graph.poison_graph_text_exact_feed(
            &self.lease,
            GraphTextExactFeedFailure::BackendError,
            &detail,
        );
        let terminal = ExactExternalFeedTerminal::WorkspaceAuthorityRevoked(revocation);
        self.terminal = Some(terminal.clone());
        ExactExternalFeedDrain::Terminal(terminal)
    }

    fn stop_runtime(
        &mut self,
        graph: &Graph,
        runtime: &PromotedLocalRuntime,
        detail: impl Into<String>,
    ) -> ExactExternalFeedDrain {
        self.abandon_active(runtime);
        let detail = detail.into();
        let _ = graph.poison_graph_text_exact_feed(
            &self.lease,
            GraphTextExactFeedFailure::BackendError,
            &detail,
        );
        let terminal = ExactExternalFeedTerminal::RuntimeAuthority(detail);
        self.terminal = Some(terminal.clone());
        ExactExternalFeedDrain::Terminal(terminal)
    }

    fn stop_graph(
        &mut self,
        graph: &Graph,
        runtime: &PromotedLocalRuntime,
        reason: GraphTextExactFeedFailure,
        detail: &str,
    ) -> ExactExternalFeedDrain {
        self.abandon_active(runtime);
        let _ = graph.poison_graph_text_exact_feed(&self.lease, reason, detail);
        let terminal = ExactExternalFeedTerminal::GraphFeed(detail.to_owned());
        self.terminal = Some(terminal.clone());
        ExactExternalFeedDrain::Terminal(terminal)
    }

    fn stop_queue(
        &mut self,
        graph: &Graph,
        runtime: &PromotedLocalRuntime,
        detail: String,
    ) -> ExactExternalFeedDrain {
        self.abandon_active(runtime);
        let _ = graph.poison_graph_text_exact_feed(
            &self.lease,
            GraphTextExactFeedFailure::OverflowOrQueueLoss,
            &detail,
        );
        let terminal = ExactExternalFeedTerminal::Queue(detail);
        self.terminal = Some(terminal.clone());
        ExactExternalFeedDrain::Terminal(terminal)
    }

    fn execute_reconciliation(
        &mut self,
        graph: &Graph,
        receipts: &ProjectionReceiptStore,
        authority: &mut LocalActiveAuthority,
        runtime: &mut PromotedLocalRuntime,
        observed_at: BaselineTimestamp,
        continuation: Option<ReconciliationPendingContinuation>,
    ) -> Result<ReconciliationSessionStep, ExecuteReconciliationError> {
        #[cfg(test)]
        let before_second_scan_pass = self.before_second_scan_pass.take();
        let Self {
            reconciliation,
            baseline,
            ..
        } = self;
        let mut window = match runtime.admit_promoted_mutation(authority, graph) {
            Ok(window) => window,
            Err(error) => {
                return Err(runtime
                    .workspace_authority_revocation()
                    .map(ExecuteReconciliationError::Revoked)
                    .unwrap_or_else(|| ExecuteReconciliationError::Runtime(error.to_string())));
            }
        };
        let (admission, engine, database, tail, bootstrap) = match window.parts_with_bootstrap() {
            Ok(parts) => parts,
            Err(error) => {
                return Err(error
                    .revocation()
                    .cloned()
                    .map(ExecuteReconciliationError::Revoked)
                    .unwrap_or_else(|| ExecuteReconciliationError::Runtime(error.to_string())));
            }
        };
        let dependencies = ReconciliationSessionDependencies {
            admission: &admission,
            graph,
            receipts,
            engine,
            database,
            tail,
            bootstrap: Some(bootstrap),
            baseline,
            observed_at,
        };
        match continuation {
            Some(token) => reconciliation.resume(token, dependencies),
            None => {
                #[cfg(test)]
                {
                    if let Some(hook) = before_second_scan_pass {
                        reconciliation.step_with_before_second_scan_pass(dependencies, hook)
                    } else {
                        reconciliation.step(dependencies)
                    }
                }
                #[cfg(not(test))]
                {
                    reconciliation.step(dependencies)
                }
            }
        }
        .map_err(|error| {
            ExecuteReconciliationError::Runtime(format!(
                "reconciliation session refused its owned action: {error:?}"
            ))
        })
    }

    fn matches_actor_runtime(&self, runtime: &PromotedLocalRuntime) -> bool {
        runtime.session_id() == self.actor_session_id
            && runtime.verification_digest() == self.actor_verification_digest
            && runtime.endpoint() == self.binding.endpoint
            && runtime.owns_watcher_epoch(self.watcher_queue_anchor)
    }

    fn runtime_watcher_handle(
        &self,
        runtime: &PromotedLocalRuntime,
    ) -> super::watcher_queue::WatcherHandle {
        debug_assert!(self.matches_actor_runtime(runtime));
        runtime.watcher_handle()
    }
}

enum ExecuteReconciliationError {
    Revoked(RuntimeRevocation),
    Runtime(String),
}

fn validate_open_binding(
    graph: &Graph,
    receipts: &ProjectionReceiptStore,
    runtime: &PromotedLocalRuntime,
    baseline: &ReconciliationBaseline,
) -> Result<ProjectionStorageBinding, ExactExternalFeedOpenError> {
    let graph_resource = graph
        .canonical_resource_id()
        .map_err(|error| ExactExternalFeedOpenError::new(error.to_string()))?;
    let scope_binding = graph
        .graph_text_scope_binding()
        .map_err(|error| ExactExternalFeedOpenError::new(error.to_string()))?;
    let endpoint = runtime.endpoint();
    let receipt_store_id = runtime
        .engine()
        .projection_receipt_store_id()
        .ok_or_else(|| ExactExternalFeedOpenError::new("promoted engine has no receipt binding"))?;
    let binding = ProjectionStorageBinding {
        endpoint,
        receipt_store_id,
    };
    if endpoint.graph_resource_id() != graph_resource
        || receipts.workspace_id() != runtime.engine().workspace_id()
        || receipts.endpoint_binding() != Some(endpoint)
        || receipts.store_id() != receipt_store_id
        || runtime.engine().projection_endpoint_binding() != Some(endpoint)
    {
        return Err(ExactExternalFeedOpenError::new(
            "Graph, promoted runtime, engine, and receipt-store binding differ",
        ));
    }
    let baseline_binding = baseline.binding();
    if baseline_binding.workspace() != runtime.engine().workspace_id()
        || baseline_binding.endpoint() != endpoint.endpoint_id()
        || baseline_binding.graph_resource() != graph_resource
        || baseline_binding.scope_binding() != scope_binding
    {
        return Err(ExactExternalFeedOpenError::new(
            "reconciliation baseline has a foreign runtime or Graph binding",
        ));
    }
    Ok(binding)
}

fn validate_live_binding(
    graph: &Graph,
    receipts: &ProjectionReceiptStore,
    runtime: &PromotedLocalRuntime,
    expected: ProjectionStorageBinding,
) -> Result<(), ExactExternalFeedOpenError> {
    let observed = validate_runtime_storage_binding(graph, receipts, runtime)?;
    if observed != expected {
        return Err(ExactExternalFeedOpenError::new(
            "exact external feed runtime binding changed",
        ));
    }
    Ok(())
}

fn validate_runtime_storage_binding(
    graph: &Graph,
    receipts: &ProjectionReceiptStore,
    runtime: &PromotedLocalRuntime,
) -> Result<ProjectionStorageBinding, ExactExternalFeedOpenError> {
    let endpoint = runtime.endpoint();
    let receipt_store_id = runtime
        .engine()
        .projection_receipt_store_id()
        .ok_or_else(|| ExactExternalFeedOpenError::new("promoted engine has no receipt binding"))?;
    if graph
        .canonical_resource_id()
        .map_err(|error| ExactExternalFeedOpenError::new(error.to_string()))?
        != endpoint.graph_resource_id()
        || receipts.workspace_id() != runtime.engine().workspace_id()
        || receipts.endpoint_binding() != Some(endpoint)
        || receipts.store_id() != receipt_store_id
        || runtime.engine().projection_endpoint_binding() != Some(endpoint)
    {
        return Err(ExactExternalFeedOpenError::new(
            "live Graph, runtime, engine, or receipt-store binding differs",
        ));
    }
    Ok(ProjectionStorageBinding {
        endpoint,
        receipt_store_id,
    })
}

#[cfg(test)]
thread_local! {
    static EXACT_FEED_AFTER_TERMINAL_BEFORE_ACK_HOOK:
        std::cell::RefCell<Option<Box<dyn FnOnce() -> std::io::Result<()>>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
fn exact_feed_after_terminal_before_ack_hook() -> std::io::Result<()> {
    EXACT_FEED_AFTER_TERMINAL_BEFORE_ACK_HOOK
        .with(|hook| hook.borrow_mut().take().map_or(Ok(()), |hook| hook()))
}

#[cfg(not(test))]
fn exact_feed_after_terminal_before_ack_hook() -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};

    use uuid::Uuid;

    use super::*;
    use crate::oplog::enrollment::{
        compose_verified_local, enrollment_application_root_for_test, EnrollmentApplicationRoot,
        EnrollmentBindingV1, EnrollmentDiscoveryHandoff, PreparationId, VerifiedLocalEvidence,
        VerifiedLocalProofSet,
    };
    use crate::oplog::import::{
        prepare_inactive_bootstrap_import, publish_install_verify_inactive_bootstrap,
        reopen_inactive_bootstrap_accepted_authority, InactiveBootstrapAcceptedAuthority,
        InactiveBootstrapPreparedPublication, InactiveBootstrapVerifiedPublication,
    };
    use crate::oplog::local_active::{
        activate_verified_local, reopen_promoted_local_runtime, seal_local_runtime_promotion,
        take_over_promoted_local_runtime, InactiveBootstrapRuntimeSession, LocalActiveRuntime,
        PromotedRuntimeOpen, RuntimeRecoveryState, SafeHandoffUnavailable,
    };
    use crate::oplog::migration_backup::{
        verify_migration_source_backup, MigrationBackupRoot, VerifiedSourceBackup,
    };
    use crate::oplog::operational_coordinator::{
        LocalMutationCoordinatorState, OperationalCoordinator,
    };
    use crate::oplog::reconciliation_baseline::{
        ReconciliationBaselineBinding, TrustedPrivateApplicationRuntimeRoot,
    };
    use crate::oplog::shadow_projection::{
        verify_inactive_bootstrap_shadow_projection, VerifiedShadowProjection,
    };
    use crate::oplog::watcher_queue::WatcherQuiesceError;
    use crate::oplog::{
        ApplicationRuntimeRoot, BlockId, BlockLocation, CanonicalArchiveResourceId, DeviceId,
        DocumentId, LineageDigest, LogicalPageName, ManagedTextKind, ObjectStore, OpenProjection,
        OperationTransaction, PageId, ProjectionEndpointBinding, ProjectionEndpointId,
        ReferenceCatalogPolicyV1, SemanticOperation, SessionId, WorkspaceId,
    };

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "tine-exact-external-feed-{label}-{}",
                Uuid::new_v4()
            ));
            fs::create_dir(&path).unwrap();
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

    /// One real inactive bootstrap and enrollment. The owner tests intentionally
    /// pay this setup cost: fake authorities cannot prove the production seam
    /// owns the sole promoted SQLite applier and admitted coordinator path.
    struct Fixture {
        root: TestRoot,
        graph_root: PathBuf,
        graph: Graph,
        receipts: ProjectionReceiptStore,
        archive_root: PathBuf,
        workspace: WorkspaceId,
        prepared: InactiveBootstrapPreparedPublication,
        verified: InactiveBootstrapVerifiedPublication,
        accepted: InactiveBootstrapAcceptedAuthority,
        backup_roots: MigrationBackupRoot,
        backup: VerifiedSourceBackup,
        bootstrap: Option<InactiveBootstrapRuntimeSession>,
        archive_resource: CanonicalArchiveResourceId,
        shadow: VerifiedShadowProjection,
        preparation: PreparationId,
    }

    impl Fixture {
        fn new(
            label: &str,
            config: Option<&[u8]>,
            files: impl IntoIterator<Item = (String, Vec<u8>)>,
        ) -> Self {
            let root = TestRoot::new(label);
            let graph_root = root.path().join("graph");
            fs::create_dir(&graph_root).unwrap();
            if let Some(config) = config {
                fs::create_dir(graph_root.join("logseq")).unwrap();
                fs::write(graph_root.join("logseq/config.edn"), config).unwrap();
            }
            for (path, bytes) in files {
                let destination = graph_root.join(path);
                fs::create_dir_all(destination.parent().unwrap()).unwrap();
                fs::write(destination, bytes).unwrap();
            }
            let graph = Graph::open(&graph_root);
            let workspace = WorkspaceId::from_uuid(Uuid::new_v4());
            let lineage = LineageDigest::of(format!("exact-feed-{label}").as_bytes());
            let catalog_document_id = DocumentId::from_uuid(Uuid::new_v4());

            let receipt_root = root.path().join("receipts");
            fs::create_dir(&receipt_root).unwrap();
            let endpoint = ProjectionEndpointBinding::enroll_graph(
                &graph,
                ProjectionEndpointId::from_uuid(Uuid::new_v4()),
                DeviceId::from_uuid(Uuid::new_v4()),
            )
            .unwrap();
            let receipts =
                ProjectionReceiptStore::open_for_endpoint(&receipt_root, workspace, endpoint)
                    .unwrap();

            let capture_root = root.path().join("capture");
            let preparation_root = root.path().join("preparation");
            fs::create_dir(&capture_root).unwrap();
            fs::create_dir(&preparation_root).unwrap();
            let capture = graph
                .capture_inactive_bootstrap_sources(&capture_root)
                .unwrap();
            let archive_root = root.path().join("archive");
            let prepared = prepare_inactive_bootstrap_import(
                &graph,
                capture,
                workspace,
                lineage,
                catalog_document_id,
                ReferenceCatalogPolicyV1::default(),
                &ObjectStore::open(&archive_root, workspace)
                    .unwrap()
                    .bootstrap_authoring_capability()
                    .unwrap(),
                &preparation_root,
            )
            .unwrap();
            let storage = ProjectionStorageBinding {
                endpoint,
                receipt_store_id: receipts.store_id(),
            };
            let verified = publish_install_verify_inactive_bootstrap(
                &prepared,
                ObjectStore::open(&archive_root, workspace).unwrap(),
                storage,
            )
            .unwrap();
            let accepted = reopen_inactive_bootstrap_accepted_authority(
                &verified,
                ObjectStore::open(&archive_root, workspace).unwrap(),
            )
            .unwrap();

            let device_root = root.path().join("device");
            fs::create_dir(&device_root).unwrap();
            let backup_roots = MigrationBackupRoot::open(&device_root, &graph_root).unwrap();
            let backup =
                verify_migration_source_backup(&backup_roots, &prepared, &verified).unwrap();
            let bootstrap_runtime =
                ApplicationRuntimeRoot::open_for_test(&root.path().join("bootstrap-runtime"))
                    .unwrap();
            let bootstrap = InactiveBootstrapRuntimeSession::open(
                &archive_root,
                workspace,
                &root.path().join("bootstrap.sqlite"),
                &bootstrap_runtime,
                &accepted,
                None,
            )
            .unwrap();
            let archive_resource = accepted
                .store()
                .provision_enrolled_archive_resource_id()
                .unwrap();
            let shadow = verify_inactive_bootstrap_shadow_projection(
                &graph,
                &backup_roots,
                &prepared,
                &verified,
                &backup,
                &accepted,
                bootstrap.projection(),
                bootstrap.sqlite_proof(),
            )
            .unwrap();
            Self {
                root,
                graph_root,
                graph,
                receipts,
                archive_root,
                workspace,
                prepared,
                verified,
                accepted,
                backup_roots,
                backup,
                bootstrap: Some(bootstrap),
                archive_resource,
                shadow,
                preparation: PreparationId::new(),
            }
        }

        fn bootstrap(&self) -> &InactiveBootstrapRuntimeSession {
            self.bootstrap.as_ref().unwrap()
        }

        fn sqlite(&self) -> &OpenProjection {
            self.bootstrap().projection()
        }

        fn proofs(&self) -> VerifiedLocalProofSet<'_> {
            VerifiedLocalProofSet {
                graph: &self.graph,
                roots: &self.backup_roots,
                prepared: &self.prepared,
                verified_publication: &self.verified,
                source_backup: &self.backup,
                accepted_authority: &self.accepted,
                sqlite: self.sqlite(),
                sqlite_projection: self.bootstrap().sqlite_proof(),
                shadow_projection: &self.shadow,
            }
        }

        fn inactive_runtime(&self) -> LocalActiveRuntime<'_> {
            LocalActiveRuntime {
                engine: self.accepted.accepted_engine(),
                projection: self.sqlite(),
            }
        }

        fn enrollment_binding(&self) -> EnrollmentBindingV1 {
            let accepted = self.accepted.binding();
            let storage = accepted.storage_binding();
            EnrollmentBindingV1::new(
                accepted.workspace_id(),
                accepted.lineage_digest(),
                self.verified.catalog_document_id(),
                storage.endpoint.endpoint_id(),
                storage.endpoint.device_id(),
                accepted.graph_resource(),
                storage.receipt_store_id,
                self.archive_resource,
                self.graph.graph_text_scope_binding().unwrap(),
            )
            .unwrap()
        }

        fn enrollment_root(&self, label: &str) -> EnrollmentApplicationRoot {
            enrollment_application_root_for_test(
                &self.root.path().join(format!("enrollment-{label}")),
            )
            .unwrap()
        }

        fn compose(&self, root: &EnrollmentApplicationRoot) -> VerifiedLocalEvidence {
            compose_verified_local(
                root,
                self.enrollment_binding(),
                self.preparation,
                &self.proofs(),
            )
            .unwrap()
        }

        fn take_bootstrap(&mut self) -> InactiveBootstrapRuntimeSession {
            self.bootstrap.take().unwrap()
        }

        fn baseline(&self, graph: &Graph, label: &str, existing: bool) -> ReconciliationBaseline {
            let runtime = ApplicationRuntimeRoot::open_for_test(
                &self.root.path().join(format!("baseline-runtime-{label}")),
            )
            .unwrap();
            let trusted =
                TrustedPrivateApplicationRuntimeRoot::from_application_runtime_root(&runtime);
            let binding = ReconciliationBaselineBinding::new(
                self.workspace,
                self.receipts.endpoint_binding().unwrap().endpoint_id(),
                graph.canonical_resource_id().unwrap(),
                graph.graph_text_scope_binding().unwrap(),
            )
            .unwrap();
            if existing {
                ReconciliationBaseline::open_existing(&trusted, binding).unwrap()
            } else {
                ReconciliationBaseline::create_fresh(&trusted, binding).unwrap()
            }
        }

        fn manifest_count(&self) -> usize {
            ObjectStore::open(&self.archive_root, self.workspace)
                .unwrap()
                .committed_manifests()
                .unwrap()
                .len()
        }
    }

    struct PromotedPaths {
        runtime_root: ApplicationRuntimeRoot,
        database_path: PathBuf,
    }

    impl PromotedPaths {
        fn new(fixture: &Fixture, label: &str) -> Self {
            Self {
                runtime_root: ApplicationRuntimeRoot::open_for_test(
                    &fixture
                        .root
                        .path()
                        .join(format!("promoted-runtime-{label}")),
                )
                .unwrap(),
                database_path: fixture.root.path().join(format!("promoted-{label}.sqlite")),
            }
        }

        fn open<'a>(&'a self, fixture: &'a Fixture, graph: &'a Graph) -> PromotedRuntimeOpen<'a> {
            PromotedRuntimeOpen {
                graph,
                receipts: &fixture.receipts,
                archive_root: &fixture.archive_root,
                database_path: &self.database_path,
                application_runtime_root: &self.runtime_root,
                graph_root: &fixture.graph_root,
                migration_backup_root: fixture.backup_roots.canonical_root(),
            }
        }
    }

    fn promote(
        fixture: &mut Fixture,
        enrollment_root: &EnrollmentApplicationRoot,
        session: SessionId,
        paths: &PromotedPaths,
    ) -> (LocalActiveAuthority, PromotedLocalRuntime) {
        let authority = activate_verified_local(
            enrollment_root,
            fixture.compose(enrollment_root),
            session,
            &fixture.proofs(),
            &fixture.inactive_runtime(),
        )
        .unwrap();
        let sealed = seal_local_runtime_promotion(
            &authority,
            &fixture.proofs(),
            &fixture.inactive_runtime(),
        )
        .unwrap();
        let bootstrap = fixture.take_bootstrap();
        let runtime = bootstrap
            .promote(sealed, &authority, &paths.open(fixture, &fixture.graph))
            .map_err(|refusal| refusal.into_parts().1)
            .unwrap();
        (authority, runtime)
    }

    fn promoted_safe_reopen(
        fixture: &mut Fixture,
        enrollment_root: &EnrollmentApplicationRoot,
        paths: &PromotedPaths,
    ) -> (LocalActiveAuthority, PromotedLocalRuntime) {
        let first = SessionId::new();
        {
            let (mut authority, mut runtime) = promote(fixture, enrollment_root, first, paths);
            runtime
                .quiesce_and_mark_safe_without_watcher_dependency_for_test(
                    &mut authority,
                    &fixture.graph,
                )
                .unwrap();
        }
        let second = SessionId::new();
        let (mut authority, mut runtime) = reopen_promoted_local_runtime(
            enrollment_root,
            &fixture.enrollment_binding(),
            second,
            &paths.open(fixture, &fixture.graph),
        )
        .unwrap();
        assert_eq!(runtime.recovery(), RuntimeRecoveryState::AdoptedSafeHandoff);
        assert_eq!(
            runtime.automatic_external_import(),
            ExternalImportAdmission::Allowed
        );
        fs::create_dir_all(fixture.graph_root.join("content/nested pages/deep")).unwrap();
        fs::create_dir_all(fixture.graph_root.join("diary/\u{65e5}\u{8a18}")).unwrap();
        // The inactive bootstrap carries no ordinary projection completions.
        // Real actor-authored mutations install authenticated expected-path
        // authority for the nested/nonstandard paths before exact-feed tests
        // exercise startup/full-scan behavior.
        let mut seed_operations = Vec::new();
        for (seed, path, name, kind, content) in [
            (
                0xEFA0_0000,
                "content/nested pages/deep/Caf\u{e9} note.md",
                "Café note",
                ManagedTextKind::Page,
                "markdown original",
            ),
            (
                0xEFA0_0100,
                "diary/\u{65e5}\u{8a18}/journal space.org",
                "Journal space",
                ManagedTextKind::Journal,
                "org original",
            ),
            (
                0xEFA0_0200,
                "content/nested pages/rename old.org",
                "Rename old",
                ManagedTextKind::Page,
                "old",
            ),
        ] {
            seed_operations.extend(local_page_operations(seed, path, name, kind, content));
        }
        append_local_operations(fixture, &mut authority, &mut runtime, seed_operations);
        (authority, runtime)
    }

    fn local_page_operations(
        seed: u128,
        path: &str,
        name: &str,
        kind: ManagedTextKind,
        content: &str,
    ) -> Vec<SemanticOperation> {
        vec![
            SemanticOperation::CreatePage {
                page_id: PageId::from_uuid(Uuid::from_u128(seed)),
                home_document_id: DocumentId::from_uuid(Uuid::from_u128(seed + 1)),
                name: LogicalPageName::parse(name).unwrap(),
                path: ManagedPath::parse(path).unwrap(),
                kind,
            },
            SemanticOperation::CreateBlock {
                block: BlockLocation {
                    block_id: BlockId::from_uuid(Uuid::from_u128(seed + 2)),
                    home_document_id: DocumentId::from_uuid(Uuid::from_u128(seed + 1)),
                },
                page_id: PageId::from_uuid(Uuid::from_u128(seed)),
                parent: None,
                order: "a".into(),
                content: content.to_owned(),
            },
        ]
    }

    fn append_local_operations(
        fixture: &Fixture,
        authority: &mut LocalActiveAuthority,
        runtime: &mut PromotedLocalRuntime,
        operations: Vec<SemanticOperation>,
    ) {
        let mut session = runtime
            .admit_promoted_mutation(authority, &fixture.graph)
            .unwrap();
        let transaction = OperationTransaction::new(operations).unwrap();
        match OperationalCoordinator::execute_local(
            &mut session,
            &fixture.graph,
            &fixture.receipts,
            &transaction,
        ) {
            LocalMutationCoordinatorState::Active(_) => {}
            LocalMutationCoordinatorState::Recovering(_) => {
                panic!("local seed unexpectedly retained recovery work")
            }
            LocalMutationCoordinatorState::Blocked(blocked) => {
                panic!("local seed was blocked: {}", blocked.failure())
            }
            LocalMutationCoordinatorState::Revoked(revoked) => {
                panic!("local seed was revoked: {}", revoked.failure())
            }
        }
    }

    fn append_local_batch(
        fixture: &Fixture,
        authority: &mut LocalActiveAuthority,
        runtime: &mut PromotedLocalRuntime,
        seed: u128,
    ) {
        append_local_operations(
            fixture,
            authority,
            runtime,
            local_page_operations(
                seed,
                &format!("content/nested pages/exact-feed-local-{seed}.md"),
                &format!("Exact Feed Local {seed}"),
                ManagedTextKind::Page,
                &format!("serialized local mutation {seed}"),
            ),
        );
    }

    fn drive_terminal(
        state: &mut ExactExternalFeedState,
        graph: &Graph,
        receipts: &ProjectionReceiptStore,
        authority: &mut LocalActiveAuthority,
        runtime: &mut PromotedLocalRuntime,
        clock: &mut u64,
    ) -> ExactExternalFeedDrain {
        for _ in 0..16 {
            *clock += 1;
            let result = state.drain_one(
                graph,
                receipts,
                authority,
                runtime,
                BaselineTimestamp::from_millis(*clock).unwrap(),
            );
            match result {
                ExactExternalFeedDrain::Recovering | ExactExternalFeedDrain::RetryFull => {}
                terminal => return terminal,
            }
        }
        panic!("exact external feed did not reach a bounded terminal actor result");
    }

    fn assert_admitted(result: ExactExternalFeedDrain) {
        assert!(
            matches!(
                result,
                ExactExternalFeedDrain::AdmittedNoop { .. }
                    | ExactExternalFeedDrain::AdmittedComplete { .. }
            ),
            "unexpected exact feed result: {result:?}"
        );
    }

    fn configured_fixture(label: &str) -> Fixture {
        configured_fixture_with_config(
            label,
            b"{:pages-directory \"content/nested pages\" :journals-directory \"diary/\xE6\x97\xA5\xE8\xA8\x98\"}\n",
        )
    }

    fn configured_fixture_with_config(label: &str, config: &[u8]) -> Fixture {
        Fixture::new(label, Some(config), [])
    }

    #[test]
    fn deferred_initial_catch_up_uses_the_sole_queue_and_one_fenced_graph_build() {
        let mut fixture = configured_fixture("sole-runtime-queue");
        let enrollment = fixture.enrollment_root("sole-runtime-queue");
        let paths = PromotedPaths::new(&fixture, "sole-runtime-queue");
        let (mut authority, mut runtime) = promoted_safe_reopen(&mut fixture, &enrollment, &paths);
        let baseline = fixture.baseline(&fixture.graph, "sole-runtime-queue", false);
        crate::model::reset_graph_text_admission_builder_counter_for_runtime_test();
        let mut state =
            ExactExternalFeedState::open(&fixture.graph, &fixture.receipts, &runtime, baseline)
                .unwrap();

        assert_eq!(
            crate::model::graph_text_admission_builder_enumerations_for_runtime_test(),
            0,
            "fast exact-feed open must only arm and retain its owed uncertainty"
        );
        assert_eq!(state.initial_build_count, 0);
        assert_eq!(state.rebase_count, 0);
        let pending = runtime.watcher_status();
        assert!(pending.pending);
        assert!(pending.pending_requires_full_scan);

        // A normalized observer may arrive after the feed was armed but before
        // the actor starts its first drain. It stays behind the same owed
        // uncertainty and is discovered by that first fenced catch-up.
        let arrived_before_catch_up = "content/nested pages/arrived before catch up.md";
        fs::write(
            fixture.graph_root.join(arrived_before_catch_up),
            b"- arrived before the first drain\n",
        )
        .unwrap();
        let manifests_before_catch_up = fixture.manifest_count();
        state
            .observe(
                &fixture.graph,
                &runtime,
                [WatcherObservation::ManagedPath(
                    ManagedPath::parse(arrived_before_catch_up).unwrap(),
                )],
            )
            .unwrap();
        let pending = runtime.watcher_status();
        let epoch = pending.latest_enqueue;
        assert!(pending.pending_requires_full_scan);
        let borrowed = runtime.begin_watcher_drain().unwrap().unwrap();
        assert_eq!(borrowed.epoch(), epoch);
        runtime.abandon_watcher_drain(borrowed.epoch()).unwrap();
        assert_eq!(
            runtime.abandon_watcher_drain(borrowed.epoch()),
            Err(WatcherSettlementError::NoDrainInFlight)
        );

        let mut clock = 0;
        assert_admitted(drive_terminal(
            &mut state,
            &fixture.graph,
            &fixture.receipts,
            &mut authority,
            &mut runtime,
            &mut clock,
        ));
        assert_eq!(fixture.manifest_count(), manifests_before_catch_up + 1);
        assert_eq!(state.initial_build_count, 1);
        assert_eq!(
            state.rebase_count, 0,
            "the first catch-up must build once at its held fence, not build then rebase"
        );
        assert!(
            crate::model::graph_text_admission_builder_enumerations_for_runtime_test() > 0,
            "the first drain, rather than open, must perform the graph-wide enumeration"
        );
        let settled = runtime.watcher_status();
        assert_eq!(settled.acknowledged, epoch);
        assert_eq!(settled.latest_enqueue, epoch);
        assert!(!settled.pending);
        assert_eq!(
            runtime.acknowledge_watcher_drain(epoch),
            Err(WatcherSettlementError::NoDrainInFlight)
        );
        assert_eq!(
            runtime.abandon_watcher_drain(epoch),
            Err(WatcherSettlementError::NoDrainInFlight)
        );

        state
            .observe(
                &fixture.graph,
                &runtime,
                [WatcherObservation::RescanRequired],
            )
            .unwrap();
        let exact_feed_intake = runtime.watcher_status();
        assert!(exact_feed_intake.pending_requires_full_scan);
        let refused_safe = runtime.quiesce_and_mark_safe(&mut authority, &fixture.graph);
        assert!(
            matches!(
                &refused_safe,
                Err(SafeHandoffUnavailable::Watcher(
                    WatcherQuiesceError::UnacknowledgedEpoch {
                        latest_enqueue,
                        acknowledged,
                    }
                )) if *latest_enqueue == exact_feed_intake.latest_enqueue
                    && *acknowledged == settled.acknowledged
            ),
            "unexpected Safe refusal: {refused_safe:?}"
        );
        assert_admitted(drive_terminal(
            &mut state,
            &fixture.graph,
            &fixture.receipts,
            &mut authority,
            &mut runtime,
            &mut clock,
        ));
        assert_eq!(
            runtime.watcher_status().acknowledged,
            exact_feed_intake.latest_enqueue
        );
        runtime
            .quiesce_and_mark_safe(&mut authority, &fixture.graph)
            .unwrap();
    }

    #[test]
    fn local_mutation_and_exact_feed_serialize_through_one_actor_pair() {
        let mut fixture = configured_fixture("serialized-local-and-feed");
        let enrollment = fixture.enrollment_root("serialized-local-and-feed");
        let paths = PromotedPaths::new(&fixture, "serialized-local-and-feed");
        let (mut authority, mut runtime) = promoted_safe_reopen(&mut fixture, &enrollment, &paths);
        let baseline = fixture.baseline(&fixture.graph, "serialized-local-and-feed", false);
        let mut state =
            ExactExternalFeedState::open(&fixture.graph, &fixture.receipts, &runtime, baseline)
                .unwrap();
        let mut clock = 0;
        assert_admitted(drive_terminal(
            &mut state,
            &fixture.graph,
            &fixture.receipts,
            &mut authority,
            &mut runtime,
            &mut clock,
        ));

        let before_local = fixture.manifest_count();
        append_local_batch(&fixture, &mut authority, &mut runtime, 0xEFA0_1000);
        assert_eq!(fixture.manifest_count(), before_local + 1);

        let local_path = format!(
            "content/nested pages/exact-feed-local-{}.md",
            0xEFA0_1000_u128
        );
        state
            .observe(
                &fixture.graph,
                &runtime,
                [WatcherObservation::ManagedPath(
                    ManagedPath::parse(&local_path).unwrap(),
                )],
            )
            .unwrap();
        let external = drive_terminal(
            &mut state,
            &fixture.graph,
            &fixture.receipts,
            &mut authority,
            &mut runtime,
            &mut clock,
        );
        assert!(
            matches!(external, ExactExternalFeedDrain::AdmittedNoop { .. }),
            "unexpected post-local exact-feed result: {external:?}"
        );
        assert_eq!(fixture.manifest_count(), before_local + 1);
        let status = runtime.watcher_status();
        assert_eq!(status.acknowledged, status.latest_enqueue);
    }

    #[test]
    fn exact_markdown_org_delete_and_both_rename_orders_admit_once_and_ack_terminally() {
        for (label, rename_order) in [
            ("rename-old-first", [0_usize, 1_usize]),
            ("rename-new-first", [1_usize, 0_usize]),
        ] {
            let mut fixture = configured_fixture(label);
            let enrollment = fixture.enrollment_root(label);
            let paths = PromotedPaths::new(&fixture, label);
            let (mut authority, mut runtime) =
                promoted_safe_reopen(&mut fixture, &enrollment, &paths);
            let baseline = fixture.baseline(&fixture.graph, label, false);
            let mut owner =
                ExactExternalFeedState::open(&fixture.graph, &fixture.receipts, &runtime, baseline)
                    .unwrap();
            let mut clock = 0;
            assert_admitted(drive_terminal(
                &mut owner,
                &fixture.graph,
                &fixture.receipts,
                &mut authority,
                &mut runtime,
                &mut clock,
            ));

            let markdown = "content/nested pages/deep/Caf\u{e9} note.md";
            let org = "diary/\u{65e5}\u{8a18}/journal space.org";
            let old = "content/nested pages/rename old.org";
            let new = "content/nested pages/deeper/renamed \u{65e5}.org";
            fs::write(
                fixture.graph_root.join(markdown),
                b"- markdown exact edit\n",
            )
            .unwrap();
            fs::write(
                fixture.graph_root.join(org),
                b"#+title: Journal\n* org exact edit\n",
            )
            .unwrap();
            fs::create_dir_all(fixture.graph_root.join("content/nested pages/deeper")).unwrap();
            let renamed_bytes = fs::read(fixture.graph_root.join(old)).unwrap();
            fs::rename(fixture.graph_root.join(old), fixture.graph_root.join(new)).unwrap();
            let rename = [
                WatcherObservation::ManagedPath(ManagedPath::parse(old).unwrap()),
                WatcherObservation::ManagedPath(ManagedPath::parse(new).unwrap()),
            ];
            let observations = [
                WatcherObservation::ManagedPath(ManagedPath::parse(markdown).unwrap()),
                WatcherObservation::ManagedPath(ManagedPath::parse(org).unwrap()),
                rename[rename_order[0]].clone(),
                rename[rename_order[1]].clone(),
                WatcherObservation::UnknownPath,
            ];
            let before = fixture.manifest_count();
            owner
                .observe(&fixture.graph, &runtime, observations)
                .unwrap();
            let result = drive_terminal(
                &mut owner,
                &fixture.graph,
                &fixture.receipts,
                &mut authority,
                &mut runtime,
                &mut clock,
            );
            assert_admitted(result);
            assert_eq!(fixture.manifest_count(), before + 1);
            assert_eq!(
                fs::read(fixture.graph_root.join(markdown)).unwrap(),
                b"- markdown exact edit\n"
            );
            assert_eq!(
                fs::read(fixture.graph_root.join(org)).unwrap(),
                b"#+title: Journal\n* org exact edit\n"
            );
            assert!(!fixture.graph_root.join(old).exists());
            assert_eq!(
                fs::read(fixture.graph_root.join(new)).unwrap(),
                renamed_bytes
            );

            fs::remove_file(fixture.graph_root.join(org)).unwrap();
            let before_delete = fixture.manifest_count();
            owner
                .observe(
                    &fixture.graph,
                    &runtime,
                    [WatcherObservation::ManagedPath(
                        ManagedPath::parse(org).unwrap(),
                    )],
                )
                .unwrap();
            assert_admitted(drive_terminal(
                &mut owner,
                &fixture.graph,
                &fixture.receipts,
                &mut authority,
                &mut runtime,
                &mut clock,
            ));
            assert_eq!(fixture.manifest_count(), before_delete + 1);
            assert!(!fixture.graph_root.join(org).exists());
            let status = runtime.watcher_status();
            assert_eq!(status.acknowledged, status.latest_enqueue);
            assert!(!status.pending);
        }
    }

    #[test]
    fn every_uncertainty_and_both_exact_bounds_collapse_to_one_rebased_full_scan_epoch() {
        let mut fixture = configured_fixture("uncertainty");
        let enrollment = fixture.enrollment_root("uncertainty");
        let paths = PromotedPaths::new(&fixture, "uncertainty");
        let (mut authority, mut runtime) = promoted_safe_reopen(&mut fixture, &enrollment, &paths);
        let baseline = fixture.baseline(&fixture.graph, "uncertainty", false);
        let mut owner =
            ExactExternalFeedState::open(&fixture.graph, &fixture.receipts, &runtime, baseline)
                .unwrap();
        let mut clock = 0;
        assert_admitted(drive_terminal(
            &mut owner,
            &fixture.graph,
            &fixture.receipts,
            &mut authority,
            &mut runtime,
            &mut clock,
        ));

        for observation in [
            WatcherObservation::UnknownPath,
            WatcherObservation::NotifyError,
            WatcherObservation::RescanRequired,
        ] {
            let rebases_before = owner.rebase_count;
            owner
                .observe(&fixture.graph, &runtime, [observation])
                .unwrap();
            let status = runtime.watcher_status();
            assert!(status.pending_requires_full_scan);
            assert_admitted(drive_terminal(
                &mut owner,
                &fixture.graph,
                &fixture.receipts,
                &mut authority,
                &mut runtime,
                &mut clock,
            ));
            let settled = runtime.watcher_status();
            assert_eq!(settled.acknowledged, settled.latest_enqueue);
            assert_eq!(owner.feed_sequence, settled.acknowledged.sequence());
            assert_eq!(owner.rebase_count, rebases_before + 1);
        }

        let rebases_before_count_overflow = owner.rebase_count;
        let count_overflow = (0..=EXACT_FEED_MAXIMUM_PATHS)
            .map(|index| {
                WatcherObservation::ManagedPath(
                    ManagedPath::parse(format!("content/nested pages/count-{index}.md")).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        owner
            .observe(&fixture.graph, &runtime, count_overflow)
            .unwrap();
        assert!(runtime.watcher_status().pending_requires_full_scan);
        let count_drain = runtime.begin_watcher_drain().unwrap().unwrap();
        assert!(count_drain
            .uncertain_reasons()
            .contains(&super::super::watcher_queue::WatcherUncertainReason::PathOverflow));
        runtime.abandon_watcher_drain(count_drain.epoch()).unwrap();
        assert_admitted(drive_terminal(
            &mut owner,
            &fixture.graph,
            &fixture.receipts,
            &mut authority,
            &mut runtime,
            &mut clock,
        ));
        assert_eq!(owner.rebase_count, rebases_before_count_overflow + 1);

        let rebases_before_byte_overflow = owner.rebase_count;
        let component = "x".repeat(100);
        let byte_overflow = (0..220)
            .map(|index| {
                WatcherObservation::ManagedPath(
                    ManagedPath::parse(format!(
                        "content/nested pages/{component}/{component}/{component}/{index}.md"
                    ))
                    .unwrap(),
                )
            })
            .collect::<Vec<_>>();
        assert!(
            byte_overflow
                .iter()
                .map(|observation| match observation {
                    WatcherObservation::ManagedPath(path) => path.as_str().len(),
                    _ => 0,
                })
                .sum::<usize>()
                > EXACT_FEED_MAXIMUM_PATH_BYTES
        );
        owner
            .observe(&fixture.graph, &runtime, byte_overflow)
            .unwrap();
        assert!(runtime.watcher_status().pending_requires_full_scan);
        let byte_drain = runtime.begin_watcher_drain().unwrap().unwrap();
        assert!(byte_drain
            .uncertain_reasons()
            .contains(&super::super::watcher_queue::WatcherUncertainReason::PathOverflow));
        runtime.abandon_watcher_drain(byte_drain.epoch()).unwrap();
        assert_admitted(drive_terminal(
            &mut owner,
            &fixture.graph,
            &fixture.receipts,
            &mut authority,
            &mut runtime,
            &mut clock,
        ));
        let settled = runtime.watcher_status();
        assert_eq!(settled.acknowledged, settled.latest_enqueue);
        assert_eq!(owner.feed_sequence, settled.acknowledged.sequence());
        assert_eq!(owner.rebase_count, rebases_before_byte_overflow + 1);
    }

    #[test]
    fn unstable_full_scan_retries_behind_the_same_epoch_fence() {
        let mut fixture = configured_fixture("unstable-full-scan");
        let enrollment = fixture.enrollment_root("unstable-full-scan");
        let paths = PromotedPaths::new(&fixture, "unstable-full-scan");
        let (mut authority, mut runtime) = promoted_safe_reopen(&mut fixture, &enrollment, &paths);
        let baseline = fixture.baseline(&fixture.graph, "unstable-full-scan", false);
        let mut owner =
            ExactExternalFeedState::open(&fixture.graph, &fixture.receipts, &runtime, baseline)
                .unwrap();
        let mut clock = 0;
        assert_admitted(drive_terminal(
            &mut owner,
            &fixture.graph,
            &fixture.receipts,
            &mut authority,
            &mut runtime,
            &mut clock,
        ));

        let rebases_before = owner.rebase_count;
        owner
            .observe(
                &fixture.graph,
                &runtime,
                [WatcherObservation::RescanRequired],
            )
            .unwrap();
        let graph_root = fixture.graph_root.clone();
        owner.before_second_scan_pass = Some(Box::new(move || {
            let changed = graph_root.join("content/nested pages/raced.md");
            fs::create_dir_all(changed.parent().unwrap()).unwrap();
            fs::write(changed, b"- arrived between scan passes\n").unwrap();
        }));

        clock += 1;
        assert_eq!(
            owner.drain_one(
                &fixture.graph,
                &fixture.receipts,
                &mut authority,
                &mut runtime,
                BaselineTimestamp::from_millis(clock).unwrap(),
            ),
            ExactExternalFeedDrain::RetryFull
        );
        assert_eq!(owner.rebase_count, rebases_before + 1);
        assert_admitted(drive_terminal(
            &mut owner,
            &fixture.graph,
            &fixture.receipts,
            &mut authority,
            &mut runtime,
            &mut clock,
        ));
        assert_eq!(
            owner.rebase_count,
            rebases_before + 2,
            "an unstable full scan must refresh the exact index behind its existing queue fence"
        );
        let settled = runtime.watcher_status();
        assert_eq!(settled.acknowledged, settled.latest_enqueue);
        assert!(!settled.pending);
    }

    #[test]
    fn continuously_unstable_epoch_bounds_graph_wide_rebases_and_still_converges() {
        let mut fixture = configured_fixture("bounded-unstable-full-scan");
        let enrollment = fixture.enrollment_root("bounded-unstable-full-scan");
        let paths = PromotedPaths::new(&fixture, "bounded-unstable-full-scan");
        let (mut authority, mut runtime) = promoted_safe_reopen(&mut fixture, &enrollment, &paths);
        let baseline = fixture.baseline(&fixture.graph, "bounded-unstable-full-scan", false);
        let mut owner =
            ExactExternalFeedState::open(&fixture.graph, &fixture.receipts, &runtime, baseline)
                .unwrap();
        let mut clock = 0;
        assert_admitted(drive_terminal(
            &mut owner,
            &fixture.graph,
            &fixture.receipts,
            &mut authority,
            &mut runtime,
            &mut clock,
        ));
        let rebases_before = owner.rebase_count;
        owner
            .observe(
                &fixture.graph,
                &runtime,
                [WatcherObservation::RescanRequired],
            )
            .unwrap();

        for race in 0..4 {
            let graph_root = fixture.graph_root.clone();
            owner.before_second_scan_pass = Some(Box::new(move || {
                let changed = graph_root.join(format!("content/nested pages/raced-{race}.md"));
                fs::create_dir_all(changed.parent().unwrap()).unwrap();
                fs::write(changed, format!("- race {race}\n")).unwrap();
            }));
            clock += 1;
            let result = owner.drain_one(
                &fixture.graph,
                &fixture.receipts,
                &mut authority,
                &mut runtime,
                BaselineTimestamp::from_millis(clock).unwrap(),
            );
            if race % 2 == 0 {
                assert_eq!(result, ExactExternalFeedDrain::RetryFull);
            } else {
                assert!(matches!(
                    result,
                    ExactExternalFeedDrain::Failed(ref detail)
                        if detail.contains("retained for bounded retry")
                ));
            }
            assert_eq!(
                owner.rebase_count,
                rebases_before + race + 1,
                "one watcher retry cycle may perform only its initial and one retry rebase"
            );
        }
        let mut terminal = None;
        for _ in 0..64 {
            clock += 1;
            let result = owner.drain_one(
                &fixture.graph,
                &fixture.receipts,
                &mut authority,
                &mut runtime,
                BaselineTimestamp::from_millis(clock).unwrap(),
            );
            if !matches!(
                result,
                ExactExternalFeedDrain::Recovering | ExactExternalFeedDrain::RetryFull
            ) {
                terminal = Some(result);
                break;
            }
        }
        assert_admitted(terminal.expect("bounded unstable epoch did not converge"));
        assert_eq!(owner.rebase_count, rebases_before + 5);
        let settled = runtime.watcher_status();
        assert_eq!(settled.acknowledged, settled.latest_enqueue);
        assert!(!settled.pending);
    }

    #[test]
    fn recovery_gate_opens_only_after_the_forced_full_scan_catch_up() {
        let mut fixture = configured_fixture("recovery-gate");
        let enrollment = fixture.enrollment_root("recovery-gate");
        let paths = PromotedPaths::new(&fixture, "recovery-gate");
        let (mut authority, mut runtime) =
            promote(&mut fixture, &enrollment, SessionId::new(), &paths);
        assert_eq!(runtime.recovery(), RuntimeRecoveryState::FirstPromotion);
        assert!(matches!(
            runtime.automatic_external_import(),
            ExternalImportAdmission::Blocked(_)
        ));
        let baseline = fixture.baseline(&fixture.graph, "recovery-gate", false);
        let mut owner =
            ExactExternalFeedState::open(&fixture.graph, &fixture.receipts, &runtime, baseline)
                .unwrap();
        let status = runtime.watcher_status();
        assert_eq!(status.latest_enqueue.sequence(), 1);
        assert_eq!(status.acknowledged.sequence(), 0);
        assert!(status.pending_requires_full_scan);

        let mut clock = 0;
        assert_admitted(drive_terminal(
            &mut owner,
            &fixture.graph,
            &fixture.receipts,
            &mut authority,
            &mut runtime,
            &mut clock,
        ));
        let status = runtime.watcher_status();
        assert_eq!(status.latest_enqueue.sequence(), 1);
        assert_eq!(status.acknowledged.sequence(), 1);
        assert!(!status.pending);
        assert_eq!(
            runtime.automatic_external_import(),
            ExternalImportAdmission::Allowed
        );
        assert_eq!(owner.feed_sequence, 1);
    }

    #[test]
    fn recovery_gate_stays_blocked_after_terminal_before_ack_until_retry_acknowledges() {
        let mut fixture = configured_fixture("recovery-gate-before-ack");
        let enrollment = fixture.enrollment_root("recovery-gate-before-ack");
        let paths = PromotedPaths::new(&fixture, "recovery-gate-before-ack");
        let (mut authority, mut runtime) =
            promote(&mut fixture, &enrollment, SessionId::new(), &paths);
        assert!(matches!(
            runtime.automatic_external_import(),
            ExternalImportAdmission::Blocked(_)
        ));
        let baseline = fixture.baseline(&fixture.graph, "recovery-gate-before-ack", false);
        let mut owner =
            ExactExternalFeedState::open(&fixture.graph, &fixture.receipts, &runtime, baseline)
                .unwrap();
        let startup_epoch = owner.watcher_queue_anchor;
        let mut clock = 0;
        EXACT_FEED_AFTER_TERMINAL_BEFORE_ACK_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(|| {
                Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "injected crash after terminal reconcile before ack",
                ))
            }));
        });
        let failed = drive_terminal(
            &mut owner,
            &fixture.graph,
            &fixture.receipts,
            &mut authority,
            &mut runtime,
            &mut clock,
        );
        assert!(matches!(failed, ExactExternalFeedDrain::Failed(_)));
        let pending = runtime.watcher_status();
        assert_eq!(pending.acknowledged.sequence(), 0);
        assert_eq!(pending.latest_enqueue, startup_epoch);
        assert!(pending.pending);
        assert!(
            !owner.recovery_catch_up_complete(&runtime),
            "caught-up publication must not open recovery before its startup epoch is acknowledged"
        );
        assert!(matches!(
            runtime.automatic_external_import(),
            ExternalImportAdmission::Blocked(_)
        ));
        assert!(matches!(
            runtime.quiesce_and_mark_safe(&mut authority, &fixture.graph),
            Err(SafeHandoffUnavailable::Watcher(
                WatcherQuiesceError::UnacknowledgedEpoch { .. }
            ))
        ));

        assert_admitted(drive_terminal(
            &mut owner,
            &fixture.graph,
            &fixture.receipts,
            &mut authority,
            &mut runtime,
            &mut clock,
        ));
        let settled = runtime.watcher_status();
        assert_eq!(settled.acknowledged, startup_epoch);
        assert_eq!(settled.acknowledged, settled.latest_enqueue);
        assert!(!settled.pending);
        assert!(owner.recovery_catch_up_complete(&runtime));
        assert_eq!(
            runtime.automatic_external_import(),
            ExternalImportAdmission::Allowed
        );
        runtime
            .quiesce_and_mark_safe(&mut authority, &fixture.graph)
            .unwrap();
    }

    #[test]
    fn crash_after_terminal_reconcile_before_ack_replays_without_duplicate_semantic_admission() {
        crate::test_support::run_on_deep_stack(|| {
            let mut fixture = configured_fixture("crash-before-ack");
            let enrollment = fixture.enrollment_root("crash-before-ack");
            let binding = fixture.enrollment_binding();
            let paths = PromotedPaths::new(&fixture, "crash-before-ack");
            let (mut authority, mut runtime) =
                promoted_safe_reopen(&mut fixture, &enrollment, &paths);
            let baseline = fixture.baseline(&fixture.graph, "crash-before-ack", false);
            let mut owner =
                ExactExternalFeedState::open(&fixture.graph, &fixture.receipts, &runtime, baseline)
                    .unwrap();
            let mut clock = 0;
            assert_admitted(drive_terminal(
                &mut owner,
                &fixture.graph,
                &fixture.receipts,
                &mut authority,
                &mut runtime,
                &mut clock,
            ));

            let markdown = "content/nested pages/deep/Caf\u{e9} note.md";
            fs::write(
                fixture.graph_root.join(markdown),
                b"- admitted exactly once\n",
            )
            .unwrap();
            owner
                .observe(
                    &fixture.graph,
                    &runtime,
                    [WatcherObservation::ManagedPath(
                        ManagedPath::parse(markdown).unwrap(),
                    )],
                )
                .unwrap();
            let acknowledged_before = runtime.watcher_status().acknowledged;
            let manifests_before = fixture.manifest_count();
            EXACT_FEED_AFTER_TERMINAL_BEFORE_ACK_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(|| {
                    Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "injected crash after terminal reconcile before ack",
                    ))
                }));
            });
            let failed = drive_terminal(
                &mut owner,
                &fixture.graph,
                &fixture.receipts,
                &mut authority,
                &mut runtime,
                &mut clock,
            );
            assert!(
                matches!(failed, ExactExternalFeedDrain::Failed(_)),
                "unexpected crash-cut result: {failed:?}"
            );
            assert_eq!(runtime.watcher_status().acknowledged, acknowledged_before);
            assert!(runtime.watcher_status().pending);
            assert_eq!(fixture.manifest_count(), manifests_before + 1);
            let committed_after_crash = fixture.manifest_count();

            // Dropping the process-local owner loses its in-memory epoch. The
            // crash takeover starts recovery-gated and cannot use exact-path import
            // authority before a full recovery catch-up or a fresh Safe reopen.
            drop(owner);
            drop(runtime);
            drop(authority);
            let reopened_graph = Graph::open(&fixture.graph_root);
            let takeover_session = SessionId::new();
            let (mut takeover_authority, mut takeover_runtime) = take_over_promoted_local_runtime(
                &enrollment,
                &binding,
                takeover_session,
                &paths.open(&fixture, &reopened_graph),
            )
            .unwrap();
            assert!(matches!(
                takeover_runtime.automatic_external_import(),
                ExternalImportAdmission::Blocked(_)
            ));

            // This fixture's exact-feed owner crashed after terminal reconcile but
            // before queue acknowledgement, leaving the old graph-feed owner
            // terminal. Use the existing test-only Safe proof boundary to exercise
            // the later fresh-safe-reopen deterministic replay neighbor without
            // broadening this packet into exact-feed lease-drop recovery.
            takeover_runtime
                .quiesce_and_mark_safe_without_watcher_dependency_for_test(
                    &mut takeover_authority,
                    &reopened_graph,
                )
                .unwrap();
            drop(takeover_runtime);
            drop(takeover_authority);
            let (mut authority, mut runtime) = reopen_promoted_local_runtime(
                &enrollment,
                &binding,
                SessionId::new(),
                &paths.open(&fixture, &reopened_graph),
            )
            .unwrap();
            assert_eq!(runtime.recovery(), RuntimeRecoveryState::AdoptedSafeHandoff);
            let baseline = fixture.baseline(&reopened_graph, "crash-before-ack", true);
            let mut reopened = ExactExternalFeedState::open(
                &reopened_graph,
                &fixture.receipts,
                &runtime,
                baseline,
            )
            .unwrap();
            assert!(runtime.watcher_status().pending_requires_full_scan);
            assert_admitted(drive_terminal(
                &mut reopened,
                &reopened_graph,
                &fixture.receipts,
                &mut authority,
                &mut runtime,
                &mut clock,
            ));
            assert_eq!(
                fixture.manifest_count(),
                committed_after_crash,
                "the fresh forced scan must reuse deterministic import/receipt identity"
            );
            let status = runtime.watcher_status();
            assert_eq!(status.acknowledged, status.latest_enqueue);
        });
    }

    #[test]
    fn foreign_binding_config_mutation_and_workspace_revocation_never_ack_or_continue() {
        let mut fixture = configured_fixture("terminal-refusal");
        let enrollment = fixture.enrollment_root("terminal-refusal");
        let paths = PromotedPaths::new(&fixture, "terminal-refusal");
        let (mut authority, mut runtime) = promoted_safe_reopen(&mut fixture, &enrollment, &paths);
        let baseline = fixture.baseline(&fixture.graph, "terminal-refusal", false);
        let mut owner =
            ExactExternalFeedState::open(&fixture.graph, &fixture.receipts, &runtime, baseline)
                .unwrap();
        let mut clock = 0;
        assert_admitted(drive_terminal(
            &mut owner,
            &fixture.graph,
            &fixture.receipts,
            &mut authority,
            &mut runtime,
            &mut clock,
        ));
        let clean = runtime.watcher_status();

        let mut foreign_fixture = configured_fixture("terminal-refusal-foreign");
        let foreign_enrollment = foreign_fixture.enrollment_root("terminal-refusal-foreign");
        let foreign_paths = PromotedPaths::new(&foreign_fixture, "terminal-refusal-foreign");
        let (mut foreign_authority, mut foreign_runtime) =
            promoted_safe_reopen(&mut foreign_fixture, &foreign_enrollment, &foreign_paths);
        let foreign_clean = foreign_runtime.watcher_status();
        assert_eq!(
            owner.observe(
                &fixture.graph,
                &foreign_runtime,
                [WatcherObservation::UnknownPath]
            ),
            Err(ExactExternalFeedObserveError::ForeignActor)
        );
        assert_eq!(runtime.watcher_status(), clean);
        assert_eq!(foreign_runtime.watcher_status(), foreign_clean);
        assert_eq!(
            owner.drain_one(
                &fixture.graph,
                &fixture.receipts,
                &mut foreign_authority,
                &mut foreign_runtime,
                BaselineTimestamp::from_millis(clock + 1).unwrap(),
            ),
            ExactExternalFeedDrain::ForeignActor
        );
        assert_eq!(runtime.watcher_status(), clean);
        assert_eq!(foreign_runtime.watcher_status(), foreign_clean);

        let markdown = "content/nested pages/deep/Caf\u{e9} note.md";
        fs::write(fixture.graph_root.join(markdown), b"- revoke me\r\n").unwrap();
        owner
            .observe(
                &fixture.graph,
                &runtime,
                [WatcherObservation::ManagedPath(
                    ManagedPath::parse(markdown).unwrap(),
                )],
            )
            .unwrap();
        let owed = runtime.watcher_status();
        let lease_path = fixture
            .archive_root
            .join(".tine-runtime")
            .join("sqlite-workspaces")
            .join(fixture.workspace.to_string())
            .join("sqlite-applier.lock");
        let incoming = lease_path.with_extension("lock.incoming");
        fs::write(&incoming, b"").unwrap();
        fs::rename(&incoming, &lease_path).unwrap();

        let revoked = owner.drain_one(
            &fixture.graph,
            &fixture.receipts,
            &mut authority,
            &mut runtime,
            BaselineTimestamp::from_millis(clock + 1).unwrap(),
        );
        assert!(matches!(
            revoked,
            ExactExternalFeedDrain::Terminal(ExactExternalFeedTerminal::WorkspaceAuthorityRevoked(
                _
            ))
        ));
        let after = runtime.watcher_status();
        assert_eq!(after.acknowledged, owed.acknowledged);
        assert!(after.pending);
        assert!(owner.terminal().is_some());
        assert!(matches!(
            owner.drain_one(
                &fixture.graph,
                &fixture.receipts,
                &mut authority,
                &mut runtime,
                BaselineTimestamp::from_millis(clock + 2).unwrap(),
            ),
            ExactExternalFeedDrain::Terminal(_)
        ));
        assert_eq!(
            owner.observe(
                &fixture.graph,
                &runtime,
                [WatcherObservation::RescanRequired]
            ),
            Err(ExactExternalFeedObserveError::Terminal)
        );
    }

    #[test]
    fn configuration_observation_is_terminal_and_requires_a_fresh_graph_owner() {
        let mut fixture = configured_fixture("config-mutation");
        let enrollment = fixture.enrollment_root("config-mutation");
        let paths = PromotedPaths::new(&fixture, "config-mutation");
        let (mut authority, mut runtime) = promoted_safe_reopen(&mut fixture, &enrollment, &paths);
        let baseline = fixture.baseline(&fixture.graph, "config-mutation", false);
        let mut owner =
            ExactExternalFeedState::open(&fixture.graph, &fixture.receipts, &runtime, baseline)
                .unwrap();
        fs::write(
            fixture.graph_root.join("logseq/config.edn"),
            b"{:pages-directory \"changed-pages\"}\n",
        )
        .unwrap();
        let scope_loss = owner.drain_one(
            &fixture.graph,
            &fixture.receipts,
            &mut authority,
            &mut runtime,
            BaselineTimestamp::from_millis(1).unwrap(),
        );
        assert!(
            matches!(
                scope_loss,
                ExactExternalFeedDrain::Terminal(ExactExternalFeedTerminal::GraphFeed(_))
            ),
            "unexpected scope-loss result: {scope_loss:?}"
        );
        assert!(matches!(
            owner.terminal(),
            Some(ExactExternalFeedTerminal::GraphFeed(_))
        ));
        let terminal_status = runtime.watcher_status();
        assert_eq!(terminal_status.acknowledged.sequence(), 0);
        assert_eq!(
            owner.observe(
                &fixture.graph,
                &runtime,
                [WatcherObservation::RescanRequired],
            ),
            Err(ExactExternalFeedObserveError::Terminal)
        );
        assert!(matches!(
            owner.drain_one(
                &fixture.graph,
                &fixture.receipts,
                &mut authority,
                &mut runtime,
                BaselineTimestamp::from_millis(2).unwrap(),
            ),
            ExactExternalFeedDrain::Terminal(ExactExternalFeedTerminal::GraphFeed(_))
        ));
        assert_eq!(runtime.watcher_status(), terminal_status);
    }

    /// Existing production-authority fixture retained for the runtime-host
    /// tests. Construction uses the same inactive bootstrap, enrollment,
    /// promotion, SQLite, receipt, and Safe/takeover boundaries as this module's
    /// causal tests; no authority is manufactured for the host.
    pub(crate) struct RuntimeHostFixture {
        fixture: Fixture,
        enrollment_path: PathBuf,
        paths: PromotedPaths,
        held_owner: Option<(LocalActiveAuthority, PromotedLocalRuntime)>,
    }

    impl RuntimeHostFixture {
        pub(crate) fn safe(label: &str) -> Self {
            Self::safe_with_fixture(label, configured_fixture(label))
        }

        pub(crate) fn safe_with_config(label: &str, config: &[u8]) -> Self {
            Self::safe_with_fixture(label, configured_fixture_with_config(label, config))
        }

        fn safe_with_fixture(label: &str, mut fixture: Fixture) -> Self {
            let enrollment = fixture.enrollment_root(label);
            let enrollment_path = enrollment.path().to_path_buf();
            let paths = PromotedPaths::new(&fixture, label);
            let (mut authority, mut runtime) =
                promoted_safe_reopen(&mut fixture, &enrollment, &paths);
            runtime
                .quiesce_and_mark_safe(&mut authority, &fixture.graph)
                .unwrap();
            drop(runtime);
            drop(authority);
            create_runtime_host_baseline(&fixture, &paths);
            Self {
                fixture,
                enrollment_path,
                paths,
                held_owner: None,
            }
        }

        pub(crate) fn unsafe_held(label: &str) -> Self {
            Self::unsafe_inner(label, true)
        }

        fn unsafe_inner(label: &str, retain_owner: bool) -> Self {
            let mut fixture = configured_fixture(label);
            let enrollment = fixture.enrollment_root(label);
            let enrollment_path = enrollment.path().to_path_buf();
            let paths = PromotedPaths::new(&fixture, label);
            let owner = promote(&mut fixture, &enrollment, SessionId::new(), &paths);
            create_runtime_host_baseline(&fixture, &paths);
            Self {
                fixture,
                enrollment_path,
                paths,
                held_owner: retain_owner.then_some(owner),
            }
        }

        pub(crate) fn release_held_owner(&mut self) {
            self.held_owner.take();
        }

        pub(crate) fn request(&self) -> crate::sync_runtime::SyncRuntimeOpenRequest {
            crate::sync_runtime::SyncRuntimeOpenRequest {
                profile: crate::sync_runtime::SyncStorageProfile::ExperimentalLocal,
                graph_root: self.fixture.graph_root.clone(),
                enrollment_root: self.enrollment_path.clone(),
                archive_root: self.fixture.archive_root.clone(),
                receipt_root: self.fixture.receipts.root_path().to_path_buf(),
                database_path: self.paths.database_path.clone(),
                application_runtime_root: self.paths.runtime_root.path().to_path_buf(),
                migration_backup_root: self.fixture.backup_roots.canonical_root().to_path_buf(),
                provider_root: self.fixture.graph_root.join(".tine-sync/v2/shared"),
                provider_journal_root: self
                    .paths
                    .runtime_root
                    .path()
                    .join("provider/device/journal"),
            }
        }

        pub(crate) fn graph_root(&self) -> &Path {
            &self.fixture.graph_root
        }

        pub(crate) fn lease_path(&self) -> PathBuf {
            self.fixture
                .archive_root
                .join(".tine-runtime")
                .join("sqlite-workspaces")
                .join(self.fixture.workspace.to_string())
                .join("sqlite-applier.lock")
        }

        pub(crate) fn manifest_count(&self) -> usize {
            self.fixture.manifest_count()
        }

        pub(crate) fn applied_batch_count(&self) -> usize {
            rusqlite::Connection::open(&self.paths.database_path)
                .unwrap()
                .query_row("SELECT COUNT(*) FROM applied_batches", [], |row| row.get(0))
                .unwrap()
        }

        pub(crate) fn replay_materialized_page(
            &self,
            page_id: PageId,
        ) -> crate::oplog::MaterializedPage {
            let enrollment = enrollment_application_root_for_test(&self.enrollment_path).unwrap();
            let (mut authority, mut runtime) = reopen_promoted_local_runtime(
                &enrollment,
                &self.fixture.enrollment_binding(),
                SessionId::new(),
                &self.paths.open(&self.fixture, &self.fixture.graph),
            )
            .unwrap();
            let page = runtime.engine().materialize_page(page_id).unwrap();
            runtime
                .quiesce_and_mark_safe_without_watcher_dependency_for_test(
                    &mut authority,
                    &self.fixture.graph,
                )
                .unwrap();
            page
        }

        pub(crate) fn handoff(&self) -> EnrollmentDiscoveryHandoff {
            let graph_resource_id = self.fixture.graph.canonical_resource_id().unwrap();
            let classification = crate::oplog::discovery::discover_startup(
                &crate::oplog::discovery::DiscoveryRequest {
                    profile: crate::oplog::discovery::StartupStorageProfile::ExperimentalSparse,
                    graph_resource_id,
                    runtime_root: &self.enrollment_path,
                    archive_root: &self.fixture.archive_root,
                },
            );
            let crate::oplog::discovery::DiscoveryClassification::ExistingLocalActive(advisory) =
                classification
            else {
                panic!("runtime-host fixture no longer classifies LocalActive");
            };
            advisory.handoff
        }
    }

    fn create_runtime_host_baseline(fixture: &Fixture, paths: &PromotedPaths) {
        let trusted = TrustedPrivateApplicationRuntimeRoot::from_application_runtime_root(
            &paths.runtime_root,
        );
        let binding = ReconciliationBaselineBinding::new(
            fixture.workspace,
            fixture.receipts.endpoint_binding().unwrap().endpoint_id(),
            fixture.graph.canonical_resource_id().unwrap(),
            fixture.graph.graph_text_scope_binding().unwrap(),
        )
        .unwrap();
        ReconciliationBaseline::create_fresh(&trusted, binding).unwrap();
    }

    // The state remains move-only, while its constructor type proves the actor
    // retains ownership of the sole runtime and authority.
    #[test]
    fn state_is_move_only_and_open_borrows_the_actor_runtime() {
        trait AmbiguousIfClone<Marker> {
            fn assert_not_clone() {}
        }
        impl<T: ?Sized> AmbiguousIfClone<()> for T {}
        impl<T: ?Sized + Clone> AmbiguousIfClone<u8> for T {}

        <ExactExternalFeedState as AmbiguousIfClone<_>>::assert_not_clone();
        assert!(std::mem::needs_drop::<ExactExternalFeedState>());
        let _open: fn(
            &Graph,
            &ProjectionReceiptStore,
            &PromotedLocalRuntime,
            ReconciliationBaseline,
        ) -> Result<ExactExternalFeedState, ExactExternalFeedOpenError> =
            ExactExternalFeedState::open;
    }
}
