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

use super::{ContentDigest, WorkspaceId};

pub(crate) const SCRATCH_DIR: &str = "engine-scratch-v2";
const MARKER_FILE: &str = "marker";
const LEASE_FILE: &str = "lease";
const PAGES_FILE: &str = "pages.index";
const BLOBS_FILE: &str = "blobs.data";
const SCRATCH_SCHEMA_VERSION: u32 = 4;
const SCRATCH_PAGE_SCHEMA_VERSION: u32 = 1;
const SCRATCH_LSM_LEVELS: usize = 32;
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
    AcceptedEvidence = 15,
    AcceptedFrontier = 16,
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
    pub accepted_evidence_root: ScratchLsmRoot,
    pub accepted_frontier_root: ScratchLsmRoot,
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
