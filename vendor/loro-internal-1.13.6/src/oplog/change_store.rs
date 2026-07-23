use self::block_encode::{decode_block, decode_header, encode_block, ChangesBlockHeader};
use super::{loro_dag::AppDagNodeInner, AppDagNode};
use crate::sync::Mutex;
use crate::{
    arena::SharedArena,
    change::Change,
    estimated_size::EstimatedSize,
    external_store::{
        causal_node_digest, causal_source_digest, decode_causal_snapshot, decode_import_baseline,
        decode_store_metadata, encode_store_metadata, metadata_digest, CausalBoundaryPart,
        CausalBoundaryProof, CausalDagSnapshot, ExternalStoreMetadata, ImportBaselineSnapshot,
    },
    kv_store::{KvStore, KvStoreHandle},
    op::Op,
    parent::register_container_and_parent_link,
    version::{Frontiers, ImVersionVector},
    VersionVector,
};
use block_encode::decode_block_range;
use bytes::Bytes;
use itertools::Itertools;
use loro_common::{
    Counter, HasCounterSpan, HasId, HasIdSpan, HasLamportSpan, IdLp, IdSpan, Lamport, LoroError,
    LoroResult, PeerID, ID,
};
use loro_kv_store::{mem_store::MemKvConfig, MemKvStore};
use once_cell::sync::OnceCell;
use rle::{HasLength, Mergable, RlePush, RleVec, Sliceable};
use sha2::{Digest, Sha256};
use std::sync::atomic::AtomicI64;
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, VecDeque},
    ops::{Bound, Deref},
    sync::Arc,
};
use tracing::{info_span, warn};
mod block_encode;
mod block_meta_encode;
pub(super) mod iter;

#[cfg(not(test))]
const MAX_BLOCK_SIZE: usize = 1024 * 4;
#[cfg(test)]
const MAX_BLOCK_SIZE: usize = 128;

/// # Invariance
///
/// - We don't allow holes in a block or between two blocks with the same peer id.
///   The [Change] should be continuous for each peer.
/// - However, the first block of a peer can have counter > 0 so that we can trim the history.
///
/// # Encoding Schema
///
/// It's based on the underlying KV store.
///
/// The entries of the KV store is made up of the following fields
///
/// |Key                          |Value             |
/// |:--                          |:----             |
/// |b"vv"                        |VersionVector     |
/// |b"fr"                        |Frontiers         |
/// |b"sv"                        |Shallow VV        |
/// |b"sf"                        |Shallow Frontiers |
/// |12 bytes PeerID + Counter    |Encoded Block     |
#[derive(Debug, Clone)]
pub struct ChangeStore {
    inner: Arc<Mutex<ChangeStoreInner>>,
    arena: SharedArena,
    /// A change may be in external_kv or in the mem_parsed_kv.
    /// mem_parsed_kv is more up-to-date.
    ///
    /// We cannot directly write into the external_kv except from the initial load
    external_kv: Arc<Mutex<dyn KvStore>>,
    /// The version vector of the external kv store.
    external_vv: Arc<Mutex<VersionVector>>,
    merge_interval: Arc<AtomicI64>,
    is_external: bool,
}

#[derive(Debug, Clone)]
struct ChangeStoreInner {
    /// The start version vector of the first block for each peer.
    /// It allows us to trim the history
    start_vv: ImVersionVector,
    /// The last version of the shallow history.
    start_frontiers: Frontiers,
    /// It's more like a parsed cache for binary_kv.
    mem_parsed_kv: BTreeMap<ID, Arc<ChangesBlock>>,
}

#[derive(Debug)]
pub(crate) struct ChangeStoreRollback {
    old_vv: VersionVector,
    blocks_before_mutation: BTreeMap<ID, Arc<ChangesBlock>>,
}

#[derive(Clone)]
struct ValidationBlock {
    block: Arc<ChangesBlock>,
    arena: SharedArena,
}

struct ExternalValidationCache<'a> {
    store: &'a ChangeStore,
    blocks: BTreeMap<ID, ValidationBlock>,
    loaded_memory_peers: BTreeSet<PeerID>,
}

impl<'a> ExternalValidationCache<'a> {
    fn new(store: &'a ChangeStore) -> Self {
        Self {
            store,
            blocks: BTreeMap::new(),
            loaded_memory_peers: BTreeSet::new(),
        }
    }

    fn load_memory_peer(&mut self, peer: PeerID, end: Counter) -> LoroResult<()> {
        if !self.loaded_memory_peers.insert(peer) || end <= 0 {
            return Ok(());
        }

        let memory_blocks = self
            .store
            .inner
            .lock()
            .mem_parsed_kv
            .range(ID::new(peer, 0)..=ID::new(peer, end - 1))
            .map(|(&id, block)| (id, block.clone()))
            .collect::<Vec<_>>();
        for (block_id, mut block) in memory_blocks {
            block.ensure_changes(&self.store.arena).map_err(|_| {
                LoroError::DecodeError(
                    format!("cached external change block starting at {block_id} is corrupt")
                        .into_boxed_str(),
                )
            })?;
            self.blocks.insert(
                block_id,
                ValidationBlock {
                    block,
                    arena: self.store.arena.clone(),
                },
            );
        }
        Ok(())
    }

    fn memory_changes_for_peer(
        &mut self,
        peer: PeerID,
        end: Counter,
    ) -> LoroResult<Vec<(BlockChangeRef, SharedArena)>> {
        self.load_memory_peer(peer, end)?;
        let mut changes = BTreeMap::new();
        for validation_block in self.blocks.values() {
            if validation_block.block.peer != peer {
                continue;
            }
            let block_changes = validation_block
                .block
                .content
                .try_changes()
                .ok_or_else(|| {
                    LoroError::DecodeError("external validation block is not parsed".into())
                })?;
            for change_index in 0..block_changes.len() {
                let change = BlockChangeRef {
                    block: validation_block.block.clone(),
                    change_index,
                };
                changes.insert(change.id, (change, validation_block.arena.clone()));
            }
        }
        Ok(changes.into_values().collect())
    }
}

impl ChangeStoreRollback {
    pub(crate) fn new(old_vv: VersionVector) -> Self {
        Self {
            old_vv,
            blocks_before_mutation: BTreeMap::new(),
        }
    }

    fn record_block_before_mutation(&mut self, id: ID, block: Arc<ChangesBlock>) {
        let old_end = self.old_vv.get(&id.peer).copied().unwrap_or(0);
        if id.counter >= old_end {
            return;
        }

        self.blocks_before_mutation.entry(id).or_insert(block);
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ChangesBlock {
    peer: PeerID,
    counter_range: (Counter, Counter),
    lamport_range: (Lamport, Lamport),
    /// Estimated size of the block in bytes
    estimated_size: usize,
    flushed: bool,
    content: ChangesBlockContent,
}

#[derive(Clone)]
pub(crate) enum ChangesBlockContent {
    Changes(Arc<Vec<Change>>),
    Bytes(ChangesBlockBytes),
    Both(Arc<Vec<Change>>, ChangesBlockBytes),
}

/// It's cheap to clone this struct because it's cheap to clone the bytes
#[derive(Clone)]
pub(crate) struct ChangesBlockBytes {
    bytes: Bytes,
    header: OnceCell<Arc<ChangesBlockHeader>>,
}

pub const START_VV_KEY: &[u8] = b"sv";
pub const START_FRONTIERS_KEY: &[u8] = b"sf";
pub const VV_KEY: &[u8] = b"vv";
pub const FRONTIERS_KEY: &[u8] = b"fr";
pub const EXTERNAL_METADATA_KEY: &[u8] = b"xm";

impl ChangeStore {
    pub fn new_mem(a: &SharedArena, merge_interval: Arc<AtomicI64>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ChangeStoreInner {
                start_vv: ImVersionVector::new(),
                start_frontiers: Frontiers::default(),
                mem_parsed_kv: BTreeMap::new(),
            })),
            arena: a.clone(),
            external_vv: Arc::new(Mutex::new(VersionVector::new())),
            external_kv: Arc::new(Mutex::new(MemKvStore::new(MemKvConfig::default()))),
            // external_kv: Arc::new(Mutex::new(BTreeMap::default())),
            merge_interval,
            is_external: false,
        }
    }

    pub(crate) fn new_external(
        arena: &SharedArena,
        merge_interval: Arc<AtomicI64>,
        external_kv: KvStoreHandle,
    ) -> LoroResult<(Self, BatchDecodeInfo)> {
        let store = Self {
            inner: Arc::new(Mutex::new(ChangeStoreInner {
                start_vv: ImVersionVector::new(),
                start_frontiers: Frontiers::default(),
                mem_parsed_kv: BTreeMap::new(),
            })),
            arena: arena.clone(),
            external_vv: Arc::new(Mutex::new(VersionVector::new())),
            external_kv,
            merge_interval,
            is_external: true,
        };
        let info = store.load_external_metadata()?;
        Ok((store, info))
    }

    pub(crate) fn is_external(&self) -> bool {
        self.is_external
    }

    pub(crate) fn probe_external_store(&self) -> LoroResult<()> {
        if !self.is_external {
            return Ok(());
        }

        let mut store = self.external_kv.lock();
        check_store_error(&mut *store)?;
        let _ = store.len();
        check_store_error(&mut *store)
    }

    pub(crate) fn verify_external_predecessor(
        &self,
        expected_metadata_digest: Option<[u8; 32]>,
    ) -> LoroResult<()> {
        if !self.is_external {
            return Ok(());
        }
        let mut store = self.external_kv.lock();
        check_store_error(&mut *store)?;
        let metadata = store.get(EXTERNAL_METADATA_KEY);
        check_store_error(&mut *store)?;
        match (expected_metadata_digest, metadata) {
            (None, None) if store.is_empty() => {
                check_store_error(&mut *store)?;
                Ok(())
            }
            (Some(expected), Some(metadata)) if metadata_digest(&metadata) == expected => Ok(()),
            _ => Err(LoroError::DecodeError(
                "external authenticated predecessor metadata changed before publication".into(),
            )),
        }
    }

    pub(crate) fn take_external_error(&self) -> Option<LoroError> {
        if !self.is_external {
            return None;
        }

        self.external_kv
            .lock()
            .take_error()
            .map(external_store_error)
    }

    fn load_external_metadata(&self) -> LoroResult<BatchDecodeInfo> {
        let metadata = {
            let mut store = self.external_kv.lock();
            check_store_error(&mut *store)?;
            let metadata = store.get(EXTERNAL_METADATA_KEY);
            check_store_error(&mut *store)?;

            if metadata.is_none() {
                let is_empty = store.is_empty();
                check_store_error(&mut *store)?;
                if is_empty {
                    return Ok(BatchDecodeInfo {
                        vv: VersionVector::new(),
                        frontiers: Frontiers::new(),
                        start_version: None,
                        causal_dag: None,
                        import_baseline: None,
                        greatest_timestamp: Some(0),
                        metadata_digest: None,
                    });
                }
                return Err(LoroError::DecodeDataCorruptionError);
            }
            decode_store_metadata(&metadata.unwrap())?
        };
        let (metadata, metadata_digest) = metadata;

        let vv = VersionVector::decode(&metadata.vv)
            .map_err(|_| LoroError::DecodeDataCorruptionError)?;
        let frontiers = Frontiers::decode(&metadata.frontiers)
            .map_err(|_| LoroError::DecodeDataCorruptionError)?;
        let start_vv = if metadata.start_vv.is_empty() {
            VersionVector::new()
        } else {
            VersionVector::decode(&metadata.start_vv)
                .map_err(|_| LoroError::DecodeDataCorruptionError)?
        };
        let start_frontiers = if metadata.start_frontiers.is_empty() {
            Frontiers::new()
        } else {
            Frontiers::decode(&metadata.start_frontiers)
                .map_err(|_| LoroError::DecodeDataCorruptionError)?
        };
        let causal_dag = decode_causal_snapshot(&metadata.causal)?;
        if causal_dag.vv != vv || causal_dag.frontiers != frontiers {
            return Err(LoroError::DecodeDataCorruptionError);
        }
        if causal_dag
            .nodes
            .iter()
            .any(|node| node.boundary_proof.is_none())
        {
            return Err(LoroError::DecodeDataCorruptionError);
        }
        let import_baseline = decode_import_baseline(&metadata.import_baseline)?;
        if import_baseline.vv != vv || import_baseline.frontiers != frontiers {
            return Err(LoroError::DecodeDataCorruptionError);
        }

        *self.external_vv.lock() = vv.clone();
        let start_version = if start_vv.is_empty() {
            None
        } else {
            let mut inner = self.inner.lock();
            inner.start_frontiers = start_frontiers.clone();
            inner.start_vv = ImVersionVector::from_vv(&start_vv);
            Some((start_vv, start_frontiers))
        };
        Ok(BatchDecodeInfo {
            vv,
            frontiers,
            start_version,
            causal_dag: Some(causal_dag),
            import_baseline: Some(import_baseline),
            greatest_timestamp: Some(metadata.greatest_timestamp),
            metadata_digest: Some(metadata_digest),
        })
    }

    #[cfg(test)]
    fn new_for_test() -> Self {
        Self::new_mem(&SharedArena::new(), Arc::new(AtomicI64::new(0)))
    }

    pub(super) fn encode_all(&self, vv: &VersionVector, frontiers: &Frontiers) -> Bytes {
        self.flush_and_compact(vv, frontiers);
        let mut kv = self.external_kv.lock();
        kv.export_all()
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub(super) fn export_from(
        &self,
        start_vv: &VersionVector,
        start_frontiers: &Frontiers,
        latest_vv: &VersionVector,
        latest_frontiers: &Frontiers,
    ) -> Bytes {
        let new_store = ChangeStore::new_mem(&self.arena, self.merge_interval.clone());
        for span in latest_vv.sub_iter(start_vv) {
            // PERF: this can be optimized by reusing the current encoded blocks
            // In the current method, it needs to parse and re-encode the blocks
            for c in self.iter_changes(span) {
                let start = ((start_vv.get(&c.id.peer).copied().unwrap_or(0) - c.id.counter).max(0)
                    as usize)
                    .min(c.atom_len());
                let end = ((latest_vv.get(&c.id.peer).copied().unwrap_or(0) - c.id.counter).max(0)
                    as usize)
                    .min(c.atom_len());

                if start == end {
                    continue;
                }

                let ch = c.slice(start, end);
                new_store.insert_change(ch, false, false);
            }
        }

        loro_common::debug!(
            "start_vv={:?} start_frontiers={:?}",
            &start_vv,
            start_frontiers
        );
        new_store.encode_from(start_vv, start_frontiers, latest_vv, latest_frontiers)
    }

    pub(super) fn export_blocks_in_range<W: std::io::Write>(&self, spans: &[IdSpan], w: &mut W) {
        let new_store = ChangeStore::new_mem(&self.arena, self.merge_interval.clone());
        for span in spans {
            let mut span = *span;
            span.normalize_();
            if span.counter.end <= 0 {
                continue;
            }

            span.counter.start = span.counter.start.max(0);
            span.counter.end = span.counter.end.max(0);
            if span.counter.start >= span.counter.end {
                continue;
            }

            // PERF: this can be optimized by reusing the current encoded blocks
            // In the current method, it needs to parse and re-encode the blocks
            for c in self.iter_changes(span) {
                let start = ((span.counter.start - c.id.counter).max(0) as usize).min(c.atom_len());
                let end = ((span.counter.end - c.id.counter).max(0) as usize).min(c.atom_len());
                if start == end {
                    continue;
                }

                let ch = c.slice(start, end);
                new_store.insert_change(ch, false, false);
            }
        }

        encode_blocks_in_store(new_store, &self.arena, w);
    }

    fn encode_from(
        &self,
        start_vv: &VersionVector,
        start_frontiers: &Frontiers,
        latest_vv: &VersionVector,
        latest_frontiers: &Frontiers,
    ) -> Bytes {
        {
            let mut store = self.external_kv.lock();
            store.set(START_VV_KEY, start_vv.encode().into());
            store.set(START_FRONTIERS_KEY, start_frontiers.encode().into());
            let mut inner = self.inner.lock();
            inner.start_frontiers = start_frontiers.clone();
            inner.start_vv = ImVersionVector::from_vv(start_vv);
        }
        self.flush_and_compact(latest_vv, latest_frontiers);
        self.external_kv.lock().export_all()
    }

    pub(crate) fn decode_snapshot_for_updates(
        bytes: Bytes,
        arena: &SharedArena,
        self_vv: &VersionVector,
    ) -> Result<Vec<Change>, LoroError> {
        let change_store = ChangeStore::new_mem(arena, Arc::new(AtomicI64::new(0)));
        let _ = change_store.import_all(bytes)?;
        let mut changes = Vec::new();
        change_store.visit_all_changes(&mut |c| {
            let cnt_threshold = self_vv.get(&c.id.peer).copied().unwrap_or(0);
            if c.id.counter >= cnt_threshold {
                changes.push(c.clone());
                return;
            }

            let change_end = c.ctr_end();
            if change_end > cnt_threshold {
                changes.push(c.slice((cnt_threshold - c.id.counter) as usize, c.atom_len()));
            }
        });

        Ok(changes)
    }

    pub(crate) fn decode_block_bytes(
        bytes: Bytes,
        arena: &SharedArena,
        self_vv: &VersionVector,
    ) -> LoroResult<Vec<Change>> {
        let mut ans = ChangesBlockBytes::new(bytes).parse(arena)?;
        if ans.is_empty() {
            return Ok(ans);
        }

        let start = self_vv.get(&ans[0].peer()).copied().unwrap_or(0);
        ans.retain_mut(|c| {
            if c.id.counter >= start {
                true
            } else if c.ctr_end() > start {
                *c = c.slice((start - c.id.counter) as usize, c.atom_len());
                true
            } else {
                false
            }
        });

        Ok(ans)
    }

    pub(crate) fn rollback_import(&self, rollback: ChangeStoreRollback) {
        let mut inner = self.inner.lock();
        inner.mem_parsed_kv.retain(|id, _| {
            let old_end = rollback.old_vv.get(&id.peer).copied().unwrap_or(0);
            id.counter < old_end
        });

        for (id, block) in rollback.blocks_before_mutation {
            inner.mem_parsed_kv.insert(id, block);
        }
    }

    pub fn get_dag_nodes_that_contains(&self, id: ID) -> Option<Vec<AppDagNode>> {
        let block = self.get_block_that_contains(id)?;
        Some(block.content.iter_dag_nodes())
    }

    pub fn get_last_dag_nodes_for_peer(&self, peer: PeerID) -> Option<Vec<AppDagNode>> {
        let block = self.get_the_last_block_of_peer(peer)?;
        Some(block.content.iter_dag_nodes())
    }

    pub fn visit_all_changes(&self, f: &mut dyn FnMut(&Change)) {
        self.ensure_block_loaded_in_range(Bound::Unbounded, Bound::Unbounded);
        let mut inner = self.inner.lock();
        for (id, block) in inner.mem_parsed_kv.iter_mut() {
            if let Err(err) = block.ensure_changes(&self.arena) {
                warn!(block_id = ?id, ?err, "failed to parse change block");
                continue;
            }
            for c in block.content.try_changes().unwrap() {
                f(c);
            }
        }
    }

    pub(crate) fn iter_blocks(&self, id_span: IdSpan) -> Vec<(Arc<ChangesBlock>, usize, usize)> {
        if id_span.counter.start == id_span.counter.end {
            return vec![];
        }

        assert!(id_span.counter.start < id_span.counter.end);
        self.ensure_block_loaded_in_range(
            Bound::Included(id_span.id_start()),
            Bound::Excluded(id_span.id_end()),
        );
        let mut inner = self.inner.lock();
        let next_back = inner.mem_parsed_kv.range(..=id_span.id_start()).next_back();
        match next_back {
            None => {
                return vec![];
            }
            Some(next_back) => {
                if next_back.0.peer != id_span.peer {
                    return vec![];
                }
            }
        }
        let start_counter = next_back.map(|(id, _)| id.counter).unwrap_or(0);
        let ans = inner
            .mem_parsed_kv
            .range_mut(
                ID::new(id_span.peer, start_counter)..ID::new(id_span.peer, id_span.counter.end),
            )
            .filter_map(|(_id, block)| {
                if block.counter_range.1 < id_span.counter.start {
                    return None;
                }

                if let Err(err) = block.ensure_changes(&self.arena) {
                    warn!(block_id = ?_id, ?err, "failed to parse change block");
                    return None;
                }
                let changes = block.content.try_changes().unwrap();
                let start;
                let end;
                if id_span.counter.start <= block.counter_range.0
                    && id_span.counter.end >= block.counter_range.1
                {
                    start = 0;
                    end = changes.len();
                } else {
                    start = block
                        .get_change_index_by_counter(id_span.counter.start)
                        .unwrap_or_else(|x| x);

                    match block.get_change_index_by_counter(id_span.counter.end - 1) {
                        Ok(e) => {
                            end = e + 1;
                        }
                        Err(0) => return None,
                        Err(e) => {
                            end = e;
                        }
                    }
                }
                if start == end {
                    return None;
                }

                Some((block.clone(), start, end))
            })
            // TODO: PERF avoid alloc
            .collect_vec();

        ans
    }

    pub fn iter_changes(&self, id_span: IdSpan) -> impl Iterator<Item = BlockChangeRef> + '_ {
        let v = self.iter_blocks(id_span);
        #[cfg(debug_assertions)]
        {
            if !v.is_empty() {
                assert_eq!(v[0].0.peer, id_span.peer);
                assert_eq!(v.last().unwrap().0.peer, id_span.peer);
                {
                    // Test start
                    let (block, start, _end) = v.first().unwrap();
                    let changes = block.content.try_changes().unwrap();
                    assert!(changes[*start].id.counter <= id_span.counter.start);
                }
                {
                    // Test end
                    let (block, _start, end) = v.last().unwrap();
                    let changes = block.content.try_changes().unwrap();
                    assert!(changes[*end - 1].ctr_end() >= id_span.counter.end);
                    assert!(changes[*end - 1].id.counter < id_span.counter.end);
                }
            }
        }

        v.into_iter().flat_map(move |(block, start, end)| {
            (start..end).map(move |i| BlockChangeRef {
                change_index: i,
                block: block.clone(),
            })
        })
    }

    #[allow(dead_code)]
    pub(crate) fn get_blocks_in_range(&self, id_span: IdSpan) -> VecDeque<Arc<ChangesBlock>> {
        let mut inner = self.inner.lock();
        let start_counter = inner
            .mem_parsed_kv
            .range(..=id_span.id_start())
            .next_back()
            .map(|(id, _)| id.counter)
            .unwrap_or(0);
        let vec = inner
            .mem_parsed_kv
            .range_mut(
                ID::new(id_span.peer, start_counter)..ID::new(id_span.peer, id_span.counter.end),
            )
            .filter_map(|(_id, block)| {
                if block.counter_range.1 < id_span.counter.start {
                    return None;
                }

                if let Err(err) = block.ensure_changes(&self.arena) {
                    warn!(block_id = ?_id, ?err, "failed to parse change block");
                    return None;
                }
                Some(block.clone())
            })
            // TODO: PERF avoid alloc
            .collect();
        vec
    }

    pub(crate) fn get_block_that_contains(&self, id: ID) -> Option<Arc<ChangesBlock>> {
        self.ensure_block_loaded_in_range(Bound::Included(id), Bound::Included(id));
        let inner = self.inner.lock();
        let block = inner
            .mem_parsed_kv
            .range(..=id)
            .next_back()
            .filter(|(_, block)| {
                block.peer == id.peer
                    && block.counter_range.0 <= id.counter
                    && id.counter < block.counter_range.1
            })
            .map(|(_, block)| block.clone());

        block
    }

    pub(crate) fn get_the_last_block_of_peer(&self, peer: PeerID) -> Option<Arc<ChangesBlock>> {
        let end_id = ID::new(peer, Counter::MAX);
        self.ensure_id_lte(end_id);
        let inner = self.inner.lock();
        let block = inner
            .mem_parsed_kv
            .range(..=end_id)
            .next_back()
            .filter(|(_, block)| block.peer == peer)
            .map(|(_, block)| block.clone());

        block
    }

    pub fn change_num(&self) -> usize {
        self.ensure_block_loaded_in_range(Bound::Unbounded, Bound::Unbounded);
        let mut inner = self.inner.lock();
        inner
            .mem_parsed_kv
            .iter_mut()
            .map(|(_, block)| block.change_num())
            .sum()
    }

    pub fn fork(
        &self,
        arena: SharedArena,
        merge_interval: Arc<AtomicI64>,
        vv: &VersionVector,
        frontiers: &Frontiers,
    ) -> Self {
        self.flush_and_compact(vv, frontiers);
        let inner = self.inner.lock();
        Self {
            inner: Arc::new(Mutex::new(ChangeStoreInner {
                start_vv: inner.start_vv.clone(),
                start_frontiers: inner.start_frontiers.clone(),
                mem_parsed_kv: BTreeMap::new(),
            })),
            arena,
            external_vv: Arc::new(Mutex::new(self.external_vv.lock().clone())),
            external_kv: self.external_kv.lock().clone_store(),
            merge_interval,
            is_external: self.is_external,
        }
    }

    pub fn kv_size(&self) -> usize {
        self.external_kv
            .lock()
            .scan(Bound::Unbounded, Bound::Unbounded)
            .map(|(k, v)| k.len() + v.len())
            .sum()
    }

    pub(crate) fn export_blocks_from<W: std::io::Write>(
        &self,
        start_vv: &VersionVector,
        shallow_since_vv: &ImVersionVector,
        latest_vv: &VersionVector,
        w: &mut W,
    ) {
        let new_store = ChangeStore::new_mem(&self.arena, self.merge_interval.clone());
        for mut span in latest_vv.sub_iter(start_vv) {
            let counter_lower_bound = shallow_since_vv.get(&span.peer).copied().unwrap_or(0);
            span.counter.start = span.counter.start.max(counter_lower_bound);
            span.counter.end = span.counter.end.max(counter_lower_bound);
            if span.counter.start >= span.counter.end {
                continue;
            }

            // PERF: this can be optimized by reusing the current encoded blocks
            // In the current method, it needs to parse and re-encode the blocks
            for c in self.iter_changes(span) {
                let start = ((start_vv.get(&c.id.peer).copied().unwrap_or(0) - c.id.counter).max(0)
                    as usize)
                    .min(c.atom_len());
                let end = ((latest_vv.get(&c.id.peer).copied().unwrap_or(0) - c.id.counter).max(0)
                    as usize)
                    .min(c.atom_len());

                assert_ne!(start, end);
                let ch = c.slice(start, end);
                new_store.insert_change(ch, false, false);
            }
        }

        let arena = &self.arena;
        encode_blocks_in_store(new_store, arena, w);
    }

    pub(crate) fn fork_changes_up_to(
        &self,
        start_vv: &ImVersionVector,
        frontiers: &Frontiers,
        vv: &VersionVector,
    ) -> Bytes {
        let new_store = ChangeStore::new_mem(&self.arena, self.merge_interval.clone());
        for mut span in vv.sub_iter_im(start_vv) {
            let counter_lower_bound = start_vv.get(&span.peer).copied().unwrap_or(0);
            span.counter.start = span.counter.start.max(counter_lower_bound);
            span.counter.end = span.counter.end.max(counter_lower_bound);
            if span.counter.start >= span.counter.end {
                continue;
            }

            // PERF: this can be optimized by reusing the current encoded blocks
            // In the current method, it needs to parse and re-encode the blocks
            for c in self.iter_changes(span) {
                let start = ((start_vv.get(&c.id.peer).copied().unwrap_or(0) - c.id.counter).max(0)
                    as usize)
                    .min(c.atom_len());
                let end = ((vv.get(&c.id.peer).copied().unwrap_or(0) - c.id.counter).max(0)
                    as usize)
                    .min(c.atom_len());

                assert_ne!(start, end);
                let ch = c.slice(start, end);
                new_store.insert_change(ch, false, false);
            }
        }

        new_store.encode_all(vv, frontiers)
    }

    pub(crate) fn evict_parsed_cache(&self) {
        self.inner.lock().mem_parsed_kv.clear();
    }

    pub(crate) fn external_version_is(&self, vv: &VersionVector) -> bool {
        self.is_external && &*self.external_vv.lock() == vv
    }

    pub(crate) fn prepare_external_metadata_semantics(
        &self,
        snapshot: &mut CausalDagSnapshot,
        prior_snapshot: Option<&CausalDagSnapshot>,
        predecessor_metadata_digest: Option<[u8; 32]>,
    ) -> LoroResult<()> {
        if snapshot.vv != *self.external_vv.lock() && self.inner.lock().mem_parsed_kv.is_empty() {
            return Err(LoroError::DecodeError(
                "external causal metadata version has no corresponding change blocks".into(),
            ));
        }
        let mut cache = ExternalValidationCache::new(self);
        let prior_nodes = prior_snapshot
            .into_iter()
            .flat_map(|snapshot| snapshot.nodes.iter())
            .map(|node| ((node.peer, node.cnt), node))
            .collect::<BTreeMap<_, _>>();
        for node in &mut snapshot.nodes {
            let len = Counter::try_from(node.len)
                .map_err(|_| causal_validation_error("node span is too large"))?;
            let end = node
                .cnt
                .checked_add(len)
                .ok_or_else(|| causal_validation_error("node span overflows"))?;
            let peer_end = snapshot
                .vv
                .get(&node.peer)
                .copied()
                .ok_or_else(|| causal_validation_error("node peer is absent from its VV"))?;
            let mut cursor = node.cnt;
            let mut parts = Vec::new();
            let mut memory_changes = None;

            while cursor < end {
                if let Some(prior) = prior_node_containing(&prior_nodes, node.peer, cursor)? {
                    let prior_len = Counter::try_from(prior.len).map_err(|_| {
                        causal_validation_error("prior authenticated node span is too large")
                    })?;
                    let prior_end = prior.cnt.checked_add(prior_len).ok_or_else(|| {
                        causal_validation_error("prior authenticated node span overflows")
                    })?;
                    let expected_lamport = node.lamport + (cursor - node.cnt) as Lamport;
                    let expected_deps = if cursor == node.cnt {
                        node.deps.clone()
                    } else {
                        Frontiers::from_id(ID::new(node.peer, cursor - 1))
                    };
                    let prior_offset = u32::try_from(cursor - prior.cnt)
                        .map_err(|_| causal_validation_error("prior span offset is invalid"))?;
                    let actual_deps = if cursor == prior.cnt {
                        prior.deps.clone()
                    } else {
                        Frontiers::from_id(ID::new(node.peer, cursor - 1))
                    };
                    if prior.lamport + prior_offset != expected_lamport
                        || actual_deps != expected_deps
                    {
                        return Err(causal_validation_error(
                            "successor node cannot omit a prior nonlinear boundary",
                        ));
                    }
                    let overlap_end = prior_end.min(end);
                    append_prior_boundary_parts(&mut parts, prior, cursor, overlap_end)?;
                    cursor = overlap_end;
                    continue;
                }

                let next_prior_start = prior_nodes
                    .range((
                        Bound::Excluded((node.peer, cursor)),
                        Bound::Excluded((node.peer, end)),
                    ))
                    .next()
                    .map(|((_, start), _)| *start)
                    .unwrap_or(end);
                let source_end = next_prior_start.min(end);
                let changes = memory_changes
                    .get_or_insert(cache.memory_changes_for_peer(node.peer, peer_end)?);
                let part = materialize_boundary_part(
                    node,
                    cursor,
                    source_end,
                    changes,
                    predecessor_metadata_digest.unwrap_or([0; 32]),
                )?;
                cursor = source_end;
                parts.push(part);
            }
            if cursor != end || parts.is_empty() {
                return Err(causal_validation_error(
                    "node is not fully covered by authenticated or new change headers",
                ));
            }
            let mut proof = CausalBoundaryProof {
                parts,
                node_digest: [0; 32],
            };
            proof.node_digest = causal_node_digest(node, &proof);
            node.boundary_proof = Some(proof);
        }

        Ok(())
    }

    pub(crate) fn validate_persisted_external_metadata_semantics(
        &self,
        snapshot: &CausalDagSnapshot,
    ) -> LoroResult<()> {
        if snapshot.vv != *self.external_vv.lock() {
            return Err(LoroError::DecodeError(
                "external causal metadata version has no corresponding change blocks".into(),
            ));
        }
        for node in &snapshot.nodes {
            let proof = node.boundary_proof.as_ref().ok_or_else(|| {
                LoroError::DecodeError("external causal DAG boundary proof is missing".into())
            })?;
            if proof.node_digest != causal_node_digest(node, proof) {
                return Err(LoroError::DecodeError(
                    "external causal DAG boundary commitment does not match its causal span".into(),
                ));
            }
        }
        Ok(())
    }
}

fn causal_validation_error(message: &str) -> LoroError {
    LoroError::DecodeError(format!("external causal DAG {message}").into_boxed_str())
}

fn prior_node_containing<'a>(
    prior_nodes: &'a BTreeMap<(PeerID, Counter), &'a crate::external_store::CausalDagNode>,
    peer: PeerID,
    counter: Counter,
) -> LoroResult<Option<&'a crate::external_store::CausalDagNode>> {
    let Some((_, node)) = prior_nodes.range(..=(peer, counter)).next_back() else {
        return Ok(None);
    };
    if node.peer != peer {
        return Ok(None);
    }
    let len = Counter::try_from(node.len)
        .map_err(|_| causal_validation_error("prior authenticated span is too large"))?;
    let end = node
        .cnt
        .checked_add(len)
        .ok_or_else(|| causal_validation_error("prior authenticated span overflows"))?;
    Ok((node.cnt <= counter && counter < end).then_some(*node))
}

fn append_prior_boundary_parts(
    output: &mut Vec<CausalBoundaryPart>,
    prior: &crate::external_store::CausalDagNode,
    start: Counter,
    end: Counter,
) -> LoroResult<()> {
    let proof = prior.boundary_proof.as_ref().ok_or_else(|| {
        causal_validation_error("prior authenticated causal span has no boundary proof")
    })?;
    let mut cursor = start;
    for part in &proof.parts {
        let overlap_start = part.start.max(start);
        let overlap_end = part.end.min(end);
        if overlap_start >= overlap_end {
            continue;
        }
        if overlap_start != cursor {
            return Err(causal_validation_error(
                "prior authenticated boundary parts are incomplete",
            ));
        }
        let mut part = part.clone();
        part.start = overlap_start;
        part.end = overlap_end;
        if let Some(previous) = output.last_mut() {
            if previous.end == part.start
                && previous.source_digest == part.source_digest
                && previous.source_start == part.source_start
                && previous.source_end == part.source_end
            {
                previous.end = part.end;
                cursor = overlap_end;
                continue;
            }
        }
        output.push(part);
        cursor = overlap_end;
    }
    if cursor != end {
        return Err(causal_validation_error(
            "prior authenticated boundary parts do not cover the requested slice",
        ));
    }
    Ok(())
}

fn materialize_boundary_part(
    node: &crate::external_store::CausalDagNode,
    source_start: Counter,
    source_end: Counter,
    changes: &[(BlockChangeRef, SharedArena)],
    anchor_metadata_digest: [u8; 32],
) -> LoroResult<CausalBoundaryPart> {
    if source_start >= source_end {
        return Err(causal_validation_error("new boundary source span is empty"));
    }
    let source_offset = u32::try_from(source_start - node.cnt)
        .map_err(|_| causal_validation_error("new boundary source offset is invalid"))?;
    let source_lamport = node.lamport + source_offset;
    let source_deps = if source_start == node.cnt {
        node.deps.clone()
    } else {
        Frontiers::from_id(ID::new(node.peer, source_start - 1))
    };
    let mut boundary_digest = Sha256::new();
    boundary_digest.update(b"LORO causal boundaries v3");
    let mut cursor = source_start;
    let mut change_count = 0_u32;
    let mut last_change_start = None;
    while cursor < source_end {
        let change = changes
            .iter()
            .find_map(|(change, _)| {
                change
                    .contains_id(ID::new(node.peer, cursor))
                    .then_some(change)
            })
            .ok_or_else(|| {
                causal_validation_error(
                    "new causal span is not covered by newly materialized change headers",
                )
            })?;
        let actual_lamport = change.lamport + (cursor - change.id.counter) as Lamport;
        let expected_lamport = source_lamport + (cursor - source_start) as Lamport;
        if actual_lamport != expected_lamport {
            return Err(causal_validation_error(
                "new source lamport does not match its stored change boundary",
            ));
        }
        let actual_deps = if cursor == change.id.counter {
            change.deps.clone()
        } else {
            Frontiers::from_id(ID::new(node.peer, cursor - 1))
        };
        let expected_deps = if cursor == source_start {
            source_deps.clone()
        } else {
            Frontiers::from_id(ID::new(node.peer, cursor - 1))
        };
        if actual_deps != expected_deps {
            return Err(causal_validation_error(
                "new source crosses a nonlinear change boundary",
            ));
        }
        change_count = change_count
            .checked_add(1)
            .ok_or_else(|| causal_validation_error("boundary count overflows"))?;
        last_change_start = Some(change.id.counter);
        update_boundary_digest(&mut boundary_digest, change);
        cursor = change.ctr_end().min(source_end);
    }
    if cursor != source_end || change_count == 0 {
        return Err(causal_validation_error(
            "new source is not fully covered by change headers",
        ));
    }
    let mut part = CausalBoundaryPart {
        start: source_start,
        end: source_end,
        source_start,
        source_end,
        source_lamport,
        source_deps,
        change_count,
        last_change_start: last_change_start
            .ok_or_else(|| causal_validation_error("new source has no change boundary"))?,
        boundary_digest: boundary_digest.finalize().into(),
        anchor_metadata_digest,
        source_digest: [0; 32],
    };
    part.source_digest = causal_source_digest(node.peer, &part);
    Ok(part)
}

fn update_boundary_digest(digest: &mut Sha256, change: &BlockChangeRef) {
    digest.update(change.id.peer.to_le_bytes());
    digest.update(change.id.counter.to_le_bytes());
    digest.update(change.lamport.to_le_bytes());
    digest.update(change.ctr_end().to_le_bytes());
    digest.update((change.deps.len() as u64).to_le_bytes());
    for dep in change.deps.iter() {
        digest.update(dep.peer.to_le_bytes());
        digest.update(dep.counter.to_le_bytes());
    }
}

fn check_store_error(store: &mut dyn KvStore) -> LoroResult<()> {
    match store.take_error() {
        Some(error) => Err(external_store_error(error)),
        None => Ok(()),
    }
}

fn external_store_error(error: impl Into<String>) -> LoroError {
    LoroError::Unknown(format!("external change store error: {}", error.into()).into_boxed_str())
}

fn encode_blocks_in_store<W: std::io::Write>(
    new_store: ChangeStore,
    arena: &SharedArena,
    w: &mut W,
) {
    let mut inner = new_store.inner.lock();
    for (_id, block) in inner.mem_parsed_kv.iter_mut() {
        let bytes = block.to_bytes(arena);
        leb128::write::unsigned(w, bytes.bytes.len() as u64).unwrap();
        w.write_all(&bytes.bytes).unwrap();
    }
}

mod mut_external_kv {
    //! Only this module contains the code that mutate the external kv store
    //! All other modules should only read from the external kv store
    use super::*;

    impl ChangeStore {
        #[tracing::instrument(skip_all, level = "debug", name = "change_store import_all")]
        pub(crate) fn import_all(&self, bytes: Bytes) -> Result<BatchDecodeInfo, LoroError> {
            let mut kv_store = self.external_kv.lock();
            assert!(
                // 2 because there are vv and frontiers
                kv_store.len() <= 2,
                "kv store should be empty when using decode_all"
            );
            kv_store
                .import_all(bytes)
                .map_err(|e| LoroError::DecodeError(e.into_boxed_str()))?;
            drop(kv_store);
            let vv_bytes = self.external_kv.lock().get(VV_KEY).unwrap_or_default();
            let vv = VersionVector::decode(&vv_bytes)
                .map_err(|_| LoroError::DecodeDataCorruptionError)?;
            let start_vv_bytes = self
                .external_kv
                .lock()
                .get(START_VV_KEY)
                .unwrap_or_default();
            let start_vv = if start_vv_bytes.is_empty() {
                Default::default()
            } else {
                VersionVector::decode(&start_vv_bytes)
                    .map_err(|_| LoroError::DecodeDataCorruptionError)?
            };

            #[cfg(test)]
            {
                // This is for tests
                for (peer, cnt) in vv.iter() {
                    self.get_change(ID::new(*peer, *cnt - 1))
                        .ok_or(LoroError::DecodeDataCorruptionError)?;
                }
            }

            *self.external_vv.lock() = vv.clone();
            let frontiers_bytes = self
                .external_kv
                .lock()
                .get(FRONTIERS_KEY)
                .unwrap_or_default();
            let frontiers = Frontiers::decode(&frontiers_bytes)
                .map_err(|_| LoroError::DecodeDataCorruptionError)?;
            let start_frontiers = self
                .external_kv
                .lock()
                .get(START_FRONTIERS_KEY)
                .unwrap_or_default();
            let start_frontiers = if start_frontiers.is_empty() {
                Default::default()
            } else {
                Frontiers::decode(&start_frontiers)
                    .map_err(|_| LoroError::DecodeDataCorruptionError)?
            };

            let mut max_lamport = None;
            let mut max_timestamp = 0;
            for id in frontiers.iter() {
                let c = self
                    .get_change(id)
                    .ok_or(LoroError::DecodeDataCorruptionError)?;
                debug_assert_ne!(c.atom_len(), 0);
                let l = c.lamport_last();
                if let Some(x) = max_lamport {
                    if l > x {
                        max_lamport = Some(l);
                    }
                } else {
                    max_lamport = Some(l);
                }

                let t = c.timestamp;
                if t > max_timestamp {
                    max_timestamp = t;
                }
            }

            Ok(BatchDecodeInfo {
                vv,
                frontiers,
                start_version: if start_vv.is_empty() {
                    None
                } else {
                    let mut inner = self.inner.lock();
                    inner.start_frontiers = start_frontiers.clone();
                    inner.start_vv = ImVersionVector::from_vv(&start_vv);
                    Some((start_vv, start_frontiers))
                },
                causal_dag: None,
                import_baseline: None,
                greatest_timestamp: None,
                metadata_digest: None,
            })
        }

        /// Flush the cached change to kv_store
        pub(crate) fn flush_and_compact(&self, vv: &VersionVector, frontiers: &Frontiers) {
            self.try_flush_and_compact(vv, frontiers)
                .expect("change store flush failed");
        }

        pub(crate) fn try_flush_and_compact(
            &self,
            vv: &VersionVector,
            frontiers: &Frontiers,
        ) -> LoroResult<()> {
            self.try_flush_and_compact_inner(vv, frontiers, None)
                .map(|_| ())
        }

        pub(crate) fn try_flush_external_with_causal(
            &self,
            vv: &VersionVector,
            frontiers: &Frontiers,
            causal_dag: Bytes,
            import_baseline: Bytes,
            greatest_timestamp: i64,
        ) -> LoroResult<[u8; 32]> {
            if !self.is_external {
                return Err(LoroError::ArgErr(
                    "causal metadata is only valid for an external change store".into(),
                ));
            }
            self.try_flush_and_compact_inner(
                vv,
                frontiers,
                Some((causal_dag, import_baseline, greatest_timestamp)),
            )
            .and_then(|digest| {
                digest.ok_or_else(|| LoroError::Unknown("external metadata was not written".into()))
            })
        }

        fn try_flush_and_compact_inner(
            &self,
            vv: &VersionVector,
            frontiers: &Frontiers,
            external_metadata: Option<(Bytes, Bytes, i64)>,
        ) -> LoroResult<Option<[u8; 32]>> {
            let mut inner = self.inner.lock();
            let mut store = self.external_kv.lock();
            check_store_error(&mut *store)?;
            let mut external_vv = self.external_vv.lock();
            for (id, block) in inner.mem_parsed_kv.iter_mut() {
                if !block.flushed {
                    let id_bytes = id.to_bytes();
                    let counter_start = external_vv.get(&id.peer).copied().unwrap_or(0);
                    if counter_start >= block.counter_range.1 {
                        return Err(LoroError::DecodeError(
                            "external change block version is inconsistent".into(),
                        ));
                    }
                    if counter_start > block.counter_range.0 {
                        if store.get(&id_bytes).is_none() {
                            check_store_error(&mut *store)?;
                            return Err(LoroError::DecodeError(
                                "external partial change block is missing".into(),
                            ));
                        }
                        check_store_error(&mut *store)?;
                    }
                    let bytes = block.to_bytes(&self.arena);
                    store.set(&id_bytes, bytes.bytes);
                    check_store_error(&mut *store)?;
                    external_vv.insert(id.peer, block.counter_range.1);
                    Arc::make_mut(block).flushed = true;
                }
            }

            if inner.start_vv.is_empty() {
                if &*external_vv != vv {
                    return Err(LoroError::DecodeError(
                        "external change blocks do not cover the live version".into(),
                    ));
                }
            } else {
                #[cfg(debug_assertions)]
                {
                    // TODO: makes some assertions here?
                }
            }
            let vv_bytes = vv.encode();
            let frontiers_bytes = frontiers.encode();
            store.set(VV_KEY, vv_bytes.clone().into());
            check_store_error(&mut *store)?;
            store.set(FRONTIERS_KEY, frontiers_bytes.clone().into());
            check_store_error(&mut *store)?;
            let metadata_digest = if let Some((causal_dag, import_baseline, greatest_timestamp)) =
                external_metadata
            {
                let metadata = encode_store_metadata(ExternalStoreMetadata {
                    vv: vv_bytes.into(),
                    frontiers: frontiers_bytes.into(),
                    start_vv: if inner.start_vv.is_empty() {
                        Bytes::new()
                    } else {
                        VersionVector::from_im_vv(&inner.start_vv).encode().into()
                    },
                    start_frontiers: if inner.start_vv.is_empty() {
                        Bytes::new()
                    } else {
                        inner.start_frontiers.encode().into()
                    },
                    causal: causal_dag.clone(),
                    import_baseline: import_baseline.clone(),
                    greatest_timestamp,
                })?;
                let (_, digest) = decode_store_metadata(&metadata)?;
                store.set(EXTERNAL_METADATA_KEY, metadata);
                check_store_error(&mut *store)?;
                Some(digest)
            } else {
                None
            };
            store.flush();
            check_store_error(&mut *store)?;
            Ok(metadata_digest)
        }
    }
}

mod mut_inner_kv {
    //! Only this module contains the code that mutate the internal kv store
    //! All other modules should only read from the internal kv store

    use super::*;
    impl ChangeStore {
        /// This method is the **only place** that push a new change into the change store
        ///
        /// The new change either merges with the previous block or is put into a new block.
        /// This method only updates the internal kv store.
        pub fn insert_change(&self, change: Change, split_when_exceeds: bool, is_local: bool) {
            self.insert_change_inner(change, split_when_exceeds, is_local, None);
        }

        pub(crate) fn insert_change_with_rollback(
            &self,
            change: Change,
            split_when_exceeds: bool,
            is_local: bool,
            rollback: &mut ChangeStoreRollback,
        ) {
            self.insert_change_inner(change, split_when_exceeds, is_local, Some(rollback));
        }

        fn insert_change_inner(
            &self,
            mut change: Change,
            split_when_exceeds: bool,
            is_local: bool,
            mut rollback: Option<&mut ChangeStoreRollback>,
        ) {
            #[cfg(debug_assertions)]
            {
                let vv = self.external_vv.lock();
                assert!(vv.get(&change.id.peer).copied().unwrap_or(0) <= change.id.counter);
            }

            let s = info_span!("change_store insert_change", id = ?change.id);
            let _e = s.enter();
            let estimated_size = change.estimate_storage_size();
            if estimated_size > MAX_BLOCK_SIZE && split_when_exceeds {
                self.split_change_then_insert(change, rollback.as_deref_mut());
                return;
            }

            let id = change.id;
            let mut inner = self.inner.lock();

            // try to merge with previous block
            if let Some((_id, block)) = inner.mem_parsed_kv.range_mut(..id).next_back() {
                if block.peer == change.id.peer {
                    if block.counter_range.1 != change.id.counter {
                        panic!("counter should be continuous")
                    }

                    if let Some(rollback) = &mut rollback {
                        rollback.record_block_before_mutation(*_id, block.clone());
                    }

                    match block.push_change(
                        change,
                        estimated_size,
                        if is_local {
                            // local change should try to merge with previous change when
                            // the timestamp interval <= the `merge_interval`
                            self.merge_interval
                                .load(std::sync::atomic::Ordering::Acquire)
                        } else {
                            0
                        },
                        &self.arena,
                    ) {
                        Ok(_) => {
                            drop(inner);
                            debug_assert!(self.get_change(id).is_some());
                            return;
                        }
                        Err(c) => change = c,
                    }
                }
            }

            inner
                .mem_parsed_kv
                .insert(id, Arc::new(ChangesBlock::new(change, &self.arena)));
            drop(inner);
            debug_assert!(self.get_change(id).is_some());
        }

        pub fn get_change(&self, id: ID) -> Option<BlockChangeRef> {
            let block = self.get_parsed_block(id)?;
            let change_index = block.get_change_index_by_counter(id.counter).ok()?;
            Some(BlockChangeRef {
                change_index,
                block: block.clone(),
            })
        }

        pub(crate) fn get_change_fallible(&self, id: ID) -> LoroResult<BlockChangeRef> {
            if let Some(change) = self.get_change(id) {
                return Ok(change);
            }
            if let Some(error) = self.take_external_error() {
                return Err(error);
            }
            Err(LoroError::DecodeError(
                format!("external change block containing {id} is missing or corrupt")
                    .into_boxed_str(),
            ))
        }

        /// Get the change with the given peer and lamport.
        ///
        /// If not found, return the change with the greatest lamport that is smaller than the given lamport.
        pub fn get_change_by_lamport_lte(&self, idlp: IdLp) -> Option<BlockChangeRef> {
            // This method is complicated because we impl binary search on top of the range api
            // It can be simplified
            let mut inner = self.inner.lock();
            let mut iter = inner
                .mem_parsed_kv
                .range_mut(ID::new(idlp.peer, 0)..ID::new(idlp.peer, i32::MAX));

            // This won't change, we only adjust upper_bound
            let mut lower_bound = 0;
            let mut upper_bound = i32::MAX;
            let mut is_binary_searching = false;
            loop {
                match iter.next_back() {
                    Some((&id, block)) => {
                        if block.lamport_range.0 <= idlp.lamport
                            && (!is_binary_searching || idlp.lamport < block.lamport_range.1)
                        {
                            if !is_binary_searching
                                && upper_bound != i32::MAX
                                && upper_bound != block.counter_range.1
                            {
                                warn!(
                                    "There is a hole between the last block and the current block"
                                );
                                // There is hole between the last block and the current block
                                // We need to load it from the kv store
                                break;
                            }

                            // Found the block
                            if let Err(err) = block.ensure_changes(&self.arena) {
                                warn!(block_id = ?id, ?err, "failed to parse change block");
                                return None;
                            }
                            let index = block.get_change_index_by_lamport_lte(idlp.lamport)?;
                            return Some(BlockChangeRef {
                                change_index: index,
                                block: block.clone(),
                            });
                        }

                        if is_binary_searching {
                            let mid_bound = (lower_bound + upper_bound) / 2;
                            if block.lamport_range.1 <= idlp.lamport {
                                // Target is larger than the current block (pointed by mid_bound)
                                lower_bound = mid_bound;
                            } else {
                                debug_assert!(
                                    idlp.lamport < block.lamport_range.0,
                                    "{} {:?}",
                                    idlp,
                                    &block.lamport_range
                                );
                                // Target is smaller than the current block (pointed by mid_bound)
                                upper_bound = mid_bound;
                            }

                            let mid_bound = (lower_bound + upper_bound) / 2;
                            iter = inner
                                .mem_parsed_kv
                                .range_mut(ID::new(idlp.peer, 0)..ID::new(idlp.peer, mid_bound));
                        } else {
                            // Test whether we need to switch to binary search by measuring the gap
                            if block.lamport_range.0 - idlp.lamport > MAX_BLOCK_SIZE as Lamport * 8
                            {
                                // Use binary search to find the block
                                upper_bound = id.counter;
                                let mid_bound = (lower_bound + upper_bound) / 2;
                                iter = inner.mem_parsed_kv.range_mut(
                                    ID::new(idlp.peer, 0)..ID::new(idlp.peer, mid_bound),
                                );
                                is_binary_searching = true;
                            }

                            upper_bound = id.counter;
                        }
                    }
                    None => {
                        if !is_binary_searching {
                            break;
                        }

                        let mid_bound = (lower_bound + upper_bound) / 2;
                        lower_bound = mid_bound;
                        if upper_bound - lower_bound <= MAX_BLOCK_SIZE as i32 {
                            // If they are too close, we can just scan the range
                            iter = inner.mem_parsed_kv.range_mut(
                                ID::new(idlp.peer, lower_bound)..ID::new(idlp.peer, upper_bound),
                            );
                            is_binary_searching = false;
                        } else {
                            let mid_bound = (lower_bound + upper_bound) / 2;
                            iter = inner
                                .mem_parsed_kv
                                .range_mut(ID::new(idlp.peer, 0)..ID::new(idlp.peer, mid_bound));
                        }
                    }
                }
            }

            let counter_end = upper_bound;
            let scan_end = ID::new(idlp.peer, counter_end).to_bytes();

            let (id, bytes) = 'block_scan: {
                let kv_store = &self.external_kv.lock();
                let iter = kv_store
                    .scan(
                        Bound::Included(&ID::new(idlp.peer, 0).to_bytes()),
                        Bound::Excluded(&scan_end),
                    )
                    .rev();

                for (id, bytes) in iter {
                    let mut block = ChangesBlockBytes::new(bytes.clone());
                    let (lamport_start, _lamport_end) = match block.lamport_range() {
                        Ok(range) => range,
                        Err(err) => {
                            let block_id = ID::from_bytes(&id);
                            warn!(
                                ?block_id,
                                ?err,
                                "failed to decode external change block range"
                            );
                            continue;
                        }
                    };
                    if lamport_start <= idlp.lamport {
                        break 'block_scan (id, bytes);
                    }
                }

                return None;
            };

            let block_id = ID::from_bytes(&id);
            let mut block = match ChangesBlock::from_bytes(bytes) {
                Ok(block) => Arc::new(block),
                Err(err) => {
                    warn!(?block_id, ?err, "failed to decode external change block");
                    return None;
                }
            };
            if let Err(err) = block.ensure_changes(&self.arena) {
                warn!(?block_id, ?err, "failed to parse external change block");
                return None;
            }
            inner.mem_parsed_kv.insert(block_id, block.clone());
            let index = block.get_change_index_by_lamport_lte(idlp.lamport)?;
            Some(BlockChangeRef {
                change_index: index,
                block,
            })
        }

        fn split_change_then_insert(
            &self,
            change: Change,
            mut rollback: Option<&mut ChangeStoreRollback>,
        ) {
            let original_len = change.atom_len();
            let mut new_change = Change {
                ops: RleVec::new(),
                deps: change.deps,
                id: change.id,
                lamport: change.lamport,
                timestamp: change.timestamp,
                commit_msg: change.commit_msg.clone(),
            };

            let mut total_len = 0;
            let mut estimated_size = new_change.estimate_storage_size();
            'outer: for mut op in change.ops.into_iter() {
                if op.estimate_storage_size() >= MAX_BLOCK_SIZE - estimated_size {
                    new_change = self._insert_splitted_change(
                        new_change,
                        &mut total_len,
                        &mut estimated_size,
                        rollback.as_deref_mut(),
                    );
                }

                while let Some(end) =
                    op.check_whether_slice_content_to_fit_in_size(MAX_BLOCK_SIZE - estimated_size)
                {
                    // The new op can take the rest of the room
                    let new = op.slice(0, end);
                    new_change.ops.push(new);
                    new_change = self._insert_splitted_change(
                        new_change,
                        &mut total_len,
                        &mut estimated_size,
                        rollback.as_deref_mut(),
                    );

                    if end < op.atom_len() {
                        op = op.slice(end, op.atom_len());
                    } else {
                        continue 'outer;
                    }
                }

                estimated_size += op.estimate_storage_size();
                if estimated_size > MAX_BLOCK_SIZE && !new_change.ops.is_empty() {
                    new_change = self._insert_splitted_change(
                        new_change,
                        &mut total_len,
                        &mut estimated_size,
                        rollback.as_deref_mut(),
                    );
                    new_change.ops.push(op);
                } else {
                    new_change.ops.push(op);
                }
            }

            if !new_change.ops.is_empty() {
                total_len += new_change.atom_len();
                self.insert_change_inner(new_change, false, false, rollback);
            }

            assert_eq!(total_len, original_len);
        }

        fn _insert_splitted_change(
            &self,
            new_change: Change,
            total_len: &mut usize,
            estimated_size: &mut usize,
            rollback: Option<&mut ChangeStoreRollback>,
        ) -> Change {
            if new_change.atom_len() == 0 {
                return new_change;
            }

            let ctr_end = new_change.id.counter + new_change.atom_len() as Counter;
            let next_lamport = new_change.lamport + new_change.atom_len() as Lamport;
            *total_len += new_change.atom_len();
            let ans = Change {
                ops: RleVec::new(),
                deps: ID::new(new_change.id.peer, ctr_end - 1).into(),
                id: ID::new(new_change.id.peer, ctr_end),
                lamport: next_lamport,
                timestamp: new_change.timestamp,
                commit_msg: new_change.commit_msg.clone(),
            };

            self.insert_change_inner(new_change, false, false, rollback);
            *estimated_size = ans.estimate_storage_size();
            ans
        }

        fn get_parsed_block(&self, id: ID) -> Option<Arc<ChangesBlock>> {
            let mut inner = self.inner.lock();
            if let Some((_id, block)) = inner.mem_parsed_kv.range_mut(..=id).next_back() {
                if block.peer == id.peer && block.counter_range.1 > id.counter {
                    if let Err(err) = block.ensure_changes(&self.arena) {
                        warn!(block_id = ?_id, ?err, "failed to parse cached change block");
                        return None;
                    }
                    return Some(block.clone());
                }
            }

            let store = self.external_kv.lock();
            let mut iter = store
                .scan(Bound::Unbounded, Bound::Included(&id.to_bytes()))
                .filter(|(id, _)| id.len() == 12);

            // println!(
            //     "\nkeys {:?}",
            //     store
            //         .scan(Bound::Unbounded, Bound::Included(&id.to_bytes()))
            //         .filter(|(id, _)| id.len() == 12)
            //         .map(|(k, _v)| ID::from_bytes(&k))
            //         .count()
            // );
            // println!("id {:?}", id);

            let (b_id, b_bytes) = iter.next_back()?;
            let block_id: ID = ID::from_bytes(&b_id[..]);
            let block = match ChangesBlock::from_bytes(b_bytes) {
                Ok(block) => block,
                Err(err) => {
                    warn!(?block_id, ?err, "failed to decode external change block");
                    return None;
                }
            };
            if block_id.peer == id.peer
                && block_id.counter <= id.counter
                && block.counter_range.1 > id.counter
            {
                let mut arc_block = Arc::new(block);
                if let Err(err) = arc_block.ensure_changes(&self.arena) {
                    warn!(?block_id, ?err, "failed to parse external change block");
                    return None;
                }
                inner.mem_parsed_kv.insert(block_id, arc_block.clone());
                return Some(arc_block);
            }

            None
        }

        /// Load all the blocks that have overlapped with the given ID range into `inner_mem_parsed_kv`
        ///
        /// This is fast because we don't actually parse the content.
        // TODO: PERF: This method feels slow.
        pub(super) fn ensure_block_loaded_in_range(&self, start: Bound<ID>, end: Bound<ID>) {
            let mut whether_need_scan_backward = match start {
                Bound::Included(id) => Some(id),
                Bound::Excluded(id) => Some(id.inc(1)),
                Bound::Unbounded => None,
            };

            {
                let start = start.map(|id| id.to_bytes());
                let end = end.map(|id| id.to_bytes());
                let kv = self.external_kv.lock();
                let mut inner = self.inner.lock();
                for (id, bytes) in kv
                    .scan(
                        start.as_ref().map(|x| x.as_slice()),
                        end.as_ref().map(|x| x.as_slice()),
                    )
                    .filter(|(id, _)| id.len() == 12)
                {
                    let id = ID::from_bytes(&id);
                    if let Some(expected_start_id) = whether_need_scan_backward {
                        if id == expected_start_id {
                            whether_need_scan_backward = None;
                        }
                    }

                    if inner.mem_parsed_kv.contains_key(&id) {
                        continue;
                    }

                    let block = match ChangesBlock::from_bytes(bytes.clone()) {
                        Ok(block) => block,
                        Err(err) => {
                            warn!(?id, ?err, "failed to decode external change block");
                            continue;
                        }
                    };
                    inner.mem_parsed_kv.insert(id, Arc::new(block));
                }
            }

            if let Some(start_id) = whether_need_scan_backward {
                self.ensure_id_lte(start_id);
            }
        }

        pub(super) fn ensure_id_lte(&self, id: ID) {
            let kv = self.external_kv.lock();
            let mut inner = self.inner.lock();
            let Some((next_back_id, next_back_bytes)) = kv
                .scan(Bound::Unbounded, Bound::Included(&id.to_bytes()))
                .rfind(|(id, _)| id.len() == 12)
            else {
                return;
            };

            let next_back_id = ID::from_bytes(&next_back_id);
            if next_back_id.peer == id.peer {
                if inner.mem_parsed_kv.contains_key(&next_back_id) {
                    return;
                }

                let block = match ChangesBlock::from_bytes(next_back_bytes) {
                    Ok(block) => block,
                    Err(err) => {
                        warn!(
                            ?next_back_id,
                            ?err,
                            "failed to decode external change block"
                        );
                        return;
                    }
                };
                inner.mem_parsed_kv.insert(next_back_id, Arc::new(block));
            }
        }
    }
}

#[must_use]
#[derive(Clone, Debug)]
pub(crate) struct BatchDecodeInfo {
    pub vv: VersionVector,
    pub frontiers: Frontiers,
    pub start_version: Option<(VersionVector, Frontiers)>,
    pub causal_dag: Option<CausalDagSnapshot>,
    pub import_baseline: Option<ImportBaselineSnapshot>,
    pub greatest_timestamp: Option<i64>,
    pub metadata_digest: Option<[u8; 32]>,
}

#[derive(Clone, Debug)]
pub struct BlockChangeRef {
    block: Arc<ChangesBlock>,
    change_index: usize,
}

impl Deref for BlockChangeRef {
    type Target = Change;
    fn deref(&self) -> &Change {
        &self.block.content.try_changes().unwrap()[self.change_index]
    }
}

impl BlockChangeRef {
    pub(crate) fn get_op_with_counter(&self, counter: Counter) -> Option<BlockOpRef> {
        if counter >= self.ctr_end() {
            return None;
        }

        let index = self.ops.search_atom_index(counter);
        Some(BlockOpRef {
            block: self.block.clone(),
            change_index: self.change_index,
            op_index: index,
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct BlockOpRef {
    pub block: Arc<ChangesBlock>,
    pub change_index: usize,
    pub op_index: usize,
}

impl Deref for BlockOpRef {
    type Target = Op;

    fn deref(&self) -> &Op {
        &self.block.content.try_changes().unwrap()[self.change_index].ops[self.op_index]
    }
}

impl BlockOpRef {
    pub fn lamport(&self) -> Lamport {
        let change = &self.block.content.try_changes().unwrap()[self.change_index];
        let op = &change.ops[self.op_index];
        (op.counter - change.id.counter) as Lamport + change.lamport
    }
}

impl ChangesBlock {
    fn from_bytes(bytes: Bytes) -> LoroResult<Self> {
        let len = bytes.len();
        let bytes = ChangesBlockBytes::new(bytes);
        bytes.ensure_header()?;
        let header = bytes
            .header
            .get()
            .expect("header should be initialized after ensure_header");
        let peer = header.peer;
        let counter_range = (
            header.counter,
            *header.counters.last().ok_or_else(|| {
                LoroError::DecodeError("Decode block error: missing counters".into())
            })?,
        );
        let lamport_range = (
            *header.lamports.first().ok_or_else(|| {
                LoroError::DecodeError("Decode block error: missing lamports".into())
            })?,
            *header.lamports.last().ok_or_else(|| {
                LoroError::DecodeError("Decode block error: missing lamports".into())
            })?,
        );
        let content = ChangesBlockContent::Bytes(bytes);
        Ok(Self {
            peer,
            estimated_size: len,
            counter_range,
            lamport_range,
            flushed: true,
            content,
        })
    }

    #[allow(dead_code)]
    pub(crate) fn content(&self) -> &ChangesBlockContent {
        &self.content
    }

    fn new(change: Change, _a: &SharedArena) -> Self {
        let atom_len = change.atom_len();
        let counter_range = (change.id.counter, change.id.counter + atom_len as Counter);
        let lamport_range = (change.lamport, change.lamport + atom_len as Lamport);
        let estimated_size = change.estimate_storage_size();
        let peer = change.id.peer;
        let content = ChangesBlockContent::Changes(Arc::new(vec![change]));
        Self {
            peer,
            counter_range,
            lamport_range,
            estimated_size,
            content,
            flushed: false,
        }
    }

    #[allow(unused)]
    fn cmp_id(&self, id: ID) -> Ordering {
        self.peer.cmp(&id.peer).then_with(|| {
            if self.counter_range.0 > id.counter {
                Ordering::Greater
            } else if self.counter_range.1 <= id.counter {
                Ordering::Less
            } else {
                Ordering::Equal
            }
        })
    }

    #[allow(unused)]
    fn cmp_idlp(&self, idlp: (PeerID, Lamport)) -> Ordering {
        self.peer.cmp(&idlp.0).then_with(|| {
            if self.lamport_range.0 > idlp.1 {
                Ordering::Greater
            } else if self.lamport_range.1 <= idlp.1 {
                Ordering::Less
            } else {
                Ordering::Equal
            }
        })
    }

    #[allow(unused)]
    fn is_full(&self) -> bool {
        self.estimated_size > MAX_BLOCK_SIZE
    }

    #[allow(clippy::result_large_err)]
    fn push_change(
        self: &mut Arc<Self>,
        change: Change,
        new_change_size: usize,
        merge_interval: i64,
        a: &SharedArena,
    ) -> Result<(), Change> {
        if self.counter_range.1 != change.id.counter {
            return Err(change);
        }

        let atom_len = change.atom_len();
        let next_lamport = change.lamport + atom_len as Lamport;
        let next_counter = change.id.counter + atom_len as Counter;

        let is_full = new_change_size + self.estimated_size > MAX_BLOCK_SIZE;
        let this = Arc::make_mut(self);
        let changes = this.content.changes_mut(a).unwrap();
        let changes = Arc::make_mut(changes);
        match changes.last_mut() {
            Some(last)
                if last.can_merge_right(&change, merge_interval)
                    && (!is_full
                        || (change.ops.len() == 1
                            && last.ops.last().unwrap().is_mergable(&change.ops[0], &()))) =>
            {
                for op in change.ops.into_iter() {
                    let size = op.estimate_storage_size();
                    if !last.ops.push(op) {
                        this.estimated_size += size;
                    }
                }
            }
            _ => {
                if is_full {
                    return Err(change);
                } else {
                    this.estimated_size += new_change_size;
                    changes.push(change);
                }
            }
        }

        this.flushed = false;
        this.counter_range.1 = next_counter;
        this.lamport_range.1 = next_lamport;
        Ok(())
    }

    fn to_bytes(self: &mut Arc<Self>, a: &SharedArena) -> ChangesBlockBytes {
        match &self.content {
            ChangesBlockContent::Bytes(bytes) => bytes.clone(),
            ChangesBlockContent::Both(_, bytes) => {
                let bytes = bytes.clone();
                let this = Arc::make_mut(self);
                this.content = ChangesBlockContent::Bytes(bytes.clone());
                bytes
            }
            ChangesBlockContent::Changes(changes) => {
                let bytes = ChangesBlockBytes::serialize(changes, a);
                let this = Arc::make_mut(self);
                this.content = ChangesBlockContent::Bytes(bytes.clone());
                bytes
            }
        }
    }

    fn ensure_changes(self: &mut Arc<Self>, a: &SharedArena) -> LoroResult<()> {
        match &self.content {
            ChangesBlockContent::Changes(_) => Ok(()),
            ChangesBlockContent::Both(_, _) => Ok(()),
            ChangesBlockContent::Bytes(bytes) => {
                let changes = bytes.parse(a)?;
                let b = bytes.clone();
                let this = Arc::make_mut(self);
                this.content = ChangesBlockContent::Both(Arc::new(changes), b);
                Ok(())
            }
        }
    }

    fn get_change_index_by_counter(&self, counter: Counter) -> Result<usize, usize> {
        let changes = self.content.try_changes().unwrap();
        changes.binary_search_by(|c| {
            if c.id.counter > counter {
                Ordering::Greater
            } else if (c.id.counter + c.content_len() as Counter) <= counter {
                Ordering::Less
            } else {
                Ordering::Equal
            }
        })
    }

    fn get_change_index_by_lamport_lte(&self, lamport: Lamport) -> Option<usize> {
        let changes = self.content.try_changes().unwrap();
        let r = changes.binary_search_by(|c| {
            if c.lamport > lamport {
                Ordering::Greater
            } else if (c.lamport + c.content_len() as Lamport) <= lamport {
                Ordering::Less
            } else {
                Ordering::Equal
            }
        });

        match r {
            Ok(found) => Some(found),
            Err(idx) => {
                if idx == 0 {
                    None
                } else {
                    Some(idx - 1)
                }
            }
        }
    }

    #[allow(unused)]
    fn get_changes(&mut self, a: &SharedArena) -> LoroResult<&Vec<Change>> {
        self.content.changes(a)
    }

    #[allow(unused)]
    fn id(&self) -> ID {
        ID::new(self.peer, self.counter_range.0)
    }

    pub fn change_num(&self) -> usize {
        match &self.content {
            ChangesBlockContent::Changes(c) => c.len(),
            ChangesBlockContent::Bytes(b) => b.len_changes(),
            ChangesBlockContent::Both(c, _) => c.len(),
        }
    }
}

impl ChangesBlockContent {
    // TODO: PERF: We can use Iter to replace Vec
    pub fn iter_dag_nodes(&self) -> Vec<AppDagNode> {
        let mut dag_nodes = Vec::new();
        match self {
            ChangesBlockContent::Changes(c) | ChangesBlockContent::Both(c, _) => {
                for change in c.iter() {
                    let new_node = AppDagNodeInner {
                        peer: change.id.peer,
                        cnt: change.id.counter,
                        lamport: change.lamport,
                        deps: change.deps.clone(),
                        vv: OnceCell::new(),
                        has_succ: false,
                        len: change.atom_len(),
                    }
                    .into();

                    dag_nodes.push_rle_element(new_node);
                }
            }
            ChangesBlockContent::Bytes(b) => {
                b.ensure_header().unwrap();
                let header = b.header.get().unwrap();
                let n = header.n_changes;
                for i in 0..n {
                    let new_node = AppDagNodeInner {
                        peer: header.peer,
                        cnt: header.counters[i],
                        lamport: header.lamports[i],
                        deps: header.deps_groups[i].clone(),
                        vv: OnceCell::new(),
                        has_succ: false,
                        len: (header.counters[i + 1] - header.counters[i]) as usize,
                    }
                    .into();

                    dag_nodes.push_rle_element(new_node);
                }
            }
        }

        dag_nodes
    }

    #[allow(unused)]
    pub fn changes(&mut self, a: &SharedArena) -> LoroResult<&Vec<Change>> {
        match self {
            ChangesBlockContent::Changes(changes) => Ok(changes),
            ChangesBlockContent::Both(changes, _) => Ok(changes),
            ChangesBlockContent::Bytes(bytes) => {
                let changes = bytes.parse(a)?;
                *self = ChangesBlockContent::Both(Arc::new(changes), bytes.clone());
                self.changes(a)
            }
        }
    }

    /// Note that this method will invalidate the stored bytes
    fn changes_mut(&mut self, a: &SharedArena) -> LoroResult<&mut Arc<Vec<Change>>> {
        match self {
            ChangesBlockContent::Changes(changes) => Ok(changes),
            ChangesBlockContent::Both(changes, _) => {
                *self = ChangesBlockContent::Changes(std::mem::take(changes));
                self.changes_mut(a)
            }
            ChangesBlockContent::Bytes(bytes) => {
                let changes = bytes.parse(a)?;
                *self = ChangesBlockContent::Changes(Arc::new(changes));
                self.changes_mut(a)
            }
        }
    }

    pub(crate) fn try_changes(&self) -> Option<&Vec<Change>> {
        match self {
            ChangesBlockContent::Changes(changes) => Some(changes),
            ChangesBlockContent::Both(changes, _) => Some(changes),
            ChangesBlockContent::Bytes(_) => None,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn len_changes(&self) -> usize {
        match self {
            ChangesBlockContent::Changes(changes) => changes.len(),
            ChangesBlockContent::Both(changes, _) => changes.len(),
            ChangesBlockContent::Bytes(bytes) => bytes.len_changes(),
        }
    }
}

impl std::fmt::Debug for ChangesBlockContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChangesBlockContent::Changes(changes) => f
                .debug_tuple("ChangesBlockContent::Changes")
                .field(changes)
                .finish(),
            ChangesBlockContent::Bytes(_bytes) => {
                f.debug_tuple("ChangesBlockContent::Bytes").finish()
            }
            ChangesBlockContent::Both(changes, _bytes) => f
                .debug_tuple("ChangesBlockContent::Both")
                .field(changes)
                .finish(),
        }
    }
}

impl ChangesBlockBytes {
    fn new(bytes: Bytes) -> Self {
        Self {
            header: OnceCell::new(),
            bytes,
        }
    }

    fn ensure_header(&self) -> LoroResult<()> {
        self.header
            .get_or_try_init(|| decode_header(&self.bytes).map(Arc::new))?;
        Ok(())
    }

    fn parse(&self, a: &SharedArena) -> LoroResult<Vec<Change>> {
        self.ensure_header()?;
        let ans: Vec<Change> = decode_block(&self.bytes, a, self.header.get().map(|h| h.as_ref()))?;
        for c in ans.iter() {
            // PERF: This can be made faster (low priority)
            register_container_and_parent_link(a, c)
        }

        Ok(ans)
    }

    fn serialize(changes: &[Change], a: &SharedArena) -> Self {
        let bytes = encode_block(changes, a);
        // TODO: Perf we can calculate header directly without parsing the bytes
        let bytes = ChangesBlockBytes::new(Bytes::from(bytes));
        bytes.ensure_header().unwrap();
        bytes
    }

    fn lamport_range(&mut self) -> LoroResult<(Lamport, Lamport)> {
        if let Some(header) = self.header.get() {
            Ok((header.lamports[0], *header.lamports.last().unwrap()))
        } else {
            decode_block_range(&self.bytes).map(|(_, lamport_range)| lamport_range)
        }
    }

    /// Length of the changes
    fn len_changes(&self) -> usize {
        self.ensure_header().unwrap();
        self.header.get().unwrap().n_changes
    }
}

#[cfg(test)]
mod test {
    use crate::cursor::PosType;
    use crate::{
        loro::ExportMode, oplog::convert_change_to_remote, state::TreeParentId, ListHandler,
        LoroDoc, MovableListHandler, TextHandler, TreeHandler,
    };

    use super::*;

    fn test_encode_decode(doc: LoroDoc) {
        doc.commit_then_renew();
        let oplog = doc.oplog().lock();
        let bytes = oplog
            .change_store
            .encode_all(oplog.vv(), oplog.dag.frontiers());
        let store = ChangeStore::new_for_test();
        let _ = store.import_all(bytes.clone()).unwrap();
        assert_eq!(store.external_kv.lock().export_all(), bytes);
        let mut changes_parsed = Vec::new();
        let a = store.arena.clone();
        store.visit_all_changes(&mut |c| {
            changes_parsed.push(convert_change_to_remote(&a, c));
        });
        let mut changes = Vec::new();
        oplog.change_store.visit_all_changes(&mut |c| {
            changes.push(convert_change_to_remote(&oplog.arena, c));
        });
        assert_eq!(changes_parsed, changes);
    }

    #[test]
    fn test_change_store() {
        let doc = LoroDoc::new_auto_commit();
        doc.set_record_timestamp(true);
        let t = doc.get_text("t");
        t.insert(0, "hello", PosType::Unicode).unwrap();
        doc.commit_then_renew();
        let t = doc.get_list("t");
        t.insert(0, "hello").unwrap();
        test_encode_decode(doc);
    }

    #[test]
    fn test_synced_doc() -> LoroResult<()> {
        let doc_a = LoroDoc::new_auto_commit();
        let doc_b = LoroDoc::new_auto_commit();
        let doc_c = LoroDoc::new_auto_commit();

        {
            // A: Create initial structure
            let map = doc_a.get_map("root");
            map.insert_container("text", TextHandler::new_detached())?;
            map.insert_container("list", ListHandler::new_detached())?;
            map.insert_container("tree", TreeHandler::new_detached())?;
        }

        {
            // Sync initial state to B and C
            let initial_state = doc_a.export(ExportMode::all_updates()).unwrap();
            doc_b.import(&initial_state)?;
            doc_c.import(&initial_state)?;
        }

        {
            // B: Edit text and list
            let map = doc_b.get_map("root");
            let text = map
                .insert_container("text", TextHandler::new_detached())
                .unwrap();
            text.insert(0, "Hello, ", PosType::Unicode)?;

            let list = map
                .insert_container("list", ListHandler::new_detached())
                .unwrap();
            list.push("world")?;
        }

        {
            // C: Edit tree and movable list
            let map = doc_c.get_map("root");
            let tree = map
                .insert_container("tree", TreeHandler::new_detached())
                .unwrap();
            let node_id = tree.create(TreeParentId::Root)?;
            tree.get_meta(node_id)?.insert("key", "value")?;
            let node_b = tree.create(TreeParentId::Root)?;
            tree.move_to(node_b, TreeParentId::Root, 0).unwrap();

            let movable_list = map
                .insert_container("movable", MovableListHandler::new_detached())
                .unwrap();
            movable_list.push("item1".into())?;
            movable_list.push("item2".into())?;
            movable_list.mov(0, 1)?;
        }

        // Sync B's changes to A
        let b_changes = doc_b
            .export(ExportMode::updates(&doc_a.oplog_vv()))
            .unwrap();
        doc_a.import(&b_changes)?;

        // Sync C's changes to A
        let c_changes = doc_c
            .export(ExportMode::updates(&doc_a.oplog_vv()))
            .unwrap();
        doc_a.import(&c_changes)?;

        test_encode_decode(doc_a);
        Ok(())
    }
}
