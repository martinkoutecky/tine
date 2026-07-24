use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::scratch_store::{ScratchLsmRoot, ScratchPageKind, ScratchRoots, ScratchStore};
use super::{BatchCausalDot, BatchId, CausalPeerId, OperationBatch};

const CAUSAL_INDEX_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CausalBatchRecord {
    schema_version: u32,
    batch_id: BatchId,
    dot: BatchCausalDot,
    clock: Vec<(CausalPeerId, u64)>,
}

impl CausalBatchRecord {
    pub(crate) fn contains(&self, dot: BatchCausalDot) -> bool {
        self.clock
            .binary_search_by_key(&dot.peer_id(), |(peer, _)| *peer)
            .ok()
            .is_some_and(|index| self.clock[index].1 >= dot.counter())
    }

    pub(crate) fn counter(&self, peer: CausalPeerId) -> u64 {
        self.clock
            .binary_search_by_key(&peer, |(candidate, _)| *candidate)
            .ok()
            .map(|index| self.clock[index].1)
            .unwrap_or(0)
    }

    pub(crate) fn clock(&self) -> &[(CausalPeerId, u64)] {
        &self.clock
    }
}

pub(crate) fn next_dot(
    store: &ScratchStore,
    roots: &ScratchRoots,
    peer: CausalPeerId,
) -> Result<(BatchCausalDot, Option<BatchId>), CausalIndexError> {
    let (counter, prior_batch) = match store.lookup(
        &roots.causal_peer_root,
        ScratchPageKind::CausalPeer,
        peer_key(peer).as_slice(),
    )? {
        Some(bytes) => {
            let (counter, batch_id) = decode_peer_record(peer, &bytes)?;
            (counter, Some(batch_id))
        }
        None => (0, None),
    };
    Ok((
        BatchCausalDot::new(
            peer,
            counter
                .checked_add(1)
                .ok_or(CausalIndexError::CounterOverflow)?,
        )
        .map_err(|_| CausalIndexError::CounterOverflow)?,
        prior_batch,
    ))
}

pub(crate) fn insert_batch(
    store: &ScratchStore,
    roots: &ScratchRoots,
    manifest: &OperationBatch,
) -> Result<ScratchRoots, CausalIndexError> {
    let batch_id = manifest.batch_id();
    let dot = manifest.causal_dot();
    if lookup_batch(store, &roots.causal_root, batch_id)?.is_some() {
        return Err(CausalIndexError::BatchReuse(batch_id));
    }
    let dot_key = dot_key(dot);
    if let Some(bytes) =
        store.lookup(&roots.causal_dot_root, ScratchPageKind::CausalDot, &dot_key)?
    {
        let existing = decode_batch_id(&bytes)?;
        return Err(CausalIndexError::DotReuse {
            dot,
            existing,
            offered: batch_id,
        });
    }

    let mut clock = BTreeMap::<CausalPeerId, u64>::new();
    for parent in manifest.causal_dependency_heads() {
        let record = lookup_batch(store, &roots.causal_root, *parent)?
            .ok_or(CausalIndexError::MissingParent(*parent))?;
        for (peer, counter) in record.clock {
            clock
                .entry(peer)
                .and_modify(|current| *current = (*current).max(counter))
                .or_insert(counter);
        }
    }
    let expected = clock
        .get(&dot.peer_id())
        .copied()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(CausalIndexError::CounterOverflow)?;
    if dot.counter() != expected {
        return Err(CausalIndexError::DotGap {
            dot,
            expected_counter: expected,
        });
    }
    clock.insert(dot.peer_id(), dot.counter());
    let record = CausalBatchRecord {
        schema_version: CAUSAL_INDEX_SCHEMA_VERSION,
        batch_id,
        dot,
        clock: clock.into_iter().collect(),
    };
    validate_record(&record)?;

    let mut next = roots.clone();
    next.causal_root = store.insert_many(
        &roots.causal_root,
        ScratchPageKind::CausalBatch,
        &BTreeMap::from([(batch_key(batch_id), Some(encode_canonical(&record)?))]),
    )?;
    next.causal_dot_root = store.insert_many(
        &roots.causal_dot_root,
        ScratchPageKind::CausalDot,
        &BTreeMap::from([(dot_key, Some(encode_canonical(&batch_id)?))]),
    )?;
    next.causal_peer_root = store.insert_many(
        &roots.causal_peer_root,
        ScratchPageKind::CausalPeer,
        &BTreeMap::from([(
            peer_key(dot.peer_id()),
            Some(encode_canonical(&(dot.peer_id(), dot.counter(), batch_id))?),
        )]),
    )?;
    Ok(next)
}

pub(crate) fn batch_record(
    store: &ScratchStore,
    roots: &ScratchRoots,
    batch_id: BatchId,
) -> Result<Option<CausalBatchRecord>, CausalIndexError> {
    lookup_batch(store, &roots.causal_root, batch_id)
}

fn lookup_batch(
    store: &ScratchStore,
    root: &ScratchLsmRoot,
    batch_id: BatchId,
) -> Result<Option<CausalBatchRecord>, CausalIndexError> {
    store
        .lookup(
            root,
            ScratchPageKind::CausalBatch,
            batch_key(batch_id).as_slice(),
        )?
        .map(|bytes| {
            let record: CausalBatchRecord = decode_canonical(&bytes)?;
            validate_record(&record)?;
            if record.batch_id != batch_id {
                return Err(CausalIndexError::MisboundRecord);
            }
            Ok(record)
        })
        .transpose()
}

fn validate_record(record: &CausalBatchRecord) -> Result<(), CausalIndexError> {
    if record.schema_version != CAUSAL_INDEX_SCHEMA_VERSION
        || record.dot.counter() == 0
        || record.clock.is_empty()
        || record.clock.windows(2).any(|pair| pair[0].0 >= pair[1].0)
        || record.counter(record.dot.peer_id()) != record.dot.counter()
    {
        return Err(CausalIndexError::MalformedRecord);
    }
    Ok(())
}

fn batch_key(batch_id: BatchId) -> Vec<u8> {
    batch_id.as_uuid().as_bytes().to_vec()
}

fn peer_key(peer: CausalPeerId) -> Vec<u8> {
    peer.as_device_id().as_uuid().as_bytes().to_vec()
}

fn dot_key(dot: BatchCausalDot) -> Vec<u8> {
    let mut key = peer_key(dot.peer_id());
    key.extend_from_slice(&dot.counter().to_be_bytes());
    key
}

fn decode_peer_record(
    peer: CausalPeerId,
    bytes: &[u8],
) -> Result<(u64, BatchId), CausalIndexError> {
    let (found_peer, counter, batch_id): (CausalPeerId, u64, BatchId) = decode_canonical(bytes)?;
    if found_peer != peer || counter == 0 {
        return Err(CausalIndexError::MisboundRecord);
    }
    Ok((counter, batch_id))
}

fn decode_batch_id(bytes: &[u8]) -> Result<BatchId, CausalIndexError> {
    decode_canonical(bytes)
}

fn encode_canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, CausalIndexError> {
    postcard::to_allocvec(value).map_err(|_| CausalIndexError::MalformedRecord)
}

fn decode_canonical<T: for<'de> Deserialize<'de> + Serialize>(
    bytes: &[u8],
) -> Result<T, CausalIndexError> {
    let value: T = postcard::from_bytes(bytes).map_err(|_| CausalIndexError::MalformedRecord)?;
    if encode_canonical(&value)? != bytes {
        return Err(CausalIndexError::MalformedRecord);
    }
    Ok(value)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CausalIndexError {
    Scratch(String),
    MissingParent(BatchId),
    BatchReuse(BatchId),
    DotReuse {
        dot: BatchCausalDot,
        existing: BatchId,
        offered: BatchId,
    },
    DotGap {
        dot: BatchCausalDot,
        expected_counter: u64,
    },
    CounterOverflow,
    MisboundRecord,
    MalformedRecord,
}

impl fmt::Display for CausalIndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scratch(error) => write!(f, "causal scratch index failed: {error}"),
            Self::MissingParent(batch) => write!(f, "missing causal parent {batch}"),
            Self::BatchReuse(batch) => write!(f, "causal record already exists for {batch}"),
            Self::DotReuse {
                dot,
                existing,
                offered,
            } => write!(
                f,
                "causal dot {:?}:{} belongs to {existing}, not {offered}",
                dot.peer_id(),
                dot.counter()
            ),
            Self::DotGap {
                dot,
                expected_counter,
            } => write!(
                f,
                "causal dot {:?}:{} is not gap-free; expected {expected_counter}",
                dot.peer_id(),
                dot.counter()
            ),
            Self::CounterOverflow => f.write_str("causal counter overflow"),
            Self::MisboundRecord => f.write_str("misbound causal scratch record"),
            Self::MalformedRecord => f.write_str("malformed causal scratch record"),
        }
    }
}

impl std::error::Error for CausalIndexError {}

impl From<super::scratch_store::ScratchError> for CausalIndexError {
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

    use crate::oplog::{
        DeviceId, DocumentId, FrontierV2, LineageDigest, ObjectKind, OperationObject,
        SemanticEffectDigest, SessionId, WorkspaceId,
    };

    fn manifest(
        workspace: WorkspaceId,
        batch: u128,
        device: u128,
        dot: BatchCausalDot,
        dependencies: Vec<BatchId>,
    ) -> OperationBatch {
        let semantic = OperationObject::new(
            workspace,
            DocumentId::from_uuid(Uuid::from_u128(10)),
            ObjectKind::SemanticEffect,
            vec![1],
        )
        .unwrap();
        OperationBatch::new_with_causality(
            workspace,
            LineageDigest::of(b"causal-index-test"),
            BatchId::from_uuid(Uuid::from_u128(batch)),
            DeviceId::from_uuid(Uuid::from_u128(device)),
            SessionId::from_uuid(Uuid::from_u128(batch + 1_000)),
            crate::oplog::BatchOrigin::BootstrapImport,
            dot,
            dependencies,
            FrontierV2::new(Vec::new()).unwrap(),
            SemanticEffectDigest::of(&[1]),
            vec![semantic.descriptor().unwrap()],
        )
        .unwrap()
    }

    #[test]
    fn correction11_exact_clocks_reject_dot_reuse_and_gaps() {
        let path = std::env::temp_dir().join(format!("tine-causal-index-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        let archive = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(1));
        let store = ScratchStore::open(&archive, workspace).unwrap();
        let peer_a = CausalPeerId::from_device_id(DeviceId::from_uuid(Uuid::from_u128(20)));
        let peer_b = CausalPeerId::from_device_id(DeviceId::from_uuid(Uuid::from_u128(21)));
        let first = manifest(
            workspace,
            100,
            20,
            BatchCausalDot::new(peer_a, 1).unwrap(),
            vec![],
        );
        let mut roots = insert_batch(&store, &ScratchRoots::default(), &first).unwrap();

        let reused = manifest(
            workspace,
            101,
            20,
            BatchCausalDot::new(peer_a, 1).unwrap(),
            vec![],
        );
        assert!(matches!(
            insert_batch(&store, &roots, &reused),
            Err(CausalIndexError::DotReuse { .. })
        ));
        let gap = manifest(
            workspace,
            102,
            20,
            BatchCausalDot::new(peer_a, 3).unwrap(),
            vec![first.batch_id()],
        );
        assert!(matches!(
            insert_batch(&store, &roots, &gap),
            Err(CausalIndexError::DotGap {
                expected_counter: 2,
                ..
            })
        ));

        let second = manifest(
            workspace,
            103,
            20,
            BatchCausalDot::new(peer_a, 2).unwrap(),
            vec![first.batch_id()],
        );
        roots = insert_batch(&store, &roots, &second).unwrap();
        let joined = manifest(
            workspace,
            104,
            21,
            BatchCausalDot::new(peer_b, 1).unwrap(),
            vec![second.batch_id()],
        );
        roots = insert_batch(&store, &roots, &joined).unwrap();
        let record = batch_record(&store, &roots, joined.batch_id())
            .unwrap()
            .unwrap();
        assert!(record.contains(BatchCausalDot::new(peer_a, 1).unwrap()));
        assert!(record.contains(BatchCausalDot::new(peer_a, 2).unwrap()));
        assert!(record.contains(BatchCausalDot::new(peer_b, 1).unwrap()));
        assert_eq!(store.stats().scratch_syncs, 0);
        drop(store);
        std::fs::remove_dir_all(path).unwrap();
    }
}
