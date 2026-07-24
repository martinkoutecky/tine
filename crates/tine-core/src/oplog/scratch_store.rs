use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(windows)]
use std::os::windows::fs::MetadataExt as _;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle as _;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(windows)]
use cap_fs_ext::OsMetadataExt as _;
use cap_std::fs::{Dir, OpenOptions};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{BatchId, ContentDigest, WorkspaceId};

pub(crate) const SCRATCH_DIR: &str = "engine-scratch-v2";
const MARKER_FILE: &str = "marker";
const LEASE_FILE: &str = "lease";
const PAGES_FILE: &str = "pages.index";
const BLOBS_FILE: &str = "blobs.data";
const SCRATCH_SCHEMA_VERSION: u32 = 9;
const SCRATCH_PAGE_SCHEMA_VERSION: u32 = 1;
const SCRATCH_LSM_LEVELS: usize = 32;
const ACCEPTED_SEQUENCE_SCHEMA_VERSION: u32 = 1;
const ACCEPTED_SEQUENCE_LEAF_CAPACITY: usize = 1;
const ACCEPTED_SEQUENCE_NODE_FANOUT: usize = 32;
const AUTHENTICATED_MAP_SCHEMA_VERSION: u32 = 1;
const MAX_AUTHENTICATED_MAP_DEPTH: usize = 256;
const CURRENT_FILTER_WORDS: usize = 16_384;
const MAX_MARKER_BYTES: u64 = 4 * 1024;
const MAX_PAGE_BYTES: usize = 256 * 1024 * 1024;
const MAX_BLOB_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ScratchStats {
    pub page_reads: usize,
    pub page_writes: usize,
    pub page_bytes_read: usize,
    pub page_bytes_written: usize,
    pub max_page_bytes_read: usize,
    pub blob_reads: usize,
    pub blob_writes: usize,
    pub blob_bytes_read: usize,
    pub blob_bytes_written: usize,
    pub point_reads: usize,
    pub range_reads: usize,
    pub scratch_syncs: usize,
    pub stale_runs_reclaimed: usize,
    pub live_runs_skipped: usize,
}

#[derive(Debug, Default)]
struct ScratchCounters {
    page_reads: AtomicUsize,
    page_writes: AtomicUsize,
    page_bytes_read: AtomicUsize,
    page_bytes_written: AtomicUsize,
    max_page_bytes_read: AtomicUsize,
    blob_reads: AtomicUsize,
    blob_writes: AtomicUsize,
    blob_bytes_read: AtomicUsize,
    blob_bytes_written: AtomicUsize,
    point_reads: AtomicUsize,
    range_reads: AtomicUsize,
    // This deliberately has no increment site. Any future scratch sync must
    // become visible to the normal-flow regression gates.
    scratch_syncs: AtomicUsize,
    stale_runs_reclaimed: AtomicUsize,
    live_runs_skipped: AtomicUsize,
}

#[derive(Debug)]
struct FixedPointFilter {
    words: Vec<u64>,
}

impl Default for FixedPointFilter {
    fn default() -> Self {
        Self {
            words: vec![0; CURRENT_FILTER_WORDS],
        }
    }
}

impl FixedPointFilter {
    fn insert(&mut self, key: &[u8]) {
        for position in self.positions(key) {
            self.words[position / 64] |= 1_u64 << (position % 64);
        }
    }

    fn might_contain(&self, key: &[u8]) -> bool {
        self.positions(key)
            .into_iter()
            .all(|position| self.words[position / 64] & (1_u64 << (position % 64)) != 0)
    }

    fn positions(&self, key: &[u8]) -> [usize; 4] {
        let digest = ContentDigest::of(key);
        let bytes = digest.as_bytes();
        let first = u64::from_be_bytes(bytes[..8].try_into().expect("digest word"));
        let second = u64::from_be_bytes(bytes[8..16].try_into().expect("digest word")) | 1;
        let bits = self.words.len() as u64 * 64;
        std::array::from_fn(|index| {
            first
                .wrapping_add(second.wrapping_mul(index as u64))
                .wrapping_rem(bits) as usize
        })
    }
}

impl ScratchCounters {
    fn snapshot(&self) -> ScratchStats {
        ScratchStats {
            page_reads: self.page_reads.load(Ordering::Relaxed),
            page_writes: self.page_writes.load(Ordering::Relaxed),
            page_bytes_read: self.page_bytes_read.load(Ordering::Relaxed),
            page_bytes_written: self.page_bytes_written.load(Ordering::Relaxed),
            max_page_bytes_read: self.max_page_bytes_read.load(Ordering::Relaxed),
            blob_reads: self.blob_reads.load(Ordering::Relaxed),
            blob_writes: self.blob_writes.load(Ordering::Relaxed),
            blob_bytes_read: self.blob_bytes_read.load(Ordering::Relaxed),
            blob_bytes_written: self.blob_bytes_written.load(Ordering::Relaxed),
            point_reads: self.point_reads.load(Ordering::Relaxed),
            range_reads: self.range_reads.load(Ordering::Relaxed),
            scratch_syncs: self.scratch_syncs.load(Ordering::Relaxed),
            stale_runs_reclaimed: self.stale_runs_reclaimed.load(Ordering::Relaxed),
            live_runs_skipped: self.live_runs_skipped.load(Ordering::Relaxed),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScratchRunMarkerV2 {
    schema_version: u32,
    workspace_id: WorkspaceId,
    run_id: Uuid,
    random_owner_nonce: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
pub(crate) enum ScratchPageKind {
    BatchStatus = 1,
    DependencyWait = 2,
    ReadyQueue = 3,
    CausalBatch = 4,
    CausalDot = 5,
    CausalPeer = 6,
    DocumentCurrent = 7,
    DocumentExact = 8,
    DocumentAfterBatch = 9,
    BlobDedup = 10,
    Conflict = 11,
    LoroHistory = 12,
    DocumentExternalCurrent = 13,
    DocumentExternalExact = 14,
    AcceptedFrontier = 15,
    AcceptedSequenceLeaf = 16,
    AcceptedSequenceNode = 17,
    AcceptedDocumentMap = 18,
    AcceptedBatchMap = 19,
    PageNameCatalogFrontier = 20,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScratchPageRef {
    offset: u64,
    encoded_len: u32,
    digest: ContentDigest,
    kind: ScratchPageKind,
    key_min: Vec<u8>,
    key_max: Vec<u8>,
}

impl ScratchPageRef {
    pub(crate) fn key_min(&self) -> &[u8] {
        &self.key_min
    }

    pub(crate) fn key_max(&self) -> &[u8] {
        &self.key_max
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScratchPageEnvelope {
    schema_version: u32,
    kind: ScratchPageKind,
    key_min: Vec<u8>,
    key_max: Vec<u8>,
    payload: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScratchBlobRef {
    offset: u64,
    encoded_len: u32,
    digest: ContentDigest,
}

impl ScratchBlobRef {
    pub(crate) const fn digest(&self) -> ContentDigest {
        self.digest
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScratchRecord {
    key: Vec<u8>,
    value: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScratchSegment {
    schema_version: u32,
    kind: ScratchPageKind,
    generation: u64,
    entries: Vec<ScratchRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScratchSegmentRef {
    generation: u64,
    entry_count: u64,
    page_ref: ScratchPageRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScratchLsmRoot {
    next_generation: u64,
    levels: Vec<Option<ScratchSegmentRef>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScratchAcceptedSequenceRoot {
    schema_version: u32,
    len: u64,
    height: u8,
    root: Option<ScratchPageRef>,
}

impl Default for ScratchAcceptedSequenceRoot {
    fn default() -> Self {
        Self {
            schema_version: ACCEPTED_SEQUENCE_SCHEMA_VERSION,
            len: 0,
            height: 0,
            root: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptedSequenceLeaf {
    schema_version: u32,
    first_sequence: u64,
    entries: Vec<AcceptedSequenceEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AcceptedSequenceEntry {
    pub batch_id: BatchId,
    pub evidence: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptedSequenceNode {
    schema_version: u32,
    height: u8,
    first_leaf: u64,
    children: Vec<ScratchPageRef>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScratchAuthenticatedMapRoot {
    schema_version: u32,
    count: u64,
    root_key: Option<[u8; 16]>,
    root_digest: ContentDigest,
    root: Option<ScratchPageRef>,
}

impl Default for ScratchAuthenticatedMapRoot {
    fn default() -> Self {
        Self {
            schema_version: AUTHENTICATED_MAP_SCHEMA_VERSION,
            count: 0,
            root_key: None,
            root_digest: authenticated_map_empty_digest(),
            root: None,
        }
    }
}

impl ScratchAuthenticatedMapRoot {
    pub(crate) const fn count(&self) -> u64 {
        self.count
    }

    pub(crate) const fn root_key(&self) -> Option<[u8; 16]> {
        self.root_key
    }

    pub(crate) const fn root_digest(&self) -> ContentDigest {
        self.root_digest
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthenticatedMapChild {
    key: [u8; 16],
    digest: ContentDigest,
    page_ref: ScratchPageRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthenticatedMapNode {
    schema_version: u32,
    key: [u8; 16],
    priority: ContentDigest,
    value_digest: ContentDigest,
    left: Option<AuthenticatedMapChild>,
    right: Option<AuthenticatedMapChild>,
}

impl Default for ScratchLsmRoot {
    fn default() -> Self {
        Self {
            next_generation: 0,
            levels: vec![None; SCRATCH_LSM_LEVELS],
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScratchRoots {
    pub batch_status_root: ScratchLsmRoot,
    pub wait_root: ScratchLsmRoot,
    pub ready_queue_root: ScratchLsmRoot,
    pub ready_queue_len: u64,
    pub causal_root: ScratchLsmRoot,
    pub causal_dot_root: ScratchLsmRoot,
    pub causal_peer_root: ScratchLsmRoot,
    pub document_current_root: ScratchLsmRoot,
    pub document_state_root: ScratchLsmRoot,
    pub document_after_batch_root: ScratchLsmRoot,
    pub blob_dedup_root: ScratchLsmRoot,
    pub conflict_root: ScratchLsmRoot,
    pub external_document_current_root: ScratchLsmRoot,
    pub external_document_state_root: ScratchLsmRoot,
    pub accepted_frontier_root: ScratchLsmRoot,
    pub accepted_sequence_root: ScratchAcceptedSequenceRoot,
    pub accepted_document_map_root: ScratchAuthenticatedMapRoot,
    pub accepted_batch_map_root: ScratchAuthenticatedMapRoot,
}

/// One reconstructible, authenticated run-local scratch namespace.
///
/// The authoritative archive is not reachable through this type. All removal
/// is capability-relative beneath the exact scratch namespace.
pub(crate) struct ScratchStore {
    namespace: Dir,
    run: Dir,
    run_name: String,
    marker: ScratchRunMarkerV2,
    lease: fs::File,
    pages: Mutex<fs::File>,
    blobs: Mutex<fs::File>,
    counters: Arc<ScratchCounters>,
    document_current_filter: Mutex<FixedPointFilter>,
}

impl fmt::Debug for ScratchStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScratchStore")
            .field("run_name", &self.run_name)
            .field("workspace_id", &self.marker.workspace_id)
            .finish_non_exhaustive()
    }
}

impl ScratchStore {
    pub(crate) fn open(
        archive_capability: &Dir,
        workspace_id: WorkspaceId,
    ) -> Result<Self, ScratchError> {
        super::object_store::ensure_directory_nofollow(archive_capability, SCRATCH_DIR)?;
        let namespace = super::object_store::open_dir_nofollow(archive_capability, SCRATCH_DIR)?;
        let run_id = Uuid::new_v4();
        let run_name = format!("run-{run_id}");
        super::object_store::ensure_directory_nofollow(&namespace, &run_name)?;
        let run = super::object_store::open_dir_nofollow(&namespace, &run_name)?;
        let nonce_a = Uuid::new_v4();
        let nonce_b = Uuid::new_v4();
        let mut random_owner_nonce = [0_u8; 32];
        random_owner_nonce[..16].copy_from_slice(nonce_a.as_bytes());
        random_owner_nonce[16..].copy_from_slice(nonce_b.as_bytes());
        let marker = ScratchRunMarkerV2 {
            schema_version: SCRATCH_SCHEMA_VERSION,
            workspace_id,
            run_id,
            random_owner_nonce,
        };
        write_new_regular(&run, MARKER_FILE, &encode_canonical(&marker)?)?;
        let lease = create_new_regular(&run, LEASE_FILE)?;
        lock_exclusive_nonblocking(&lease)?
            .then_some(())
            .ok_or_else(|| {
                ScratchError::UnsafeEntry("new scratch lease was already locked".into())
            })?;
        let pages = create_new_regular(&run, PAGES_FILE)?;
        let blobs = create_new_regular(&run, BLOBS_FILE)?;
        let store = Self {
            namespace,
            run,
            run_name,
            marker,
            lease,
            pages: Mutex::new(pages),
            blobs: Mutex::new(blobs),
            counters: Arc::new(ScratchCounters::default()),
            document_current_filter: Mutex::new(FixedPointFilter::default()),
        };
        if let Err(error) = store.reclaim_stale_runs() {
            store.cleanup_own_run();
            return Err(error);
        }
        Ok(store)
    }

    pub(crate) fn stats(&self) -> ScratchStats {
        self.counters.snapshot()
    }

    pub(crate) const fn workspace_id(&self) -> WorkspaceId {
        self.marker.workspace_id
    }

    #[cfg(test)]
    pub(crate) fn truncate_pages_for_test(&self) {
        self.pages
            .lock()
            .expect("scratch pages lock")
            .set_len(0)
            .expect("truncate scratch pages");
    }

    #[cfg(test)]
    pub(crate) fn tamper_page_byte_for_test(&self, offset: u64) {
        let mut pages = self.pages.lock().expect("scratch pages lock");
        pages
            .seek(SeekFrom::Start(offset))
            .expect("seek scratch page");
        let mut byte = [0_u8; 1];
        pages.read_exact(&mut byte).expect("read scratch page byte");
        byte[0] ^= 0x80;
        pages
            .seek(SeekFrom::Start(offset))
            .expect("seek scratch page");
        pages.write_all(&byte).expect("tamper scratch page byte");
    }

    #[cfg(test)]
    pub(crate) fn misbind_page_ref_for_test(page_ref: &mut ScratchPageRef) {
        page_ref.kind = ScratchPageKind::BatchStatus;
    }

    pub(crate) fn binding_digest(&self) -> Result<ContentDigest, ScratchError> {
        Ok(ContentDigest::of(&encode_canonical(&self.marker)?))
    }

    pub(crate) fn clone_pages_file(&self) -> Result<fs::File, ScratchError> {
        self.pages
            .lock()
            .map_err(|_| ScratchError::Poisoned)?
            .try_clone()
            .map_err(Into::into)
    }

    pub(crate) fn insert_many(
        &self,
        root: &ScratchLsmRoot,
        kind: ScratchPageKind,
        records: &BTreeMap<Vec<u8>, Option<Vec<u8>>>,
    ) -> Result<ScratchLsmRoot, ScratchError> {
        if records.is_empty() {
            return Ok(root.clone());
        }
        validate_root(root)?;
        let generation = root
            .next_generation
            .checked_add(1)
            .ok_or(ScratchError::MalformedPage)?;
        let mut merged = records.clone();
        let mut next = root.clone();
        next.next_generation = generation;
        for level in 0..SCRATCH_LSM_LEVELS {
            if let Some(existing) = next.levels[level].take() {
                let old = self.read_segment(kind, &existing)?;
                for record in old.entries {
                    merged.entry(record.key).or_insert(record.value);
                }
                continue;
            }
            let entries = merged
                .into_iter()
                .map(|(key, value)| ScratchRecord { key, value })
                .collect::<Vec<_>>();
            let segment = ScratchSegment {
                schema_version: SCRATCH_PAGE_SCHEMA_VERSION,
                kind,
                generation,
                entries,
            };
            validate_segment(&segment)?;
            let key_min = segment
                .entries
                .first()
                .expect("nonempty insertion")
                .key
                .clone();
            let key_max = segment
                .entries
                .last()
                .expect("nonempty insertion")
                .key
                .clone();
            let page_ref = self.append_page(kind, key_min, key_max, &segment)?;
            next.levels[level] = Some(ScratchSegmentRef {
                generation,
                entry_count: segment.entries.len() as u64,
                page_ref,
            });
            if kind == ScratchPageKind::DocumentCurrent {
                let mut filter = self
                    .document_current_filter
                    .lock()
                    .map_err(|_| ScratchError::Poisoned)?;
                for (key, value) in records {
                    if value.is_some() {
                        filter.insert(key);
                    }
                }
            }
            return Ok(next);
        }
        Err(ScratchError::IndexCapacity)
    }

    pub(crate) fn lookup(
        &self,
        root: &ScratchLsmRoot,
        kind: ScratchPageKind,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, ScratchError> {
        validate_root(root)?;
        self.counters.point_reads.fetch_add(1, Ordering::Relaxed);
        if kind == ScratchPageKind::DocumentCurrent
            && !self
                .document_current_filter
                .lock()
                .map_err(|_| ScratchError::Poisoned)?
                .might_contain(key)
        {
            return Ok(None);
        }
        let mut segments = root
            .levels
            .iter()
            .flatten()
            .collect::<Vec<&ScratchSegmentRef>>();
        segments.sort_unstable_by_key(|segment| std::cmp::Reverse(segment.generation));
        for segment_ref in segments {
            if key < segment_ref.page_ref.key_min.as_slice()
                || key > segment_ref.page_ref.key_max.as_slice()
            {
                continue;
            }
            let segment = self.read_segment(kind, segment_ref)?;
            if let Ok(index) = segment
                .entries
                .binary_search_by(|record| record.key.as_slice().cmp(key))
            {
                return Ok(segment.entries[index].value.clone());
            }
        }
        Ok(None)
    }

    pub(crate) fn append_accepted_sequence(
        &self,
        root: &ScratchAcceptedSequenceRoot,
        sequence: u64,
        batch_id: BatchId,
        evidence: Vec<u8>,
    ) -> Result<ScratchAcceptedSequenceRoot, ScratchError> {
        validate_accepted_sequence_root(root)?;
        if sequence == 0 || sequence != root.len.saturating_add(1) {
            return Err(ScratchError::MalformedPage);
        }
        let leaf_index = (sequence - 1) / ACCEPTED_SEQUENCE_LEAF_CAPACITY as u64;
        let (page_ref, height) = match &root.root {
            None => (
                self.write_accepted_sequence_leaf(
                    sequence,
                    vec![AcceptedSequenceEntry { batch_id, evidence }],
                )?,
                0,
            ),
            Some(current)
                if leaf_index
                    < accepted_sequence_leaf_capacity(root.height)
                        .ok_or(ScratchError::IndexCapacity)? =>
            {
                (
                    self.append_accepted_sequence_at(
                        current,
                        root.height,
                        0,
                        leaf_index,
                        sequence,
                        batch_id,
                        evidence,
                    )?,
                    root.height,
                )
            }
            Some(current) => {
                let height = root
                    .height
                    .checked_add(1)
                    .ok_or(ScratchError::IndexCapacity)?;
                let new_child = self.build_accepted_sequence_path(
                    root.height,
                    leaf_index,
                    sequence,
                    batch_id,
                    evidence,
                )?;
                let node = AcceptedSequenceNode {
                    schema_version: ACCEPTED_SEQUENCE_SCHEMA_VERSION,
                    height,
                    first_leaf: 0,
                    children: vec![current.clone(), new_child],
                };
                (self.write_accepted_sequence_node(&node)?, height)
            }
        };
        let next = ScratchAcceptedSequenceRoot {
            schema_version: ACCEPTED_SEQUENCE_SCHEMA_VERSION,
            len: sequence,
            height,
            root: Some(page_ref),
        };
        validate_accepted_sequence_root(&next)?;
        Ok(next)
    }

    pub(crate) fn lookup_accepted_sequence(
        &self,
        root: &ScratchAcceptedSequenceRoot,
        sequence: u64,
    ) -> Result<Option<AcceptedSequenceEntry>, ScratchError> {
        validate_accepted_sequence_root(root)?;
        self.counters.point_reads.fetch_add(1, Ordering::Relaxed);
        if sequence == 0 || sequence > root.len {
            return Ok(None);
        }
        let leaf_index = (sequence - 1) / ACCEPTED_SEQUENCE_LEAF_CAPACITY as u64;
        let mut page_ref = root.root.clone().ok_or(ScratchError::MalformedPage)?;
        let mut height = root.height;
        let mut first_leaf = 0_u64;
        while height > 0 {
            let node = self.read_accepted_sequence_node(&page_ref, height, first_leaf)?;
            let child_capacity =
                accepted_sequence_leaf_capacity(height - 1).ok_or(ScratchError::IndexCapacity)?;
            let slot = usize::try_from((leaf_index - first_leaf) / child_capacity)
                .map_err(|_| ScratchError::MalformedPage)?;
            page_ref = node
                .children
                .get(slot)
                .cloned()
                .ok_or(ScratchError::MalformedPage)?;
            first_leaf = first_leaf
                .checked_add(
                    u64::try_from(slot)
                        .map_err(|_| ScratchError::MalformedPage)?
                        .saturating_mul(child_capacity),
                )
                .ok_or(ScratchError::MalformedPage)?;
            height -= 1;
        }
        let leaf = self.read_accepted_sequence_leaf(&page_ref, first_leaf)?;
        let offset = usize::try_from((sequence - 1) % ACCEPTED_SEQUENCE_LEAF_CAPACITY as u64)
            .map_err(|_| ScratchError::MalformedPage)?;
        leaf.entries
            .get(offset)
            .cloned()
            .ok_or(ScratchError::MalformedPage)
            .map(Some)
    }

    pub(crate) fn accepted_sequence_cursor<'a>(
        &'a self,
        root: &'a ScratchAcceptedSequenceRoot,
    ) -> Result<ScratchAcceptedSequenceCursor<'a>, ScratchError> {
        validate_accepted_sequence_root(root)?;
        Ok(ScratchAcceptedSequenceCursor {
            store: self,
            root,
            stack: Vec::new(),
            leaf: None,
            next_sequence: 1,
            initialized: false,
            page_reads: 0,
            page_bytes_read: 0,
            max_page_bytes_read: 0,
        })
    }

    pub(crate) fn authenticated_map_upsert(
        &self,
        root: &ScratchAuthenticatedMapRoot,
        key: [u8; 16],
        value_digest: ContentDigest,
    ) -> Result<ScratchAuthenticatedMapRoot, ScratchError> {
        self.authenticated_map_upsert_for_kind(
            ScratchPageKind::AcceptedDocumentMap,
            root,
            key,
            value_digest,
        )
    }

    pub(crate) fn accepted_batch_map_upsert(
        &self,
        root: &ScratchAuthenticatedMapRoot,
        key: [u8; 16],
        value_digest: ContentDigest,
    ) -> Result<ScratchAuthenticatedMapRoot, ScratchError> {
        self.authenticated_map_upsert_for_kind(
            ScratchPageKind::AcceptedBatchMap,
            root,
            key,
            value_digest,
        )
    }

    fn authenticated_map_upsert_for_kind(
        &self,
        kind: ScratchPageKind,
        root: &ScratchAuthenticatedMapRoot,
        key: [u8; 16],
        value_digest: ContentDigest,
    ) -> Result<ScratchAuthenticatedMapRoot, ScratchError> {
        validate_authenticated_map_root(root)?;
        let (child, inserted) = self.authenticated_map_upsert_child(
            kind,
            root.root.as_ref().map(|page_ref| AuthenticatedMapChild {
                key: root.root_key.expect("validated nonempty root key"),
                digest: root.root_digest,
                page_ref: page_ref.clone(),
            }),
            key,
            value_digest,
            0,
        )?;
        let count = if inserted {
            root.count
                .checked_add(1)
                .ok_or(ScratchError::IndexCapacity)?
        } else {
            root.count
        };
        let next = ScratchAuthenticatedMapRoot {
            schema_version: AUTHENTICATED_MAP_SCHEMA_VERSION,
            count,
            root_key: Some(child.key),
            root_digest: child.digest,
            root: Some(child.page_ref),
        };
        validate_authenticated_map_root(&next)?;
        Ok(next)
    }

    fn authenticated_map_upsert_child(
        &self,
        kind: ScratchPageKind,
        current: Option<AuthenticatedMapChild>,
        key: [u8; 16],
        value_digest: ContentDigest,
        depth: usize,
    ) -> Result<(AuthenticatedMapChild, bool), ScratchError> {
        if depth > MAX_AUTHENTICATED_MAP_DEPTH {
            return Err(ScratchError::IndexCapacity);
        }
        let Some(current) = current else {
            let node = AuthenticatedMapNode {
                schema_version: AUTHENTICATED_MAP_SCHEMA_VERSION,
                key,
                priority: authenticated_map_priority(key),
                value_digest,
                left: None,
                right: None,
            };
            return Ok((self.write_authenticated_map_node(kind, &node)?, true));
        };
        let mut node = self.read_authenticated_map_node(kind, &current)?;
        let inserted;
        match key.cmp(&node.key) {
            std::cmp::Ordering::Equal => {
                node.value_digest = value_digest;
                inserted = false;
            }
            std::cmp::Ordering::Less => {
                let (left, was_inserted) = self.authenticated_map_upsert_child(
                    kind,
                    node.left.take(),
                    key,
                    value_digest,
                    depth + 1,
                )?;
                node.left = Some(left);
                inserted = was_inserted;
                if node.left.as_ref().is_some_and(|left| {
                    authenticated_map_priority_order(left.key, node.key).is_lt()
                }) {
                    return Ok((self.rotate_authenticated_map_right(kind, node)?, inserted));
                }
            }
            std::cmp::Ordering::Greater => {
                let (right, was_inserted) = self.authenticated_map_upsert_child(
                    kind,
                    node.right.take(),
                    key,
                    value_digest,
                    depth + 1,
                )?;
                node.right = Some(right);
                inserted = was_inserted;
                if node.right.as_ref().is_some_and(|right| {
                    authenticated_map_priority_order(right.key, node.key).is_lt()
                }) {
                    return Ok((self.rotate_authenticated_map_left(kind, node)?, inserted));
                }
            }
        }
        Ok((self.write_authenticated_map_node(kind, &node)?, inserted))
    }

    fn rotate_authenticated_map_right(
        &self,
        kind: ScratchPageKind,
        mut node: AuthenticatedMapNode,
    ) -> Result<AuthenticatedMapChild, ScratchError> {
        let left = node.left.take().ok_or(ScratchError::MalformedPage)?;
        let mut left_node = self.read_authenticated_map_node(kind, &left)?;
        node.left = left_node.right.take();
        left_node.right = Some(self.write_authenticated_map_node(kind, &node)?);
        self.write_authenticated_map_node(kind, &left_node)
    }

    fn rotate_authenticated_map_left(
        &self,
        kind: ScratchPageKind,
        mut node: AuthenticatedMapNode,
    ) -> Result<AuthenticatedMapChild, ScratchError> {
        let right = node.right.take().ok_or(ScratchError::MalformedPage)?;
        let mut right_node = self.read_authenticated_map_node(kind, &right)?;
        node.right = right_node.left.take();
        right_node.left = Some(self.write_authenticated_map_node(kind, &node)?);
        self.write_authenticated_map_node(kind, &right_node)
    }

    pub(crate) fn scan_prefix(
        &self,
        root: &ScratchLsmRoot,
        kind: ScratchPageKind,
        prefix: &[u8],
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, ScratchError> {
        validate_root(root)?;
        self.counters.range_reads.fetch_add(1, Ordering::Relaxed);
        let mut segments = root
            .levels
            .iter()
            .flatten()
            .collect::<Vec<&ScratchSegmentRef>>();
        segments.sort_unstable_by_key(|segment| segment.generation);
        let mut merged = BTreeMap::<Vec<u8>, Option<Vec<u8>>>::new();
        for segment_ref in segments {
            let segment = self.read_segment(kind, segment_ref)?;
            for record in segment.entries {
                if record.key.starts_with(prefix) {
                    merged.insert(record.key, record.value);
                }
            }
        }
        Ok(merged
            .into_iter()
            .filter_map(|(key, value)| value.map(|value| (key, value)))
            .collect())
    }

    pub(crate) fn materialize(
        &self,
        root: &ScratchLsmRoot,
        kind: ScratchPageKind,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, ScratchError> {
        self.scan_prefix(root, kind, &[])
    }

    pub(crate) fn append_blob(&self, bytes: &[u8]) -> Result<ScratchBlobRef, ScratchError> {
        if bytes.is_empty() || bytes.len() > MAX_BLOB_BYTES {
            return Err(ScratchError::MalformedBlob);
        }
        let digest = ContentDigest::of(bytes);
        let encoded_len = u32::try_from(bytes.len()).map_err(|_| ScratchError::MalformedBlob)?;
        let mut file = self.blobs.lock().map_err(|_| ScratchError::Poisoned)?;
        let offset = file.seek(SeekFrom::End(0))?;
        file.write_all(bytes)?;
        self.counters.blob_writes.fetch_add(1, Ordering::Relaxed);
        self.counters
            .blob_bytes_written
            .fetch_add(bytes.len(), Ordering::Relaxed);
        Ok(ScratchBlobRef {
            offset,
            encoded_len,
            digest,
        })
    }

    pub(crate) fn read_blob(&self, blob_ref: &ScratchBlobRef) -> Result<Vec<u8>, ScratchError> {
        let length =
            usize::try_from(blob_ref.encoded_len).map_err(|_| ScratchError::MalformedBlob)?;
        if length == 0 || length > MAX_BLOB_BYTES {
            return Err(ScratchError::MalformedBlob);
        }
        let mut bytes = vec![0_u8; length];
        let mut file = self.blobs.lock().map_err(|_| ScratchError::Poisoned)?;
        file.seek(SeekFrom::Start(blob_ref.offset))?;
        file.read_exact(&mut bytes)
            .map_err(|_| ScratchError::MalformedBlob)?;
        if ContentDigest::of(&bytes) != blob_ref.digest {
            return Err(ScratchError::BlobDigestMismatch(blob_ref.digest));
        }
        self.counters.blob_reads.fetch_add(1, Ordering::Relaxed);
        self.counters
            .blob_bytes_read
            .fetch_add(bytes.len(), Ordering::Relaxed);
        Ok(bytes)
    }

    pub(crate) fn append_page<T: Serialize>(
        &self,
        kind: ScratchPageKind,
        key_min: Vec<u8>,
        key_max: Vec<u8>,
        value: &T,
    ) -> Result<ScratchPageRef, ScratchError> {
        if key_min.is_empty() || key_min > key_max {
            return Err(ScratchError::MalformedPage);
        }
        let payload = encode_canonical(value)?;
        let envelope = ScratchPageEnvelope {
            schema_version: SCRATCH_PAGE_SCHEMA_VERSION,
            kind,
            key_min: key_min.clone(),
            key_max: key_max.clone(),
            payload,
        };
        let bytes = encode_canonical(&envelope)?;
        if bytes.len() > MAX_PAGE_BYTES {
            return Err(ScratchError::PageTooLarge(bytes.len()));
        }
        let digest = ContentDigest::of(&bytes);
        let encoded_len = u32::try_from(bytes.len()).map_err(|_| ScratchError::MalformedPage)?;
        let mut file = self.pages.lock().map_err(|_| ScratchError::Poisoned)?;
        let offset = file.seek(SeekFrom::End(0))?;
        file.write_all(&bytes)?;
        self.counters.page_writes.fetch_add(1, Ordering::Relaxed);
        self.counters
            .page_bytes_written
            .fetch_add(bytes.len(), Ordering::Relaxed);
        Ok(ScratchPageRef {
            offset,
            encoded_len,
            digest,
            kind,
            key_min,
            key_max,
        })
    }

    pub(crate) fn read_page<T: DeserializeOwned + Serialize>(
        &self,
        page_ref: &ScratchPageRef,
        expected_kind: ScratchPageKind,
    ) -> Result<T, ScratchError> {
        if page_ref.kind != expected_kind {
            return Err(ScratchError::PageBindingMismatch);
        }
        let length =
            usize::try_from(page_ref.encoded_len).map_err(|_| ScratchError::MalformedPage)?;
        if length == 0 || length > MAX_PAGE_BYTES {
            return Err(ScratchError::MalformedPage);
        }
        let mut bytes = vec![0_u8; length];
        let mut file = self.pages.lock().map_err(|_| ScratchError::Poisoned)?;
        file.seek(SeekFrom::Start(page_ref.offset))?;
        file.read_exact(&mut bytes)
            .map_err(|_| ScratchError::MalformedPage)?;
        if ContentDigest::of(&bytes) != page_ref.digest {
            return Err(ScratchError::PageDigestMismatch(page_ref.digest));
        }
        let envelope: ScratchPageEnvelope = decode_canonical(&bytes)?;
        if envelope.schema_version != SCRATCH_PAGE_SCHEMA_VERSION
            || envelope.kind != expected_kind
            || envelope.key_min != page_ref.key_min
            || envelope.key_max != page_ref.key_max
        {
            return Err(ScratchError::PageBindingMismatch);
        }
        self.counters.page_reads.fetch_add(1, Ordering::Relaxed);
        self.counters
            .page_bytes_read
            .fetch_add(bytes.len(), Ordering::Relaxed);
        self.counters
            .max_page_bytes_read
            .fetch_max(bytes.len(), Ordering::Relaxed);
        decode_canonical(&envelope.payload)
    }

    fn read_segment(
        &self,
        kind: ScratchPageKind,
        segment_ref: &ScratchSegmentRef,
    ) -> Result<ScratchSegment, ScratchError> {
        let segment: ScratchSegment = self.read_page(&segment_ref.page_ref, kind)?;
        validate_segment(&segment)?;
        if segment.kind != kind
            || segment.generation != segment_ref.generation
            || segment.entries.len() as u64 != segment_ref.entry_count
            || segment
                .entries
                .first()
                .is_none_or(|record| record.key != segment_ref.page_ref.key_min)
            || segment
                .entries
                .last()
                .is_none_or(|record| record.key != segment_ref.page_ref.key_max)
        {
            return Err(ScratchError::PageBindingMismatch);
        }
        Ok(segment)
    }

    #[allow(clippy::too_many_arguments)]
    fn append_accepted_sequence_at(
        &self,
        page_ref: &ScratchPageRef,
        height: u8,
        first_leaf: u64,
        leaf_index: u64,
        sequence: u64,
        batch_id: BatchId,
        evidence: Vec<u8>,
    ) -> Result<ScratchPageRef, ScratchError> {
        if height == 0 {
            let mut leaf = self.read_accepted_sequence_leaf(page_ref, first_leaf)?;
            if leaf.entries.len() >= ACCEPTED_SEQUENCE_LEAF_CAPACITY
                || sequence
                    != leaf
                        .first_sequence
                        .saturating_add(leaf.entries.len() as u64)
            {
                return Err(ScratchError::MalformedPage);
            }
            leaf.entries
                .push(AcceptedSequenceEntry { batch_id, evidence });
            return self.write_accepted_sequence_leaf(leaf.first_sequence, leaf.entries);
        }
        let mut node = self.read_accepted_sequence_node(page_ref, height, first_leaf)?;
        let child_capacity =
            accepted_sequence_leaf_capacity(height - 1).ok_or(ScratchError::IndexCapacity)?;
        let slot = usize::try_from((leaf_index - first_leaf) / child_capacity)
            .map_err(|_| ScratchError::MalformedPage)?;
        if slot >= ACCEPTED_SEQUENCE_NODE_FANOUT || slot > node.children.len() {
            return Err(ScratchError::MalformedPage);
        }
        let child_first = first_leaf
            .checked_add(
                u64::try_from(slot)
                    .map_err(|_| ScratchError::MalformedPage)?
                    .saturating_mul(child_capacity),
            )
            .ok_or(ScratchError::MalformedPage)?;
        let child = if slot == node.children.len() {
            self.build_accepted_sequence_path(
                height - 1,
                child_first,
                sequence,
                batch_id,
                evidence,
            )?
        } else {
            self.append_accepted_sequence_at(
                &node.children[slot],
                height - 1,
                child_first,
                leaf_index,
                sequence,
                batch_id,
                evidence,
            )?
        };
        if slot == node.children.len() {
            node.children.push(child);
        } else {
            node.children[slot] = child;
        }
        self.write_accepted_sequence_node(&node)
    }

    fn build_accepted_sequence_path(
        &self,
        height: u8,
        first_leaf: u64,
        sequence: u64,
        batch_id: BatchId,
        evidence: Vec<u8>,
    ) -> Result<ScratchPageRef, ScratchError> {
        if height == 0 {
            return self.write_accepted_sequence_leaf(
                sequence,
                vec![AcceptedSequenceEntry { batch_id, evidence }],
            );
        }
        let child = self.build_accepted_sequence_path(
            height - 1,
            first_leaf,
            sequence,
            batch_id,
            evidence,
        )?;
        self.write_accepted_sequence_node(&AcceptedSequenceNode {
            schema_version: ACCEPTED_SEQUENCE_SCHEMA_VERSION,
            height,
            first_leaf,
            children: vec![child],
        })
    }

    fn write_accepted_sequence_leaf(
        &self,
        first_sequence: u64,
        entries: Vec<AcceptedSequenceEntry>,
    ) -> Result<ScratchPageRef, ScratchError> {
        let leaf = AcceptedSequenceLeaf {
            schema_version: ACCEPTED_SEQUENCE_SCHEMA_VERSION,
            first_sequence,
            entries,
        };
        validate_accepted_sequence_leaf(&leaf)?;
        let last_sequence = first_sequence
            .checked_add(leaf.entries.len() as u64 - 1)
            .ok_or(ScratchError::MalformedPage)?;
        self.append_page(
            ScratchPageKind::AcceptedSequenceLeaf,
            first_sequence.to_be_bytes().to_vec(),
            last_sequence.to_be_bytes().to_vec(),
            &leaf,
        )
    }

    fn read_accepted_sequence_leaf(
        &self,
        page_ref: &ScratchPageRef,
        first_leaf: u64,
    ) -> Result<AcceptedSequenceLeaf, ScratchError> {
        let leaf: AcceptedSequenceLeaf =
            self.read_page(page_ref, ScratchPageKind::AcceptedSequenceLeaf)?;
        validate_accepted_sequence_leaf(&leaf)?;
        let expected_first = first_leaf
            .checked_mul(ACCEPTED_SEQUENCE_LEAF_CAPACITY as u64)
            .and_then(|value| value.checked_add(1))
            .ok_or(ScratchError::MalformedPage)?;
        let last = leaf
            .first_sequence
            .checked_add(leaf.entries.len() as u64 - 1)
            .ok_or(ScratchError::MalformedPage)?;
        if leaf.first_sequence != expected_first
            || page_ref.key_min != leaf.first_sequence.to_be_bytes()
            || page_ref.key_max != last.to_be_bytes()
        {
            return Err(ScratchError::PageBindingMismatch);
        }
        Ok(leaf)
    }

    fn write_accepted_sequence_node(
        &self,
        node: &AcceptedSequenceNode,
    ) -> Result<ScratchPageRef, ScratchError> {
        validate_accepted_sequence_node(node)?;
        let first_sequence = node
            .first_leaf
            .checked_mul(ACCEPTED_SEQUENCE_LEAF_CAPACITY as u64)
            .and_then(|value| value.checked_add(1))
            .ok_or(ScratchError::MalformedPage)?;
        let last_sequence = node
            .children
            .last()
            .and_then(|child| <[u8; 8]>::try_from(child.key_max.as_slice()).ok())
            .map(u64::from_be_bytes)
            .ok_or(ScratchError::MalformedPage)?;
        self.append_page(
            ScratchPageKind::AcceptedSequenceNode,
            first_sequence.to_be_bytes().to_vec(),
            last_sequence.to_be_bytes().to_vec(),
            node,
        )
    }

    fn read_accepted_sequence_node(
        &self,
        page_ref: &ScratchPageRef,
        height: u8,
        first_leaf: u64,
    ) -> Result<AcceptedSequenceNode, ScratchError> {
        let node: AcceptedSequenceNode =
            self.read_page(page_ref, ScratchPageKind::AcceptedSequenceNode)?;
        validate_accepted_sequence_node(&node)?;
        let first_sequence = first_leaf
            .checked_mul(ACCEPTED_SEQUENCE_LEAF_CAPACITY as u64)
            .and_then(|value| value.checked_add(1))
            .ok_or(ScratchError::MalformedPage)?;
        if node.height != height
            || node.first_leaf != first_leaf
            || page_ref.key_min != first_sequence.to_be_bytes()
            || page_ref.key_max
                != node
                    .children
                    .last()
                    .ok_or(ScratchError::MalformedPage)?
                    .key_max
        {
            return Err(ScratchError::PageBindingMismatch);
        }
        Ok(node)
    }

    fn write_authenticated_map_node(
        &self,
        kind: ScratchPageKind,
        node: &AuthenticatedMapNode,
    ) -> Result<AuthenticatedMapChild, ScratchError> {
        validate_authenticated_map_node(node)?;
        let digest = authenticated_map_node_digest(
            node.key,
            node.value_digest,
            node.left.as_ref().map(|child| (child.key, child.digest)),
            node.right.as_ref().map(|child| (child.key, child.digest)),
        );
        let key = node.key.to_vec();
        let page_ref = self.append_page(kind, key.clone(), key, node)?;
        Ok(AuthenticatedMapChild {
            key: node.key,
            digest,
            page_ref,
        })
    }

    fn read_authenticated_map_node(
        &self,
        kind: ScratchPageKind,
        child: &AuthenticatedMapChild,
    ) -> Result<AuthenticatedMapNode, ScratchError> {
        let node: AuthenticatedMapNode = self.read_page(&child.page_ref, kind)?;
        validate_authenticated_map_node(&node)?;
        if node.key != child.key
            || child.page_ref.key_min != child.key
            || child.page_ref.key_max != child.key
            || authenticated_map_node_digest(
                node.key,
                node.value_digest,
                node.left
                    .as_ref()
                    .map(|candidate| (candidate.key, candidate.digest)),
                node.right
                    .as_ref()
                    .map(|candidate| (candidate.key, candidate.digest)),
            ) != child.digest
        {
            return Err(ScratchError::PageBindingMismatch);
        }
        Ok(node)
    }

    fn reclaim_stale_runs(&self) -> Result<(), ScratchError> {
        for entry in self.namespace.entries()? {
            let entry = entry?;
            let name = entry
                .file_name()
                .to_str()
                .ok_or_else(|| ScratchError::UnsafeEntry("non-UTF-8 scratch run".into()))?
                .to_owned();
            let run_id = parse_run_name(&name)?;
            require_real_directory(&entry, &name)?;
            if name == self.run_name {
                continue;
            }
            let run = super::object_store::open_dir_nofollow(&self.namespace, &name)?;
            let marker_bytes = read_regular_nofollow(&run, MARKER_FILE, MAX_MARKER_BYTES)?;
            let marker: ScratchRunMarkerV2 = decode_canonical(&marker_bytes)?;
            if marker.schema_version != SCRATCH_SCHEMA_VERSION
                || marker.workspace_id != self.marker.workspace_id
                || marker.run_id != run_id
            {
                return Err(ScratchError::MalformedMarker(name));
            }
            validate_run_entries(&run)?;
            let lease = open_regular_read_write_nofollow(&run, LEASE_FILE)?;
            if !lock_exclusive_nonblocking(&lease)? {
                self.counters
                    .live_runs_skipped
                    .fetch_add(1, Ordering::Relaxed);
                continue;
            }
            remove_stale_run(&self.namespace, &run, &name, lease)?;
            self.counters
                .stale_runs_reclaimed
                .fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    fn cleanup_own_run(&self) {
        for name in [PAGES_FILE, BLOBS_FILE, MARKER_FILE] {
            let _ = self.run.remove_file(name);
        }
        unlock(&self.lease);
        let _ = self.run.remove_file(LEASE_FILE);
        let _ = self.namespace.remove_dir(&self.run_name);
    }
}

impl Drop for ScratchStore {
    fn drop(&mut self) {
        self.cleanup_own_run();
    }
}

struct AcceptedSequenceCursorFrame {
    node: AcceptedSequenceNode,
    next_child: usize,
}

pub(crate) struct ScratchAcceptedSequenceCursor<'a> {
    store: &'a ScratchStore,
    root: &'a ScratchAcceptedSequenceRoot,
    stack: Vec<AcceptedSequenceCursorFrame>,
    leaf: Option<(AcceptedSequenceLeaf, usize)>,
    next_sequence: u64,
    initialized: bool,
    page_reads: usize,
    page_bytes_read: usize,
    max_page_bytes_read: usize,
}

impl ScratchAcceptedSequenceCursor<'_> {
    pub(crate) const fn page_stats(&self) -> (usize, usize, usize) {
        (
            self.page_reads,
            self.page_bytes_read,
            self.max_page_bytes_read,
        )
    }

    pub(crate) fn next_batch(
        &mut self,
    ) -> Result<Option<(u64, AcceptedSequenceEntry)>, ScratchError> {
        if self.next_sequence > self.root.len {
            return Ok(None);
        }
        if !self.initialized {
            self.initialized = true;
            let root = self.root.root.clone().ok_or(ScratchError::MalformedPage)?;
            self.descend_left(root, self.root.height, 0)?;
        }
        loop {
            if let Some((leaf, index)) = &mut self.leaf {
                if let Some(entry) = leaf.entries.get(*index).cloned() {
                    let sequence = self.next_sequence;
                    if sequence
                        != leaf
                            .first_sequence
                            .checked_add(*index as u64)
                            .ok_or(ScratchError::MalformedPage)?
                    {
                        return Err(ScratchError::MalformedPage);
                    }
                    *index += 1;
                    self.next_sequence += 1;
                    return Ok(Some((sequence, entry)));
                }
                self.leaf = None;
            }
            let mut next = None;
            while let Some(frame) = self.stack.last_mut() {
                if frame.next_child < frame.node.children.len() {
                    let slot = frame.next_child;
                    frame.next_child += 1;
                    let child_capacity = accepted_sequence_leaf_capacity(frame.node.height - 1)
                        .ok_or(ScratchError::IndexCapacity)?;
                    let first_leaf = frame
                        .node
                        .first_leaf
                        .checked_add(
                            u64::try_from(slot)
                                .map_err(|_| ScratchError::MalformedPage)?
                                .saturating_mul(child_capacity),
                        )
                        .ok_or(ScratchError::MalformedPage)?;
                    next = Some((
                        frame.node.children[slot].clone(),
                        frame.node.height - 1,
                        first_leaf,
                    ));
                    break;
                }
                self.stack.pop();
            }
            let Some((page_ref, height, first_leaf)) = next else {
                return Err(ScratchError::MalformedPage);
            };
            self.descend_left(page_ref, height, first_leaf)?;
        }
    }

    fn descend_left(
        &mut self,
        mut page_ref: ScratchPageRef,
        mut height: u8,
        mut first_leaf: u64,
    ) -> Result<(), ScratchError> {
        while height > 0 {
            self.record_page_read(&page_ref);
            let node = self
                .store
                .read_accepted_sequence_node(&page_ref, height, first_leaf)?;
            let child = node
                .children
                .first()
                .cloned()
                .ok_or(ScratchError::MalformedPage)?;
            self.stack.push(AcceptedSequenceCursorFrame {
                node,
                next_child: 1,
            });
            page_ref = child;
            height -= 1;
            first_leaf = self
                .stack
                .last()
                .expect("pushed accepted sequence frame")
                .node
                .first_leaf;
        }
        self.leaf = Some((
            {
                self.record_page_read(&page_ref);
                self.store
                    .read_accepted_sequence_leaf(&page_ref, first_leaf)?
            },
            0,
        ));
        Ok(())
    }

    fn record_page_read(&mut self, page_ref: &ScratchPageRef) {
        let length = page_ref.encoded_len as usize;
        self.page_reads = self.page_reads.saturating_add(1);
        self.page_bytes_read = self.page_bytes_read.saturating_add(length);
        self.max_page_bytes_read = self.max_page_bytes_read.max(length);
    }
}

pub(crate) fn authenticated_map_empty_digest() -> ContentDigest {
    ContentDigest::of(b"tine/oplog/authenticated-map/v1/empty")
}

pub(crate) fn authenticated_map_priority(key: [u8; 16]) -> ContentDigest {
    let mut bytes = b"tine/oplog/authenticated-map/v1/priority\0".to_vec();
    bytes.extend_from_slice(&key);
    ContentDigest::of(&bytes)
}

pub(crate) fn authenticated_map_node_digest(
    key: [u8; 16],
    value_digest: ContentDigest,
    left: Option<([u8; 16], ContentDigest)>,
    right: Option<([u8; 16], ContentDigest)>,
) -> ContentDigest {
    let mut bytes = b"tine/oplog/authenticated-map/v1/node\0".to_vec();
    bytes.extend_from_slice(&key);
    bytes.extend_from_slice(value_digest.as_bytes());
    for child in [left, right] {
        match child {
            Some((child_key, digest)) => {
                bytes.push(1);
                bytes.extend_from_slice(&child_key);
                bytes.extend_from_slice(digest.as_bytes());
            }
            None => bytes.push(0),
        }
    }
    ContentDigest::of(&bytes)
}

pub(crate) fn authenticated_map_priority_order(
    left: [u8; 16],
    right: [u8; 16],
) -> std::cmp::Ordering {
    authenticated_map_priority(left)
        .as_bytes()
        .cmp(authenticated_map_priority(right).as_bytes())
        .then_with(|| left.cmp(&right))
}

fn accepted_sequence_leaf_capacity(height: u8) -> Option<u64> {
    let mut capacity = 1_u64;
    for _ in 0..height {
        capacity = capacity.checked_mul(ACCEPTED_SEQUENCE_NODE_FANOUT as u64)?;
    }
    Some(capacity)
}

fn validate_accepted_sequence_root(root: &ScratchAcceptedSequenceRoot) -> Result<(), ScratchError> {
    if root.schema_version != ACCEPTED_SEQUENCE_SCHEMA_VERSION
        || (root.len == 0) != root.root.is_none()
        || (root.len == 0 && root.height != 0)
    {
        return Err(ScratchError::MalformedPage);
    }
    if root.len > 0 {
        let leaf_count = root
            .len
            .saturating_add(ACCEPTED_SEQUENCE_LEAF_CAPACITY as u64 - 1)
            / ACCEPTED_SEQUENCE_LEAF_CAPACITY as u64;
        let capacity =
            accepted_sequence_leaf_capacity(root.height).ok_or(ScratchError::IndexCapacity)?;
        if leaf_count == 0
            || leaf_count > capacity
            || (root.height > 0
                && leaf_count
                    <= accepted_sequence_leaf_capacity(root.height - 1)
                        .ok_or(ScratchError::IndexCapacity)?)
        {
            return Err(ScratchError::MalformedPage);
        }
    }
    Ok(())
}

fn validate_accepted_sequence_leaf(leaf: &AcceptedSequenceLeaf) -> Result<(), ScratchError> {
    if leaf.schema_version != ACCEPTED_SEQUENCE_SCHEMA_VERSION
        || leaf.first_sequence == 0
        || leaf.entries.is_empty()
        || leaf.entries.len() > ACCEPTED_SEQUENCE_LEAF_CAPACITY
        || leaf.entries.iter().any(|entry| entry.evidence.is_empty())
        || !(leaf.first_sequence - 1).is_multiple_of(ACCEPTED_SEQUENCE_LEAF_CAPACITY as u64)
    {
        return Err(ScratchError::MalformedPage);
    }
    Ok(())
}

fn validate_accepted_sequence_node(node: &AcceptedSequenceNode) -> Result<(), ScratchError> {
    if node.schema_version != ACCEPTED_SEQUENCE_SCHEMA_VERSION
        || node.height == 0
        || node.children.is_empty()
        || node.children.len() > ACCEPTED_SEQUENCE_NODE_FANOUT
    {
        return Err(ScratchError::MalformedPage);
    }
    let child_capacity =
        accepted_sequence_leaf_capacity(node.height - 1).ok_or(ScratchError::IndexCapacity)?;
    for (index, child) in node.children.iter().enumerate() {
        let child_first_leaf = node
            .first_leaf
            .checked_add(
                u64::try_from(index)
                    .map_err(|_| ScratchError::MalformedPage)?
                    .saturating_mul(child_capacity),
            )
            .ok_or(ScratchError::MalformedPage)?;
        let expected_first = child_first_leaf
            .checked_mul(ACCEPTED_SEQUENCE_LEAF_CAPACITY as u64)
            .and_then(|value| value.checked_add(1))
            .ok_or(ScratchError::MalformedPage)?;
        if child.key_min != expected_first.to_be_bytes()
            || child.key_max.len() != std::mem::size_of::<u64>()
        {
            return Err(ScratchError::PageBindingMismatch);
        }
    }
    Ok(())
}

fn validate_authenticated_map_root(root: &ScratchAuthenticatedMapRoot) -> Result<(), ScratchError> {
    if root.schema_version != AUTHENTICATED_MAP_SCHEMA_VERSION
        || (root.count == 0)
            != (root.root.is_none()
                && root.root_key.is_none()
                && root.root_digest == authenticated_map_empty_digest())
        || (root.count > 0 && (root.root.is_none() || root.root_key.is_none()))
    {
        return Err(ScratchError::MalformedPage);
    }
    Ok(())
}

fn validate_authenticated_map_node(node: &AuthenticatedMapNode) -> Result<(), ScratchError> {
    if node.schema_version != AUTHENTICATED_MAP_SCHEMA_VERSION
        || node.priority != authenticated_map_priority(node.key)
        || node.left.as_ref().is_some_and(|left| {
            left.key >= node.key || !authenticated_map_priority_order(node.key, left.key).is_lt()
        })
        || node.right.as_ref().is_some_and(|right| {
            right.key <= node.key || !authenticated_map_priority_order(node.key, right.key).is_lt()
        })
    {
        return Err(ScratchError::MalformedPage);
    }
    Ok(())
}

fn validate_root(root: &ScratchLsmRoot) -> Result<(), ScratchError> {
    if root.levels.len() != SCRATCH_LSM_LEVELS {
        return Err(ScratchError::MalformedPage);
    }
    for segment in root.levels.iter().flatten() {
        if segment.generation == 0
            || segment.generation > root.next_generation
            || segment.entry_count == 0
        {
            return Err(ScratchError::MalformedPage);
        }
    }
    Ok(())
}

fn validate_segment(segment: &ScratchSegment) -> Result<(), ScratchError> {
    if segment.schema_version != SCRATCH_PAGE_SCHEMA_VERSION
        || segment.generation == 0
        || segment.entries.is_empty()
    {
        return Err(ScratchError::MalformedPage);
    }
    let mut previous: Option<&[u8]> = None;
    for record in &segment.entries {
        if record.key.is_empty()
            || previous.is_some_and(|previous| previous >= record.key.as_slice())
        {
            return Err(ScratchError::MalformedPage);
        }
        previous = Some(&record.key);
    }
    Ok(())
}

fn parse_run_name(name: &str) -> Result<Uuid, ScratchError> {
    let suffix = name
        .strip_prefix("run-")
        .ok_or_else(|| ScratchError::UnsafeEntry(format!("unknown scratch entry {name:?}")))?;
    let run_id = Uuid::parse_str(suffix)
        .map_err(|_| ScratchError::UnsafeEntry(format!("malformed scratch run {name:?}")))?;
    if format!("run-{run_id}") != name {
        return Err(ScratchError::UnsafeEntry(format!(
            "non-canonical scratch run {name:?}"
        )));
    }
    Ok(run_id)
}

fn validate_run_entries(run: &Dir) -> Result<(), ScratchError> {
    let mut seen = BTreeMap::new();
    for entry in run.entries()? {
        let entry = entry?;
        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| ScratchError::UnsafeEntry("non-UTF-8 scratch entry".into()))?
            .to_owned();
        if ![MARKER_FILE, LEASE_FILE, PAGES_FILE, BLOBS_FILE].contains(&name.as_str()) {
            return Err(ScratchError::UnsafeEntry(format!(
                "unknown scratch run entry {name:?}"
            )));
        }
        require_regular_entry(&entry, &name)?;
        if seen.insert(name.clone(), ()).is_some() {
            return Err(ScratchError::UnsafeEntry(format!(
                "duplicate scratch run entry {name:?}"
            )));
        }
    }
    for required in [MARKER_FILE, LEASE_FILE, PAGES_FILE, BLOBS_FILE] {
        if !seen.contains_key(required) {
            return Err(ScratchError::UnsafeEntry(format!(
                "scratch run is missing {required:?}"
            )));
        }
    }
    Ok(())
}

fn remove_stale_run(
    namespace: &Dir,
    run: &Dir,
    run_name: &str,
    lease: fs::File,
) -> Result<(), ScratchError> {
    // Validate the complete entry set before unlinking anything. No recursive
    // ambient deletion is used and no authoritative namespace is reachable.
    validate_run_entries(run)?;
    for name in [PAGES_FILE, BLOBS_FILE, MARKER_FILE] {
        run.remove_file(name)?;
    }
    unlock(&lease);
    drop(lease);
    run.remove_file(LEASE_FILE)?;
    namespace.remove_dir(run_name)?;
    Ok(())
}

fn create_new_regular(dir: &Dir, name: &str) -> Result<fs::File, ScratchError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    let file = dir.open_with(name, &options)?.into_std();
    ensure_opened_regular(&file, name)?;
    Ok(file)
}

fn write_new_regular(dir: &Dir, name: &str, bytes: &[u8]) -> Result<(), ScratchError> {
    let mut file = create_new_regular(dir, name)?;
    file.write_all(bytes)?;
    Ok(())
}

fn open_regular_read_write_nofollow(dir: &Dir, name: &str) -> Result<fs::File, ScratchError> {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::fd::AsFd as _;
        let name = CString::new(name)
            .map_err(|_| ScratchError::UnsafeEntry("invalid scratch filename".into()))?;
        // SAFETY: the path is a live C string and dirfd is an opened capability.
        let fd = unsafe {
            libc::openat(
                dir.as_fd().as_raw_fd(),
                name.as_ptr(),
                libc::O_RDWR | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        // SAFETY: a successful openat returned an owned descriptor.
        let file = unsafe { fs::File::from_raw_fd(fd) };
        ensure_opened_regular(&file, LEASE_FILE)?;
        Ok(file)
    }
    #[cfg(windows)]
    {
        use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _};
        let mut options = OpenOptions::new();
        options.read(true).write(true);
        options.follow(FollowSymlinks::No);
        let file = dir.open_with(name, &options)?.into_std();
        ensure_opened_regular(&file, name)?;
        return Ok(file);
    }
}

fn read_regular_nofollow(dir: &Dir, name: &str, limit: u64) -> Result<Vec<u8>, ScratchError> {
    let mut file = open_regular_read_write_nofollow(dir, name)?;
    let metadata = file.metadata()?;
    if metadata.len() > limit {
        return Err(ScratchError::UnsafeEntry(format!(
            "scratch file {name:?} exceeds its bound"
        )));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn ensure_opened_regular(file: &fs::File, name: &str) -> Result<(), ScratchError> {
    let metadata = file.metadata()?;
    #[cfg(windows)]
    {
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(ScratchError::UnsafeEntry(format!(
                "{name:?} is a reparse point"
            )));
        }
    }
    if !metadata.is_file() {
        return Err(ScratchError::UnsafeEntry(format!(
            "{name:?} is not a regular file"
        )));
    }
    Ok(())
}

fn require_real_directory(entry: &cap_std::fs::DirEntry, name: &str) -> Result<(), ScratchError> {
    let file_type = entry.file_type()?;
    if file_type.is_symlink() || !file_type.is_dir() {
        return Err(ScratchError::UnsafeEntry(format!(
            "{name:?} is not a real directory"
        )));
    }
    #[cfg(windows)]
    {
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        if entry.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(ScratchError::UnsafeEntry(format!(
                "{name:?} is a reparse point"
            )));
        }
    }
    Ok(())
}

fn require_regular_entry(entry: &cap_std::fs::DirEntry, name: &str) -> Result<(), ScratchError> {
    let file_type = entry.file_type()?;
    if file_type.is_symlink() || !file_type.is_file() {
        return Err(ScratchError::UnsafeEntry(format!(
            "{name:?} is not a regular file"
        )));
    }
    #[cfg(windows)]
    {
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        if entry.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(ScratchError::UnsafeEntry(format!(
                "{name:?} is a reparse point"
            )));
        }
    }
    Ok(())
}

fn encode_canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, ScratchError> {
    postcard::to_allocvec(value).map_err(|_| ScratchError::MalformedPage)
}

fn decode_canonical<T: DeserializeOwned + Serialize>(bytes: &[u8]) -> Result<T, ScratchError> {
    let value: T = postcard::from_bytes(bytes).map_err(|_| ScratchError::MalformedPage)?;
    if encode_canonical(&value)? != bytes {
        return Err(ScratchError::MalformedPage);
    }
    Ok(value)
}

#[cfg(unix)]
fn lock_exclusive_nonblocking(file: &fs::File) -> Result<bool, ScratchError> {
    // SAFETY: flock only observes the live owned descriptor.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
        return Ok(false);
    }
    Err(error.into())
}

#[cfg(unix)]
fn unlock(file: &fs::File) {
    // SAFETY: flock only observes the live owned descriptor.
    let _ = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
}

#[cfg(windows)]
fn lock_exclusive_nonblocking(file: &fs::File) -> Result<bool, ScratchError> {
    use windows_sys::Win32::Foundation::{ERROR_LOCK_VIOLATION, FALSE};
    use windows_sys::Win32::Storage::FileSystem::{
        LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
    };
    let mut overlapped = unsafe { std::mem::zeroed() };
    // SAFETY: the handle and OVERLAPPED remain live for the synchronous call.
    let result = unsafe {
        LockFileEx(
            file.as_raw_handle() as _,
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        )
    };
    if result != FALSE {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32) {
        return Ok(false);
    }
    Err(error.into())
}

#[cfg(windows)]
fn unlock(file: &fs::File) {
    use windows_sys::Win32::Storage::FileSystem::UnlockFileEx;
    let mut overlapped = unsafe { std::mem::zeroed() };
    // SAFETY: the handle and OVERLAPPED remain live for the synchronous call.
    let _ = unsafe {
        UnlockFileEx(
            file.as_raw_handle() as _,
            0,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        )
    };
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ScratchError {
    Io(String),
    UnsafeEntry(String),
    MalformedMarker(String),
    MalformedPage,
    MalformedBlob,
    PageTooLarge(usize),
    PageDigestMismatch(ContentDigest),
    BlobDigestMismatch(ContentDigest),
    PageBindingMismatch,
    IndexCapacity,
    Poisoned,
}

impl fmt::Display for ScratchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "scratch I/O failed: {error}"),
            Self::UnsafeEntry(reason) => write!(f, "unsafe scratch entry: {reason}"),
            Self::MalformedMarker(run) => write!(f, "malformed scratch marker in {run}"),
            Self::MalformedPage => write!(f, "malformed or non-canonical scratch page"),
            Self::MalformedBlob => write!(f, "malformed scratch blob"),
            Self::PageTooLarge(length) => write!(f, "scratch page is too large: {length} bytes"),
            Self::PageDigestMismatch(digest) => {
                write!(f, "scratch page digest mismatch for {digest}")
            }
            Self::BlobDigestMismatch(digest) => {
                write!(f, "scratch blob digest mismatch for {digest}")
            }
            Self::PageBindingMismatch => write!(f, "scratch page reference is misbound"),
            Self::IndexCapacity => write!(f, "scratch LSM exceeded its fixed level capacity"),
            Self::Poisoned => write!(f, "scratch file lock was poisoned"),
        }
    }
}

impl std::error::Error for ScratchError {}

impl From<std::io::Error> for ScratchError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

impl From<super::object_store::StoreError> for ScratchError {
    fn from(error: super::object_store::StoreError) -> Self {
        Self::Io(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cap_std::ambient_authority;
    use std::path::Path;

    fn workspace(value: u128) -> WorkspaceId {
        WorkspaceId::from_uuid(Uuid::from_u128(value))
    }

    fn archive(root: &Path) -> Dir {
        fs::create_dir_all(root).unwrap();
        Dir::open_ambient_dir(root, ambient_authority()).unwrap()
    }

    #[test]
    fn authenticated_lsm_is_canonical_and_newest_wins() {
        let path = std::env::temp_dir().join(format!("tine-scratch-lsm-{}", Uuid::new_v4()));
        let archive = archive(&path);
        let store = ScratchStore::open(&archive, workspace(1)).unwrap();
        let mut root = ScratchLsmRoot::default();
        root = store
            .insert_many(
                &root,
                ScratchPageKind::BatchStatus,
                &BTreeMap::from([
                    (b"a".to_vec(), Some(b"one".to_vec())),
                    (b"b".to_vec(), Some(b"two".to_vec())),
                ]),
            )
            .unwrap();
        root = store
            .insert_many(
                &root,
                ScratchPageKind::BatchStatus,
                &BTreeMap::from([
                    (b"a".to_vec(), Some(b"new".to_vec())),
                    (b"b".to_vec(), None),
                ]),
            )
            .unwrap();
        assert_eq!(
            store
                .lookup(&root, ScratchPageKind::BatchStatus, b"a")
                .unwrap(),
            Some(b"new".to_vec())
        );
        assert_eq!(
            store
                .lookup(&root, ScratchPageKind::BatchStatus, b"b")
                .unwrap(),
            None
        );
        assert_eq!(
            store
                .scan_prefix(&root, ScratchPageKind::BatchStatus, b"")
                .unwrap(),
            vec![(b"a".to_vec(), b"new".to_vec())]
        );
        assert_eq!(store.stats().scratch_syncs, 0);
        drop(store);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn live_lease_survives_another_open_and_drop_reclaims_own_run() {
        let path = std::env::temp_dir().join(format!("tine-scratch-lease-{}", Uuid::new_v4()));
        let archive = archive(&path);
        let first = ScratchStore::open(&archive, workspace(2)).unwrap();
        let first_name = first.run_name.clone();
        let second = ScratchStore::open(&archive, workspace(2)).unwrap();
        assert!(second.stats().live_runs_skipped >= 1);
        assert!(path.join(SCRATCH_DIR).join(&first_name).is_dir());
        drop(second);
        assert!(path.join(SCRATCH_DIR).join(&first_name).is_dir());
        drop(first);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn restart_reclaims_an_authenticated_stale_run_without_syncing() {
        let path = std::env::temp_dir().join(format!("tine-scratch-stale-{}", Uuid::new_v4()));
        let archive = archive(&path);
        let first = ScratchStore::open(&archive, workspace(4)).unwrap();
        let run_name = first.run_name.clone();
        let marker = first.marker.clone();
        drop(first);
        let run_path = path.join(SCRATCH_DIR).join(&run_name);
        fs::create_dir(&run_path).unwrap();
        fs::write(
            run_path.join(MARKER_FILE),
            encode_canonical(&marker).unwrap(),
        )
        .unwrap();
        for name in [LEASE_FILE, PAGES_FILE, BLOBS_FILE] {
            fs::write(run_path.join(name), []).unwrap();
        }
        let restarted = ScratchStore::open(&archive, workspace(4)).unwrap();
        assert_eq!(restarted.stats().stale_runs_reclaimed, 1);
        assert_eq!(restarted.stats().scratch_syncs, 0);
        assert!(!run_path.exists());
        drop(restarted);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn restart_fails_closed_on_tampered_marker() {
        let path = std::env::temp_dir().join(format!("tine-scratch-marker-{}", Uuid::new_v4()));
        let archive = archive(&path);
        let first = ScratchStore::open(&archive, workspace(5)).unwrap();
        let run_name = first.run_name.clone();
        drop(first);
        let run_path = path.join(SCRATCH_DIR).join(run_name);
        fs::create_dir(&run_path).unwrap();
        fs::write(run_path.join(MARKER_FILE), b"tampered").unwrap();
        for name in [LEASE_FILE, PAGES_FILE, BLOBS_FILE] {
            fs::write(run_path.join(name), []).unwrap();
        }
        assert!(ScratchStore::open(&archive, workspace(5)).is_err());
        assert!(run_path.exists());
        fs::remove_dir_all(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_refuses_symlink_entries_without_following_them() {
        use std::os::unix::fs::symlink;
        let path = std::env::temp_dir().join(format!("tine-scratch-link-{}", Uuid::new_v4()));
        let archive = archive(&path);
        let first = ScratchStore::open(&archive, workspace(3)).unwrap();
        let run_path = path.join(SCRATCH_DIR).join(&first.run_name);
        drop(first);
        fs::create_dir(&run_path).unwrap();
        symlink("/tmp", run_path.join("marker")).unwrap();
        fs::write(run_path.join("lease"), []).unwrap();
        fs::write(run_path.join("pages.index"), []).unwrap();
        fs::write(run_path.join("blobs.data"), []).unwrap();
        assert!(ScratchStore::open(&archive, workspace(3)).is_err());
        assert!(Path::new("/tmp").is_dir());
        fs::remove_dir_all(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_refuses_special_entries_without_unlinking_them() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt as _;

        let path = std::env::temp_dir().join(format!("tine-scratch-fifo-{}", Uuid::new_v4()));
        let archive = archive(&path);
        let first = ScratchStore::open(&archive, workspace(6)).unwrap();
        let run_name = first.run_name.clone();
        let marker = first.marker.clone();
        drop(first);
        let run_path = path.join(SCRATCH_DIR).join(run_name);
        fs::create_dir(&run_path).unwrap();
        fs::write(
            run_path.join(MARKER_FILE),
            encode_canonical(&marker).unwrap(),
        )
        .unwrap();
        fs::write(run_path.join(LEASE_FILE), []).unwrap();
        fs::write(run_path.join(BLOBS_FILE), []).unwrap();
        let fifo = run_path.join(PAGES_FILE);
        let fifo_c = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: `fifo_c` is a live NUL-terminated path in this test directory.
        assert_eq!(unsafe { libc::mkfifo(fifo_c.as_ptr(), 0o600) }, 0);
        assert!(ScratchStore::open(&archive, workspace(6)).is_err());
        assert!(fifo.exists());
        fs::remove_dir_all(path).unwrap();
    }
}
