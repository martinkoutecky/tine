//! Admitted local semantic mutation and one-shot external reconciliation
//! through one sealed authoritative and derived-state drain.
//!
//! This remains crate-private and deliberately has no startup, enrollment, or
//! application-routing surface.

#![allow(dead_code)] // activated only by the later persisted-enrollment packet

use std::fmt;
use std::sync::Arc;

use crate::model::{HandoffSafeGuard, PublishedHandoffLatch};
use crate::Graph;

use super::enrollment::{EnrollmentError, VerifiedLocalCompositionError};
use super::hot_engine::{LocalAuthorCapture, ReconciliationNeeded};
use super::import::plan_affected_import_with_bootstrap;
use super::local_active::{
    LocalRuntimeAdmission, PromotedRuntimeSession, RuntimePromotionError, RuntimeRevocation,
    WorkspaceAuthorityBoundary, WorkspaceAuthorityRefusal,
};
#[cfg(test)]
use super::plan_affected_import;
use super::shadow_projection::BootstrapProjectionAuthority;
use super::{
    AcceptedBatchEvent, AuthorBatch, BatchDisposition, BatchId, BatchInspection, BatchOrigin,
    ContentDigest, CrdtPeerId, ImportId, ImportPlan, ImportPlanStatus, ObjectStore,
    OperationTransaction, PreparedBatch, ProjectionEndpointBinding, ProjectionReceiptStore,
    RebuildSource, SessionId, ShardedHotEngine, SqliteFrontier, TailOverlay, TailReservation,
};

const CRDT_PEER_PROBE_BUDGET: u64 = 8;
const RESUME_OPERATION_BUDGET: usize = 16;

struct ResumeBudget {
    remaining: usize,
}

impl ResumeBudget {
    fn new() -> Self {
        Self {
            remaining: RESUME_OPERATION_BUDGET,
        }
    }

    /// Charge `count` units to the phase that actually performed the work.
    ///
    /// The phase is supplied by the call site rather than assumed, so an
    /// exhaustion failure always names the drain that overran and phase
    /// assertions in regressions stay meaningful.
    fn consume(
        &mut self,
        count: usize,
        phase: OperationalPhase,
    ) -> Result<(), OperationalCoordinatorError> {
        if count > self.remaining {
            return Err(OperationalCoordinatorError::new(
                phase,
                "coordinator resume operation budget was exceeded",
            ));
        }
        self.remaining -= count;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OperationalPhase {
    Bindings,
    Planning,
    Draft,
    Capture,
    Finalize,
    TailReservation,
    Publication,
    ArchiveStage,
    TailAdmission,
    SqliteDrain,
    ProjectionDrain,
}

/// Stable post-publication evidence that retrying the exact immutable batch
/// cannot turn into progress.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RetainedBlockReason {
    Rejected(super::EngineError),
    Quarantined,
    PublishedAuthentication,
    StableBinding,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OperationalCoordinatorError {
    phase: OperationalPhase,
    detail: String,
    revocation: Option<RuntimeRevocation>,
    retained_block: Option<RetainedBlockReason>,
}

impl OperationalCoordinatorError {
    fn new(phase: OperationalPhase, detail: impl Into<String>) -> Self {
        Self {
            phase,
            detail: detail.into(),
            revocation: None,
            retained_block: None,
        }
    }

    fn revoked(phase: OperationalPhase, refusal: WorkspaceAuthorityRefusal) -> Self {
        Self {
            phase,
            detail: refusal.to_string(),
            revocation: refusal.revocation().cloned(),
            retained_block: None,
        }
    }

    fn retained_block(
        phase: OperationalPhase,
        detail: impl Into<String>,
        reason: RetainedBlockReason,
    ) -> Self {
        Self {
            phase,
            detail: detail.into(),
            revocation: None,
            retained_block: Some(reason),
        }
    }

    pub(crate) const fn phase(&self) -> OperationalPhase {
        self.phase
    }

    pub(crate) fn detail(&self) -> &str {
        &self.detail
    }

    /// Terminal workspace-authority loss, if this failure observed one.
    ///
    /// This is diagnosis only. The live admission remains the sole authority,
    /// and the runtime's own latch independently refuses every later boundary.
    pub(crate) const fn revocation(&self) -> Option<&RuntimeRevocation> {
        self.revocation.as_ref()
    }

    pub(crate) const fn retained_block_reason(&self) -> Option<&RetainedBlockReason> {
        self.retained_block.as_ref()
    }
}

impl fmt::Display for OperationalCoordinatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.phase, self.detail)
    }
}

impl std::error::Error for OperationalCoordinatorError {}

/// Re-derive archive-rooted workspace authority immediately before one
/// authority-changing boundary, and report a refusal as that exact phase.
///
/// The five side-effect call sites below are the coordinator's complete set of
/// boundaries that take, change, or externalize authority: immutable
/// publication, accepted-history archive staging, tail admission, the SQLite
/// advance, and each manifested Markdown projection step.
/// Every one of them is already an [`OperationalPhase`], so a lost workspace
/// stays diagnosable by phase rather than collapsing into one generic error.
///
/// The proof is one held-handle stat plus one no-follow resolution of the lease
/// pathname — a few per external reconciliation, and none on the keystroke
/// path. A failure latches the promoted runtime's terminal revocation, so the
/// journey cannot continue at any later boundary and no later admission,
/// window, or coordinator run can start either.
fn reprove_workspace_authority(
    admission: &LocalRuntimeAdmission<'_>,
    boundary: WorkspaceAuthorityBoundary,
    phase: OperationalPhase,
) -> Result<(), OperationalCoordinatorError> {
    admission
        .reprove_workspace_authority(boundary)
        .map_err(|refusal| OperationalCoordinatorError::revoked(phase, refusal))
}

fn authorize_coordinator(
    admission: &LocalRuntimeAdmission<'_>,
    graph: &Graph,
    engine: &ShardedHotEngine,
) -> Result<(), OperationalCoordinatorError> {
    // Preserve a typed terminal outcome when the runtime was revoked before
    // this call. `authorize` still performs the complete enrolled binding proof
    // immediately afterwards.
    reprove_workspace_authority(
        admission,
        WorkspaceAuthorityBoundary::WindowAuthorization,
        OperationalPhase::Bindings,
    )?;
    admission
        .authorize(graph, engine)
        .map_err(classify_authorization_failure)
}

fn classify_authorization_failure(error: RuntimePromotionError) -> OperationalCoordinatorError {
    match error {
        RuntimePromotionError::WorkspaceAuthorityRevoked(refusal) => {
            OperationalCoordinatorError::revoked(OperationalPhase::Bindings, refusal)
        }
        RuntimePromotionError::WorkspaceAuthorityCheckUnavailable(refusal) => {
            OperationalCoordinatorError::new(OperationalPhase::Bindings, refusal.to_string())
        }
        RuntimePromotionError::Enrollment(VerifiedLocalCompositionError::Enrollment(
            EnrollmentError::Io(detail),
        )) => OperationalCoordinatorError::new(
            OperationalPhase::Bindings,
            EnrollmentError::Io(detail).to_string(),
        ),
        RuntimePromotionError::Store(super::StoreError::Io(error)) => {
            OperationalCoordinatorError::new(OperationalPhase::Bindings, error.to_string())
        }
        stable => OperationalCoordinatorError::retained_block(
            OperationalPhase::Bindings,
            stable.to_string(),
            RetainedBlockReason::StableBinding,
        ),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OperationalCompletion {
    batch_id: BatchId,
    import_id: ImportId,
}

impl OperationalCompletion {
    pub(crate) const fn batch_id(self) -> BatchId {
        self.batch_id
    }

    pub(crate) const fn import_id(self) -> ImportId {
        self.import_id
    }
}

pub(crate) enum OperationalCoordinatorState {
    Blocked(ImportPlan),
    Noop,
    Complete(OperationalCompletion),
    FailedClosed(ExternalPublishedContinuation),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LocalMutationCompletion {
    batch_id: BatchId,
}

impl LocalMutationCompletion {
    pub(crate) const fn batch_id(self) -> BatchId {
        self.batch_id
    }
}

/// Typed recovery work retained by an admitted local semantic mutation.
pub(crate) enum LocalMutationRecovery {
    /// Exact graph bytes changed before the local draft could be sealed. The
    /// caller must reconcile these engine-derived paths and redraft.
    ReconciliationRequired(ReconciliationNeeded),
    /// Immutable publication may have happened. This exact continuation must
    /// be retried; redrafting would create a second mutation writer.
    Published(LocalPublishedContinuation),
}

impl LocalMutationRecovery {
    pub(crate) fn reconciliation_paths(&self) -> Option<&[super::ManagedPath]> {
        match self {
            Self::ReconciliationRequired(reconciliation) => Some(reconciliation.paths()),
            Self::Published(_) => None,
        }
    }

    pub(crate) fn published(&self) -> Option<&LocalPublishedContinuation> {
        match self {
            Self::ReconciliationRequired(_) => None,
            Self::Published(continuation) => Some(continuation),
        }
    }

    pub(crate) fn into_published(self) -> Option<LocalPublishedContinuation> {
        match self {
            Self::ReconciliationRequired(_) => None,
            Self::Published(continuation) => Some(continuation),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LocalMutationBlockReason {
    Prepublication,
    Retained(RetainedBlockReason),
}

/// A prepublication refusal or a stable post-publication block. The latter
/// retains the exact affine continuation and its immutable evidence.
pub(crate) struct BlockedLocalMutation {
    failure: OperationalCoordinatorError,
    reason: LocalMutationBlockReason,
    continuation: Option<LocalPublishedContinuation>,
}

impl BlockedLocalMutation {
    pub(crate) fn failure(&self) -> &OperationalCoordinatorError {
        &self.failure
    }

    pub(crate) fn reason(&self) -> &LocalMutationBlockReason {
        &self.reason
    }

    pub(crate) fn continuation(&self) -> Option<&LocalPublishedContinuation> {
        self.continuation.as_ref()
    }

    pub(crate) fn into_continuation(self) -> Option<LocalPublishedContinuation> {
        self.continuation
    }
}

/// Terminal authority loss, with the post-publication continuation retained
/// when recovery still owes derived-state drains.
pub(crate) struct RevokedLocalMutation {
    failure: OperationalCoordinatorError,
    continuation: Option<LocalPublishedContinuation>,
}

impl RevokedLocalMutation {
    pub(crate) fn failure(&self) -> &OperationalCoordinatorError {
        &self.failure
    }

    pub(crate) fn continuation(&self) -> Option<&LocalPublishedContinuation> {
        self.continuation.as_ref()
    }

    pub(crate) fn into_continuation(self) -> Option<LocalPublishedContinuation> {
        self.continuation
    }
}

/// Facade-ready result of one already-translated local semantic mutation.
///
/// The variants deliberately match the runtime states a later actor/Tauri
/// adapter needs. None carries a `LocalActive` permit or runtime admission.
pub(crate) enum LocalMutationCoordinatorState {
    Active(LocalMutationCompletion),
    Recovering(LocalMutationRecovery),
    Blocked(BlockedLocalMutation),
    Revoked(RevokedLocalMutation),
}

impl LocalMutationCoordinatorState {
    fn blocked(error: OperationalCoordinatorError) -> Self {
        if error.revocation().is_some() {
            Self::Revoked(RevokedLocalMutation {
                failure: error,
                continuation: None,
            })
        } else {
            Self::Blocked(BlockedLocalMutation {
                failure: error,
                reason: LocalMutationBlockReason::Prepublication,
                continuation: None,
            })
        }
    }

    fn from_failed(continuation: LocalPublishedContinuation) -> Self {
        if continuation.failure().revocation().is_some() {
            let failure = continuation.failure().clone();
            Self::Revoked(RevokedLocalMutation {
                failure,
                continuation: Some(continuation),
            })
        } else if let Some(reason) = continuation.failure().retained_block_reason().cloned() {
            let failure = continuation.failure().clone();
            Self::Blocked(BlockedLocalMutation {
                failure,
                reason: LocalMutationBlockReason::Retained(reason),
                continuation: Some(continuation),
            })
        } else {
            Self::Recovering(LocalMutationRecovery::Published(continuation))
        }
    }
}

/// Post-manifest retry state. It owns the original graph handoff guard and the
/// exact immutable publication identity; retry never redrafts or republishes.
struct PublishedContinuationCore {
    guard: PublishedHandoffLatch,
    endpoint: ProjectionEndpointBinding,
    archive: Arc<ObjectStore>,
    batch_id: BatchId,
    origin: BatchOrigin,
    manifest_digest: ContentDigest,
    retained_bytes: usize,
    reservation: Option<TailReservation>,
    provider_ingress: bool,
    failure: OperationalCoordinatorError,
}

impl PublishedContinuationCore {
    pub(crate) const fn batch_id(&self) -> BatchId {
        self.batch_id
    }

    pub(crate) const fn phase(&self) -> OperationalPhase {
        self.failure.phase()
    }

    pub(crate) fn failure(&self) -> &OperationalCoordinatorError {
        &self.failure
    }

    fn authorize(
        &mut self,
        admission: &LocalRuntimeAdmission<'_>,
        graph: &Graph,
        engine: &ShardedHotEngine,
    ) -> bool {
        if let Err(error) = authorize_coordinator(admission, graph, engine) {
            self.failure = error;
            false
        } else {
            true
        }
    }

    fn resume(
        &mut self,
        admission: &LocalRuntimeAdmission<'_>,
        graph: &Graph,
        receipts: &ProjectionReceiptStore,
        engine: &mut ShardedHotEngine,
        database: &mut SqliteFrontier,
        tail: &mut TailOverlay,
    ) -> Result<BatchId, OperationalCoordinatorError> {
        let mut budget = ResumeBudget::new();
        verify_bindings(graph, receipts, engine, self.endpoint, Some(&self.archive))?;
        self.guard
            .verify_binding(graph, engine.workspace_id(), self.endpoint)
            .map_err(|error| {
                OperationalCoordinatorError::retained_block(
                    OperationalPhase::Bindings,
                    error.to_string(),
                    RetainedBlockReason::StableBinding,
                )
            })?;
        authenticate_published(
            &self.archive,
            self.batch_id,
            self.origin,
            self.manifest_digest,
            self.retained_bytes,
        )?;

        // Reserve enough of the one-resume budget to authenticate/admit every
        // event accepted by this staging slice plus the exact published event
        // if it was accepted on an earlier slice.
        //
        // No unit is reserved for the projection drain. Projection work cannot
        // run ahead of SQLite catch-up, so reserving one would only move an
        // honest continuation from the projection phase to the SQLite phase
        // while pushing total work for the journey above the 16-unit target.
        let already_accepted = self.provider_ingress
            && engine
                .accepted_batch_is_active(self.batch_id)
                .map_err(|error| {
                    OperationalCoordinatorError::new(
                        OperationalPhase::ArchiveStage,
                        error.to_string(),
                    )
                })?;
        let (mut events, stage_has_more) = if already_accepted {
            (Vec::new(), false)
        } else {
            let stage_limit = budget.remaining.saturating_sub(1) / 2;
            reprove_workspace_authority(
                admission,
                WorkspaceAuthorityBoundary::ArchiveStage,
                OperationalPhase::ArchiveStage,
            )?;
            let stage = engine
                .stage_archive_batch_bounded(self.batch_id, stage_limit)
                .map_err(|error| {
                    OperationalCoordinatorError::new(
                        OperationalPhase::ArchiveStage,
                        error.to_string(),
                    )
                })?;
            budget.consume(stage.work(), OperationalPhase::ArchiveStage)?;
            fault(OperationalFaultPoint::AfterStage)?;
            require_accepted_stage_disposition(self.batch_id, &stage.outcome().disposition())?;
            let events = stage
                .outcome()
                .newly_accepted()
                .iter()
                .map(|accepted| {
                    AcceptedBatchEvent::from_accepted(engine, &self.archive, accepted.batch_id)
                        .map_err(|error| {
                            OperationalCoordinatorError::new(
                                OperationalPhase::TailAdmission,
                                error.to_string(),
                            )
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            (events, stage.has_more())
        };
        if self.reservation.is_some()
            && !events.iter().any(|event| event.batch_id() == self.batch_id)
        {
            events.push(
                AcceptedBatchEvent::from_accepted(engine, &self.archive, self.batch_id).map_err(
                    |error| {
                        OperationalCoordinatorError::new(
                            OperationalPhase::TailAdmission,
                            error.to_string(),
                        )
                    },
                )?,
            );
        }
        events.sort_unstable_by_key(AcceptedBatchEvent::acceptance_sequence);
        fault(OperationalFaultPoint::BeforeTailAdmission)?;
        reprove_workspace_authority(
            admission,
            WorkspaceAuthorityBoundary::TailAdmission,
            OperationalPhase::TailAdmission,
        )?;
        for event in events {
            budget.consume(1, OperationalPhase::TailAdmission)?;
            if event.batch_id() == self.batch_id {
                if event.retained_bytes() != self.retained_bytes {
                    return Err(OperationalCoordinatorError::retained_block(
                        OperationalPhase::TailAdmission,
                        "published accepted event retained bytes differ from the reserved prepared batch",
                        RetainedBlockReason::PublishedAuthentication,
                    ));
                }
                if let Some(reservation) = self.reservation {
                    tail.enqueue_reserved(reservation, database, engine, event)
                        .map_err(|error| {
                            OperationalCoordinatorError::new(
                                OperationalPhase::TailAdmission,
                                error.to_string(),
                            )
                        })?;
                    self.reservation = None;
                    continue;
                }
            }
            tail.try_enqueue(database, engine, &event)
                .map_err(|error| {
                    OperationalCoordinatorError::new(
                        OperationalPhase::TailAdmission,
                        error.to_string(),
                    )
                })?;
        }
        if self.reservation.is_some() {
            return Err(OperationalCoordinatorError::retained_block(
                OperationalPhase::TailAdmission,
                "published reservation survived tail admission of the published event",
                RetainedBlockReason::PublishedAuthentication,
            ));
        }
        fault(OperationalFaultPoint::AfterTailAdmission)?;
        if stage_has_more {
            return Err(OperationalCoordinatorError::new(
                OperationalPhase::ArchiveStage,
                "bounded staging slice has durable ready/fanout continuation",
            ));
        }

        reprove_workspace_authority(
            admission,
            WorkspaceAuthorityBoundary::SqliteDrain,
            OperationalPhase::SqliteDrain,
        )?;
        let source = RebuildSource::new(engine, &self.archive).map_err(|error| {
            OperationalCoordinatorError::new(OperationalPhase::SqliteDrain, error.to_string())
        })?;
        let applied = tail
            .drain_ready(database, &source, budget.remaining)
            .map_err(|error| {
                OperationalCoordinatorError::new(OperationalPhase::SqliteDrain, error.to_string())
            })?;
        budget.consume(applied, OperationalPhase::SqliteDrain)?;
        if applied > 0 {
            fault(OperationalFaultPoint::AfterSqliteApply)?;
        }
        let accepted_root = engine.accepted_frontier_root().map_err(|error| {
            OperationalCoordinatorError::new(OperationalPhase::SqliteDrain, error.to_string())
        })?;
        if database.frontier_root().map_err(|error| {
            OperationalCoordinatorError::new(OperationalPhase::SqliteDrain, error.to_string())
        })? != accepted_root
        {
            return Err(OperationalCoordinatorError::new(
                OperationalPhase::SqliteDrain,
                "SQLite bounded slice has durable accepted-sequence continuation",
            ));
        }

        loop {
            let work = {
                let page = engine
                    .projection_work_index()
                    .map_err(|error| {
                        OperationalCoordinatorError::new(
                            OperationalPhase::ProjectionDrain,
                            error.to_string(),
                        )
                    })?
                    .ready_page(None, 1)
                    .map_err(|error| {
                        OperationalCoordinatorError::new(
                            OperationalPhase::ProjectionDrain,
                            error.to_string(),
                        )
                    })?;
                page.work().first().cloned()
            };
            let Some(work) = work else {
                break;
            };
            if budget.remaining == 0 {
                return Err(OperationalCoordinatorError::new(
                    OperationalPhase::ProjectionDrain,
                    "projection bounded slice has ready-work continuation",
                ));
            }
            reprove_workspace_authority(
                admission,
                WorkspaceAuthorityBoundary::ProjectionDrain,
                OperationalPhase::ProjectionDrain,
            )?;
            fault(OperationalFaultPoint::BeforeProjection)?;
            super::projection::execute_manifested_projection_work_under_handoff(
                graph,
                receipts,
                engine,
                &work,
                &self.guard,
            )
            .map_err(|error| {
                OperationalCoordinatorError::new(
                    OperationalPhase::ProjectionDrain,
                    error.to_string(),
                )
            })?;
            budget.consume(1, OperationalPhase::ProjectionDrain)?;
            fault(OperationalFaultPoint::AfterProjection)?;
        }

        let receiver_endpoint = engine
            .projection_endpoint_binding()
            .ok_or_else(|| {
                OperationalCoordinatorError::new(
                    OperationalPhase::ProjectionDrain,
                    "provider receiver has no enrolled projection endpoint",
                )
            })?
            .endpoint_id();
        let batch = match self.archive.inspect_batch(self.batch_id).map_err(|error| {
            OperationalCoordinatorError::new(OperationalPhase::ProjectionDrain, error.to_string())
        })? {
            BatchInspection::Ready(batch) => batch,
            BatchInspection::Absent | BatchInspection::Staged { .. } => {
                return Err(OperationalCoordinatorError::new(
                    OperationalPhase::ProjectionDrain,
                    "provider ingress batch became partial before receiver projection",
                ));
            }
        };
        let projection = super::projection_manifest::validate_projection_object_set(
            batch.manifest(),
            batch.objects(),
        )
        .map_err(|error| {
            OperationalCoordinatorError::new(OperationalPhase::ProjectionDrain, error.to_string())
        })?;
        for source in projection
            .intents()
            .iter()
            .filter(|source| source.source_endpoint_id() != receiver_endpoint)
        {
            reprove_workspace_authority(
                admission,
                WorkspaceAuthorityBoundary::ProjectionDrain,
                OperationalPhase::ProjectionDrain,
            )?;
            let consumed = super::projection::execute_receiver_local_projection_under_handoff(
                graph,
                receipts,
                engine,
                source,
                &self.guard,
                budget.remaining > 0,
            )
            .map_err(|error| {
                OperationalCoordinatorError::new(
                    OperationalPhase::ProjectionDrain,
                    error.to_string(),
                )
            })?;
            let Some(consumed) = consumed else {
                return Err(OperationalCoordinatorError::new(
                    OperationalPhase::ProjectionDrain,
                    "bounded receiver-local provider projection has durable continuation",
                ));
            };
            if consumed {
                budget.consume(1, OperationalPhase::ProjectionDrain)?;
            }
        }
        Ok(self.batch_id)
    }
}

fn require_accepted_stage_disposition(
    batch_id: BatchId,
    disposition: &BatchDisposition,
) -> Result<(), OperationalCoordinatorError> {
    match disposition {
        BatchDisposition::Accepted { .. } | BatchDisposition::DuplicateAccepted { .. } => Ok(()),
        BatchDisposition::IncompleteStaged { .. } => Err(OperationalCoordinatorError::new(
            OperationalPhase::ArchiveStage,
            format!("bounded staging slice for {batch_id} retains dependency/work continuation"),
        )),
        BatchDisposition::Rejected { error } => Err(OperationalCoordinatorError::retained_block(
            OperationalPhase::ArchiveStage,
            format!("published mutation {batch_id} was rejected: {error}"),
            RetainedBlockReason::Rejected(error.clone()),
        )),
        BatchDisposition::Quarantined => Err(OperationalCoordinatorError::retained_block(
            OperationalPhase::ArchiveStage,
            format!("published mutation {batch_id} was quarantined"),
            RetainedBlockReason::Quarantined,
        )),
    }
}

/// Affine external-reconciliation continuation. Only this type exposes an
/// import identity and the external retry API.
pub(crate) struct ExternalPublishedContinuation {
    import_id: ImportId,
    core: PublishedContinuationCore,
}

/// Compatibility name for the existing external reconciliation session.
/// Local mutation continuations are a distinct type and cannot enter it.
pub(crate) type FailedClosedOperationalCoordinator = ExternalPublishedContinuation;

impl ExternalPublishedContinuation {
    pub(crate) const fn batch_id(&self) -> BatchId {
        self.core.batch_id()
    }

    pub(crate) const fn import_id(&self) -> ImportId {
        self.import_id
    }

    pub(crate) const fn phase(&self) -> OperationalPhase {
        self.core.phase()
    }

    pub(crate) fn failure(&self) -> &OperationalCoordinatorError {
        self.core.failure()
    }

    #[cfg(test)]
    const fn retained_bytes(&self) -> usize {
        self.core.retained_bytes
    }

    pub(crate) fn retry(
        mut self,
        admission: &LocalRuntimeAdmission<'_>,
        graph: &Graph,
        receipts: &ProjectionReceiptStore,
        engine: &mut ShardedHotEngine,
        database: &mut SqliteFrontier,
        tail: &mut TailOverlay,
    ) -> OperationalCoordinatorState {
        if !self.core.authorize(admission, graph, engine) {
            return OperationalCoordinatorState::FailedClosed(self);
        }
        match self
            .core
            .resume(admission, graph, receipts, engine, database, tail)
        {
            Ok(batch_id) => {
                self.core.guard.complete();
                OperationalCoordinatorState::Complete(OperationalCompletion {
                    batch_id,
                    import_id: self.import_id,
                })
            }
            Err(error) => {
                self.core.failure = error;
                OperationalCoordinatorState::FailedClosed(self)
            }
        }
    }
}

/// Affine admitted-local continuation. It has no import accessor and exposes
/// only the local retry API.
pub(crate) struct LocalPublishedContinuation {
    core: PublishedContinuationCore,
}

impl LocalPublishedContinuation {
    pub(crate) const fn batch_id(&self) -> BatchId {
        self.core.batch_id()
    }

    pub(crate) const fn phase(&self) -> OperationalPhase {
        self.core.phase()
    }

    pub(crate) fn failure(&self) -> &OperationalCoordinatorError {
        self.core.failure()
    }

    pub(crate) fn retry(
        mut self,
        admission: &LocalRuntimeAdmission<'_>,
        graph: &Graph,
        receipts: &ProjectionReceiptStore,
        engine: &mut ShardedHotEngine,
        database: &mut SqliteFrontier,
        tail: &mut TailOverlay,
    ) -> LocalMutationCoordinatorState {
        if !self.core.authorize(admission, graph, engine) {
            return LocalMutationCoordinatorState::from_failed(self);
        }
        match self
            .core
            .resume(admission, graph, receipts, engine, database, tail)
        {
            Ok(batch_id) => {
                self.core.guard.complete();
                LocalMutationCoordinatorState::Active(LocalMutationCompletion { batch_id })
            }
            Err(error) => {
                self.core.failure = error;
                LocalMutationCoordinatorState::from_failed(self)
            }
        }
    }
}

pub(crate) struct OperationalCoordinator;

pub(crate) enum ProviderArchiveIngress {
    Complete,
    Pending(ProviderArchiveContinuation),
}

pub(crate) struct ProviderArchiveContinuation {
    core: PublishedContinuationCore,
}

impl ProviderArchiveContinuation {
    pub(crate) const fn batch_id(&self) -> BatchId {
        self.core.batch_id()
    }

    pub(crate) fn failure(&self) -> &OperationalCoordinatorError {
        self.core.failure()
    }
}

impl OperationalCoordinator {
    /// Admit one immutable provider-delivered archive batch through the same
    /// promoted authority, accepted-history, SQLite, and graph-projection
    /// boundaries used by authored work.
    ///
    /// Provider transport can stage bytes, but it cannot authorize them. This
    /// method is the production bridge from exact retained bytes to the
    /// one-actor runtime. Its affine continuation retains the graph handoff
    /// across every post-manifest retry.
    pub(crate) fn ingest_archive_batch(
        session: &mut PromotedRuntimeSession<'_>,
        graph: &Graph,
        receipts: &ProjectionReceiptStore,
        batch_id: BatchId,
    ) -> Result<ProviderArchiveIngress, OperationalCoordinatorError> {
        let (admission, engine, database, tail) = session.parts().map_err(|refusal| {
            OperationalCoordinatorError::revoked(OperationalPhase::Bindings, refusal)
        })?;
        authorize_coordinator(&admission, graph, engine)?;
        let endpoint = engine.projection_endpoint_binding().ok_or_else(|| {
            OperationalCoordinatorError::new(
                OperationalPhase::Bindings,
                "engine has no enrolled projection endpoint",
            )
        })?;
        let archive = verify_bindings(graph, receipts, engine, endpoint, None)?;
        let validated = match archive.inspect_batch(batch_id).map_err(|error| {
            OperationalCoordinatorError::new(OperationalPhase::Publication, error.to_string())
        })? {
            BatchInspection::Ready(validated) => validated,
            BatchInspection::Absent | BatchInspection::Staged { .. } => {
                return Err(OperationalCoordinatorError::new(
                    OperationalPhase::Publication,
                    "provider ingress batch is not complete",
                ));
            }
        };
        let manifest_bytes = validated.manifest().encode().map_err(|error| {
            OperationalCoordinatorError::new(OperationalPhase::Publication, error.to_string())
        })?;
        let retained_bytes =
            validated
                .objects()
                .iter()
                .try_fold(manifest_bytes.len(), |total, object| {
                    object
                        .encode()
                        .map_err(|error| {
                            OperationalCoordinatorError::new(
                                OperationalPhase::Publication,
                                error.to_string(),
                            )
                        })
                        .and_then(|bytes| {
                            total.checked_add(bytes.len()).ok_or_else(|| {
                                OperationalCoordinatorError::new(
                                    OperationalPhase::Publication,
                                    "provider ingress retained-byte count overflowed",
                                )
                            })
                        })
                })?;
        let origin = validated.manifest().origin();
        let handoff = graph
            .mint_handoff_safe(engine.workspace_id(), endpoint)
            .map_err(|error| {
                OperationalCoordinatorError::new(OperationalPhase::Bindings, error.to_string())
            })?;
        let guard = handoff.into_publisher_guard();
        guard
            .verify_binding(graph, engine.workspace_id(), endpoint)
            .map_err(|error| {
                OperationalCoordinatorError::new(OperationalPhase::Bindings, error.to_string())
            })?;
        let mut core = PublishedContinuationCore {
            guard: guard.into_published_latch(),
            endpoint,
            archive,
            batch_id,
            origin,
            manifest_digest: ContentDigest::of(&manifest_bytes),
            retained_bytes,
            reservation: None,
            provider_ingress: true,
            failure: OperationalCoordinatorError::new(
                OperationalPhase::ArchiveStage,
                "provider ingress has not completed its first bounded slice",
            ),
        };
        match core.resume(&admission, graph, receipts, engine, database, tail) {
            Ok(_) => {
                core.guard.complete();
                Ok(ProviderArchiveIngress::Complete)
            }
            Err(error) => {
                core.failure = error;
                Ok(ProviderArchiveIngress::Pending(
                    ProviderArchiveContinuation { core },
                ))
            }
        }
    }

    pub(crate) fn retry_archive_batch(
        session: &mut PromotedRuntimeSession<'_>,
        graph: &Graph,
        receipts: &ProjectionReceiptStore,
        mut continuation: ProviderArchiveContinuation,
    ) -> ProviderArchiveIngress {
        let (admission, engine, database, tail) = match session.parts() {
            Ok(parts) => parts,
            Err(refusal) => {
                continuation.core.failure =
                    OperationalCoordinatorError::revoked(OperationalPhase::Bindings, refusal);
                return ProviderArchiveIngress::Pending(continuation);
            }
        };
        match continuation
            .core
            .resume(&admission, graph, receipts, engine, database, tail)
        {
            Ok(_) => {
                continuation.core.guard.complete();
                ProviderArchiveIngress::Complete
            }
            Err(error) => {
                continuation.core.failure = error;
                ProviderArchiveIngress::Pending(continuation)
            }
        }
    }

    /// Execute one bounded external reconciliation.
    ///
    /// `admission` is the new-architecture write gate: it is derived only from a
    /// live [`LocalActiveAuthority`](super::local_active::LocalActiveAuthority)
    /// permit, and it revalidates the enrolled graph/endpoint/device binding
    /// against this exact live graph and engine before any authoritative,
    /// projection, or SQLite work is admitted.
    pub(crate) fn execute(
        admission: &LocalRuntimeAdmission<'_>,
        graph: &Graph,
        receipts: &ProjectionReceiptStore,
        engine: &mut ShardedHotEngine,
        database: &mut SqliteFrontier,
        tail: &mut TailOverlay,
        requested_paths: &[&str],
    ) -> Result<OperationalCoordinatorState, OperationalCoordinatorError> {
        Self::execute_with_bootstrap(
            admission,
            graph,
            receipts,
            engine,
            database,
            tail,
            None,
            requested_paths,
        )
    }

    pub(crate) fn execute_with_bootstrap(
        admission: &LocalRuntimeAdmission<'_>,
        graph: &Graph,
        receipts: &ProjectionReceiptStore,
        engine: &mut ShardedHotEngine,
        database: &mut SqliteFrontier,
        tail: &mut TailOverlay,
        bootstrap: Option<&BootstrapProjectionAuthority>,
        requested_paths: &[&str],
    ) -> Result<OperationalCoordinatorState, OperationalCoordinatorError> {
        authorize_coordinator(admission, graph, engine)?;
        let endpoint = engine.projection_endpoint_binding().ok_or_else(|| {
            OperationalCoordinatorError::new(
                OperationalPhase::Bindings,
                "engine has no enrolled projection endpoint",
            )
        })?;
        let archive = verify_bindings(graph, receipts, engine, endpoint, None)?;
        let handoff = graph
            .mint_handoff_safe(engine.workspace_id(), endpoint)
            .map_err(|error| {
                OperationalCoordinatorError::new(OperationalPhase::Bindings, error.to_string())
            })?;
        handoff
            .verify_binding(graph, engine.workspace_id(), endpoint)
            .map_err(|error| {
                OperationalCoordinatorError::new(OperationalPhase::Bindings, error.to_string())
            })?;
        fault(OperationalFaultPoint::AfterHandoff)?;

        let plan = plan_affected_import_with_bootstrap(
            graph,
            receipts,
            engine,
            bootstrap,
            requested_paths,
        );
        fault(OperationalFaultPoint::AfterPlan)?;
        match plan.status() {
            ImportPlanStatus::Blocked => {
                handoff.cancel();
                return Ok(OperationalCoordinatorState::Blocked(plan));
            }
            ImportPlanStatus::Noop => {
                if let Some(formatting) = plan.into_formatting_material() {
                    let guard = handoff.into_publisher_guard();
                    for page in formatting.pages() {
                        if let Err(error) = super::projection::adopt_existing_projection_formatting(
                            graph,
                            receipts,
                            engine,
                            &guard,
                            page.page_id(),
                            page.bytes(),
                            page.annotations(),
                        ) {
                            return Err(OperationalCoordinatorError::new(
                                OperationalPhase::Planning,
                                format!(
                                    "formatting-only baseline adoption for {} failed: {error}",
                                    page.path()
                                ),
                            ));
                        }
                    }
                    drop(guard);
                    return Ok(OperationalCoordinatorState::Noop);
                }
                handoff.cancel();
                return Ok(OperationalCoordinatorState::Noop);
            }
            ImportPlanStatus::Reconcile => {}
        }

        let guard = handoff.into_publisher_guard();
        guard
            .verify_binding(graph, engine.workspace_id(), endpoint)
            .map_err(|error| {
                OperationalCoordinatorError::new(OperationalPhase::Bindings, error.to_string())
            })?;
        let material = plan.into_execution_material().map_err(|error| {
            OperationalCoordinatorError::new(OperationalPhase::Planning, error.to_string())
        })?;
        let import_id = material.import_id();
        if material.origin() != (BatchOrigin::ExternalReconciliation { import_id }) {
            return Err(OperationalCoordinatorError::new(
                OperationalPhase::Planning,
                "sealed execution material lost its external-import identity",
            ));
        }
        let (author, draft) =
            draft_with_bounded_peer_candidates(engine, endpoint, &material, |attempt| {
                CrdtPeerId::external_import_candidate(engine.workspace_id(), import_id, attempt)
            })?;
        fault(OperationalFaultPoint::AfterDraft)?;
        let captured = engine
            .capture_external_author_transaction(draft, graph, receipts, endpoint, bootstrap)
            .map_err(|error| {
                OperationalCoordinatorError::new(OperationalPhase::Capture, error.to_string())
            })?;
        fault(OperationalFaultPoint::AfterCapture)?;
        let prepared = engine
            .finalize_captured_author_transaction(captured, receipts)
            .map_err(|error| {
                OperationalCoordinatorError::new(OperationalPhase::Finalize, error.to_string())
            })?;
        if prepared.manifest().batch_id() != author.batch_id
            || prepared.manifest().origin() != (BatchOrigin::ExternalReconciliation { import_id })
        {
            return Err(OperationalCoordinatorError::new(
                OperationalPhase::Finalize,
                "finalized batch lost its exact external-import identity",
            ));
        }
        fault(OperationalFaultPoint::AfterFinalize)?;
        let origin = BatchOrigin::ExternalReconciliation { import_id };
        match publish_and_drain(
            admission,
            graph,
            receipts,
            engine,
            database,
            tail,
            endpoint,
            archive,
            guard,
            prepared,
            author.batch_id,
            origin,
        )? {
            PublishedPipelineState::Complete(batch_id) => Ok(
                OperationalCoordinatorState::Complete(OperationalCompletion {
                    batch_id,
                    import_id,
                }),
            ),
            PublishedPipelineState::FailedClosed(continuation) => Ok(
                OperationalCoordinatorState::FailedClosed(ExternalPublishedContinuation {
                    import_id,
                    core: continuation,
                }),
            ),
        }
    }

    /// Execute one already-translated semantic local mutation under the
    /// currently admitted `LocalActive` runtime.
    ///
    /// The caller supplies semantic operations only. The nonconstructible
    /// promoted runtime/session mints batch, device, session, and CRDT-peer
    /// identity inside this boundary.
    pub(crate) fn execute_local(
        session: &mut PromotedRuntimeSession<'_>,
        graph: &Graph,
        receipts: &ProjectionReceiptStore,
        transaction: &OperationTransaction,
    ) -> LocalMutationCoordinatorState {
        let (admission, engine, database, tail, bootstrap) = match session.parts_with_bootstrap() {
            Ok(parts) => parts,
            Err(refusal) => {
                return LocalMutationCoordinatorState::blocked(
                    OperationalCoordinatorError::revoked(OperationalPhase::Bindings, refusal),
                );
            }
        };
        match execute_local_inner(
            &admission,
            graph,
            receipts,
            engine,
            database,
            tail,
            Some(bootstrap),
            LocalDraftSource::Promoted,
            transaction,
        ) {
            Ok(state) => state,
            Err(error) => LocalMutationCoordinatorState::blocked(error),
        }
    }

    /// Resume one exact published local mutation through the same promoted
    /// session boundary as initial execution.
    ///
    /// Keeping this split here prevents actor facades from acquiring direct
    /// access to the engine, SQLite applier, tail, or runtime admission.
    pub(crate) fn retry_local(
        session: &mut PromotedRuntimeSession<'_>,
        graph: &Graph,
        receipts: &ProjectionReceiptStore,
        continuation: LocalPublishedContinuation,
    ) -> LocalMutationCoordinatorState {
        let (admission, engine, database, tail) = match session.parts() {
            Ok(parts) => parts,
            Err(refusal) => {
                let mut continuation = continuation;
                continuation.core.failure =
                    OperationalCoordinatorError::revoked(OperationalPhase::Bindings, refusal);
                return LocalMutationCoordinatorState::from_failed(continuation);
            }
        };
        continuation.retry(&admission, graph, receipts, engine, database, tail)
    }

    /// Raw-author escape hatch for deterministic pre-enrollment fixtures.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    fn execute_local_with_author(
        admission: &LocalRuntimeAdmission<'_>,
        graph: &Graph,
        receipts: &ProjectionReceiptStore,
        engine: &mut ShardedHotEngine,
        database: &mut SqliteFrontier,
        tail: &mut TailOverlay,
        author: AuthorBatch,
        transaction: &OperationTransaction,
    ) -> LocalMutationCoordinatorState {
        match execute_local_inner(
            admission,
            graph,
            receipts,
            engine,
            database,
            tail,
            None,
            LocalDraftSource::Raw(author),
            transaction,
        ) {
            Ok(state) => state,
            Err(error) => LocalMutationCoordinatorState::blocked(error),
        }
    }
}

enum LocalDraftSource {
    Promoted,
    #[cfg(test)]
    Raw(AuthorBatch),
}

#[allow(clippy::too_many_arguments)]
fn execute_local_inner(
    admission: &LocalRuntimeAdmission<'_>,
    graph: &Graph,
    receipts: &ProjectionReceiptStore,
    engine: &mut ShardedHotEngine,
    database: &mut SqliteFrontier,
    tail: &mut TailOverlay,
    bootstrap: Option<&BootstrapProjectionAuthority>,
    source: LocalDraftSource,
    transaction: &OperationTransaction,
) -> Result<LocalMutationCoordinatorState, OperationalCoordinatorError> {
    authorize_coordinator(admission, graph, engine)?;
    let endpoint = engine.projection_endpoint_binding().ok_or_else(|| {
        OperationalCoordinatorError::new(
            OperationalPhase::Bindings,
            "engine has no enrolled projection endpoint",
        )
    })?;
    let archive = verify_bindings(graph, receipts, engine, endpoint, None)?;
    let handoff = graph
        .mint_handoff_safe(engine.workspace_id(), endpoint)
        .map_err(|error| {
            OperationalCoordinatorError::new(OperationalPhase::Bindings, error.to_string())
        })?;
    handoff
        .verify_binding(graph, engine.workspace_id(), endpoint)
        .map_err(|error| {
            OperationalCoordinatorError::new(OperationalPhase::Bindings, error.to_string())
        })?;
    fault(OperationalFaultPoint::AfterHandoff)?;
    let guard = handoff.into_publisher_guard();
    guard
        .verify_binding(graph, engine.workspace_id(), endpoint)
        .map_err(|error| {
            OperationalCoordinatorError::new(OperationalPhase::Bindings, error.to_string())
        })?;

    let (batch_id, author_device_id, author_session_id, draft) = match source {
        LocalDraftSource::Promoted => {
            let authority = admission
                .mint_local_author_authority(graph, engine, endpoint)
                .map_err(classify_authorization_failure)?;
            let author_device_id = authority.device_id();
            let author_session_id = authority.session_id();
            let (batch_id, draft) = engine
                .draft_admitted_local_author_transaction(&authority, transaction)
                .map_err(|error| {
                    OperationalCoordinatorError::new(OperationalPhase::Draft, error.to_string())
                })?;
            (batch_id, author_device_id, author_session_id, draft)
        }
        #[cfg(test)]
        LocalDraftSource::Raw(author) => {
            if author.author_device_id != endpoint.device_id() {
                return Err(OperationalCoordinatorError::new(
                    OperationalPhase::Bindings,
                    "local author device does not match the admitted projection endpoint",
                ));
            }
            let draft = engine
                .draft_author_transaction(author, BatchOrigin::LocalMutation, transaction)
                .map_err(|error| {
                    OperationalCoordinatorError::new(OperationalPhase::Draft, error.to_string())
                })?;
            (
                author.batch_id,
                author.author_device_id,
                author.author_session_id,
                draft,
            )
        }
    };
    fault(OperationalFaultPoint::AfterDraft)?;
    let captured = match engine
        .capture_local_author_transaction(draft, graph, receipts, endpoint, bootstrap)
        .map_err(|error| {
            OperationalCoordinatorError::new(OperationalPhase::Capture, error.to_string())
        })? {
        LocalAuthorCapture::Captured(captured) => captured,
        LocalAuthorCapture::ReconciliationNeeded(reconciliation) => {
            drop(guard);
            return Ok(LocalMutationCoordinatorState::Recovering(
                LocalMutationRecovery::ReconciliationRequired(reconciliation),
            ));
        }
    };
    fault(OperationalFaultPoint::AfterCapture)?;
    let prepared = engine
        .finalize_captured_author_transaction(captured, receipts)
        .map_err(|error| {
            OperationalCoordinatorError::new(OperationalPhase::Finalize, error.to_string())
        })?;
    let manifest = prepared.manifest();
    if manifest.batch_id() != batch_id
        || manifest.author_device_id() != author_device_id
        || manifest.author_session_id() != author_session_id
        || manifest.origin() != BatchOrigin::LocalMutation
    {
        return Err(OperationalCoordinatorError::new(
            OperationalPhase::Finalize,
            "finalized batch lost its exact local-author identity",
        ));
    }
    fault(OperationalFaultPoint::AfterFinalize)?;

    match publish_and_drain(
        admission,
        graph,
        receipts,
        engine,
        database,
        tail,
        endpoint,
        archive,
        guard,
        prepared,
        batch_id,
        BatchOrigin::LocalMutation,
    )? {
        PublishedPipelineState::Complete(batch_id) => Ok(LocalMutationCoordinatorState::Active(
            LocalMutationCompletion { batch_id },
        )),
        PublishedPipelineState::FailedClosed(continuation) => {
            Ok(LocalMutationCoordinatorState::from_failed(
                LocalPublishedContinuation { core: continuation },
            ))
        }
    }
}

enum PublishedPipelineState {
    Complete(BatchId),
    FailedClosed(PublishedContinuationCore),
}

/// The sole terminal commit pipeline for both local semantic mutations and
/// external reconciliation. Callers may differ before finalization; every
/// durable or derived-state side effect converges here.
#[allow(clippy::too_many_arguments)]
fn publish_and_drain(
    admission: &LocalRuntimeAdmission<'_>,
    graph: &Graph,
    receipts: &ProjectionReceiptStore,
    engine: &mut ShardedHotEngine,
    database: &mut SqliteFrontier,
    tail: &mut TailOverlay,
    endpoint: ProjectionEndpointBinding,
    archive: Arc<ObjectStore>,
    guard: HandoffSafeGuard,
    prepared: PreparedBatch,
    batch_id: BatchId,
    origin: BatchOrigin,
) -> Result<PublishedPipelineState, OperationalCoordinatorError> {
    if prepared.manifest().batch_id() != batch_id || prepared.manifest().origin() != origin {
        return Err(OperationalCoordinatorError::new(
            OperationalPhase::Finalize,
            "prepared batch does not match the sealed terminal-pipeline identity",
        ));
    }
    #[cfg(test)]
    TERMINAL_PIPELINE_ORIGINS.with(|origins| origins.borrow_mut().push(origin));
    let retained_bytes = prepared.retained_bytes().map_err(|error| {
        OperationalCoordinatorError::new(OperationalPhase::TailReservation, error.to_string())
    })?;
    let reservation = tail
        .reserve_bound_mutation(database, engine, retained_bytes)
        .map_err(|error| {
            OperationalCoordinatorError::new(OperationalPhase::TailReservation, error.to_string())
        })?;
    if let Err(failure) = fault(OperationalFaultPoint::AfterReservation) {
        tail.cancel_reservation(reservation).map_err(|error| {
            OperationalCoordinatorError::new(OperationalPhase::TailReservation, error.to_string())
        })?;
        return Err(failure);
    }
    let manifest_bytes = match prepared.manifest().encode() {
        Ok(bytes) => bytes,
        Err(error) => {
            tail.cancel_reservation(reservation).map_err(|cancel| {
                OperationalCoordinatorError::new(
                    OperationalPhase::TailReservation,
                    format!("{error}; reservation cancellation failed: {cancel}"),
                )
            })?;
            return Err(OperationalCoordinatorError::new(
                OperationalPhase::Publication,
                error.to_string(),
            ));
        }
    };
    let manifest_digest = ContentDigest::of(&manifest_bytes);

    // Publication is the first irreversible step. The reservation is still
    // cancellable and the publisher guard has not yet been consumed.
    if let Err(failure) = reprove_workspace_authority(
        admission,
        WorkspaceAuthorityBoundary::Publication,
        OperationalPhase::Publication,
    ) {
        drop(guard);
        tail.cancel_reservation(reservation).map_err(|cancel| {
            OperationalCoordinatorError::new(
                OperationalPhase::TailReservation,
                format!("{failure}; reservation cancellation failed: {cancel}"),
            )
        })?;
        return Err(failure);
    }
    let published_latch = guard.into_published_latch();

    if let Err(error) = archive.publish_prepared(&prepared) {
        let publication = archive.inspect_batch(batch_id);
        if matches!(publication, Ok(BatchInspection::Absent)) {
            published_latch.cancel_prepublication();
            tail.cancel_reservation(reservation).map_err(|cancel| {
                OperationalCoordinatorError::new(
                    OperationalPhase::TailReservation,
                    format!("{error}; reservation cancellation failed: {cancel}"),
                )
            })?;
            return Err(OperationalCoordinatorError::new(
                OperationalPhase::Publication,
                error.to_string(),
            ));
        }
        return Ok(PublishedPipelineState::FailedClosed(
            PublishedContinuationCore {
                guard: published_latch,
                endpoint,
                archive,
                batch_id,
                origin,
                manifest_digest,
                retained_bytes,
                reservation: Some(reservation),
                provider_ingress: false,
                failure: OperationalCoordinatorError::new(
                    OperationalPhase::Publication,
                    error.to_string(),
                ),
            },
        ));
    }
    let boundary = fault(OperationalFaultPoint::AfterManifest);
    let mut coordinator = PublishedContinuationCore {
        guard: published_latch,
        endpoint,
        archive,
        batch_id,
        origin,
        manifest_digest,
        retained_bytes,
        reservation: Some(reservation),
        provider_ingress: false,
        failure: boundary.clone().err().unwrap_or_else(|| {
            OperationalCoordinatorError::new(
                OperationalPhase::ArchiveStage,
                "published mutation is awaiting derived-state drains",
            )
        }),
    };
    if let Err(failure) = boundary {
        coordinator.failure = failure;
        return Ok(PublishedPipelineState::FailedClosed(coordinator));
    }
    match coordinator.resume(admission, graph, receipts, engine, database, tail) {
        Ok(batch_id) => {
            coordinator.guard.complete();
            Ok(PublishedPipelineState::Complete(batch_id))
        }
        Err(failure) => {
            coordinator.failure = failure;
            Ok(PublishedPipelineState::FailedClosed(coordinator))
        }
    }
}

fn draft_with_bounded_peer_candidates(
    engine: &ShardedHotEngine,
    endpoint: ProjectionEndpointBinding,
    material: &super::import::ImportExecutionMaterial,
    mut candidate_at: impl FnMut(u64) -> CrdtPeerId,
) -> Result<(AuthorBatch, super::AuthorTransactionDraft), OperationalCoordinatorError> {
    for attempt in 0..CRDT_PEER_PROBE_BUDGET {
        let crdt_peer_id = candidate_at(attempt);
        if crdt_peer_id.as_u64() == 0 {
            continue;
        }
        let author = AuthorBatch {
            batch_id: material.batch_id(),
            author_device_id: endpoint.device_id(),
            author_session_id: SessionId::for_external_import_author(
                engine.workspace_id(),
                material.import_id(),
            ),
            crdt_peer_id,
        };
        match engine.draft_external_import_transaction(author, material.clone()) {
            Ok(draft) => return Ok((author, draft)),
            Err(super::EngineError::CrdtPeerCollision(collision)) if collision == crdt_peer_id => {}
            Err(error) => {
                return Err(OperationalCoordinatorError::new(
                    OperationalPhase::Draft,
                    error.to_string(),
                ));
            }
        }
    }
    Err(OperationalCoordinatorError::new(
        OperationalPhase::Draft,
        format!(
            "no collision-free nonzero CRDT peer in the bounded {CRDT_PEER_PROBE_BUDGET}-candidate probe"
        ),
    ))
}

fn verify_bindings(
    graph: &Graph,
    receipts: &ProjectionReceiptStore,
    engine: &ShardedHotEngine,
    endpoint: ProjectionEndpointBinding,
    expected_archive: Option<&Arc<ObjectStore>>,
) -> Result<Arc<ObjectStore>, OperationalCoordinatorError> {
    let graph_resource = graph.canonical_resource_id().map_err(|error| {
        OperationalCoordinatorError::new(OperationalPhase::Bindings, error.to_string())
    })?;
    if endpoint.graph_resource_id() != graph_resource
        || receipts.workspace_id() != engine.workspace_id()
        || receipts.endpoint_binding() != Some(endpoint)
        || engine.projection_receipt_store_id() != Some(receipts.store_id())
    {
        return Err(OperationalCoordinatorError::retained_block(
            OperationalPhase::Bindings,
            "graph, engine endpoint, or receipt namespace binding mismatch",
            RetainedBlockReason::StableBinding,
        ));
    }
    let (archive, index) = engine.enrolled_projection_runtime().map_err(|error| {
        OperationalCoordinatorError::new(OperationalPhase::Bindings, error.to_string())
    })?;
    if index.endpoint_id() != endpoint.endpoint_id()
        || index.graph_resource_id() != endpoint.graph_resource_id()
        || index.receipt_store_id() != receipts.store_id()
    {
        return Err(OperationalCoordinatorError::retained_block(
            OperationalPhase::Bindings,
            "enrolled archive/projection runtime binding changed",
            RetainedBlockReason::StableBinding,
        ));
    }
    // The retained continuation authenticates the archive by stable workspace
    // and no-follow resource identity rather than by `Arc` pointer identity, so
    // a same-process engine reconstruction over the exact same enrolled archive
    // can resume. A substituted or copied archive directory still fails.
    if let Some(expected) = expected_archive {
        let identity = |store: &ObjectStore| {
            store.canonical_archive_identity().map_err(|error| {
                OperationalCoordinatorError::new(OperationalPhase::Bindings, error.to_string())
            })
        };
        if expected.workspace_id() != archive.workspace_id()
            || identity(expected)? != identity(&archive)?
        {
            return Err(OperationalCoordinatorError::retained_block(
                OperationalPhase::Bindings,
                "enrolled archive resource identity changed",
                RetainedBlockReason::StableBinding,
            ));
        }
    }
    Ok(archive)
}

fn authenticate_published(
    archive: &ObjectStore,
    batch_id: BatchId,
    origin: BatchOrigin,
    manifest_digest: ContentDigest,
    retained_bytes: usize,
) -> Result<(), OperationalCoordinatorError> {
    let inspection = archive
        .inspect_batch(batch_id)
        .map_err(|error| match error {
            super::StoreError::Io(error) => {
                OperationalCoordinatorError::new(OperationalPhase::Publication, error.to_string())
            }
            stable => OperationalCoordinatorError::retained_block(
                OperationalPhase::Publication,
                stable.to_string(),
                RetainedBlockReason::PublishedAuthentication,
            ),
        })?;
    let validated = match inspection {
        BatchInspection::Ready(validated) => validated,
        BatchInspection::Absent | BatchInspection::Staged { .. } => {
            return Err(OperationalCoordinatorError::retained_block(
                OperationalPhase::Publication,
                "published mutation is not a complete immutable batch",
                RetainedBlockReason::PublishedAuthentication,
            ));
        }
    };
    let encoded = validated.manifest().encode().map_err(|error| {
        OperationalCoordinatorError::retained_block(
            OperationalPhase::Publication,
            error.to_string(),
            RetainedBlockReason::PublishedAuthentication,
        )
    })?;
    if validated.manifest().batch_id() != batch_id
        || validated.manifest().origin() != origin
        || ContentDigest::of(&encoded) != manifest_digest
    {
        return Err(OperationalCoordinatorError::retained_block(
            OperationalPhase::Publication,
            "durable manifest differs from the failed-closed publication identity",
            RetainedBlockReason::PublishedAuthentication,
        ));
    }
    let actual = validated
        .objects()
        .iter()
        .try_fold(encoded.len(), |total, object| {
            object
                .encode()
                .map_err(|error| {
                    OperationalCoordinatorError::retained_block(
                        OperationalPhase::Publication,
                        error.to_string(),
                        RetainedBlockReason::PublishedAuthentication,
                    )
                })
                .and_then(|bytes| {
                    total.checked_add(bytes.len()).ok_or_else(|| {
                        OperationalCoordinatorError::retained_block(
                            OperationalPhase::Publication,
                            "durable retained-byte count overflowed",
                            RetainedBlockReason::PublishedAuthentication,
                        )
                    })
                })
        })?;
    if actual != retained_bytes {
        return Err(OperationalCoordinatorError::retained_block(
            OperationalPhase::Publication,
            "durable batch bytes differ from the reserved prepared-byte count",
            RetainedBlockReason::PublishedAuthentication,
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OperationalFaultPoint {
    AfterHandoff,
    AfterPlan,
    AfterDraft,
    AfterCapture,
    AfterFinalize,
    AfterReservation,
    AfterManifest,
    AfterStage,
    BeforeTailAdmission,
    AfterTailAdmission,
    AfterSqliteApply,
    BeforeProjection,
    AfterProjection,
}

thread_local! {
    static OPERATIONAL_FAULT: std::cell::Cell<Option<OperationalFaultPoint>> =
        const { std::cell::Cell::new(None) };
    #[cfg(test)]
    static OPERATIONAL_REPEATED_FAULT:
        std::cell::Cell<Option<(OperationalFaultPoint, u8)>> =
        const { std::cell::Cell::new(None) };
    static OPERATIONAL_ACTION: std::cell::RefCell<
        Option<(OperationalFaultPoint, Box<dyn FnOnce()>)>,
    > = std::cell::RefCell::new(None);
    #[cfg(test)]
    static TERMINAL_PIPELINE_ORIGINS: std::cell::RefCell<Vec<BatchOrigin>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
fn reset_terminal_pipeline_origins() {
    TERMINAL_PIPELINE_ORIGINS.with(|origins| origins.borrow_mut().clear());
}

#[cfg(test)]
fn terminal_pipeline_origins() -> Vec<BatchOrigin> {
    TERMINAL_PIPELINE_ORIGINS.with(|origins| origins.borrow().clone())
}

pub(crate) fn fail_once_at(point: OperationalFaultPoint) {
    OPERATIONAL_FAULT.set(Some(point));
}

#[cfg(test)]
pub(crate) fn fail_repeatedly_at(point: OperationalFaultPoint, failures: u8) {
    assert!(failures > 0, "a repeated operational fault needs work");
    OPERATIONAL_REPEATED_FAULT.set(Some((point, failures)));
}

pub(crate) fn act_once_at(point: OperationalFaultPoint, action: impl FnOnce() + 'static) {
    OPERATIONAL_ACTION.with(|slot| {
        *slot.borrow_mut() = Some((point, Box::new(action)));
    });
}

fn fault(point: OperationalFaultPoint) -> Result<(), OperationalCoordinatorError> {
    OPERATIONAL_ACTION.with(|slot| {
        let matches = slot
            .borrow()
            .as_ref()
            .is_some_and(|(scheduled, _)| *scheduled == point);
        if matches {
            let (_, action) = slot.borrow_mut().take().expect("checked action exists");
            action();
        }
    });
    #[cfg(test)]
    if let Some((scheduled, failures)) = OPERATIONAL_REPEATED_FAULT.get() {
        if scheduled == point {
            OPERATIONAL_REPEATED_FAULT.set(
                failures
                    .checked_sub(1)
                    .filter(|remaining| *remaining > 0)
                    .map(|remaining| (scheduled, remaining)),
            );
            return Err(operational_fault_error(point));
        }
    }
    if OPERATIONAL_FAULT.get() == Some(point) {
        OPERATIONAL_FAULT.set(None);
        return Err(operational_fault_error(point));
    }
    Ok(())
}

fn operational_fault_error(point: OperationalFaultPoint) -> OperationalCoordinatorError {
    OperationalCoordinatorError::new(
        match point {
            OperationalFaultPoint::AfterHandoff => OperationalPhase::Bindings,
            OperationalFaultPoint::AfterPlan => OperationalPhase::Planning,
            OperationalFaultPoint::AfterDraft => OperationalPhase::Draft,
            OperationalFaultPoint::AfterCapture => OperationalPhase::Capture,
            OperationalFaultPoint::AfterFinalize => OperationalPhase::Finalize,
            OperationalFaultPoint::AfterReservation => OperationalPhase::TailReservation,
            OperationalFaultPoint::AfterManifest => OperationalPhase::Publication,
            OperationalFaultPoint::AfterStage => OperationalPhase::ArchiveStage,
            OperationalFaultPoint::BeforeTailAdmission => OperationalPhase::TailAdmission,
            OperationalFaultPoint::AfterTailAdmission => OperationalPhase::TailAdmission,
            OperationalFaultPoint::AfterSqliteApply => OperationalPhase::SqliteDrain,
            OperationalFaultPoint::BeforeProjection | OperationalFaultPoint::AfterProjection => {
                OperationalPhase::ProjectionDrain
            }
        },
        "deterministic operational fault",
    )
}

/// Real-storage adapter used only by the deterministic scenario corpus. It
/// owns no alternate import, SQLite, or projection implementation: every
/// transition below calls the production coordinator or production recovery
/// surfaces directly. Keeping it crate-private prevents app startup from
/// gaining an experimental activation route.
pub(crate) mod simulator_harness {
    use std::collections::BTreeMap;
    use std::fs;
    use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
    use std::path::{Path, PathBuf};

    use rusqlite::{params, Connection};
    use uuid::Uuid;

    use super::{
        act_once_at, fail_once_at, ExternalPublishedContinuation, LocalRuntimeAdmission,
        OperationalCoordinator, OperationalCoordinatorState, OperationalFaultPoint,
    };
    use crate::oplog::simulator::{
        publish_bootstrap_prepared_for_simulator_fixture, CoordinatorAction,
        CoordinatorDurableBoundary, CoordinatorExpectedState, CoordinatorFailureWitness,
        CoordinatorFault, CoordinatorHandoffState, CoordinatorObservation, CoordinatorOracle,
        CoordinatorReadGate, CoordinatorRunOutcome, CoordinatorSqliteMutation, ExternalFileFixture,
        ScenarioDevice, ScenarioWorkspace, WireBytes,
    };
    use crate::oplog::{
        write_projection_exact, ApplicationRuntimeRoot, AuthorBatch, BatchDisposition, BatchId,
        BlockId, BlockLocation, ContentDigest, CrdtPeerId, DocumentId, LogicalPageName,
        ManagedPath, ObjectStore, OperationTransaction, PageId, ProjectionClaim,
        ProjectionEndpointBinding, ProjectionEndpointId, ProjectionReceiptStore, RebuildSource,
        SemanticOperation, SessionId, ShardedHotEngine, SqliteFrontier, TailOverlay,
    };
    use crate::Graph;

    pub(crate) struct CoordinatorHarness {
        graph_root: PathBuf,
        archive_root: PathBuf,
        receipt_root: PathBuf,
        database_path: PathBuf,
        runtime_path: PathBuf,
        workspace: ScenarioWorkspace,
        identity: ScenarioDevice,
        graph: Option<Graph>,
        receipts: Option<ProjectionReceiptStore>,
        archive: Option<ObjectStore>,
        engine: Option<ShardedHotEngine>,
        runtime_root: Option<ApplicationRuntimeRoot>,
        database: Option<SqliteFrontier>,
        tail: Option<TailOverlay>,
        failed: Option<ExternalPublishedContinuation>,
        crashed_observation: Option<CoordinatorObservation>,
        enrollment_pending_unprotected: bool,
        checkpoints: BTreeMap<String, CoordinatorObservation>,
        expected_failure: Option<CoordinatorExpectedState>,
        durable_boundary: CoordinatorDurableBoundary,
        last_outcome: Option<CoordinatorRunOutcome>,
    }

    impl CoordinatorHarness {
        pub(crate) fn setup(
            root: PathBuf,
            workspace: &ScenarioWorkspace,
            identity: &ScenarioDevice,
            action: &CoordinatorAction,
        ) -> Result<Self, String> {
            let CoordinatorAction::Setup {
                managed_path,
                kind,
                config_edn,
            } = action
            else {
                return Err("coordinator harness requires setup action".into());
            };
            fs::create_dir_all(&root).map_err(io)?;
            let graph_root = root.join("graph");
            fs::create_dir_all(&graph_root).map_err(io)?;
            if let Some(config) = config_edn {
                fs::create_dir_all(graph_root.join("logseq")).map_err(io)?;
                fs::write(graph_root.join("logseq/config.edn"), &config.0).map_err(io)?;
            }
            let graph = Graph::open(&graph_root);
            let endpoint = ProjectionEndpointBinding::enroll_graph(
                &graph,
                ProjectionEndpointId::from_uuid(identity.device_id.as_uuid()),
                identity.device_id,
            )
            .map_err(display)?;
            let receipt_root = root.join("receipts");
            let receipts = ProjectionReceiptStore::open_for_endpoint(
                &receipt_root,
                workspace.workspace_id,
                endpoint,
            )
            .map_err(display)?;
            let page_id = PageId::from_uuid(Uuid::from_u128(5));
            let home = DocumentId::from_uuid(Uuid::from_u128(6));
            let block = BlockId::from_uuid(Uuid::from_u128(7));
            let managed_path = ManagedPath::parse(managed_path).map_err(display)?;
            let projection_path = graph_root.join(managed_path.as_str());
            if let Some(parent) = projection_path.parent() {
                fs::create_dir_all(parent).map_err(io)?;
            }
            let transaction = OperationTransaction::new(vec![
                SemanticOperation::CreatePage {
                    page_id,
                    home_document_id: home,
                    name: LogicalPageName::parse("Coordinator Scenario Page").map_err(display)?,
                    path: managed_path,
                    kind: *kind,
                },
                SemanticOperation::CreateBlock {
                    block: BlockLocation {
                        block_id: block,
                        home_document_id: home,
                    },
                    page_id,
                    parent: None,
                    order: "a".into(),
                    content: "root".into(),
                },
                SemanticOperation::CreateBlock {
                    block: BlockLocation {
                        block_id: BlockId::from_uuid(Uuid::from_u128(8)),
                        home_document_id: home,
                    },
                    page_id,
                    parent: Some(block),
                    order: "a".into(),
                    content: "child".into(),
                },
            ])
            .map_err(display)?;
            let archive_root = root.join("archive");
            let archive =
                ObjectStore::open(&archive_root, workspace.workspace_id).map_err(display)?;
            let mut engine = ShardedHotEngine::with_enrolled_projection(
                archive,
                workspace.lineage_digest,
                workspace.catalog_document_id,
                &graph,
                &receipts,
            );
            let prepared = engine
                .prepare_bootstrap_transaction(
                    AuthorBatch {
                        batch_id: BatchId::from_uuid(Uuid::from_u128(9)),
                        author_device_id: endpoint.device_id(),
                        author_session_id: SessionId::from_uuid(Uuid::from_u128(11)),
                        crdt_peer_id: CrdtPeerId::from_u64(12),
                    },
                    &transaction,
                )
                .map_err(display)?;
            let archive =
                ObjectStore::open(&archive_root, workspace.workspace_id).map_err(display)?;
            publish_bootstrap_prepared_for_simulator_fixture(&archive, &prepared)
                .map_err(display)?;
            engine
                .stage_archive_batch(prepared.manifest().batch_id())
                .map_err(display)?;
            write_projection_exact(&graph, &receipts, &engine, page_id, None).map_err(display)?;
            let archive =
                ObjectStore::open(&archive_root, workspace.workspace_id).map_err(display)?;
            let runtime_path = root.join("runtime");
            let runtime_root =
                ApplicationRuntimeRoot::open_for_harness(&runtime_path).map_err(display)?;
            let database_path = root.join("sqlite/materialized.sqlite3");
            let source = RebuildSource::new(&engine, &archive).map_err(display)?;
            let database = SqliteFrontier::open_or_rebuild(
                &database_path,
                &runtime_root,
                ProjectionClaim::current(workspace.workspace_id, workspace.lineage_digest),
                source,
            )
            .map_err(display)?
            .database;
            let source = RebuildSource::new(&engine, &archive).map_err(display)?;
            let tail = TailOverlay::from_durable(&database, &source).map_err(display)?;
            Ok(Self {
                graph_root,
                archive_root,
                receipt_root,
                database_path,
                runtime_path,
                workspace: workspace.clone(),
                identity: identity.clone(),
                graph: Some(graph),
                receipts: Some(receipts),
                archive: Some(archive),
                engine: Some(engine),
                runtime_root: Some(runtime_root),
                database: Some(database),
                tail: Some(tail),
                failed: None,
                crashed_observation: None,
                enrollment_pending_unprotected: false,
                checkpoints: BTreeMap::new(),
                expected_failure: None,
                durable_boundary: CoordinatorDurableBoundary::Setup,
                last_outcome: None,
            })
        }

        pub(crate) fn run(&mut self, action: &CoordinatorAction) -> Result<(), String> {
            match action {
                CoordinatorAction::Setup { .. } => Err("coordinator setup ran twice".into()),
                CoordinatorAction::ExternalWrite { path, bytes_b64 } => {
                    self.external_write(path, &bytes_b64.0)
                }
                CoordinatorAction::ExternalDelete { path } => self.external_delete(path),
                CoordinatorAction::ExternalRename { from_path, to_path } => {
                    self.external_rename(from_path, to_path)
                }
                CoordinatorAction::InterfereAt {
                    point,
                    path,
                    bytes_b64,
                } => {
                    let path = self.graph_root.join(path);
                    let bytes = bytes_b64.0.clone();
                    let point = fault_point(*point)
                        .ok_or("coordinator interference requires an outer coordinator boundary")?;
                    act_once_at(point, move || {
                        if let Some(parent) = path.parent() {
                            fs::create_dir_all(parent)
                                .expect("coordinator interference parent must be writable");
                        }
                        fs::write(path, bytes)
                            .expect("coordinator interference target must be writable");
                    });
                    Ok(())
                }
                CoordinatorAction::InterfereReceiptAt { point, path } => {
                    self.interfere_receipt_at(*point, path)
                }
                CoordinatorAction::RestoreInterferedReceipt => self.restore_interfered_receipts(),
                CoordinatorAction::Execute { paths, fault } => self.execute(paths, *fault),
                CoordinatorAction::Retry { fault } => self.retry(*fault),
                CoordinatorAction::Crash => self.crash(),
                CoordinatorAction::Reopen => self.reopen(),
                CoordinatorAction::Sqlite { mutation } => self.sqlite(mutation),
                CoordinatorAction::Checkpoint { name } => {
                    let observation = self.observation()?;
                    if self.checkpoints.insert(name.clone(), observation).is_some() {
                        return Err(format!("duplicate coordinator checkpoint {name}"));
                    }
                    Ok(())
                }
                CoordinatorAction::AssertCheckpoint { name } => {
                    let expected = self
                        .checkpoints
                        .get(name)
                        .ok_or_else(|| format!("unknown coordinator checkpoint {name}"))?;
                    let observed = self.observation()?;
                    if expected == &observed {
                        Ok(())
                    } else {
                        self.expected_failure = Some(CoordinatorExpectedState::Exact(
                            CoordinatorFailureWitness::from(expected),
                        ));
                        Err(format!(
                            "coordinator checkpoint {name} changed: expected {expected:?}, observed {observed:?}"
                        ))
                    }
                }
                CoordinatorAction::AssertDurableCheckpoint { name } => {
                    let expected = self
                        .checkpoints
                        .get(name)
                        .ok_or_else(|| format!("unknown coordinator checkpoint {name}"))?;
                    let observed = self.observation()?;
                    if same_durable_evidence(expected, &observed) {
                        Ok(())
                    } else {
                        self.expected_failure = Some(CoordinatorExpectedState::Exact(
                            CoordinatorFailureWitness::from(expected),
                        ));
                        Err(format!(
                            "coordinator durable checkpoint {name} changed: expected {expected:?}, observed {observed:?}"
                        ))
                    }
                }
                CoordinatorAction::AssertAcceptedArchiveCheckpoint { name } => {
                    let expected = self
                        .checkpoints
                        .get(name)
                        .ok_or_else(|| format!("unknown coordinator checkpoint {name}"))?;
                    let observed = self.observation()?;
                    if same_accepted_archive_evidence(expected, &observed) {
                        Ok(())
                    } else {
                        self.expected_failure = Some(CoordinatorExpectedState::Exact(
                            CoordinatorFailureWitness::from(expected),
                        ));
                        Err(format!(
                            "coordinator accepted archive checkpoint {name} changed: expected {expected:?}, observed {observed:?}"
                        ))
                    }
                }
                CoordinatorAction::AssertMaterializationCheckpoint { name } => {
                    let expected = self
                        .checkpoints
                        .get(name)
                        .ok_or_else(|| format!("unknown coordinator checkpoint {name}"))?;
                    let observed = self.observation()?;
                    if same_materialization_evidence(expected, &observed) {
                        Ok(())
                    } else {
                        self.expected_failure = Some(CoordinatorExpectedState::Exact(
                            CoordinatorFailureWitness::from(expected),
                        ));
                        Err(format!(
                            "coordinator materialization checkpoint {name} changed: expected {expected:?}, observed {observed:?}"
                        ))
                    }
                }
                CoordinatorAction::Assert { oracle } => self.assert_oracle(oracle),
            }
        }

        pub(crate) fn observation(&self) -> Result<CoordinatorObservation, String> {
            let Some(engine) = self.engine.as_ref() else {
                let mut observed = self
                    .crashed_observation
                    .clone()
                    .ok_or("coordinator is crashed without a durable observation")?;
                observed.managed_files = snapshot_tree(&self.graph_root, true)?;
                observed.archive_files = snapshot_archive(&self.archive_root)?;
                observed.receipt_files = snapshot_tree(&self.receipt_root, false)?;
                observed.sqlite_files = snapshot_sqlite(&self.database_path)?;
                observed.handoff = CoordinatorHandoffState::EnrollmentPendingUnprotected;
                return Ok(observed);
            };
            let archive = self
                .archive
                .as_ref()
                .ok_or("coordinator archive handle is closed")?;
            let receipts = self
                .receipts
                .as_ref()
                .ok_or("coordinator receipt handle is closed")?;
            let accepted = engine.accepted_frontier_root().map_err(display)?;
            let accepted_frontier_digest = frontier_digest(&accepted)?;
            let source = RebuildSource::new(engine, archive).map_err(display)?;
            let mut accepted_batches =
                Vec::with_capacity(usize::try_from(accepted.acceptance_sequence()).unwrap_or(0));
            for sequence in 1..=accepted.acceptance_sequence() {
                accepted_batches.push(
                    source
                        .accepted_event_at(sequence)
                        .map_err(display)?
                        .batch_id(),
                );
            }
            let (sqlite_sequence, sqlite_frontier_digest, sqlite_row_digest, read_gate) =
                if let Some(database) = &self.database {
                    let root = database.frontier_root().map_err(display)?;
                    let frontier_matches = root == accepted;
                    let (read_gate, row_digest) =
                        if frontier_matches && database.materialized_read().is_ok() {
                            let digest = database
                                .materialized_row_digest_for_harness()
                                .map_err(display)?;
                            (CoordinatorReadGate::Open, Some(hex(digest.as_bytes())))
                        } else {
                            (CoordinatorReadGate::Closed, None)
                        };
                    (
                        Some(root.acceptance_sequence()),
                        Some(frontier_digest(&root)?),
                        row_digest,
                        read_gate,
                    )
                } else {
                    (None, None, None, CoordinatorReadGate::Closed)
                };
            let (tail_unapplied_batches, tail_retained_bytes) = self
                .tail
                .as_ref()
                .map(|tail| {
                    let status = tail.status();
                    (status.unapplied_batches, status.retained_bytes)
                })
                .unwrap_or((0, 0));
            Ok(CoordinatorObservation {
                accepted_sequence: accepted.acceptance_sequence(),
                accepted_frontier_digest,
                accepted_batches,
                sqlite_sequence,
                sqlite_frontier_digest,
                sqlite_row_digest,
                sqlite_files: snapshot_sqlite(&self.database_path)?,
                managed_files: snapshot_tree(&self.graph_root, true)?,
                archive_files: snapshot_archive(&self.archive_root)?,
                receipt_files: snapshot_tree(receipts.root_path(), false)?,
                pending_projection_work: pending_projection_work(engine)?,
                tail_unapplied_batches,
                tail_retained_bytes,
                handoff: if self.enrollment_pending_unprotected {
                    CoordinatorHandoffState::EnrollmentPendingUnprotected
                } else if self.failed.is_some() {
                    CoordinatorHandoffState::HeldFailedClosed
                } else if self.last_outcome.is_some() {
                    CoordinatorHandoffState::Released
                } else {
                    CoordinatorHandoffState::Unused
                },
                read_gate,
                durable_boundary: self.durable_boundary,
                last_outcome: self.last_outcome.clone(),
            })
        }

        fn external_write(&self, path: &str, bytes: &[u8]) -> Result<(), String> {
            let path = self.graph_root.join(path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(io)?;
            }
            fs::write(path, bytes).map_err(io)
        }

        fn external_delete(&self, path: &str) -> Result<(), String> {
            match fs::remove_file(self.graph_root.join(path)) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(io(error)),
            }
        }

        fn external_rename(&self, from_path: &str, to_path: &str) -> Result<(), String> {
            let destination = self.graph_root.join(to_path);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(io)?;
            }
            fs::rename(self.graph_root.join(from_path), destination).map_err(io)
        }

        fn restore_interfered_receipts(&self) -> Result<(), String> {
            let completions = self.receipt_root.join("completions");
            let mut entries = fs::read_dir(&completions)
                .map_err(io)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(io)?;
            entries.sort_by_key(|entry| entry.file_name());
            let mut restored = 0_usize;
            for entry in entries {
                let path = entry.path();
                let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                    return Err("receipt interference path is not UTF-8".into());
                };
                let Some(original) = name.strip_suffix(".held") else {
                    continue;
                };
                fs::rename(&path, path.with_file_name(original)).map_err(io)?;
                restored = restored.saturating_add(1);
            }
            if restored == 1 {
                Ok(())
            } else {
                Err(format!(
                    "expected one interfered completion receipt, restored {restored}"
                ))
            }
        }

        fn interfere_receipt_at(&self, fault: CoordinatorFault, path: &str) -> Result<(), String> {
            let point = fault_point(fault)
                .ok_or("receipt interference requires an outer coordinator boundary")?;
            let managed_path = ManagedPath::parse(path).map_err(display)?;
            let engine = self.engine.as_ref().ok_or("coordinator engine is closed")?;
            let index = engine.projection_work_index().map_err(display)?;
            let completions = index
                .completed_receipts_for_path(&managed_path)
                .map_err(display)?;
            let [completion_id] = completions.as_slice() else {
                return Err(format!(
                    "receipt interference for {path} requires exactly one completion, found {}",
                    completions.len()
                ));
            };
            let completion = self.receipt_root.join("completions").join(format!(
                "{}.completion",
                hex(completion_id.intent_id().as_bytes())
            ));
            act_once_at(point, move || {
                let held = completion.with_extension("completion.held");
                fs::rename(completion, held)
                    .expect("completion receipt must move at interference boundary");
            });
            Ok(())
        }

        fn execute(
            &mut self,
            paths: &[String],
            fault: Option<CoordinatorFault>,
        ) -> Result<(), String> {
            let graph = self.graph.as_ref().ok_or("coordinator graph is closed")?;
            let receipts = self
                .receipts
                .as_ref()
                .ok_or("coordinator receipts are closed")?;
            let engine = self.engine.as_mut().ok_or("coordinator engine is closed")?;
            let database = self
                .database
                .as_mut()
                .ok_or("coordinator SQLite is closed")?;
            let tail = self.tail.as_mut().ok_or("coordinator tail is closed")?;
            let requested = paths.iter().map(String::as_str).collect::<Vec<_>>();
            let _fault_scope = fault.and_then(install_fault);
            match OperationalCoordinator::execute(
                &LocalRuntimeAdmission::unenrolled_pre_activation(),
                graph,
                receipts,
                engine,
                database,
                tail,
                &requested,
            ) {
                Ok(OperationalCoordinatorState::Complete(_)) => {
                    self.last_outcome = Some(CoordinatorRunOutcome::Complete);
                    self.durable_boundary = CoordinatorDurableBoundary::Complete;
                }
                Ok(OperationalCoordinatorState::Blocked(_)) => {
                    self.last_outcome = Some(CoordinatorRunOutcome::Blocked);
                    self.durable_boundary = CoordinatorDurableBoundary::Blocked;
                }
                Ok(OperationalCoordinatorState::Noop) => {
                    self.last_outcome = Some(CoordinatorRunOutcome::Noop);
                    self.durable_boundary = CoordinatorDurableBoundary::Noop;
                }
                Ok(OperationalCoordinatorState::FailedClosed(failed)) => {
                    self.last_outcome = Some(CoordinatorRunOutcome::FailedClosed {
                        phase: format!("{:?}", failed.phase()),
                    });
                    self.failed = Some(failed);
                    if let Some(point) = fault {
                        self.durable_boundary = boundary_for_fault(point);
                    }
                }
                Err(error) => {
                    self.last_outcome = Some(CoordinatorRunOutcome::PrepublicationError {
                        phase: format!("{:?}", error.phase()),
                    });
                    if let Some(point) = fault {
                        self.durable_boundary = boundary_for_fault(point);
                    }
                }
            }
            Ok(())
        }

        fn retry(&mut self, fault: Option<CoordinatorFault>) -> Result<(), String> {
            if self.failed.is_none() && self.enrollment_pending_unprotected {
                return self.recover_reopened(fault);
            }
            let failed = self
                .failed
                .take()
                .ok_or("coordinator has no failed-closed retry")?;
            let database = self
                .database
                .as_mut()
                .ok_or("coordinator SQLite is closed")?;
            let tail = self.tail.as_mut().ok_or("coordinator tail is closed")?;
            let graph = self.graph.as_ref().ok_or("coordinator graph is closed")?;
            let receipts = self
                .receipts
                .as_ref()
                .ok_or("coordinator receipts are closed")?;
            let engine = self.engine.as_mut().ok_or("coordinator engine is closed")?;
            let _fault_scope = fault.and_then(install_fault);
            match failed.retry(
                &LocalRuntimeAdmission::unenrolled_pre_activation(),
                graph,
                receipts,
                engine,
                database,
                tail,
            ) {
                OperationalCoordinatorState::Complete(_) => {
                    self.last_outcome = Some(CoordinatorRunOutcome::Complete);
                    self.durable_boundary = CoordinatorDurableBoundary::Complete;
                }
                OperationalCoordinatorState::FailedClosed(next) => {
                    self.last_outcome = Some(CoordinatorRunOutcome::FailedClosed {
                        phase: format!("{:?}", next.phase()),
                    });
                    self.failed = Some(next);
                    if let Some(point) = fault {
                        self.durable_boundary = boundary_for_fault(point);
                    }
                }
                OperationalCoordinatorState::Blocked(_) | OperationalCoordinatorState::Noop => {
                    return Err("published coordinator retry changed to blocked/noop".into());
                }
            }
            Ok(())
        }

        fn crash(&mut self) -> Result<(), String> {
            if self.engine.is_none() {
                return Err("coordinator is already crashed".into());
            }
            let mut observation = self.observation()?;
            observation.handoff = CoordinatorHandoffState::EnrollmentPendingUnprotected;
            self.failed.take();
            self.tail.take();
            self.database.take();
            self.engine.take();
            self.archive.take();
            self.receipts.take();
            self.graph.take();
            self.runtime_root.take();
            self.enrollment_pending_unprotected = true;
            self.crashed_observation = Some(observation);
            Ok(())
        }

        fn reopen(&mut self) -> Result<(), String> {
            if self.engine.is_some()
                || self.archive.is_some()
                || self.graph.is_some()
                || self.receipts.is_some()
                || self.database.is_some()
                || self.tail.is_some()
                || self.runtime_root.is_some()
                || !self.enrollment_pending_unprotected
            {
                return Err("coordinator reopen requires a dropped crashed process".into());
            }
            let graph = Graph::open(&self.graph_root);
            let endpoint = ProjectionEndpointBinding::enroll_graph(
                &graph,
                ProjectionEndpointId::from_uuid(self.identity.device_id.as_uuid()),
                self.identity.device_id,
            )
            .map_err(display)?;
            let receipts = ProjectionReceiptStore::open_for_endpoint(
                &self.receipt_root,
                self.workspace.workspace_id,
                endpoint,
            )
            .map_err(display)?;
            let archive_audit = ObjectStore::open(&self.archive_root, self.workspace.workspace_id)
                .map_err(display)?;
            let manifests = archive_audit.committed_manifests().map_err(display)?;
            let engine_store = ObjectStore::open(&self.archive_root, self.workspace.workspace_id)
                .map_err(display)?;
            let (mut engine, _) = ShardedHotEngine::open_enrolled_projection(
                engine_store,
                self.workspace.lineage_digest,
                self.workspace.catalog_document_id,
                &graph,
                &receipts,
                &manifests,
            )
            .map_err(display)?;

            // A manifest may be durably published immediately before the
            // process dies, while its accepted-history record is necessarily
            // absent. Authenticated recovery first replays the existing
            // history; ordinary production staging then admits any complete
            // archive-only commit marker. Repeated deterministic passes cover
            // BatchId order differing from dependency/acceptance order.
            for _ in 0..=manifests.len() {
                let before = engine
                    .accepted_frontier_root()
                    .map_err(display)?
                    .acceptance_sequence();
                for manifest in &manifests {
                    if engine.accepted_batch_evidence(manifest.batch_id()).is_ok() {
                        continue;
                    }
                    let outcome = engine
                        .stage_archive_batch(manifest.batch_id())
                        .map_err(display)?;
                    if !matches!(
                        outcome.disposition(),
                        BatchDisposition::Accepted { .. }
                            | BatchDisposition::DuplicateAccepted { .. }
                            | BatchDisposition::IncompleteStaged { .. }
                    ) {
                        return Err(format!(
                            "coordinator reopen rejected durable manifest {}: {:?}",
                            manifest.batch_id(),
                            outcome.disposition()
                        ));
                    }
                }
                let after = engine
                    .accepted_frontier_root()
                    .map_err(display)?
                    .acceptance_sequence();
                if after == before {
                    break;
                }
            }
            let accepted = engine
                .accepted_frontier_root()
                .map_err(display)?
                .acceptance_sequence();
            if accepted != manifests.len() as u64 {
                return Err(format!(
                    "coordinator reopen accepted {accepted} of {} durable manifests",
                    manifests.len()
                ));
            }

            let archive = ObjectStore::open(&self.archive_root, self.workspace.workspace_id)
                .map_err(display)?;
            let runtime_root =
                ApplicationRuntimeRoot::open_for_harness(&self.runtime_path).map_err(display)?;
            let source = RebuildSource::new(&engine, &archive).map_err(display)?;
            let database = SqliteFrontier::open_or_rebuild(
                &self.database_path,
                &runtime_root,
                ProjectionClaim::current(
                    self.workspace.workspace_id,
                    self.workspace.lineage_digest,
                ),
                source,
            )
            .map_err(display)?
            .database;
            let source = RebuildSource::new(&engine, &archive).map_err(display)?;
            let tail = TailOverlay::from_durable(&database, &source).map_err(display)?;
            self.graph = Some(graph);
            self.receipts = Some(receipts);
            self.archive = Some(archive);
            self.engine = Some(engine);
            self.runtime_root = Some(runtime_root);
            self.database = Some(database);
            self.tail = Some(tail);
            self.crashed_observation = None;
            Ok(())
        }

        fn recover_reopened(&mut self, fault: Option<CoordinatorFault>) -> Result<(), String> {
            let graph = self.graph.as_ref().ok_or("coordinator graph is closed")?;
            let receipts = self
                .receipts
                .as_ref()
                .ok_or("coordinator receipts are closed")?;
            let engine = self.engine.as_mut().ok_or("coordinator engine is closed")?;
            let _fault_scope = fault.and_then(install_fault);
            loop {
                let work = engine
                    .projection_work_index()
                    .map_err(display)?
                    .ready_page(None, 1)
                    .map_err(display)?
                    .work()
                    .first()
                    .cloned();
                let Some(work) = work else {
                    break;
                };
                if let Err(error) = super::fault(OperationalFaultPoint::BeforeProjection) {
                    self.last_outcome = Some(CoordinatorRunOutcome::FailedClosed {
                        phase: format!("{:?}", error.phase()),
                    });
                    if let Some(point) = fault {
                        self.durable_boundary = boundary_for_fault(point);
                    }
                    return Ok(());
                }
                if let Err(_error) = super::super::projection::execute_manifested_projection_work(
                    graph, receipts, engine, &work,
                ) {
                    self.last_outcome = Some(CoordinatorRunOutcome::FailedClosed {
                        phase: format!("{:?}", super::OperationalPhase::ProjectionDrain),
                    });
                    if let Some(point) = fault {
                        self.durable_boundary = boundary_for_fault(point);
                    }
                    return Ok(());
                }
                if let Err(error) = super::fault(OperationalFaultPoint::AfterProjection) {
                    self.last_outcome = Some(CoordinatorRunOutcome::FailedClosed {
                        phase: format!("{:?}", error.phase()),
                    });
                    if let Some(point) = fault {
                        self.durable_boundary = boundary_for_fault(point);
                    }
                    return Ok(());
                }
            }
            self.last_outcome = Some(CoordinatorRunOutcome::Complete);
            self.durable_boundary = CoordinatorDurableBoundary::Complete;
            Ok(())
        }

        fn sqlite(&mut self, mutation: &CoordinatorSqliteMutation) -> Result<(), String> {
            match mutation {
                CoordinatorSqliteMutation::Reopen => self.reopen_sqlite(),
                CoordinatorSqliteMutation::Delete => {
                    self.close_sqlite();
                    match fs::remove_file(&self.database_path) {
                        Ok(()) => Ok(()),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                        Err(error) => Err(io(error)),
                    }
                }
                CoordinatorSqliteMutation::Truncate { len } => {
                    self.close_sqlite();
                    let file = fs::OpenOptions::new()
                        .write(true)
                        .open(&self.database_path)
                        .map_err(io)?;
                    file.set_len(u64::try_from(*len).map_err(display)?)
                        .map_err(io)
                }
                CoordinatorSqliteMutation::Corrupt { offset, mask } => {
                    self.close_sqlite();
                    let mut file = fs::OpenOptions::new()
                        .read(true)
                        .write(true)
                        .open(&self.database_path)
                        .map_err(io)?;
                    let len =
                        usize::try_from(file.metadata().map_err(io)?.len()).map_err(display)?;
                    if *offset >= len {
                        return Err("SQLite corruption offset is outside the file".into());
                    }
                    file.seek(SeekFrom::Start(u64::try_from(*offset).map_err(display)?))
                        .map_err(io)?;
                    let mut byte = [0_u8; 1];
                    file.read_exact(&mut byte).map_err(io)?;
                    byte[0] ^= *mask;
                    file.seek(SeekFrom::Start(u64::try_from(*offset).map_err(display)?))
                        .map_err(io)?;
                    file.write_all(&byte).map_err(io)?;
                    file.sync_all().map_err(io)
                }
                CoordinatorSqliteMutation::StaleFrontier => self.stale_sqlite_frontier(),
            }
        }

        /// Replace the persisted frontier with the preceding authenticated
        /// root while leaving the newer materialized rows in place.  This is
        /// a storage fault injection only; recovery always uses the production
        /// authenticated `open_or_rebuild` path below.
        fn stale_sqlite_frontier(&mut self) -> Result<(), String> {
            self.close_sqlite();
            let engine = self.engine.as_ref().ok_or("coordinator engine is closed")?;
            let archive = self
                .archive
                .as_ref()
                .ok_or("coordinator archive is closed")?;
            let accepted = engine
                .accepted_frontier_root()
                .map_err(display)?
                .acceptance_sequence();
            let stale_sequence = accepted
                .checked_sub(1)
                .filter(|sequence| *sequence > 0)
                .ok_or("SQLite stale-frontier fault requires two accepted batches")?;
            let source = RebuildSource::new(engine, archive).map_err(display)?;
            let stale = source
                .accepted_event_at(stale_sequence)
                .map_err(display)?
                .post_frontier_root()
                .clone();
            let root_bytes = postcard::to_allocvec(&stale).map_err(display)?;
            let digest = ContentDigest::of(&root_bytes);
            let connection = Connection::open(&self.database_path).map_err(display)?;
            connection
                .execute(
                    "UPDATE frontier
                     SET frontier_root = ?1,
                         frontier_root_digest = ?2,
                         applied_batch_count = ?3
                     WHERE singleton = 1",
                    params![
                        root_bytes,
                        digest.as_bytes().as_slice(),
                        i64::try_from(stale_sequence).map_err(display)?,
                    ],
                )
                .map_err(display)?;
            connection
                .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")
                .map_err(display)
        }

        fn close_sqlite(&mut self) {
            self.tail.take();
            self.database.take();
        }

        fn reopen_sqlite(&mut self) -> Result<(), String> {
            self.close_sqlite();
            let engine = self.engine.as_ref().ok_or("coordinator engine is closed")?;
            let archive = self
                .archive
                .as_ref()
                .ok_or("coordinator archive is closed")?;
            let runtime_root = self
                .runtime_root
                .as_ref()
                .ok_or("coordinator runtime root is closed")?;
            let source = RebuildSource::new(engine, archive).map_err(display)?;
            let database = SqliteFrontier::open_or_rebuild(
                &self.database_path,
                runtime_root,
                ProjectionClaim::current(
                    self.workspace.workspace_id,
                    self.workspace.lineage_digest,
                ),
                source,
            )
            .map_err(display)?
            .database;
            let source = RebuildSource::new(engine, archive).map_err(display)?;
            let tail = TailOverlay::from_durable(&database, &source).map_err(display)?;
            self.database = Some(database);
            self.tail = Some(tail);
            Ok(())
        }

        fn assert_oracle(&mut self, oracle: &CoordinatorOracle) -> Result<(), String> {
            let observed = self.observation()?;
            let frontiers_match = observed
                .sqlite_frontier_digest
                .as_deref()
                .is_some_and(|sqlite| sqlite == observed.accepted_frontier_digest);
            let scalar_matches = oracle
                .accepted_sequence
                .is_none_or(|value| value == observed.accepted_sequence)
                && oracle
                    .accepted_frontier_digest
                    .as_ref()
                    .is_none_or(|value| value == &observed.accepted_frontier_digest)
                && oracle
                    .accepted_batches
                    .as_ref()
                    .is_none_or(|value| value == &observed.accepted_batches)
                && oracle
                    .sqlite_sequence
                    .is_none_or(|value| observed.sqlite_sequence == Some(value))
                && oracle
                    .sqlite_frontier_digest
                    .as_ref()
                    .is_none_or(|value| Some(value) == observed.sqlite_frontier_digest.as_ref())
                && oracle
                    .sqlite_row_digest
                    .as_ref()
                    .is_none_or(|value| Some(value) == observed.sqlite_row_digest.as_ref())
                && oracle
                    .frontiers_match
                    .is_none_or(|value| value == frontiers_match)
                && oracle
                    .pending_projection_work
                    .is_none_or(|value| value == observed.pending_projection_work)
                && oracle
                    .tail_unapplied_batches
                    .is_none_or(|value| value == observed.tail_unapplied_batches)
                && oracle
                    .tail_retained_bytes
                    .is_none_or(|value| value == observed.tail_retained_bytes)
                && oracle.handoff.is_none_or(|value| value == observed.handoff)
                && oracle
                    .read_gate
                    .is_none_or(|value| value == observed.read_gate)
                && oracle
                    .durable_boundary
                    .is_none_or(|value| value == observed.durable_boundary)
                && oracle
                    .last_outcome
                    .as_ref()
                    .is_none_or(|value| Some(value) == observed.last_outcome.as_ref());
            let files_match = oracle
                .sqlite_files
                .as_ref()
                .is_none_or(|expected| expected == &observed.sqlite_files)
                && oracle
                    .managed_files
                    .as_ref()
                    .is_none_or(|expected| expected == &observed.managed_files)
                && oracle
                    .archive_files
                    .as_ref()
                    .is_none_or(|expected| expected == &observed.archive_files)
                && oracle
                    .receipt_files
                    .as_ref()
                    .is_none_or(|expected| expected == &observed.receipt_files)
                && oracle
                    .sqlite_file_digests
                    .as_ref()
                    .is_none_or(|expected| expected == &file_digests(&observed.sqlite_files))
                && oracle
                    .archive_file_digests
                    .as_ref()
                    .is_none_or(|expected| expected == &file_digests(&observed.archive_files))
                && oracle
                    .receipt_file_digests
                    .as_ref()
                    .is_none_or(|expected| expected == &file_digests(&observed.receipt_files));
            if scalar_matches && files_match {
                Ok(())
            } else {
                self.expected_failure = Some(CoordinatorExpectedState::Oracle(oracle.clone()));
                Err(format!(
                    "coordinator oracle mismatch: expected {oracle:?}, observed {observed:?}"
                ))
            }
        }

        pub(crate) fn expected_failure(&self) -> Option<CoordinatorExpectedState> {
            self.expected_failure.clone()
        }

        pub(crate) fn assert_global_oracle(&self) -> Result<(), String> {
            let observed = self.observation()?;
            if observed.read_gate == CoordinatorReadGate::Open
                && (observed.sqlite_row_digest.is_none()
                    || observed.sqlite_files.is_empty()
                    || observed.sqlite_sequence != Some(observed.accepted_sequence)
                    || observed.sqlite_frontier_digest.as_deref()
                        != Some(observed.accepted_frontier_digest.as_str()))
            {
                return Err(format!(
                    "open coordinator read gate lacks exact accepted SQLite evidence: {observed:?}"
                ));
            }
            if observed.last_outcome == Some(CoordinatorRunOutcome::Complete)
                && ((observed.sqlite_sequence.is_some()
                    && observed.read_gate != CoordinatorReadGate::Open)
                    || observed.pending_projection_work != 0
                    || observed.tail_unapplied_batches != 0
                    || observed.tail_retained_bytes != 0
                    || !matches!(
                        observed.handoff,
                        CoordinatorHandoffState::Released
                            | CoordinatorHandoffState::EnrollmentPendingUnprotected
                    ))
            {
                return Err(format!(
                    "completed coordinator retained unfinished durable work: {observed:?}"
                ));
            }
            Ok(())
        }
    }

    fn install_fault(
        point: CoordinatorFault,
    ) -> Option<super::super::projection::ManifestedProjectionFaultScope> {
        match point {
            CoordinatorFault::AfterObjects => {
                super::super::object_store::fail_next_publish_after_objects_for_harness();
                None
            }
            CoordinatorFault::DuringSqliteApply => {
                super::super::sqlite::fail_next_apply_during_materialization_for_harness();
                None
            }
            CoordinatorFault::DuringProjection => Some(
                super::super::projection::fail_next_manifested_projection_during_write_for_harness(
                ),
            ),
            point => {
                fail_once_at(fault_point(point).expect("ordinary coordinator fault point"));
                None
            }
        }
    }

    fn fault_point(point: CoordinatorFault) -> Option<OperationalFaultPoint> {
        Some(match point {
            CoordinatorFault::AfterHandoff => OperationalFaultPoint::AfterHandoff,
            CoordinatorFault::AfterPlan => OperationalFaultPoint::AfterPlan,
            CoordinatorFault::AfterDraft => OperationalFaultPoint::AfterDraft,
            CoordinatorFault::AfterCapture => OperationalFaultPoint::AfterCapture,
            CoordinatorFault::AfterFinalize => OperationalFaultPoint::AfterFinalize,
            CoordinatorFault::AfterReservation => OperationalFaultPoint::AfterReservation,
            CoordinatorFault::AfterManifest => OperationalFaultPoint::AfterManifest,
            CoordinatorFault::AfterStage => OperationalFaultPoint::AfterStage,
            CoordinatorFault::AfterTailAdmission => OperationalFaultPoint::AfterTailAdmission,
            CoordinatorFault::AfterSqliteApply => OperationalFaultPoint::AfterSqliteApply,
            CoordinatorFault::BeforeProjection => OperationalFaultPoint::BeforeProjection,
            CoordinatorFault::AfterProjection => OperationalFaultPoint::AfterProjection,
            CoordinatorFault::AfterObjects
            | CoordinatorFault::DuringSqliteApply
            | CoordinatorFault::DuringProjection => return None,
        })
    }

    fn boundary_for_fault(point: CoordinatorFault) -> CoordinatorDurableBoundary {
        match point {
            CoordinatorFault::AfterHandoff => CoordinatorDurableBoundary::AfterHandoff,
            CoordinatorFault::AfterPlan => CoordinatorDurableBoundary::AfterPlan,
            CoordinatorFault::AfterDraft => CoordinatorDurableBoundary::AfterDraft,
            CoordinatorFault::AfterCapture => CoordinatorDurableBoundary::AfterCapture,
            CoordinatorFault::AfterFinalize => CoordinatorDurableBoundary::AfterFinalize,
            CoordinatorFault::AfterReservation => CoordinatorDurableBoundary::AfterReservation,
            CoordinatorFault::AfterObjects => CoordinatorDurableBoundary::AfterObjects,
            CoordinatorFault::AfterManifest => CoordinatorDurableBoundary::AfterManifest,
            CoordinatorFault::AfterStage => CoordinatorDurableBoundary::AfterStage,
            CoordinatorFault::AfterTailAdmission => CoordinatorDurableBoundary::AfterTailAdmission,
            CoordinatorFault::DuringSqliteApply => CoordinatorDurableBoundary::DuringSqliteApply,
            CoordinatorFault::AfterSqliteApply => CoordinatorDurableBoundary::AfterSqliteApply,
            CoordinatorFault::BeforeProjection => CoordinatorDurableBoundary::BeforeProjection,
            CoordinatorFault::DuringProjection => CoordinatorDurableBoundary::DuringProjection,
            CoordinatorFault::AfterProjection => CoordinatorDurableBoundary::AfterProjection,
        }
    }

    fn file_digests(files: &[ExternalFileFixture]) -> BTreeMap<String, String> {
        files
            .iter()
            .map(|file| {
                (
                    file.path.clone(),
                    hex(ContentDigest::of(&file.bytes_b64.0).as_bytes()),
                )
            })
            .collect()
    }

    fn pending_projection_work(engine: &ShardedHotEngine) -> Result<usize, String> {
        let index = engine.projection_work_index().map_err(display)?;
        let mut ready = 0_usize;
        let mut cursor = None;
        loop {
            let page = index.ready_page(cursor.as_ref(), 1).map_err(display)?;
            ready = ready.saturating_add(page.work().len());
            let next = page.next().cloned();
            if next.is_none() {
                break;
            }
            cursor = next;
        }
        let mut pending = 0_usize;
        let mut cursor = None;
        loop {
            let page = index
                .pending_activation_page(cursor.as_ref(), 1)
                .map_err(display)?;
            pending = pending.saturating_add(
                page.pending()
                    .iter()
                    .map(|entry| entry.work_ids().len())
                    .sum::<usize>(),
            );
            let next = page.next().cloned();
            if next.is_none() {
                break;
            }
            cursor = next;
        }
        Ok(ready.saturating_add(pending))
    }

    fn frontier_digest(root: &impl serde::Serialize) -> Result<String, String> {
        let bytes = postcard::to_allocvec(root).map_err(display)?;
        Ok(hex(ContentDigest::of(&bytes).as_bytes()))
    }

    fn snapshot_archive(root: &Path) -> Result<Vec<ExternalFileFixture>, String> {
        let mut files = Vec::new();
        for directory in ["objects", "batches"] {
            let nested = root.join(directory);
            let mut entries = snapshot_tree(&nested, false)?;
            for entry in &mut entries {
                entry.path = format!("{directory}/{}", entry.path);
            }
            files.extend(entries);
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(files)
    }

    fn snapshot_sqlite(database_path: &Path) -> Result<Vec<ExternalFileFixture>, String> {
        let parent = database_path
            .parent()
            .ok_or("SQLite database has no parent directory")?;
        snapshot_tree(parent, false)
    }

    fn same_durable_evidence(
        expected: &CoordinatorObservation,
        observed: &CoordinatorObservation,
    ) -> bool {
        expected.accepted_sequence == observed.accepted_sequence
            && expected.accepted_frontier_digest == observed.accepted_frontier_digest
            && expected.accepted_batches == observed.accepted_batches
            && expected.sqlite_sequence == observed.sqlite_sequence
            && expected.sqlite_frontier_digest == observed.sqlite_frontier_digest
            && expected.sqlite_row_digest == observed.sqlite_row_digest
            && expected.sqlite_files == observed.sqlite_files
            && expected.managed_files == observed.managed_files
            && expected.archive_files == observed.archive_files
            && expected.receipt_files == observed.receipt_files
            && expected.pending_projection_work == observed.pending_projection_work
            && expected.tail_unapplied_batches == observed.tail_unapplied_batches
            && expected.tail_retained_bytes == observed.tail_retained_bytes
            && expected.handoff == observed.handoff
            && expected.read_gate == observed.read_gate
    }

    fn same_accepted_archive_evidence(
        expected: &CoordinatorObservation,
        observed: &CoordinatorObservation,
    ) -> bool {
        expected.accepted_sequence == observed.accepted_sequence
            && expected.accepted_frontier_digest == observed.accepted_frontier_digest
            && expected.accepted_batches == observed.accepted_batches
            && expected.archive_files == observed.archive_files
    }

    fn same_materialization_evidence(
        expected: &CoordinatorObservation,
        observed: &CoordinatorObservation,
    ) -> bool {
        expected.accepted_sequence == observed.accepted_sequence
            && expected.accepted_frontier_digest == observed.accepted_frontier_digest
            && expected.accepted_batches == observed.accepted_batches
            && expected.sqlite_sequence == observed.sqlite_sequence
            && expected.sqlite_frontier_digest == observed.sqlite_frontier_digest
            && expected.sqlite_row_digest == observed.sqlite_row_digest
            // A clean rebuild is allowed to choose a different SQLite page
            // layout, but it must recreate a visible database file alongside
            // the exact type-tagged row digest and frontier.
            && !observed.sqlite_files.is_empty()
            && expected.managed_files == observed.managed_files
            && expected.archive_files == observed.archive_files
            && expected.receipt_files == observed.receipt_files
            && expected.pending_projection_work == observed.pending_projection_work
            && expected.tail_unapplied_batches == observed.tail_unapplied_batches
            && expected.tail_retained_bytes == observed.tail_retained_bytes
            && expected.handoff == observed.handoff
            && expected.read_gate == observed.read_gate
    }

    fn snapshot_tree(root: &Path, skip_config: bool) -> Result<Vec<ExternalFileFixture>, String> {
        fn walk(
            root: &Path,
            current: &Path,
            skip_config: bool,
            output: &mut Vec<ExternalFileFixture>,
        ) -> Result<(), String> {
            let mut entries = fs::read_dir(current)
                .map_err(io)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(io)?;
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let path = entry.path();
                if entry.file_type().map_err(io)?.is_dir() {
                    walk(root, &path, skip_config, output)?;
                    continue;
                }
                let relative = path
                    .strip_prefix(root)
                    .map_err(display)?
                    .to_str()
                    .ok_or("non-UTF-8 harness path")?
                    .replace('\\', "/");
                if skip_config && relative == "logseq/config.edn" {
                    continue;
                }
                output.push(ExternalFileFixture {
                    path: relative,
                    bytes_b64: WireBytes(fs::read(path).map_err(io)?),
                });
            }
            Ok(())
        }
        let mut output = Vec::new();
        walk(root, root, skip_config, &mut output)?;
        Ok(output)
    }

    fn hex(bytes: &[u8]) -> String {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut value = String::with_capacity(bytes.len().saturating_mul(2));
        for byte in bytes {
            value.push(DIGITS[(byte >> 4) as usize] as char);
            value.push(DIGITS[(byte & 0x0f) as usize] as char);
        }
        value
    }

    fn io(error: std::io::Error) -> String {
        error.to_string()
    }

    fn display(error: impl std::fmt::Display) -> String {
        error.to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use uuid::Uuid;

    use super::*;
    use crate::model::{projection_graph_test_counters, reset_projection_graph_test_counters};
    use crate::oplog::object_store::{
        fail_next_engine_history_head_swap, fail_next_publish_after_objects,
    };
    use crate::oplog::projection::fail_next_formatting_adoption_after_intent_for_harness;
    use crate::oplog::{
        recover_incomplete_projections, write_projection_exact, AnnotatedProjectionBase,
        ApplicationRuntimeRoot, BlockId, BlockLocation, DeviceId, DocumentId, LineageDigest,
        LogicalPageName, ManagedPath, ManagedTextKind, ManifestProjectionPrecondition,
        ManifestedProjectionIntent, ObjectKind, OperationTransaction, PageId, ProjectionClaim,
        ProjectionEndpointId, SemanticOperation, TAIL_MAX_BYTES,
    };

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "tine-operational-coordinator-{label}-{}",
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

    struct Fixture {
        _root: TestRoot,
        graph_root: PathBuf,
        archive_root: PathBuf,
        graph: Graph,
        receipts: ProjectionReceiptStore,
        archive: ObjectStore,
        engine: ShardedHotEngine,
        database: SqliteFrontier,
        tail: TailOverlay,
        lineage: LineageDigest,
        catalog: DocumentId,
        home_document_id: DocumentId,
        block_id: BlockId,
        intent: super::super::ProjectionIntent,
        path: String,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            Self::new_at(
                label,
                "pages/deep/projects/a.md",
                None,
                ManagedTextKind::Page,
            )
        }

        fn configured(label: &str) -> Self {
            Self::new_at(
                label,
                "content/pages/deep/projects/a.md",
                Some(
                    "{:pages-directory \"content/pages\"\n\
                      :journals-directory \"content/journals\"}\n",
                ),
                ManagedTextKind::Page,
            )
        }

        fn formatting_only(label: &str) -> Self {
            Self::new_at_named(
                label,
                "pages/Coordinator Page.md",
                None,
                ManagedTextKind::Page,
                "Coordinator Page",
                true,
            )
        }

        fn new_at(label: &str, path: &str, config: Option<&str>, kind: ManagedTextKind) -> Self {
            Self::new_at_named(label, path, config, kind, "Coordinator Page", false)
        }

        fn new_at_named(
            label: &str,
            path: &str,
            config: Option<&str>,
            kind: ManagedTextKind,
            logical_name: &str,
            imported_orders: bool,
        ) -> Self {
            let root = TestRoot::new(label);
            let graph_root = root.path().join("graph");
            fs::create_dir_all(&graph_root).unwrap();
            if let Some(config) = config {
                fs::create_dir_all(graph_root.join("logseq")).unwrap();
                fs::write(graph_root.join("logseq/config.edn"), config).unwrap();
            }
            let graph = Graph::open(&graph_root);
            let workspace_id = super::super::WorkspaceId::from_uuid(Uuid::from_u128(1));
            let endpoint = ProjectionEndpointBinding::enroll_graph(
                &graph,
                ProjectionEndpointId::from_uuid(Uuid::from_u128(2)),
                DeviceId::from_uuid(Uuid::from_u128(3)),
            )
            .unwrap();
            let receipts = ProjectionReceiptStore::open_for_endpoint(
                &root.path().join("receipts"),
                workspace_id,
                endpoint,
            )
            .unwrap();
            let lineage = LineageDigest::of(label.as_bytes());
            let catalog = DocumentId::from_uuid(Uuid::from_u128(4));
            let page_id = PageId::from_uuid(Uuid::from_u128(5));
            let home = DocumentId::from_uuid(Uuid::from_u128(6));
            let block = BlockId::from_uuid(Uuid::from_u128(7));
            let managed_path = ManagedPath::parse(path).unwrap();
            let fixture_order = if imported_orders {
                super::super::import::imported_order(0)
            } else {
                "a".into()
            };
            let transaction = OperationTransaction::new(vec![
                SemanticOperation::CreatePage {
                    page_id,
                    home_document_id: home,
                    name: LogicalPageName::parse(logical_name).unwrap(),
                    path: managed_path,
                    kind,
                },
                SemanticOperation::CreateBlock {
                    block: BlockLocation {
                        block_id: block,
                        home_document_id: home,
                    },
                    page_id,
                    parent: None,
                    order: fixture_order.clone(),
                    content: "root".into(),
                },
                SemanticOperation::CreateBlock {
                    block: BlockLocation {
                        block_id: BlockId::from_uuid(Uuid::from_u128(8)),
                        home_document_id: home,
                    },
                    page_id,
                    parent: Some(block),
                    order: fixture_order,
                    content: "child".into(),
                },
            ])
            .unwrap();
            let author = ShardedHotEngine::new(workspace_id, lineage, catalog);
            let bootstrap = author
                .prepare_bootstrap_transaction(
                    AuthorBatch {
                        batch_id: BatchId::from_uuid(Uuid::from_u128(9)),
                        author_device_id: DeviceId::from_uuid(Uuid::from_u128(10)),
                        author_session_id: SessionId::from_uuid(Uuid::from_u128(11)),
                        crdt_peer_id: CrdtPeerId::from_u64(12),
                    },
                    &transaction,
                )
                .unwrap();
            let archive_root = root.path().join("archive");
            ObjectStore::open(&archive_root, workspace_id)
                .unwrap()
                .publish_bootstrap_prepared_for_test(&bootstrap)
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
            let intent = write_projection_exact(&graph, &receipts, &engine, page_id, None)
                .unwrap()
                .plan
                .intent()
                .clone();
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
            assert_eq!(
                database.frontier_root().unwrap(),
                engine.accepted_frontier_root().unwrap()
            );
            Self {
                _root: root,
                graph_root,
                archive_root,
                graph,
                receipts,
                archive,
                engine,
                database,
                tail,
                lineage,
                catalog,
                home_document_id: home,
                block_id: block,
                intent,
                path: path.into(),
            }
        }

        fn overwrite(&self, bytes: &[u8]) {
            fs::write(self.graph_root.join(&self.path), bytes).unwrap();
        }

        fn execute(&mut self, paths: &[&str]) -> OperationalCoordinatorState {
            OperationalCoordinator::execute(
                &LocalRuntimeAdmission::unenrolled_pre_activation(),
                &self.graph,
                &self.receipts,
                &mut self.engine,
                &mut self.database,
                &mut self.tail,
                paths,
            )
            .unwrap()
        }

        fn local_author(&self, seed: u128) -> AuthorBatch {
            AuthorBatch {
                batch_id: BatchId::from_uuid(Uuid::from_u128(seed)),
                author_device_id: self
                    .engine
                    .projection_endpoint_binding()
                    .unwrap()
                    .device_id(),
                author_session_id: SessionId::from_uuid(Uuid::from_u128(seed + 1)),
                crdt_peer_id: CrdtPeerId::from_u64((seed as u64).saturating_add(10_001)),
            }
        }

        fn execute_local(
            &mut self,
            author: AuthorBatch,
            transaction: &OperationTransaction,
        ) -> LocalMutationCoordinatorState {
            OperationalCoordinator::execute_local_with_author(
                &LocalRuntimeAdmission::unenrolled_pre_activation(),
                &self.graph,
                &self.receipts,
                &mut self.engine,
                &mut self.database,
                &mut self.tail,
                author,
                transaction,
            )
        }

        fn local_edit(&mut self, seed: u128, content: &str) -> LocalMutationCoordinatorState {
            let transaction =
                OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
                    block: BlockLocation {
                        block_id: self.block_id,
                        home_document_id: self.home_document_id,
                    },
                    content: content.into(),
                }])
                .unwrap();
            let author = self.local_author(seed);
            self.execute_local(author, &transaction)
        }

        fn assert_drained(&self) {
            assert_eq!(
                self.database.frontier_root().unwrap(),
                self.engine.accepted_frontier_root().unwrap()
            );
            assert!(self
                .engine
                .projection_work_index()
                .unwrap()
                .ready_page(None, 1)
                .unwrap()
                .work()
                .is_empty());
            assert_eq!(self.tail.status().unapplied_batches, 0);
            assert_eq!(self.tail.status().retained_bytes, 0);
        }

        fn restart_projection_runtime(self) -> Self {
            let Self {
                _root,
                graph_root,
                archive_root,
                graph,
                receipts,
                archive,
                engine,
                database,
                tail,
                lineage,
                catalog,
                home_document_id,
                block_id,
                intent,
                path,
            } = self;
            let endpoint = receipts.endpoint_binding().unwrap();
            let receipt_root = receipts.root_path().to_path_buf();
            let workspace = engine.workspace_id();
            drop(tail);
            drop(database);
            drop(engine);
            drop(archive);
            drop(receipts);
            drop(graph);

            let graph = Graph::open(&graph_root);
            let receipts =
                ProjectionReceiptStore::open_for_endpoint(&receipt_root, workspace, endpoint)
                    .unwrap();
            let archive = ObjectStore::open(&archive_root, workspace).unwrap();
            let manifests = archive.committed_manifests().unwrap();
            let engine = ShardedHotEngine::open_enrolled_projection_resuming(
                ObjectStore::open(&archive_root, workspace).unwrap(),
                lineage,
                catalog,
                &graph,
                &receipts,
                None,
                &manifests,
                None,
            )
            .unwrap()
            .0;
            let runtime =
                ApplicationRuntimeRoot::open_for_test(&_root.path().join("runtime")).unwrap();
            let database_path = _root.path().join("sqlite/materialized.sqlite3");
            let source = RebuildSource::new(&engine, &archive).unwrap();
            let database = SqliteFrontier::open_or_rebuild(
                &database_path,
                &runtime,
                ProjectionClaim::current(workspace, lineage),
                source,
            )
            .unwrap()
            .database;
            let source = RebuildSource::new(&engine, &archive).unwrap();
            let tail = TailOverlay::from_durable(&database, &source).unwrap();
            Self {
                _root,
                graph_root,
                archive_root,
                graph,
                receipts,
                archive,
                engine,
                database,
                tail,
                lineage,
                catalog,
                home_document_id,
                block_id,
                intent,
                path,
            }
        }
    }

    fn expect_complete(state: OperationalCoordinatorState) -> OperationalCompletion {
        match state {
            OperationalCoordinatorState::Complete(completion) => completion,
            OperationalCoordinatorState::Blocked(plan) => {
                panic!("unexpected blocked plan: {:?}", plan.blocks())
            }
            OperationalCoordinatorState::Noop => panic!("unexpected no-op"),
            OperationalCoordinatorState::FailedClosed(failed) => {
                panic!("unexpected failed-closed state: {}", failed.failure())
            }
        }
    }

    fn expect_failed(state: OperationalCoordinatorState) -> ExternalPublishedContinuation {
        match state {
            OperationalCoordinatorState::FailedClosed(failed) => failed,
            OperationalCoordinatorState::Blocked(plan) => {
                panic!("unexpected blocked plan: {:?}", plan.blocks())
            }
            OperationalCoordinatorState::Noop => panic!("unexpected no-op"),
            OperationalCoordinatorState::Complete(_) => panic!("unexpected completion"),
        }
    }

    fn expect_local_active(state: LocalMutationCoordinatorState) -> LocalMutationCompletion {
        match state {
            LocalMutationCoordinatorState::Active(completion) => completion,
            LocalMutationCoordinatorState::Recovering(recovery) => match recovery {
                LocalMutationRecovery::ReconciliationRequired(reconciliation) => panic!(
                    "unexpected local reconciliation: {:?}",
                    reconciliation.paths()
                ),
                LocalMutationRecovery::Published(continuation) => {
                    panic!("unexpected local continuation: {}", continuation.failure())
                }
            },
            LocalMutationCoordinatorState::Blocked(blocked) => {
                panic!("unexpected blocked local mutation: {}", blocked.failure())
            }
            LocalMutationCoordinatorState::Revoked(revoked) => {
                panic!("unexpected revoked local mutation: {}", revoked.failure())
            }
        }
    }

    /// Settle a local mutation whose publication legitimately needs more than
    /// one bounded turn.
    fn settle_local(
        fixture: &mut Fixture,
        mut state: LocalMutationCoordinatorState,
    ) -> LocalMutationCompletion {
        for _ in 0..8 {
            match state {
                LocalMutationCoordinatorState::Active(completion) => return completion,
                LocalMutationCoordinatorState::Recovering(LocalMutationRecovery::Published(
                    continuation,
                )) => {
                    state = continuation.retry(
                        &LocalRuntimeAdmission::unenrolled_pre_activation(),
                        &fixture.graph,
                        &fixture.receipts,
                        &mut fixture.engine,
                        &mut fixture.database,
                        &mut fixture.tail,
                    );
                }
                other => return expect_local_active(other),
            }
        }
        panic!("local mutation did not settle within the bounded turn budget")
    }

    fn expect_local_published_recovery(
        state: LocalMutationCoordinatorState,
    ) -> LocalPublishedContinuation {
        match state {
            LocalMutationCoordinatorState::Recovering(LocalMutationRecovery::Published(
                continuation,
            )) => continuation,
            LocalMutationCoordinatorState::Recovering(
                LocalMutationRecovery::ReconciliationRequired(reconciliation),
            ) => panic!(
                "unexpected local reconciliation: {:?}",
                reconciliation.paths()
            ),
            LocalMutationCoordinatorState::Active(_) => {
                panic!("unexpected completed local mutation")
            }
            LocalMutationCoordinatorState::Blocked(blocked) => {
                panic!("unexpected blocked local mutation: {}", blocked.failure())
            }
            LocalMutationCoordinatorState::Revoked(revoked) => {
                panic!("unexpected revoked local mutation: {}", revoked.failure())
            }
        }
    }

    #[test]
    fn fresh_nested_layout_reconcile_drains_history_sqlite_and_projection() {
        let mut fixture = Fixture::configured("nested-success");
        let path = fixture.path.clone();
        fixture.overwrite(b"- root edited\n\t- child edited\n");
        let completion = expect_complete(fixture.execute(&[&path]));
        assert_eq!(completion.batch_id(), completion.import_id().batch_id());
        fixture.assert_drained();
        assert_eq!(
            fs::read(fixture.graph_root.join(path)).unwrap(),
            b"- root edited\n\t- child edited\n"
        );
    }

    #[test]
    fn admitted_local_semantic_mutation_commits_history_sqlite_and_projection_once() {
        let mut fixture = Fixture::configured("local-success");
        let path = fixture.path.clone();
        let manifests_before = fixture.archive.committed_manifests().unwrap().len();
        let accepted_before = fixture.engine.accepted_batch_count().unwrap();
        let sqlite_before = fixture.database.applied_batch_count().unwrap();
        let releases_before = fixture.graph.handoff_release_count();
        reset_projection_graph_test_counters();

        let completion = expect_local_active(fixture.local_edit(40_000, "local semantic edit"));

        assert_eq!(
            fixture.archive.committed_manifests().unwrap().len(),
            manifests_before + 1
        );
        assert_eq!(
            fixture.engine.accepted_batch_count().unwrap(),
            accepted_before + 1
        );
        assert_eq!(
            fixture.database.applied_batch_count().unwrap(),
            sqlite_before + 1,
            "the local accepted event must be applied to SQLite exactly once"
        );
        assert!(fixture
            .database
            .contains_batch(completion.batch_id())
            .unwrap());
        let batch = match fixture
            .archive
            .inspect_batch(completion.batch_id())
            .unwrap()
        {
            BatchInspection::Ready(batch) => batch,
            other => panic!("local mutation did not reach authenticated history: {other:?}"),
        };
        assert_eq!(batch.manifest().origin(), BatchOrigin::LocalMutation);
        assert_eq!(
            fixture.database.frontier_root().unwrap(),
            fixture.engine.accepted_frontier_root().unwrap()
        );
        assert_eq!(projection_graph_test_counters().write_calls, 1);
        assert_eq!(
            fixture.graph.handoff_release_count(),
            releases_before + 1,
            "successful completion releases the handoff exactly once"
        );
        assert_eq!(
            fs::read(fixture.graph_root.join(path)).unwrap(),
            b"- local semantic edit\n\t- child\n"
        );
        fixture.assert_drained();
    }

    #[test]
    fn local_exact_path_drift_requests_reconciliation_without_publication() {
        let mut fixture = Fixture::new("local-reconcile-first");
        let path = fixture.path.clone();
        fixture.overwrite(b"- externally moved local base\n\t- child\n");
        let immutable_before = snapshot_immutable_publication(&fixture.archive_root);
        let frontier_before = fixture.engine.accepted_frontier_root().unwrap();
        let sqlite_before = fixture.database.frontier_root().unwrap();
        reset_projection_graph_test_counters();

        let state = fixture.local_edit(40_100, "must not overwrite external bytes");
        let LocalMutationCoordinatorState::Recovering(
            LocalMutationRecovery::ReconciliationRequired(reconciliation),
        ) = state
        else {
            panic!("exact local path drift must request reconciliation");
        };
        assert_eq!(
            reconciliation.paths(),
            &[ManagedPath::parse(&path).unwrap()]
        );
        assert_eq!(
            snapshot_immutable_publication(&fixture.archive_root),
            immutable_before
        );
        assert_eq!(
            fixture.engine.accepted_frontier_root().unwrap(),
            frontier_before
        );
        assert_eq!(fixture.database.frontier_root().unwrap(), sqlite_before);
        assert_eq!(projection_graph_test_counters().write_calls, 0);
        assert_eq!(
            fs::read(fixture.graph_root.join(path)).unwrap(),
            b"- externally moved local base\n\t- child\n"
        );
        fixture.graph.probe_managed_text_writer().unwrap();
    }

    #[test]
    fn stale_local_binding_is_typed_blocked_before_any_writer_side_effect() {
        let mut fixture = Fixture::new("local-stale-binding");
        let foreign_root = TestRoot::new("local-stale-binding-foreign");
        let foreign_graph_root = foreign_root.path().join("graph");
        fs::create_dir_all(&foreign_graph_root).unwrap();
        let foreign_graph = Graph::open(&foreign_graph_root);
        let foreign_endpoint = ProjectionEndpointBinding::enroll_graph(
            &foreign_graph,
            ProjectionEndpointId::from_uuid(Uuid::from_u128(44_100)),
            DeviceId::from_uuid(Uuid::from_u128(44_101)),
        )
        .unwrap();
        let foreign_receipts = ProjectionReceiptStore::open_for_endpoint(
            &foreign_root.path().join("receipts"),
            fixture.engine.workspace_id(),
            foreign_endpoint,
        )
        .unwrap();
        let immutable_before = snapshot_immutable_publication(&fixture.archive_root);
        let frontier_before = fixture.engine.accepted_frontier_root().unwrap();
        let sqlite_before = fixture.database.frontier_root().unwrap();
        let transaction = OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
            block: BlockLocation {
                block_id: fixture.block_id,
                home_document_id: fixture.home_document_id,
            },
            content: "blocked stale binding".into(),
        }])
        .unwrap();
        let author = fixture.local_author(40_200);

        let state = OperationalCoordinator::execute_local_with_author(
            &LocalRuntimeAdmission::unenrolled_pre_activation(),
            &fixture.graph,
            &foreign_receipts,
            &mut fixture.engine,
            &mut fixture.database,
            &mut fixture.tail,
            author,
            &transaction,
        );
        let LocalMutationCoordinatorState::Blocked(blocked) = state else {
            panic!("a stale local runtime binding must return Blocked");
        };
        assert_eq!(blocked.failure().phase(), OperationalPhase::Bindings);
        assert_eq!(
            snapshot_immutable_publication(&fixture.archive_root),
            immutable_before
        );
        assert_eq!(
            fixture.engine.accepted_frontier_root().unwrap(),
            frontier_before
        );
        assert_eq!(fixture.database.frontier_root().unwrap(), sqlite_before);
        fixture.graph.probe_managed_text_writer().unwrap();
    }

    #[test]
    fn local_and_external_mutations_enter_the_identical_terminal_pipeline() {
        reset_terminal_pipeline_origins();
        let mut local = Fixture::new("shared-terminal-local");
        expect_local_active(local.local_edit(41_000, "shared terminal local"));

        let mut external = Fixture::new("shared-terminal-external");
        let path = external.path.clone();
        external.overwrite(b"- shared terminal external\n\t- child\n");
        expect_complete(external.execute(&[&path]));

        let origins = terminal_pipeline_origins();
        assert_eq!(origins.len(), 2);
        assert_eq!(origins[0], BatchOrigin::LocalMutation);
        assert!(matches!(
            origins[1],
            BatchOrigin::ExternalReconciliation { .. }
        ));
    }

    #[test]
    fn local_late_failure_retries_exact_publication_without_a_second_writer() {
        for (index, point) in [
            OperationalFaultPoint::AfterManifest,
            OperationalFaultPoint::AfterStage,
            OperationalFaultPoint::AfterTailAdmission,
            OperationalFaultPoint::AfterSqliteApply,
            OperationalFaultPoint::BeforeProjection,
            OperationalFaultPoint::AfterProjection,
        ]
        .into_iter()
        .enumerate()
        {
            let mut fixture = Fixture::new(&format!("local-late-{point:?}"));
            let path = fixture.path.clone();
            let manifests_before = fixture.archive.committed_manifests().unwrap().len();
            reset_projection_graph_test_counters();
            fail_once_at(point);
            let failed = expect_local_published_recovery(
                fixture.local_edit(42_000 + index as u128 * 10, "late local edit"),
            );
            let batch_id = failed.batch_id();
            assert_eq!(
                fixture.archive.committed_manifests().unwrap().len(),
                manifests_before + 1
            );
            assert!(fixture.graph.probe_managed_text_writer().is_err());

            let completion = expect_local_active(failed.retry(
                &LocalRuntimeAdmission::unenrolled_pre_activation(),
                &fixture.graph,
                &fixture.receipts,
                &mut fixture.engine,
                &mut fixture.database,
                &mut fixture.tail,
            ));
            assert_eq!(completion.batch_id(), batch_id);
            assert_eq!(
                fixture.archive.committed_manifests().unwrap().len(),
                manifests_before + 1,
                "late retry republished the local mutation"
            );
            assert!(projection_graph_test_counters().write_calls <= 1);
            assert_eq!(
                fs::read(fixture.graph_root.join(&path)).unwrap(),
                b"- late local edit\n\t- child\n"
            );
            fixture.graph.probe_managed_text_writer().unwrap();
            fixture.assert_drained();
        }
    }

    #[test]
    fn local_semantic_paths_accept_nested_nonstandard_utf8_markdown_and_org() {
        for (index, (path, kind, expected)) in [
            (
                "content/pages/研究/über topic.md",
                ManagedTextKind::Page,
                b"- utf local edit\n\t- child\n".as_slice(),
            ),
            (
                "content/pages/研究/über topic.org",
                ManagedTextKind::Page,
                b"* utf local edit\n** child\n".as_slice(),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let mut fixture = Fixture::new_at(
                &format!("local-utf-{index}"),
                path,
                Some(
                    "{:pages-directory \"content/pages\"\n\
                      :journals-directory \"content/journals\"}\n",
                ),
                kind,
            );
            expect_local_active(fixture.local_edit(43_000 + index as u128 * 10, "utf local edit"));
            assert_eq!(fs::read(fixture.graph_root.join(path)).unwrap(), expected);
            fixture.assert_drained();
        }
    }

    /// The draft derivation that reads the unchanged catalog in place must
    /// publish exactly what the previous whole-copy derivation published, in
    /// both managed source languages, including when publication only settles
    /// on the durable retry.
    #[test]
    fn in_place_catalog_derivation_publishes_the_same_markdown_and_org_source() {
        for (index, (path, kind, edited, inserted, settled)) in [
            (
                "content/pages/研究/über topic.md",
                ManagedTextKind::Page,
                b"- TODO utf derivation edit [[Other Page]]\n\t- child\n".as_slice(),
                b"- TODO utf derivation edit [[Other Page]]\n\t- child\n- DONE appended tail\n"
                    .as_slice(),
                b"- TODO settled after deferral\n\t- child\n- DONE appended tail\n".as_slice(),
            ),
            (
                "content/pages/研究/über topic.org",
                ManagedTextKind::Page,
                b"* TODO utf derivation edit [[Other Page]]\n** child\n".as_slice(),
                b"* TODO utf derivation edit [[Other Page]]\n** child\n* DONE appended tail\n"
                    .as_slice(),
                b"* TODO settled after deferral\n** child\n* DONE appended tail\n".as_slice(),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let mut fixture = Fixture::new_at(
                &format!("local-derivation-{index}"),
                path,
                Some(
                    "{:pages-directory \"content/pages\"\n\
                      :journals-directory \"content/journals\"}\n",
                ),
                kind,
            );

            let edit_author = fixture.local_author(44_000 + index as u128 * 100);
            let edit = OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: fixture.block_id,
                    home_document_id: fixture.home_document_id,
                },
                content: "TODO utf derivation edit [[Other Page]]".into(),
            }])
            .unwrap();
            let observed = fixture.engine.assert_draft_matches_previous_derivation(
                edit_author,
                BatchOrigin::LocalMutation,
                &edit,
            );
            assert_eq!(observed.refused, None);
            assert_eq!(
                observed.optimized_catalog_copies, 0,
                "a page-local content edit must read the catalog in place"
            );
            assert!(observed.oracle_catalog_copies >= 1);
            let state = fixture.execute_local(edit_author, &edit);
            settle_local(&mut fixture, state);
            assert_eq!(fs::read(fixture.graph_root.join(path)).unwrap(), edited);
            fixture.assert_drained();

            // Deferred, then durable: the same derivation must survive a
            // publication that only completes on the retry.
            let insert_author = fixture.local_author(44_050 + index as u128 * 100);
            let insert = OperationTransaction::new(vec![SemanticOperation::CreateBlock {
                block: BlockLocation {
                    block_id: BlockId::from_uuid(Uuid::from_u128(44_900 + index as u128)),
                    home_document_id: fixture.home_document_id,
                },
                page_id: PageId::from_uuid(Uuid::from_u128(5)),
                parent: None,
                order: "b".into(),
                content: "DONE appended tail".into(),
            }])
            .unwrap();
            let observed = fixture.engine.assert_draft_matches_previous_derivation(
                insert_author,
                BatchOrigin::LocalMutation,
                &insert,
            );
            assert_eq!(observed.refused, None);
            assert_eq!(observed.optimized_catalog_copies, 0);
            assert!(observed.oracle_catalog_copies >= 1);

            let state = fixture.execute_local(insert_author, &insert);
            settle_local(&mut fixture, state);
            assert_eq!(fs::read(fixture.graph_root.join(path)).unwrap(), inserted);
            fixture.assert_drained();

            // Deferred, then durable: the same derivation must survive a
            // publication that only completes on the retry.
            let settle_author = fixture.local_author(44_070 + index as u128 * 100);
            let settle = OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: fixture.block_id,
                    home_document_id: fixture.home_document_id,
                },
                content: "TODO settled after deferral".into(),
            }])
            .unwrap();
            let observed = fixture.engine.assert_draft_matches_previous_derivation(
                settle_author,
                BatchOrigin::LocalMutation,
                &settle,
            );
            assert_eq!(observed.refused, None);
            assert_eq!(observed.optimized_catalog_copies, 0);
            fail_once_at(OperationalFaultPoint::BeforeProjection);
            let deferred =
                expect_local_published_recovery(fixture.execute_local(settle_author, &settle));
            let batch_id = deferred.batch_id();
            let state = deferred.retry(
                &LocalRuntimeAdmission::unenrolled_pre_activation(),
                &fixture.graph,
                &fixture.receipts,
                &mut fixture.engine,
                &mut fixture.database,
                &mut fixture.tail,
            );
            let completion = settle_local(&mut fixture, state);
            assert_eq!(completion.batch_id(), batch_id);
            assert_eq!(fs::read(fixture.graph_root.join(path)).unwrap(), settled);
            fixture.assert_drained();

            // Restart and replay must reproduce exactly the same source.
            let restarted = fixture.restart_projection_runtime();
            assert_eq!(fs::read(restarted.graph_root.join(path)).unwrap(), settled);
            restarted.assert_drained();
        }
    }

    #[test]
    fn production_local_api_owns_author_identity_and_raw_entry_is_test_only() {
        let source = include_str!("operational_coordinator.rs");
        let production = source
            .split_once("\n#[cfg(test)]\nmod tests")
            .expect("the coordinator test module must remain separated")
            .0;
        let signature = production
            .split_once("pub(crate) fn execute_local(")
            .expect("the production local entry must exist")
            .1
            .split_once(") -> LocalMutationCoordinatorState")
            .expect("the production local signature must close")
            .0;
        assert!(signature.contains("session: &mut PromotedRuntimeSession<'_>"));
        assert!(signature.contains("transaction: &OperationTransaction"));
        for forbidden in [
            "AuthorBatch",
            "BatchId",
            "DeviceId",
            "SessionId",
            "CrdtPeerId",
            "LocalRuntimeAdmission",
            "ShardedHotEngine",
            "SqliteFrontier",
            "TailOverlay",
        ] {
            assert!(
                !signature.contains(forbidden),
                "production local callers can still supply authoritative `{forbidden}`"
            );
        }
        assert!(
            production.contains(
                "#[cfg(test)]\n    #[allow(clippy::too_many_arguments)]\n    fn \
                 execute_local_with_author("
            ),
            "the raw-author coordinator entry must stay test-only"
        );

        let engine = include_str!("hot_engine.rs");
        let raw_engine = engine
            .split_once("pub fn draft_author_transaction(")
            .expect("the legacy raw fixture helper remains named")
            .1
            .split_once("self.draft_author_transaction_with_observation")
            .expect("the raw helper must still delegate to the shared draft core")
            .0;
        assert!(raw_engine.contains("#[cfg(not(test))]"));
        assert!(raw_engine.contains("self.promoted_lineage().is_some()"));
        assert!(raw_engine.contains("origin == BatchOrigin::LocalMutation"));
        assert!(
            raw_engine.contains("raw local author identity is unavailable"),
            "a production promoted engine must refuse the raw-author fixture helper"
        );
    }

    #[test]
    fn origin_specific_continuations_are_affine_nonserialized_and_panic_free() {
        let source = include_str!("operational_coordinator.rs");
        let production = source
            .split_once("\n#[cfg(test)]\nmod tests")
            .expect("the coordinator test module must remain separated")
            .0;
        assert!(production.contains("pub(crate) struct ExternalPublishedContinuation"));
        assert!(production.contains("pub(crate) struct LocalPublishedContinuation"));
        assert!(!production.contains("panic!("));

        for name in [
            "ExternalPublishedContinuation",
            "LocalPublishedContinuation",
        ] {
            let declaration = format!("pub(crate) struct {name}");
            let offset = production.find(&declaration).unwrap();
            let prefix = &production[offset.saturating_sub(120)..offset];
            assert!(
                !prefix.contains("#[derive("),
                "{name} must remain non-cloneable and non-serializable"
            );
        }
        let external = production
            .split_once("impl ExternalPublishedContinuation {")
            .unwrap()
            .1
            .split_once("/// Affine admitted-local continuation")
            .unwrap()
            .0;
        assert!(external.contains("pub(crate) const fn import_id"));
        assert!(external.contains("pub(crate) fn retry"));
        assert!(!external.contains("LocalMutationCoordinatorState"));

        let local = production
            .split_once("impl LocalPublishedContinuation {")
            .unwrap()
            .1
            .split_once("pub(crate) struct OperationalCoordinator")
            .unwrap()
            .0;
        assert!(local.contains("pub(crate) fn retry"));
        assert!(!local.contains("import_id"));
        assert!(!local.contains("OperationalCoordinatorState"));
    }

    #[test]
    fn local_continuation_drop_stays_closed_and_completion_releases_once() {
        let mut dropped = Fixture::new("drop-local-published");
        let releases = dropped.graph.handoff_release_count();
        fail_once_at(OperationalFaultPoint::AfterManifest);
        let continuation =
            expect_local_published_recovery(dropped.local_edit(43_500, "drop local continuation"));
        drop(continuation);
        assert_eq!(dropped.graph.handoff_release_count(), releases);
        assert!(dropped.graph.probe_managed_text_writer().is_err());

        let mut completed = Fixture::new("complete-local-once");
        let releases = completed.graph.handoff_release_count();
        expect_local_active(completed.local_edit(43_510, "complete local once"));
        assert_eq!(completed.graph.handoff_release_count(), releases + 1);
        completed.graph.probe_managed_text_writer().unwrap();
        completed.graph.probe_managed_text_writer().unwrap();
        assert_eq!(completed.graph.handoff_release_count(), releases + 1);
    }

    #[test]
    fn retained_terminal_dispositions_are_blocked_while_progress_is_recovering() {
        let cases = [
            (
                "rejected",
                BatchDisposition::Rejected {
                    error: super::super::EngineError::AuthorDraftStale,
                },
                Some(RetainedBlockReason::Rejected(
                    super::super::EngineError::AuthorDraftStale,
                )),
            ),
            (
                "quarantined",
                BatchDisposition::Quarantined,
                Some(RetainedBlockReason::Quarantined),
            ),
            (
                "bounded",
                BatchDisposition::IncompleteStaged {
                    missing_objects: 0,
                    missing_dependencies: Vec::new(),
                },
                None,
            ),
        ];
        for (index, (label, disposition, expected_block)) in cases.into_iter().enumerate() {
            let mut fixture = Fixture::new(&format!("retained-{label}"));
            fail_once_at(OperationalFaultPoint::AfterManifest);
            let mut continuation = expect_local_published_recovery(
                fixture.local_edit(43_600 + index as u128 * 10, "retained classification"),
            );
            let manifests = fixture.archive.committed_manifests().unwrap().len();
            continuation.core.failure =
                require_accepted_stage_disposition(continuation.batch_id(), &disposition)
                    .expect_err("the synthetic final/progress disposition must retain work");
            let state = LocalMutationCoordinatorState::from_failed(continuation);
            match expected_block {
                Some(reason) => {
                    let LocalMutationCoordinatorState::Blocked(blocked) = state else {
                        panic!("{label} must be a retained typed Blocked outcome");
                    };
                    assert_eq!(
                        blocked.reason(),
                        &LocalMutationBlockReason::Retained(reason)
                    );
                    assert!(blocked.continuation().is_some());
                }
                None => {
                    let LocalMutationCoordinatorState::Recovering(
                        LocalMutationRecovery::Published(continuation),
                    ) = state
                    else {
                        panic!("bounded staging work must remain Recovering");
                    };
                    assert_eq!(continuation.phase(), OperationalPhase::ArchiveStage);
                }
            }
            assert_eq!(
                fixture.archive.committed_manifests().unwrap().len(),
                manifests,
                "classification must not redraft or republish"
            );
        }

        let mut fixture = Fixture::new("retained-authentication");
        fail_once_at(OperationalFaultPoint::AfterManifest);
        let mut continuation =
            expect_local_published_recovery(fixture.local_edit(43_700, "stable auth"));
        continuation.core.failure = OperationalCoordinatorError::retained_block(
            OperationalPhase::Publication,
            "stable immutable authentication mismatch",
            RetainedBlockReason::PublishedAuthentication,
        );
        let LocalMutationCoordinatorState::Blocked(blocked) =
            LocalMutationCoordinatorState::from_failed(continuation)
        else {
            panic!("stable authentication failure must be retained Blocked");
        };
        assert_eq!(
            blocked.reason(),
            &LocalMutationBlockReason::Retained(RetainedBlockReason::PublishedAuthentication)
        );
        assert!(blocked.continuation().is_some());
    }

    #[test]
    fn rejected_published_local_batch_retains_typed_blocked_evidence() {
        let mut fixture = Fixture::new("published-rejected-blocked");
        let accepted_before = fixture.engine.accepted_frontier_root().unwrap();
        let sqlite_before = fixture.database.frontier_root().unwrap();
        let manifests_before = fixture.archive.committed_manifests().unwrap().len();
        act_once_at(OperationalFaultPoint::AfterManifest, || {
            fail_next_engine_history_head_swap();
        });
        let LocalMutationCoordinatorState::Blocked(blocked) =
            fixture.local_edit(43_800, "history head rejection")
        else {
            panic!("durable history rejection must return retained Blocked");
        };
        assert!(matches!(
            blocked.reason(),
            LocalMutationBlockReason::Retained(RetainedBlockReason::Rejected(_))
        ));
        let continuation = blocked
            .continuation()
            .expect("the rejected immutable batch retains its continuation/evidence");
        assert_eq!(continuation.phase(), OperationalPhase::ArchiveStage);
        assert_eq!(
            fixture.archive.committed_manifests().unwrap().len(),
            manifests_before + 1
        );
        assert_eq!(
            fixture.engine.accepted_frontier_root().unwrap(),
            accepted_before
        );
        assert_eq!(fixture.database.frontier_root().unwrap(), sqlite_before);
        assert!(fixture.graph.probe_managed_text_writer().is_err());
    }

    #[test]
    fn stable_postpublication_binding_failure_retains_typed_blocked_continuation() {
        let mut fixture = Fixture::new("local-stable-postpublication-binding");
        fail_once_at(OperationalFaultPoint::AfterManifest);
        let continuation =
            expect_local_published_recovery(fixture.local_edit(43_900, "stable binding"));
        let foreign_root = TestRoot::new("local-stable-postpublication-binding-foreign");
        let foreign_graph_root = foreign_root.path().join("graph");
        fs::create_dir_all(&foreign_graph_root).unwrap();
        let foreign_graph = Graph::open(&foreign_graph_root);

        let LocalMutationCoordinatorState::Blocked(blocked) = continuation.retry(
            &LocalRuntimeAdmission::unenrolled_pre_activation(),
            &foreign_graph,
            &fixture.receipts,
            &mut fixture.engine,
            &mut fixture.database,
            &mut fixture.tail,
        ) else {
            panic!("stable rebound graph authentication must return retained Blocked");
        };
        assert_eq!(
            blocked.reason(),
            &LocalMutationBlockReason::Retained(RetainedBlockReason::StableBinding)
        );
        assert!(blocked.continuation().is_some());
        assert!(fixture.graph.probe_managed_text_writer().is_err());
    }

    #[test]
    fn blocked_and_noop_cancel_without_durable_or_derived_mutation() {
        let mut fixture = Fixture::new("blocked-noop");
        let accepted = fixture.engine.accepted_frontier_root().unwrap();
        let sqlite = fixture.database.frontier_root().unwrap();
        let tail = fixture.tail.status();
        let graph = fs::read(fixture.graph_root.join(&fixture.path)).unwrap();
        let archive = snapshot_tree(&fixture.archive_root);
        let receipts = snapshot_tree(fixture.receipts.root_path());

        assert!(matches!(
            fixture.execute(&["../escape.md"]),
            OperationalCoordinatorState::Blocked(_)
        ));
        let path = fixture.path.clone();
        assert!(matches!(
            fixture.execute(&[&path]),
            OperationalCoordinatorState::Noop
        ));
        assert_eq!(fixture.engine.accepted_frontier_root().unwrap(), accepted);
        assert_eq!(fixture.database.frontier_root().unwrap(), sqlite);
        assert_eq!(fixture.tail.status(), tail);
        assert_eq!(
            fs::read(fixture.graph_root.join(&fixture.path)).unwrap(),
            graph
        );
        assert_eq!(snapshot_tree(&fixture.archive_root), archive);
        assert_eq!(snapshot_tree(fixture.receipts.root_path()), receipts);
        fixture.graph.probe_managed_text_writer().unwrap();
    }

    #[test]
    fn formatting_only_noop_adopts_exact_bytes_without_a_semantic_batch() {
        let mut fixture = Fixture::formatting_only("formatting-only-noop");
        let path = fixture.path.clone();
        let formatted = b"- root\r\n\r\n\t- child\r\n";
        fixture.overwrite(formatted);
        let accepted = fixture.engine.accepted_frontier_root().unwrap();
        let sqlite = fixture.database.frontier_root().unwrap();
        let manifests = fixture.archive.committed_manifests().unwrap();
        let receipts_before = snapshot_tree(fixture.receipts.root_path());
        let plan =
            plan_affected_import(&fixture.graph, &fixture.receipts, &fixture.engine, &[&path]);
        assert_eq!(plan.status(), ImportPlanStatus::Noop, "{plan:?}");

        assert!(matches!(
            fixture.execute(&[&path]),
            OperationalCoordinatorState::Noop
        ));
        assert_eq!(fixture.engine.accepted_frontier_root().unwrap(), accepted);
        assert_eq!(fixture.database.frontier_root().unwrap(), sqlite);
        assert_eq!(fixture.archive.committed_manifests().unwrap(), manifests);
        assert_ne!(
            snapshot_tree(fixture.receipts.root_path()),
            receipts_before,
            "the endpoint-local exact baseline must advance"
        );
        assert_eq!(fs::read(fixture.graph_root.join(&path)).unwrap(), formatted);

        expect_local_active(fixture.local_edit(49_000, "root edited"));
        fixture.assert_drained();
        assert_eq!(
            fs::read(fixture.graph_root.join(&path)).unwrap(),
            b"- root edited\r\n\r\n\t- child\r\n",
            "the next real semantic edit must render from the adopted formatting baseline"
        );
    }

    #[test]
    fn formatting_only_intent_recovers_after_restart_before_the_next_real_edit() {
        let mut fixture = Fixture::formatting_only("formatting-only-restart");
        let path = fixture.path.clone();
        let formatted = b"- root\r\n\r\n\t- child\r\n";
        fixture.overwrite(formatted);
        let manifests = fixture.archive.committed_manifests().unwrap();
        fail_next_formatting_adoption_after_intent_for_harness();
        let failed = match OperationalCoordinator::execute(
            &LocalRuntimeAdmission::unenrolled_pre_activation(),
            &fixture.graph,
            &fixture.receipts,
            &mut fixture.engine,
            &mut fixture.database,
            &mut fixture.tail,
            &[&path],
        ) {
            Err(error) => error,
            Ok(_) => panic!("formatting adoption fault unexpectedly completed"),
        };
        assert_eq!(failed.phase(), OperationalPhase::Planning);
        assert_eq!(fixture.archive.committed_manifests().unwrap(), manifests);
        assert_eq!(fs::read(fixture.graph_root.join(&path)).unwrap(), formatted);

        fixture = fixture.restart_projection_runtime();
        let recovered =
            recover_incomplete_projections(&fixture.graph, &fixture.receipts, &fixture.engine)
                .unwrap();
        assert_eq!(recovered.len(), 1);
        assert!(matches!(
            fixture.execute(&[&path]),
            OperationalCoordinatorState::Noop
        ));
        expect_local_active(fixture.local_edit(49_100, "after restart"));
        assert_eq!(
            fs::read(fixture.graph_root.join(&path)).unwrap(),
            b"- after restart\r\n\r\n\t- child\r\n"
        );
    }

    #[test]
    fn every_pre_manifest_boundary_releases_and_allows_fresh_retry() {
        let cases = [
            OperationalFaultPoint::AfterHandoff,
            OperationalFaultPoint::AfterPlan,
            OperationalFaultPoint::AfterDraft,
            OperationalFaultPoint::AfterCapture,
            OperationalFaultPoint::AfterFinalize,
            OperationalFaultPoint::AfterReservation,
        ];
        for (index, point) in cases.into_iter().enumerate() {
            let mut fixture = Fixture::new(&format!("pre-manifest-{index}"));
            let path = fixture.path.clone();
            fixture.overwrite(b"- changed\n\t- still nested\n");
            let accepted = fixture.engine.accepted_frontier_root().unwrap();
            let sqlite = fixture.database.frontier_root().unwrap();
            fail_once_at(point);
            assert!(OperationalCoordinator::execute(
                &LocalRuntimeAdmission::unenrolled_pre_activation(),
                &fixture.graph,
                &fixture.receipts,
                &mut fixture.engine,
                &mut fixture.database,
                &mut fixture.tail,
                &[&path],
            )
            .is_err());
            assert_eq!(fixture.engine.accepted_frontier_root().unwrap(), accepted);
            assert_eq!(fixture.database.frontier_root().unwrap(), sqlite);
            assert_eq!(fixture.tail.status().unapplied_batches, 0);
            fixture.graph.probe_managed_text_writer().unwrap();
            expect_complete(fixture.execute(&[&path]));
            fixture.assert_drained();
        }
    }

    #[test]
    fn stale_observation_and_receipt_capture_reject_before_publication() {
        let mut observation = Fixture::new("stale-observation");
        let path = observation.path.clone();
        observation.overwrite(b"- first external edit\n");
        let target = observation.graph_root.join(&path);
        act_once_at(OperationalFaultPoint::AfterPlan, move || {
            fs::write(target, b"- replacement during draft\n").unwrap();
        });
        assert!(matches!(
            OperationalCoordinator::execute(
                &LocalRuntimeAdmission::unenrolled_pre_activation(),
                &observation.graph,
                &observation.receipts,
                &mut observation.engine,
                &mut observation.database,
                &mut observation.tail,
                &[&path],
            ),
            Err(OperationalCoordinatorError {
                phase: OperationalPhase::Capture,
                ..
            })
        ));
        observation.graph.probe_managed_text_writer().unwrap();
        expect_complete(observation.execute(&[&path]));
        observation.assert_drained();

        let mut receipt = Fixture::new("stale-receipt");
        let path = receipt.path.clone();
        receipt.overwrite(b"- receipt edit\n");
        let completion = receipt
            .receipts
            .root_path()
            .join("completions")
            .join(format!(
                "{}.completion",
                hex(receipt.intent.id().unwrap().as_bytes())
            ));
        let held = completion.with_extension("completion.held");
        let move_from = completion.clone();
        let move_to = held.clone();
        act_once_at(OperationalFaultPoint::AfterCapture, move || {
            fs::rename(move_from, move_to).unwrap();
        });
        assert!(matches!(
            OperationalCoordinator::execute(
                &LocalRuntimeAdmission::unenrolled_pre_activation(),
                &receipt.graph,
                &receipt.receipts,
                &mut receipt.engine,
                &mut receipt.database,
                &mut receipt.tail,
                &[&path],
            ),
            Err(OperationalCoordinatorError {
                phase: OperationalPhase::Finalize,
                ..
            })
        ));
        fs::rename(held, completion).unwrap();
        receipt.graph.probe_managed_text_writer().unwrap();
        expect_complete(receipt.execute(&[&path]));
        receipt.assert_drained();
    }

    #[test]
    fn exact_reservation_precedes_manifest_and_object_only_cut_has_no_semantic_effect() {
        let mut pressured = Fixture::new("reservation-first");
        let path = pressured.path.clone();
        pressured.overwrite(b"- pressure edit\n");
        let filler = pressured.tail.reserve_mutation(TAIL_MAX_BYTES).unwrap();
        let accepted = pressured.engine.accepted_frontier_root().unwrap();
        let result = OperationalCoordinator::execute(
            &LocalRuntimeAdmission::unenrolled_pre_activation(),
            &pressured.graph,
            &pressured.receipts,
            &mut pressured.engine,
            &mut pressured.database,
            &mut pressured.tail,
            &[&path],
        );
        assert!(matches!(
            result,
            Err(OperationalCoordinatorError {
                phase: OperationalPhase::TailReservation,
                ..
            })
        ));
        assert_eq!(pressured.engine.accepted_frontier_root().unwrap(), accepted);
        pressured.tail.cancel_reservation(filler).unwrap();
        pressured.graph.probe_managed_text_writer().unwrap();

        let mut objects = Fixture::new("objects-only");
        let path = objects.path.clone();
        objects.overwrite(b"- objects only\n");
        let accepted = objects.engine.accepted_frontier_root().unwrap();
        let sqlite = objects.database.frontier_root().unwrap();
        let before = snapshot_tree(&objects.archive_root);
        fail_next_publish_after_objects();
        assert!(matches!(
            OperationalCoordinator::execute(
                &LocalRuntimeAdmission::unenrolled_pre_activation(),
                &objects.graph,
                &objects.receipts,
                &mut objects.engine,
                &mut objects.database,
                &mut objects.tail,
                &[&path],
            ),
            Err(OperationalCoordinatorError {
                phase: OperationalPhase::Publication,
                ..
            })
        ));
        assert_eq!(objects.engine.accepted_frontier_root().unwrap(), accepted);
        assert_eq!(objects.database.frontier_root().unwrap(), sqlite);
        assert_ne!(snapshot_tree(&objects.archive_root), before);
        objects.graph.probe_managed_text_writer().unwrap();
        expect_complete(objects.execute(&[&path]));
        objects.assert_drained();
    }

    #[test]
    fn every_post_manifest_failure_retains_guard_and_retries_idempotently() {
        let cases = [
            OperationalFaultPoint::AfterManifest,
            OperationalFaultPoint::AfterStage,
            OperationalFaultPoint::AfterTailAdmission,
            OperationalFaultPoint::AfterSqliteApply,
            OperationalFaultPoint::BeforeProjection,
            OperationalFaultPoint::AfterProjection,
        ];
        for (index, point) in cases.into_iter().enumerate() {
            let mut fixture = Fixture::new(&format!("post-manifest-{index}"));
            let path = fixture.path.clone();
            fixture.overwrite(b"- durable edit\n\t- nested durable edit\n");
            fail_once_at(point);
            let failed = expect_failed(fixture.execute(&[&path]));
            assert_eq!(failed.batch_id(), failed.import_id().batch_id());
            assert!(fixture.graph.probe_managed_text_writer().is_err());
            if point == OperationalFaultPoint::AfterManifest {
                assert_eq!(
                    fixture.tail.status().retained_bytes,
                    failed.retained_bytes()
                );
            }
            if matches!(
                point,
                OperationalFaultPoint::BeforeProjection | OperationalFaultPoint::AfterProjection
            ) {
                assert_eq!(
                    fixture.database.frontier_root().unwrap(),
                    fixture.engine.accepted_frontier_root().unwrap(),
                    "projection faults are reachable only after exact SQLite catch-up"
                );
            }
            let completion = expect_complete(failed.retry(
                &LocalRuntimeAdmission::unenrolled_pre_activation(),
                &fixture.graph,
                &fixture.receipts,
                &mut fixture.engine,
                &mut fixture.database,
                &mut fixture.tail,
            ));
            assert_eq!(completion.batch_id(), failed_batch_id(&completion));
            fixture.graph.probe_managed_text_writer().unwrap();
            fixture.assert_drained();
            assert_eq!(
                fs::read(fixture.graph_root.join(path)).unwrap(),
                b"- durable edit\n\t- nested durable edit\n"
            );
        }
    }

    #[test]
    fn sqlite_budget_boundary_retains_handoff_and_resumes_without_republication() {
        const PREEXISTING: usize = 20;
        let mut fixture = Fixture::new("bounded-sqlite-resume");
        for index in 0..PREEXISTING {
            let transaction = if index == 0 {
                OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
                    block: BlockLocation {
                        block_id: fixture.block_id,
                        home_document_id: fixture.home_document_id,
                    },
                    content: "durable accepted tail base".into(),
                }])
                .unwrap()
            } else {
                OperationTransaction::new(vec![SemanticOperation::CreatePage {
                    page_id: PageId::from_uuid(Uuid::from_u128(64_000 + index as u128)),
                    home_document_id: DocumentId::from_uuid(Uuid::from_u128(
                        65_000 + index as u128,
                    )),
                    name: LogicalPageName::parse(&format!("Bounded Tail {index}")).unwrap(),
                    path: ManagedPath::parse(&format!("pages/bounded-tail-{index}.md")).unwrap(),
                    kind: ManagedTextKind::Page,
                }])
                .unwrap()
            };
            let prepared = fixture
                .engine
                .prepare_bootstrap_transaction(
                    AuthorBatch {
                        batch_id: BatchId::from_uuid(Uuid::from_u128(60_000 + index as u128)),
                        author_device_id: DeviceId::from_uuid(Uuid::from_u128(
                            61_000 + index as u128,
                        )),
                        author_session_id: SessionId::from_uuid(Uuid::from_u128(
                            62_000 + index as u128,
                        )),
                        crdt_peer_id: CrdtPeerId::from_u64(63_000 + index as u64),
                    },
                    &transaction,
                )
                .unwrap();
            let batch_id = prepared.manifest().batch_id();
            fixture
                .archive
                .publish_bootstrap_prepared_for_test(&prepared)
                .unwrap();
            assert!(matches!(
                fixture
                    .engine
                    .stage_archive_batch(batch_id)
                    .unwrap()
                    .disposition(),
                BatchDisposition::Accepted { .. }
            ));
            let event =
                AcceptedBatchEvent::from_accepted(&fixture.engine, &fixture.archive, batch_id)
                    .unwrap();
            fixture
                .tail
                .try_enqueue(&mut fixture.database, &fixture.engine, &event)
                .unwrap();
        }
        let current = fs::read(fixture.graph_root.join(&fixture.path)).unwrap();
        fixture.intent = write_projection_exact(
            &fixture.graph,
            &fixture.receipts,
            &fixture.engine,
            PageId::from_uuid(Uuid::from_u128(5)),
            Some(&current),
        )
        .unwrap()
        .plan
        .intent()
        .clone();
        assert_eq!(fixture.tail.status().unapplied_batches, PREEXISTING);

        let path = fixture.path.clone();
        fixture.overwrite(b"- bounded coordinator drain\n");
        let releases_before = fixture.graph.handoff_release_count();
        let mut failed = expect_failed(fixture.execute(&[&path]));
        let batch_id = failed.batch_id();
        assert_eq!(failed.batch_id(), failed.import_id().batch_id());
        assert!(fixture.graph.probe_managed_text_writer().is_err());
        // The immutable publication is complete and byte-frozen from here on.
        let published = snapshot_immutable_publication(&fixture.archive_root);
        let published_count = fixture.archive.committed_manifests().unwrap().len();
        assert_eq!(published_count, PREEXISTING + 2);

        // Exact per-resume accounting: phase, remaining backlog, retained
        // publication bytes, and handoff release count.
        let mut phases = vec![failed.phase()];
        let mut backlog = vec![fixture.tail.status().unapplied_batches];
        let completion = loop {
            // Nothing about the immutable publication, the retained latch, or
            // the release counter may move on a failed retry.
            assert_eq!(
                snapshot_immutable_publication(&fixture.archive_root),
                published,
                "a failed retry republished, mutated, or left residue in the immutable archive"
            );
            assert_eq!(
                fixture.archive.committed_manifests().unwrap().len(),
                published_count
            );
            assert_eq!(
                fixture.graph.handoff_release_count(),
                releases_before,
                "a failed retry released the retained managed-text handoff"
            );
            assert!(fixture.graph.probe_managed_text_writer().is_err());
            match failed.retry(
                &LocalRuntimeAdmission::unenrolled_pre_activation(),
                &fixture.graph,
                &fixture.receipts,
                &mut fixture.engine,
                &mut fixture.database,
                &mut fixture.tail,
            ) {
                OperationalCoordinatorState::Complete(completion) => break completion,
                OperationalCoordinatorState::FailedClosed(next) => {
                    failed = next;
                    phases.push(failed.phase());
                    backlog.push(fixture.tail.status().unapplied_batches);
                    assert!(phases.len() <= PREEXISTING + 4);
                }
                OperationalCoordinatorState::Blocked(_) | OperationalCoordinatorState::Noop => {
                    panic!("published bounded retry changed semantic state")
                }
            }
        };
        assert_eq!(completion.batch_id(), batch_id);
        // Weighted parent-clock and whole-batch prepayment may require several
        // ArchiveStage continuations before the published event can enter the
        // tail. Those retries consume only their own staging budget and retain
        // the latch. Once staging completes, the unchanged SQLite arithmetic
        // applies a strict bounded prefix and reports its durable remainder.
        assert_eq!(phases.last(), Some(&OperationalPhase::SqliteDrain));
        assert!(phases[..phases.len() - 1]
            .iter()
            .all(|phase| *phase == OperationalPhase::ArchiveStage));
        let sqlite_remainder = *backlog.last().unwrap();
        assert!(
            sqlite_remainder > 0 && sqlite_remainder < PREEXISTING + 1,
            "SQLite must apply a nonempty strict prefix under the remaining resume budget: {backlog:?}"
        );

        // Completion is the only release, and it happens exactly once.
        assert_eq!(fixture.graph.handoff_release_count(), releases_before + 1);
        assert_eq!(
            snapshot_immutable_publication(&fixture.archive_root),
            published,
            "completion republished or left residue in the immutable archive"
        );
        assert_eq!(
            fixture.engine.accepted_batch_count().unwrap(),
            u64::try_from(PREEXISTING + 2).unwrap()
        );
        fixture.graph.probe_managed_text_writer().unwrap();
        fixture.graph.probe_managed_text_writer().unwrap();
        assert_eq!(fixture.graph.handoff_release_count(), releases_before + 1);
        fixture.assert_drained();
    }

    #[test]
    fn retained_continuation_authenticates_the_archive_by_stable_resource_identity() {
        let fixture = Fixture::new("archive-identity");
        let workspace = fixture.engine.workspace_id();
        let endpoint = fixture.engine.projection_endpoint_binding().unwrap();

        // A separately opened handle to the exact enrolled archive resource is
        // accepted, so a same-process engine reconstruction does not have to
        // preserve `Arc` pointer identity to resume a published continuation.
        let reopened = Arc::new(ObjectStore::open(&fixture.archive_root, workspace).unwrap());
        verify_bindings(
            &fixture.graph,
            &fixture.receipts,
            &fixture.engine,
            endpoint,
            Some(&reopened),
        )
        .unwrap();

        // A different archive directory carrying the same workspace identity is
        // still rejected, so the relaxation does not weaken authentication.
        let foreign_root = TestRoot::new("archive-identity-foreign");
        let foreign =
            Arc::new(ObjectStore::open(&foreign_root.path().join("archive"), workspace).unwrap());
        let rejected = verify_bindings(
            &fixture.graph,
            &fixture.receipts,
            &fixture.engine,
            endpoint,
            Some(&foreign),
        )
        .expect_err("a substituted archive resource must not authenticate");
        assert_eq!(rejected.phase(), OperationalPhase::Bindings);
        assert!(rejected.detail().contains("archive resource identity"));
        fixture.graph.probe_managed_text_writer().unwrap();
    }

    #[test]
    fn published_continuation_survives_same_process_engine_reconstruction() {
        let mut fixture = Fixture::new("engine-reconstruction");
        let path = fixture.path.clone();
        fixture.overwrite(b"- reconstructed continuation\n\t- nested\n");
        let releases = fixture.graph.handoff_release_count();
        fail_once_at(OperationalFaultPoint::AfterManifest);
        let failed = expect_failed(fixture.execute(&[&path]));
        assert!(fixture.graph.probe_managed_text_writer().is_err());

        // Discard every run-local derived engine structure. Only the retained
        // capabilities and their authenticated durable roots survive.
        fixture.engine.reconstruct_run_local_state().unwrap();
        assert_eq!(fixture.graph.handoff_release_count(), releases);

        let completion = expect_complete(failed.retry(
            &LocalRuntimeAdmission::unenrolled_pre_activation(),
            &fixture.graph,
            &fixture.receipts,
            &mut fixture.engine,
            &mut fixture.database,
            &mut fixture.tail,
        ));
        assert_eq!(completion.batch_id(), completion.import_id().batch_id());
        assert_eq!(fixture.graph.handoff_release_count(), releases + 1);
        fixture.graph.probe_managed_text_writer().unwrap();
        fixture.assert_drained();
        assert_eq!(
            fs::read(fixture.graph_root.join(path)).unwrap(),
            b"- reconstructed continuation\n\t- nested\n"
        );
    }

    #[test]
    fn dropping_published_continuation_stays_closed_and_completion_releases_once() {
        let mut dropped = Fixture::new("drop-published");
        let path = dropped.path.clone();
        dropped.overwrite(b"- durable dropped continuation\n");
        fail_once_at(OperationalFaultPoint::AfterManifest);
        let failed = expect_failed(dropped.execute(&[&path]));
        let releases = dropped.graph.handoff_release_count();
        drop(failed);
        assert_eq!(dropped.graph.handoff_release_count(), releases);
        assert!(dropped.graph.probe_managed_text_writer().is_err());

        let mut completed = Fixture::new("complete-once");
        let path = completed.path.clone();
        completed.overwrite(b"- successful explicit completion\n");
        let releases = completed.graph.handoff_release_count();
        expect_complete(completed.execute(&[&path]));
        assert_eq!(completed.graph.handoff_release_count(), releases + 1);
        completed.graph.probe_managed_text_writer().unwrap();
        completed.graph.probe_managed_text_writer().unwrap();
        assert_eq!(completed.graph.handoff_release_count(), releases + 1);
    }

    #[test]
    fn manifested_preconditions_are_exact_fresh_external_observations() {
        let mut edit = Fixture::new("observed-precondition-edit");
        let path = edit.path.clone();
        let prior = fs::read(edit.graph_root.join(&path)).unwrap();
        let observed = b"- externally changed bytes\n\t- current annotation source\n".to_vec();
        edit.overwrite(&observed);
        let completion = expect_complete(edit.execute(&[&path]));
        let batch = match edit.archive.inspect_batch(completion.batch_id()).unwrap() {
            BatchInspection::Ready(batch) => batch,
            other => panic!("completed batch is not immutable Ready: {other:?}"),
        };
        let intent = batch
            .objects()
            .iter()
            .find(|object| object.kind() == ObjectKind::ProjectionIntent)
            .map(|object| ManifestedProjectionIntent::decode(object.payload()).unwrap())
            .expect("edit carries manifested projection intent");
        let ManifestProjectionPrecondition::Present { base } = intent.precondition() else {
            panic!("fresh present edit must manifest Present");
        };
        let manifested_base = batch
            .objects()
            .iter()
            .find(|object| {
                object.kind() == ObjectKind::AnnotatedBaseBlob
                    && object.document_id() == base.document_id()
            })
            .map(|object| AnnotatedProjectionBase::decode(object.payload()).unwrap())
            .expect("manifested observed base exists");
        assert_eq!(manifested_base.bytes(), observed);
        assert_ne!(manifested_base.bytes(), prior);

        let mut absent = Fixture::new("observed-precondition-absent");
        let path = absent.path.clone();
        fs::remove_file(absent.graph_root.join(&path)).unwrap();
        let completion = expect_complete(absent.execute(&[&path]));
        let batch = match absent.archive.inspect_batch(completion.batch_id()).unwrap() {
            BatchInspection::Ready(batch) => batch,
            other => panic!("completed batch is not immutable Ready: {other:?}"),
        };
        let intent = batch
            .objects()
            .iter()
            .find(|object| object.kind() == ObjectKind::ProjectionIntent)
            .map(|object| ManifestedProjectionIntent::decode(object.payload()).unwrap())
            .expect("delete carries manifested projection intent");
        assert!(matches!(
            intent.precondition(),
            ManifestProjectionPrecondition::Absent
        ));
    }

    #[test]
    fn crdt_peer_probe_is_bounded_for_zero_collision_and_exhaustion() {
        let fixture = Fixture::new("peer-probe");
        let path = fixture.path.clone();
        fixture.overwrite(b"- peer probe edit\n");
        let plan =
            plan_affected_import(&fixture.graph, &fixture.receipts, &fixture.engine, &[&path]);
        let material = plan.into_execution_material().unwrap();
        let endpoint = fixture.engine.projection_endpoint_binding().unwrap();
        let candidates = [0, 12, 13];
        let (author, _) =
            draft_with_bounded_peer_candidates(&fixture.engine, endpoint, &material, |attempt| {
                CrdtPeerId::from_u64(candidates[usize::try_from(attempt).unwrap().min(2)])
            })
            .unwrap();
        assert_eq!(author.crdt_peer_id, CrdtPeerId::from_u64(13));

        let exhausted =
            match draft_with_bounded_peer_candidates(&fixture.engine, endpoint, &material, |_| {
                CrdtPeerId::from_u64(12)
            }) {
                Err(error) => error,
                Ok(_) => panic!("colliding bounded peer probe unexpectedly succeeded"),
            };
        assert_eq!(exhausted.phase(), OperationalPhase::Draft);
        assert!(exhausted.detail().contains("bounded 8-candidate probe"));
        fixture.graph.probe_managed_text_writer().unwrap();
    }

    fn failed_batch_id(completion: &OperationalCompletion) -> BatchId {
        completion.import_id().batch_id()
    }

    #[test]
    fn delete_and_rename_project_exact_old_removal_and_new_render_base() {
        let mut deletion = Fixture::new("delete");
        let delete_path = deletion.path.clone();
        fs::remove_file(deletion.graph_root.join(&delete_path)).unwrap();
        expect_complete(deletion.execute(&[&delete_path]));
        deletion.assert_drained();
        assert!(!deletion.graph_root.join(delete_path).exists());

        let mut rename = Fixture::new("rename");
        let old = rename.path.clone();
        let new = "pages/elsewhere/deeper/renamed.md";
        fs::create_dir_all(rename.graph_root.join(new).parent().unwrap()).unwrap();
        fs::rename(rename.graph_root.join(&old), rename.graph_root.join(new)).unwrap();
        let completion = expect_complete(rename.execute(&[&old, new]));
        rename.assert_drained();
        assert!(!rename.graph_root.join(&old).exists());
        assert_eq!(
            fs::read(rename.graph_root.join(new)).unwrap(),
            b"- root\n\t- child\n"
        );
        let batch = match rename.archive.inspect_batch(completion.batch_id()).unwrap() {
            BatchInspection::Ready(batch) => batch,
            other => panic!("completed rename batch is not Ready: {other:?}"),
        };
        let intents = batch
            .objects()
            .iter()
            .filter(|object| object.kind() == ObjectKind::ProjectionIntent)
            .map(|object| ManifestedProjectionIntent::decode(object.payload()).unwrap())
            .collect::<Vec<_>>();
        let old_intent = intents
            .iter()
            .find(|intent| intent.path().as_str() == old)
            .expect("rename carries old-path removal");
        assert!(matches!(
            old_intent.precondition(),
            ManifestProjectionPrecondition::Absent
        ));
        let new_intent = intents
            .iter()
            .find(|intent| intent.path().as_str() == new)
            .expect("rename carries new-path projection");
        let ManifestProjectionPrecondition::Present { base } = new_intent.precondition() else {
            panic!("fresh rename destination is present");
        };
        let base = batch
            .objects()
            .iter()
            .find(|object| {
                object.kind() == ObjectKind::AnnotatedBaseBlob
                    && object.document_id() == base.document_id()
            })
            .map(|object| AnnotatedProjectionBase::decode(object.payload()).unwrap())
            .expect("rename destination observed base exists");
        assert_eq!(base.bytes(), b"- root\n\t- child\n");
    }

    #[test]
    fn binding_mismatch_rejects_before_handoff_or_publication() {
        let mut fixture = Fixture::new("binding");
        let foreign_root = TestRoot::new("foreign-receipts");
        let foreign_graph_root = foreign_root.path().join("graph");
        fs::create_dir_all(&foreign_graph_root).unwrap();
        let foreign_graph = Graph::open(&foreign_graph_root);
        let foreign_endpoint = ProjectionEndpointBinding::enroll_graph(
            &foreign_graph,
            ProjectionEndpointId::from_uuid(Uuid::from_u128(900)),
            DeviceId::from_uuid(Uuid::from_u128(901)),
        )
        .unwrap();
        let foreign = ProjectionReceiptStore::open_for_endpoint(
            &foreign_root.path().join("receipts"),
            fixture.engine.workspace_id(),
            foreign_endpoint,
        )
        .unwrap();
        let path = fixture.path.clone();
        fixture.overwrite(b"- rejected binding\n");
        assert!(matches!(
            OperationalCoordinator::execute(
                &LocalRuntimeAdmission::unenrolled_pre_activation(),
                &fixture.graph,
                &foreign,
                &mut fixture.engine,
                &mut fixture.database,
                &mut fixture.tail,
                &[&path],
            ),
            Err(OperationalCoordinatorError {
                phase: OperationalPhase::Bindings,
                ..
            })
        ));
        fixture.graph.probe_managed_text_writer().unwrap();
    }

    #[test]
    fn post_manifest_retry_rejects_rebound_graph_and_keeps_original_guard() {
        let mut fixture = Fixture::new("retry-binding");
        let path = fixture.path.clone();
        fixture.overwrite(b"- durable retry binding\n");
        fail_once_at(OperationalFaultPoint::AfterManifest);
        let failed = expect_failed(fixture.execute(&[&path]));

        let foreign_root = TestRoot::new("retry-binding-foreign");
        let foreign_graph_root = foreign_root.path().join("graph");
        fs::create_dir_all(&foreign_graph_root).unwrap();
        let foreign_graph = Graph::open(&foreign_graph_root);
        let failed = expect_failed(failed.retry(
            &LocalRuntimeAdmission::unenrolled_pre_activation(),
            &foreign_graph,
            &fixture.receipts,
            &mut fixture.engine,
            &mut fixture.database,
            &mut fixture.tail,
        ));
        assert_eq!(failed.phase(), OperationalPhase::Bindings);
        assert!(fixture.graph.probe_managed_text_writer().is_err());
        expect_complete(failed.retry(
            &LocalRuntimeAdmission::unenrolled_pre_activation(),
            &fixture.graph,
            &fixture.receipts,
            &mut fixture.engine,
            &mut fixture.database,
            &mut fixture.tail,
        ));
        fixture.graph.probe_managed_text_writer().unwrap();
        fixture.assert_drained();
    }

    #[test]
    fn reordered_batch_ids_drain_by_authenticated_acceptance_sequence() {
        let mut fixture = Fixture::new("acceptance-sequence");
        let first = fixture
            .engine
            .prepare_bootstrap_transaction(
                AuthorBatch {
                    batch_id: BatchId::from_uuid(Uuid::from_u128(u128::MAX - 1)),
                    author_device_id: DeviceId::from_uuid(Uuid::from_u128(700)),
                    author_session_id: SessionId::from_uuid(Uuid::from_u128(701)),
                    crdt_peer_id: CrdtPeerId::from_u64(702),
                },
                &OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
                    block: BlockLocation {
                        block_id: fixture.block_id,
                        home_document_id: fixture.home_document_id,
                    },
                    content: "first accepted".into(),
                }])
                .unwrap(),
            )
            .unwrap();
        fixture
            .archive
            .publish_bootstrap_prepared_for_test(&first)
            .unwrap();
        fixture
            .engine
            .stage_archive_batch(first.manifest().batch_id())
            .unwrap();
        let second = fixture
            .engine
            .prepare_bootstrap_transaction(
                AuthorBatch {
                    batch_id: BatchId::from_uuid(Uuid::from_u128(20)),
                    author_device_id: DeviceId::from_uuid(Uuid::from_u128(703)),
                    author_session_id: SessionId::from_uuid(Uuid::from_u128(704)),
                    crdt_peer_id: CrdtPeerId::from_u64(705),
                },
                &OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
                    block: BlockLocation {
                        block_id: fixture.block_id,
                        home_document_id: fixture.home_document_id,
                    },
                    content: "second accepted".into(),
                }])
                .unwrap(),
            )
            .unwrap();
        fixture
            .archive
            .publish_bootstrap_prepared_for_test(&second)
            .unwrap();
        fixture
            .engine
            .stage_archive_batch(second.manifest().batch_id())
            .unwrap();
        assert!(first.manifest().batch_id() > second.manifest().batch_id());
        let first_event = AcceptedBatchEvent::from_accepted(
            &fixture.engine,
            &fixture.archive,
            first.manifest().batch_id(),
        )
        .unwrap();
        let second_event = AcceptedBatchEvent::from_accepted(
            &fixture.engine,
            &fixture.archive,
            second.manifest().batch_id(),
        )
        .unwrap();
        assert!(first_event.acceptance_sequence() < second_event.acceptance_sequence());
        fixture
            .tail
            .try_enqueue(&mut fixture.database, &fixture.engine, &second_event)
            .unwrap();
        fixture
            .tail
            .try_enqueue(&mut fixture.database, &fixture.engine, &first_event)
            .unwrap();
        let source = RebuildSource::new(&fixture.engine, &fixture.archive).unwrap();
        assert_eq!(
            fixture
                .tail
                .drain_ready(&mut fixture.database, &source, 64)
                .unwrap(),
            2
        );
        assert_eq!(
            fixture.database.frontier_root().unwrap(),
            fixture.engine.accepted_frontier_root().unwrap()
        );
    }

    fn hex(bytes: &[u8]) -> String {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut encoded = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            encoded.push(DIGITS[(byte >> 4) as usize] as char);
            encoded.push(DIGITS[(byte & 0x0f) as usize] as char);
        }
        encoded
    }

    /// Byte-for-byte image of the immutable publication surface.
    ///
    /// Every object and batch manifest file is compared by exact bytes, so an
    /// extra object, a rewritten manifest, or leftover temporary residue under
    /// either directory is detected. The archive's top-level entry names are
    /// included so a stray sibling namespace is detected too. Derived
    /// namespaces the resume is expected to advance (durable engine history,
    /// the projection work index, run-local scratch) are deliberately not
    /// byte-compared.
    fn snapshot_immutable_publication(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut image = Vec::new();
        let mut names = fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        names.sort_unstable();
        image.push((
            PathBuf::from("<archive-entry-names>"),
            format!("{names:?}").into_bytes(),
        ));
        for immutable in ["objects", "batches"] {
            let directory = root.join(immutable);
            assert!(
                directory.is_dir(),
                "{immutable} is not an archive directory"
            );
            image.extend(
                snapshot_tree(&directory)
                    .into_iter()
                    .map(|(path, bytes)| (PathBuf::from(immutable).join(path), bytes)),
            );
        }
        image
    }

    fn snapshot_tree(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        fn walk(base: &Path, current: &Path, output: &mut Vec<(PathBuf, Vec<u8>)>) {
            let mut entries = fs::read_dir(current)
                .unwrap()
                .map(|entry| entry.unwrap())
                .collect::<Vec<_>>();
            entries.sort_unstable_by_key(|entry| entry.file_name());
            for entry in entries {
                let path = entry.path();
                if entry.file_type().unwrap().is_dir() {
                    walk(base, &path, output);
                } else {
                    output.push((
                        path.strip_prefix(base).unwrap().to_path_buf(),
                        fs::read(path).unwrap(),
                    ));
                }
            }
        }
        let mut output = Vec::new();
        walk(root, root, &mut output);
        output
    }
}
