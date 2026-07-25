//! One-shot external reconciliation from a quiesced graph through every
//! authoritative and derived-state drain.
//!
//! This remains crate-private and deliberately has no startup, enrollment, or
//! application-routing surface.

#![allow(dead_code)] // activated only by the later persisted-enrollment packet

use std::fmt;
use std::sync::Arc;

use crate::model::PublishedHandoffLatch;
use crate::Graph;

use super::{
    plan_affected_import, AcceptedBatchEvent, AuthorBatch, BatchDisposition, BatchId,
    BatchInspection, BatchOrigin, ContentDigest, CrdtPeerId, ImportId, ImportPlan,
    ImportPlanStatus, ObjectStore, ProjectionEndpointBinding, ProjectionReceiptStore,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OperationalCoordinatorError {
    phase: OperationalPhase,
    detail: String,
}

impl OperationalCoordinatorError {
    fn new(phase: OperationalPhase, detail: impl Into<String>) -> Self {
        Self {
            phase,
            detail: detail.into(),
        }
    }

    pub(crate) const fn phase(&self) -> OperationalPhase {
        self.phase
    }

    pub(crate) fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for OperationalCoordinatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.phase, self.detail)
    }
}

impl std::error::Error for OperationalCoordinatorError {}

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
    FailedClosed(FailedClosedOperationalCoordinator),
}

/// Post-manifest retry state. It owns the original graph handoff guard and the
/// exact immutable publication identity; retry never redrafts or republishes.
pub(crate) struct FailedClosedOperationalCoordinator {
    guard: PublishedHandoffLatch,
    endpoint: ProjectionEndpointBinding,
    archive: Arc<ObjectStore>,
    batch_id: BatchId,
    import_id: ImportId,
    manifest_digest: ContentDigest,
    retained_bytes: usize,
    reservation: Option<TailReservation>,
    failure: OperationalCoordinatorError,
}

impl FailedClosedOperationalCoordinator {
    pub(crate) const fn batch_id(&self) -> BatchId {
        self.batch_id
    }

    pub(crate) const fn import_id(&self) -> ImportId {
        self.import_id
    }

    pub(crate) const fn phase(&self) -> OperationalPhase {
        self.failure.phase()
    }

    pub(crate) fn failure(&self) -> &OperationalCoordinatorError {
        &self.failure
    }

    pub(crate) fn retry(
        mut self,
        graph: &Graph,
        receipts: &ProjectionReceiptStore,
        engine: &mut ShardedHotEngine,
        database: &mut SqliteFrontier,
        tail: &mut TailOverlay,
    ) -> OperationalCoordinatorState {
        match self.resume(graph, receipts, engine, database, tail) {
            Ok(completion) => {
                self.guard.complete();
                OperationalCoordinatorState::Complete(completion)
            }
            Err(error) => {
                self.failure = error;
                OperationalCoordinatorState::FailedClosed(self)
            }
        }
    }

    fn resume(
        &mut self,
        graph: &Graph,
        receipts: &ProjectionReceiptStore,
        engine: &mut ShardedHotEngine,
        database: &mut SqliteFrontier,
        tail: &mut TailOverlay,
    ) -> Result<OperationalCompletion, OperationalCoordinatorError> {
        let mut budget = ResumeBudget::new();
        verify_bindings(graph, receipts, engine, self.endpoint, Some(&self.archive))?;
        self.guard
            .verify_binding(graph, engine.workspace_id(), self.endpoint)
            .map_err(|error| {
                OperationalCoordinatorError::new(OperationalPhase::Bindings, error.to_string())
            })?;
        authenticate_published(
            &self.archive,
            self.batch_id,
            self.import_id,
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
        let stage_limit = budget.remaining.saturating_sub(1) / 2;
        let stage = engine
            .stage_archive_batch_bounded(self.batch_id, stage_limit)
            .map_err(|error| {
                OperationalCoordinatorError::new(OperationalPhase::ArchiveStage, error.to_string())
            })?;
        budget.consume(stage.work(), OperationalPhase::ArchiveStage)?;
        fault(OperationalFaultPoint::AfterStage)?;
        if !matches!(
            stage.outcome().disposition(),
            BatchDisposition::Accepted { .. } | BatchDisposition::DuplicateAccepted { .. }
        ) {
            return Err(OperationalCoordinatorError::new(
                OperationalPhase::ArchiveStage,
                format!(
                    "published reconciliation {} did not accept: {:?}",
                    self.batch_id,
                    stage.outcome().disposition()
                ),
            ));
        }

        let mut events = stage
            .outcome()
            .newly_accepted()
            .iter()
            .map(|accepted| {
                AcceptedBatchEvent::from_accepted(engine, &self.archive, accepted.batch_id).map_err(
                    |error| {
                        OperationalCoordinatorError::new(
                            OperationalPhase::TailAdmission,
                            error.to_string(),
                        )
                    },
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        // A live reservation means the published event has not been admitted
        // yet. Adding it here is what makes the single in-loop retained-byte
        // check and reserved admission the only such path.
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
        for event in events {
            budget.consume(1, OperationalPhase::TailAdmission)?;
            if event.batch_id() == self.batch_id {
                if event.retained_bytes() != self.retained_bytes {
                    return Err(OperationalCoordinatorError::new(
                        OperationalPhase::TailAdmission,
                        "published accepted event retained bytes differ from the reserved prepared batch",
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
            return Err(OperationalCoordinatorError::new(
                OperationalPhase::TailAdmission,
                "published reservation survived tail admission of the published event",
            ));
        }
        fault(OperationalFaultPoint::AfterTailAdmission)?;
        // Load-bearing order: newly accepted events and the exact published
        // reservation are admitted to the tail *before* a staging continuation
        // is reported. Reversing this would return with the reservation still
        // live, so the next resume would re-reserve tail capacity that the
        // published batch already owns. Admission is idempotent per acceptance
        // sequence, so admitting first and then failing closed is safe.
        if stage.has_more() {
            return Err(OperationalCoordinatorError::new(
                OperationalPhase::ArchiveStage,
                format!(
                    "bounded staging slice consumed {} operations and has durable ready/fanout continuation",
                    stage.work()
                ),
            ));
        }

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
        Ok(OperationalCompletion {
            batch_id: self.batch_id,
            import_id: self.import_id,
        })
    }
}

pub(crate) struct OperationalCoordinator;

impl OperationalCoordinator {
    pub(crate) fn execute(
        graph: &Graph,
        receipts: &ProjectionReceiptStore,
        engine: &mut ShardedHotEngine,
        database: &mut SqliteFrontier,
        tail: &mut TailOverlay,
        requested_paths: &[&str],
    ) -> Result<OperationalCoordinatorState, OperationalCoordinatorError> {
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

        let plan = plan_affected_import(graph, receipts, engine, requested_paths);
        fault(OperationalFaultPoint::AfterPlan)?;
        match plan.status() {
            ImportPlanStatus::Blocked => {
                handoff.cancel();
                return Ok(OperationalCoordinatorState::Blocked(plan));
            }
            ImportPlanStatus::Noop => {
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
        let captured = draft
            .capture_projection_inputs(engine, graph, receipts, endpoint)
            .map_err(|error| {
                OperationalCoordinatorError::new(OperationalPhase::Capture, error.to_string())
            })?;
        fault(OperationalFaultPoint::AfterCapture)?;
        let prepared = engine
            .finalize_external_import_transaction(draft, endpoint, captured, receipts)
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
        let retained_bytes = prepared.retained_bytes().map_err(|error| {
            OperationalCoordinatorError::new(OperationalPhase::TailReservation, error.to_string())
        })?;
        let reservation = tail
            .reserve_bound_mutation(database, engine, retained_bytes)
            .map_err(|error| {
                OperationalCoordinatorError::new(
                    OperationalPhase::TailReservation,
                    error.to_string(),
                )
            })?;
        if let Err(failure) = fault(OperationalFaultPoint::AfterReservation) {
            tail.cancel_reservation(reservation).map_err(|error| {
                OperationalCoordinatorError::new(
                    OperationalPhase::TailReservation,
                    error.to_string(),
                )
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
        let published_latch = guard.into_published_latch();

        if let Err(error) = archive.publish_prepared(&prepared) {
            let publication = archive.inspect_batch(author.batch_id);
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
            let failed = FailedClosedOperationalCoordinator {
                guard: published_latch,
                endpoint,
                archive,
                batch_id: author.batch_id,
                import_id,
                manifest_digest,
                retained_bytes,
                reservation: Some(reservation),
                failure: OperationalCoordinatorError::new(
                    OperationalPhase::Publication,
                    error.to_string(),
                ),
            };
            return Ok(OperationalCoordinatorState::FailedClosed(failed));
        }
        let boundary = fault(OperationalFaultPoint::AfterManifest);
        let mut coordinator = FailedClosedOperationalCoordinator {
            guard: published_latch,
            endpoint,
            archive,
            batch_id: author.batch_id,
            import_id,
            manifest_digest,
            retained_bytes,
            reservation: Some(reservation),
            failure: boundary.clone().err().unwrap_or_else(|| {
                OperationalCoordinatorError::new(
                    OperationalPhase::ArchiveStage,
                    "published reconciliation is awaiting derived-state drains",
                )
            }),
        };
        if let Err(failure) = boundary {
            coordinator.failure = failure;
            return Ok(OperationalCoordinatorState::FailedClosed(coordinator));
        }
        match coordinator.resume(graph, receipts, engine, database, tail) {
            Ok(completion) => {
                coordinator.guard.complete();
                Ok(OperationalCoordinatorState::Complete(completion))
            }
            Err(failure) => {
                coordinator.failure = failure;
                Ok(OperationalCoordinatorState::FailedClosed(coordinator))
            }
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
        return Err(OperationalCoordinatorError::new(
            OperationalPhase::Bindings,
            "graph, engine endpoint, or receipt namespace binding mismatch",
        ));
    }
    let (archive, index) = engine.enrolled_projection_runtime().map_err(|error| {
        OperationalCoordinatorError::new(OperationalPhase::Bindings, error.to_string())
    })?;
    if index.endpoint_id() != endpoint.endpoint_id()
        || index.graph_resource_id() != endpoint.graph_resource_id()
        || index.receipt_store_id() != receipts.store_id()
    {
        return Err(OperationalCoordinatorError::new(
            OperationalPhase::Bindings,
            "enrolled archive/projection runtime binding changed",
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
            return Err(OperationalCoordinatorError::new(
                OperationalPhase::Bindings,
                "enrolled archive resource identity changed",
            ));
        }
    }
    Ok(archive)
}

fn authenticate_published(
    archive: &ObjectStore,
    batch_id: BatchId,
    import_id: ImportId,
    manifest_digest: ContentDigest,
    retained_bytes: usize,
) -> Result<(), OperationalCoordinatorError> {
    let validated = match archive.inspect_batch(batch_id).map_err(|error| {
        OperationalCoordinatorError::new(OperationalPhase::Publication, error.to_string())
    })? {
        BatchInspection::Ready(validated) => validated,
        BatchInspection::Absent | BatchInspection::Staged { .. } => {
            return Err(OperationalCoordinatorError::new(
                OperationalPhase::Publication,
                "published reconciliation is not a complete immutable batch",
            ));
        }
    };
    let encoded = validated.manifest().encode().map_err(|error| {
        OperationalCoordinatorError::new(OperationalPhase::Publication, error.to_string())
    })?;
    if validated.manifest().batch_id() != batch_id
        || validated.manifest().origin() != (BatchOrigin::ExternalReconciliation { import_id })
        || ContentDigest::of(&encoded) != manifest_digest
    {
        return Err(OperationalCoordinatorError::new(
            OperationalPhase::Publication,
            "durable manifest differs from the failed-closed publication identity",
        ));
    }
    let actual = validated
        .objects()
        .iter()
        .try_fold(encoded.len(), |total, object| {
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
                            "durable retained-byte count overflowed",
                        )
                    })
                })
        })?;
    if actual != retained_bytes {
        return Err(OperationalCoordinatorError::new(
            OperationalPhase::Publication,
            "durable batch bytes differ from the reserved prepared-byte count",
        ));
    }
    Ok(())
}

#[cfg(test)]
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
    AfterTailAdmission,
    AfterSqliteApply,
    BeforeProjection,
    AfterProjection,
}

#[cfg(not(test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OperationalFaultPoint {
    AfterHandoff,
    AfterPlan,
    AfterDraft,
    AfterCapture,
    AfterFinalize,
    AfterReservation,
    AfterManifest,
    AfterStage,
    AfterTailAdmission,
    AfterSqliteApply,
    BeforeProjection,
    AfterProjection,
}

#[cfg(test)]
thread_local! {
    static OPERATIONAL_FAULT: std::cell::Cell<Option<OperationalFaultPoint>> =
        const { std::cell::Cell::new(None) };
    static OPERATIONAL_ACTION: std::cell::RefCell<
        Option<(OperationalFaultPoint, Box<dyn FnOnce()>)>,
    > = std::cell::RefCell::new(None);
}

#[cfg(test)]
pub(crate) fn fail_once_at(point: OperationalFaultPoint) {
    OPERATIONAL_FAULT.set(Some(point));
}

#[cfg(test)]
pub(crate) fn act_once_at(point: OperationalFaultPoint, action: impl FnOnce() + 'static) {
    OPERATIONAL_ACTION.with(|slot| {
        *slot.borrow_mut() = Some((point, Box::new(action)));
    });
}

#[cfg(test)]
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
    if OPERATIONAL_FAULT.get() == Some(point) {
        OPERATIONAL_FAULT.set(None);
        return Err(OperationalCoordinatorError::new(
            match point {
                OperationalFaultPoint::AfterHandoff => OperationalPhase::Bindings,
                OperationalFaultPoint::AfterPlan => OperationalPhase::Planning,
                OperationalFaultPoint::AfterDraft => OperationalPhase::Draft,
                OperationalFaultPoint::AfterCapture => OperationalPhase::Capture,
                OperationalFaultPoint::AfterFinalize => OperationalPhase::Finalize,
                OperationalFaultPoint::AfterReservation => OperationalPhase::TailReservation,
                OperationalFaultPoint::AfterManifest => OperationalPhase::Publication,
                OperationalFaultPoint::AfterStage => OperationalPhase::ArchiveStage,
                OperationalFaultPoint::AfterTailAdmission => OperationalPhase::TailAdmission,
                OperationalFaultPoint::AfterSqliteApply => OperationalPhase::SqliteDrain,
                OperationalFaultPoint::BeforeProjection
                | OperationalFaultPoint::AfterProjection => OperationalPhase::ProjectionDrain,
            },
            "deterministic operational fault",
        ));
    }
    Ok(())
}

#[cfg(not(test))]
fn fault(_: OperationalFaultPoint) -> Result<(), OperationalCoordinatorError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use uuid::Uuid;

    use super::*;
    use crate::oplog::object_store::fail_next_publish_after_objects;
    use crate::oplog::{
        write_projection_exact, AnnotatedProjectionBase, ApplicationRuntimeRoot, BlockId,
        BlockLocation, DeviceId, DocumentId, LineageDigest, LogicalPageName, ManagedPath,
        ManagedTextKind, ManifestProjectionPrecondition, ManifestedProjectionIntent, ObjectKind,
        OperationTransaction, PageId, ProjectionClaim, ProjectionEndpointId, SemanticOperation,
        TAIL_MAX_BYTES,
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

        fn new_at(label: &str, path: &str, config: Option<&str>, kind: ManagedTextKind) -> Self {
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
            let transaction = OperationTransaction::new(vec![
                SemanticOperation::CreatePage {
                    page_id,
                    home_document_id: home,
                    name: LogicalPageName::parse("Coordinator Page").unwrap(),
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
                &self.graph,
                &self.receipts,
                &mut self.engine,
                &mut self.database,
                &mut self.tail,
                paths,
            )
            .unwrap()
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

    fn expect_failed(state: OperationalCoordinatorState) -> FailedClosedOperationalCoordinator {
        match state {
            OperationalCoordinatorState::FailedClosed(failed) => failed,
            OperationalCoordinatorState::Blocked(plan) => {
                panic!("unexpected blocked plan: {:?}", plan.blocks())
            }
            OperationalCoordinatorState::Noop => panic!("unexpected no-op"),
            OperationalCoordinatorState::Complete(_) => panic!("unexpected completion"),
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
                assert_eq!(fixture.tail.status().retained_bytes, failed.retained_bytes);
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
            fixture.archive.publish_prepared(&prepared).unwrap();
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
            &foreign_graph,
            &fixture.receipts,
            &mut fixture.engine,
            &mut fixture.database,
            &mut fixture.tail,
        ));
        assert_eq!(failed.phase(), OperationalPhase::Bindings);
        assert!(fixture.graph.probe_managed_text_writer().is_err());
        expect_complete(failed.retry(
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
        fixture.archive.publish_prepared(&first).unwrap();
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
        fixture.archive.publish_prepared(&second).unwrap();
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
