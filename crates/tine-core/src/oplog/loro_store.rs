use std::collections::BTreeMap;
use std::fmt;
use std::ops::Bound;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use loro::{kv_store_handle, KvStore, KvStoreHandle};
use serde::{Deserialize, Serialize};

use super::scratch_store::{ScratchBlobRef, ScratchPageKind, ScratchPageRef, ScratchStore};
use super::{BatchId, ContentDigest, DocumentCausalDigest, DocumentId, WorkspaceId};

pub(crate) trait TupleFirst {
    type First;
}

impl<A, B> TupleFirst for (A, B) {
    type First = A;
}

pub(crate) type Bytes = <<loro::kv_store::block::BlockIter as Iterator>::Item as TupleFirst>::First;

const LORO_STORE_SCHEMA_VERSION: u32 = 1;
const LORO_EXPORT_SCHEMA_VERSION: u32 = 1;
const MAX_LORO_KEY_BYTES: usize = 1024;
const MAX_LORO_VALUE_BYTES: usize = 256 * 1024 * 1024;
const MAX_LORO_EXPORT_BYTES: usize = 256 * 1024 * 1024;
const LORO_NODE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct LoroStoreStats {
    pub flush_calls: usize,
    pub point_reads: usize,
    pub range_scans: usize,
    pub history_page_reads: usize,
    pub history_blob_reads: usize,
}

#[derive(Debug, Default)]
struct LoroStoreCounters {
    flush_calls: AtomicUsize,
    point_reads: AtomicUsize,
    range_scans: AtomicUsize,
    history_page_reads: AtomicUsize,
    history_blob_reads: AtomicUsize,
}

impl LoroStoreCounters {
    fn snapshot(&self) -> LoroStoreStats {
        LoroStoreStats {
            flush_calls: self.flush_calls.load(Ordering::Relaxed),
            point_reads: self.point_reads.load(Ordering::Relaxed),
            range_scans: self.range_scans.load(Ordering::Relaxed),
            history_page_reads: self.history_page_reads.load(Ordering::Relaxed),
            history_blob_reads: self.history_blob_reads.load(Ordering::Relaxed),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoroHistoryWitness {
    workspace_id: WorkspaceId,
    document_id: DocumentId,
    lane: u8,
    causal_digest: DocumentCausalDigest,
    latest_source_batch: BatchId,
    latest_manifest_fingerprint: ContentDigest,
    latest_update_digest: ContentDigest,
}

impl LoroHistoryWitness {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        workspace_id: WorkspaceId,
        document_id: DocumentId,
        lane: u8,
        causal_digest: DocumentCausalDigest,
        latest_source_batch: BatchId,
        latest_manifest_fingerprint: ContentDigest,
        latest_update_digest: ContentDigest,
    ) -> Result<Self, LoroStoreError> {
        if lane > 1 {
            return Err(LoroStoreError::MalformedWitness);
        }
        Ok(Self {
            workspace_id,
            document_id,
            lane,
            causal_digest,
            latest_source_batch,
            latest_manifest_fingerprint,
            latest_update_digest,
        })
    }

    pub(crate) const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub(crate) const fn document_id(&self) -> DocumentId {
        self.document_id
    }

    pub(crate) const fn lane(&self) -> u8 {
        self.lane
    }

    pub(crate) const fn causal_digest(&self) -> DocumentCausalDigest {
        self.causal_digest
    }

    pub(crate) const fn latest_source_batch(&self) -> BatchId {
        self.latest_source_batch
    }

    pub(crate) const fn latest_manifest_fingerprint(&self) -> ContentDigest {
        self.latest_manifest_fingerprint
    }

    pub(crate) const fn latest_update_digest(&self) -> ContentDigest {
        self.latest_update_digest
    }

    fn validate(&self) -> Result<(), LoroStoreError> {
        if self.lane > 1 {
            return Err(LoroStoreError::MalformedWitness);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LoroNodeRef {
    page_ref: ScratchPageRef,
    height: u16,
    entry_count: u64,
    logical_bytes: u64,
    root_key: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LoroNode {
    schema_version: u32,
    key: Vec<u8>,
    value: Option<ScratchBlobRef>,
    value_len: u64,
    left: Option<LoroNodeRef>,
    right: Option<LoroNodeRef>,
    height: u16,
    entry_count: u64,
    logical_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoroStoreRoot {
    schema_version: u32,
    scratch_binding: ContentDigest,
    index_root: Option<LoroNodeRef>,
    entry_count: u64,
    logical_bytes: u64,
    witness: LoroHistoryWitness,
    authenticator: ContentDigest,
}

impl LoroStoreRoot {
    pub(crate) fn witness(&self) -> &LoroHistoryWitness {
        &self.witness
    }

    #[cfg(test)]
    pub(crate) const fn entry_count(&self) -> u64 {
        self.entry_count
    }

    pub(crate) fn validate_for(
        &self,
        scratch: &ScratchStore,
        expected_witness: &LoroHistoryWitness,
    ) -> Result<(), LoroStoreError> {
        self.validate(scratch, expected_witness)
    }

    fn new(
        scratch_binding: ContentDigest,
        index_root: Option<LoroNodeRef>,
        entry_count: u64,
        logical_bytes: u64,
        witness: LoroHistoryWitness,
    ) -> Result<Self, LoroStoreError> {
        witness.validate()?;
        let authenticator = root_authenticator(
            scratch_binding,
            &index_root,
            entry_count,
            logical_bytes,
            &witness,
        )?;
        Ok(Self {
            schema_version: LORO_STORE_SCHEMA_VERSION,
            scratch_binding,
            index_root,
            entry_count,
            logical_bytes,
            witness,
            authenticator,
        })
    }

    fn validate(
        &self,
        scratch: &ScratchStore,
        expected_witness: &LoroHistoryWitness,
    ) -> Result<(), LoroStoreError> {
        self.witness.validate()?;
        if self.schema_version != LORO_STORE_SCHEMA_VERSION
            || &self.witness != expected_witness
            || self.scratch_binding != scratch.binding_digest()?
            || self.authenticator
                != root_authenticator(
                    self.scratch_binding,
                    &self.index_root,
                    self.entry_count,
                    self.logical_bytes,
                    &self.witness,
                )?
        {
            return Err(LoroStoreError::MisboundRoot);
        }
        if self.entry_count != node_count(self.index_root.as_ref())
            || self.logical_bytes != node_bytes(self.index_root.as_ref())
        {
            return Err(LoroStoreError::MisboundRoot);
        }
        Ok(())
    }
}

#[derive(Debug)]
struct OverlayLayer {
    parent: Option<Arc<OverlayLayer>>,
    changes: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
}

#[derive(Debug)]
struct StoreState {
    index_root: Option<LoroNodeRef>,
    entry_count: u64,
    logical_bytes: u64,
    shared_overlay: Option<Arc<OverlayLayer>>,
    overlay: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
}

#[derive(Clone, Debug)]
enum OwnedBound {
    Included(Vec<u8>),
    Excluded(Vec<u8>),
    Unbounded,
}

impl OwnedBound {
    fn from_borrowed(bound: Bound<&[u8]>) -> Self {
        match bound {
            Bound::Included(key) => Self::Included(key.to_vec()),
            Bound::Excluded(key) => Self::Excluded(key.to_vec()),
            Bound::Unbounded => Self::Unbounded,
        }
    }

    fn as_borrowed(&self) -> Bound<&[u8]> {
        match self {
            Self::Included(key) => Bound::Included(key),
            Self::Excluded(key) => Bound::Excluded(key),
            Self::Unbounded => Bound::Unbounded,
        }
    }
}

struct TreeRangeIterator<'a> {
    store: &'a AuthenticatedLoroStore,
    root: Option<LoroNodeRef>,
    front: OwnedBound,
    back: OwnedBound,
    failed: bool,
}

impl fmt::Debug for TreeRangeIterator<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TreeRangeIterator")
            .field("front", &self.front)
            .field("back", &self.back)
            .field("failed", &self.failed)
            .finish_non_exhaustive()
    }
}

impl Iterator for TreeRangeIterator<'_> {
    type Item = (Bytes, Bytes);

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        match self.store.tree_edge(
            self.root.as_ref(),
            &self.front.as_borrowed(),
            &self.back.as_borrowed(),
            true,
        ) {
            Ok(Some((key, value))) => {
                self.front = OwnedBound::Excluded(key.clone());
                Some((Bytes::from(key), Bytes::from(value)))
            }
            Ok(None) => None,
            Err(error) => {
                self.store.record_error(error);
                self.failed = true;
                None
            }
        }
    }
}

impl DoubleEndedIterator for TreeRangeIterator<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        match self.store.tree_edge(
            self.root.as_ref(),
            &self.front.as_borrowed(),
            &self.back.as_borrowed(),
            false,
        ) {
            Ok(Some((key, value))) => {
                self.back = OwnedBound::Excluded(key.clone());
                Some((Bytes::from(key), Bytes::from(value)))
            }
            Ok(None) => None,
            Err(error) => {
                self.store.record_error(error);
                self.failed = true;
                None
            }
        }
    }
}

/// A run-local authenticated Loro change store.
///
/// `Clone` is a control handle for the same adapter. Loro's `clone_store`
/// operation creates an independent copy-on-write adapter instead.
#[derive(Clone)]
pub(crate) struct AuthenticatedLoroStore {
    scratch: Arc<ScratchStore>,
    state: Arc<Mutex<StoreState>>,
    sticky_error: Arc<Mutex<Option<String>>>,
    counters: Arc<LoroStoreCounters>,
}

impl fmt::Debug for AuthenticatedLoroStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        f.debug_struct("AuthenticatedLoroStore")
            .field("entry_count", &state.entry_count)
            .field("logical_bytes", &state.logical_bytes)
            .field("overlay_entries", &state.overlay.len())
            .finish_non_exhaustive()
    }
}

impl AuthenticatedLoroStore {
    pub(crate) fn empty(scratch: Arc<ScratchStore>) -> Self {
        Self {
            scratch,
            state: Arc::new(Mutex::new(StoreState {
                index_root: None,
                entry_count: 0,
                logical_bytes: 0,
                shared_overlay: None,
                overlay: BTreeMap::new(),
            })),
            sticky_error: Arc::new(Mutex::new(None)),
            counters: Arc::new(LoroStoreCounters::default()),
        }
    }

    pub(crate) fn reopen(
        scratch: Arc<ScratchStore>,
        root: &LoroStoreRoot,
        expected_witness: &LoroHistoryWitness,
    ) -> Result<Self, LoroStoreError> {
        root.validate(&scratch, expected_witness)?;
        Ok(Self {
            scratch,
            state: Arc::new(Mutex::new(StoreState {
                index_root: root.index_root.clone(),
                entry_count: root.entry_count,
                logical_bytes: root.logical_bytes,
                shared_overlay: None,
                overlay: BTreeMap::new(),
            })),
            sticky_error: Arc::new(Mutex::new(None)),
            counters: Arc::new(LoroStoreCounters::default()),
        })
    }

    pub(crate) fn handle(&self) -> KvStoreHandle {
        kv_store_handle(self.clone())
    }

    pub(crate) fn stats(&self) -> LoroStoreStats {
        self.counters.snapshot()
    }

    #[cfg(test)]
    pub(crate) fn poison_for_test(&self, message: &str) {
        self.record_error(message);
    }

    pub(crate) fn publish_root(
        &self,
        witness: LoroHistoryWitness,
    ) -> Result<LoroStoreRoot, LoroStoreError> {
        let result = witness
            .validate()
            .and_then(|_| self.check_sticky())
            .and_then(|_| self.flush_inner())
            .and_then(|_| self.check_sticky())
            .and_then(|_| {
                let state = self.lock_state()?;
                LoroStoreRoot::new(
                    self.scratch.binding_digest()?,
                    state.index_root.clone(),
                    state.entry_count,
                    state.logical_bytes,
                    witness,
                )
            });
        if let Err(error) = &result {
            self.record_error(error);
        }
        result
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, StoreState>, LoroStoreError> {
        self.state.lock().map_err(|_| LoroStoreError::Poisoned)
    }

    fn check_sticky(&self) -> Result<(), LoroStoreError> {
        let error = self
            .sticky_error
            .lock()
            .map_err(|_| LoroStoreError::Poisoned)?;
        match error.as_ref() {
            Some(error) => Err(LoroStoreError::Sticky(error.clone())),
            None => Ok(()),
        }
    }

    fn record_error(&self, error: impl ToString) {
        let mut sticky = self
            .sticky_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if sticky.is_none() {
            *sticky = Some(error.to_string());
        }
    }

    fn read_node(&self, node_ref: &LoroNodeRef) -> Result<LoroNode, LoroStoreError> {
        self.counters
            .history_page_reads
            .fetch_add(1, Ordering::Relaxed);
        let node: LoroNode = self
            .scratch
            .read_page(&node_ref.page_ref, ScratchPageKind::LoroHistory)?;
        validate_key(&node.key)?;
        let expected_height = 1_u16
            .checked_add(node_height(node.left.as_ref()).max(node_height(node.right.as_ref())))
            .ok_or(LoroStoreError::Bounds)?;
        let expected_count = 1_u64
            .checked_add(node_count(node.left.as_ref()))
            .and_then(|count| count.checked_add(node_count(node.right.as_ref())))
            .ok_or(LoroStoreError::Bounds)?;
        let expected_bytes = (node.key.len() as u64)
            .checked_add(node.value_len)
            .and_then(|bytes| bytes.checked_add(node_bytes(node.left.as_ref())))
            .and_then(|bytes| bytes.checked_add(node_bytes(node.right.as_ref())))
            .ok_or(LoroStoreError::Bounds)?;
        let expected_min = node
            .left
            .as_ref()
            .map_or(node.key.as_slice(), |left| left.page_ref.key_min());
        let expected_max = node
            .right
            .as_ref()
            .map_or(node.key.as_slice(), |right| right.page_ref.key_max());
        if node.schema_version != LORO_NODE_SCHEMA_VERSION
            || node.value_len > MAX_LORO_VALUE_BYTES as u64
            || (node.value_len == 0) != node.value.is_none()
            || node.height != expected_height
            || node.entry_count != expected_count
            || node.logical_bytes != expected_bytes
            || node_ref.height != node.height
            || node_ref.entry_count != node.entry_count
            || node_ref.logical_bytes != node.logical_bytes
            || node_ref.root_key != node.key
            || node_ref.page_ref.key_min() != expected_min
            || node_ref.page_ref.key_max() != expected_max
            || node
                .left
                .as_ref()
                .is_some_and(|left| left.page_ref.key_max() >= node.key.as_slice())
            || node
                .right
                .as_ref()
                .is_some_and(|right| right.page_ref.key_min() <= node.key.as_slice())
        {
            return Err(LoroStoreError::MisboundRoot);
        }
        Ok(node)
    }

    fn read_node_value(&self, node: &LoroNode) -> Result<Vec<u8>, LoroStoreError> {
        let Some(value_ref) = &node.value else {
            return Ok(Vec::new());
        };
        self.counters
            .history_blob_reads
            .fetch_add(1, Ordering::Relaxed);
        let value = self.scratch.read_blob(value_ref)?;
        if value.len() as u64 != node.value_len {
            return Err(LoroStoreError::MisboundRoot);
        }
        validate_value(&value)?;
        Ok(value)
    }

    fn write_node(&self, mut node: LoroNode) -> Result<LoroNodeRef, LoroStoreError> {
        validate_key(&node.key)?;
        if node.value_len > MAX_LORO_VALUE_BYTES as u64
            || (node.value_len == 0) != node.value.is_none()
        {
            return Err(LoroStoreError::Bounds);
        }
        node.schema_version = LORO_NODE_SCHEMA_VERSION;
        node.height = 1_u16
            .checked_add(node_height(node.left.as_ref()).max(node_height(node.right.as_ref())))
            .ok_or(LoroStoreError::Bounds)?;
        node.entry_count = 1_u64
            .checked_add(node_count(node.left.as_ref()))
            .and_then(|count| count.checked_add(node_count(node.right.as_ref())))
            .ok_or(LoroStoreError::Bounds)?;
        node.logical_bytes = (node.key.len() as u64)
            .checked_add(node.value_len)
            .and_then(|bytes| bytes.checked_add(node_bytes(node.left.as_ref())))
            .and_then(|bytes| bytes.checked_add(node_bytes(node.right.as_ref())))
            .ok_or(LoroStoreError::Bounds)?;
        let key_min = node
            .left
            .as_ref()
            .map_or_else(|| node.key.clone(), |left| left.page_ref.key_min().to_vec());
        let key_max = node.right.as_ref().map_or_else(
            || node.key.clone(),
            |right| right.page_ref.key_max().to_vec(),
        );
        let page_ref =
            self.scratch
                .append_page(ScratchPageKind::LoroHistory, key_min, key_max, &node)?;
        Ok(LoroNodeRef {
            page_ref,
            height: node.height,
            entry_count: node.entry_count,
            logical_bytes: node.logical_bytes,
            root_key: node.key,
        })
    }

    fn stored_value(&self, value: &[u8]) -> Result<(Option<ScratchBlobRef>, u64), LoroStoreError> {
        validate_value(value)?;
        if value.is_empty() {
            Ok((None, 0))
        } else {
            Ok((Some(self.scratch.append_blob(value)?), value.len() as u64))
        }
    }

    fn build_balanced_tree(
        &self,
        sorted_entries: &[(Vec<u8>, Vec<u8>)],
    ) -> Result<Option<LoroNodeRef>, LoroStoreError> {
        if sorted_entries.is_empty() {
            return Ok(None);
        }
        let middle = sorted_entries.len() / 2;
        let (left_entries, right_with_root) = sorted_entries.split_at(middle);
        let (root_entry, right_entries) = right_with_root
            .split_first()
            .ok_or(LoroStoreError::Bounds)?;

        let left = self.build_balanced_tree(left_entries)?;
        let right = self.build_balanced_tree(right_entries)?;
        let (value, value_len) = self.stored_value(&root_entry.1)?;
        self.write_node(LoroNode {
            schema_version: LORO_NODE_SCHEMA_VERSION,
            key: root_entry.0.clone(),
            value,
            value_len,
            left,
            right,
            height: 1,
            entry_count: 1,
            logical_bytes: 0,
        })
        .map(Some)
    }

    fn insert_tree(
        &self,
        root: Option<LoroNodeRef>,
        key: Vec<u8>,
        value: Option<ScratchBlobRef>,
        value_len: u64,
    ) -> Result<LoroNodeRef, LoroStoreError> {
        let Some(root_ref) = root else {
            return self.write_node(LoroNode {
                schema_version: LORO_NODE_SCHEMA_VERSION,
                key,
                value,
                value_len,
                left: None,
                right: None,
                height: 1,
                entry_count: 1,
                logical_bytes: 0,
            });
        };
        let mut node = self.read_node(&root_ref)?;
        match key.cmp(&node.key) {
            std::cmp::Ordering::Less => {
                node.left = Some(self.insert_tree(node.left.take(), key, value, value_len)?);
            }
            std::cmp::Ordering::Greater => {
                node.right = Some(self.insert_tree(node.right.take(), key, value, value_len)?);
            }
            std::cmp::Ordering::Equal => {
                node.value = value;
                node.value_len = value_len;
                return self.write_node(node);
            }
        }
        self.rebalance(node)
    }

    fn remove_tree(
        &self,
        root: Option<LoroNodeRef>,
        key: &[u8],
    ) -> Result<Option<LoroNodeRef>, LoroStoreError> {
        let Some(root_ref) = root else {
            return Ok(None);
        };
        let mut node = self.read_node(&root_ref)?;
        match key.cmp(&node.key) {
            std::cmp::Ordering::Less => {
                node.left = self.remove_tree(node.left.take(), key)?;
            }
            std::cmp::Ordering::Greater => {
                node.right = self.remove_tree(node.right.take(), key)?;
            }
            std::cmp::Ordering::Equal => match (node.left.take(), node.right.take()) {
                (None, None) => return Ok(None),
                (Some(child), None) | (None, Some(child)) => return Ok(Some(child)),
                (Some(left), Some(right)) => {
                    let successor = self.minimum_node(&right)?;
                    node.key = successor.key.clone();
                    node.value = successor.value.clone();
                    node.value_len = successor.value_len;
                    node.left = Some(left);
                    node.right = self.remove_tree(Some(right), &successor.key)?;
                }
            },
        }
        Ok(Some(self.rebalance(node)?))
    }

    fn minimum_node(&self, root: &LoroNodeRef) -> Result<LoroNode, LoroStoreError> {
        let mut node = self.read_node(root)?;
        while let Some(left) = node.left.as_ref() {
            node = self.read_node(left)?;
        }
        Ok(node)
    }

    fn rebalance(&self, mut node: LoroNode) -> Result<LoroNodeRef, LoroStoreError> {
        let balance =
            node_height(node.left.as_ref()) as i32 - node_height(node.right.as_ref()) as i32;
        if balance > 1 {
            let left_ref = node.left.take().ok_or(LoroStoreError::MisboundRoot)?;
            let left = self.read_node(&left_ref)?;
            if node_height(left.left.as_ref()) < node_height(left.right.as_ref()) {
                node.left = Some(self.rotate_left(left)?);
            } else {
                node.left = Some(left_ref);
            }
            return self.rotate_right(node);
        }
        if balance < -1 {
            let right_ref = node.right.take().ok_or(LoroStoreError::MisboundRoot)?;
            let right = self.read_node(&right_ref)?;
            if node_height(right.right.as_ref()) < node_height(right.left.as_ref()) {
                node.right = Some(self.rotate_right(right)?);
            } else {
                node.right = Some(right_ref);
            }
            return self.rotate_left(node);
        }
        self.write_node(node)
    }

    fn rotate_left(&self, mut node: LoroNode) -> Result<LoroNodeRef, LoroStoreError> {
        let right_ref = node.right.take().ok_or(LoroStoreError::MisboundRoot)?;
        let mut right = self.read_node(&right_ref)?;
        node.right = right.left.take();
        right.left = Some(self.write_node(node)?);
        self.write_node(right)
    }

    fn rotate_right(&self, mut node: LoroNode) -> Result<LoroNodeRef, LoroStoreError> {
        let left_ref = node.left.take().ok_or(LoroStoreError::MisboundRoot)?;
        let mut left = self.read_node(&left_ref)?;
        node.left = left.right.take();
        left.right = Some(self.write_node(node)?);
        self.write_node(left)
    }

    fn lookup_tree(
        &self,
        root: Option<&LoroNodeRef>,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, LoroStoreError> {
        let mut current = root.cloned();
        while let Some(node_ref) = current {
            let node = self.read_node(&node_ref)?;
            match key.cmp(&node.key) {
                std::cmp::Ordering::Less => current = node.left,
                std::cmp::Ordering::Greater => current = node.right,
                std::cmp::Ordering::Equal => return self.read_node_value(&node).map(Some),
            }
        }
        Ok(None)
    }

    fn scan_tree(
        &self,
        root: Option<&LoroNodeRef>,
        start: &Bound<&[u8]>,
        end: &Bound<&[u8]>,
        rows: &mut BTreeMap<Vec<u8>, Vec<u8>>,
    ) -> Result<(), LoroStoreError> {
        let Some(node_ref) = root else {
            return Ok(());
        };
        if range_excludes(
            start,
            end,
            node_ref.page_ref.key_min(),
            node_ref.page_ref.key_max(),
        ) {
            return Ok(());
        }
        let node = self.read_node(node_ref)?;
        self.scan_tree(node.left.as_ref(), start, end, rows)?;
        if within_bounds(&node.key, start, end) {
            rows.insert(node.key.clone(), self.read_node_value(&node)?);
        }
        self.scan_tree(node.right.as_ref(), start, end, rows)
    }

    fn tree_edge(
        &self,
        root: Option<&LoroNodeRef>,
        start: &Bound<&[u8]>,
        end: &Bound<&[u8]>,
        forward: bool,
    ) -> Result<Option<(Vec<u8>, Vec<u8>)>, LoroStoreError> {
        let mut current = root.cloned();
        let mut candidate = None;
        while let Some(node_ref) = current {
            let node = self.read_node(&node_ref)?;
            if !after_start(&node.key, start) {
                current = node.right;
                continue;
            }
            if !before_end(&node.key, end) {
                current = node.left;
                continue;
            }
            current = if forward {
                node.left.clone()
            } else {
                node.right.clone()
            };
            candidate = Some(node);
        }
        candidate
            .map(|node| {
                let value = self.read_node_value(&node)?;
                Ok((node.key, value))
            })
            .transpose()
    }

    fn lookup_locked(
        &self,
        state: &StoreState,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, LoroStoreError> {
        if let Some(value) = state.overlay.get(key) {
            return Ok(value.clone());
        }
        let mut layer = state.shared_overlay.as_ref().map(Arc::clone);
        while let Some(current) = layer {
            if let Some(value) = current.changes.get(key) {
                return Ok(value.clone());
            }
            layer = current.parent.as_ref().map(Arc::clone);
        }
        self.lookup_tree(state.index_root.as_ref(), key)
    }

    fn set_locked(
        &self,
        state: &mut StoreState,
        key: &[u8],
        value: Vec<u8>,
    ) -> Result<(), LoroStoreError> {
        let old = self.lookup_locked(state, key)?;
        adjust_metrics(state, key, old.as_deref(), Some(&value))?;
        state.overlay.insert(key.to_vec(), Some(value));
        Ok(())
    }

    fn remove_locked(
        &self,
        state: &mut StoreState,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, LoroStoreError> {
        let old = self.lookup_locked(state, key)?;
        if old.is_some() {
            adjust_metrics(state, key, old.as_deref(), None)?;
            state.overlay.insert(key.to_vec(), None);
        }
        Ok(old)
    }

    fn seal_overlay(state: &mut StoreState) {
        if state.overlay.is_empty() {
            return;
        }
        let changes = std::mem::take(&mut state.overlay);
        state.shared_overlay = Some(Arc::new(OverlayLayer {
            parent: state.shared_overlay.take(),
            changes,
        }));
    }

    fn collect_overlay(state: &StoreState) -> BTreeMap<Vec<u8>, Option<Vec<u8>>> {
        let mut layers = Vec::new();
        let mut layer = state.shared_overlay.as_ref().map(Arc::clone);
        while let Some(current) = layer {
            layer = current.parent.as_ref().map(Arc::clone);
            layers.push(current);
        }
        let mut changes = BTreeMap::new();
        for layer in layers.into_iter().rev() {
            for (key, value) in &layer.changes {
                changes.insert(key.clone(), value.clone());
            }
        }
        for (key, value) in &state.overlay {
            changes.insert(key.clone(), value.clone());
        }
        changes
    }

    fn flush_inner(&self) -> Result<(), LoroStoreError> {
        self.check_sticky()?;
        let mut state = self.lock_state()?;
        let changes = Self::collect_overlay(&state);
        if changes.is_empty() {
            return Ok(());
        }
        let next_root = if state.index_root.is_none() {
            let entries = changes
                .into_iter()
                .filter_map(|(key, value)| value.map(|value| (key, value)))
                .collect::<Vec<_>>();
            self.build_balanced_tree(&entries)?
        } else {
            let mut next_root = state.index_root.clone();
            for (key, value) in changes {
                next_root = match value {
                    Some(value) => {
                        let (value_ref, value_len) = self.stored_value(&value)?;
                        Some(self.insert_tree(next_root, key, value_ref, value_len)?)
                    }
                    None => self.remove_tree(next_root, &key)?,
                };
            }
            next_root
        };
        if node_count(next_root.as_ref()) != state.entry_count
            || node_bytes(next_root.as_ref()) != state.logical_bytes
        {
            return Err(LoroStoreError::MisboundRoot);
        }
        state.index_root = next_root;
        state.shared_overlay = None;
        state.overlay.clear();
        Ok(())
    }

    fn fork_independent(&self) -> Result<Self, LoroStoreError> {
        let mut state = self.lock_state()?;
        Self::seal_overlay(&mut state);
        let child_state = StoreState {
            index_root: state.index_root.clone(),
            entry_count: state.entry_count,
            logical_bytes: state.logical_bytes,
            shared_overlay: state.shared_overlay.as_ref().map(Arc::clone),
            overlay: BTreeMap::new(),
        };
        let inherited_error = self
            .sticky_error
            .lock()
            .map_err(|_| LoroStoreError::Poisoned)?
            .clone();
        Ok(Self {
            scratch: Arc::clone(&self.scratch),
            state: Arc::new(Mutex::new(child_state)),
            sticky_error: Arc::new(Mutex::new(inherited_error)),
            counters: Arc::new(LoroStoreCounters::default()),
        })
    }

    fn scan_inner(
        &self,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
    ) -> Result<Vec<(Bytes, Bytes)>, LoroStoreError> {
        validate_bound(&start)?;
        validate_bound(&end)?;
        let state = self.lock_state()?;
        let mut rows = BTreeMap::new();
        self.scan_tree(state.index_root.as_ref(), &start, &end, &mut rows)?;
        let mut layers = Vec::new();
        let mut layer = state.shared_overlay.as_ref().map(Arc::clone);
        while let Some(current) = layer {
            layer = current.parent.as_ref().map(Arc::clone);
            layers.push(current);
        }
        for layer in layers.into_iter().rev() {
            merge_range(&mut rows, &layer.changes, &start, &end);
        }
        merge_range(&mut rows, &state.overlay, &start, &end);
        Ok(rows
            .into_iter()
            .map(|(key, value)| (Bytes::from(key), Bytes::from(value)))
            .collect())
    }
}

impl KvStore for AuthenticatedLoroStore {
    fn get(&self, key: &[u8]) -> Option<Bytes> {
        self.counters.point_reads.fetch_add(1, Ordering::Relaxed);
        if self.check_sticky().is_err() {
            return None;
        }
        let result = validate_key(key).and_then(|_| {
            let state = self.lock_state()?;
            self.lookup_locked(&state, key)
        });
        match result {
            Ok(value) => value.map(Bytes::from),
            Err(error) => {
                self.record_error(error);
                None
            }
        }
    }

    fn set(&mut self, key: &[u8], value: Bytes) {
        if self.check_sticky().is_err() {
            return;
        }
        let result = validate_key(key)
            .and_then(|_| validate_value(&value))
            .and_then(|_| {
                let mut state = self.lock_state()?;
                self.set_locked(&mut state, key, value.to_vec())
            });
        if let Err(error) = result {
            self.record_error(error);
        }
    }

    fn compare_and_swap(&mut self, key: &[u8], old: Option<Bytes>, new: Bytes) -> bool {
        if self.check_sticky().is_err() {
            return false;
        }
        let result = validate_key(key)
            .and_then(|_| validate_value(&new))
            .and_then(|_| {
                let mut state = self.lock_state()?;
                let current = self.lookup_locked(&state, key)?;
                if current.as_deref() != old.as_deref() {
                    return Ok(false);
                }
                self.set_locked(&mut state, key, new.to_vec())?;
                Ok(true)
            });
        match result {
            Ok(swapped) => swapped,
            Err(error) => {
                self.record_error(error);
                false
            }
        }
    }

    fn remove(&mut self, key: &[u8]) -> Option<Bytes> {
        if self.check_sticky().is_err() {
            return None;
        }
        let result = validate_key(key).and_then(|_| {
            let mut state = self.lock_state()?;
            self.remove_locked(&mut state, key)
        });
        match result {
            Ok(value) => value.map(Bytes::from),
            Err(error) => {
                self.record_error(error);
                None
            }
        }
    }

    fn contains_key(&self, key: &[u8]) -> bool {
        self.get(key).is_some()
    }

    fn scan(
        &self,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
    ) -> Box<dyn DoubleEndedIterator<Item = (Bytes, Bytes)> + '_> {
        self.counters.range_scans.fetch_add(1, Ordering::Relaxed);
        if self.check_sticky().is_err() {
            return Box::new(Vec::new().into_iter());
        }
        if let Err(error) = validate_bound(&start).and_then(|_| validate_bound(&end)) {
            self.record_error(error);
            return Box::new(Vec::new().into_iter());
        }
        let clean_root = match self.lock_state() {
            Ok(state) if state.shared_overlay.is_none() && state.overlay.is_empty() => {
                Some(state.index_root.clone())
            }
            Ok(_) => None,
            Err(error) => {
                self.record_error(error);
                return Box::new(Vec::new().into_iter());
            }
        };
        if let Some(root) = clean_root {
            return Box::new(TreeRangeIterator {
                store: self,
                root,
                front: OwnedBound::from_borrowed(start),
                back: OwnedBound::from_borrowed(end),
                failed: false,
            });
        }
        match self.scan_inner(start, end) {
            Ok(rows) => Box::new(rows.into_iter()),
            Err(error) => {
                self.record_error(error);
                Box::new(Vec::new().into_iter())
            }
        }
    }

    fn len(&self) -> usize {
        if self.check_sticky().is_err() {
            return 0;
        }
        match self.lock_state().and_then(|state| {
            usize::try_from(state.entry_count).map_err(|_| LoroStoreError::Bounds)
        }) {
            Ok(length) => length,
            Err(error) => {
                self.record_error(error);
                0
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn size(&self) -> usize {
        if self.check_sticky().is_err() {
            return 0;
        }
        match self.lock_state().and_then(|state| {
            usize::try_from(state.logical_bytes).map_err(|_| LoroStoreError::Bounds)
        }) {
            Ok(size) => size,
            Err(error) => {
                self.record_error(error);
                0
            }
        }
    }

    fn export_all(&mut self) -> Bytes {
        if self.check_sticky().is_err() {
            return Bytes::new();
        }
        let result = self
            .scan_inner(Bound::Unbounded, Bound::Unbounded)
            .and_then(|rows| {
                let export = LoroStoreExport {
                    schema_version: LORO_EXPORT_SCHEMA_VERSION,
                    rows: rows
                        .into_iter()
                        .map(|(key, value)| (key.to_vec(), value.to_vec()))
                        .collect(),
                };
                let encoded =
                    postcard::to_allocvec(&export).map_err(|_| LoroStoreError::MalformedExport)?;
                if encoded.len() > MAX_LORO_EXPORT_BYTES {
                    return Err(LoroStoreError::Bounds);
                }
                Ok(Bytes::from(encoded))
            });
        match result {
            Ok(bytes) => bytes,
            Err(error) => {
                self.record_error(error);
                Bytes::new()
            }
        }
    }

    fn import_all(&mut self, bytes: Bytes) -> Result<(), String> {
        if let Err(error) = self.check_sticky() {
            return Err(error.to_string());
        }
        let result = decode_export(&bytes).and_then(|rows| {
            let mut state = self.lock_state()?;
            for (key, value) in rows {
                self.set_locked(&mut state, &key, value)?;
            }
            Ok(())
        });
        match result {
            Ok(()) => Ok(()),
            Err(error) => {
                self.record_error(&error);
                Err(error.to_string())
            }
        }
    }

    fn clone_store(&self) -> KvStoreHandle {
        match self.fork_independent() {
            Ok(store) => kv_store_handle(store),
            Err(error) => {
                self.record_error(&error);
                let child = Self::empty(Arc::clone(&self.scratch));
                child.record_error(error);
                kv_store_handle(child)
            }
        }
    }

    fn flush(&mut self) {
        self.counters.flush_calls.fetch_add(1, Ordering::Relaxed);
        if let Err(error) = self.flush_inner() {
            self.record_error(error);
        }
    }

    fn take_error(&mut self) -> Option<String> {
        self.sticky_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LoroStoreExport {
    schema_version: u32,
    rows: Vec<(Vec<u8>, Vec<u8>)>,
}

fn decode_export(bytes: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>, LoroStoreError> {
    if bytes.len() > MAX_LORO_EXPORT_BYTES {
        return Err(LoroStoreError::Bounds);
    }
    let export: LoroStoreExport =
        postcard::from_bytes(bytes).map_err(|_| LoroStoreError::MalformedExport)?;
    if export.schema_version != LORO_EXPORT_SCHEMA_VERSION
        || postcard::to_allocvec(&export).map_err(|_| LoroStoreError::MalformedExport)? != bytes
    {
        return Err(LoroStoreError::MalformedExport);
    }
    let mut previous: Option<&[u8]> = None;
    for (key, value) in &export.rows {
        validate_key(key)?;
        validate_value(value)?;
        if previous.is_some_and(|previous| previous >= key.as_slice()) {
            return Err(LoroStoreError::MalformedExport);
        }
        previous = Some(key);
    }
    Ok(export.rows)
}

fn root_authenticator(
    scratch_binding: ContentDigest,
    index_root: &Option<LoroNodeRef>,
    entry_count: u64,
    logical_bytes: u64,
    witness: &LoroHistoryWitness,
) -> Result<ContentDigest, LoroStoreError> {
    let bytes = postcard::to_allocvec(&(
        LORO_STORE_SCHEMA_VERSION,
        scratch_binding,
        index_root,
        entry_count,
        logical_bytes,
        witness,
    ))
    .map_err(|_| LoroStoreError::MalformedRoot)?;
    Ok(ContentDigest::of(&bytes))
}

fn validate_key(key: &[u8]) -> Result<(), LoroStoreError> {
    if key.is_empty() || key.len() > MAX_LORO_KEY_BYTES {
        return Err(LoroStoreError::Bounds);
    }
    Ok(())
}

fn validate_value(value: &[u8]) -> Result<(), LoroStoreError> {
    if value.len() > MAX_LORO_VALUE_BYTES {
        return Err(LoroStoreError::Bounds);
    }
    Ok(())
}

fn validate_bound(bound: &Bound<&[u8]>) -> Result<(), LoroStoreError> {
    match bound {
        Bound::Included(key) | Bound::Excluded(key) => validate_key(key),
        Bound::Unbounded => Ok(()),
    }
}

fn node_height(node: Option<&LoroNodeRef>) -> u16 {
    node.map_or(0, |node| node.height)
}

fn node_count(node: Option<&LoroNodeRef>) -> u64 {
    node.map_or(0, |node| node.entry_count)
}

fn node_bytes(node: Option<&LoroNodeRef>) -> u64 {
    node.map_or(0, |node| node.logical_bytes)
}

fn range_excludes(
    start: &Bound<&[u8]>,
    end: &Bound<&[u8]>,
    key_min: &[u8],
    key_max: &[u8],
) -> bool {
    let before_start = match start {
        Bound::Included(start) => key_max < *start,
        Bound::Excluded(start) => key_max <= *start,
        Bound::Unbounded => false,
    };
    let after_end = match end {
        Bound::Included(end) => key_min > *end,
        Bound::Excluded(end) => key_min >= *end,
        Bound::Unbounded => false,
    };
    before_start || after_end
}

fn within_bounds(key: &[u8], start: &Bound<&[u8]>, end: &Bound<&[u8]>) -> bool {
    after_start(key, start) && before_end(key, end)
}

fn after_start(key: &[u8], start: &Bound<&[u8]>) -> bool {
    match start {
        Bound::Included(start) => key >= *start,
        Bound::Excluded(start) => key > *start,
        Bound::Unbounded => true,
    }
}

fn before_end(key: &[u8], end: &Bound<&[u8]>) -> bool {
    match end {
        Bound::Included(end) => key <= *end,
        Bound::Excluded(end) => key < *end,
        Bound::Unbounded => true,
    }
}

fn merge_range(
    rows: &mut BTreeMap<Vec<u8>, Vec<u8>>,
    changes: &BTreeMap<Vec<u8>, Option<Vec<u8>>>,
    start: &Bound<&[u8]>,
    end: &Bound<&[u8]>,
) {
    for (key, value) in changes {
        if !within_bounds(key, start, end) {
            continue;
        }
        match value {
            Some(value) => {
                rows.insert(key.clone(), value.clone());
            }
            None => {
                rows.remove(key);
            }
        }
    }
}

fn adjust_metrics(
    state: &mut StoreState,
    key: &[u8],
    old: Option<&[u8]>,
    new: Option<&[u8]>,
) -> Result<(), LoroStoreError> {
    let old_bytes = old.map_or(0_u64, |value| (key.len() + value.len()) as u64);
    let new_bytes = new.map_or(0_u64, |value| (key.len() + value.len()) as u64);
    state.logical_bytes = state
        .logical_bytes
        .checked_sub(old_bytes)
        .and_then(|bytes| bytes.checked_add(new_bytes))
        .ok_or(LoroStoreError::Bounds)?;
    state.entry_count = match (old.is_some(), new.is_some()) {
        (false, true) => state
            .entry_count
            .checked_add(1)
            .ok_or(LoroStoreError::Bounds)?,
        (true, false) => state
            .entry_count
            .checked_sub(1)
            .ok_or(LoroStoreError::Bounds)?,
        _ => state.entry_count,
    };
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LoroStoreError {
    Scratch(String),
    Sticky(String),
    MisboundRoot,
    MalformedRoot,
    MalformedWitness,
    MalformedExport,
    Bounds,
    Poisoned,
}

impl fmt::Display for LoroStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scratch(error) => write!(f, "authenticated Loro scratch store failed: {error}"),
            Self::Sticky(error) => write!(f, "authenticated Loro store is latched: {error}"),
            Self::MisboundRoot => f.write_str("authenticated Loro root is misbound"),
            Self::MalformedRoot => f.write_str("malformed authenticated Loro root"),
            Self::MalformedWitness => f.write_str("malformed Loro history witness"),
            Self::MalformedExport => f.write_str("malformed canonical Loro store export"),
            Self::Bounds => f.write_str("Loro store key, value, count, or size exceeds its bound"),
            Self::Poisoned => f.write_str("authenticated Loro store lock was poisoned"),
        }
    }
}

impl std::error::Error for LoroStoreError {}

impl From<super::scratch_store::ScratchError> for LoroStoreError {
    fn from(error: super::scratch_store::ScratchError) -> Self {
        Self::Scratch(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cap_std::{ambient_authority, fs::Dir};
    use loro::{ExportMode, LoroDoc};
    use std::fs;
    use std::path::Path;
    use uuid::Uuid;

    fn workspace(value: u128) -> WorkspaceId {
        WorkspaceId::from_uuid(Uuid::from_u128(value))
    }

    fn document(value: u128) -> DocumentId {
        DocumentId::from_uuid(Uuid::from_u128(value))
    }

    fn batch(value: u128) -> BatchId {
        BatchId::from_uuid(Uuid::from_u128(value))
    }

    fn archive(root: &Path) -> Dir {
        fs::create_dir_all(root).unwrap();
        Dir::open_ambient_dir(root, ambient_authority()).unwrap()
    }

    fn witness(workspace_id: WorkspaceId, version: u8) -> LoroHistoryWitness {
        let document_id = document(2);
        let latest_source_batch = batch(version as u128 + 10);
        let causal_digest = super::super::DocumentDependencies::new(
            document_id,
            Vec::new(),
            vec![latest_source_batch],
        )
        .unwrap()
        .causal_state_digest();
        LoroHistoryWitness::new(
            workspace_id,
            document_id,
            0,
            causal_digest,
            latest_source_batch,
            ContentDigest::of(&[version, 1]),
            ContentDigest::of(&[version, 2]),
        )
        .unwrap()
    }

    fn cold_old_base_work(age: usize) -> (usize, usize) {
        let path = std::env::temp_dir().join(format!("tine-loro-cold-work-{}", Uuid::new_v4()));
        let archive = archive(&path);
        let workspace_id = workspace(age as u128 + 100);
        let scratch = Arc::new(ScratchStore::open(&archive, workspace_id).unwrap());
        let control = AuthenticatedLoroStore::empty(Arc::clone(&scratch));
        let external = LoroDoc::from_external_store(None, control.handle()).unwrap();
        external.set_peer_id(1).unwrap();
        external.get_map("map").insert("base", 0).unwrap();
        external.commit();
        let base_vv = external.oplog_vv();
        let base_updates = external.export(ExportMode::all_updates()).unwrap();

        let old_branch = LoroDoc::new();
        old_branch.import(&base_updates).unwrap();
        old_branch.set_peer_id(2).unwrap();
        old_branch.get_map("map").insert("old", true).unwrap();
        old_branch.commit();
        let old_update = old_branch.export(ExportMode::updates(&base_vv)).unwrap();

        external.set_peer_id(3).unwrap();
        for index in 0..age {
            external
                .get_map("map")
                .insert(&format!("age-{index:04}"), index as i64)
                .unwrap();
            external.commit();
        }
        let checkpoint = external.flush_external_store().unwrap();
        let root = control.publish_root(witness(workspace_id, 1)).unwrap();
        drop(external);
        drop(control);

        let before = scratch.stats();
        let control =
            AuthenticatedLoroStore::reopen(Arc::clone(&scratch), &root, root.witness()).unwrap();
        let reopened = LoroDoc::from_external_store(Some(&checkpoint), control.handle()).unwrap();
        reopened.evict_external_store_cache().unwrap();
        assert!(reopened.import(&old_update).unwrap().pending.is_none());
        let after = scratch.stats();
        let work = (
            (after.page_reads - before.page_reads) + (after.blob_reads - before.blob_reads),
            (after.page_bytes_read - before.page_bytes_read)
                + (after.blob_bytes_read - before.blob_bytes_read),
        );
        drop(reopened);
        drop(control);
        drop(scratch);
        fs::remove_dir_all(path).unwrap();
        work
    }

    #[test]
    fn root_reopens_canonical_ordered_store_without_syncing() {
        let path = std::env::temp_dir().join(format!("tine-loro-reopen-{}", Uuid::new_v4()));
        let archive = archive(&path);
        let workspace_id = workspace(1);
        let scratch = Arc::new(ScratchStore::open(&archive, workspace_id).unwrap());
        let mut control = AuthenticatedLoroStore::empty(Arc::clone(&scratch));
        control.set(b"b", Bytes::from_static(b"two"));
        control.set(b"a", Bytes::from_static(b"one"));
        let root = control.publish_root(witness(workspace_id, 1)).unwrap();
        assert_eq!(root.entry_count(), 2);

        let reopened =
            AuthenticatedLoroStore::reopen(Arc::clone(&scratch), &root, root.witness()).unwrap();
        assert_eq!(reopened.get(b"a"), Some(Bytes::from_static(b"one")));
        assert_eq!(
            reopened
                .scan(Bound::Unbounded, Bound::Unbounded)
                .collect::<Vec<_>>(),
            vec![
                (Bytes::from_static(b"a"), Bytes::from_static(b"one")),
                (Bytes::from_static(b"b"), Bytes::from_static(b"two")),
            ]
        );
        let stats = reopened.stats();
        assert_eq!(stats.flush_calls, 0);
        assert_eq!(stats.point_reads, 1);
        assert_eq!(stats.range_scans, 1);
        assert!(stats.history_page_reads > 0);
        assert!(stats.history_blob_reads > 0);
        assert_eq!(scratch.stats().scratch_syncs, 0);
        drop(reopened);
        drop(control);
        drop(scratch);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn empty_root_bulk_build_writes_each_node_once_without_history_reads_and_reopens() {
        let path = std::env::temp_dir().join(format!("tine-loro-bulk-{}", Uuid::new_v4()));
        let archive = archive(&path);
        let workspace_id = workspace(10);
        let scratch = Arc::new(ScratchStore::open(&archive, workspace_id).unwrap());
        let mut store = AuthenticatedLoroStore::empty(Arc::clone(&scratch));
        let expected = (0..4096_u32)
            .map(|index| {
                let key = format!("key-{index:04}").into_bytes();
                let value = if index % 17 == 0 {
                    Vec::new()
                } else {
                    format!("value-{index}").into_bytes()
                };
                store.set(&key, Bytes::copy_from_slice(&value));
                (Bytes::from(key), Bytes::from(value))
            })
            .collect::<Vec<_>>();

        let before = scratch.stats();
        let root = store.publish_root(witness(workspace_id, 1)).unwrap();
        let after = scratch.stats();
        assert_eq!(store.stats().history_page_reads, 0);
        assert_eq!(after.page_reads - before.page_reads, 0);
        assert_eq!(after.page_writes - before.page_writes, expected.len());
        assert_eq!(
            after.blob_writes - before.blob_writes,
            expected
                .iter()
                .filter(|(_, value)| !value.is_empty())
                .count()
        );
        assert_eq!(root.entry_count, expected.len() as u64);
        assert_eq!(
            root.logical_bytes,
            expected
                .iter()
                .map(|(key, value)| (key.len() + value.len()) as u64)
                .sum::<u64>()
        );

        let mut reopened =
            AuthenticatedLoroStore::reopen(Arc::clone(&scratch), &root, root.witness()).unwrap();
        assert_eq!(
            reopened
                .scan(Bound::Unbounded, Bound::Unbounded)
                .collect::<Vec<_>>(),
            expected
        );
        assert!(reopened.take_error().is_none());
        drop(reopened);
        drop(store);
        drop(scratch);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn empty_root_bulk_build_is_canonical_across_set_order() {
        fn build(order: impl IntoIterator<Item = u32>) -> (LoroNodeRef, std::path::PathBuf) {
            let path = std::env::temp_dir().join(format!("tine-loro-canonical-{}", Uuid::new_v4()));
            let archive = archive(&path);
            let workspace_id = workspace(11);
            let scratch = Arc::new(ScratchStore::open(&archive, workspace_id).unwrap());
            let mut store = AuthenticatedLoroStore::empty(Arc::clone(&scratch));
            for index in order {
                store.set(
                    format!("key-{index:04}").as_bytes(),
                    Bytes::from(format!("value-{index}").into_bytes()),
                );
            }
            let root = store.publish_root(witness(workspace_id, 1)).unwrap();
            (root.index_root.expect("nonempty canonical root"), path)
        }

        let (ascending, ascending_path) = build(0..257);
        let (descending, descending_path) = build((0..257).rev());
        assert_eq!(ascending, descending);
        fs::remove_dir_all(ascending_path).unwrap();
        fs::remove_dir_all(descending_path).unwrap();
    }

    #[test]
    fn empty_root_bulk_build_collapses_deletes_to_an_empty_reopenable_root() {
        let path = std::env::temp_dir().join(format!("tine-loro-bulk-empty-{}", Uuid::new_v4()));
        let archive = archive(&path);
        let workspace_id = workspace(12);
        let scratch = Arc::new(ScratchStore::open(&archive, workspace_id).unwrap());
        let mut store = AuthenticatedLoroStore::empty(Arc::clone(&scratch));
        store.set(b"removed", Bytes::from_static(b"value"));
        assert_eq!(store.remove(b"removed"), Some(Bytes::from_static(b"value")));
        assert_eq!(store.remove(b"missing"), None);

        let before = scratch.stats();
        let root = store.publish_root(witness(workspace_id, 1)).unwrap();
        let after = scratch.stats();
        assert!(root.index_root.is_none());
        assert_eq!(root.entry_count, 0);
        assert_eq!(root.logical_bytes, 0);
        assert_eq!(after.page_reads - before.page_reads, 0);
        assert_eq!(after.page_writes - before.page_writes, 0);
        assert_eq!(after.blob_writes - before.blob_writes, 0);

        let reopened =
            AuthenticatedLoroStore::reopen(Arc::clone(&scratch), &root, root.witness()).unwrap();
        assert!(reopened.is_empty());
        assert_eq!(
            reopened
                .scan(Bound::Unbounded, Bound::Unbounded)
                .collect::<Vec<_>>(),
            Vec::new()
        );
        drop(reopened);
        drop(store);
        drop(scratch);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn bulk_built_root_uses_existing_cow_path_for_later_updates() {
        let path = std::env::temp_dir().join(format!("tine-loro-bulk-cow-{}", Uuid::new_v4()));
        let archive = archive(&path);
        let workspace_id = workspace(13);
        let scratch = Arc::new(ScratchStore::open(&archive, workspace_id).unwrap());
        let mut store = AuthenticatedLoroStore::empty(Arc::clone(&scratch));
        for index in 0..257_u32 {
            store.set(
                format!("key-{index:04}").as_bytes(),
                Bytes::from(format!("value-{index}").into_bytes()),
            );
        }
        let first_root = store.publish_root(witness(workspace_id, 1)).unwrap();
        let mut reopened =
            AuthenticatedLoroStore::reopen(Arc::clone(&scratch), &first_root, first_root.witness())
                .unwrap();
        reopened.set(b"key-0128", Bytes::from_static(b"updated"));
        assert!(reopened.remove(b"key-0000").is_some());
        reopened.set(b"key-9999", Bytes::from_static(b"added"));

        let before = reopened.stats();
        let second_root = reopened.publish_root(witness(workspace_id, 2)).unwrap();
        let after = reopened.stats();
        assert!(
            after.history_page_reads > before.history_page_reads,
            "a nonempty root must use authenticated COW traversal"
        );
        assert_eq!(second_root.entry_count, 257);

        let mut final_store = AuthenticatedLoroStore::reopen(
            Arc::clone(&scratch),
            &second_root,
            second_root.witness(),
        )
        .unwrap();
        assert_eq!(final_store.get(b"key-0000"), None);
        assert_eq!(
            final_store.get(b"key-0128"),
            Some(Bytes::from_static(b"updated"))
        );
        assert_eq!(
            final_store.get(b"key-9999"),
            Some(Bytes::from_static(b"added"))
        );
        assert_eq!(final_store.len(), 257);
        assert!(final_store.take_error().is_none());
        drop(final_store);
        drop(reopened);
        drop(store);
        drop(scratch);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn clone_store_has_independent_overlay_and_root_publication() {
        let path = std::env::temp_dir().join(format!("tine-loro-cow-{}", Uuid::new_v4()));
        let archive = archive(&path);
        let workspace_id = workspace(3);
        let scratch = Arc::new(ScratchStore::open(&archive, workspace_id).unwrap());
        let mut parent = AuthenticatedLoroStore::empty(Arc::clone(&scratch));
        parent.set(b"base", Bytes::from_static(b"shared"));
        for index in 0..256 {
            parent.set(
                format!("base-{index:04}").as_bytes(),
                Bytes::from_static(b"resident"),
            );
        }
        parent.flush();
        let child = parent.clone_store();
        child
            .lock()
            .set(b"child", Bytes::from_static(b"only-child"));
        parent.set(b"parent", Bytes::from_static(b"only-parent"));

        assert_eq!(parent.get(b"base"), Some(Bytes::from_static(b"shared")));
        assert_eq!(parent.get(b"child"), None);
        assert_eq!(
            child.lock().get(b"parent"),
            None,
            "child must not observe the parent's later overlay"
        );
        assert_eq!(
            child.lock().get(b"child"),
            Some(Bytes::from_static(b"only-child"))
        );
        let before_child_flush = scratch.stats();
        child.lock().flush();
        let after_child_flush = scratch.stats();
        assert_eq!(
            after_child_flush.blob_writes - before_child_flush.blob_writes,
            1,
            "one-key child overlay must append only its changed value"
        );
        assert!(
            after_child_flush.page_writes - before_child_flush.page_writes <= 16,
            "one-key child overlay rewrote more than one AVL path"
        );
        assert!(
            after_child_flush.page_bytes_written - before_child_flush.page_bytes_written
                <= 32 * 1024,
            "one-key child overlay rewrote history-sized index bytes"
        );
        let parent_root = parent.publish_root(witness(workspace_id, 1)).unwrap();
        assert_eq!(parent_root.entry_count(), 258);
        assert_eq!(scratch.stats().scratch_syncs, 0);
        drop(child);
        drop(parent);
        drop(scratch);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn range_scan_honors_inclusive_exclusive_bounds_and_overlay_tombstones() {
        let path = std::env::temp_dir().join(format!("tine-loro-range-{}", Uuid::new_v4()));
        let archive = archive(&path);
        let workspace_id = workspace(4);
        let scratch = Arc::new(ScratchStore::open(&archive, workspace_id).unwrap());
        let mut store = AuthenticatedLoroStore::empty(Arc::clone(&scratch));
        for key in [b"a", b"b", b"c", b"d"] {
            store.set(key, Bytes::copy_from_slice(key));
        }
        store.flush();
        store.remove(b"c");
        store.set(b"bb", Bytes::from_static(b"between"));
        assert_eq!(
            store
                .scan(Bound::Excluded(b"a"), Bound::Included(b"c"))
                .collect::<Vec<_>>(),
            vec![
                (Bytes::from_static(b"b"), Bytes::from_static(b"b")),
                (Bytes::from_static(b"bb"), Bytes::from_static(b"between")),
            ]
        );
        drop(store);
        drop(scratch);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn tampered_page_fails_closed_and_error_is_sticky_until_taken() {
        let path = std::env::temp_dir().join(format!("tine-loro-tamper-{}", Uuid::new_v4()));
        let archive = archive(&path);
        let workspace_id = workspace(5);
        let scratch = Arc::new(ScratchStore::open(&archive, workspace_id).unwrap());
        let mut store = AuthenticatedLoroStore::empty(Arc::clone(&scratch));
        store.set(b"vv", Bytes::from_static(b"value"));
        let root = store.publish_root(witness(workspace_id, 1)).unwrap();
        scratch.tamper_page_byte_for_test(0);
        let mut reopened =
            AuthenticatedLoroStore::reopen(Arc::clone(&scratch), &root, root.witness()).unwrap();
        assert_eq!(reopened.get(b"vv"), None);
        assert_eq!(reopened.len(), 0);
        let error = reopened.take_error().expect("sticky authentication error");
        assert!(error.contains("digest mismatch"));
        assert_eq!(reopened.get(b"vv"), None);
        assert!(reopened.take_error().is_some());
        drop(reopened);
        drop(store);
        drop(scratch);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn misbound_page_fails_closed_through_sticky_error_channel() {
        let path = std::env::temp_dir().join(format!("tine-loro-misbound-{}", Uuid::new_v4()));
        let archive = archive(&path);
        let workspace_id = workspace(8);
        let scratch = Arc::new(ScratchStore::open(&archive, workspace_id).unwrap());
        let mut store = AuthenticatedLoroStore::empty(Arc::clone(&scratch));
        store.set(b"vv", Bytes::from_static(b"value"));
        let mut root = store.publish_root(witness(workspace_id, 1)).unwrap();
        ScratchStore::misbind_page_ref_for_test(
            &mut root
                .index_root
                .as_mut()
                .expect("populated Loro root")
                .page_ref,
        );
        root.authenticator = root_authenticator(
            root.scratch_binding,
            &root.index_root,
            root.entry_count,
            root.logical_bytes,
            &root.witness,
        )
        .unwrap();

        let mut reopened =
            AuthenticatedLoroStore::reopen(Arc::clone(&scratch), &root, root.witness()).unwrap();
        assert_eq!(reopened.get(b"vv"), None);
        assert!(reopened
            .take_error()
            .expect("sticky page-binding error")
            .contains("misbound"));
        drop(reopened);
        drop(store);
        drop(scratch);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn missing_page_fails_closed() {
        let path = std::env::temp_dir().join(format!("tine-loro-missing-{}", Uuid::new_v4()));
        let archive = archive(&path);
        let workspace_id = workspace(6);
        let scratch = Arc::new(ScratchStore::open(&archive, workspace_id).unwrap());
        let mut store = AuthenticatedLoroStore::empty(Arc::clone(&scratch));
        store.set(b"vv", Bytes::from_static(b"value"));
        let root = store.publish_root(witness(workspace_id, 1)).unwrap();
        scratch.truncate_pages_for_test();
        let mut reopened =
            AuthenticatedLoroStore::reopen(Arc::clone(&scratch), &root, root.witness()).unwrap();
        assert_eq!(reopened.get(b"vv"), None);
        assert!(reopened
            .take_error()
            .expect("sticky missing-page error")
            .contains("malformed"));
        drop(reopened);
        drop(store);
        drop(scratch);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn old_base_map_and_text_merge_survives_process_style_reopen() {
        let path = std::env::temp_dir().join(format!("tine-loro-merge-{}", Uuid::new_v4()));
        let archive = archive(&path);
        let workspace_id = workspace(7);
        let scratch = Arc::new(ScratchStore::open(&archive, workspace_id).unwrap());
        let control = AuthenticatedLoroStore::empty(Arc::clone(&scratch));
        let external = LoroDoc::from_external_store(None, control.handle()).unwrap();
        external.set_peer_id(1).unwrap();
        external.get_map("map").insert("key", "base").unwrap();
        external.get_text("text").insert(0, "base").unwrap();
        external.commit();

        let base_vv = external.oplog_vv();
        let base_updates = external.export(ExportMode::all_updates()).unwrap();
        let old_branch = LoroDoc::new();
        old_branch.import(&base_updates).unwrap();
        old_branch.set_peer_id(2).unwrap();
        old_branch
            .get_map("map")
            .insert("key", "old-branch")
            .unwrap();
        old_branch.get_text("text").insert(4, "-old").unwrap();
        old_branch.commit();
        let old_update = old_branch.export(ExportMode::updates(&base_vv)).unwrap();

        let expected = LoroDoc::new();
        expected.import(&base_updates).unwrap();
        expected.set_peer_id(3).unwrap();
        expected.get_map("map").insert("key", "new-branch").unwrap();
        expected.get_text("text").insert(4, "-new").unwrap();
        expected.commit();
        assert!(expected.import(&old_update).unwrap().pending.is_none());

        external.set_peer_id(3).unwrap();
        external.get_map("map").insert("key", "new-branch").unwrap();
        external.get_text("text").insert(4, "-new").unwrap();
        external.commit();
        let checkpoint = external.flush_external_store().unwrap();
        let root = control.publish_root(witness(workspace_id, 1)).unwrap();
        drop(external);
        drop(control);

        let control =
            AuthenticatedLoroStore::reopen(Arc::clone(&scratch), &root, root.witness()).unwrap();
        let reopened = LoroDoc::from_external_store(Some(&checkpoint), control.handle()).unwrap();
        reopened.evict_external_store_cache().unwrap();
        assert!(reopened.import(&old_update).unwrap().pending.is_none());
        let merged_checkpoint = reopened.flush_external_store().unwrap();
        let merged_root = control.publish_root(witness(workspace_id, 2)).unwrap();
        drop(reopened);
        drop(control);

        let control = AuthenticatedLoroStore::reopen(
            Arc::clone(&scratch),
            &merged_root,
            merged_root.witness(),
        )
        .unwrap();
        let reopened =
            LoroDoc::from_external_store(Some(&merged_checkpoint), control.handle()).unwrap();
        reopened.evict_external_store_cache().unwrap();
        assert_eq!(reopened.get_deep_value(), expected.get_deep_value());
        assert_eq!(reopened.oplog_vv(), expected.oplog_vv());
        assert_eq!(
            reopened.get_map("map").get_deep_value(),
            expected.get_map("map").get_deep_value()
        );
        let text = reopened.get_text("text").to_string();
        assert!(text.contains("-new"));
        assert!(text.contains("-old"));
        assert_eq!(scratch.stats().scratch_syncs, 0);
        drop(reopened);
        drop(control);
        drop(scratch);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    #[ignore = "vendored Loro still traverses current-branch blocks during an old-base DAG join"]
    fn cold_old_base_import_work_is_page_age_independent() {
        let short = cold_old_base_work(8);
        let long = cold_old_base_work(4096);
        assert!(
            long.0 <= short.0 + 4,
            "cold page reads grew with history age: short={short:?} long={long:?}"
        );
        assert!(
            long.1 <= short.1.saturating_mul(2).saturating_add(16 * 1024),
            "cold page bytes grew with history age: short={short:?} long={long:?}"
        );
    }

    #[test]
    fn broad_predecessor_scan_reads_one_authenticated_tree_path() {
        let path = std::env::temp_dir().join(format!("tine-loro-lazy-scan-{}", Uuid::new_v4()));
        let archive = archive(&path);
        let scratch = Arc::new(ScratchStore::open(&archive, workspace(9)).unwrap());
        let mut store = AuthenticatedLoroStore::empty(Arc::clone(&scratch));
        for index in 0..1024_u32 {
            store.set(
                &index.to_be_bytes(),
                Bytes::copy_from_slice(&index.to_be_bytes()),
            );
        }
        store.flush();
        let before = scratch.stats();
        assert_eq!(
            store
                .scan(Bound::Unbounded, Bound::Included(&900_u32.to_be_bytes()))
                .next_back(),
            Some((
                Bytes::copy_from_slice(&900_u32.to_be_bytes()),
                Bytes::copy_from_slice(&900_u32.to_be_bytes()),
            ))
        );
        let after = scratch.stats();
        assert!(
            after.page_reads - before.page_reads <= 16,
            "one predecessor lookup read more than one AVL path"
        );
        assert_eq!(after.blob_reads - before.blob_reads, 1);
        assert!(
            (after.page_bytes_read - before.page_bytes_read)
                + (after.blob_bytes_read - before.blob_bytes_read)
                <= 32 * 1024,
            "one predecessor lookup read history-sized bytes"
        );
        drop(store);
        drop(scratch);
        fs::remove_dir_all(path).unwrap();
    }
}
