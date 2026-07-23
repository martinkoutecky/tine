use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use super::scratch_store::{ScratchPageKind, ScratchRoots, ScratchStore};
use super::{BatchId, ContentDigest};

const DEPENDENCY_QUEUE_SCHEMA_VERSION: u32 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CompactBatchStatus {
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
    direct_dependencies: Vec<BatchId>,
    unresolved_count: u32,
    status: CompactBatchStatus,
    final_status: Option<Vec<u8>>,
}

impl StagedBatchRecord {
    pub(crate) const fn batch_id(&self) -> BatchId {
        self.batch_id
    }

    pub(crate) const fn manifest_fingerprint(&self) -> ContentDigest {
        self.manifest_fingerprint
    }

    pub(crate) fn direct_dependencies(&self) -> &[BatchId] {
        &self.direct_dependencies
    }

    pub(crate) const fn status(&self) -> CompactBatchStatus {
        self.status
    }

    pub(crate) fn final_status(&self) -> Option<&[u8]> {
        self.final_status.as_deref()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct QueueWork {
    pub wait_edge_visits: usize,
    pub ready_queue_residency: usize,
}

pub(crate) fn stage(
    store: &ScratchStore,
    roots: &ScratchRoots,
    batch_id: BatchId,
    manifest_fingerprint: ContentDigest,
    direct_dependencies: impl IntoIterator<Item = BatchId>,
    dependency_is_final: impl Fn(BatchId) -> Result<bool, DependencyQueueError>,
) -> Result<(ScratchRoots, StagedBatchRecord, QueueWork), DependencyQueueError> {
    if let Some(existing) = lookup(store, roots, batch_id)? {
        if existing.manifest_fingerprint != manifest_fingerprint {
            return Err(DependencyQueueError::BatchCollision(batch_id));
        }
        return Ok((roots.clone(), existing, QueueWork::default()));
    }
    let direct_dependencies = direct_dependencies.into_iter().collect::<BTreeSet<_>>();
    if direct_dependencies.contains(&batch_id) {
        return Err(DependencyQueueError::SelfDependency(batch_id));
    }
    let mut unresolved = Vec::new();
    for dependency in &direct_dependencies {
        if !dependency_is_final(*dependency)? {
            unresolved.push(*dependency);
        }
    }
    let unresolved_count =
        u32::try_from(unresolved.len()).map_err(|_| DependencyQueueError::TooManyDependencies)?;
    let record = StagedBatchRecord {
        schema_version: DEPENDENCY_QUEUE_SCHEMA_VERSION,
        batch_id,
        manifest_fingerprint,
        direct_dependencies: direct_dependencies.into_iter().collect(),
        unresolved_count,
        status: if unresolved_count == 0 {
            CompactBatchStatus::Ready
        } else {
            CompactBatchStatus::Waiting
        },
        final_status: None,
    };
    validate_record(&record)?;
    let mut next = roots.clone();
    next.batch_status_root = put_record(store, &next, &record)?;
    if !unresolved.is_empty() {
        let waits = unresolved
            .into_iter()
            .map(|parent| (wait_key(parent, batch_id), Some(Vec::new())))
            .collect();
        next.wait_root =
            store.insert_many(&next.wait_root, ScratchPageKind::DependencyWait, &waits)?;
    } else {
        next = enqueue(store, &next, batch_id)?;
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
    if record.status != CompactBatchStatus::Ready || record.unresolved_count != 0 {
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
    next.ready_queue_root = store.insert_many(
        &next.ready_queue_root,
        ScratchPageKind::ReadyQueue,
        &changes,
    )?;
    next.ready_queue_len -= 1;
    next.batch_status_root = put_record(store, &next, &record)?;
    Ok((next, Some(batch_id)))
}

pub(crate) fn finish(
    store: &ScratchStore,
    roots: &ScratchRoots,
    batch_id: BatchId,
    final_status: Vec<u8>,
) -> Result<(ScratchRoots, Vec<BatchId>, QueueWork), DependencyQueueError> {
    if final_status.is_empty() {
        return Err(DependencyQueueError::MalformedRecord);
    }
    let mut record =
        lookup(store, roots, batch_id)?.ok_or(DependencyQueueError::MissingRecord(batch_id))?;
    if record.status == CompactBatchStatus::Final {
        if record.final_status.as_deref() == Some(final_status.as_slice()) {
            return Ok((roots.clone(), Vec::new(), QueueWork::default()));
        }
        return Err(DependencyQueueError::MalformedRecord);
    }
    if record.status != CompactBatchStatus::Processing {
        return Err(DependencyQueueError::MalformedRecord);
    }
    record.status = CompactBatchStatus::Final;
    record.final_status = Some(final_status);
    let mut next = roots.clone();
    next.batch_status_root = put_record(store, &next, &record)?;

    let prefix = batch_key(batch_id);
    let waits = store.scan_prefix(&next.wait_root, ScratchPageKind::DependencyWait, &prefix)?;
    let mut awakened = Vec::new();
    let mut wait_deletes = BTreeMap::new();
    for (key, _) in &waits {
        let (parent, child) = decode_wait_key(key)?;
        if parent != batch_id {
            return Err(DependencyQueueError::MisboundRecord);
        }
        let mut child_record =
            lookup(store, &next, child)?.ok_or(DependencyQueueError::MissingRecord(child))?;
        if child_record.status != CompactBatchStatus::Waiting || child_record.unresolved_count == 0
        {
            return Err(DependencyQueueError::MalformedRecord);
        }
        child_record.unresolved_count -= 1;
        if child_record.unresolved_count == 0 {
            child_record.status = CompactBatchStatus::Ready;
            next = enqueue(store, &next, child)?;
            awakened.push(child);
        }
        next.batch_status_root = put_record(store, &next, &child_record)?;
        wait_deletes.insert(key.clone(), None);
    }
    if !wait_deletes.is_empty() {
        next.wait_root = store.insert_many(
            &next.wait_root,
            ScratchPageKind::DependencyWait,
            &wait_deletes,
        )?;
    }
    let ready_queue_residency = usize::try_from(next.ready_queue_len).unwrap_or(usize::MAX);
    Ok((
        next,
        awakened,
        QueueWork {
            wait_edge_visits: waits.len(),
            ready_queue_residency,
        },
    ))
}

pub(crate) fn lookup(
    store: &ScratchStore,
    roots: &ScratchRoots,
    batch_id: BatchId,
) -> Result<Option<StagedBatchRecord>, DependencyQueueError> {
    store
        .lookup(
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

pub(crate) fn all_records(
    store: &ScratchStore,
    roots: &ScratchRoots,
) -> Result<Vec<StagedBatchRecord>, DependencyQueueError> {
    store
        .materialize(&roots.batch_status_root, ScratchPageKind::BatchStatus)?
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

fn put_record(
    store: &ScratchStore,
    roots: &ScratchRoots,
    record: &StagedBatchRecord,
) -> Result<super::scratch_store::ScratchLsmRoot, DependencyQueueError> {
    validate_record(record)?;
    store
        .insert_many(
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
    next.ready_queue_root = store.insert_many(
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
        .lookup(
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
        || record
            .direct_dependencies
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        || record
            .direct_dependencies
            .binary_search(&record.batch_id)
            .is_ok()
        || record.unresolved_count as usize > record.direct_dependencies.len()
        || (record.status == CompactBatchStatus::Ready && record.unresolved_count != 0)
        || (record.status == CompactBatchStatus::Processing && record.unresolved_count != 0)
        || (record.status == CompactBatchStatus::Final) != record.final_status.is_some()
    {
        return Err(DependencyQueueError::MalformedRecord);
    }
    Ok(())
}

fn batch_key(batch_id: BatchId) -> Vec<u8> {
    batch_id.as_uuid().as_bytes().to_vec()
}

fn wait_key(parent: BatchId, child: BatchId) -> Vec<u8> {
    let mut key = batch_key(parent);
    key.extend_from_slice(child.as_uuid().as_bytes());
    key
}

fn decode_wait_key(key: &[u8]) -> Result<(BatchId, BatchId), DependencyQueueError> {
    if key.len() != 32 {
        return Err(DependencyQueueError::MisboundRecord);
    }
    let parent = uuid::Uuid::from_slice(&key[..16])
        .map(BatchId::from_uuid)
        .map_err(|_| DependencyQueueError::MisboundRecord)?;
    let child = uuid::Uuid::from_slice(&key[16..])
        .map(BatchId::from_uuid)
        .map_err(|_| DependencyQueueError::MisboundRecord)?;
    Ok((parent, child))
}

fn decode_batch_id(bytes: &[u8]) -> Result<BatchId, DependencyQueueError> {
    decode_canonical(bytes)
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
    SelfDependency(BatchId),
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
            Self::SelfDependency(batch) => write!(f, "batch {batch} depends on itself"),
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
    use uuid::Uuid;

    use crate::oplog::WorkspaceId;

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
                [parent],
                |_| Ok(false),
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
            [],
            |_| Ok(false),
        )
        .unwrap();
        roots = next;
        assert_eq!(parent_record.status(), CompactBatchStatus::Ready);
        assert_eq!(parent_work.ready_queue_residency, 1);

        let (next, ready) = pop_ready(&store, &roots).unwrap();
        roots = next;
        assert_eq!(ready, Some(parent));
        let (next, awakened, work) = finish(&store, &roots, parent, vec![1]).unwrap();
        roots = next;
        assert_eq!(awakened.len(), CHILDREN);
        assert_eq!(work.wait_edge_visits, CHILDREN);
        assert_eq!(work.ready_queue_residency, CHILDREN);

        let mut observed = Vec::new();
        while let (next, Some(batch_id)) = pop_ready(&store, &roots).unwrap() {
            roots = next;
            observed.push(batch_id);
            roots = finish(&store, &roots, batch_id, vec![1]).unwrap().0;
        }
        assert_eq!(observed.len(), CHILDREN);
        assert!(observed.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(roots.ready_queue_len, 0);
        assert_eq!(store.stats().scratch_syncs, 0);
        drop(store);
        std::fs::remove_dir_all(path).unwrap();
    }
}
