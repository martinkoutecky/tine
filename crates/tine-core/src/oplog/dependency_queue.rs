use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::scratch_store::{
    ScratchAuthenticatedPointRoot, ScratchCausalAccumulatorRoot, ScratchPageKind, ScratchRoots,
    ScratchStore,
};
use super::{BatchId, CausalPeerId, ContentDigest};

/// Version 6 replaces timing-dependent rejected-parent identity with a
/// commitment to the complete canonical dependency sequence, and replaces the
/// growing inline causal clock with a bounded root for a point-keyed sparse
/// accumulator.
///
/// The scratch namespace is run-local and newly minted for every engine open,
/// so no schema-4 migration or reopen path exists. A mismatched record can only
/// come from a namespace this build did not create and is rejected.
const DEPENDENCY_QUEUE_SCHEMA_VERSION: u32 = 6;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CompactBatchStatus {
    /// Direct-dependency registration is point-paged and incomplete.
    Registering,
    Waiting,
    Ready,
    Processing,
    Final,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StagedBatchRecord {
    schema_version: u32,
    batch_id: BatchId,
    manifest_fingerprint: ContentDigest,
    event_binding_digest: ContentDigest,
    dependency_set_commitment: ContentDigest,
    dependency_count: u32,
    registered_ordinal: u32,
    unresolved_count: u32,
    dependency_rejected: bool,
    causal_accumulator_root: ScratchCausalAccumulatorRoot,
    causal_materialization_charged: bool,
    registration_work_remaining: Option<u64>,
    finalization_work_remaining: u64,
    status: CompactBatchStatus,
    final_status: Option<Vec<u8>>,
    final_dependency_status: Option<FinalDependencyStatus>,
}

impl StagedBatchRecord {
    pub(crate) const fn batch_id(&self) -> BatchId {
        self.batch_id
    }

    pub(crate) const fn manifest_fingerprint(&self) -> ContentDigest {
        self.manifest_fingerprint
    }

    pub(crate) const fn event_binding_digest(&self) -> ContentDigest {
        self.event_binding_digest
    }

    pub(crate) const fn dependency_count(&self) -> u32 {
        self.dependency_count
    }

    pub(crate) const fn registered_ordinal(&self) -> u32 {
        self.registered_ordinal
    }

    pub(crate) const fn unresolved_count(&self) -> u32 {
        self.unresolved_count
    }

    pub(crate) const fn dependency_set_commitment(&self) -> ContentDigest {
        self.dependency_set_commitment
    }

    pub(crate) const fn dependency_rejected(&self) -> bool {
        self.dependency_rejected
    }

    #[cfg(test)]
    pub(crate) const fn registration_work_remaining(&self) -> Option<u64> {
        self.registration_work_remaining
    }

    #[cfg(test)]
    pub(crate) const fn finalization_work_remaining(&self) -> u64 {
        self.finalization_work_remaining
    }

    pub(crate) const fn status(&self) -> CompactBatchStatus {
        self.status
    }

    pub(crate) fn final_status(&self) -> Option<&[u8]> {
        self.final_status.as_deref()
    }

    pub(crate) const fn final_dependency_status(&self) -> Option<FinalDependencyStatus> {
        self.final_dependency_status
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FinalDependencyStatus {
    Satisfied,
    Rejected,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DependencyClassification {
    Satisfied {
        causal_clock: Vec<(CausalPeerId, u64)>,
    },
    Pending,
    Rejected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RegistrationStep {
    AlreadyComplete,
    Registered {
        ordinal: u32,
        dependency: BatchId,
        classification: FinalDependencyStatusOrPending,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PrepaymentStep {
    pub consumed: usize,
    pub complete: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FinalDependencyStatusOrPending {
    Satisfied,
    Pending,
    Rejected,
}

/// Durable per-parent wait-edge ordinals.
///
/// Edges are appended at `registered` and consumed at `drained`, so undrained
/// edges are always the contiguous range `drained..registered`. Finding the
/// next edge of a final parent is therefore a single point lookup and never
/// walks tombstones left by already-drained edges.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitProgress {
    registered: u64,
    drained: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitEdge {
    child: BatchId,
    dependency_ordinal: u32,
}

/// One point-paged dependent-fanout step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FanoutStep {
    /// The durable fanout index is empty; no final parent owns a live edge.
    Idle,
    /// One fully drained parent left the durable fanout index.
    Retired(BatchId),
    /// Exactly one durable `(parent, child)` edge was authenticated and
    /// resolved.
    Resolved {
        parent: BatchId,
        child: BatchId,
        awakened: bool,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct QueueWork {
    pub wait_edge_visits: usize,
    pub ready_queue_residency: usize,
}

/// Number of final parents that still own at least one undrained wait edge.
pub(crate) const fn pending_fanout(roots: &ScratchRoots) -> u64 {
    roots.fanout_tail.saturating_sub(roots.fanout_head)
}

/// Number of staged records whose registration cursor has not reached the end
/// of their immutable dependency sequence.
pub(crate) const fn pending_registration(roots: &ScratchRoots) -> u64 {
    roots.registering_len
}

/// Create or reidentify one compact staged record without visiting, collecting,
/// sorting, cloning, or encoding any direct dependency.
///
/// The already-authenticated canonical manifest supplies only its dependency
/// count here. Its fingerprint binds that immutable sequence; individual
/// identities enter the authenticated point map one charged ordinal at a time.
pub(crate) fn begin_stage(
    store: &ScratchStore,
    roots: &ScratchRoots,
    batch_id: BatchId,
    manifest_fingerprint: ContentDigest,
    event_binding_digest: ContentDigest,
    dependency_set_commitment: ContentDigest,
    dependency_count: usize,
    finalization_work: u64,
) -> Result<(ScratchRoots, StagedBatchRecord, QueueWork), DependencyQueueError> {
    if let Some(existing) = lookup(store, roots, batch_id)? {
        if existing.manifest_fingerprint != manifest_fingerprint
            || existing.event_binding_digest != event_binding_digest
            || existing.dependency_set_commitment != dependency_set_commitment
            || usize::try_from(existing.dependency_count).ok() != Some(dependency_count)
        {
            return Err(DependencyQueueError::BatchCollision(batch_id));
        }
        return Ok((roots.clone(), existing, QueueWork::default()));
    }
    let dependency_count =
        u32::try_from(dependency_count).map_err(|_| DependencyQueueError::TooManyDependencies)?;
    let complete = dependency_count == 0;
    let record = StagedBatchRecord {
        schema_version: DEPENDENCY_QUEUE_SCHEMA_VERSION,
        batch_id,
        manifest_fingerprint,
        event_binding_digest,
        dependency_set_commitment,
        dependency_count,
        registered_ordinal: 0,
        unresolved_count: 0,
        dependency_rejected: false,
        causal_accumulator_root: ScratchCausalAccumulatorRoot::default(),
        causal_materialization_charged: complete,
        registration_work_remaining: None,
        finalization_work_remaining: finalization_work,
        status: if complete {
            CompactBatchStatus::Ready
        } else {
            CompactBatchStatus::Registering
        },
        final_status: None,
        final_dependency_status: None,
    };
    validate_record(&record)?;
    let mut next = roots.clone();
    next.batch_status_root = put_record(store, &next, &record)?;
    if complete {
        next = enqueue(store, &next, batch_id)?;
    } else {
        next.registering_len = next
            .registering_len
            .checked_add(1)
            .ok_or(DependencyQueueError::TooManyDependencies)?;
    }
    let ready_queue_residency = usize::try_from(next.ready_queue_len).unwrap_or(usize::MAX);
    Ok((
        next,
        record,
        QueueWork {
            wait_edge_visits: 0,
            ready_queue_residency,
        },
    ))
}

/// Visit exactly one canonical manifest dependency at the durable ordinal.
///
/// `manifest_fingerprint`, `ordinal`, and `dependency` are selected from the
/// same already-decoded immutable manifest by the exclusive engine mutation
/// that owns this call. The compact record authenticates the manifest binding
/// and exact cursor before the identity is appended to the point map.
///
/// A satisfied parent's sparse causal clock is the semantic work of this one
/// charged dependency: it is merged into the durable sparse accumulator here.
/// No dependency or unresolved collection is decoded or rewritten.
pub(crate) fn advance_registration(
    store: &ScratchStore,
    roots: &ScratchRoots,
    batch_id: BatchId,
    manifest_fingerprint: ContentDigest,
    ordinal: u32,
    dependency: BatchId,
    classify: impl FnOnce(BatchId) -> Result<DependencyClassification, DependencyQueueError>,
) -> Result<(ScratchRoots, StagedBatchRecord, RegistrationStep), DependencyQueueError> {
    let mut record =
        lookup(store, roots, batch_id)?.ok_or(DependencyQueueError::MissingRecord(batch_id))?;
    if record.status != CompactBatchStatus::Registering {
        return Ok((roots.clone(), record, RegistrationStep::AlreadyComplete));
    }
    if record.manifest_fingerprint != manifest_fingerprint
        || record.registered_ordinal != ordinal
        || ordinal >= record.dependency_count
        || dependency == batch_id
    {
        return Err(DependencyQueueError::MalformedRecord);
    }
    let point_key = dependency_point_key(batch_id, ordinal);
    if store
        .authenticated_point_lookup(
            &roots.dependency_root,
            ScratchPageKind::DependencyIdentity,
            &point_key,
        )?
        .is_some()
    {
        return Err(DependencyQueueError::MalformedRecord);
    }
    let classification = classify(dependency)?;
    if matches!(classification, DependencyClassification::Satisfied { .. })
        && record.registration_work_remaining != Some(0)
    {
        return Err(DependencyQueueError::MalformedRecord);
    }
    record.registration_work_remaining = None;
    let mut next = roots.clone();
    next.dependency_root = store.authenticated_point_apply(
        &next.dependency_root,
        ScratchPageKind::DependencyIdentity,
        &BTreeMap::from([(point_key.clone(), Some(encode_canonical(&dependency)?))]),
    )?;
    let result = match classification {
        DependencyClassification::Satisfied { causal_clock } => {
            merge_causal_clock(store, &mut record, &causal_clock)?;
            FinalDependencyStatusOrPending::Satisfied
        }
        DependencyClassification::Pending => {
            // Once a canonical earlier ordinal proved rejection, later pending
            // parents cannot change the deterministic result and therefore do
            // not acquire needless wait edges.
            if !record.dependency_rejected {
                next = append_wait_edge(store, &next, dependency, batch_id, ordinal)?;
                next.unresolved_dependency_root = store.authenticated_point_apply(
                    &next.unresolved_dependency_root,
                    ScratchPageKind::DependencyUnresolved,
                    &BTreeMap::from([(point_key, Some(encode_canonical(&dependency)?))]),
                )?;
                record.unresolved_count = record
                    .unresolved_count
                    .checked_add(1)
                    .ok_or(DependencyQueueError::TooManyDependencies)?;
            }
            FinalDependencyStatusOrPending::Pending
        }
        DependencyClassification::Rejected => {
            record.dependency_rejected = true;
            record.causal_materialization_charged = true;
            record.finalization_work_remaining = record.finalization_work_remaining.min(1);
            FinalDependencyStatusOrPending::Rejected
        }
    };
    record.registered_ordinal = record
        .registered_ordinal
        .checked_add(1)
        .ok_or(DependencyQueueError::TooManyDependencies)?;
    if record.dependency_rejected || record.registered_ordinal == record.dependency_count {
        record.status = if record.dependency_rejected || record.unresolved_count == 0 {
            CompactBatchStatus::Ready
        } else {
            CompactBatchStatus::Waiting
        };
        next.registering_len = next
            .registering_len
            .checked_sub(1)
            .ok_or(DependencyQueueError::MalformedRecord)?;
    }
    if record.status == CompactBatchStatus::Ready {
        charge_causal_materialization(&mut record)?;
    }
    next.batch_status_root = put_record(store, &next, &record)?;
    if record.status == CompactBatchStatus::Ready {
        next = enqueue(store, &next, batch_id)?;
    }
    Ok((
        next,
        record,
        RegistrationStep::Registered {
            ordinal,
            dependency,
            classification: result,
        },
    ))
}

/// Durably prepay the sparse-clock work for the current satisfied dependency.
///
/// The bound comes from the parent's fixed-size clock-length point record. A
/// caller may contribute any nonzero slice; the actual parent clock is read and
/// merged only after the stored remainder reaches zero.
pub(crate) fn prepay_registration(
    store: &ScratchStore,
    roots: &ScratchRoots,
    batch_id: BatchId,
    manifest_fingerprint: ContentDigest,
    ordinal: u32,
    required_work: u64,
    available: usize,
) -> Result<(ScratchRoots, StagedBatchRecord, PrepaymentStep), DependencyQueueError> {
    let mut record =
        lookup(store, roots, batch_id)?.ok_or(DependencyQueueError::MissingRecord(batch_id))?;
    if record.status != CompactBatchStatus::Registering
        || record.manifest_fingerprint != manifest_fingerprint
        || record.registered_ordinal != ordinal
        || required_work == 0
    {
        return Err(DependencyQueueError::MalformedRecord);
    }
    let remaining = match record.registration_work_remaining {
        Some(remaining) => remaining,
        None => required_work,
    };
    if remaining > required_work {
        return Err(DependencyQueueError::MalformedRecord);
    }
    let consumed_u64 = remaining.min(available as u64);
    let consumed = usize::try_from(consumed_u64).unwrap_or(available);
    if consumed == 0 {
        return Ok((
            roots.clone(),
            record,
            PrepaymentStep {
                consumed: 0,
                complete: remaining == 0,
            },
        ));
    }
    record.registration_work_remaining = Some(remaining - consumed_u64);
    let mut next = roots.clone();
    next.batch_status_root = put_record(store, &next, &record)?;
    Ok((
        next,
        record.clone(),
        PrepaymentStep {
            consumed,
            complete: record.registration_work_remaining == Some(0),
        },
    ))
}

/// Durably prepay whole-batch finalization. Execution may be coarse after this
/// phase, but no caller slice must be large enough to hold the whole bound.
pub(crate) fn prepay_finalization(
    store: &ScratchStore,
    roots: &ScratchRoots,
    batch_id: BatchId,
    available: usize,
) -> Result<(ScratchRoots, PrepaymentStep), DependencyQueueError> {
    let mut record =
        lookup(store, roots, batch_id)?.ok_or(DependencyQueueError::MissingRecord(batch_id))?;
    if record.status != CompactBatchStatus::Ready {
        return Err(DependencyQueueError::MalformedRecord);
    }
    let consumed_u64 = record.finalization_work_remaining.min(available as u64);
    let consumed = usize::try_from(consumed_u64).unwrap_or(available);
    if consumed == 0 {
        return Ok((
            roots.clone(),
            PrepaymentStep {
                consumed: 0,
                complete: record.finalization_work_remaining == 0,
            },
        ));
    }
    record.finalization_work_remaining -= consumed_u64;
    let mut next = roots.clone();
    next.batch_status_root = put_record(store, &next, &record)?;
    Ok((
        next,
        PrepaymentStep {
            consumed,
            complete: record.finalization_work_remaining == 0,
        },
    ))
}

/// Unbounded staging: `begin_stage` followed by `advance_registration` to
/// completion.
///
/// Production has exactly one staging path — the engine drives the same two
/// primitives with or without a resume budget — so this convenience exists only
/// to state and exercise the unbounded contract directly.
#[cfg(test)]
fn stage(
    store: &ScratchStore,
    roots: &ScratchRoots,
    batch_id: BatchId,
    manifest_fingerprint: ContentDigest,
    direct_dependencies: &[BatchId],
    classify: impl Fn(BatchId) -> Result<DependencyClassification, DependencyQueueError>,
) -> Result<(ScratchRoots, StagedBatchRecord, QueueWork), DependencyQueueError> {
    let (mut roots, mut record, work) = begin_stage(
        store,
        roots,
        batch_id,
        manifest_fingerprint,
        manifest_fingerprint,
        manifest_fingerprint,
        direct_dependencies.len(),
        0,
    )?;
    while record.status == CompactBatchStatus::Registering {
        let ordinal = record.registered_ordinal;
        let dependency = direct_dependencies[ordinal as usize];
        let classification = classify(dependency)?;
        if let DependencyClassification::Satisfied { causal_clock } = &classification {
            let (next, _advanced, _) = prepay_registration(
                store,
                &roots,
                batch_id,
                manifest_fingerprint,
                ordinal,
                causal_clock.len() as u64 + 1,
                usize::MAX,
            )?;
            roots = next;
        }
        let (next, advanced, _) = advance_registration(
            store,
            &roots,
            batch_id,
            manifest_fingerprint,
            ordinal,
            dependency,
            |_| Ok(classification),
        )?;
        roots = next;
        record = advanced;
    }
    let ready_queue_residency = usize::try_from(roots.ready_queue_len).unwrap_or(usize::MAX);
    Ok((
        roots,
        record,
        QueueWork {
            ready_queue_residency,
            ..work
        },
    ))
}

pub(crate) fn pop_ready(
    store: &ScratchStore,
    roots: &ScratchRoots,
) -> Result<(ScratchRoots, Option<BatchId>), DependencyQueueError> {
    if roots.ready_queue_len == 0 {
        return Ok((roots.clone(), None));
    }
    let batch_id = ready_at(store, roots, 0)?.ok_or(DependencyQueueError::MalformedRecord)?;
    let mut record =
        lookup(store, roots, batch_id)?.ok_or(DependencyQueueError::MissingRecord(batch_id))?;
    if record.status != CompactBatchStatus::Ready
        || record.finalization_work_remaining != 0
        || (record.unresolved_count != 0 && !record.dependency_rejected)
    {
        return Err(DependencyQueueError::MalformedRecord);
    }
    record.status = CompactBatchStatus::Processing;
    let mut next = roots.clone();
    let last_index = roots.ready_queue_len - 1;
    let last = ready_at(store, roots, last_index)?.ok_or(DependencyQueueError::MalformedRecord)?;
    let mut changes = BTreeMap::from([(ready_slot_key(last_index), None)]);
    if last_index != 0 {
        let mut hole = 0_u64;
        loop {
            let left = hole
                .checked_mul(2)
                .and_then(|index| index.checked_add(1))
                .ok_or(DependencyQueueError::TooManyDependencies)?;
            if left >= last_index {
                break;
            }
            let right = left + 1;
            let left_batch =
                ready_at(store, roots, left)?.ok_or(DependencyQueueError::MalformedRecord)?;
            let (child_index, child_batch) = if right < last_index {
                let right_batch =
                    ready_at(store, roots, right)?.ok_or(DependencyQueueError::MalformedRecord)?;
                if right_batch < left_batch {
                    (right, right_batch)
                } else {
                    (left, left_batch)
                }
            } else {
                (left, left_batch)
            };
            if last <= child_batch {
                break;
            }
            changes.insert(ready_slot_key(hole), Some(encode_canonical(&child_batch)?));
            hole = child_index;
        }
        changes.insert(ready_slot_key(hole), Some(encode_canonical(&last)?));
    }
    next.ready_queue_root = store.authenticated_point_apply(
        &next.ready_queue_root,
        ScratchPageKind::ReadyQueue,
        &changes,
    )?;
    next.ready_queue_len -= 1;
    next.batch_status_root = put_record(store, &next, &record)?;
    Ok((next, Some(batch_id)))
}

pub(crate) fn peek_ready(
    store: &ScratchStore,
    roots: &ScratchRoots,
) -> Result<Option<BatchId>, DependencyQueueError> {
    if roots.ready_queue_len == 0 {
        return Ok(None);
    }
    ready_at(store, roots, 0)
}

/// Unbounded finish convenience for callers with no resume budget.
///
/// It is exactly `begin_finish` followed by `advance_fanout` until the durable
/// fanout index is empty, so the bounded and unbounded paths share one durable
/// primitive. Draining the whole index rather than only this parent's edges is
/// a superset guarantee: an unbounded caller never leaves a final parent with
/// live wait edges behind.
#[cfg(test)]
pub(crate) fn finish(
    store: &ScratchStore,
    roots: &ScratchRoots,
    batch_id: BatchId,
    final_status: Vec<u8>,
    final_dependency_status: FinalDependencyStatus,
    parent_clock: impl Fn(BatchId) -> Result<Vec<(CausalPeerId, u64)>, DependencyQueueError>,
) -> Result<(ScratchRoots, Vec<BatchId>, QueueWork), DependencyQueueError> {
    let mut next = begin_finish(
        store,
        roots,
        batch_id,
        final_status,
        final_dependency_status,
    )?;
    let mut awakened = Vec::new();
    let mut wait_edge_visits = 0;
    loop {
        let (advanced, step) = advance_fanout(store, &next, &parent_clock)?;
        next = advanced;
        match step {
            FanoutStep::Idle => break,
            FanoutStep::Retired(_) => {}
            FanoutStep::Resolved {
                child,
                awakened: woke,
                ..
            } => {
                wait_edge_visits += 1;
                if woke {
                    awakened.push(child);
                }
            }
        }
    }
    let ready_queue_residency = usize::try_from(next.ready_queue_len).unwrap_or(usize::MAX);
    Ok((
        next,
        awakened,
        QueueWork {
            wait_edge_visits,
            ready_queue_residency,
        },
    ))
}

/// Mark one ready batch final without draining its dependent fanout.
///
/// When the parent still owns undrained wait edges it is appended to the
/// durable fanout index. That index, not any run-local map, is the sole
/// continuation authority: a reconstructed engine reading the same scratch
/// roots rediscovers the exact remaining `(parent, child)` work.
pub(crate) fn begin_finish(
    store: &ScratchStore,
    roots: &ScratchRoots,
    batch_id: BatchId,
    final_status: Vec<u8>,
    final_dependency_status: FinalDependencyStatus,
) -> Result<ScratchRoots, DependencyQueueError> {
    if final_status.is_empty() {
        return Err(DependencyQueueError::MalformedRecord);
    }
    let mut record =
        lookup(store, roots, batch_id)?.ok_or(DependencyQueueError::MissingRecord(batch_id))?;
    if record.status == CompactBatchStatus::Final {
        if record.final_status.as_deref() == Some(final_status.as_slice())
            && record.final_dependency_status == Some(final_dependency_status)
        {
            return Ok(roots.clone());
        }
        return Err(DependencyQueueError::MalformedRecord);
    }
    if record.status != CompactBatchStatus::Processing {
        return Err(DependencyQueueError::MalformedRecord);
    }
    if record.dependency_rejected && final_dependency_status != FinalDependencyStatus::Rejected {
        return Err(DependencyQueueError::MalformedRecord);
    }
    record.status = CompactBatchStatus::Final;
    record.final_status = Some(final_status);
    record.final_dependency_status = Some(final_dependency_status);
    let mut next = roots.clone();
    next.batch_status_root = put_record(store, &next, &record)?;
    let progress = wait_progress(store, &next, batch_id)?;
    if progress.drained < progress.registered {
        next.fanout_root = store.authenticated_point_apply(
            &next.fanout_root,
            ScratchPageKind::DependencyFanout,
            &BTreeMap::from([(
                fanout_slot_key(next.fanout_tail),
                Some(encode_canonical(&batch_id)?),
            )]),
        )?;
        next.fanout_tail = next
            .fanout_tail
            .checked_add(1)
            .ok_or(DependencyQueueError::TooManyDependencies)?;
    }
    Ok(next)
}

/// Take exactly one point-paged step over the durable dependent-fanout index.
///
/// Every step is a constant number of point lookups and point inserts: the
/// head slot names the parent, the parent's `(registered, drained)` cursor
/// names the exact next edge ordinal, and both the parent record and the child
/// record are authenticated before any mutation. No wait-edge range is scanned
/// and no run-local state participates.
pub(crate) fn advance_fanout(
    store: &ScratchStore,
    roots: &ScratchRoots,
    parent_clock: impl FnOnce(BatchId) -> Result<Vec<(CausalPeerId, u64)>, DependencyQueueError>,
) -> Result<(ScratchRoots, FanoutStep), DependencyQueueError> {
    if roots.fanout_head == roots.fanout_tail {
        return Ok((roots.clone(), FanoutStep::Idle));
    }
    let slot = fanout_slot_key(roots.fanout_head);
    let parent = store
        .authenticated_point_lookup(&roots.fanout_root, ScratchPageKind::DependencyFanout, &slot)?
        .ok_or(DependencyQueueError::MalformedRecord)
        .and_then(|bytes| decode_batch_id(&bytes))?;
    let parent_record =
        lookup(store, roots, parent)?.ok_or(DependencyQueueError::MissingRecord(parent))?;
    if parent_record.status != CompactBatchStatus::Final {
        return Err(DependencyQueueError::MalformedRecord);
    }
    let mut progress = wait_progress(store, roots, parent)?;
    let mut next = roots.clone();
    if progress.drained == progress.registered {
        next.fanout_root = store.authenticated_point_apply(
            &next.fanout_root,
            ScratchPageKind::DependencyFanout,
            &BTreeMap::from([(slot, None)]),
        )?;
        next.fanout_head += 1;
        return Ok((next, FanoutStep::Retired(parent)));
    }
    let edge_key = wait_edge_key(parent, progress.drained);
    let edge = store
        .authenticated_point_lookup(&roots.wait_root, ScratchPageKind::DependencyWait, &edge_key)?
        .ok_or(DependencyQueueError::MissingRecord(parent))
        .and_then(|bytes| decode_canonical::<WaitEdge>(&bytes))?;
    let child = edge.child;
    let mut child_record =
        lookup(store, roots, child)?.ok_or(DependencyQueueError::MissingRecord(child))?;
    if !matches!(
        child_record.status,
        CompactBatchStatus::Registering | CompactBatchStatus::Waiting
    ) && !child_record.dependency_rejected
    {
        return Err(DependencyQueueError::MalformedRecord);
    }
    let unresolved_key = dependency_point_key(child, edge.dependency_ordinal);
    let unresolved_parent = store
        .authenticated_point_lookup(
            &roots.unresolved_dependency_root,
            ScratchPageKind::DependencyUnresolved,
            &unresolved_key,
        )?
        .ok_or(DependencyQueueError::MalformedRecord)
        .and_then(|bytes| decode_batch_id(&bytes))?;
    let identity = dependency_at(store, roots, child, edge.dependency_ordinal)?
        .ok_or(DependencyQueueError::MalformedRecord)?;
    if unresolved_parent != parent || identity != parent || child_record.unresolved_count == 0 {
        return Err(DependencyQueueError::MalformedRecord);
    }
    if !child_record.dependency_rejected {
        match parent_record
            .final_dependency_status
            .ok_or(DependencyQueueError::MalformedRecord)?
        {
            FinalDependencyStatus::Satisfied => {
                merge_causal_clock(store, &mut child_record, &parent_clock(parent)?)?;
            }
            FinalDependencyStatus::Rejected => {
                child_record.dependency_rejected = true;
                child_record.causal_materialization_charged = true;
                child_record.finalization_work_remaining =
                    child_record.finalization_work_remaining.min(1);
            }
        }
    }
    child_record.unresolved_count -= 1;
    // A child whose registration is still paged must not enter the ready queue
    // even at zero unresolved dependencies; `advance_registration` enqueues it
    // when the cursor reaches the end of its immutable dependency sequence.
    let awakened = child_record.dependency_rejected
        && matches!(
            child_record.status,
            CompactBatchStatus::Registering | CompactBatchStatus::Waiting
        )
        || (child_record.unresolved_count == 0
            && child_record.status == CompactBatchStatus::Waiting);
    if awakened {
        if child_record.status == CompactBatchStatus::Registering {
            next.registering_len = next
                .registering_len
                .checked_sub(1)
                .ok_or(DependencyQueueError::MalformedRecord)?;
        }
        child_record.status = CompactBatchStatus::Ready;
        child_record.registration_work_remaining = None;
        charge_causal_materialization(&mut child_record)?;
    }
    next.batch_status_root = put_record(store, &next, &child_record)?;
    next.wait_root = store.authenticated_point_apply(
        &next.wait_root,
        ScratchPageKind::DependencyWait,
        &BTreeMap::from([(edge_key, None)]),
    )?;
    next.unresolved_dependency_root = store.authenticated_point_apply(
        &next.unresolved_dependency_root,
        ScratchPageKind::DependencyUnresolved,
        &BTreeMap::from([(unresolved_key, None)]),
    )?;
    progress.drained += 1;
    next.wait_progress_root = put_wait_progress(store, &next, parent, progress)?;
    if awakened {
        next = enqueue(store, &next, child)?;
    }
    Ok((
        next,
        FanoutStep::Resolved {
            parent,
            child,
            awakened,
        },
    ))
}

pub(crate) fn lookup(
    store: &ScratchStore,
    roots: &ScratchRoots,
    batch_id: BatchId,
) -> Result<Option<StagedBatchRecord>, DependencyQueueError> {
    store
        .authenticated_point_lookup(
            &roots.batch_status_root,
            ScratchPageKind::BatchStatus,
            batch_key(batch_id).as_slice(),
        )?
        .map(|bytes| {
            let record: StagedBatchRecord = decode_canonical(&bytes)?;
            validate_record(&record)?;
            if record.batch_id != batch_id {
                return Err(DependencyQueueError::MisboundRecord);
            }
            Ok(record)
        })
        .transpose()
}

pub(crate) fn current_fanout_parent(
    store: &ScratchStore,
    roots: &ScratchRoots,
) -> Result<Option<BatchId>, DependencyQueueError> {
    if roots.fanout_head == roots.fanout_tail {
        return Ok(None);
    }
    store
        .authenticated_point_lookup(
            &roots.fanout_root,
            ScratchPageKind::DependencyFanout,
            &fanout_slot_key(roots.fanout_head),
        )?
        .map(|bytes| decode_batch_id(&bytes))
        .transpose()
}

pub(crate) fn all_records(
    store: &ScratchStore,
    roots: &ScratchRoots,
) -> Result<Vec<StagedBatchRecord>, DependencyQueueError> {
    store
        .authenticated_point_materialize(&roots.batch_status_root, ScratchPageKind::BatchStatus)?
        .into_iter()
        .map(|(key, bytes)| {
            let record: StagedBatchRecord = decode_canonical(&bytes)?;
            validate_record(&record)?;
            if key != batch_key(record.batch_id) {
                return Err(DependencyQueueError::MisboundRecord);
            }
            Ok(record)
        })
        .collect()
}

pub(crate) fn dependency_at(
    store: &ScratchStore,
    roots: &ScratchRoots,
    batch_id: BatchId,
    ordinal: u32,
) -> Result<Option<BatchId>, DependencyQueueError> {
    store
        .authenticated_point_lookup(
            &roots.dependency_root,
            ScratchPageKind::DependencyIdentity,
            &dependency_point_key(batch_id, ordinal),
        )?
        .map(|bytes| decode_batch_id(&bytes))
        .transpose()
}

/// Materialize the legacy public missing-dependency disposition only after all
/// ordinals have already been charged and point-persisted. Bounded slices do
/// not call this helper.
pub(crate) fn unresolved_dependencies(
    store: &ScratchStore,
    roots: &ScratchRoots,
    batch_id: BatchId,
) -> Result<Vec<BatchId>, DependencyQueueError> {
    let record =
        lookup(store, roots, batch_id)?.ok_or(DependencyQueueError::MissingRecord(batch_id))?;
    if record.dependency_rejected
        || record.status == CompactBatchStatus::Registering
        || record.registered_ordinal != record.dependency_count
    {
        return Err(DependencyQueueError::MalformedRecord);
    }
    let mut unresolved = Vec::with_capacity(record.unresolved_count as usize);
    for ordinal in 0..record.dependency_count {
        let key = dependency_point_key(batch_id, ordinal);
        let Some(bytes) = store.authenticated_point_lookup(
            &roots.unresolved_dependency_root,
            ScratchPageKind::DependencyUnresolved,
            &key,
        )?
        else {
            continue;
        };
        let dependency = decode_batch_id(&bytes)?;
        if dependency_at(store, roots, batch_id, ordinal)? != Some(dependency) {
            return Err(DependencyQueueError::MalformedRecord);
        }
        unresolved.push(dependency);
    }
    if unresolved.len() != record.unresolved_count as usize {
        return Err(DependencyQueueError::MalformedRecord);
    }
    Ok(unresolved)
}

#[cfg(test)]
pub(crate) fn point_state_counts(
    store: &ScratchStore,
    roots: &ScratchRoots,
    batch_id: BatchId,
) -> Result<(usize, usize), DependencyQueueError> {
    let prefix = batch_key(batch_id);
    let identities = store
        .authenticated_point_materialize(
            &roots.dependency_root,
            ScratchPageKind::DependencyIdentity,
        )?
        .iter()
        .filter(|(key, _)| key.starts_with(&prefix))
        .count();
    let unresolved = store
        .authenticated_point_materialize(
            &roots.unresolved_dependency_root,
            ScratchPageKind::DependencyUnresolved,
        )?
        .iter()
        .filter(|(key, _)| key.starts_with(&prefix))
        .count();
    Ok((identities, unresolved))
}

fn put_record(
    store: &ScratchStore,
    roots: &ScratchRoots,
    record: &StagedBatchRecord,
) -> Result<ScratchAuthenticatedPointRoot, DependencyQueueError> {
    validate_record(record)?;
    store
        .authenticated_point_apply(
            &roots.batch_status_root,
            ScratchPageKind::BatchStatus,
            &BTreeMap::from([(batch_key(record.batch_id), Some(encode_canonical(record)?))]),
        )
        .map_err(Into::into)
}

fn enqueue(
    store: &ScratchStore,
    roots: &ScratchRoots,
    batch_id: BatchId,
) -> Result<ScratchRoots, DependencyQueueError> {
    let mut next = roots.clone();
    let mut hole = roots.ready_queue_len;
    let mut changes = BTreeMap::new();
    while hole != 0 {
        let parent = (hole - 1) / 2;
        let parent_batch =
            ready_at(store, roots, parent)?.ok_or(DependencyQueueError::MalformedRecord)?;
        if parent_batch <= batch_id {
            break;
        }
        changes.insert(ready_slot_key(hole), Some(encode_canonical(&parent_batch)?));
        hole = parent;
    }
    changes.insert(ready_slot_key(hole), Some(encode_canonical(&batch_id)?));
    next.ready_queue_root = store.authenticated_point_apply(
        &roots.ready_queue_root,
        ScratchPageKind::ReadyQueue,
        &changes,
    )?;
    next.ready_queue_len = next
        .ready_queue_len
        .checked_add(1)
        .ok_or(DependencyQueueError::TooManyDependencies)?;
    Ok(next)
}

fn ready_at(
    store: &ScratchStore,
    roots: &ScratchRoots,
    index: u64,
) -> Result<Option<BatchId>, DependencyQueueError> {
    store
        .authenticated_point_lookup(
            &roots.ready_queue_root,
            ScratchPageKind::ReadyQueue,
            &ready_slot_key(index),
        )?
        .map(|bytes| decode_batch_id(&bytes))
        .transpose()
}

fn ready_slot_key(index: u64) -> Vec<u8> {
    let mut key = vec![b'h'];
    key.extend_from_slice(&index.to_be_bytes());
    key
}

fn validate_record(record: &StagedBatchRecord) -> Result<(), DependencyQueueError> {
    if record.schema_version != DEPENDENCY_QUEUE_SCHEMA_VERSION
        || record.registered_ordinal > record.dependency_count
        || record.unresolved_count > record.registered_ordinal
        || (record.status == CompactBatchStatus::Registering)
            != (record.registered_ordinal < record.dependency_count && !record.dependency_rejected)
        || (matches!(
            record.status,
            CompactBatchStatus::Ready | CompactBatchStatus::Processing | CompactBatchStatus::Final
        ) && record.unresolved_count != 0
            && !record.dependency_rejected)
        || (record.status == CompactBatchStatus::Waiting
            && (record.unresolved_count == 0 || record.dependency_rejected))
        || (record.registration_work_remaining.is_some()
            && record.status != CompactBatchStatus::Registering)
        || (matches!(
            record.status,
            CompactBatchStatus::Ready | CompactBatchStatus::Processing | CompactBatchStatus::Final
        ) && !record.causal_materialization_charged)
        || (matches!(
            record.status,
            CompactBatchStatus::Registering | CompactBatchStatus::Waiting
        ) && record.causal_materialization_charged)
        || (matches!(
            record.status,
            CompactBatchStatus::Processing | CompactBatchStatus::Final
        ) && record.finalization_work_remaining != 0)
        || (record.status == CompactBatchStatus::Final)
            != (record.final_status.is_some() && record.final_dependency_status.is_some())
        || (record.status != CompactBatchStatus::Final
            && (record.final_status.is_some() || record.final_dependency_status.is_some()))
    {
        return Err(DependencyQueueError::MalformedRecord);
    }
    Ok(())
}

fn wait_progress(
    store: &ScratchStore,
    roots: &ScratchRoots,
    parent: BatchId,
) -> Result<WaitProgress, DependencyQueueError> {
    let Some(bytes) = store.authenticated_point_lookup(
        &roots.wait_progress_root,
        ScratchPageKind::DependencyWaitProgress,
        batch_key(parent).as_slice(),
    )?
    else {
        return Ok(WaitProgress::default());
    };
    let progress: WaitProgress = decode_canonical(&bytes)?;
    if progress.drained > progress.registered {
        return Err(DependencyQueueError::MalformedRecord);
    }
    Ok(progress)
}

fn put_wait_progress(
    store: &ScratchStore,
    roots: &ScratchRoots,
    parent: BatchId,
    progress: WaitProgress,
) -> Result<ScratchAuthenticatedPointRoot, DependencyQueueError> {
    store
        .authenticated_point_apply(
            &roots.wait_progress_root,
            ScratchPageKind::DependencyWaitProgress,
            &BTreeMap::from([(batch_key(parent), Some(encode_canonical(&progress)?))]),
        )
        .map_err(Into::into)
}

fn append_wait_edge(
    store: &ScratchStore,
    roots: &ScratchRoots,
    parent: BatchId,
    child: BatchId,
    dependency_ordinal: u32,
) -> Result<ScratchRoots, DependencyQueueError> {
    let mut progress = wait_progress(store, roots, parent)?;
    let mut next = roots.clone();
    next.wait_root = store.authenticated_point_apply(
        &next.wait_root,
        ScratchPageKind::DependencyWait,
        &BTreeMap::from([(
            wait_edge_key(parent, progress.registered),
            Some(encode_canonical(&WaitEdge {
                child,
                dependency_ordinal,
            })?),
        )]),
    )?;
    progress.registered = progress
        .registered
        .checked_add(1)
        .ok_or(DependencyQueueError::TooManyDependencies)?;
    next.wait_progress_root = put_wait_progress(store, &next, parent, progress)?;
    Ok(next)
}

fn batch_key(batch_id: BatchId) -> Vec<u8> {
    batch_id.as_uuid().as_bytes().to_vec()
}

fn dependency_point_key(batch_id: BatchId, ordinal: u32) -> Vec<u8> {
    let mut key = batch_key(batch_id);
    key.extend_from_slice(&ordinal.to_be_bytes());
    key
}

fn wait_edge_key(parent: BatchId, ordinal: u64) -> Vec<u8> {
    let mut key = batch_key(parent);
    key.extend_from_slice(&ordinal.to_be_bytes());
    key
}

fn fanout_slot_key(index: u64) -> Vec<u8> {
    let mut key = vec![b'p'];
    key.extend_from_slice(&index.to_be_bytes());
    key
}

fn decode_batch_id(bytes: &[u8]) -> Result<BatchId, DependencyQueueError> {
    decode_canonical(bytes)
}

fn merge_causal_clock(
    store: &ScratchStore,
    record: &mut StagedBatchRecord,
    parent: &[(CausalPeerId, u64)],
) -> Result<(), DependencyQueueError> {
    if parent.iter().any(|(_, counter)| *counter == 0)
        || parent.windows(2).any(|pair| pair[0].0 >= pair[1].0)
    {
        return Err(DependencyQueueError::MalformedRecord);
    }
    for (peer, counter) in parent {
        record.causal_accumulator_root = store.causal_accumulator_upsert_max(
            &record.causal_accumulator_root,
            causal_accumulator_key(*peer),
            *counter,
        )?;
    }
    Ok(())
}

fn charge_causal_materialization(
    record: &mut StagedBatchRecord,
) -> Result<(), DependencyQueueError> {
    if record.causal_materialization_charged {
        return Ok(());
    }
    if record.dependency_rejected {
        record.causal_materialization_charged = true;
        return Ok(());
    }
    record.finalization_work_remaining = record
        .finalization_work_remaining
        .checked_add(record.causal_accumulator_root.count())
        .ok_or(DependencyQueueError::TooManyDependencies)?;
    record.causal_materialization_charged = true;
    Ok(())
}

pub(crate) fn materialize_causal_accumulator(
    store: &ScratchStore,
    record: &StagedBatchRecord,
) -> Result<Vec<(CausalPeerId, u64)>, DependencyQueueError> {
    let mut clock = Vec::new();
    for (key, counter) in store.causal_accumulator_entries(&record.causal_accumulator_root)? {
        let peer =
            CausalPeerId::from_device_id(super::DeviceId::from_uuid(uuid::Uuid::from_bytes(key)));
        clock.push((peer, counter));
    }
    if clock.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
        return Err(DependencyQueueError::MalformedRecord);
    }
    Ok(clock)
}

pub(crate) fn encoded_record_len(record: &StagedBatchRecord) -> usize {
    encode_canonical(record)
        .expect("validated staged record remains encodable")
        .len()
}

fn causal_accumulator_key(peer: CausalPeerId) -> [u8; 16] {
    *peer.as_device_id().as_uuid().as_bytes()
}

fn encode_canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, DependencyQueueError> {
    postcard::to_allocvec(value).map_err(|_| DependencyQueueError::MalformedRecord)
}

fn decode_canonical<T: for<'de> Deserialize<'de> + Serialize>(
    bytes: &[u8],
) -> Result<T, DependencyQueueError> {
    let value: T =
        postcard::from_bytes(bytes).map_err(|_| DependencyQueueError::MalformedRecord)?;
    if encode_canonical(&value)? != bytes {
        return Err(DependencyQueueError::MalformedRecord);
    }
    Ok(value)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DependencyQueueError {
    Scratch(String),
    BatchCollision(BatchId),
    MissingRecord(BatchId),
    TooManyDependencies,
    MisboundRecord,
    MalformedRecord,
}

impl fmt::Display for DependencyQueueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scratch(error) => write!(f, "dependency scratch index failed: {error}"),
            Self::BatchCollision(batch) => write!(f, "batch fingerprint collision for {batch}"),
            Self::MissingRecord(batch) => write!(f, "missing staged record for {batch}"),
            Self::TooManyDependencies => f.write_str("batch dependency count exceeds u32"),
            Self::MisboundRecord => f.write_str("misbound dependency-queue record"),
            Self::MalformedRecord => f.write_str("malformed dependency-queue record"),
        }
    }
}

impl std::error::Error for DependencyQueueError {}

impl From<super::scratch_store::ScratchError> for DependencyQueueError {
    fn from(error: super::scratch_store::ScratchError) -> Self {
        Self::Scratch(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cap_std::ambient_authority;
    use cap_std::fs::Dir;
    use std::collections::BTreeSet;
    use uuid::Uuid;

    use crate::oplog::{DeviceId, WorkspaceId};

    #[test]
    fn correction11_n_children_before_parent_visits_each_wait_edge_once() {
        const CHILDREN: usize = 256;
        let path = std::env::temp_dir().join(format!("tine-dependency-queue-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        let archive = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
        let store =
            ScratchStore::open(&archive, WorkspaceId::from_uuid(Uuid::from_u128(1))).unwrap();
        let parent = BatchId::from_uuid(Uuid::from_u128(2));
        let mut roots = ScratchRoots::default();
        for index in 0..CHILDREN {
            let child = BatchId::from_uuid(Uuid::from_u128(100 + index as u128));
            let (next, record, _) = stage(
                &store,
                &roots,
                child,
                ContentDigest::of(child.as_uuid().as_bytes()),
                &[parent],
                |_| Ok(DependencyClassification::Pending),
            )
            .unwrap();
            assert_eq!(record.status(), CompactBatchStatus::Waiting);
            roots = next;
        }
        let (next, parent_record, parent_work) = stage(
            &store,
            &roots,
            parent,
            ContentDigest::of(parent.as_uuid().as_bytes()),
            &[],
            |_| Ok(DependencyClassification::Pending),
        )
        .unwrap();
        roots = next;
        assert_eq!(parent_record.status(), CompactBatchStatus::Ready);
        assert_eq!(parent_work.ready_queue_residency, 1);

        let (next, ready) = pop_ready(&store, &roots).unwrap();
        roots = next;
        assert_eq!(ready, Some(parent));
        let (next, awakened, work) = finish(
            &store,
            &roots,
            parent,
            vec![1],
            FinalDependencyStatus::Satisfied,
            |_| Ok(Vec::new()),
        )
        .unwrap();
        roots = next;
        assert_eq!(awakened.len(), CHILDREN);
        assert_eq!(work.wait_edge_visits, CHILDREN);
        assert_eq!(work.ready_queue_residency, CHILDREN);

        let mut observed = Vec::new();
        while let (next, Some(batch_id)) = pop_ready(&store, &roots).unwrap() {
            roots = next;
            observed.push(batch_id);
            roots = finish(
                &store,
                &roots,
                batch_id,
                vec![1],
                FinalDependencyStatus::Satisfied,
                |_| Ok(Vec::new()),
            )
            .unwrap()
            .0;
        }
        assert_eq!(observed.len(), CHILDREN);
        assert!(observed.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(roots.ready_queue_len, 0);
        assert_eq!(store.stats().scratch_syncs, 0);
        drop(store);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn bounded_finish_pages_fanout_and_recursive_wakeups_without_skips() {
        const FANOUT: usize = 23;
        const SLICE: usize = 6;
        let path = std::env::temp_dir().join(format!("tine-bounded-dependency-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        let archive = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
        let store =
            ScratchStore::open(&archive, WorkspaceId::from_uuid(Uuid::from_u128(51))).unwrap();
        let root = BatchId::from_uuid(Uuid::from_u128(52));
        let chain = (0..FANOUT)
            .map(|index| BatchId::from_uuid(Uuid::from_u128(1000 + index as u128)))
            .collect::<Vec<_>>();
        let mut roots = ScratchRoots::default();
        for index in (0..FANOUT).rev() {
            let parent = if index == 0 { root } else { chain[index - 1] };
            roots = stage(
                &store,
                &roots,
                chain[index],
                ContentDigest::of(chain[index].as_uuid().as_bytes()),
                &[parent],
                |_| Ok(DependencyClassification::Pending),
            )
            .unwrap()
            .0;
        }
        roots = stage(
            &store,
            &roots,
            root,
            ContentDigest::of(root.as_uuid().as_bytes()),
            &[],
            |_| Ok(DependencyClassification::Pending),
        )
        .unwrap()
        .0;

        // `roots` is the only state that crosses a slice boundary. Every
        // per-slice cursor is rediscovered from the authenticated durable
        // roots, which is exactly what a reconstructed engine can do.
        let mut completed = Vec::new();
        let mut resolved = Vec::new();
        let mut resumes = 0;
        loop {
            resumes += 1;
            let mut work = 0;
            while work < SLICE {
                let (next, step) = advance_fanout(&store, &roots, |_| Ok(Vec::new())).unwrap();
                if step != FanoutStep::Idle {
                    roots = next;
                    work += 1;
                    if let FanoutStep::Resolved { parent, child, .. } = step {
                        resolved.push((parent, child));
                    }
                    continue;
                }
                let (next, ready) = pop_ready(&store, &roots).unwrap();
                roots = next;
                let Some(ready) = ready else {
                    break;
                };
                roots = begin_finish(
                    &store,
                    &roots,
                    ready,
                    vec![1],
                    FinalDependencyStatus::Satisfied,
                )
                .unwrap();
                completed.push(ready);
                work += 1;
            }
            assert!(work <= SLICE);
            if pending_fanout(&roots) == 0 && roots.ready_queue_len == 0 {
                break;
            }
            assert!(resumes <= 4 * FANOUT);
        }
        assert_eq!(completed.len(), FANOUT + 1);
        assert_eq!(completed[0], root);
        assert_eq!(&completed[1..], chain.as_slice());
        assert_eq!(resolved.len(), FANOUT);
        assert_eq!(
            resolved.iter().copied().collect::<BTreeSet<_>>().len(),
            FANOUT,
            "every durable edge is resolved exactly once"
        );
        assert_eq!(pending_registration(&roots), 0);
        drop(store);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn registration_is_point_paged_and_resumes_from_the_durable_cursor() {
        const DEPENDENCIES: usize = 9;
        let path = std::env::temp_dir().join(format!("tine-paged-registration-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        let archive = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
        let store =
            ScratchStore::open(&archive, WorkspaceId::from_uuid(Uuid::from_u128(71))).unwrap();
        let child = BatchId::from_uuid(Uuid::from_u128(72));
        let fingerprint = ContentDigest::of(child.as_uuid().as_bytes());
        let parents = (0..DEPENDENCIES)
            .map(|index| BatchId::from_uuid(Uuid::from_u128(2000 + index as u128)))
            .collect::<Vec<_>>();
        let mut roots = ScratchRoots::default();

        // Opening the record visits no dependency at all.
        let (next, record, work) = begin_stage(
            &store,
            &roots,
            child,
            fingerprint,
            fingerprint,
            fingerprint,
            parents.len(),
            0,
        )
        .unwrap();
        roots = next;
        assert_eq!(record.status(), CompactBatchStatus::Registering);
        assert_eq!(record.registered_ordinal(), 0);
        assert_eq!(point_state_counts(&store, &roots, child).unwrap(), (0, 0));
        assert_eq!(work.wait_edge_visits, 0);
        assert_eq!(pending_registration(&roots), 1);
        let compact_record_len = encode_canonical(&record).unwrap().len();

        // A repeated offer of the identical manifest is idempotent and never
        // restarts or advances the cursor.
        let (next, repeated, work) = begin_stage(
            &store,
            &roots,
            child,
            fingerprint,
            fingerprint,
            fingerprint,
            parents.len(),
            0,
        )
        .unwrap();
        assert_eq!(repeated, record);
        assert_eq!(work, QueueWork::default());
        roots = next;

        // A different manifest under the same batch id is a collision, so the
        // registration sequence stays bound to the immutable manifest.
        assert_eq!(
            begin_stage(
                &store,
                &roots,
                child,
                ContentDigest::of(b"other manifest"),
                fingerprint,
                fingerprint,
                parents.len(),
                0,
            ),
            Err(DependencyQueueError::BatchCollision(child)),
        );

        // Each call appends exactly one identity point and at most one
        // unresolved point. The compact record encoding remains independent
        // of dependency count; no vector grows behind the unit counter.
        const PREFIX: usize = 4;
        let mut record = record;
        for ordinal in 0..PREFIX {
            let dependency = parents[ordinal];
            if ordinal == 0 {
                let (next, _advanced, prepayment) =
                    prepay_registration(&store, &roots, child, fingerprint, ordinal as u32, 1, 1)
                        .unwrap();
                roots = next;
                assert!(prepayment.complete);
            }
            let (next, advanced, step) = advance_registration(
                &store,
                &roots,
                child,
                fingerprint,
                ordinal as u32,
                dependency,
                |_| {
                    Ok(if ordinal == 0 {
                        DependencyClassification::Satisfied {
                            causal_clock: Vec::new(),
                        }
                    } else {
                        DependencyClassification::Pending
                    })
                },
            )
            .unwrap();
            roots = next;
            record = advanced;
            assert_eq!(
                step,
                RegistrationStep::Registered {
                    ordinal: ordinal as u32,
                    dependency,
                    classification: if ordinal == 0 {
                        FinalDependencyStatusOrPending::Satisfied
                    } else {
                        FinalDependencyStatusOrPending::Pending
                    },
                }
            );
            assert_eq!(record.status(), CompactBatchStatus::Registering);
            assert_eq!(record.registered_ordinal(), (ordinal + 1) as u32);
            assert_eq!(
                point_state_counts(&store, &roots, child).unwrap(),
                (ordinal + 1, ordinal)
            );
            assert_eq!(encode_canonical(&record).unwrap().len(), compact_record_len);
            assert_eq!(
                lookup(&store, &roots, child).unwrap().unwrap(),
                record,
                "the cursor is durable, not run-local"
            );
        }
        // Resume from the durable cursor alone.
        let mut visits = PREFIX;
        while record.status() == CompactBatchStatus::Registering {
            let ordinal = record.registered_ordinal();
            let dependency = parents[ordinal as usize];
            let (next, advanced, step) = advance_registration(
                &store,
                &roots,
                child,
                fingerprint,
                ordinal,
                dependency,
                |_| Ok(DependencyClassification::Pending),
            )
            .unwrap();
            roots = next;
            record = advanced;
            assert!(matches!(step, RegistrationStep::Registered { .. }));
            visits += 1;
            assert!(visits <= DEPENDENCIES);
        }
        assert_eq!(visits, DEPENDENCIES, "each dependency is visited once");
        assert_eq!(record.status(), CompactBatchStatus::Waiting);
        assert_eq!(pending_registration(&roots), 0);
        assert_eq!(record.unresolved_count(), (DEPENDENCIES - 1) as u32);
        assert_eq!(
            point_state_counts(&store, &roots, child).unwrap(),
            (DEPENDENCIES, DEPENDENCIES - 1)
        );
        assert_eq!(
            unresolved_dependencies(&store, &roots, child).unwrap(),
            parents[1..]
        );

        // A completed record never re-enters registration.
        let (_, again, step) = advance_registration(
            &store,
            &roots,
            child,
            fingerprint,
            DEPENDENCIES as u32,
            parents[0],
            |_| Ok(DependencyClassification::Rejected),
        )
        .unwrap();
        assert_eq!(step, RegistrationStep::AlreadyComplete);
        assert_eq!(again, record);
        drop(store);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn already_rejected_parent_rejects_child_without_a_wait_edge() {
        let path =
            std::env::temp_dir().join(format!("tine-late-rejected-parent-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        let archive = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
        let store =
            ScratchStore::open(&archive, WorkspaceId::from_uuid(Uuid::from_u128(81))).unwrap();
        let parent = BatchId::from_uuid(Uuid::from_u128(82));
        let child = BatchId::from_uuid(Uuid::from_u128(83));
        let fingerprint = ContentDigest::of(child.as_uuid().as_bytes());
        let (mut roots, _, _) = begin_stage(
            &store,
            &ScratchRoots::default(),
            child,
            fingerprint,
            fingerprint,
            fingerprint,
            1,
            0,
        )
        .unwrap();
        let (next, record, step) =
            advance_registration(&store, &roots, child, fingerprint, 0, parent, |_| {
                Ok(DependencyClassification::Rejected)
            })
            .unwrap();
        roots = next;
        assert_eq!(
            step,
            RegistrationStep::Registered {
                ordinal: 0,
                dependency: parent,
                classification: FinalDependencyStatusOrPending::Rejected,
            }
        );
        assert_eq!(record.status(), CompactBatchStatus::Ready);
        assert!(record.dependency_rejected());
        assert_eq!(record.dependency_set_commitment(), fingerprint);
        assert_eq!(point_state_counts(&store, &roots, child).unwrap(), (1, 0));
        assert_eq!(
            wait_progress(&store, &roots, parent).unwrap(),
            WaitProgress::default()
        );
        assert_eq!(pending_fanout(&roots), 0);
        assert_eq!(pending_registration(&roots), 0);

        let reconstructed = roots.clone();
        let (next, ready) = pop_ready(&store, &reconstructed).unwrap();
        assert_eq!(ready, Some(child));
        roots = begin_finish(
            &store,
            &next,
            child,
            vec![2],
            FinalDependencyStatus::Rejected,
        )
        .unwrap();
        assert_eq!(
            advance_fanout(&store, &roots, |_| panic!("rejected parent has no clock"))
                .unwrap()
                .1,
            FanoutStep::Idle
        );
        assert_eq!(pending_fanout(&roots), 0);
        drop(store);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn pending_parent_rejection_fanout_folds_the_same_rejected_witness() {
        let path =
            std::env::temp_dir().join(format!("tine-pending-rejected-parent-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        let archive = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
        let store =
            ScratchStore::open(&archive, WorkspaceId::from_uuid(Uuid::from_u128(91))).unwrap();
        let parent = BatchId::from_uuid(Uuid::from_u128(92));
        let child = BatchId::from_uuid(Uuid::from_u128(93));
        let child_fingerprint = ContentDigest::of(child.as_uuid().as_bytes());
        let mut roots = begin_stage(
            &store,
            &ScratchRoots::default(),
            child,
            child_fingerprint,
            child_fingerprint,
            child_fingerprint,
            1,
            0,
        )
        .unwrap()
        .0;
        roots = advance_registration(&store, &roots, child, child_fingerprint, 0, parent, |_| {
            Ok(DependencyClassification::Pending)
        })
        .unwrap()
        .0;
        assert_eq!(point_state_counts(&store, &roots, child).unwrap(), (1, 1));

        roots = stage(
            &store,
            &roots,
            parent,
            ContentDigest::of(parent.as_uuid().as_bytes()),
            &[],
            |_| Ok(DependencyClassification::Pending),
        )
        .unwrap()
        .0;
        let (next, ready) = pop_ready(&store, &roots).unwrap();
        assert_eq!(ready, Some(parent));
        roots = begin_finish(
            &store,
            &next,
            parent,
            vec![2],
            FinalDependencyStatus::Rejected,
        )
        .unwrap();
        let (next, step) =
            advance_fanout(&store, &roots, |_| panic!("rejected parent has no clock")).unwrap();
        roots = next;
        assert_eq!(
            step,
            FanoutStep::Resolved {
                parent,
                child,
                awakened: true,
            }
        );
        let child_record = lookup(&store, &roots, child).unwrap().unwrap();
        assert_eq!(child_record.status(), CompactBatchStatus::Ready);
        assert!(child_record.dependency_rejected());
        assert_eq!(child_record.dependency_set_commitment(), child_fingerprint);
        assert_eq!(point_state_counts(&store, &roots, child).unwrap(), (1, 0));
        drop(store);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn three_parent_two_rejection_timing_matrix_is_commitment_stable() {
        #[derive(Clone, Copy, Debug)]
        struct Case {
            registered_before_first_rejection: usize,
            rejection_order: [usize; 2],
        }

        fn finish_parent(
            store: &ScratchStore,
            roots: &mut ScratchRoots,
            parent: BatchId,
            status: FinalDependencyStatus,
        ) {
            *roots = stage(
                store,
                roots,
                parent,
                ContentDigest::of(parent.as_uuid().as_bytes()),
                &[],
                |_| Ok(DependencyClassification::Pending),
            )
            .unwrap()
            .0;
            let (next, ready) = pop_ready(store, roots).unwrap();
            assert_eq!(ready, Some(parent));
            *roots = begin_finish(store, &next, parent, vec![status as u8 + 1], status).unwrap();
        }

        fn drain_fanout(store: &ScratchStore, roots: &mut ScratchRoots) {
            loop {
                let (next, step) = advance_fanout(store, roots, |_| Ok(Vec::new())).unwrap();
                *roots = next;
                if step == FanoutStep::Idle {
                    break;
                }
            }
        }

        let cases = [0, 1, 3].into_iter().flat_map(|registered| {
            [[0, 1], [1, 0]].map(move |rejection_order| Case {
                registered_before_first_rejection: registered,
                rejection_order,
            })
        });
        let mut observed_commitment = None;
        for case in cases {
            let path = std::env::temp_dir().join(format!(
                "tine-rejection-timing-matrix-{}-{}",
                case.registered_before_first_rejection,
                Uuid::new_v4()
            ));
            std::fs::create_dir_all(&path).unwrap();
            let archive = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
            let store =
                ScratchStore::open(&archive, WorkspaceId::from_uuid(Uuid::from_u128(301))).unwrap();
            let child = BatchId::from_uuid(Uuid::from_u128(302));
            let parents = [
                BatchId::from_uuid(Uuid::from_u128(310)),
                BatchId::from_uuid(Uuid::from_u128(311)),
                BatchId::from_uuid(Uuid::from_u128(312)),
            ];
            let fingerprint = ContentDigest::of(b"three-parent-child");
            let commitment = ContentDigest::of(b"canonical-three-parent-dependency-set");
            let mut roots = begin_stage(
                &store,
                &ScratchRoots::default(),
                child,
                fingerprint,
                fingerprint,
                commitment,
                parents.len(),
                0,
            )
            .unwrap()
            .0;

            for (ordinal, parent) in parents
                .iter()
                .copied()
                .enumerate()
                .take(case.registered_before_first_rejection)
            {
                roots = advance_registration(
                    &store,
                    &roots,
                    child,
                    fingerprint,
                    ordinal as u32,
                    parent,
                    |_| Ok(DependencyClassification::Pending),
                )
                .unwrap()
                .0;
            }

            let first = parents[case.rejection_order[0]];
            finish_parent(&store, &mut roots, first, FinalDependencyStatus::Rejected);
            let (next, _) = advance_fanout(&store, &roots, |_| Ok(Vec::new())).unwrap();
            roots = next;

            let mut record = lookup(&store, &roots, child).unwrap().unwrap();
            while record.status() == CompactBatchStatus::Registering {
                let ordinal = record.registered_ordinal();
                let dependency = parents[ordinal as usize];
                let classification = if dependency == first {
                    DependencyClassification::Rejected
                } else {
                    DependencyClassification::Pending
                };
                let (next, advanced, _) = advance_registration(
                    &store,
                    &roots,
                    child,
                    fingerprint,
                    ordinal,
                    dependency,
                    |_| Ok(classification),
                )
                .unwrap();
                roots = next;
                record = advanced;
            }
            assert_eq!(record.status(), CompactBatchStatus::Ready, "{case:?}");
            assert!(record.dependency_rejected(), "{case:?}");
            assert_eq!(record.dependency_set_commitment(), commitment, "{case:?}");

            let (next, ready) = pop_ready(&store, &roots).unwrap();
            assert_eq!(ready, Some(child), "{case:?}");
            roots = begin_finish(
                &store,
                &next,
                child,
                vec![9],
                FinalDependencyStatus::Rejected,
            )
            .unwrap();
            let reconstructed = roots.clone();
            roots = reconstructed;
            drain_fanout(&store, &mut roots);

            let second = parents[case.rejection_order[1]];
            finish_parent(&store, &mut roots, second, FinalDependencyStatus::Rejected);
            drain_fanout(&store, &mut roots);
            let third = parents[2];
            finish_parent(&store, &mut roots, third, FinalDependencyStatus::Satisfied);
            drain_fanout(&store, &mut roots);

            let final_record = lookup(&store, &roots, child).unwrap().unwrap();
            assert_eq!(final_record.status(), CompactBatchStatus::Final, "{case:?}");
            assert_eq!(final_record.unresolved_count(), 0, "{case:?}");
            assert_eq!(final_record.dependency_set_commitment(), commitment);
            assert_eq!(pending_registration(&roots), 0, "{case:?}");
            assert_eq!(pending_fanout(&roots), 0, "{case:?}");
            assert_eq!(roots.ready_queue_len, 0, "{case:?}");
            assert_eq!(
                observed_commitment.get_or_insert(commitment),
                &commitment,
                "{case:?}"
            );
            drop(store);
            std::fs::remove_dir_all(path).unwrap();
        }
    }

    #[test]
    fn disjoint_sparse_parent_clocks_use_fixed_root_and_weighted_small_slices() {
        let path = std::env::temp_dir().join(format!("tine-causal-accumulator-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        let archive = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
        let store =
            ScratchStore::open(&archive, WorkspaceId::from_uuid(Uuid::from_u128(401))).unwrap();
        let child = BatchId::from_uuid(Uuid::from_u128(402));
        let parents = [
            BatchId::from_uuid(Uuid::from_u128(410)),
            BatchId::from_uuid(Uuid::from_u128(411)),
            BatchId::from_uuid(Uuid::from_u128(412)),
        ];
        let fingerprint = ContentDigest::of(b"disjoint-clock-child");
        let mut roots = begin_stage(
            &store,
            &ScratchRoots::default(),
            child,
            fingerprint,
            fingerprint,
            ContentDigest::of(b"disjoint-clock-dependency-set"),
            parents.len(),
            0,
        )
        .unwrap()
        .0;
        let metric_before = store.stats();
        let mut accumulated = 0;
        let mut record_lengths = Vec::new();
        for (ordinal, cardinality) in [8_usize, 16, 32].into_iter().enumerate() {
            let clock = (0..cardinality)
                .map(|offset| {
                    (
                        CausalPeerId::from_device_id(DeviceId::from_uuid(Uuid::from_u128(
                            1_000 + accumulated as u128 + offset as u128,
                        ))),
                        1,
                    )
                })
                .collect::<Vec<_>>();
            let required = cardinality as u64 + 1;
            let mut prepaid = 0;
            loop {
                let (next, record, step) = prepay_registration(
                    &store,
                    &roots,
                    child,
                    fingerprint,
                    ordinal as u32,
                    required,
                    1,
                )
                .unwrap();
                roots = next;
                prepaid += step.consumed;
                assert!(record.registration_work_remaining().unwrap() <= required);
                let reconstructed = roots.clone();
                let remaining_before = record.registration_work_remaining();
                roots = reconstructed;
                assert_eq!(
                    lookup(&store, &roots, child)
                        .unwrap()
                        .unwrap()
                        .registration_work_remaining(),
                    remaining_before,
                    "slice-one reconstruction preserves exact remaining credit"
                );
                if step.complete {
                    break;
                }
            }
            assert_eq!(prepaid as u64, required);
            let before = store.stats();
            let (next, record, _) = advance_registration(
                &store,
                &roots,
                child,
                fingerprint,
                ordinal as u32,
                parents[ordinal],
                |_| {
                    Ok(DependencyClassification::Satisfied {
                        causal_clock: clock.clone(),
                    })
                },
            )
            .unwrap();
            roots = next;
            accumulated += cardinality;
            let materialized = materialize_causal_accumulator(&store, &record).unwrap();
            assert_eq!(materialized.len(), accumulated);
            assert_eq!(
                materialized
                    .iter()
                    .map(|(_, counter)| *counter)
                    .sum::<u64>(),
                accumulated as u64
            );
            let after = store.stats();
            assert!(
                after.page_writes.saturating_sub(before.page_writes) >= cardinality,
                "each selected parent point caused authenticated point work"
            );
            assert!(
                after.page_bytes_written > before.page_bytes_written,
                "scratch byte work is instrumented"
            );
            record_lengths.push(encoded_record_len(&record));
        }
        let min = *record_lengths.iter().min().unwrap();
        let max = *record_lengths.iter().max().unwrap();
        assert!(
            max <= min + 16 && max < 1024,
            "the staged record retains only a fixed-size accumulator root: {record_lengths:?}"
        );
        let final_record = lookup(&store, &roots, child).unwrap().unwrap();
        assert_eq!(final_record.status(), CompactBatchStatus::Ready);
        assert_eq!(
            final_record.finalization_work_remaining(),
            accumulated as u64
        );
        let mut finalization_prepaid = 0;
        loop {
            let (next, step) = prepay_finalization(&store, &roots, child, 2).unwrap();
            roots = next;
            finalization_prepaid += step.consumed;
            if step.complete {
                break;
            }
        }
        assert_eq!(finalization_prepaid, accumulated);
        assert_eq!(
            lookup(&store, &roots, child)
                .unwrap()
                .unwrap()
                .finalization_work_remaining(),
            0
        );
        let metric_after = store.stats();
        let page_io = metric_after
            .page_reads
            .saturating_sub(metric_before.page_reads)
            .saturating_add(
                metric_after
                    .page_writes
                    .saturating_sub(metric_before.page_writes),
            );
        let linear_io_bound = accumulated
            .saturating_mul(32)
            .saturating_mul(super::super::scratch_store::AUTHENTICATED_POINT_MAX_IO_PER_MUTATION);
        assert!(
            page_io <= linear_io_bound,
            "disjoint sparse clocks used {page_io} page operations, linear bound {linear_io_bound}"
        );
        let byte_io = metric_after
            .page_bytes_read
            .saturating_sub(metric_before.page_bytes_read)
            .saturating_add(
                metric_after
                    .page_bytes_written
                    .saturating_sub(metric_before.page_bytes_written),
            );
        assert!(
            byte_io
                <= linear_io_bound.saturating_mul(
                    super::super::scratch_store::AUTHENTICATED_POINT_MAX_PAGE_BYTES,
                ),
            "measured scratch bytes remain within the same linear physical metric"
        );
        drop(store);
        std::fs::remove_dir_all(path).unwrap();
    }
}
