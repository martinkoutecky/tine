//! Admitted local semantic mutation and one-shot external reconciliation
//! through one sealed authoritative and derived-state drain.
//!
//! The managed runtime owns this crate-private coordinator and invokes it for
//! admitted local edits, provider ingress, recovery, and derived-state drains.

use std::fmt;
#[cfg(test)]
use std::time::{Duration, Instant};

use crate::model::{HandoffSafeGuard, PublishedHandoffLatch};
use crate::Graph;

use super::absence_sweep::{SweepError, SweepRecorder};
use super::enrollment::{EnrollmentError, VerifiedLocalCompositionError};
use super::hot_engine::prepared_manifest_fingerprint;
use super::hot_engine::{LocalAuthorCapture, ReconciliationNeeded};
use super::import::plan_clean_affected_import;
use super::local_active::{
    CleanRuntimeSession, LocalRuntimeAdmission, RuntimePromotionError, RuntimeRevocation,
    WorkspaceAuthorityBoundary, WorkspaceAuthorityRefusal,
};
use super::writer_lane::{WriterLaneError, WriterLaneTip, WriterRole};
use super::{
    AcceptedBatchEvent, AuthorBatch, BatchDisposition, BatchId, BatchInspection, BatchOrigin,
    CausalPeerId, ContentDigest, CrdtPeerId, ImportPlanStatus, NeedsFullHistory,
    OperationTransaction, PageId, PreparedBatch, ProjectionEndpointBinding, ProjectionReceiptStore,
    SessionId, ShardedHotEngine, SqliteFrontier,
};
#[cfg(test)]
use super::{ObjectStore, RebuildSource, TailOverlay};
use crate::oplog::projection_turn_journal::ProjectionTurnJournalState;

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TrustedLocalPreparationStageTimings {
    pub(crate) session_parts: Duration,
    pub(crate) bindings: Duration,
    pub(crate) draft: Duration,
    pub(crate) capture: Duration,
    pub(crate) finalize: Duration,
}

#[cfg(test)]
thread_local! {
    static LAST_TRUSTED_LOCAL_PREPARATION_STAGE_TIMINGS:
        std::cell::Cell<TrustedLocalPreparationStageTimings> =
        std::cell::Cell::new(TrustedLocalPreparationStageTimings {
            session_parts: Duration::ZERO,
            bindings: Duration::ZERO,
            draft: Duration::ZERO,
            capture: Duration::ZERO,
            finalize: Duration::ZERO,
        });
}

#[cfg(test)]
fn reset_trusted_local_preparation_stage_timings() {
    LAST_TRUSTED_LOCAL_PREPARATION_STAGE_TIMINGS
        .set(TrustedLocalPreparationStageTimings::default());
}

#[cfg(test)]
fn note_trusted_local_preparation_stage(
    update: impl FnOnce(&mut TrustedLocalPreparationStageTimings),
) {
    LAST_TRUSTED_LOCAL_PREPARATION_STAGE_TIMINGS.with(|timings| {
        let mut current = timings.get();
        update(&mut current);
        timings.set(current);
    });
}

#[cfg(test)]
pub(crate) fn last_trusted_local_preparation_stage_timings() -> TrustedLocalPreparationStageTimings
{
    LAST_TRUSTED_LOCAL_PREPARATION_STAGE_TIMINGS.get()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OperationalPhase {
    Bindings,
    Planning,
    Draft,
    Capture,
    Finalize,
    Publication,
    ArchiveStage,
    SqliteDrain,
    ProjectionDrain,
}

/// Stable post-publication evidence that retrying the exact immutable batch
/// cannot turn into progress.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RetainedBlockReason {
    PublishedAuthentication,
    StableBinding,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OperationalCoordinatorError {
    phase: OperationalPhase,
    detail: String,
    revocation: Option<RuntimeRevocation>,
    retained_block: Option<RetainedBlockReason>,
    /// This "failure" is a bounded slice asking to be resumed, not something
    /// that went wrong. It must not be charged against the transient-failure
    /// retry budget: retrying cannot continue a slice, so a legitimate
    /// multi-slice import would exhaust three attempts and wedge permanently.
    continuation_required: bool,
}

impl OperationalCoordinatorError {
    fn new(phase: OperationalPhase, detail: impl Into<String>) -> Self {
        Self {
            phase,
            detail: detail.into(),
            revocation: None,
            retained_block: None,
            continuation_required: false,
        }
    }

    /// A bounded slice completed its portion and must be resumed. Distinct from
    /// a failure so the caller can continue instead of retrying.
    #[cfg(test)]
    fn continuation_required(phase: OperationalPhase, detail: impl Into<String>) -> Self {
        Self {
            phase,
            detail: detail.into(),
            revocation: None,
            retained_block: None,
            continuation_required: true,
        }
    }

    fn revoked(phase: OperationalPhase, refusal: WorkspaceAuthorityRefusal) -> Self {
        Self {
            phase,
            detail: refusal.to_string(),
            revocation: refusal.revocation().cloned(),
            retained_block: None,
            continuation_required: false,
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
            continuation_required: false,
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
    #[cfg(test)]
    pub(crate) const fn revocation(&self) -> Option<&RuntimeRevocation> {
        self.revocation.as_ref()
    }

    #[cfg(test)]
    pub(crate) const fn retained_block_reason(&self) -> Option<&RetainedBlockReason> {
        self.retained_block.as_ref()
    }

    /// True when this is a resume request from a bounded slice rather than a
    /// failure. See `continuation_required`.
    pub(crate) const fn is_continuation_required(&self) -> bool {
        self.continuation_required
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
/// Six call sites below are this coordinator's complete set of boundaries that
/// take, change, or externalize authority:
///
/// - [`WorkspaceAuthorityBoundary::WindowAuthorization`], once, in
///   `authorize_coordinator` — and therefore on every coordinator entry point;
/// - [`WorkspaceAuthorityBoundary::Publication`], three times, one per
///   immutable publication path (`execute_clean_local_inner`,
///   `ingest_clean_prepared`, `execute_clean_external`);
/// - [`WorkspaceAuthorityBoundary::SqliteDrain`], once, at the SQLite advance;
/// - [`WorkspaceAuthorityBoundary::ProjectionDrain`], once per manifested
///   Markdown projection step.
///
/// Each names its own [`OperationalPhase`], so a lost workspace stays
/// diagnosable by phase rather than collapsing into one generic error.
/// `the_documented_authority_boundaries_are_the_complete_set` pins that
/// enumeration. The remaining two coordinator boundaries, `ArchiveStage` and
/// `TailAdmission`, belong to the sibling drain in `local_journal_drain.rs`;
/// this list previously named them, having drifted from that file's.
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

pub(crate) struct OperationalCoordinator;

pub(crate) enum CleanProviderAdmissionState {
    Mutation(CleanLocalMutationState),
    NeedsFullHistory(NeedsFullHistory),
}

impl OperationalCoordinator {
    /// Prepare a bounded existing-page transaction for the durable foreground
    /// journal without forcing accepted-history, SQLite, or graph projection
    /// work into the caller's turn. Unlike the editor-specialized variant,
    /// this admits every affected existing-page projection in the batch.
    pub(crate) fn prepare_clean_trusted_local_compound(
        session: &mut CleanRuntimeSession<'_>,
        graph: &Graph,
        receipts: &ProjectionReceiptStore,
        transaction: &OperationTransaction,
        correlated_batch_id: BatchId,
        page_home_hints: &std::collections::BTreeMap<PageId, super::DocumentId>,
    ) -> Result<PreparedLocalMutationState, OperationalCoordinatorError> {
        #[cfg(test)]
        {
            reset_trusted_local_preparation_stage_timings();
            super::hot_engine::reset_local_mutation_detail_timings();
        }
        #[cfg(test)]
        let parts_started = Instant::now();
        let (admission, engine, database) = session.parts().map_err(|refusal| {
            OperationalCoordinatorError::revoked(OperationalPhase::Bindings, refusal)
        })?;
        #[cfg(test)]
        note_trusted_local_preparation_stage(|timings| {
            timings.session_parts = parts_started.elapsed();
        });
        let claim_source = database.materialized_read().map_err(|error| {
            OperationalCoordinatorError::new(
                OperationalPhase::Draft,
                format!("clean SQLite identity candidates are unavailable: {error}"),
            )
        })?;
        prepare_local_inner(
            &admission,
            graph,
            receipts,
            engine,
            LocalDraftSource::Promoted {
                batch_id: Some(correlated_batch_id),
            },
            transaction,
            None,
            Some(&claim_source),
            Some(page_home_hints),
        )
    }

    /// Prepare one ordinary existing-page edit for the clean runtime's durable
    /// foreground journal boundary.
    ///
    /// This is the preparation half of the same coordinator path used by a
    /// synchronous clean manifest commit.  It deliberately stops before
    /// SQLite identity preflight and archive publication: the trusted-local
    /// coordinator makes the canonical prepared batch durable in the local
    /// journal first, while archive/SQLite expansion remains later actor work.
    pub(crate) fn prepare_clean_trusted_local(
        session: &mut CleanRuntimeSession<'_>,
        graph: &Graph,
        receipts: &ProjectionReceiptStore,
        transaction: &OperationTransaction,
        prepared_editor_projection: super::projection::PreparedEditorProjection,
    ) -> Result<PreparedLocalMutationState, OperationalCoordinatorError> {
        #[cfg(test)]
        {
            reset_trusted_local_preparation_stage_timings();
            super::hot_engine::reset_local_mutation_detail_timings();
        }
        #[cfg(test)]
        let parts_started = Instant::now();
        let (admission, engine, database) = session.parts().map_err(|refusal| {
            OperationalCoordinatorError::revoked(OperationalPhase::Bindings, refusal)
        })?;
        #[cfg(test)]
        note_trusted_local_preparation_stage(|timings| {
            timings.session_parts = parts_started.elapsed();
        });
        let claim_source = database.materialized_read().map_err(|error| {
            OperationalCoordinatorError::new(
                OperationalPhase::Draft,
                format!("clean SQLite identity candidates are unavailable: {error}"),
            )
        })?;
        prepare_local_inner(
            &admission,
            graph,
            receipts,
            engine,
            LocalDraftSource::Promoted { batch_id: None },
            transaction,
            Some(prepared_editor_projection),
            Some(&claim_source),
            None,
        )
    }
}

/// Result after the clean runtime's only irreversible boundary. A pending
/// continuation means the manifest is already the durable operation commit;
/// only disposable SQLite and/or exact Markdown projection remains.
pub(crate) enum CleanLocalMutationState {
    Complete(BatchId),
    DurablePending(CleanPublishedContinuation),
}

pub(crate) enum CleanExternalMutationState {
    Noop,
    Complete(BatchId),
    DurablePending(CleanPublishedContinuation),
}

pub(crate) struct CleanPublishedContinuation {
    guard: PublishedHandoffLatch,
    batch_id: BatchId,
    identity: super::sqlite::PreparedSqliteIdentityTransition,
    failure: OperationalCoordinatorError,
}

impl CleanPublishedContinuation {
    pub(crate) const fn batch_id(&self) -> BatchId {
        self.batch_id
    }

    pub(crate) fn failure(&self) -> &OperationalCoordinatorError {
        &self.failure
    }
}

fn execute_clean_local_inner(
    session: &mut CleanRuntimeSession<'_>,
    graph: &Graph,
    receipts: &ProjectionReceiptStore,
    transaction: &OperationTransaction,
    batch_id: Option<BatchId>,
    persist_fingerprint: Option<&mut dyn FnMut(ContentDigest) -> Result<(), String>>,
    projection_turns: &mut ProjectionTurnJournalState,
) -> Result<CleanLocalMutationState, OperationalCoordinatorError> {
    let (admission, engine, database) = session.parts().map_err(|refusal| {
        OperationalCoordinatorError::revoked(OperationalPhase::Bindings, refusal)
    })?;
    let claim_source = database.materialized_read().map_err(|error| {
        OperationalCoordinatorError::new(
            OperationalPhase::Draft,
            format!("clean SQLite identity candidates are unavailable: {error}"),
        )
    })?;
    let mut prepared = match prepare_local_inner(
        &admission,
        graph,
        receipts,
        engine,
        LocalDraftSource::Promoted { batch_id },
        transaction,
        None,
        Some(&claim_source),
        None,
    )? {
        PreparedLocalMutationState::Prepared(prepared) => prepared,
        PreparedLocalMutationState::ReconciliationRequired(_) => {
            return Err(OperationalCoordinatorError::new(
                OperationalPhase::Planning,
                "clean local mutation requires external reconciliation before publication",
            ));
        }
    };
    drop(claim_source);
    prepared.preflight_identity(database, engine)?;
    if let Some(persist_fingerprint) = persist_fingerprint {
        let manifest_bytes = prepared.prepared.manifest().encode().map_err(|error| {
            OperationalCoordinatorError::new(OperationalPhase::Finalize, error.to_string())
        })?;
        persist_fingerprint(ContentDigest::of(&manifest_bytes)).map_err(|error| {
            OperationalCoordinatorError::new(OperationalPhase::Publication, error)
        })?;
    }
    let PreparedLocalMutation {
        endpoint: _,
        guard,
        prepared,
        batch_id,
        identity,
    } = prepared;
    let identity = identity.expect("clean local publication requires identity preflight");
    reprove_workspace_authority(
        &admission,
        WorkspaceAuthorityBoundary::Publication,
        OperationalPhase::Publication,
    )?;
    let archive = engine.archive_store_capability().ok_or_else(|| {
        OperationalCoordinatorError::new(
            OperationalPhase::ArchiveStage,
            "clean runtime has no retained operation archive",
        )
    })?;
    let published = guard.into_published_latch();
    let commit_claim_source = database.materialized_read().map_err(|error| {
        OperationalCoordinatorError::new(
            OperationalPhase::Publication,
            format!("clean SQLite identity candidates are unavailable: {error}"),
        )
    })?;
    let outcome = match engine.commit_clean_prepared(&prepared, &commit_claim_source) {
        Ok(outcome) => outcome,
        Err(error) => {
            let failure =
                OperationalCoordinatorError::new(OperationalPhase::Publication, error.to_string());
            if matches!(archive.inspect_batch(batch_id), Ok(BatchInspection::Absent)) {
                admission.note_writer_lane_reservation_absent(batch_id);
                published.cancel_prepublication();
                return Err(failure);
            }
            return Ok(CleanLocalMutationState::DurablePending(
                CleanPublishedContinuation {
                    guard: published,
                    batch_id,
                    identity,
                    failure,
                },
            ));
        }
    };
    drop(commit_claim_source);
    if !matches!(outcome.disposition(), BatchDisposition::Accepted { .. }) {
        return Err(OperationalCoordinatorError::new(
            OperationalPhase::ArchiveStage,
            "clean manifest commit did not leave one accepted operation",
        ));
    }
    let mut continuation = CleanPublishedContinuation {
        guard: published,
        batch_id,
        identity,
        failure: OperationalCoordinatorError::new(
            OperationalPhase::SqliteDrain,
            "durable clean operation is awaiting derived-state application",
        ),
    };
    if let Err(error) = fault(OperationalFaultPoint::AfterManifest) {
        continuation.failure = error;
        return Ok(CleanLocalMutationState::DurablePending(continuation));
    }
    match resume_clean_published(
        &admission,
        graph,
        receipts,
        engine,
        database,
        &continuation,
        projection_turns,
    ) {
        Ok(()) => {
            continuation.guard.complete();
            Ok(CleanLocalMutationState::Complete(batch_id))
        }
        Err(error) => {
            continuation.failure = error;
            Ok(CleanLocalMutationState::DurablePending(continuation))
        }
    }
}

impl OperationalCoordinator {
    /// Execute one local semantic operation through the clean
    /// baseline-plus-manifest runtime. Validation, exact graph capture and the
    /// SQLite identity transition are completed at frontier F before the
    /// manifest commit. After that commit, SQLite and Markdown projection are
    /// recoverable derived work and are retained in an affine continuation on
    /// failure.
    pub(crate) fn execute_clean_local(
        session: &mut CleanRuntimeSession<'_>,
        graph: &Graph,
        receipts: &ProjectionReceiptStore,
        transaction: &OperationTransaction,
        projection_turns: &mut ProjectionTurnJournalState,
    ) -> Result<CleanLocalMutationState, OperationalCoordinatorError> {
        execute_clean_local_inner(
            session,
            graph,
            receipts,
            transaction,
            None,
            None,
            projection_turns,
        )
    }

    /// Execute one clean local mutation with a stable application-owned batch
    /// identity. The immutable episode record is published before the manifest
    /// commit, so a crash can distinguish a retry of the same move from an
    /// unrelated batch collision without creating a second semantic edit.
    #[cfg(test)]
    pub(crate) fn execute_clean_local_correlated(
        session: &mut CleanRuntimeSession<'_>,
        graph: &Graph,
        receipts: &ProjectionReceiptStore,
        batch_id: BatchId,
        transaction: &OperationTransaction,
        mut persist_fingerprint: impl FnMut(ContentDigest) -> Result<(), String>,
        projection_turns: &mut ProjectionTurnJournalState,
    ) -> Result<CleanLocalMutationState, OperationalCoordinatorError> {
        execute_clean_local_inner(
            session,
            graph,
            receipts,
            transaction,
            Some(batch_id),
            Some(&mut persist_fingerprint),
            projection_turns,
        )
    }

    pub(crate) fn retry_clean_local_with_turns(
        session: &mut CleanRuntimeSession<'_>,
        graph: &Graph,
        receipts: &ProjectionReceiptStore,
        mut continuation: CleanPublishedContinuation,
        projection_turns: &mut ProjectionTurnJournalState,
    ) -> CleanLocalMutationState {
        let (admission, engine, database) = match session.parts() {
            Ok(parts) => parts,
            Err(refusal) => {
                continuation.failure =
                    OperationalCoordinatorError::revoked(OperationalPhase::Bindings, refusal);
                return CleanLocalMutationState::DurablePending(continuation);
            }
        };
        match resume_clean_published(
            &admission,
            graph,
            receipts,
            engine,
            database,
            &continuation,
            projection_turns,
        ) {
            Ok(()) => {
                let batch_id = continuation.batch_id;
                continuation.guard.complete();
                CleanLocalMutationState::Complete(batch_id)
            }
            Err(error) => {
                continuation.failure = error;
                CleanLocalMutationState::DurablePending(continuation)
            }
        }
    }

    /// Validate and commit one provider-delivered clean batch without first
    /// placing its manifest in the accepted archive. Provider bytes are only
    /// transport evidence; the manifest becomes authority at the same
    /// commit-last boundary used by local and external authoring.
    pub(crate) fn ingest_clean_prepared(
        session: &mut CleanRuntimeSession<'_>,
        graph: &Graph,
        receipts: &ProjectionReceiptStore,
        projection_turns: &mut ProjectionTurnJournalState,
        prepared: &PreparedBatch,
    ) -> Result<CleanProviderAdmissionState, OperationalCoordinatorError> {
        let (admission, engine, database) = session.parts().map_err(|refusal| {
            OperationalCoordinatorError::revoked(OperationalPhase::Bindings, refusal)
        })?;
        authorize_coordinator(&admission, graph, engine)?;
        let endpoint = engine.projection_endpoint_binding().ok_or_else(|| {
            OperationalCoordinatorError::new(
                OperationalPhase::Bindings,
                "clean engine has no projection endpoint",
            )
        })?;
        verify_projection_bindings(graph, receipts, engine, endpoint)?;
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
        if let Some(need) = engine
            .preflight_prepared_full_history(prepared)
            .map_err(|error| {
                OperationalCoordinatorError::new(OperationalPhase::Finalize, error.to_string())
            })?
        {
            handoff.cancel();
            return Ok(CleanProviderAdmissionState::NeedsFullHistory(need));
        }
        let identity = database
            .preflight_prepared_identity_transition(engine, prepared)
            .map_err(|error| {
                OperationalCoordinatorError::new(OperationalPhase::Finalize, error.to_string())
            })?;
        reprove_workspace_authority(
            &admission,
            WorkspaceAuthorityBoundary::Publication,
            OperationalPhase::Publication,
        )?;
        let archive = engine.archive_store_capability().ok_or_else(|| {
            OperationalCoordinatorError::new(
                OperationalPhase::ArchiveStage,
                "clean runtime has no retained operation archive",
            )
        })?;
        let published = handoff.into_publisher_guard().into_published_latch();
        let batch_id = prepared.manifest().batch_id();
        let commit_claim_source = database.materialized_read().map_err(|error| {
            OperationalCoordinatorError::new(
                OperationalPhase::Publication,
                format!("clean SQLite identity candidates are unavailable: {error}"),
            )
        })?;
        let outcome = match engine.commit_clean_prepared(&prepared, &commit_claim_source) {
            Ok(outcome) => outcome,
            Err(super::EngineError::NeedsFullHistory(need)) => {
                admission.note_writer_lane_reservation_absent(batch_id);
                published.cancel_prepublication();
                return Ok(CleanProviderAdmissionState::NeedsFullHistory(need));
            }
            Err(error) => {
                let failure = OperationalCoordinatorError::new(
                    OperationalPhase::Publication,
                    error.to_string(),
                );
                if matches!(archive.inspect_batch(batch_id), Ok(BatchInspection::Absent)) {
                    // Proved never durable: the lane may reuse this counter.
                    // Anything else stays an unproved outcome and continues to
                    // block the lane until a reopen resolves it.
                    admission.note_writer_lane_reservation_absent(batch_id);
                    published.cancel_prepublication();
                    return Err(failure);
                }
                return Ok(CleanProviderAdmissionState::Mutation(
                    CleanLocalMutationState::DurablePending(CleanPublishedContinuation {
                        guard: published,
                        batch_id,
                        identity,
                        failure,
                    }),
                ));
            }
        };
        drop(commit_claim_source);
        if !matches!(outcome.disposition(), BatchDisposition::Accepted { .. }) {
            return Err(OperationalCoordinatorError::new(
                OperationalPhase::ArchiveStage,
                "clean provider manifest did not leave one accepted operation",
            ));
        }
        let mut continuation = CleanPublishedContinuation {
            guard: published,
            batch_id,
            identity,
            failure: OperationalCoordinatorError::new(
                OperationalPhase::SqliteDrain,
                "durable clean provider operation is awaiting derived-state application",
            ),
        };
        #[cfg(test)]
        if let Err(error) = fault(OperationalFaultPoint::AfterManifest) {
            continuation.failure = error;
            return Ok(CleanProviderAdmissionState::Mutation(
                CleanLocalMutationState::DurablePending(continuation),
            ));
        }
        match resume_clean_published(
            &admission,
            graph,
            receipts,
            engine,
            database,
            &continuation,
            projection_turns,
        ) {
            Ok(()) => {
                continuation.guard.complete();
                Ok(CleanProviderAdmissionState::Mutation(
                    CleanLocalMutationState::Complete(batch_id),
                ))
            }
            Err(error) => {
                continuation.failure = error;
                Ok(CleanProviderAdmissionState::Mutation(
                    CleanLocalMutationState::DurablePending(continuation),
                ))
            }
        }
    }

    /// Reconcile exact externally changed graph paths through the clean
    /// baseline-plus-manifest authority.  Planning reuses the established
    /// structural matcher, but its predecessor evidence is reconstructed from
    /// the lazy-genesis baseline and accepted manifests rather than the
    /// persistent Patricia work index.
    pub(crate) fn execute_clean_external(
        session: &mut CleanRuntimeSession<'_>,
        graph: &Graph,
        receipts: &ProjectionReceiptStore,
        requested_paths: &[&str],
        sweep_recorder: &mut dyn SweepRecorder,
        projection_turns: &mut ProjectionTurnJournalState,
    ) -> Result<CleanExternalMutationState, OperationalCoordinatorError> {
        let (admission, engine, database) = session.parts().map_err(|refusal| {
            OperationalCoordinatorError::revoked(OperationalPhase::Bindings, refusal)
        })?;
        authorize_coordinator(&admission, graph, engine)?;
        let endpoint = engine.projection_endpoint_binding().ok_or_else(|| {
            OperationalCoordinatorError::new(
                OperationalPhase::Bindings,
                "clean engine has no projection endpoint",
            )
        })?;
        verify_projection_bindings(graph, receipts, engine, endpoint)?;
        let archive = engine.archive_store_capability().ok_or_else(|| {
            OperationalCoordinatorError::new(
                OperationalPhase::ArchiveStage,
                "clean runtime has no retained operation archive",
            )
        })?;
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
        let plan = plan_clean_affected_import(graph, engine, database, requested_paths);
        fault(OperationalFaultPoint::AfterPlan)?;
        match plan.status() {
            ImportPlanStatus::Noop => {
                handoff.cancel();
                return Ok(CleanExternalMutationState::Noop);
            }
            ImportPlanStatus::Blocked => {
                handoff.cancel();
                let detail = plan
                    .blocks()
                    .first()
                    .map(|blocked| blocked.detail.clone())
                    .unwrap_or_else(|| "clean external reconciliation was blocked".into());
                return Err(OperationalCoordinatorError::new(
                    OperationalPhase::Planning,
                    detail,
                ));
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
        let claim_source = database.materialized_read().map_err(|error| {
            OperationalCoordinatorError::new(
                OperationalPhase::Draft,
                format!("clean SQLite identity candidates are unavailable: {error}"),
            )
        })?;
        let external_lane = admission
            .writer_lane_peer(WriterRole::External)
            .map_err(classify_authorization_failure)?;
        let causal_peer = admission
            .writer_lane_causal_peer()
            .map_err(classify_authorization_failure)?;
        let (author, draft) = draft_on_writer_lane(
            engine,
            endpoint,
            &material,
            Some(&claim_source),
            external_lane,
            causal_peer,
        )?;
        drop(claim_source);
        fault(OperationalFaultPoint::AfterDraft)?;
        let captured = engine
            .capture_external_author_transaction(draft, graph, receipts, endpoint)
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
                "clean external batch lost its import identity",
            ));
        }
        fault(OperationalFaultPoint::AfterFinalize)?;
        let identity = database
            .preflight_prepared_identity_transition(engine, &prepared)
            .map_err(|error| {
                OperationalCoordinatorError::new(OperationalPhase::Finalize, error.to_string())
            })?;
        reprove_workspace_authority(
            &admission,
            WorkspaceAuthorityBoundary::Publication,
            OperationalPhase::Publication,
        )?;
        // Manager correction 1: external publication is manifest-first, so the
        // exact archive commit below — not any local journal append — is this
        // path's durable prefix. The reservation precedes it.
        reserve_writer_lane_for_publication(
            &admission,
            engine,
            WriterRole::External,
            external_lane,
            &prepared,
        )?;
        let published = guard.into_published_latch();
        let batch_id = author.batch_id;
        let commit_claim_source = database.materialized_read().map_err(|error| {
            OperationalCoordinatorError::new(
                OperationalPhase::Publication,
                format!("clean SQLite identity candidates are unavailable: {error}"),
            )
        })?;
        let mut page_count_at_open = || {
            let mut count = 0_usize;
            let mut cursor = None;
            loop {
                let pages = commit_claim_source
                    .page_inventory_after(
                        cursor.as_ref().map(|(path, page_id)| (path, *page_id)),
                        None,
                        512,
                    )
                    .map_err(|error| SweepError::Invalid(error.to_string()))?;
                if pages.is_empty() {
                    break;
                }
                count = count.saturating_add(pages.len());
                let last = pages.last().expect("nonempty page inventory batch");
                cursor = Some((last.path.clone(), last.page_id));
                if pages.len() < 512 {
                    break;
                }
            }
            Ok(count)
        };
        if let Err(error) = sweep_recorder.record_prepared_absence_batch(
            engine,
            &prepared,
            &commit_claim_source,
            &mut page_count_at_open,
        ) {
            published.cancel_prepublication();
            return Err(OperationalCoordinatorError::new(
                OperationalPhase::Publication,
                format!("durable sweep membership append failed before deletion commit: {error}"),
            ));
        }
        if let Err(error) = fault(OperationalFaultPoint::AfterSweepRecord) {
            published.cancel_prepublication();
            return Err(error);
        }
        let outcome = match engine.commit_clean_prepared(&prepared, &commit_claim_source) {
            Ok(outcome) => outcome,
            Err(error) => {
                let failure = OperationalCoordinatorError::new(
                    OperationalPhase::Publication,
                    error.to_string(),
                );
                if matches!(archive.inspect_batch(batch_id), Ok(BatchInspection::Absent)) {
                    published.cancel_prepublication();
                    return Err(failure);
                }
                return Ok(CleanExternalMutationState::DurablePending(
                    CleanPublishedContinuation {
                        guard: published,
                        batch_id,
                        identity,
                        failure,
                    },
                ));
            }
        };
        drop(commit_claim_source);
        if !matches!(outcome.disposition(), BatchDisposition::Accepted { .. }) {
            return Err(OperationalCoordinatorError::new(
                OperationalPhase::ArchiveStage,
                "clean external manifest did not leave one accepted operation",
            ));
        }
        let mut continuation = CleanPublishedContinuation {
            guard: published,
            batch_id,
            identity,
            failure: OperationalCoordinatorError::new(
                OperationalPhase::SqliteDrain,
                "durable clean external operation is awaiting derived-state application",
            ),
        };
        if let Err(error) = fault(OperationalFaultPoint::AfterManifest) {
            continuation.failure = error;
            return Ok(CleanExternalMutationState::DurablePending(continuation));
        }
        match resume_clean_published(
            &admission,
            graph,
            receipts,
            engine,
            database,
            &continuation,
            projection_turns,
        ) {
            Ok(()) => {
                continuation.guard.complete();
                Ok(CleanExternalMutationState::Complete(batch_id))
            }
            Err(error) => {
                continuation.failure = error;
                Ok(CleanExternalMutationState::DurablePending(continuation))
            }
        }
    }

    #[cfg(test)]
    pub(super) fn prepare_trusted_local_with_author(
        admission: &LocalRuntimeAdmission<'_>,
        graph: &Graph,
        receipts: &ProjectionReceiptStore,
        engine: &mut ShardedHotEngine,
        author: AuthorBatch,
        transaction: &OperationTransaction,
    ) -> Result<PreparedLocalMutationState, OperationalCoordinatorError> {
        prepare_local_inner(
            admission,
            graph,
            receipts,
            engine,
            LocalDraftSource::Raw(author),
            transaction,
            None,
            None,
            None,
        )
    }
}

enum LocalDraftSource {
    Promoted {
        batch_id: Option<BatchId>,
    },
    #[cfg(test)]
    Raw(AuthorBatch),
}

pub(crate) enum PreparedLocalMutationState {
    Prepared(PreparedLocalMutation),
    ReconciliationRequired(ReconciliationNeeded),
}

/// Finalized local-author transaction plus the exact graph handoff retained
/// from draft capture. Construction stays in the established local
/// coordinator; the trusted-local commit consumes it and deliberately releases
/// the old pipeline handoff before entering the journal-authorized path guard.
pub(crate) struct PreparedLocalMutation {
    endpoint: ProjectionEndpointBinding,
    guard: HandoffSafeGuard,
    prepared: PreparedBatch,
    batch_id: BatchId,
    identity: Option<super::sqlite::PreparedSqliteIdentityTransition>,
}

impl PreparedLocalMutation {
    pub(crate) const fn prepared_batch(&self) -> &PreparedBatch {
        &self.prepared
    }

    fn preflight_identity(
        &mut self,
        database: &SqliteFrontier,
        engine: &ShardedHotEngine,
    ) -> Result<(), OperationalCoordinatorError> {
        if self.identity.is_some() {
            return Err(OperationalCoordinatorError::new(
                OperationalPhase::Finalize,
                "prepared local mutation was identity-preflighted twice",
            ));
        }
        self.identity = Some(
            database
                .preflight_prepared_identity_transition(engine, &self.prepared)
                .map_err(|error| {
                    OperationalCoordinatorError::new(OperationalPhase::Finalize, error.to_string())
                })?,
        );
        Ok(())
    }

    pub(super) fn into_trusted_batch(self) -> PreparedBatch {
        let Self {
            endpoint: _,
            guard,
            prepared,
            batch_id: _,
            identity: _,
        } = self;
        drop(guard);
        prepared
    }
}

#[allow(clippy::too_many_arguments)]
fn prepare_local_inner(
    admission: &LocalRuntimeAdmission<'_>,
    graph: &Graph,
    receipts: &ProjectionReceiptStore,
    engine: &mut ShardedHotEngine,
    source: LocalDraftSource,
    transaction: &OperationTransaction,
    prepared_editor_projection: Option<super::projection::PreparedEditorProjection>,
    claim_source: Option<&dyn super::hot_engine::ProjectionClaimSource>,
    page_home_hints: Option<&std::collections::BTreeMap<PageId, super::DocumentId>>,
) -> Result<PreparedLocalMutationState, OperationalCoordinatorError> {
    #[cfg(test)]
    let bindings_started = Instant::now();
    authorize_coordinator(admission, graph, engine)?;
    let endpoint = engine.projection_endpoint_binding().ok_or_else(|| {
        OperationalCoordinatorError::new(
            OperationalPhase::Bindings,
            "engine has no enrolled projection endpoint",
        )
    })?;
    verify_projection_bindings(graph, receipts, engine, endpoint)?;
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
    #[cfg(test)]
    note_trusted_local_preparation_stage(|timings| {
        timings.bindings = bindings_started.elapsed();
    });

    #[cfg(test)]
    let draft_started = Instant::now();
    let (batch_id, author_device_id, author_session_id, local_lane, draft) = match source {
        LocalDraftSource::Promoted { batch_id } => {
            let authority = admission
                .mint_local_author_authority(graph, engine, endpoint)
                .map_err(classify_authorization_failure)?;
            let author_device_id = authority.device_id();
            let author_session_id = authority.session_id();
            let local_lane = authority.crdt_peer_id();
            let (batch_id, draft) = match batch_id {
                Some(batch_id) => engine.draft_admitted_local_author_transaction_with_batch_id(
                    &authority,
                    batch_id,
                    transaction,
                    prepared_editor_projection,
                    claim_source,
                    page_home_hints,
                ),
                None => engine.draft_admitted_local_author_transaction(
                    &authority,
                    transaction,
                    prepared_editor_projection,
                    claim_source,
                    page_home_hints,
                ),
            }
            .map_err(|error| {
                OperationalCoordinatorError::new(OperationalPhase::Draft, error.to_string())
            })?;
            (
                batch_id,
                author_device_id,
                author_session_id,
                Some(local_lane),
                draft,
            )
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
                // The raw-author fixture hatch deliberately supplies its own
                // peer, so it has no admitted lane to reserve.
                None,
                draft,
            )
        }
    };
    fault(OperationalFaultPoint::AfterDraft)?;
    #[cfg(test)]
    note_trusted_local_preparation_stage(|timings| {
        timings.draft = draft_started.elapsed();
    });
    #[cfg(test)]
    let capture_started = Instant::now();
    let captured = match engine
        .capture_local_author_transaction(draft, graph, receipts, endpoint)
        .map_err(|error| {
            OperationalCoordinatorError::new(OperationalPhase::Capture, error.to_string())
        })? {
        LocalAuthorCapture::Captured(captured) => captured,
        LocalAuthorCapture::ReconciliationNeeded(reconciliation) => {
            drop(guard);
            return Ok(PreparedLocalMutationState::ReconciliationRequired(
                reconciliation,
            ));
        }
    };
    fault(OperationalFaultPoint::AfterCapture)?;
    #[cfg(test)]
    note_trusted_local_preparation_stage(|timings| {
        timings.capture = capture_started.elapsed();
    });
    #[cfg(test)]
    let finalize_started = Instant::now();
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
    // The trusted-local commit boundary makes this exact prepared batch durable
    // in the foreground journal without returning here, and `execute_clean_local`
    // publishes the same batch straight into the archive. Reserving the lane at
    // the end of preparation therefore precedes BOTH durable publications with
    // one call site, and the batch's causal dot is already final.
    if let Some(local_lane) = local_lane {
        reserve_writer_lane_for_publication(
            admission,
            engine,
            WriterRole::Local,
            local_lane,
            &prepared,
        )?;
    }
    fault(OperationalFaultPoint::AfterFinalize)?;
    #[cfg(test)]
    note_trusted_local_preparation_stage(|timings| {
        timings.finalize = finalize_started.elapsed();
    });
    Ok(PreparedLocalMutationState::Prepared(
        PreparedLocalMutation {
            endpoint,
            guard,
            prepared,
            batch_id,
            identity: None,
        },
    ))
}

fn resume_clean_published(
    admission: &LocalRuntimeAdmission<'_>,
    graph: &Graph,
    receipts: &ProjectionReceiptStore,
    engine: &mut ShardedHotEngine,
    database: &mut SqliteFrontier,
    continuation: &CleanPublishedContinuation,
    projection_turns: &mut ProjectionTurnJournalState,
) -> Result<(), OperationalCoordinatorError> {
    authorize_coordinator(admission, graph, engine)?;
    let event = AcceptedBatchEvent::from_accepted(
        engine,
        engine.archive_store().ok_or_else(|| {
            OperationalCoordinatorError::retained_block(
                OperationalPhase::ArchiveStage,
                "clean committed operation has no retained archive",
                RetainedBlockReason::PublishedAuthentication,
            )
        })?,
        continuation.batch_id,
    )
    .map_err(|error| {
        OperationalCoordinatorError::retained_block(
            OperationalPhase::ArchiveStage,
            error.to_string(),
            RetainedBlockReason::PublishedAuthentication,
        )
    })?
    .with_prepared_identity_transition(continuation.identity.clone())
    .map_err(|error| {
        OperationalCoordinatorError::retained_block(
            OperationalPhase::SqliteDrain,
            error.to_string(),
            RetainedBlockReason::PublishedAuthentication,
        )
    })?;
    let applied = database.frontier_root().map_err(|error| {
        OperationalCoordinatorError::new(OperationalPhase::SqliteDrain, error.to_string())
    })?;
    if applied.same_accepted_authority(event.prior_frontier_root()) {
        reprove_workspace_authority(
            admission,
            WorkspaceAuthorityBoundary::SqliteDrain,
            OperationalPhase::SqliteDrain,
        )?;
        database
            .apply_engine_owned_accepted(&event, engine)
            .map_err(|error| {
                OperationalCoordinatorError::new(OperationalPhase::SqliteDrain, error.to_string())
            })?;
        fault(OperationalFaultPoint::AfterSqliteApply)?;
    } else if !applied.same_accepted_authority(event.post_frontier_root()) {
        return Err(OperationalCoordinatorError::new(
            OperationalPhase::SqliteDrain,
            "clean SQLite frontier is neither before nor after the durable operation",
        ));
    }

    {
        let turns = projection_turns;
        let work = engine
            .clean_projection_work_for_batch(continuation.batch_id)
            .map_err(|error| {
                OperationalCoordinatorError::retained_block(
                    OperationalPhase::ProjectionDrain,
                    error.to_string(),
                    RetainedBlockReason::PublishedAuthentication,
                )
            })?;
        if !turns.retains_batch(continuation.batch_id) {
            let receiver = engine
                .projection_endpoint_binding()
                .ok_or_else(|| {
                    OperationalCoordinatorError::new(
                        OperationalPhase::ProjectionDrain,
                        "clean provider receiver has no enrolled projection endpoint",
                    )
                })?
                .endpoint_id();
            let archive = engine.archive_store().ok_or_else(|| {
                OperationalCoordinatorError::new(
                    OperationalPhase::ProjectionDrain,
                    "clean provider continuation has no accepted archive",
                )
            })?;
            let batch = match archive
                .inspect_batch(continuation.batch_id)
                .map_err(|error| {
                    OperationalCoordinatorError::new(
                        OperationalPhase::ProjectionDrain,
                        error.to_string(),
                    )
                })? {
                BatchInspection::Ready(batch) => batch,
                BatchInspection::Absent | BatchInspection::Staged { .. } => {
                    return Err(OperationalCoordinatorError::new(
                        OperationalPhase::ProjectionDrain,
                        "clean provider continuation batch is incomplete",
                    ));
                }
            };
            let projection = super::projection_manifest::validate_projection_object_set(
                batch.manifest(),
                batch.objects(),
            )
            .map_err(|error| {
                OperationalCoordinatorError::new(
                    OperationalPhase::ProjectionDrain,
                    error.to_string(),
                )
            })?;
            let foreign = projection
                .intents()
                .iter()
                .filter(|source| source.source_endpoint_id() != receiver)
                .cloned()
                .collect::<Vec<_>>();
            let (origin, pages) = if let Some(source) = foreign.first() {
                (
                    super::TurnOrigin::IngressForeign {
                        batch_id: continuation.batch_id,
                        source_endpoint_id: source.source_endpoint_id(),
                    },
                    super::projection::projection_turn_pages_for_foreign_sources(engine, &foreign)
                        .map_err(|error| {
                            OperationalCoordinatorError::new(
                                OperationalPhase::ProjectionDrain,
                                error.to_string(),
                            )
                        })?,
                )
            } else {
                (
                    super::TurnOrigin::IngressLocal {
                        batch_id: continuation.batch_id,
                    },
                    super::projection::projection_turn_pages_for_work(engine, &work).map_err(
                        |error| {
                            OperationalCoordinatorError::new(
                                OperationalPhase::ProjectionDrain,
                                error.to_string(),
                            )
                        },
                    )?,
                )
            };
            if !pages.is_empty() {
                turns.append(origin, pages).map_err(|error| {
                    OperationalCoordinatorError::new(
                        OperationalPhase::ProjectionDrain,
                        error.to_string(),
                    )
                })?;
            }
        }
        while let Some(turn) = turns.front().cloned() {
            reprove_workspace_authority(
                admission,
                WorkspaceAuthorityBoundary::ProjectionDrain,
                OperationalPhase::ProjectionDrain,
            )?;
            fault(OperationalFaultPoint::BeforeProjection)?;
            super::projection::replay_projection_turn(
                graph,
                receipts,
                engine,
                database,
                &turn,
                Some(&continuation.guard),
            )
            .map_err(|error| {
                OperationalCoordinatorError::new(
                    OperationalPhase::ProjectionDrain,
                    error.to_string(),
                )
            })?;
            turns.checkpoint_front().map_err(|error| {
                OperationalCoordinatorError::new(OperationalPhase::ProjectionDrain, error)
            })?;
            fault(OperationalFaultPoint::AfterProjection)?;
        }
        return Ok(());
    }
}

/// Durably reserve the exact batch that is about to become outwardly visible.
///
/// This is the ONE added foreground durability barrier on a publishing turn:
/// a fixed-size exact replacement of the device-private lane record, before the
/// trusted local journal append or the external archive manifest commit. It
/// re-proves workspace authority first, so a runtime that lost its lease cannot
/// reserve, and it refuses outright when a previous reservation has no proved
/// outcome — authoring past an unknown publication is exactly what would reuse
/// counters that may already be visible elsewhere.
fn reserve_writer_lane_for_publication(
    admission: &LocalRuntimeAdmission<'_>,
    engine: &ShardedHotEngine,
    role: WriterRole,
    peer: CrdtPeerId,
    prepared: &PreparedBatch,
) -> Result<(), OperationalCoordinatorError> {
    let manifest = prepared.manifest();
    // The coverage question is about the incarnation this batch actually
    // authored under, not the enrolled device: one device may have published
    // under several incarnations, and only this one's prefix decides.
    let proved = engine.own_causal_dot_high_water(manifest.causal_dot().peer_id());
    // Steady state: the previous reservation was absorbed by the live prefix,
    // so this is pure memory and reads no archive.
    if let Some((stale, counter)) = admission.pending_writer_lane_reservation() {
        if counter > proved
            && engine.archive_store_capability().is_some_and(|archive| {
                matches!(
                    archive.inspect_batch(stale.batch_id),
                    Ok(BatchInspection::Absent)
                )
            })
        {
            admission.note_writer_lane_reservation_absent(stale.batch_id);
        }
    }
    // The reservation names the exact publication, not merely a counter: an
    // integer alone cannot distinguish "my batch reached the store" from "a
    // different batch took my dot".
    let tip = WriterLaneTip {
        batch_id: manifest.batch_id(),
        manifest_digest: prepared_manifest_fingerprint(prepared),
    };
    admission
        .reserve_writer_lane(role, peer, tip, manifest.causal_dot().counter(), proved)
        .map_err(|error| match error {
            RuntimePromotionError::WriterLane(WriterLaneError::Uncontinuable(detail)) => {
                OperationalCoordinatorError::retained_block(
                    OperationalPhase::Publication,
                    detail,
                    RetainedBlockReason::StableBinding,
                )
            }
            other => classify_authorization_failure(other),
        })
}

/// Draft one external reconciliation on this endpoint's persistent import lane.
///
/// The bounded fresh-peer probe this replaced existed only to find a peer that
/// was *absent* from every affected document. That was the fresh-peer-per-import
/// invariant, and it is exactly what made a repeatedly reconciled document carry
/// one version-vector entry per import forever. `ImportId` keeps every other
/// role it had — deterministic `BatchId`, synthetic author session, observation
/// document — so import idempotence is untouched; only the CRDT peer stops
/// being per-import.
fn draft_on_writer_lane(
    engine: &ShardedHotEngine,
    endpoint: ProjectionEndpointBinding,
    material: &super::import::ImportExecutionMaterial,
    claim_source: Option<&dyn super::hot_engine::ProjectionClaimSource>,
    crdt_peer_id: CrdtPeerId,
    causal_peer_id: CausalPeerId,
) -> Result<(AuthorBatch, super::AuthorTransactionDraft), OperationalCoordinatorError> {
    if crdt_peer_id.as_u64() == 0 {
        return Err(OperationalCoordinatorError::new(
            OperationalPhase::Draft,
            "the admitted external-import writer lane has no CRDT peer",
        ));
    }
    let author = AuthorBatch {
        batch_id: material.batch_id(),
        author_device_id: endpoint.device_id(),
        author_session_id: SessionId::for_external_import_author(
            engine.workspace_id(),
            material.import_id(),
        ),
        crdt_peer_id,
        // The external role keeps its own Loro lane but shares this device's
        // ONE causal chain, so an import's counter continues the same sequence
        // as ordinary local work. `ImportId` keeps every other role it had.
        causal_peer_id,
    };
    let drafted = match claim_source {
        Some(source) => {
            engine.draft_clean_external_import_transaction(author, material.clone(), source)
        }
        None => engine.draft_external_import_transaction(author, material.clone()),
    };
    drafted.map(|draft| (author, draft)).map_err(|error| {
        OperationalCoordinatorError::new(OperationalPhase::Draft, error.to_string())
    })
}

fn verify_projection_bindings(
    graph: &Graph,
    receipts: &ProjectionReceiptStore,
    engine: &ShardedHotEngine,
    endpoint: ProjectionEndpointBinding,
) -> Result<(), OperationalCoordinatorError> {
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
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OperationalFaultPoint {
    AfterHandoff,
    AfterPlan,
    AfterDraft,
    AfterCapture,
    AfterFinalize,
    AfterSweepRecord,
    AfterManifest,
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

#[cfg(test)]
pub(crate) fn fail_once_at(point: OperationalFaultPoint) {
    OPERATIONAL_FAULT.set(Some(point));
}

#[cfg(test)]
pub(crate) fn fail_repeatedly_at(point: OperationalFaultPoint, failures: u8) {
    assert!(failures > 0, "a repeated operational fault needs work");
    OPERATIONAL_REPEATED_FAULT.set(Some((point, failures)));
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

#[cfg(test)]
pub(crate) fn fail_next_clean_after_manifest_for_harness() {
    fail_once_at(OperationalFaultPoint::AfterManifest);
}

fn operational_fault_error(point: OperationalFaultPoint) -> OperationalCoordinatorError {
    OperationalCoordinatorError::new(
        match point {
            OperationalFaultPoint::AfterHandoff => OperationalPhase::Bindings,
            OperationalFaultPoint::AfterPlan => OperationalPhase::Planning,
            OperationalFaultPoint::AfterDraft => OperationalPhase::Draft,
            OperationalFaultPoint::AfterCapture => OperationalPhase::Capture,
            OperationalFaultPoint::AfterFinalize => OperationalPhase::Finalize,
            OperationalFaultPoint::AfterSweepRecord => OperationalPhase::Publication,
            OperationalFaultPoint::AfterManifest => OperationalPhase::Publication,
            OperationalFaultPoint::AfterSqliteApply => OperationalPhase::SqliteDrain,
            OperationalFaultPoint::BeforeProjection | OperationalFaultPoint::AfterProjection => {
                OperationalPhase::ProjectionDrain
            }
        },
        "deterministic operational fault",
    )
}

#[cfg(test)]
mod tests {
    /// Explicit, stable fixture writer incarnation for the import lane.
    /// Production reads the saved one from the durable lane record.
    fn fixture_import_incarnation(endpoint: ProjectionEndpointBinding) -> CausalPeerId {
        CausalPeerId::from_key(crate::oplog::WriterIncarnationId::fixture_for_device(
            endpoint.device_id(),
        ))
    }

    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    use uuid::Uuid;

    use super::*;
    use crate::model::{projection_graph_test_counters, reset_projection_graph_test_counters};
    use crate::oplog::import::{
        commit_clean_activation, open_clean_activation, prepare_clean_activation,
    };
    use crate::oplog::local_active::CleanLocalRuntime;
    use crate::oplog::object_store::fail_next_publish_after_objects;
    use crate::oplog::sqlite::{LeasedWorkspaceProjection, WorkspaceRuntimeLease};
    use crate::oplog::{
        recover_incomplete_projections, AnnotatedProjectionBase, ApplicationRuntimeRoot, BlockId,
        BlockLocation, DeviceId, DocumentId, EngineError, LineageDigest, LogicalPageName,
        ManagedPath, ManagedTextKind, ManifestProjectionPrecondition, ManifestedProjectionIntent,
        ObjectKind, OperationBatch, OperationObject, OperationTransaction, PageId, PreparedBatch,
        ProjectionClaim, ProjectionEndpointId, SemanticOperation, ValidatedBatch, TAIL_MAX_BYTES,
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

    /// The baseline-plus-manifest composition used by the clean coordinator
    /// corpus. Source files are the genesis and operation manifests are the
    /// only durable semantic history; no enrolled history, projection-work
    /// index, scratch store, or persistent tail participates.
    struct CleanCoordinatorFixture {
        _root: TestRoot,
        graph_root: PathBuf,
        archive_root: PathBuf,
        enrollment_root: PathBuf,
        database_path: PathBuf,
        graph: Graph,
        receipts: ProjectionReceiptStore,
        archive: ObjectStore,
        runtime: CleanLocalRuntime,
        projection_turns: crate::oplog::projection_turn_journal::ProjectionTurnJournalState,
        lineage: LineageDigest,
        catalog: DocumentId,
        page_id: PageId,
        home_document_id: DocumentId,
        block_id: BlockId,
        path: String,
    }

    impl CleanCoordinatorFixture {
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
            Self::new_at(
                label,
                "pages/Coordinator Page.md",
                None,
                ManagedTextKind::Page,
            )
        }

        fn new_at(label: &str, path: &str, config: Option<&str>, _kind: ManagedTextKind) -> Self {
            let root = TestRoot::new(&format!("{label}-clean"));
            let graph_root = root.path().join("graph");
            fs::create_dir_all(&graph_root).unwrap();
            if let Some(config) = config {
                fs::create_dir_all(graph_root.join("logseq")).unwrap();
                fs::write(graph_root.join("logseq/config.edn"), config).unwrap();
            }
            let target = graph_root.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(&target, clean_coordinator_source(path, "root", "child")).unwrap();
            let graph = Graph::open(&graph_root);

            let workspace = super::super::WorkspaceId::from_uuid(Uuid::from_u128(1));
            let lineage = LineageDigest::of(label.as_bytes());
            let catalog = DocumentId::from_uuid(Uuid::from_u128(4));
            let database_path = root.path().join("clean-projection.sqlite");
            let archive_parent = root.path().join("clean-archive");
            let enrollment_root = root.path().join("clean-enrollment");
            fs::create_dir(&archive_parent).unwrap();
            let capture_root = root.path().join("clean-capture");
            fs::create_dir(&capture_root).unwrap();
            let capture = graph
                .capture_inactive_bootstrap_sources(&capture_root)
                .unwrap();
            let preparation = prepare_clean_activation(
                &graph,
                capture,
                workspace,
                lineage,
                catalog,
                &root.path().join("clean-preparation"),
                &database_path,
                &super::super::ReferenceCatalogPolicyV1::default(),
            )
            .unwrap();
            let page_id = preparation
                .candidates()
                .baseline()
                .page_ids()
                .find(|page_id| {
                    preparation
                        .candidates()
                        .baseline()
                        .page(*page_id)
                        .unwrap()
                        .is_some_and(|page| page.path.as_str() == path)
                })
                .unwrap_or_else(|| panic!("clean baseline has no {path}"));
            let committed = commit_clean_activation(
                &graph,
                preparation,
                &archive_parent.join(crate::oplog::lazy_genesis::LAZY_GENESIS_BASELINE_DIRECTORY),
                &enrollment_root,
            )
            .unwrap();
            let (baseline, physical, baseline_frontier, _) = committed.into_parts();
            drop(physical);
            drop(baseline);
            let reopened = open_clean_activation(
                &enrollment_root,
                &archive_parent.join(crate::oplog::lazy_genesis::LAZY_GENESIS_BASELINE_DIRECTORY),
                &database_path,
                catalog,
                super::super::ReferenceCatalogPolicyV1::default(),
                &crate::config::ParseConfig::default(),
            )
            .unwrap()
            .expect("published clean coordinator activation reopens");
            let (mut engine, projection, _) = reopened.into_parts();
            let archive_root = archive_parent.join("operations");
            engine
                .attach_clean_archive_store(ObjectStore::open(&archive_root, workspace).unwrap())
                .unwrap();
            let archive = ObjectStore::open(&archive_root, workspace).unwrap();
            let lease = WorkspaceRuntimeLease::acquire(&archive, workspace).unwrap();
            let projection = LeasedWorkspaceProjection::adopt_clean_genesis(
                lease,
                &database_path,
                ProjectionClaim::current(workspace, lineage),
                &baseline_frontier,
                &archive,
                &engine,
                projection,
                crate::config::ParseConfig::default(),
            )
            .map_err(|(_, error)| error)
            .unwrap();
            let endpoint = ProjectionEndpointBinding::enroll_graph(
                &graph,
                ProjectionEndpointId::from_uuid(Uuid::from_u128(2)),
                DeviceId::from_uuid(Uuid::from_u128(3)),
            )
            .unwrap();
            let receipts = ProjectionReceiptStore::open_for_endpoint(
                &root.path().join("clean-receipts"),
                workspace,
                endpoint,
            )
            .unwrap();
            engine
                .attach_clean_projection_endpoint(&graph, &receipts)
                .unwrap();
            let runtime = CleanLocalRuntime::from_open_parts(
                SessionId::from_uuid(Uuid::from_u128(7)),
                endpoint,
                engine,
                projection,
            )
            .unwrap();
            let page = runtime.engine().materialize_page(page_id).unwrap();
            let home_document_id = page.home_document_id;
            let block_id = page.blocks[0].block_id;
            Self {
                projection_turns:
                    crate::oplog::projection_turn_journal::open_scratch_projection_turn_journal_for(
                        runtime.engine(),
                    ),
                _root: root,
                graph_root,
                archive_root,
                enrollment_root,
                database_path,
                graph,
                receipts,
                archive,
                runtime,
                lineage,
                catalog,
                page_id,
                home_document_id,
                block_id,
                path: path.into(),
            }
        }

        fn overwrite(&self, bytes: &[u8]) {
            fs::write(self.graph_root.join(&self.path), bytes).unwrap();
        }

        fn engine(&self) -> &ShardedHotEngine {
            self.runtime.engine()
        }

        fn database(&self) -> &SqliteFrontier {
            self.runtime.database()
        }

        fn local_edit_transaction(&self, content: &str) -> OperationTransaction {
            OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: self.block_id,
                    home_document_id: self.home_document_id,
                },
                content: content.into(),
            }])
            .unwrap()
        }

        fn prepare_admitted_local(&mut self, batch_id: BatchId, content: &str) -> PreparedBatch {
            let transaction = self.local_edit_transaction(content);
            let page_home_hints = BTreeMap::from([(self.page_id, self.home_document_id)]);
            let prepared = {
                let mut session = self.runtime.admit_clean_mutation(&self.graph).unwrap();
                OperationalCoordinator::prepare_clean_trusted_local_compound(
                    &mut session,
                    &self.graph,
                    &self.receipts,
                    &transaction,
                    batch_id,
                    &page_home_hints,
                )
                .unwrap()
            };
            match prepared {
                PreparedLocalMutationState::Prepared(prepared) => prepared.into_trusted_batch(),
                PreparedLocalMutationState::ReconciliationRequired(_) => {
                    panic!("admitted local edit unexpectedly requires reconciliation")
                }
            }
        }

        fn local_author(&self, seed: u128) -> AuthorBatch {
            AuthorBatch {
                batch_id: BatchId::from_uuid(Uuid::from_u128(seed)),
                author_device_id: self.runtime.endpoint().device_id(),
                author_session_id: SessionId::from_uuid(Uuid::from_u128(seed + 1)),
                crdt_peer_id: CrdtPeerId::from_u64((seed as u64).saturating_add(10_001)),
                causal_peer_id: CausalPeerId::from_key(
                    crate::oplog::WriterIncarnationId::fixture_for_device(
                        self.runtime.endpoint().device_id(),
                    ),
                ),
            }
        }

        fn execute_local(
            &mut self,
            transaction: &OperationTransaction,
        ) -> Result<CleanLocalMutationState, OperationalCoordinatorError> {
            let mut session = self.runtime.admit_clean_mutation(&self.graph).unwrap();
            OperationalCoordinator::execute_clean_local(
                &mut session,
                &self.graph,
                &self.receipts,
                transaction,
                &mut self.projection_turns,
            )
        }

        fn execute_local_correlated(
            &mut self,
            batch_id: BatchId,
            transaction: &OperationTransaction,
        ) -> Result<CleanLocalMutationState, OperationalCoordinatorError> {
            let mut session = self.runtime.admit_clean_mutation(&self.graph).unwrap();
            OperationalCoordinator::execute_clean_local_correlated(
                &mut session,
                &self.graph,
                &self.receipts,
                batch_id,
                transaction,
                |_| Ok(()),
                &mut self.projection_turns,
            )
        }

        fn local_edit(
            &mut self,
            content: &str,
        ) -> Result<CleanLocalMutationState, OperationalCoordinatorError> {
            let transaction = self.local_edit_transaction(content);
            self.execute_local(&transaction)
        }

        fn execute_external(
            &mut self,
            paths: &[&str],
        ) -> Result<CleanExternalMutationState, OperationalCoordinatorError> {
            let mut session = self.runtime.admit_clean_mutation(&self.graph).unwrap();
            OperationalCoordinator::execute_clean_external(
                &mut session,
                &self.graph,
                &self.receipts,
                paths,
                &mut crate::oplog::absence_sweep::NoopSweepRecorder,
                &mut self.projection_turns,
            )
        }

        fn retry_clean(
            &mut self,
            continuation: CleanPublishedContinuation,
        ) -> CleanLocalMutationState {
            let mut session = self
                .runtime
                .admit_clean_derived_recovery(&self.graph)
                .unwrap();
            OperationalCoordinator::retry_clean_local_with_turns(
                &mut session,
                &self.graph,
                &self.receipts,
                continuation,
                &mut self.projection_turns,
            )
        }

        fn assert_clean_drained(&self) {
            assert_eq!(
                self.database().frontier_root().unwrap(),
                self.engine().accepted_frontier_root().unwrap()
            );
            let source = RebuildSource::new(self.engine(), &self.archive).unwrap();
            let tail = TailOverlay::from_durable(self.database(), &source).unwrap();
            assert_eq!(tail.status().unapplied_batches, 0);
            assert_eq!(tail.status().retained_bytes, 0);
        }

        fn restart_clean_runtime(self) -> Self {
            let Self {
                _root,
                graph_root,
                archive_root,
                enrollment_root,
                database_path,
                graph,
                projection_turns,
                receipts,
                archive,
                runtime,
                lineage,
                catalog,
                page_id,
                home_document_id: _,
                block_id: _,
                path,
            } = self;
            let endpoint = receipts.endpoint_binding().unwrap();
            let workspace = runtime.engine().workspace_id();
            drop(runtime);
            drop(archive);
            let archive_parent = archive_root.parent().unwrap();
            let reopened = open_clean_activation(
                &enrollment_root,
                &archive_parent.join(crate::oplog::lazy_genesis::LAZY_GENESIS_BASELINE_DIRECTORY),
                &database_path,
                catalog,
                super::super::ReferenceCatalogPolicyV1::default(),
                &crate::config::ParseConfig::default(),
            )
            .unwrap()
            .expect("published clean coordinator activation reopens after restart");
            let (mut engine, baseline_projection, _) = reopened.into_parts();
            engine
                .attach_clean_archive_store(ObjectStore::open(&archive_root, workspace).unwrap())
                .unwrap();
            let baseline_root = engine.accepted_frontier_root().unwrap();
            let baseline_claim_source = super::super::sqlite::clean_genesis_materialized_read(
                &baseline_projection,
                &baseline_root,
            )
            .unwrap();
            let replayed = engine
                .replay_clean_committed_tail(&baseline_claim_source)
                .unwrap();
            drop(baseline_claim_source);
            let archive = ObjectStore::open(&archive_root, workspace).unwrap();
            let lease = WorkspaceRuntimeLease::acquire(&archive, workspace).unwrap();
            let projection = if replayed == 0 {
                let expected = engine.accepted_frontier_root().unwrap();
                LeasedWorkspaceProjection::adopt_clean_genesis(
                    lease,
                    &database_path,
                    ProjectionClaim::current(workspace, lineage),
                    &expected,
                    &archive,
                    &engine,
                    baseline_projection,
                    crate::config::ParseConfig::default(),
                )
                .map_err(|(_, error)| error)
                .unwrap()
            } else {
                drop(baseline_projection);
                let application_runtime = ApplicationRuntimeRoot::open_for_test(
                    &_root.path().join("clean-application-runtime"),
                )
                .unwrap();
                let source = RebuildSource::new(&engine, &archive).unwrap();
                LeasedWorkspaceProjection::open_under(lease, |slot| {
                    let opened = SqliteFrontier::open_or_rebuild_with_applier_slot(
                        &database_path,
                        &application_runtime,
                        ProjectionClaim::current(workspace, lineage),
                        source,
                        slot,
                    )?;
                    Ok::<_, super::super::SqliteProjectionError>((opened, ()))
                })
                .map(|(projection, ())| projection)
                .map_err(|(_, error)| error)
                .unwrap()
            };
            engine
                .attach_clean_projection_endpoint(&graph, &receipts)
                .unwrap();
            let runtime = CleanLocalRuntime::from_open_parts(
                SessionId::from_uuid(Uuid::from_u128(7)),
                endpoint,
                engine,
                projection,
            )
            .unwrap();
            let page = runtime.engine().materialize_page(page_id).unwrap();
            let home_document_id = page.home_document_id;
            let block_id = page.blocks[0].block_id;
            Self {
                _root,
                graph_root,
                archive_root,
                enrollment_root,
                database_path,
                graph,
                projection_turns,
                receipts,
                archive,
                runtime,
                lineage,
                catalog,
                page_id,
                home_document_id,
                block_id,
                path,
            }
        }
    }

    fn without_projection_objects(prepared: &PreparedBatch) -> PreparedBatch {
        let objects = prepared
            .objects()
            .iter()
            .filter(|object| {
                !matches!(
                    object.kind(),
                    ObjectKind::ProjectionIntent | ObjectKind::AnnotatedBaseBlob
                )
            })
            .cloned()
            .collect::<Vec<OperationObject>>();
        let manifest = prepared.manifest();
        let manifest = OperationBatch::new_with_causality(
            manifest.workspace_id(),
            manifest.lineage_digest(),
            manifest.batch_id(),
            manifest.author_device_id(),
            manifest.author_session_id(),
            manifest.origin(),
            manifest.causal_dot(),
            manifest.causal_dependency_heads().to_vec(),
            manifest.dependency_frontier().clone(),
            manifest.semantic_effect_digest(),
            objects
                .iter()
                .map(OperationObject::descriptor)
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
        )
        .unwrap();
        PreparedBatch::new(manifest, objects).unwrap()
    }

    fn stage_admitted_candidate(
        fixture: &mut CleanCoordinatorFixture,
        prepared: PreparedBatch,
    ) -> BatchDisposition {
        let mut session = fixture
            .runtime
            .admit_clean_mutation(&fixture.graph)
            .unwrap();
        let (_, engine, _) = session.parts().unwrap();
        engine
            .stage_ready(ValidatedBatch::new(prepared))
            .disposition()
    }

    fn clean_coordinator_source(path: &str, root: &str, child: &str) -> Vec<u8> {
        if path.ends_with(".org") {
            format!("* {root}\n** {child}\n").into_bytes()
        } else {
            format!("- {root}\n  - {child}\n").into_bytes()
        }
    }

    fn expect_clean_local_complete(state: CleanLocalMutationState) -> BatchId {
        match state {
            CleanLocalMutationState::Complete(batch_id) => batch_id,
            CleanLocalMutationState::DurablePending(pending) => {
                panic!(
                    "unexpected durable clean local continuation: {}",
                    pending.failure()
                )
            }
        }
    }

    #[test]
    fn admitted_missing_projection_intent_refuses_live_causal_base_even_after_receiver_deletion() {
        assert!(
            std::env::var_os("TINE_SQLITE_HELPER_MODE").is_none(),
            "SQLite helper mode must not authorize this production-path negative"
        );

        let mut live = CleanCoordinatorFixture::new("missing-live-projection-intent");
        let live_batch_id = BatchId::from_uuid(Uuid::from_u128(0xc5102_3200));
        let live_prepared = live.prepare_admitted_local(live_batch_id, "missing live intent");
        assert!(live_prepared
            .objects()
            .iter()
            .any(|object| object.kind() == ObjectKind::ProjectionIntent));
        assert!(!crate::oplog::hot_engine::is_test_fixture_batch(
            live_batch_id
        ));
        let live_missing = without_projection_objects(&live_prepared);
        assert!(matches!(
            stage_admitted_candidate(&mut live, live_missing),
            BatchDisposition::Rejected {
                error: EngineError::ProjectionManifest(ref detail)
            } if detail == "origin requires complete affected-path projection intents"
        ));

        let mut deleted_receiver =
            CleanCoordinatorFixture::new("missing-causal-live-projection-intent");
        let stale_batch_id = BatchId::from_uuid(Uuid::from_u128(0xc5102_3300));
        let stale_prepared =
            deleted_receiver.prepare_admitted_local(stale_batch_id, "authored while live");
        assert!(stale_prepared
            .objects()
            .iter()
            .any(|object| object.kind() == ObjectKind::ProjectionIntent));
        assert!(!crate::oplog::hot_engine::is_test_fixture_batch(
            stale_batch_id
        ));
        let stale_missing = without_projection_objects(&stale_prepared);

        let remote_device = DeviceId::from_uuid(Uuid::from_u128(0xc5102_3310));
        let remote_delete = deleted_receiver
            .engine()
            .prepare_fixture_transaction(
                AuthorBatch {
                    batch_id: BatchId::from_uuid(Uuid::from_u128(0xc5102_3311)),
                    author_device_id: remote_device,
                    author_session_id: SessionId::from_uuid(Uuid::from_u128(0xc5102_3312)),
                    crdt_peer_id: CrdtPeerId::from_u64(0xc5102_3313),
                    causal_peer_id: CausalPeerId::from_key(
                        crate::oplog::WriterIncarnationId::fixture_for_device(remote_device),
                    ),
                },
                &OperationTransaction::new(vec![SemanticOperation::DeletePage {
                    page_id: deleted_receiver.page_id,
                }])
                .unwrap(),
            )
            .unwrap();
        let (delete_outcome, stale_outcome) = {
            let mut session = deleted_receiver
                .runtime
                .admit_clean_mutation(&deleted_receiver.graph)
                .unwrap();
            let (_, engine, _) = session.parts().unwrap();
            let delete_outcome = engine
                .stage_ready(ValidatedBatch::new(remote_delete))
                .disposition();
            let stale_outcome = engine
                .stage_ready(ValidatedBatch::new(stale_missing))
                .disposition();
            (delete_outcome, stale_outcome)
        };
        assert!(matches!(delete_outcome, BatchDisposition::Accepted { .. }));
        assert!(matches!(
            deleted_receiver
                .engine()
                .materialize_page(deleted_receiver.page_id),
            Err(EngineError::PageDeleted(page_id)) if page_id == deleted_receiver.page_id
        ));

        assert!(matches!(
            stale_outcome,
            BatchDisposition::Rejected {
                error: EngineError::ProjectionManifest(ref detail)
            } if detail == "origin requires complete affected-path projection intents"
        ), "receiver-current tombstone accepted or misclassified the causal-live omission: {stale_outcome:?}");
    }

    fn expect_clean_local_pending(state: CleanLocalMutationState) -> CleanPublishedContinuation {
        match state {
            CleanLocalMutationState::DurablePending(pending) => pending,
            CleanLocalMutationState::Complete(_) => {
                panic!("unexpected completed clean local mutation")
            }
        }
    }

    fn expect_clean_external_complete(state: CleanExternalMutationState) -> BatchId {
        match state {
            CleanExternalMutationState::Complete(batch_id) => batch_id,
            CleanExternalMutationState::Noop => panic!("unexpected clean external no-op"),
            CleanExternalMutationState::DurablePending(pending) => panic!(
                "unexpected durable clean external continuation: {}",
                pending.failure()
            ),
        }
    }

    fn expect_clean_external_pending(
        state: CleanExternalMutationState,
    ) -> CleanPublishedContinuation {
        match state {
            CleanExternalMutationState::DurablePending(pending) => pending,
            CleanExternalMutationState::Noop => panic!("unexpected clean external no-op"),
            CleanExternalMutationState::Complete(_) => {
                panic!("unexpected completed clean external mutation")
            }
        }
    }

    fn settle_clean_local(
        fixture: &mut CleanCoordinatorFixture,
        mut state: CleanLocalMutationState,
    ) -> BatchId {
        for _ in 0..8 {
            match state {
                CleanLocalMutationState::Complete(batch_id) => return batch_id,
                CleanLocalMutationState::DurablePending(pending) => {
                    state = fixture.retry_clean(pending);
                }
            }
        }
        panic!("clean local mutation did not settle within the bounded turn budget")
    }

    #[test]
    fn fresh_nested_layout_reconcile_drains_history_sqlite_and_projection_clean() {
        let mut fixture = CleanCoordinatorFixture::configured("nested-success");
        let path = fixture.path.clone();
        fixture.overwrite(b"- root edited\n  - child edited\n");
        let batch_id = expect_clean_external_complete(fixture.execute_external(&[&path]).unwrap());
        let batch = match fixture.archive.inspect_batch(batch_id).unwrap() {
            BatchInspection::Ready(batch) => batch,
            other => panic!("clean external batch did not become Ready: {other:?}"),
        };
        let BatchOrigin::ExternalReconciliation { import_id } = batch.manifest().origin() else {
            panic!("clean external batch lost its external origin")
        };
        assert_eq!(batch_id, import_id.batch_id());
        fixture.assert_clean_drained();
        assert_eq!(
            fs::read(fixture.graph_root.join(path)).unwrap(),
            b"- root edited\n  - child edited\n"
        );
    }

    #[test]
    fn admitted_local_semantic_mutation_commits_history_sqlite_and_projection_once_clean() {
        let mut fixture = CleanCoordinatorFixture::configured("local-success");
        let path = fixture.path.clone();
        let manifests_before = fixture.archive.committed_manifests().unwrap().len();
        let accepted_before = fixture.engine().accepted_batch_count().unwrap();
        let sqlite_before = fixture.database().applied_batch_count().unwrap();
        let releases_before = fixture.graph.handoff_release_count();
        reset_projection_graph_test_counters();

        let batch_id =
            expect_clean_local_complete(fixture.local_edit("local semantic edit").unwrap());

        assert_eq!(
            fixture.archive.committed_manifests().unwrap().len(),
            manifests_before + 1
        );
        assert_eq!(
            fixture.engine().accepted_batch_count().unwrap(),
            accepted_before + 1
        );
        assert_eq!(
            fixture.database().applied_batch_count().unwrap(),
            sqlite_before + 1
        );
        assert!(fixture.database().contains_batch(batch_id).unwrap());
        let batch = match fixture.archive.inspect_batch(batch_id).unwrap() {
            BatchInspection::Ready(batch) => batch,
            other => panic!("clean local batch did not become Ready: {other:?}"),
        };
        assert_eq!(batch.manifest().origin(), BatchOrigin::LocalMutation);
        assert_eq!(projection_graph_test_counters().write_calls, 1);
        assert_eq!(fixture.graph.handoff_release_count(), releases_before + 1);
        assert_eq!(
            fs::read(fixture.graph_root.join(path)).unwrap(),
            clean_coordinator_source("pages/a.md", "local semantic edit", "child")
        );
        fixture.assert_clean_drained();
    }

    #[test]
    fn local_exact_path_drift_requests_reconciliation_without_publication_clean() {
        let mut fixture = CleanCoordinatorFixture::new("local-reconcile-first");
        let path = fixture.path.clone();
        fixture.overwrite(b"- externally moved local base\n  - child\n");
        let immutable_before = snapshot_immutable_publication(&fixture.archive_root);
        let frontier_before = fixture.engine().accepted_frontier_root().unwrap();
        let sqlite_before = fixture.database().frontier_root().unwrap();
        reset_projection_graph_test_counters();

        let error = match fixture.local_edit("must not overwrite external bytes") {
            Err(error) => error,
            Ok(_) => panic!("exact clean local path drift must request reconciliation"),
        };
        assert_eq!(error.phase(), OperationalPhase::Planning);
        assert!(error.detail().contains("requires external reconciliation"));
        assert_eq!(
            snapshot_immutable_publication(&fixture.archive_root),
            immutable_before
        );
        assert_eq!(
            fixture.engine().accepted_frontier_root().unwrap(),
            frontier_before
        );
        assert_eq!(fixture.database().frontier_root().unwrap(), sqlite_before);
        assert_eq!(projection_graph_test_counters().write_calls, 0);
        assert_eq!(
            fs::read(fixture.graph_root.join(path)).unwrap(),
            b"- externally moved local base\n  - child\n"
        );
        fixture.graph.probe_managed_text_writer().unwrap();
    }

    #[test]
    fn stale_local_binding_is_typed_blocked_before_any_writer_side_effect_clean() {
        let mut fixture = CleanCoordinatorFixture::new("local-stale-binding");
        let foreign_root = TestRoot::new("local-stale-binding-clean-foreign");
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
            fixture.engine().workspace_id(),
            foreign_endpoint,
        )
        .unwrap();
        let immutable_before = snapshot_immutable_publication(&fixture.archive_root);
        let frontier_before = fixture.engine().accepted_frontier_root().unwrap();
        let sqlite_before = fixture.database().frontier_root().unwrap();
        let transaction = fixture.local_edit_transaction("blocked stale binding");
        let mut session = fixture
            .runtime
            .admit_clean_mutation(&fixture.graph)
            .unwrap();

        let error = match OperationalCoordinator::execute_clean_local(
            &mut session,
            &fixture.graph,
            &foreign_receipts,
            &transaction,
            &mut fixture.projection_turns,
        ) {
            Err(error) => error,
            Ok(_) => panic!("a stale clean local binding must be typed before publication"),
        };
        assert_eq!(error.phase(), OperationalPhase::Bindings);
        assert_eq!(
            snapshot_immutable_publication(&fixture.archive_root),
            immutable_before
        );
        assert_eq!(
            fixture.engine().accepted_frontier_root().unwrap(),
            frontier_before
        );
        assert_eq!(fixture.database().frontier_root().unwrap(), sqlite_before);
        fixture.graph.probe_managed_text_writer().unwrap();
    }

    #[test]
    fn local_and_external_mutations_enter_the_identical_terminal_pipeline_clean() {
        let mut local = CleanCoordinatorFixture::new("shared-terminal-local");
        fail_once_at(OperationalFaultPoint::AfterManifest);
        let local_pending =
            expect_clean_local_pending(local.local_edit("shared terminal local").unwrap());
        let local_batch = local_pending.batch_id();
        let local_manifest = match local.archive.inspect_batch(local_batch).unwrap() {
            BatchInspection::Ready(batch) => batch,
            other => panic!("clean local publication is not Ready: {other:?}"),
        };
        assert_eq!(
            local_manifest.manifest().origin(),
            BatchOrigin::LocalMutation
        );
        assert_eq!(
            expect_clean_local_complete(local.retry_clean(local_pending)),
            local_batch
        );
        local.assert_clean_drained();

        let mut external = CleanCoordinatorFixture::new("shared-terminal-external");
        let path = external.path.clone();
        external.overwrite(b"- shared terminal external\n  - child\n");
        fail_once_at(OperationalFaultPoint::AfterManifest);
        let external_pending =
            expect_clean_external_pending(external.execute_external(&[&path]).unwrap());
        let external_batch = external_pending.batch_id();
        let external_manifest = match external.archive.inspect_batch(external_batch).unwrap() {
            BatchInspection::Ready(batch) => batch,
            other => panic!("clean external publication is not Ready: {other:?}"),
        };
        assert!(matches!(
            external_manifest.manifest().origin(),
            BatchOrigin::ExternalReconciliation { .. }
        ));
        assert_eq!(
            expect_clean_local_complete(external.retry_clean(external_pending)),
            external_batch
        );
        external.assert_clean_drained();
    }

    #[test]
    fn local_late_failure_retries_exact_publication_without_a_second_writer_clean() {
        for (index, point) in [
            OperationalFaultPoint::AfterManifest,
            OperationalFaultPoint::AfterSqliteApply,
            OperationalFaultPoint::BeforeProjection,
            OperationalFaultPoint::AfterProjection,
        ]
        .into_iter()
        .enumerate()
        {
            let mut fixture = CleanCoordinatorFixture::new(&format!("local-late-{point:?}"));
            let path = fixture.path.clone();
            let manifests_before = fixture.archive.committed_manifests().unwrap().len();
            let releases_before = fixture.graph.handoff_release_count();
            reset_projection_graph_test_counters();
            fail_once_at(OperationalFaultPoint::AfterManifest);
            let mut pending =
                expect_clean_local_pending(fixture.local_edit("late local edit").unwrap());
            let batch_id = pending.batch_id();
            assert_eq!(
                fixture.archive.committed_manifests().unwrap().len(),
                manifests_before + 1
            );
            assert!(fixture.graph.probe_managed_text_writer().is_err());

            if point != OperationalFaultPoint::AfterManifest {
                fail_once_at(point);
                pending = expect_clean_local_pending(fixture.retry_clean(pending));
                assert_eq!(pending.batch_id(), batch_id);
                assert_eq!(fixture.graph.handoff_release_count(), releases_before);
                assert!(fixture.graph.probe_managed_text_writer().is_err());
            }
            let completion = expect_clean_local_complete(fixture.retry_clean(pending));
            assert_eq!(
                completion, batch_id,
                "case {index} changed clean publication"
            );
            assert_eq!(
                fixture.archive.committed_manifests().unwrap().len(),
                manifests_before + 1,
                "late clean retry republished the local mutation"
            );
            assert!(projection_graph_test_counters().write_calls <= 1);
            assert_eq!(
                fs::read(fixture.graph_root.join(&path)).unwrap(),
                clean_coordinator_source(&path, "root", "late local edit")
            );
            fixture.graph.probe_managed_text_writer().unwrap();
            fixture.assert_clean_drained();
        }
    }

    #[test]
    fn local_semantic_paths_accept_nested_nonstandard_utf8_markdown_and_org_clean() {
        for (index, path) in [
            "content/pages/研究/über topic.md",
            "content/pages/研究/über topic.org",
        ]
        .into_iter()
        .enumerate()
        {
            let mut fixture = CleanCoordinatorFixture::new_at(
                &format!("local-utf-{index}"),
                path,
                Some(
                    "{:pages-directory \"content/pages\"\n\
                      :journals-directory \"content/journals\"}\n",
                ),
                ManagedTextKind::Page,
            );
            expect_clean_local_complete(fixture.local_edit("utf local edit").unwrap());
            let expected = if path.ends_with(".org") {
                clean_coordinator_source(path, "utf local edit", "child")
            } else {
                clean_coordinator_source(path, "root", "utf local edit")
            };
            assert_eq!(fs::read(fixture.graph_root.join(path)).unwrap(), expected);
            fixture.assert_clean_drained();
        }
    }

    #[test]
    fn in_place_catalog_derivation_publishes_the_same_markdown_and_org_source_clean() {
        for (index, path) in [
            "content/pages/研究/über topic.md",
            "content/pages/研究/über topic.org",
        ]
        .into_iter()
        .enumerate()
        {
            let mut fixture = CleanCoordinatorFixture::new_at(
                &format!("local-derivation-{index}"),
                path,
                Some(
                    "{:pages-directory \"content/pages\"\n\
                      :journals-directory \"content/journals\"}\n",
                ),
                ManagedTextKind::Page,
            );

            let edit_author = fixture.local_author(44_000 + index as u128 * 100);
            let edit = fixture.local_edit_transaction("TODO utf derivation edit [[Other Page]]");
            let observed = fixture.engine().assert_draft_matches_previous_derivation(
                edit_author,
                BatchOrigin::LocalMutation,
                &edit,
            );
            assert_eq!(observed.refused, None);
            assert_eq!(observed.optimized_catalog_copies, 0);
            assert!(observed.oracle_catalog_copies >= 1);
            let state = fixture.execute_local(&edit).unwrap();
            settle_clean_local(&mut fixture, state);
            let expected = if path.ends_with(".org") {
                clean_coordinator_source(path, "TODO utf derivation edit [[Other Page]]", "child")
            } else {
                clean_coordinator_source(path, "root", "TODO utf derivation edit [[Other Page]]")
            };
            assert_eq!(fs::read(fixture.graph_root.join(path)).unwrap(), expected);
            fixture.assert_clean_drained();

            let insert_author = fixture.local_author(44_050 + index as u128 * 100);
            let insert = OperationTransaction::new(vec![SemanticOperation::CreateBlock {
                block: BlockLocation {
                    block_id: BlockId::from_uuid(Uuid::from_u128(44_900 + index as u128)),
                    home_document_id: fixture.home_document_id,
                },
                page_id: fixture.page_id,
                parent: None,
                order: "b".into(),
                content: "DONE appended tail".into(),
            }])
            .unwrap();
            let observed = fixture.engine().assert_draft_matches_previous_derivation(
                insert_author,
                BatchOrigin::LocalMutation,
                &insert,
            );
            assert_eq!(observed.refused, None);
            assert_eq!(observed.optimized_catalog_copies, 0);
            assert!(observed.oracle_catalog_copies >= 1);
            let state = fixture.execute_local(&insert).unwrap();
            settle_clean_local(&mut fixture, state);
            fixture.assert_clean_drained();

            let settle_author = fixture.local_author(44_070 + index as u128 * 100);
            let settle = fixture.local_edit_transaction("TODO settled after deferral");
            let observed = fixture.engine().assert_draft_matches_previous_derivation(
                settle_author,
                BatchOrigin::LocalMutation,
                &settle,
            );
            assert_eq!(observed.refused, None);
            assert_eq!(observed.optimized_catalog_copies, 0);
            fail_once_at(OperationalFaultPoint::AfterManifest);
            let pending = expect_clean_local_pending(fixture.execute_local(&settle).unwrap());
            let batch_id = pending.batch_id();
            fail_once_at(OperationalFaultPoint::BeforeProjection);
            let pending = expect_clean_local_pending(fixture.retry_clean(pending));
            assert_eq!(pending.batch_id(), batch_id);
            assert_eq!(
                expect_clean_local_complete(fixture.retry_clean(pending)),
                batch_id
            );
            fixture.assert_clean_drained();

            let restarted = fixture.restart_clean_runtime();
            restarted.assert_clean_drained();
            assert_eq!(
                fs::read(restarted.graph_root.join(path)).unwrap(),
                fs::read(restarted.graph_root.join(&restarted.path)).unwrap()
            );
        }
    }

    #[test]
    fn local_continuation_drop_stays_closed_and_completion_releases_once_clean() {
        let mut dropped = CleanCoordinatorFixture::new("drop-local-published");
        let releases = dropped.graph.handoff_release_count();
        fail_once_at(OperationalFaultPoint::AfterManifest);
        let continuation =
            expect_clean_local_pending(dropped.local_edit("drop local continuation").unwrap());
        drop(continuation);
        assert_eq!(dropped.graph.handoff_release_count(), releases);
        assert!(dropped.graph.probe_managed_text_writer().is_err());

        let mut completed = CleanCoordinatorFixture::new("complete-local-once");
        let releases = completed.graph.handoff_release_count();
        expect_clean_local_complete(completed.local_edit("complete local once").unwrap());
        assert_eq!(completed.graph.handoff_release_count(), releases + 1);
        completed.graph.probe_managed_text_writer().unwrap();
        completed.graph.probe_managed_text_writer().unwrap();
        assert_eq!(completed.graph.handoff_release_count(), releases + 1);
    }

    #[test]
    fn retained_terminal_dispositions_are_blocked_while_progress_is_recovering_clean() {
        let mut fixture = CleanCoordinatorFixture::new("retained-clean-published");
        let releases = fixture.graph.handoff_release_count();
        let manifests_before = fixture.archive.committed_manifests().unwrap().len();
        fail_once_at(OperationalFaultPoint::AfterManifest);
        let mut pending =
            expect_clean_local_pending(fixture.local_edit("retained clean core").unwrap());
        let batch_id = pending.batch_id();
        let published = snapshot_immutable_publication(&fixture.archive_root);
        fail_repeatedly_at(OperationalFaultPoint::BeforeProjection, 2);

        for _ in 0..2 {
            pending = expect_clean_local_pending(fixture.retry_clean(pending));
            assert_eq!(pending.batch_id(), batch_id);
            assert_eq!(pending.failure().phase(), OperationalPhase::ProjectionDrain);
            assert_eq!(
                snapshot_immutable_publication(&fixture.archive_root),
                published
            );
            assert_eq!(fixture.graph.handoff_release_count(), releases);
            assert!(fixture.graph.probe_managed_text_writer().is_err());
        }
        assert_eq!(
            fixture.archive.committed_manifests().unwrap().len(),
            manifests_before + 1
        );
        assert_eq!(
            expect_clean_local_complete(fixture.retry_clean(pending)),
            batch_id
        );
        assert_eq!(fixture.graph.handoff_release_count(), releases + 1);
        fixture.graph.probe_managed_text_writer().unwrap();
        fixture.assert_clean_drained();
    }

    #[test]
    fn rejected_published_local_batch_retains_typed_blocked_evidence_clean() {
        let mut fixture = CleanCoordinatorFixture::new("published-rejected-blocked");
        let sqlite_before = fixture.database().frontier_root().unwrap();
        let releases = fixture.graph.handoff_release_count();
        fail_once_at(OperationalFaultPoint::AfterManifest);
        let pending =
            expect_clean_local_pending(fixture.local_edit("durable history rejection").unwrap());
        let batch_id = pending.batch_id();
        let manifest = fixture
            .archive_root
            .join(crate::oplog::sync_layout::ARCHIVE_BATCHES_DIR)
            .join(format!("{batch_id}.manifest"));
        let held = manifest.with_extension("manifest.held");
        fs::rename(&manifest, &held).unwrap();

        let pending = expect_clean_local_pending(fixture.retry_clean(pending));
        assert_eq!(pending.batch_id(), batch_id);
        assert_eq!(pending.failure().phase(), OperationalPhase::ArchiveStage);
        assert_eq!(
            pending.failure().retained_block_reason(),
            Some(&RetainedBlockReason::PublishedAuthentication)
        );
        assert_eq!(fixture.database().frontier_root().unwrap(), sqlite_before);
        assert_eq!(fixture.graph.handoff_release_count(), releases);
        assert!(fixture.graph.probe_managed_text_writer().is_err());

        fs::rename(held, manifest).unwrap();
        assert_eq!(
            expect_clean_local_complete(fixture.retry_clean(pending)),
            batch_id
        );
        assert_eq!(fixture.graph.handoff_release_count(), releases + 1);
        fixture.assert_clean_drained();
    }

    #[test]
    fn stable_postpublication_binding_failure_retains_typed_blocked_continuation_clean() {
        let mut fixture = CleanCoordinatorFixture::new("local-stable-postpublication-binding");
        fail_once_at(OperationalFaultPoint::AfterManifest);
        let pending = expect_clean_local_pending(fixture.local_edit("stable binding").unwrap());
        let batch_id = pending.batch_id();
        let foreign_root = TestRoot::new("local-stable-postpublication-binding-clean-foreign");
        let foreign_graph_root = foreign_root.path().join("graph");
        fs::create_dir_all(&foreign_graph_root).unwrap();
        let foreign_graph = Graph::open(&foreign_graph_root);
        let mut session = fixture
            .runtime
            .admit_clean_derived_recovery(&fixture.graph)
            .unwrap();

        let pending =
            expect_clean_local_pending(OperationalCoordinator::retry_clean_local_with_turns(
                &mut session,
                &foreign_graph,
                &fixture.receipts,
                pending,
                &mut fixture.projection_turns,
            ));
        assert_eq!(pending.batch_id(), batch_id);
        assert_eq!(pending.failure().phase(), OperationalPhase::Bindings);
        assert!(fixture.graph.probe_managed_text_writer().is_err());
        assert_eq!(
            expect_clean_local_complete(fixture.retry_clean(pending)),
            batch_id
        );
        fixture.assert_clean_drained();
    }

    #[test]
    fn blocked_and_noop_cancel_without_durable_or_derived_mutation_clean() {
        let mut fixture = CleanCoordinatorFixture::new("blocked-noop");
        let accepted = fixture.engine().accepted_frontier_root().unwrap();
        let sqlite = fixture.database().frontier_root().unwrap();
        let graph = fs::read(fixture.graph_root.join(&fixture.path)).unwrap();
        let archive = snapshot_tree(&fixture.archive_root);
        let receipts = snapshot_tree(fixture.receipts.root_path());

        let blocked = fixture.execute_external(&["../escape.md"]);
        assert!(matches!(
            blocked,
            Err(OperationalCoordinatorError {
                phase: OperationalPhase::Planning,
                ..
            })
        ));
        let path = fixture.path.clone();
        assert!(matches!(
            fixture.execute_external(&[&path]).unwrap(),
            CleanExternalMutationState::Noop
        ));
        assert_eq!(fixture.engine().accepted_frontier_root().unwrap(), accepted);
        assert_eq!(fixture.database().frontier_root().unwrap(), sqlite);
        assert_eq!(
            fs::read(fixture.graph_root.join(&fixture.path)).unwrap(),
            graph
        );
        assert_eq!(snapshot_tree(&fixture.archive_root), archive);
        assert_eq!(snapshot_tree(fixture.receipts.root_path()), receipts);
        fixture.graph.probe_managed_text_writer().unwrap();
    }

    #[test]
    fn every_pre_manifest_boundary_releases_and_allows_fresh_retry_clean() {
        for (index, point) in [
            OperationalFaultPoint::AfterHandoff,
            OperationalFaultPoint::AfterDraft,
            OperationalFaultPoint::AfterCapture,
            OperationalFaultPoint::AfterFinalize,
        ]
        .into_iter()
        .enumerate()
        {
            let mut fixture = CleanCoordinatorFixture::new(&format!("pre-manifest-local-{index}"));
            let accepted = fixture.engine().accepted_frontier_root().unwrap();
            let sqlite = fixture.database().frontier_root().unwrap();
            let archive = snapshot_immutable_publication(&fixture.archive_root);
            fail_once_at(point);
            assert!(
                fixture.local_edit("changed local").is_err(),
                "{point:?} was not reached"
            );
            assert_eq!(fixture.engine().accepted_frontier_root().unwrap(), accepted);
            assert_eq!(fixture.database().frontier_root().unwrap(), sqlite);
            assert_eq!(
                snapshot_immutable_publication(&fixture.archive_root),
                archive
            );
            fixture.graph.probe_managed_text_writer().unwrap();
            expect_clean_local_complete(fixture.local_edit("changed local").unwrap());
            fixture.assert_clean_drained();
        }

        for (index, point) in [
            OperationalFaultPoint::AfterHandoff,
            OperationalFaultPoint::AfterPlan,
            OperationalFaultPoint::AfterDraft,
            OperationalFaultPoint::AfterCapture,
            OperationalFaultPoint::AfterFinalize,
        ]
        .into_iter()
        .enumerate()
        {
            let mut fixture =
                CleanCoordinatorFixture::new(&format!("pre-manifest-external-{index}"));
            let path = fixture.path.clone();
            fixture.overwrite(b"- changed external\n  - still nested\n");
            let accepted = fixture.engine().accepted_frontier_root().unwrap();
            let sqlite = fixture.database().frontier_root().unwrap();
            let archive = snapshot_immutable_publication(&fixture.archive_root);
            fail_once_at(point);
            assert!(
                fixture.execute_external(&[&path]).is_err(),
                "{point:?} was not reached"
            );
            assert_eq!(fixture.engine().accepted_frontier_root().unwrap(), accepted);
            assert_eq!(fixture.database().frontier_root().unwrap(), sqlite);
            assert_eq!(
                snapshot_immutable_publication(&fixture.archive_root),
                archive
            );
            fixture.graph.probe_managed_text_writer().unwrap();
            expect_clean_external_complete(fixture.execute_external(&[&path]).unwrap());
            fixture.assert_clean_drained();
        }
    }

    #[test]
    fn exact_reservation_precedes_manifest_and_object_only_cut_has_no_semantic_effect_clean() {
        let mut fixture = CleanCoordinatorFixture::new("objects-only");
        let sqlite = fixture.database().frontier_root().unwrap();
        let manifests = fixture.archive.committed_manifests().unwrap();
        let before = snapshot_tree(&fixture.archive_root);
        fail_next_publish_after_objects();
        let error = match fixture.local_edit("objects only") {
            Err(error) => error,
            Ok(_) => panic!("object-only clean publication cut unexpectedly committed"),
        };
        assert_eq!(error.phase(), OperationalPhase::Publication);
        assert_eq!(fixture.database().frontier_root().unwrap(), sqlite);
        assert_eq!(fixture.archive.committed_manifests().unwrap(), manifests);
        assert_ne!(snapshot_tree(&fixture.archive_root), before);
        assert!(
            fixture.engine().materialize_page(fixture.page_id).is_err(),
            "the speculative failed-commit engine must remain poisoned and unobservable"
        );
        fixture.graph.probe_managed_text_writer().unwrap();
    }

    #[test]
    fn every_post_manifest_failure_retains_guard_and_retries_idempotently_clean() {
        for (index, point) in [
            OperationalFaultPoint::AfterManifest,
            OperationalFaultPoint::AfterSqliteApply,
            OperationalFaultPoint::BeforeProjection,
            OperationalFaultPoint::AfterProjection,
        ]
        .into_iter()
        .enumerate()
        {
            let mut fixture = CleanCoordinatorFixture::new(&format!("post-manifest-{index}"));
            let path = fixture.path.clone();
            fixture.overwrite(b"- durable edit\n  - nested durable edit\n");
            let releases = fixture.graph.handoff_release_count();
            fail_once_at(OperationalFaultPoint::AfterManifest);
            let mut pending =
                expect_clean_external_pending(fixture.execute_external(&[&path]).unwrap());
            let batch_id = pending.batch_id();
            assert!(fixture.graph.probe_managed_text_writer().is_err());
            if point != OperationalFaultPoint::AfterManifest {
                fail_once_at(point);
                pending = expect_clean_local_pending(fixture.retry_clean(pending));
                assert_eq!(pending.batch_id(), batch_id);
                assert_eq!(fixture.graph.handoff_release_count(), releases);
                assert!(fixture.graph.probe_managed_text_writer().is_err());
            }
            assert_eq!(
                expect_clean_local_complete(fixture.retry_clean(pending)),
                batch_id
            );
            assert_eq!(fixture.graph.handoff_release_count(), releases + 1);
            fixture.graph.probe_managed_text_writer().unwrap();
            fixture.assert_clean_drained();
            assert_eq!(
                fs::read(fixture.graph_root.join(path)).unwrap(),
                b"- durable edit\n  - nested durable edit\n"
            );
        }
    }

    #[test]
    fn sqlite_budget_boundary_retains_handoff_and_resumes_without_republication_clean() {
        let mut fixture = CleanCoordinatorFixture::new("bounded-clean-resume");
        let releases = fixture.graph.handoff_release_count();
        let manifests_before = fixture.archive.committed_manifests().unwrap().len();
        fail_once_at(OperationalFaultPoint::AfterManifest);
        let mut pending =
            expect_clean_local_pending(fixture.local_edit("immutable clean retry").unwrap());
        let batch_id = pending.batch_id();
        let published = snapshot_immutable_publication(&fixture.archive_root);
        let published_count = fixture.archive.committed_manifests().unwrap().len();
        assert_eq!(published_count, manifests_before + 1);
        fail_repeatedly_at(OperationalFaultPoint::BeforeProjection, 3);

        for _ in 0..3 {
            assert_eq!(
                snapshot_immutable_publication(&fixture.archive_root),
                published
            );
            assert_eq!(
                fixture.archive.committed_manifests().unwrap().len(),
                published_count
            );
            assert_eq!(fixture.graph.handoff_release_count(), releases);
            assert!(fixture.graph.probe_managed_text_writer().is_err());
            pending = expect_clean_local_pending(fixture.retry_clean(pending));
            assert_eq!(pending.batch_id(), batch_id);
            assert_eq!(pending.failure().phase(), OperationalPhase::ProjectionDrain);
        }

        assert_eq!(
            expect_clean_local_complete(fixture.retry_clean(pending)),
            batch_id
        );
        assert_eq!(fixture.graph.handoff_release_count(), releases + 1);
        assert_eq!(
            snapshot_immutable_publication(&fixture.archive_root),
            published
        );
        assert_eq!(
            fixture.archive.committed_manifests().unwrap().len(),
            published_count
        );
        fixture.graph.probe_managed_text_writer().unwrap();
        fixture.graph.probe_managed_text_writer().unwrap();
        assert_eq!(fixture.graph.handoff_release_count(), releases + 1);
        fixture.assert_clean_drained();
    }

    #[test]
    fn published_continuation_survives_same_process_engine_reconstruction_clean() {
        let mut fixture = CleanCoordinatorFixture::new("engine-reconstruction");
        let path = fixture.path.clone();
        fixture.overwrite(b"- reconstructed continuation\n  - nested\n");
        let releases = fixture.graph.handoff_release_count();
        fail_once_at(OperationalFaultPoint::AfterManifest);
        let pending = expect_clean_external_pending(fixture.execute_external(&[&path]).unwrap());
        let batch_id = pending.batch_id();
        assert!(fixture.graph.probe_managed_text_writer().is_err());

        fixture = fixture.restart_clean_runtime();
        assert_eq!(fixture.graph.handoff_release_count(), releases);
        assert_eq!(
            expect_clean_local_complete(fixture.retry_clean(pending)),
            batch_id
        );
        assert_eq!(fixture.graph.handoff_release_count(), releases + 1);
        fixture.graph.probe_managed_text_writer().unwrap();
        fixture.assert_clean_drained();
        assert_eq!(
            fs::read(fixture.graph_root.join(path)).unwrap(),
            b"- reconstructed continuation\n  - nested\n"
        );
    }

    #[test]
    fn dropping_published_continuation_stays_closed_and_completion_releases_once_clean() {
        let mut dropped = CleanCoordinatorFixture::new("drop-published");
        let path = dropped.path.clone();
        dropped.overwrite(b"- durable dropped continuation\n");
        fail_once_at(OperationalFaultPoint::AfterManifest);
        let pending = expect_clean_external_pending(dropped.execute_external(&[&path]).unwrap());
        let releases = dropped.graph.handoff_release_count();
        drop(pending);
        assert_eq!(dropped.graph.handoff_release_count(), releases);
        assert!(dropped.graph.probe_managed_text_writer().is_err());

        let mut completed = CleanCoordinatorFixture::new("complete-once");
        let path = completed.path.clone();
        completed.overwrite(b"- successful explicit completion\n");
        let releases = completed.graph.handoff_release_count();
        expect_clean_external_complete(completed.execute_external(&[&path]).unwrap());
        assert_eq!(completed.graph.handoff_release_count(), releases + 1);
        completed.graph.probe_managed_text_writer().unwrap();
        completed.graph.probe_managed_text_writer().unwrap();
        assert_eq!(completed.graph.handoff_release_count(), releases + 1);
    }

    #[test]
    fn manifested_preconditions_are_exact_fresh_external_observations_clean() {
        let mut edit = CleanCoordinatorFixture::new("observed-precondition-edit");
        let path = edit.path.clone();
        let prior = fs::read(edit.graph_root.join(&path)).unwrap();
        let observed = b"- externally changed bytes\n  - current annotation source\n".to_vec();
        edit.overwrite(&observed);
        let batch_id = expect_clean_external_complete(edit.execute_external(&[&path]).unwrap());
        let batch = match edit.archive.inspect_batch(batch_id).unwrap() {
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

        let mut absent = CleanCoordinatorFixture::new("observed-precondition-absent");
        let path = absent.path.clone();
        fs::remove_file(absent.graph_root.join(&path)).unwrap();
        let batch_id = expect_clean_external_complete(absent.execute_external(&[&path]).unwrap());
        let batch = match absent.archive.inspect_batch(batch_id).unwrap() {
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

    /// Semantic replacement for the retired bounded fresh-peer probe.
    ///
    /// That test proved the probe skipped a zero/genesis candidate and gave up
    /// after eight collisions. Both properties belonged to the fresh-peer
    /// invariant: a colliding peer meant "already present in an affected
    /// document", which under persistent lanes is the NORMAL and required case.
    /// The property that replaces them is the one the rebaselining bound
    /// actually needs — repeated imports of the same document reuse one import
    /// lane, so its version vector does not grow per import — plus the refusals
    /// that stand in for the retired collision arm: the zero peer, and a peer
    /// belonging to somebody else's lane.
    #[test]
    fn repeated_external_imports_reuse_one_import_lane_and_refuse_foreign_peers() {
        let mut fixture = CleanCoordinatorFixture::new("import-lane");
        let path = fixture.path.clone();
        let endpoint = fixture.engine().projection_endpoint_binding().unwrap();
        let mut peers = std::collections::BTreeSet::new();
        for revision in 0..3 {
            fixture.overwrite(format!("- import lane edit {revision}\n").as_bytes());
            expect_clean_external_complete(fixture.execute_external(&[&path]).unwrap());
            let document = fixture
                .engine()
                .materialize_page(fixture.page_id)
                .unwrap()
                .home_document_id;
            peers.extend(
                fixture
                    .engine()
                    .accepted_document_peer_ids_for_test(document)
                    .unwrap(),
            );
        }
        // Lanes are qualified by the first admission, so read them afterwards.
        let lane = fixture
            .runtime
            .writer_lanes()
            .peer(WriterRole::External)
            .unwrap();
        let local_lane = fixture
            .runtime
            .writer_lanes()
            .peer(WriterRole::Local)
            .unwrap();
        assert_ne!(lane, local_lane);
        // Three reconciliations of one page, one import lane. A fresh peer per
        // import would have left three.
        assert!(peers.contains(&lane), "import lane authored the document");
        assert_eq!(
            peers.iter().filter(|peer| **peer == lane).count(),
            1,
            "one import lane, not one peer per import: {peers:?}"
        );
        assert_eq!(
            fixture
                .engine()
                .crdt_lane_owner(lane)
                .map(|owner| owner.role),
            Some(WriterRole::External)
        );

        // The retired collision arm's replacements. Refusing a foreign lane is
        // an ACCEPTED-STATE question, so the ordinary lane has to have actually
        // authored something before "an import may not advance it" can mean
        // anything; an unused lane is unowned and refuses nobody.
        let live_block = fixture
            .engine()
            .materialize_page(fixture.page_id)
            .unwrap()
            .blocks[0]
            .clone();
        let live_edit = OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
            block: BlockLocation {
                block_id: live_block.block_id,
                home_document_id: live_block.home_document_id,
            },
            content: "local lane owner edit".into(),
        }])
        .unwrap();
        expect_clean_local_complete(fixture.execute_local(&live_edit).unwrap());
        assert_eq!(
            fixture
                .engine()
                .crdt_lane_owner(local_lane)
                .map(|owner| owner.role),
            Some(WriterRole::Local),
            "the ordinary lane is bound by its own first admitted use"
        );
        fixture.overwrite(b"- refusal probe\n");
        let plan = plan_clean_affected_import(
            &fixture.graph,
            fixture.engine(),
            fixture.database(),
            &[&path],
        );
        let material = plan.into_execution_material().unwrap();
        let claim_source = fixture.database().materialized_read().unwrap();
        let zero = match draft_on_writer_lane(
            fixture.engine(),
            endpoint,
            &material,
            Some(&claim_source),
            CrdtPeerId::from_u64(0),
            fixture_import_incarnation(endpoint),
        ) {
            Err(error) => error,
            Ok(_) => panic!("the zero peer is never a lane"),
        };
        assert_eq!(zero.phase(), OperationalPhase::Draft);

        let foreign = match draft_on_writer_lane(
            fixture.engine(),
            endpoint,
            &material,
            Some(&claim_source),
            local_lane,
            fixture_import_incarnation(endpoint),
        ) {
            Err(error) => error,
            Ok(_) => panic!("an import may not advance this device's local lane"),
        };
        assert_eq!(foreign.phase(), OperationalPhase::Draft);
        assert!(
            foreign.detail().contains("writer lane"),
            "{}",
            foreign.detail()
        );
        drop(claim_source);
        fixture.graph.probe_managed_text_writer().unwrap();
    }

    #[test]
    fn delete_and_rename_project_exact_old_removal_and_new_render_base_clean() {
        let mut deletion = CleanCoordinatorFixture::new("delete");
        let delete_path = deletion.path.clone();
        fs::remove_file(deletion.graph_root.join(&delete_path)).unwrap();
        expect_clean_external_complete(deletion.execute_external(&[&delete_path]).unwrap());
        deletion.assert_clean_drained();
        assert!(!deletion.graph_root.join(delete_path).exists());

        let mut rename = CleanCoordinatorFixture::new("rename");
        let old = rename.path.clone();
        let new = "pages/elsewhere/deeper/renamed.md";
        fs::create_dir_all(rename.graph_root.join(new).parent().unwrap()).unwrap();
        fs::rename(rename.graph_root.join(&old), rename.graph_root.join(new)).unwrap();
        let batch_id =
            expect_clean_external_complete(rename.execute_external(&[&old, new]).unwrap());
        rename.assert_clean_drained();
        assert!(!rename.graph_root.join(&old).exists());
        assert_eq!(
            fs::read(rename.graph_root.join(new)).unwrap(),
            b"- root\n  - child\n"
        );
        let batch = match rename.archive.inspect_batch(batch_id).unwrap() {
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
        assert_eq!(base.bytes(), b"- root\n  - child\n");
    }

    #[test]
    fn binding_mismatch_rejects_before_handoff_or_publication_clean() {
        let mut fixture = CleanCoordinatorFixture::new("binding");
        let foreign_root = TestRoot::new("foreign-receipts-clean");
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
            fixture.engine().workspace_id(),
            foreign_endpoint,
        )
        .unwrap();
        let releases = fixture.graph.handoff_release_count();
        let transaction = fixture.local_edit_transaction("rejected binding");
        let mut session = fixture
            .runtime
            .admit_clean_mutation(&fixture.graph)
            .unwrap();
        assert!(matches!(
            OperationalCoordinator::execute_clean_local(
                &mut session,
                &fixture.graph,
                &foreign,
                &transaction,
                &mut fixture.projection_turns,
            ),
            Err(OperationalCoordinatorError {
                phase: OperationalPhase::Bindings,
                ..
            })
        ));
        assert_eq!(fixture.graph.handoff_release_count(), releases);
        fixture.graph.probe_managed_text_writer().unwrap();
        assert!(fixture.archive.committed_manifests().unwrap().is_empty());
    }

    #[test]
    fn post_manifest_retry_rejects_rebound_graph_and_keeps_original_guard_clean() {
        let mut fixture = CleanCoordinatorFixture::new("retry-binding");
        fail_once_at(OperationalFaultPoint::AfterManifest);
        let pending =
            expect_clean_local_pending(fixture.local_edit("durable retry binding").unwrap());
        let batch_id = pending.batch_id();

        let foreign_root = TestRoot::new("retry-binding-clean-foreign");
        let foreign_graph_root = foreign_root.path().join("graph");
        fs::create_dir_all(&foreign_graph_root).unwrap();
        let foreign_graph = Graph::open(&foreign_graph_root);
        let mut session = fixture
            .runtime
            .admit_clean_derived_recovery(&fixture.graph)
            .unwrap();
        let pending =
            expect_clean_local_pending(OperationalCoordinator::retry_clean_local_with_turns(
                &mut session,
                &foreign_graph,
                &fixture.receipts,
                pending,
                &mut fixture.projection_turns,
            ));
        assert_eq!(pending.batch_id(), batch_id);
        assert_eq!(pending.failure().phase(), OperationalPhase::Bindings);
        assert!(fixture.graph.probe_managed_text_writer().is_err());
        assert_eq!(
            expect_clean_local_complete(fixture.retry_clean(pending)),
            batch_id
        );
        fixture.graph.probe_managed_text_writer().unwrap();
        fixture.assert_clean_drained();
    }

    #[test]
    fn reordered_batch_ids_drain_by_authenticated_acceptance_sequence_clean() {
        let mut fixture = CleanCoordinatorFixture::new("acceptance-sequence");
        let first_id = BatchId::from_uuid(Uuid::from_u128(u128::MAX - 1));
        let second_id = BatchId::from_uuid(Uuid::from_u128(20));
        let first = fixture.local_edit_transaction("first accepted");
        assert_eq!(
            expect_clean_local_complete(
                fixture.execute_local_correlated(first_id, &first).unwrap()
            ),
            first_id
        );
        let second = fixture.local_edit_transaction("second accepted");
        assert_eq!(
            expect_clean_local_complete(
                fixture
                    .execute_local_correlated(second_id, &second)
                    .unwrap()
            ),
            second_id
        );
        assert!(first_id > second_id);
        let first_event =
            AcceptedBatchEvent::from_accepted(fixture.engine(), &fixture.archive, first_id)
                .unwrap();
        let second_event =
            AcceptedBatchEvent::from_accepted(fixture.engine(), &fixture.archive, second_id)
                .unwrap();
        assert!(first_event.acceptance_sequence() < second_event.acceptance_sequence());
        assert_eq!(
            fixture
                .engine()
                .accepted_batch_id_at(first_event.acceptance_sequence())
                .unwrap(),
            Some(first_id)
        );
        assert_eq!(
            fixture
                .engine()
                .accepted_batch_id_at(second_event.acceptance_sequence())
                .unwrap(),
            Some(second_id)
        );
        fixture.assert_clean_drained();
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
            // The clean-open checkpoint (Harvest A5) is a disposable
            // acceleration cache the engine may write at any time; it is not
            // part of the immutable publication surface this image compares.
            .filter(|name| name != "sealed-v3")
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

    /// Guards the "complete set" enumeration in `reprove_workspace_authority`'s
    /// doc comment. That sentence drifted once already: it named the five
    /// boundaries of `local_journal_drain.rs` while this file was calling six
    /// with a different mix, so the one call site reached from every entry
    /// point went uncounted.
    #[test]
    fn the_documented_authority_boundaries_are_the_complete_set() {
        let source = include_str!("operational_coordinator.rs");
        let production = source
            .split_once("\n#[cfg(test)]\nmod tests {")
            .expect("this file still has a test module")
            .0;

        let mut used: BTreeMap<&str, usize> = BTreeMap::new();
        for line in production.lines() {
            let Some((_, rest)) = line.split_once("WorkspaceAuthorityBoundary::") else {
                continue;
            };
            if line.trim_start().starts_with("///") {
                continue;
            }
            let boundary = rest.trim_end_matches(&[',', ')', ';'][..]);
            *used.entry(boundary).or_default() += 1;
        }

        let documented = BTreeMap::from([
            ("WindowAuthorization", 1),
            ("Publication", 3),
            ("SqliteDrain", 1),
            ("ProjectionDrain", 1),
        ]);
        assert_eq!(
            used, documented,
            "the coordinator's authority boundaries no longer match the enumeration in \
             reprove_workspace_authority's doc comment. Update that list — including its \
             count and the entry points it names — in the same change (invariant I-11: \
             code does not lie about itself)."
        );

        let calls = production.matches("reprove_workspace_authority(").count();
        assert_eq!(
            calls,
            documented.values().sum::<usize>() + 2,
            "reprove_workspace_authority's own definition, its inner admission call and \
             one call per documented boundary are all this file should contain; a new \
             call site must be added to that doc comment's enumeration too (I-11)."
        );
    }
}
