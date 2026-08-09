use std::collections::{BTreeMap, BTreeSet};
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
use std::sync::Mutex;

#[cfg(windows)]
use cap_fs_ext::OsMetadataExt as _;
use cap_std::fs::{Dir, OpenOptions};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    ensure_directory_nofollow, open_dir_nofollow, sync_dir_required, ContentDigest, FilesystemError,
};

pub const SCRATCH_DIR: &str = "engine-scratch-v2";
pub const SCRATCH_MARKER_FILE: &str = "marker";
pub const SCRATCH_LEASE_FILE: &str = "lease";
pub const SCRATCH_PAGES_FILE: &str = "pages.index";
pub const SCRATCH_BLOBS_FILE: &str = "blobs.data";
pub const SCRATCH_SCHEMA_VERSION: u32 = 13;
pub const SCRATCH_PAGE_SCHEMA_VERSION: u32 = 1;
pub const SCRATCH_LSM_LEVELS: usize = 32;
pub const MAX_SCRATCH_PAGE_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_SCRATCH_BLOB_BYTES: usize = 256 * 1024 * 1024;

const MAX_MARKER_BYTES: u64 = 4 * 1024;
const SCRATCH_APPEND_BUFFER_BYTES: usize = 1024 * 1024;

/// Durable retention mode authenticated by a scratch run's marker.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ScratchRetention {
    Ephemeral,
    Retained,
}

/// Generic schema-13 marker. Field order and representations are persistent.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScratchRunMarker<Owner> {
    schema_version: u32,
    owner: Owner,
    run_id: Uuid,
    retention: ScratchRetention,
    random_owner_nonce: [u8; 32],
}

/// A failure at the generic physical scratch-run boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScratchRunError {
    Io(String),
    UnsafeEntry(String),
    MalformedMarker(String),
    MalformedEncoding,
    MalformedPage,
    MalformedBlob,
    PageTooLarge(usize),
    PageDigestMismatch(ContentDigest),
    BlobDigestMismatch(ContentDigest),
    PageBindingMismatch,
    IndexCapacity,
    Poisoned,
}

impl fmt::Display for ScratchRunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "scratch I/O failed: {error}"),
            Self::UnsafeEntry(reason) => write!(f, "unsafe scratch entry: {reason}"),
            Self::MalformedMarker(run) => write!(f, "malformed scratch marker in {run}"),
            Self::MalformedEncoding => f.write_str("malformed or non-canonical scratch page"),
            Self::MalformedPage => f.write_str("malformed or non-canonical scratch page"),
            Self::MalformedBlob => f.write_str("malformed scratch blob"),
            Self::PageTooLarge(length) => {
                write!(f, "scratch page is too large: {length} bytes")
            }
            Self::PageDigestMismatch(digest) => {
                write!(f, "scratch page digest mismatch for {digest}")
            }
            Self::BlobDigestMismatch(digest) => {
                write!(f, "scratch blob digest mismatch for {digest}")
            }
            Self::PageBindingMismatch => f.write_str("scratch page reference is misbound"),
            Self::IndexCapacity => f.write_str("scratch index exceeded its fixed capacity"),
            Self::Poisoned => f.write_str("scratch file lock was poisoned"),
        }
    }
}

impl std::error::Error for ScratchRunError {}

impl From<std::io::Error> for ScratchRunError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

impl From<FilesystemError> for ScratchRunError {
    fn from(error: FilesystemError) -> Self {
        let error = match error {
            FilesystemError::Io(error) => error.to_string(),
            FilesystemError::UnsafeEntry(message) => format!("unsafe store entry: {message}"),
            FilesystemError::StoredLengthMismatch {
                path,
                expected,
                actual,
            } => format!(
                "stored file length mismatch at {path}: expected {expected}, found {actual}"
            ),
            FilesystemError::StoredFileTooLarge {
                path,
                length,
                limit,
            } => format!("stored file at {path} is {length} bytes, exceeding limit {limit}"),
            FilesystemError::ByteCollision => "immutable immutable publication collision".into(),
        };
        Self::Io(error)
    }
}

/// Fallible physical boundaries observable by a core-owned test fault policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScratchConstructionBoundary {
    AfterRunDirectory,
    AfterNamespaceSync,
    AfterRunOpen,
    AfterMarkerWrite,
    AfterLeaseCreate,
    AfterLeaseLock,
    AfterPagesCreate,
    AfterBlobsCreate,
    InspectSibling,
    AfterReclaim,
}

/// Counts from opportunistic cleanup performed while creating a fresh run.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScratchRunLifecycleStats {
    pub stale_runs_reclaimed: usize,
    pub live_runs_skipped: usize,
    pub retained_runs_preserved: usize,
    pub unclassified_runs_preserved: usize,
}

/// Generic operation counts for one physical scratch run.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScratchOperationStats {
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
    /// Physical writes used to publish the logical append-only page records.
    #[cfg(any(test, feature = "test-support"))]
    pub page_append_batches: usize,
    /// Physical writes used to publish the logical append-only blob records.
    #[cfg(any(test, feature = "test-support"))]
    pub blob_append_batches: usize,
}

/// Count-only diagnostics for one process-local authenticated LSM lookup
/// session.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScratchLookupSessionStats {
    pub hits: usize,
    pub misses: usize,
    pub evictions: usize,
    pub oversize: usize,
    pub resident_bytes: usize,
    pub peak_resident_bytes: usize,
}

#[derive(Debug, Default)]
struct ScratchOperationCounters {
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
    // Deliberately has no increment site. Any future scratch sync must become
    // visible to normal-flow regression gates.
    scratch_syncs: AtomicUsize,
}

impl ScratchOperationCounters {
    fn snapshot(&self) -> ScratchOperationStats {
        ScratchOperationStats {
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
            #[cfg(any(test, feature = "test-support"))]
            page_append_batches: 0,
            #[cfg(any(test, feature = "test-support"))]
            blob_append_batches: 0,
        }
    }
}

/// A caller-owned page-kind tag serialized verbatim into scratch envelopes.
pub trait ScratchPageTag: Copy + Eq + Serialize + DeserializeOwned {
    /// The caller's widest tag for saturation-only encoded-root bound proofs.
    #[doc(hidden)]
    fn saturation_tag() -> Self;
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScratchPageRef<Tag> {
    pub offset: u64,
    pub encoded_len: u32,
    pub digest: ContentDigest,
    pub kind: Tag,
    pub key_min: Vec<u8>,
    pub key_max: Vec<u8>,
}

impl<Tag> ScratchPageRef<Tag> {
    pub fn key_min(&self) -> &[u8] {
        &self.key_min
    }

    pub fn key_max(&self) -> &[u8] {
        &self.key_max
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScratchPageEnvelope<Tag> {
    schema_version: u32,
    kind: Tag,
    key_min: Vec<u8>,
    key_max: Vec<u8>,
    payload: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScratchBlobRef {
    pub offset: u64,
    pub encoded_len: u32,
    pub digest: ContentDigest,
}

impl ScratchBlobRef {
    pub const fn digest(&self) -> ContentDigest {
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
struct ScratchSegment<Tag> {
    schema_version: u32,
    kind: Tag,
    generation: u64,
    entries: Vec<ScratchRecord>,
}

struct CachedScratchSegment<Tag> {
    segment_ref: ScratchSegmentRef<Tag>,
    segment: ScratchSegment<Tag>,
    charge: usize,
    last_access: u64,
}

/// Owned, nonpersistent decoded-segment cache bound to one physical run, one
/// exact LSM root, and one serialized page kind.
///
/// This state is deliberately not `Clone`: callers create it for one bounded
/// operation and discard it before releasing or promoting the scratch run.
pub struct ScratchLookupSession<Tag> {
    run_binding: ContentDigest,
    root: ScratchLsmRoot<Tag>,
    kind_encoding: Vec<u8>,
    budget_bytes: usize,
    entries: [Option<Box<CachedScratchSegment<Tag>>>; SCRATCH_LSM_LEVELS],
    access_clock: u64,
    stats: ScratchLookupSessionStats,
}

enum SessionSegment<'session, Tag> {
    Cached(&'session ScratchSegment<Tag>),
    Uncached(ScratchSegment<Tag>),
}

impl<Tag> SessionSegment<'_, Tag> {
    fn entries(&self) -> &[ScratchRecord] {
        match self {
            Self::Cached(segment) => &segment.entries,
            Self::Uncached(segment) => &segment.entries,
        }
    }
}

impl<Tag> ScratchLookupSession<Tag> {
    pub const fn stats(&self) -> ScratchLookupSessionStats {
        self.stats
    }

    fn next_access(&mut self) -> u64 {
        self.access_clock = self.access_clock.saturating_add(1);
        self.access_clock
    }

    fn evict_lru(&mut self) {
        let Some(index) = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| entry.as_ref().map(|entry| (index, entry.last_access)))
            .min_by_key(|(_, last_access)| *last_access)
            .map(|(index, _)| index)
        else {
            return;
        };
        let evicted = self.entries[index]
            .take()
            .expect("selected lookup-session entry exists");
        self.stats.resident_bytes = self.stats.resident_bytes.saturating_sub(evicted.charge);
        self.stats.evictions = self.stats.evictions.saturating_add(1);
    }
}

fn decoded_segment_charge<Tag>(
    segment_ref: &ScratchSegmentRef<Tag>,
    segment: &ScratchSegment<Tag>,
) -> usize {
    let mut charge = std::mem::size_of::<CachedScratchSegment<Tag>>()
        .saturating_add(segment_ref.page_ref.key_min.capacity())
        .saturating_add(segment_ref.page_ref.key_max.capacity())
        .saturating_add(
            segment
                .entries
                .capacity()
                .saturating_mul(std::mem::size_of::<ScratchRecord>()),
        );
    for record in &segment.entries {
        charge = charge.saturating_add(record.key.capacity());
        if let Some(value) = &record.value {
            charge = charge.saturating_add(value.capacity());
        }
    }
    charge
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScratchSegmentRef<Tag> {
    pub generation: u64,
    pub entry_count: u64,
    pub page_ref: ScratchPageRef<Tag>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScratchLsmRoot<Tag> {
    pub next_generation: u64,
    pub levels: Vec<Option<ScratchSegmentRef<Tag>>>,
}

impl<Tag: Clone> Default for ScratchLsmRoot<Tag> {
    fn default() -> Self {
        Self {
            next_generation: 0,
            levels: vec![None; SCRATCH_LSM_LEVELS],
        }
    }
}

impl<Tag: ScratchPageTag> ScratchPageRef<Tag> {
    #[doc(hidden)]
    pub fn saturated_for_test(key_bytes: usize) -> Self {
        Self {
            offset: u64::MAX,
            encoded_len: u32::MAX,
            digest: ContentDigest::of(b"saturated scratch page"),
            kind: Tag::saturation_tag(),
            key_min: vec![0xff; key_bytes],
            key_max: vec![0xff; key_bytes],
        }
    }
}

impl<Tag: ScratchPageTag> ScratchLsmRoot<Tag> {
    /// Every fixed binary-carry level occupied with widest encodable fields.
    #[doc(hidden)]
    pub fn saturated_for_test(key_bytes: usize) -> Self {
        Self {
            next_generation: u64::MAX,
            levels: vec![
                Some(ScratchSegmentRef {
                    generation: u64::MAX,
                    entry_count: u64::MAX,
                    page_ref: ScratchPageRef::saturated_for_test(key_bytes),
                });
                SCRATCH_LSM_LEVELS
            ],
        }
    }
}

/// One exclusively leased physical scratch run and its raw address spaces.
pub struct ScratchRun<Owner> {
    namespace: Dir,
    run: Dir,
    run_name: String,
    marker: ScratchRunMarker<Owner>,
    lease: fs::File,
    pages: Mutex<BufferedAppendFile>,
    blobs: Mutex<BufferedAppendFile>,
    operation_counters: ScratchOperationCounters,
    lifecycle_stats: ScratchRunLifecycleStats,
}

/// A bounded userspace append buffer for one run-local scratch address space.
///
/// Scratch references bind byte offsets, lengths, and digests, but the scratch
/// files themselves are append-only and deliberately have no per-record sync
/// boundary. Buffering consecutive records preserves that contract while
/// avoiding one filesystem call for every small authenticated node. Any
/// operation that may observe or mutate the raw file flushes first.
struct BufferedAppendFile {
    file: fs::File,
    committed_len: u64,
    pending: Vec<u8>,
    write_batches: usize,
}

impl BufferedAppendFile {
    fn new(mut file: fs::File) -> Result<Self, ScratchRunError> {
        let committed_len = file.seek(SeekFrom::End(0))?;
        Ok(Self {
            file,
            committed_len,
            pending: Vec::with_capacity(SCRATCH_APPEND_BUFFER_BYTES),
            write_batches: 0,
        })
    }

    fn append(&mut self, bytes: &[u8]) -> Result<u64, ScratchRunError> {
        let pending_len =
            u64::try_from(self.pending.len()).map_err(|_| ScratchRunError::IndexCapacity)?;
        let offset = self
            .committed_len
            .checked_add(pending_len)
            .ok_or(ScratchRunError::IndexCapacity)?;
        if !self.pending.is_empty()
            && self.pending.len().saturating_add(bytes.len()) > SCRATCH_APPEND_BUFFER_BYTES
        {
            self.flush()?;
        }
        if bytes.len() > SCRATCH_APPEND_BUFFER_BYTES {
            self.committed_len = self.file.seek(SeekFrom::End(0))?;
            self.file.write_all(bytes)?;
            self.committed_len = self
                .committed_len
                .checked_add(bytes.len() as u64)
                .ok_or(ScratchRunError::IndexCapacity)?;
            self.write_batches = self.write_batches.saturating_add(1);
        } else {
            self.pending.extend_from_slice(bytes);
        }
        Ok(offset)
    }

    fn flush(&mut self) -> Result<(), ScratchRunError> {
        if self.pending.is_empty() {
            return Ok(());
        }
        self.committed_len = self.file.seek(SeekFrom::End(0))?;
        self.file.write_all(&self.pending)?;
        self.committed_len = self
            .committed_len
            .checked_add(self.pending.len() as u64)
            .ok_or(ScratchRunError::IndexCapacity)?;
        self.pending.clear();
        self.write_batches = self.write_batches.saturating_add(1);
        Ok(())
    }

    fn with_file<T>(
        &mut self,
        operation: impl FnOnce(&mut fs::File) -> T,
    ) -> Result<T, ScratchRunError> {
        self.flush()?;
        let result = operation(&mut self.file);
        self.committed_len = self.file.seek(SeekFrom::End(0))?;
        Ok(result)
    }
}

impl<Owner: fmt::Debug> fmt::Debug for ScratchRun<Owner> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScratchRun")
            .field("run_name", &self.run_name)
            .field("owner", &self.marker.owner)
            .finish_non_exhaustive()
    }
}

impl<Owner> ScratchRun<Owner>
where
    Owner: Clone + Eq + Serialize + DeserializeOwned,
{
    pub fn create_ephemeral(
        archive_capability: &Dir,
        owner: Owner,
    ) -> Result<Self, ScratchRunError> {
        Self::create_ephemeral_observed(archive_capability, owner, |_| Ok(()))
    }

    pub fn create_ephemeral_observed(
        archive_capability: &Dir,
        owner: Owner,
        observer: impl FnMut(ScratchConstructionBoundary) -> Result<(), ScratchRunError>,
    ) -> Result<Self, ScratchRunError> {
        Self::create_run(
            archive_capability,
            owner,
            ScratchRetention::Ephemeral,
            observer,
        )
    }

    pub fn create_retained(
        archive_capability: &Dir,
        owner: Owner,
    ) -> Result<Self, ScratchRunError> {
        Self::create_retained_observed(archive_capability, owner, |_| Ok(()))
    }

    pub fn create_retained_observed(
        archive_capability: &Dir,
        owner: Owner,
        observer: impl FnMut(ScratchConstructionBoundary) -> Result<(), ScratchRunError>,
    ) -> Result<Self, ScratchRunError> {
        Self::create_run(
            archive_capability,
            owner,
            ScratchRetention::Retained,
            observer,
        )
    }

    fn create_run(
        archive_capability: &Dir,
        owner: Owner,
        retention: ScratchRetention,
        mut observer: impl FnMut(ScratchConstructionBoundary) -> Result<(), ScratchRunError>,
    ) -> Result<Self, ScratchRunError> {
        ensure_directory_nofollow(archive_capability, SCRATCH_DIR)?;
        let namespace = open_dir_nofollow(archive_capability, SCRATCH_DIR)?;
        let run_id = Uuid::new_v4();
        let run_name = format!("run-{run_id}");
        namespace.create_dir(&run_name)?;
        let construction = Self::construct_own_run(
            &namespace,
            &run_name,
            run_id,
            owner,
            retention,
            &mut observer,
        );
        let mut run = match construction {
            Ok(run) => run,
            Err(error) => {
                remove_partial_own_run(&namespace, &run_name);
                return Err(error);
            }
        };
        run.reclaim_stale_runs(&mut observer);
        if let Err(error) = observer(ScratchConstructionBoundary::AfterReclaim) {
            run.cleanup_own_run();
            return Err(error);
        }
        Ok(run)
    }

    fn construct_own_run(
        namespace: &Dir,
        run_name: &str,
        run_id: Uuid,
        owner: Owner,
        retention: ScratchRetention,
        observer: &mut impl FnMut(ScratchConstructionBoundary) -> Result<(), ScratchRunError>,
    ) -> Result<Self, ScratchRunError> {
        observer(ScratchConstructionBoundary::AfterRunDirectory)?;
        sync_dir_required(namespace)?;
        observer(ScratchConstructionBoundary::AfterNamespaceSync)?;
        let run = open_dir_nofollow(namespace, run_name)?;
        observer(ScratchConstructionBoundary::AfterRunOpen)?;
        let nonce_a = Uuid::new_v4();
        let nonce_b = Uuid::new_v4();
        let mut random_owner_nonce = [0_u8; 32];
        random_owner_nonce[..16].copy_from_slice(nonce_a.as_bytes());
        random_owner_nonce[16..].copy_from_slice(nonce_b.as_bytes());
        let marker = ScratchRunMarker {
            schema_version: SCRATCH_SCHEMA_VERSION,
            owner,
            run_id,
            retention,
            random_owner_nonce,
        };
        write_new_regular(&run, SCRATCH_MARKER_FILE, &encode_canonical(&marker)?)?;
        observer(ScratchConstructionBoundary::AfterMarkerWrite)?;
        let lease = create_new_regular(&run, SCRATCH_LEASE_FILE)?;
        observer(ScratchConstructionBoundary::AfterLeaseCreate)?;
        lock_exclusive_nonblocking(&lease)?
            .then_some(())
            .ok_or_else(|| {
                ScratchRunError::UnsafeEntry("new scratch lease was already locked".into())
            })?;
        observer(ScratchConstructionBoundary::AfterLeaseLock)?;
        let pages = create_new_regular(&run, SCRATCH_PAGES_FILE)?;
        observer(ScratchConstructionBoundary::AfterPagesCreate)?;
        let blobs = create_new_regular(&run, SCRATCH_BLOBS_FILE)?;
        observer(ScratchConstructionBoundary::AfterBlobsCreate)?;
        Ok(Self {
            namespace: namespace.try_clone()?,
            run,
            run_name: run_name.to_owned(),
            marker,
            lease,
            pages: Mutex::new(BufferedAppendFile::new(pages)?),
            blobs: Mutex::new(BufferedAppendFile::new(blobs)?),
            operation_counters: ScratchOperationCounters::default(),
            lifecycle_stats: ScratchRunLifecycleStats::default(),
        })
    }

    pub fn adopt_retained(
        archive_capability: &Dir,
        owner: Owner,
        run_id: Uuid,
    ) -> Result<Self, ScratchRunError> {
        let namespace = open_dir_nofollow(archive_capability, SCRATCH_DIR)?;
        let run_name = format!("run-{run_id}");
        if parse_run_name(&run_name)? != run_id {
            return Err(ScratchRunError::MalformedMarker(run_name));
        }
        let run = open_dir_nofollow(&namespace, &run_name)?;
        let lease = open_regular_read_write_nofollow(&run, SCRATCH_LEASE_FILE)?;
        if !lock_exclusive_nonblocking(&lease)? {
            return Err(ScratchRunError::UnsafeEntry(format!(
                "retained scratch run {run_name:?} is still leased"
            )));
        }
        let marker_bytes = read_regular_nofollow(&run, SCRATCH_MARKER_FILE, MAX_MARKER_BYTES)?;
        let marker: ScratchRunMarker<Owner> = decode_canonical(&marker_bytes)?;
        if marker.schema_version != SCRATCH_SCHEMA_VERSION
            || marker.owner != owner
            || marker.run_id != run_id
            || marker.retention != ScratchRetention::Retained
        {
            return Err(ScratchRunError::MalformedMarker(run_name));
        }
        validate_run_entries(&run)?;
        let pages = open_regular_read_write_nofollow(&run, SCRATCH_PAGES_FILE)?;
        let blobs = open_regular_read_write_nofollow(&run, SCRATCH_BLOBS_FILE)?;
        Ok(Self {
            namespace,
            run,
            run_name,
            marker,
            lease,
            pages: Mutex::new(BufferedAppendFile::new(pages)?),
            blobs: Mutex::new(BufferedAppendFile::new(blobs)?),
            operation_counters: ScratchOperationCounters::default(),
            lifecycle_stats: ScratchRunLifecycleStats::default(),
        })
    }

    pub fn clone_retained_into(&self, archive_capability: &Dir) -> Result<Self, ScratchRunError> {
        if self.marker.retention != ScratchRetention::Retained {
            return Err(ScratchRunError::UnsafeEntry(
                "scratch migration source is not retained".into(),
            ));
        }
        ensure_directory_nofollow(archive_capability, SCRATCH_DIR)?;
        let namespace = open_dir_nofollow(archive_capability, SCRATCH_DIR)?;
        let run_name = format!("run-{}", self.run_id());
        namespace.create_dir(&run_name)?;
        let construction = (|| {
            sync_dir_required(&namespace)?;
            let run = open_dir_nofollow(&namespace, &run_name)?;
            write_new_regular(&run, SCRATCH_MARKER_FILE, &encode_canonical(&self.marker)?)?;
            let lease = create_new_regular(&run, SCRATCH_LEASE_FILE)?;
            lock_exclusive_nonblocking(&lease)?
                .then_some(())
                .ok_or_else(|| {
                    ScratchRunError::UnsafeEntry("migrated scratch lease was already locked".into())
                })?;
            let pages = create_new_regular(&run, SCRATCH_PAGES_FILE)?;
            let blobs = create_new_regular(&run, SCRATCH_BLOBS_FILE)?;
            let migrated = Self {
                namespace: namespace.try_clone()?,
                run,
                run_name: run_name.clone(),
                marker: self.marker.clone(),
                lease,
                pages: Mutex::new(BufferedAppendFile::new(pages)?),
                blobs: Mutex::new(BufferedAppendFile::new(blobs)?),
                operation_counters: ScratchOperationCounters::default(),
                lifecycle_stats: ScratchRunLifecycleStats::default(),
            };
            migrated.copy_exact_from(self)?;
            Ok(migrated)
        })();
        match construction {
            Ok(migrated) => Ok(migrated),
            Err(error) => {
                remove_partial_own_run(&namespace, &run_name);
                Err(error)
            }
        }
    }

    pub const fn run_id(&self) -> Uuid {
        self.marker.run_id
    }

    pub const fn retention(&self) -> ScratchRetention {
        self.marker.retention
    }

    pub const fn owner(&self) -> &Owner {
        &self.marker.owner
    }

    pub fn binding_digest(&self) -> Result<ContentDigest, ScratchRunError> {
        Ok(ContentDigest::of(&encode_canonical(&self.marker)?))
    }

    pub const fn lifecycle_stats(&self) -> ScratchRunLifecycleStats {
        self.lifecycle_stats
    }

    /// Execute one operation against the locked raw page-file address space.
    pub fn with_pages<T>(
        &self,
        operation: impl FnOnce(&mut fs::File) -> T,
    ) -> Result<T, ScratchRunError> {
        self.pages
            .lock()
            .map_err(|_| ScratchRunError::Poisoned)?
            .with_file(operation)
    }

    /// Execute one operation against the locked raw blob-file address space.
    pub fn with_blobs<T>(
        &self,
        operation: impl FnOnce(&mut fs::File) -> T,
    ) -> Result<T, ScratchRunError> {
        self.blobs
            .lock()
            .map_err(|_| ScratchRunError::Poisoned)?
            .with_file(operation)
    }

    pub fn operation_stats(&self) -> ScratchOperationStats {
        #[allow(unused_mut)]
        let mut stats = self.operation_counters.snapshot();
        #[cfg(any(test, feature = "test-support"))]
        {
            stats.page_append_batches = self
                .pages
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .write_batches;
            stats.blob_append_batches = self
                .blobs
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .write_batches;
        }
        stats
    }

    /// Start an empty decoded-segment lookup session bound to this exact
    /// physical run, LSM root, and serialized page kind.
    pub fn lookup_session<Tag>(
        &self,
        root: &ScratchLsmRoot<Tag>,
        kind: Tag,
        budget_bytes: usize,
    ) -> Result<ScratchLookupSession<Tag>, ScratchRunError>
    where
        Tag: ScratchPageTag,
    {
        validate_lsm_root(root)?;
        Ok(ScratchLookupSession {
            run_binding: self.binding_digest()?,
            root: root.clone(),
            kind_encoding: encode_page_canonical(&kind)?,
            budget_bytes,
            entries: std::array::from_fn(|_| None),
            access_clock: 0,
            stats: ScratchLookupSessionStats::default(),
        })
    }

    /// Record caller-defined logical point operations implemented with generic
    /// scratch pages rather than the storage-owned LSM codec.
    pub fn record_point_reads(&self, count: usize) {
        self.operation_counters
            .point_reads
            .fetch_add(count, Ordering::Relaxed);
    }

    /// Record one caller-defined logical range operation implemented with
    /// generic scratch pages rather than the storage-owned LSM codec.
    pub fn record_range_read(&self) {
        self.operation_counters
            .range_reads
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn append_blob(&self, bytes: &[u8]) -> Result<ScratchBlobRef, ScratchRunError> {
        if bytes.is_empty() || bytes.len() > MAX_SCRATCH_BLOB_BYTES {
            return Err(ScratchRunError::MalformedBlob);
        }
        let digest = ContentDigest::of(bytes);
        let encoded_len = u32::try_from(bytes.len()).map_err(|_| ScratchRunError::MalformedBlob)?;
        let offset = self
            .blobs
            .lock()
            .map_err(|_| ScratchRunError::Poisoned)?
            .append(bytes)?;
        self.operation_counters
            .blob_writes
            .fetch_add(1, Ordering::Relaxed);
        self.operation_counters
            .blob_bytes_written
            .fetch_add(bytes.len(), Ordering::Relaxed);
        Ok(ScratchBlobRef {
            offset,
            encoded_len,
            digest,
        })
    }

    pub fn read_blob(&self, blob_ref: &ScratchBlobRef) -> Result<Vec<u8>, ScratchRunError> {
        let length =
            usize::try_from(blob_ref.encoded_len).map_err(|_| ScratchRunError::MalformedBlob)?;
        if length == 0 || length > MAX_SCRATCH_BLOB_BYTES {
            return Err(ScratchRunError::MalformedBlob);
        }
        let mut bytes = vec![0_u8; length];
        self.with_blobs(|file| -> Result<_, ScratchRunError> {
            file.seek(SeekFrom::Start(blob_ref.offset))?;
            file.read_exact(&mut bytes)
                .map_err(|_| ScratchRunError::MalformedBlob)
        })??;
        if ContentDigest::of(&bytes) != blob_ref.digest {
            return Err(ScratchRunError::BlobDigestMismatch(blob_ref.digest));
        }
        self.operation_counters
            .blob_reads
            .fetch_add(1, Ordering::Relaxed);
        self.operation_counters
            .blob_bytes_read
            .fetch_add(bytes.len(), Ordering::Relaxed);
        Ok(bytes)
    }

    pub fn append_page<Tag, Value>(
        &self,
        kind: Tag,
        key_min: Vec<u8>,
        key_max: Vec<u8>,
        value: &Value,
    ) -> Result<ScratchPageRef<Tag>, ScratchRunError>
    where
        Tag: ScratchPageTag,
        Value: Serialize,
    {
        if key_min.is_empty() || key_min > key_max {
            return Err(ScratchRunError::MalformedPage);
        }
        let payload = encode_page_canonical(value)?;
        let envelope = ScratchPageEnvelope {
            schema_version: SCRATCH_PAGE_SCHEMA_VERSION,
            kind,
            key_min: key_min.clone(),
            key_max: key_max.clone(),
            payload,
        };
        let bytes = encode_page_canonical(&envelope)?;
        if bytes.len() > MAX_SCRATCH_PAGE_BYTES {
            return Err(ScratchRunError::PageTooLarge(bytes.len()));
        }
        let digest = ContentDigest::of(&bytes);
        let encoded_len = u32::try_from(bytes.len()).map_err(|_| ScratchRunError::MalformedPage)?;
        let offset = self
            .pages
            .lock()
            .map_err(|_| ScratchRunError::Poisoned)?
            .append(&bytes)?;
        self.operation_counters
            .page_writes
            .fetch_add(1, Ordering::Relaxed);
        self.operation_counters
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

    pub fn read_page<Tag, Value>(
        &self,
        page_ref: &ScratchPageRef<Tag>,
        expected_kind: Tag,
    ) -> Result<Value, ScratchRunError>
    where
        Tag: ScratchPageTag,
        Value: DeserializeOwned + Serialize,
    {
        if page_ref.kind != expected_kind {
            return Err(ScratchRunError::PageBindingMismatch);
        }
        let length =
            usize::try_from(page_ref.encoded_len).map_err(|_| ScratchRunError::MalformedPage)?;
        if length == 0 || length > MAX_SCRATCH_PAGE_BYTES {
            return Err(ScratchRunError::MalformedPage);
        }
        let mut bytes = vec![0_u8; length];
        self.with_pages(|file| -> Result<_, ScratchRunError> {
            file.seek(SeekFrom::Start(page_ref.offset))?;
            file.read_exact(&mut bytes)
                .map_err(|_| ScratchRunError::MalformedPage)
        })??;
        if ContentDigest::of(&bytes) != page_ref.digest {
            return Err(ScratchRunError::PageDigestMismatch(page_ref.digest));
        }
        let envelope: ScratchPageEnvelope<Tag> = decode_page_canonical(&bytes)?;
        if envelope.schema_version != SCRATCH_PAGE_SCHEMA_VERSION
            || envelope.kind != expected_kind
            || envelope.key_min != page_ref.key_min
            || envelope.key_max != page_ref.key_max
        {
            return Err(ScratchRunError::PageBindingMismatch);
        }
        self.operation_counters
            .page_reads
            .fetch_add(1, Ordering::Relaxed);
        self.operation_counters
            .page_bytes_read
            .fetch_add(bytes.len(), Ordering::Relaxed);
        self.operation_counters
            .max_page_bytes_read
            .fetch_max(bytes.len(), Ordering::Relaxed);
        decode_page_canonical(&envelope.payload)
    }

    pub fn insert_many<Tag>(
        &self,
        root: &ScratchLsmRoot<Tag>,
        kind: Tag,
        records: &BTreeMap<Vec<u8>, Option<Vec<u8>>>,
    ) -> Result<ScratchLsmRoot<Tag>, ScratchRunError>
    where
        Tag: ScratchPageTag,
    {
        if records.is_empty() {
            return Ok(root.clone());
        }
        validate_lsm_root(root)?;
        let generation = root
            .next_generation
            .checked_add(1)
            .ok_or(ScratchRunError::MalformedPage)?;
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
            return Ok(next);
        }
        Err(ScratchRunError::IndexCapacity)
    }

    pub fn lookup<Tag>(
        &self,
        root: &ScratchLsmRoot<Tag>,
        kind: Tag,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, ScratchRunError>
    where
        Tag: ScratchPageTag,
    {
        self.lookup_with_known_absent(root, kind, key, false)
    }

    /// Perform one authenticated lookup while allowing a caller-owned policy
    /// filter to prove the point absent without reading segment pages.
    pub fn lookup_with_known_absent<Tag>(
        &self,
        root: &ScratchLsmRoot<Tag>,
        kind: Tag,
        key: &[u8],
        known_absent: bool,
    ) -> Result<Option<Vec<u8>>, ScratchRunError>
    where
        Tag: ScratchPageTag,
    {
        self.lookup_with_absence_policy(root, kind, key, || Ok(known_absent))
    }

    /// Validate and account for a point lookup before consulting a caller-owned
    /// absence policy, then perform the generic authenticated lookup if needed.
    pub fn lookup_with_absence_policy<Tag>(
        &self,
        root: &ScratchLsmRoot<Tag>,
        kind: Tag,
        key: &[u8],
        absence_policy: impl FnOnce() -> Result<bool, ScratchRunError>,
    ) -> Result<Option<Vec<u8>>, ScratchRunError>
    where
        Tag: ScratchPageTag,
    {
        validate_lsm_root(root)?;
        self.operation_counters
            .point_reads
            .fetch_add(1, Ordering::Relaxed);
        let known_absent = absence_policy()?;
        if known_absent {
            return Ok(None);
        }
        let mut segments = root
            .levels
            .iter()
            .flatten()
            .collect::<Vec<&ScratchSegmentRef<Tag>>>();
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

    pub fn lookup_many<Tag>(
        &self,
        root: &ScratchLsmRoot<Tag>,
        kind: Tag,
        keys: &[Vec<u8>],
    ) -> Result<Vec<Option<Vec<u8>>>, ScratchRunError>
    where
        Tag: ScratchPageTag,
    {
        self.lookup_many_with_known_absent(root, kind, keys, &vec![false; keys.len()])
    }

    /// Batched authenticated lookup with caller-owned, per-key absence proofs.
    pub fn lookup_many_with_known_absent<Tag>(
        &self,
        root: &ScratchLsmRoot<Tag>,
        kind: Tag,
        keys: &[Vec<u8>],
        known_absent: &[bool],
    ) -> Result<Vec<Option<Vec<u8>>>, ScratchRunError>
    where
        Tag: ScratchPageTag,
    {
        self.lookup_many_with_absence_policy(root, kind, keys, || Ok(known_absent.to_vec()))
    }

    /// Validate and account for a batched lookup before consulting caller-owned
    /// absence policy, while reading each relevant segment at most once.
    pub fn lookup_many_with_absence_policy<Tag>(
        &self,
        root: &ScratchLsmRoot<Tag>,
        kind: Tag,
        keys: &[Vec<u8>],
        absence_policy: impl FnOnce() -> Result<Vec<bool>, ScratchRunError>,
    ) -> Result<Vec<Option<Vec<u8>>>, ScratchRunError>
    where
        Tag: ScratchPageTag,
    {
        validate_lsm_root(root)?;
        self.operation_counters
            .point_reads
            .fetch_add(keys.len(), Ordering::Relaxed);
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let known_absent = absence_policy()?;
        if known_absent.len() != keys.len() {
            return Err(ScratchRunError::MalformedPage);
        }
        let mut resolved = known_absent;
        let mut values = vec![None; keys.len()];
        let mut segments = root
            .levels
            .iter()
            .flatten()
            .collect::<Vec<&ScratchSegmentRef<Tag>>>();
        segments.sort_unstable_by_key(|segment| std::cmp::Reverse(segment.generation));
        for segment_ref in segments {
            let selected = keys
                .iter()
                .enumerate()
                .filter_map(|(index, key)| {
                    (!resolved[index]
                        && key.as_slice() >= segment_ref.page_ref.key_min.as_slice()
                        && key.as_slice() <= segment_ref.page_ref.key_max.as_slice())
                    .then_some(index)
                })
                .collect::<Vec<_>>();
            if selected.is_empty() {
                continue;
            }
            let segment = self.read_segment(kind, segment_ref)?;
            for index in selected {
                if let Ok(record_index) = segment
                    .entries
                    .binary_search_by(|record| record.key.as_slice().cmp(keys[index].as_slice()))
                {
                    values[index] = segment.entries[record_index].value.clone();
                    resolved[index] = true;
                }
            }
        }
        Ok(values)
    }

    /// Batched authenticated lookup using a caller-owned decoded-segment
    /// session. The caller-owned absence policy is still evaluated for every
    /// call, before any segment cache is consulted.
    pub fn lookup_many_with_session_and_absence_policy<Tag>(
        &self,
        session: &mut ScratchLookupSession<Tag>,
        root: &ScratchLsmRoot<Tag>,
        kind: Tag,
        keys: &[Vec<u8>],
        absence_policy: impl FnOnce() -> Result<Vec<bool>, ScratchRunError>,
    ) -> Result<Vec<Option<Vec<u8>>>, ScratchRunError>
    where
        Tag: ScratchPageTag,
    {
        validate_lsm_root(root)?;
        if session.run_binding != self.binding_digest()?
            || session.root != *root
            || session.kind_encoding != encode_page_canonical(&kind)?
        {
            return Err(ScratchRunError::PageBindingMismatch);
        }
        self.operation_counters
            .point_reads
            .fetch_add(keys.len(), Ordering::Relaxed);
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let known_absent = absence_policy()?;
        if known_absent.len() != keys.len() {
            return Err(ScratchRunError::MalformedPage);
        }
        let mut resolved = known_absent;
        let mut values = vec![None; keys.len()];
        let mut segments = root
            .levels
            .iter()
            .flatten()
            .collect::<Vec<&ScratchSegmentRef<Tag>>>();
        segments.sort_unstable_by_key(|segment| std::cmp::Reverse(segment.generation));
        for segment_ref in segments {
            let selected = keys
                .iter()
                .enumerate()
                .filter_map(|(index, key)| {
                    (!resolved[index]
                        && key.as_slice() >= segment_ref.page_ref.key_min.as_slice()
                        && key.as_slice() <= segment_ref.page_ref.key_max.as_slice())
                    .then_some(index)
                })
                .collect::<Vec<_>>();
            if selected.is_empty() {
                continue;
            }
            let segment = self.read_segment_with_session(session, kind, segment_ref)?;
            for index in selected {
                if let Ok(record_index) = segment
                    .entries()
                    .binary_search_by(|record| record.key.as_slice().cmp(keys[index].as_slice()))
                {
                    values[index] = segment.entries()[record_index].value.clone();
                    resolved[index] = true;
                }
            }
        }
        Ok(values)
    }

    pub fn scan_prefix<Tag>(
        &self,
        root: &ScratchLsmRoot<Tag>,
        kind: Tag,
        prefix: &[u8],
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, ScratchRunError>
    where
        Tag: ScratchPageTag,
    {
        validate_lsm_root(root)?;
        self.operation_counters
            .range_reads
            .fetch_add(1, Ordering::Relaxed);
        let mut segments = root
            .levels
            .iter()
            .flatten()
            .collect::<Vec<&ScratchSegmentRef<Tag>>>();
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

    pub fn materialize<Tag>(
        &self,
        root: &ScratchLsmRoot<Tag>,
        kind: Tag,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, ScratchRunError>
    where
        Tag: ScratchPageTag,
    {
        self.scan_prefix(root, kind, &[])
    }

    fn read_segment<Tag>(
        &self,
        kind: Tag,
        segment_ref: &ScratchSegmentRef<Tag>,
    ) -> Result<ScratchSegment<Tag>, ScratchRunError>
    where
        Tag: ScratchPageTag,
    {
        let segment: ScratchSegment<Tag> = self.read_page(&segment_ref.page_ref, kind)?;
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
            return Err(ScratchRunError::PageBindingMismatch);
        }
        Ok(segment)
    }

    fn read_segment_with_session<'session, Tag>(
        &self,
        session: &'session mut ScratchLookupSession<Tag>,
        kind: Tag,
        segment_ref: &ScratchSegmentRef<Tag>,
    ) -> Result<SessionSegment<'session, Tag>, ScratchRunError>
    where
        Tag: ScratchPageTag,
    {
        if let Some(index) = session.entries.iter().position(|entry| {
            entry
                .as_ref()
                .is_some_and(|entry| entry.segment_ref == *segment_ref)
        }) {
            let access = session.next_access();
            session.stats.hits = session.stats.hits.saturating_add(1);
            let entry = session.entries[index]
                .as_mut()
                .expect("located lookup-session entry exists");
            entry.last_access = access;
            return Ok(SessionSegment::Cached(&entry.segment));
        }

        session.stats.misses = session.stats.misses.saturating_add(1);
        let segment = self.read_segment(kind, segment_ref)?;
        let charge = decoded_segment_charge(segment_ref, &segment);
        if charge > session.budget_bytes {
            session.stats.oversize = session.stats.oversize.saturating_add(1);
            return Ok(SessionSegment::Uncached(segment));
        }
        while session.stats.resident_bytes.saturating_add(charge) > session.budget_bytes {
            session.evict_lru();
        }
        if session.entries.iter().all(Option::is_some) {
            session.evict_lru();
        }
        let index = session
            .entries
            .iter()
            .position(Option::is_none)
            .expect("eviction leaves one lookup-session slot");
        let access = session.next_access();
        session.entries[index] = Some(Box::new(CachedScratchSegment {
            segment_ref: segment_ref.clone(),
            segment,
            charge,
            last_access: access,
        }));
        session.stats.resident_bytes = session.stats.resident_bytes.saturating_add(charge);
        session.stats.peak_resident_bytes = session
            .stats
            .peak_resident_bytes
            .max(session.stats.resident_bytes);
        Ok(SessionSegment::Cached(
            &session.entries[index]
                .as_ref()
                .expect("admitted lookup-session entry exists")
                .segment,
        ))
    }

    pub fn clone_pages_file(&self) -> Result<fs::File, ScratchRunError> {
        let mut pages = self.pages.lock().map_err(|_| ScratchRunError::Poisoned)?;
        pages.flush()?;
        pages.file.try_clone().map_err(Into::into)
    }

    fn copy_exact_from(&self, source: &Self) -> Result<(), ScratchRunError> {
        if self.marker != source.marker || self.binding_digest()? != source.binding_digest()? {
            return Err(ScratchRunError::UnsafeEntry(
                "scratch migration source and destination identity mismatch".into(),
            ));
        }

        fn copy_file(
            source: &Mutex<BufferedAppendFile>,
            destination: &Mutex<BufferedAppendFile>,
        ) -> Result<(), ScratchRunError> {
            let mut source = source.lock().map_err(|_| ScratchRunError::Poisoned)?;
            let mut destination = destination.lock().map_err(|_| ScratchRunError::Poisoned)?;
            source.flush()?;
            destination.flush()?;
            if destination.file.metadata()?.len() != 0 {
                return Err(ScratchRunError::UnsafeEntry(
                    "scratch migration destination is not empty".into(),
                ));
            }
            source.file.seek(SeekFrom::Start(0))?;
            destination.file.seek(SeekFrom::Start(0))?;
            let expected = source.file.metadata()?.len();
            let copied = std::io::copy(&mut source.file, &mut destination.file)?;
            if copied != expected || destination.file.metadata()?.len() != expected {
                return Err(ScratchRunError::UnsafeEntry(
                    "scratch migration did not copy the exact byte extent".into(),
                ));
            }
            source.committed_len = expected;
            destination.committed_len = expected;
            Ok(())
        }

        copy_file(&source.pages, &self.pages)?;
        copy_file(&source.blobs, &self.blobs)
    }

    fn reclaim_stale_runs(
        &mut self,
        observer: &mut impl FnMut(ScratchConstructionBoundary) -> Result<(), ScratchRunError>,
    ) {
        let Ok(entries) = self.namespace.entries() else {
            self.lifecycle_stats.unclassified_runs_preserved += 1;
            return;
        };
        for entry in entries {
            let disposition = entry
                .map_err(ScratchRunError::from)
                .and_then(|entry| self.classify_stale_sibling(&entry, observer))
                .unwrap_or(StaleRunDisposition::Unclassified);
            match disposition {
                StaleRunDisposition::OwnRun => {}
                StaleRunDisposition::Reclaimed => self.lifecycle_stats.stale_runs_reclaimed += 1,
                StaleRunDisposition::LivePreserved => self.lifecycle_stats.live_runs_skipped += 1,
                StaleRunDisposition::RetainedPreserved => {
                    self.lifecycle_stats.retained_runs_preserved += 1;
                }
                StaleRunDisposition::Unclassified => {
                    self.lifecycle_stats.unclassified_runs_preserved += 1;
                }
            }
        }
    }

    fn classify_stale_sibling(
        &self,
        entry: &cap_std::fs::DirEntry,
        observer: &mut impl FnMut(ScratchConstructionBoundary) -> Result<(), ScratchRunError>,
    ) -> Result<StaleRunDisposition, ScratchRunError> {
        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| ScratchRunError::UnsafeEntry("non-UTF-8 scratch run".into()))?
            .to_owned();
        if name == self.run_name {
            return Ok(StaleRunDisposition::OwnRun);
        }
        observer(ScratchConstructionBoundary::InspectSibling)?;
        let run_id = parse_run_name(&name)?;
        require_real_directory(entry, &name)?;
        let run = open_dir_nofollow(&self.namespace, &name)?;
        let marker_bytes = read_regular_nofollow(&run, SCRATCH_MARKER_FILE, MAX_MARKER_BYTES)?;
        let marker: ScratchRunMarker<Owner> = decode_canonical(&marker_bytes)?;
        if marker.schema_version != SCRATCH_SCHEMA_VERSION
            || marker.owner != self.marker.owner
            || marker.run_id != run_id
        {
            return Err(ScratchRunError::MalformedMarker(name));
        }
        validate_run_entries(&run)?;
        if marker.retention == ScratchRetention::Retained {
            return Ok(StaleRunDisposition::RetainedPreserved);
        }
        let lease = open_regular_read_write_nofollow(&run, SCRATCH_LEASE_FILE)?;
        if !lock_exclusive_nonblocking(&lease)? {
            return Ok(StaleRunDisposition::LivePreserved);
        }
        remove_stale_run(&self.namespace, &run, &name, lease)?;
        Ok(StaleRunDisposition::Reclaimed)
    }

    fn cleanup_own_run(&self) {
        for name in [SCRATCH_PAGES_FILE, SCRATCH_BLOBS_FILE, SCRATCH_MARKER_FILE] {
            let _ = self.run.remove_file(name);
        }
        unlock(&self.lease);
        let _ = self.run.remove_file(SCRATCH_LEASE_FILE);
        let _ = self.namespace.remove_dir(&self.run_name);
    }
}

impl<Owner> Drop for ScratchRun<Owner> {
    fn drop(&mut self) {
        if let Ok(pages) = self.pages.get_mut() {
            let _ = pages.flush();
        }
        if let Ok(blobs) = self.blobs.get_mut() {
            let _ = blobs.flush();
        }
        match self.marker.retention {
            ScratchRetention::Ephemeral => self.cleanup_own_run_unbounded(),
            ScratchRetention::Retained => unlock(&self.lease),
        }
    }
}

impl<Owner> ScratchRun<Owner> {
    fn cleanup_own_run_unbounded(&self) {
        for name in [SCRATCH_PAGES_FILE, SCRATCH_BLOBS_FILE, SCRATCH_MARKER_FILE] {
            let _ = self.run.remove_file(name);
        }
        unlock(&self.lease);
        let _ = self.run.remove_file(SCRATCH_LEASE_FILE);
        let _ = self.namespace.remove_dir(&self.run_name);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StaleRunDisposition {
    OwnRun,
    Reclaimed,
    LivePreserved,
    RetainedPreserved,
    Unclassified,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RetainedRunCensus {
    pub retained: usize,
    pub ephemeral: usize,
    pub unclassified: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RetainedRunReclamation {
    pub retained_reachable: usize,
    pub retained_reclaimed: usize,
    pub retained_live_skipped: usize,
    pub ephemeral_preserved: usize,
    pub unclassified_preserved: usize,
}

pub fn census_retained_runs<Owner>(
    archive_capability: &Dir,
    owner: &Owner,
) -> Result<RetainedRunCensus, ScratchRunError>
where
    Owner: Eq + Serialize + DeserializeOwned,
{
    let mut census = RetainedRunCensus::default();
    let Some(namespace) = open_scratch_namespace(archive_capability)? else {
        return Ok(census);
    };
    for entry in namespace.entries()? {
        match entry
            .map_err(ScratchRunError::from)
            .and_then(|entry| authenticate_scratch_sibling(&namespace, &entry, owner))
        {
            Ok((_, AuthenticatedScratchSibling::Retained(_))) => census.retained += 1,
            Ok((_, AuthenticatedScratchSibling::Ephemeral)) => census.ephemeral += 1,
            Err(_) => census.unclassified += 1,
        }
    }
    Ok(census)
}

/// Delete free authenticated retained runs excluded by a caller-proved set.
///
/// # Safety
///
/// The caller must have authenticated a complete authoritative reachability
/// scan and must pass every retained run identity reachable by that scan.
/// This is unsafe so an ordinary safe downstream call site cannot turn a
/// partial or guessed set into deletion authority.
pub unsafe fn reclaim_unreachable_retained_runs<Owner>(
    archive_capability: &Dir,
    owner: &Owner,
    reachable: impl Fn(Uuid) -> bool,
) -> Result<RetainedRunReclamation, ScratchRunError>
where
    Owner: Eq + Serialize + DeserializeOwned,
{
    let mut outcome = RetainedRunReclamation::default();
    let Some(namespace) = open_scratch_namespace(archive_capability)? else {
        return Ok(outcome);
    };
    for entry in namespace.entries()? {
        let disposition = entry
            .map_err(ScratchRunError::from)
            .and_then(|entry| classify_retained_sibling(&namespace, &entry, owner, &reachable))
            .unwrap_or(RetainedRunDisposition::Unclassified);
        match disposition {
            RetainedRunDisposition::Reachable => outcome.retained_reachable += 1,
            RetainedRunDisposition::Reclaimed => outcome.retained_reclaimed += 1,
            RetainedRunDisposition::LivePreserved => outcome.retained_live_skipped += 1,
            RetainedRunDisposition::EphemeralPreserved => outcome.ephemeral_preserved += 1,
            RetainedRunDisposition::Unclassified => outcome.unclassified_preserved += 1,
        }
    }
    Ok(outcome)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RetainedRunDisposition {
    Reachable,
    Reclaimed,
    LivePreserved,
    EphemeralPreserved,
    Unclassified,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AuthenticatedScratchSibling {
    Retained(Uuid),
    Ephemeral,
}

fn open_scratch_namespace(archive_capability: &Dir) -> Result<Option<Dir>, ScratchRunError> {
    match archive_capability.symlink_metadata(SCRATCH_DIR) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(ScratchRunError::UnsafeEntry(format!(
                "{SCRATCH_DIR} is not a real no-follow directory"
            )));
        }
        Ok(_) => {}
    }
    Ok(Some(open_dir_nofollow(archive_capability, SCRATCH_DIR)?))
}

fn authenticate_scratch_sibling<Owner>(
    namespace: &Dir,
    entry: &cap_std::fs::DirEntry,
    owner: &Owner,
) -> Result<(String, AuthenticatedScratchSibling), ScratchRunError>
where
    Owner: Eq + Serialize + DeserializeOwned,
{
    let name = entry
        .file_name()
        .to_str()
        .ok_or_else(|| ScratchRunError::UnsafeEntry("non-UTF-8 scratch run".into()))?
        .to_owned();
    let run_id = parse_run_name(&name)?;
    require_real_directory(entry, &name)?;
    let run = open_dir_nofollow(namespace, &name)?;
    let marker_bytes = read_regular_nofollow(&run, SCRATCH_MARKER_FILE, MAX_MARKER_BYTES)?;
    let marker: ScratchRunMarker<Owner> = decode_canonical(&marker_bytes)?;
    if marker.schema_version != SCRATCH_SCHEMA_VERSION
        || &marker.owner != owner
        || marker.run_id != run_id
    {
        return Err(ScratchRunError::MalformedMarker(name));
    }
    validate_run_entries(&run)?;
    let sibling = match marker.retention {
        ScratchRetention::Retained => AuthenticatedScratchSibling::Retained(run_id),
        ScratchRetention::Ephemeral => AuthenticatedScratchSibling::Ephemeral,
    };
    Ok((name, sibling))
}

fn classify_retained_sibling<Owner>(
    namespace: &Dir,
    entry: &cap_std::fs::DirEntry,
    owner: &Owner,
    reachable: &impl Fn(Uuid) -> bool,
) -> Result<RetainedRunDisposition, ScratchRunError>
where
    Owner: Eq + Serialize + DeserializeOwned,
{
    let (name, sibling) = authenticate_scratch_sibling(namespace, entry, owner)?;
    let AuthenticatedScratchSibling::Retained(run_id) = sibling else {
        return Ok(RetainedRunDisposition::EphemeralPreserved);
    };
    if reachable(run_id) {
        return Ok(RetainedRunDisposition::Reachable);
    }
    let run = open_dir_nofollow(namespace, &name)?;
    let lease = open_regular_read_write_nofollow(&run, SCRATCH_LEASE_FILE)?;
    if !lock_exclusive_nonblocking(&lease)? {
        return Ok(RetainedRunDisposition::LivePreserved);
    }
    remove_stale_run(namespace, &run, &name, lease)?;
    Ok(RetainedRunDisposition::Reclaimed)
}

fn parse_run_name(name: &str) -> Result<Uuid, ScratchRunError> {
    let suffix = name
        .strip_prefix("run-")
        .ok_or_else(|| ScratchRunError::UnsafeEntry(format!("unknown scratch entry {name:?}")))?;
    let run_id = Uuid::parse_str(suffix)
        .map_err(|_| ScratchRunError::UnsafeEntry(format!("malformed scratch run {name:?}")))?;
    if format!("run-{run_id}") != name {
        return Err(ScratchRunError::UnsafeEntry(format!(
            "non-canonical scratch run {name:?}"
        )));
    }
    Ok(run_id)
}

fn validate_run_entries(run: &Dir) -> Result<(), ScratchRunError> {
    let mut seen = BTreeSet::new();
    for entry in run.entries()? {
        let entry = entry?;
        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| ScratchRunError::UnsafeEntry("non-UTF-8 scratch entry".into()))?
            .to_owned();
        if ![
            SCRATCH_MARKER_FILE,
            SCRATCH_LEASE_FILE,
            SCRATCH_PAGES_FILE,
            SCRATCH_BLOBS_FILE,
        ]
        .contains(&name.as_str())
        {
            return Err(ScratchRunError::UnsafeEntry(format!(
                "unknown scratch run entry {name:?}"
            )));
        }
        require_regular_entry(&entry, &name)?;
        if !seen.insert(name.clone()) {
            return Err(ScratchRunError::UnsafeEntry(format!(
                "duplicate scratch run entry {name:?}"
            )));
        }
    }
    for required in [
        SCRATCH_MARKER_FILE,
        SCRATCH_LEASE_FILE,
        SCRATCH_PAGES_FILE,
        SCRATCH_BLOBS_FILE,
    ] {
        if !seen.contains(required) {
            return Err(ScratchRunError::UnsafeEntry(format!(
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
) -> Result<(), ScratchRunError> {
    validate_run_entries(run)?;
    for name in [SCRATCH_PAGES_FILE, SCRATCH_BLOBS_FILE, SCRATCH_MARKER_FILE] {
        run.remove_file(name)?;
    }
    unlock(&lease);
    drop(lease);
    run.remove_file(SCRATCH_LEASE_FILE)?;
    namespace.remove_dir(run_name)?;
    Ok(())
}

fn remove_partial_own_run(namespace: &Dir, run_name: &str) {
    if let Ok(run) = open_dir_nofollow(namespace, run_name) {
        for name in [
            SCRATCH_PAGES_FILE,
            SCRATCH_BLOBS_FILE,
            SCRATCH_MARKER_FILE,
            SCRATCH_LEASE_FILE,
        ] {
            let _ = run.remove_file(name);
        }
    }
    let _ = namespace.remove_dir(run_name);
}

fn create_new_regular(dir: &Dir, name: &str) -> Result<fs::File, ScratchRunError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    let file = dir.open_with(name, &options)?.into_std();
    ensure_opened_regular(&file, name)?;
    Ok(file)
}

fn write_new_regular(dir: &Dir, name: &str, bytes: &[u8]) -> Result<(), ScratchRunError> {
    let mut file = create_new_regular(dir, name)?;
    file.write_all(bytes)?;
    Ok(())
}

fn open_regular_read_write_nofollow(dir: &Dir, name: &str) -> Result<fs::File, ScratchRunError> {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::fd::AsFd as _;
        let path = CString::new(name)
            .map_err(|_| ScratchRunError::UnsafeEntry("invalid scratch filename".into()))?;
        let fd = unsafe {
            libc::openat(
                dir.as_fd().as_raw_fd(),
                path.as_ptr(),
                libc::O_RDWR | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let file = unsafe { fs::File::from_raw_fd(fd) };
        ensure_opened_regular(&file, name)?;
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
        Ok(file)
    }
}

fn read_regular_nofollow(dir: &Dir, name: &str, limit: u64) -> Result<Vec<u8>, ScratchRunError> {
    let mut file = open_regular_read_write_nofollow(dir, name)?;
    let metadata = file.metadata()?;
    if metadata.len() > limit {
        return Err(ScratchRunError::UnsafeEntry(format!(
            "scratch file {name:?} exceeds its bound"
        )));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn ensure_opened_regular(file: &fs::File, name: &str) -> Result<(), ScratchRunError> {
    let metadata = file.metadata()?;
    #[cfg(windows)]
    {
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(ScratchRunError::UnsafeEntry(format!(
                "{name:?} is a reparse point"
            )));
        }
    }
    if !metadata.is_file() {
        return Err(ScratchRunError::UnsafeEntry(format!(
            "{name:?} is not a regular file"
        )));
    }
    Ok(())
}

fn require_real_directory(
    entry: &cap_std::fs::DirEntry,
    name: &str,
) -> Result<(), ScratchRunError> {
    let file_type = entry.file_type()?;
    if file_type.is_symlink() || !file_type.is_dir() {
        return Err(ScratchRunError::UnsafeEntry(format!(
            "{name:?} is not a real directory"
        )));
    }
    #[cfg(windows)]
    {
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        if entry.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(ScratchRunError::UnsafeEntry(format!(
                "{name:?} is a reparse point"
            )));
        }
    }
    Ok(())
}

fn require_regular_entry(entry: &cap_std::fs::DirEntry, name: &str) -> Result<(), ScratchRunError> {
    let file_type = entry.file_type()?;
    if file_type.is_symlink() || !file_type.is_file() {
        return Err(ScratchRunError::UnsafeEntry(format!(
            "{name:?} is not a regular file"
        )));
    }
    #[cfg(windows)]
    {
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        if entry.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(ScratchRunError::UnsafeEntry(format!(
                "{name:?} is a reparse point"
            )));
        }
    }
    Ok(())
}

fn validate_lsm_root<Tag>(root: &ScratchLsmRoot<Tag>) -> Result<(), ScratchRunError> {
    if root.levels.len() != SCRATCH_LSM_LEVELS {
        return Err(ScratchRunError::MalformedPage);
    }
    for segment in root.levels.iter().flatten() {
        if segment.generation == 0
            || segment.generation > root.next_generation
            || segment.entry_count == 0
        {
            return Err(ScratchRunError::MalformedPage);
        }
    }
    Ok(())
}

fn validate_segment<Tag>(segment: &ScratchSegment<Tag>) -> Result<(), ScratchRunError> {
    if segment.schema_version != SCRATCH_PAGE_SCHEMA_VERSION
        || segment.generation == 0
        || segment.entries.is_empty()
    {
        return Err(ScratchRunError::MalformedPage);
    }
    let mut previous: Option<&[u8]> = None;
    for record in &segment.entries {
        if record.key.is_empty()
            || previous.is_some_and(|previous| previous >= record.key.as_slice())
        {
            return Err(ScratchRunError::MalformedPage);
        }
        previous = Some(&record.key);
    }
    Ok(())
}

fn encode_page_canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, ScratchRunError> {
    postcard::to_allocvec(value).map_err(|_| ScratchRunError::MalformedPage)
}

fn decode_page_canonical<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
) -> Result<T, ScratchRunError> {
    let value: T = postcard::from_bytes(bytes).map_err(|_| ScratchRunError::MalformedPage)?;
    if encode_page_canonical(&value)? != bytes {
        return Err(ScratchRunError::MalformedPage);
    }
    Ok(value)
}

fn encode_canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, ScratchRunError> {
    postcard::to_allocvec(value).map_err(|_| ScratchRunError::MalformedEncoding)
}

fn decode_canonical<T: DeserializeOwned + Serialize>(bytes: &[u8]) -> Result<T, ScratchRunError> {
    let value: T = postcard::from_bytes(bytes).map_err(|_| ScratchRunError::MalformedEncoding)?;
    if encode_canonical(&value)? != bytes {
        return Err(ScratchRunError::MalformedEncoding);
    }
    Ok(value)
}

#[cfg(unix)]
fn lock_exclusive_nonblocking(file: &fs::File) -> Result<bool, ScratchRunError> {
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
    let _ = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
}

#[cfg(windows)]
fn lock_exclusive_nonblocking(file: &fs::File) -> Result<bool, ScratchRunError> {
    use windows_sys::Win32::Foundation::{ERROR_LOCK_VIOLATION, FALSE};
    use windows_sys::Win32::Storage::FileSystem::{
        LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
    };
    let mut overlapped = unsafe { std::mem::zeroed() };
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

#[cfg(test)]
mod tests {
    use super::*;
    use cap_std::ambient_authority;
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Path, PathBuf};

    #[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
    #[serde(transparent)]
    struct TestOwner(Uuid);

    #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
    enum TestPageKind {
        Primary,
        Other,
    }

    impl ScratchPageTag for TestPageKind {
        fn saturation_tag() -> Self {
            Self::Other
        }
    }

    fn scratch_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("tine-storage-scratch-{label}-{}", Uuid::new_v4()))
    }

    fn archive(root: &Path) -> Dir {
        fs::create_dir_all(root).unwrap();
        Dir::open_ambient_dir(root, ambient_authority()).unwrap()
    }

    fn run_path(root: &Path, run_id: Uuid) -> PathBuf {
        root.join(SCRATCH_DIR).join(format!("run-{run_id}"))
    }

    fn run_snapshot(root: &Path, run_id: Uuid) -> BTreeMap<&'static str, Vec<u8>> {
        let run = run_path(root, run_id);
        [
            SCRATCH_MARKER_FILE,
            SCRATCH_LEASE_FILE,
            SCRATCH_PAGES_FILE,
            SCRATCH_BLOBS_FILE,
        ]
        .into_iter()
        .map(|name| (name, fs::read(run.join(name)).unwrap()))
        .collect()
    }

    fn namespace_names(root: &Path) -> BTreeSet<String> {
        fs::read_dir(root.join(SCRATCH_DIR))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect()
    }

    /// Exact bytes produced by the pre-extraction schema-13 marker codec for:
    /// schema 13, owner UUID bytes 00..0f, run UUID bytes 10..1f, retained,
    /// and owner nonce bytes 20..3f.
    const PRE_EXTRACTION_SCHEMA_13_MARKER: [u8; 68] = [
        0x0d, 0x10, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
        0x0d, 0x0e, 0x0f, 0x10, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a,
        0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x01, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28,
        0x29, 0x2a, 0x2b, 0x2c, 0x2d, 0x2e, 0x2f, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37,
        0x38, 0x39, 0x3a, 0x3b, 0x3c, 0x3d, 0x3e, 0x3f,
    ];

    #[test]
    fn pre_extraction_schema_13_run_reopens_and_clones_byte_exactly() {
        let source_root = scratch_root("schema-13-source");
        let destination_root = scratch_root("schema-13-destination");
        let source = archive(&source_root);
        let destination = archive(&destination_root);
        let owner = TestOwner(Uuid::from_bytes([
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f,
        ]));
        let run_id = Uuid::from_bytes([
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d,
            0x1e, 0x1f,
        ]);
        let run = run_path(&source_root, run_id);
        fs::create_dir_all(&run).unwrap();
        fs::write(
            run.join(SCRATCH_MARKER_FILE),
            PRE_EXTRACTION_SCHEMA_13_MARKER,
        )
        .unwrap();
        fs::write(run.join(SCRATCH_LEASE_FILE), []).unwrap();
        fs::write(run.join(SCRATCH_PAGES_FILE), b"existing page extent").unwrap();
        fs::write(run.join(SCRATCH_BLOBS_FILE), b"existing blob extent").unwrap();
        let baseline = run_snapshot(&source_root, run_id);

        let adopted = ScratchRun::adopt_retained(&source, owner.clone(), run_id).unwrap();
        assert_eq!(
            adopted.binding_digest().unwrap(),
            ContentDigest::of(&PRE_EXTRACTION_SCHEMA_13_MARKER)
        );
        assert_eq!(adopted.retention(), ScratchRetention::Retained);
        let cloned = adopted.clone_retained_into(&destination).unwrap();
        assert_eq!(run_snapshot(&source_root, run_id), baseline);
        assert_eq!(run_snapshot(&destination_root, run_id), baseline);
        assert_eq!(
            namespace_names(&source_root),
            BTreeSet::from([format!("run-{run_id}")])
        );
        assert_eq!(
            baseline.keys().copied().collect::<BTreeSet<_>>(),
            BTreeSet::from(["blobs.data", "lease", "marker", "pages.index",])
        );
        drop(cloned);
        drop(adopted);

        let reopened = ScratchRun::adopt_retained(&source, owner, run_id).unwrap();
        assert_eq!(run_snapshot(&source_root, run_id), baseline);
        drop(reopened);
        fs::remove_dir_all(source_root).unwrap();
        fs::remove_dir_all(destination_root).unwrap();
    }

    #[test]
    fn append_only_scratch_batches_physical_writes_and_reopens_exactly() {
        let root = scratch_root("batched-appends");
        let archive = archive(&root);
        let owner = TestOwner(Uuid::from_u128(0xba7c_0001));
        let run = ScratchRun::create_retained(&archive, owner.clone()).unwrap();
        let run_id = run.run_id();
        let mut first = None;
        let mut last = None;
        for index in 0_u64..4_096 {
            let page = run
                .append_page(
                    TestPageKind::Primary,
                    index.to_be_bytes().to_vec(),
                    index.to_be_bytes().to_vec(),
                    &index,
                )
                .unwrap();
            let blob = run.append_blob(&index.to_be_bytes()).unwrap();
            first.get_or_insert((page.clone(), blob.clone(), index));
            last = Some((page, blob, index));
        }

        run.with_pages(|_| ()).unwrap();
        run.with_blobs(|_| ()).unwrap();
        let stats = run.operation_stats();
        assert_eq!(stats.page_writes, 4_096);
        assert_eq!(stats.blob_writes, 4_096);
        assert!(stats.page_append_batches < 32, "{stats:?}");
        assert!(stats.blob_append_batches < 32, "{stats:?}");
        drop(run);

        let reopened = ScratchRun::adopt_retained(&archive, owner, run_id).unwrap();
        for (page, blob, expected) in [first.unwrap(), last.unwrap()] {
            assert_eq!(
                reopened
                    .read_page::<_, u64>(&page, TestPageKind::Primary)
                    .unwrap(),
                expected
            );
            assert_eq!(reopened.read_blob(&blob).unwrap(), expected.to_be_bytes());
        }
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn page_and_blob_refusal_is_fail_closed_and_read_only() {
        let root = scratch_root("data-refusal");
        let archive = archive(&root);
        let run =
            ScratchRun::create_retained(&archive, TestOwner(Uuid::from_u128(0xfeed))).unwrap();
        let run_id = run.run_id();
        let page_ref = run
            .append_page(TestPageKind::Primary, b"a".to_vec(), b"z".to_vec(), &7_u64)
            .unwrap();
        let blob_ref = run.append_blob(b"authenticated blob").unwrap();
        run.with_pages(|_| ()).unwrap();
        run.with_blobs(|_| ()).unwrap();

        let assert_unchanged = |before: &BTreeMap<&'static str, Vec<u8>>| {
            assert_eq!(run_snapshot(&root, run_id), *before);
        };
        let baseline = run_snapshot(&root, run_id);

        assert_eq!(
            run.append_page(TestPageKind::Primary, Vec::new(), b"z".to_vec(), &7_u64),
            Err(ScratchRunError::MalformedPage)
        );
        assert_eq!(run.append_blob(&[]), Err(ScratchRunError::MalformedBlob));
        assert_unchanged(&baseline);

        let mut wrong_kind = page_ref.clone();
        wrong_kind.kind = TestPageKind::Other;
        assert_eq!(
            run.read_page::<_, u64>(&wrong_kind, TestPageKind::Primary),
            Err(ScratchRunError::PageBindingMismatch)
        );
        assert_unchanged(&baseline);

        let mut wrong_page_digest = page_ref.clone();
        wrong_page_digest.digest = ContentDigest::of(b"wrong page");
        assert!(matches!(
            run.read_page::<_, u64>(&wrong_page_digest, TestPageKind::Primary),
            Err(ScratchRunError::PageDigestMismatch(_))
        ));
        assert_unchanged(&baseline);

        let mut wrong_page_length = page_ref.clone();
        wrong_page_length.encoded_len = 0;
        assert_eq!(
            run.read_page::<_, u64>(&wrong_page_length, TestPageKind::Primary),
            Err(ScratchRunError::MalformedPage)
        );
        assert_unchanged(&baseline);

        let mut wrong_page_offset = page_ref.clone();
        wrong_page_offset.offset = u64::MAX;
        assert!(matches!(
            run.read_page::<_, u64>(&wrong_page_offset, TestPageKind::Primary),
            Err(ScratchRunError::Io(_)) | Err(ScratchRunError::MalformedPage)
        ));
        assert_unchanged(&baseline);

        let mut wrong_page_range = page_ref.clone();
        wrong_page_range.key_min = b"b".to_vec();
        assert_eq!(
            run.read_page::<_, u64>(&wrong_page_range, TestPageKind::Primary),
            Err(ScratchRunError::PageBindingMismatch)
        );
        assert_unchanged(&baseline);

        let noncanonical_payload = vec![0x80, 0x00];
        let noncanonical_envelope = ScratchPageEnvelope {
            schema_version: SCRATCH_PAGE_SCHEMA_VERSION,
            kind: TestPageKind::Primary,
            key_min: b"n".to_vec(),
            key_max: b"n".to_vec(),
            payload: noncanonical_payload,
        };
        let noncanonical_bytes = encode_page_canonical(&noncanonical_envelope).unwrap();
        let noncanonical_ref = run
            .with_pages(|pages| -> Result<_, ScratchRunError> {
                let offset = pages.seek(SeekFrom::End(0))?;
                pages.write_all(&noncanonical_bytes)?;
                Ok(ScratchPageRef {
                    offset,
                    encoded_len: noncanonical_bytes.len() as u32,
                    digest: ContentDigest::of(&noncanonical_bytes),
                    kind: TestPageKind::Primary,
                    key_min: b"n".to_vec(),
                    key_max: b"n".to_vec(),
                })
            })
            .unwrap()
            .unwrap();
        let before_noncanonical_read = run_snapshot(&root, run_id);
        assert_eq!(
            run.read_page::<_, u64>(&noncanonical_ref, TestPageKind::Primary),
            Err(ScratchRunError::MalformedPage)
        );
        assert_unchanged(&before_noncanonical_read);

        let mut wrong_blob_digest = blob_ref.clone();
        wrong_blob_digest.digest = ContentDigest::of(b"wrong blob");
        assert!(matches!(
            run.read_blob(&wrong_blob_digest),
            Err(ScratchRunError::BlobDigestMismatch(_))
        ));
        assert_unchanged(&before_noncanonical_read);

        let mut wrong_blob_length = blob_ref.clone();
        wrong_blob_length.encoded_len = 0;
        assert_eq!(
            run.read_blob(&wrong_blob_length),
            Err(ScratchRunError::MalformedBlob)
        );
        assert_unchanged(&before_noncanonical_read);

        let mut wrong_blob_offset = blob_ref.clone();
        wrong_blob_offset.offset = u64::MAX;
        assert!(matches!(
            run.read_blob(&wrong_blob_offset),
            Err(ScratchRunError::Io(_)) | Err(ScratchRunError::MalformedBlob)
        ));
        assert_unchanged(&before_noncanonical_read);

        run.with_blobs(|blobs| blobs.set_len(blob_ref.encoded_len as u64 - 1))
            .unwrap()
            .unwrap();
        let truncated = run_snapshot(&root, run_id);
        assert_eq!(
            run.read_blob(&blob_ref),
            Err(ScratchRunError::MalformedBlob)
        );
        assert_unchanged(&truncated);

        drop(run);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bounded_lsm_preserves_merge_range_batch_and_carry_semantics() {
        let root = scratch_root("bounded-lsm");
        let archive = archive(&root);
        let run =
            ScratchRun::create_ephemeral(&archive, TestOwner(Uuid::from_u128(0xcafe))).unwrap();
        let original = (0_u8..64)
            .map(|index| {
                (
                    format!("key-{index:02}").into_bytes(),
                    Some(format!("old-{index:02}").into_bytes()),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut lsm = run
            .insert_many(&ScratchLsmRoot::default(), TestPageKind::Primary, &original)
            .unwrap();
        lsm = run
            .insert_many(
                &lsm,
                TestPageKind::Primary,
                &BTreeMap::from([
                    (b"key-00".to_vec(), Some(b"new-00".to_vec())),
                    (b"key-32".to_vec(), None),
                ]),
            )
            .unwrap();
        assert_eq!(
            run.lookup(&lsm, TestPageKind::Primary, b"key-00").unwrap(),
            Some(b"new-00".to_vec())
        );
        assert_eq!(
            run.lookup(&lsm, TestPageKind::Primary, b"key-32").unwrap(),
            None
        );
        assert_eq!(
            run.scan_prefix(&lsm, TestPageKind::Primary, b"key-0")
                .unwrap()
                .first(),
            Some(&(b"key-00".to_vec(), b"new-00".to_vec()))
        );
        let keys = (0_u8..64)
            .map(|index| format!("key-{index:02}").into_bytes())
            .chain([b"absent".to_vec(), b"key-00".to_vec()])
            .collect::<Vec<_>>();
        let expected = keys
            .iter()
            .map(|key| run.lookup(&lsm, TestPageKind::Primary, key).unwrap())
            .collect::<Vec<_>>();
        let before = run.operation_stats();
        let batched = run.lookup_many(&lsm, TestPageKind::Primary, &keys).unwrap();
        let after = run.operation_stats();
        assert_eq!(batched, expected);
        assert!(after.page_reads - before.page_reads <= lsm.levels.iter().flatten().count());

        let mut carry = ScratchLsmRoot::default();
        for index in 0_u64..31 {
            carry = run
                .insert_many(
                    &carry,
                    TestPageKind::Other,
                    &BTreeMap::from([(
                        index.to_be_bytes().to_vec(),
                        Some(index.to_be_bytes().to_vec()),
                    )]),
                )
                .unwrap();
        }
        let before = run.operation_stats();
        carry = run
            .insert_many(
                &carry,
                TestPageKind::Other,
                &BTreeMap::from([(
                    31_u64.to_be_bytes().to_vec(),
                    Some(31_u64.to_be_bytes().to_vec()),
                )]),
            )
            .unwrap();
        let after = run.operation_stats();
        let reads = after.page_reads - before.page_reads;
        let writes = after.page_writes - before.page_writes;
        let bytes = (after.page_bytes_read - before.page_bytes_read)
            + (after.page_bytes_written - before.page_bytes_written);
        assert_eq!(reads, 5);
        assert_eq!(writes, 1);
        assert!(reads + writes <= SCRATCH_LSM_LEVELS + 1);
        assert!(bytes <= (SCRATCH_LSM_LEVELS + 1) * MAX_SCRATCH_PAGE_BYTES);
        assert_eq!(
            run.materialize(&carry, TestPageKind::Other).unwrap().len(),
            32
        );

        drop(run);
        assert!(!root.join(SCRATCH_DIR).exists() || namespace_names(&root).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lookup_sessions_preserve_semantics_and_bound_cross_call_segment_residency() {
        let root = scratch_root("lookup-session-bounds");
        let archive = archive(&root);
        let run = ScratchRun::create_ephemeral(&archive, TestOwner(Uuid::from_u128(0x51))).unwrap();
        let original = (0_u16..192)
            .map(|index| {
                (
                    format!("key-{index:03}").into_bytes(),
                    Some(format!("old-{index:03}").into_bytes()),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut lsm = run
            .insert_many(&ScratchLsmRoot::default(), TestPageKind::Primary, &original)
            .unwrap();
        lsm = run
            .insert_many(
                &lsm,
                TestPageKind::Primary,
                &BTreeMap::from([
                    (b"key-000".to_vec(), Some(b"new-000".to_vec())),
                    (b"key-032".to_vec(), None),
                ]),
            )
            .unwrap();
        lsm = run
            .insert_many(
                &lsm,
                TestPageKind::Primary,
                &BTreeMap::from([(b"key-096".to_vec(), Some(b"new-096".to_vec()))]),
            )
            .unwrap();

        let semantic_keys = vec![
            b"key-000".to_vec(),
            b"key-032".to_vec(),
            b"key-096".to_vec(),
            b"key-096".to_vec(),
            b"missing".to_vec(),
        ];
        let ordinary = run
            .lookup_many(&lsm, TestPageKind::Primary, &semantic_keys)
            .unwrap();
        let mut zero = run.lookup_session(&lsm, TestPageKind::Primary, 0).unwrap();
        let zero_values = run
            .lookup_many_with_session_and_absence_policy(
                &mut zero,
                &lsm,
                TestPageKind::Primary,
                &semantic_keys,
                || Ok(vec![false; semantic_keys.len()]),
            )
            .unwrap();
        let mut fitting = run
            .lookup_session(&lsm, TestPageKind::Primary, usize::MAX)
            .unwrap();
        let fitting_values = run
            .lookup_many_with_session_and_absence_policy(
                &mut fitting,
                &lsm,
                TestPageKind::Primary,
                &semantic_keys,
                || Ok(vec![false; semantic_keys.len()]),
            )
            .unwrap();
        assert_eq!(zero_values, ordinary);
        assert_eq!(fitting_values, ordinary);
        assert_eq!(ordinary[0], Some(b"new-000".to_vec()));
        assert_eq!(ordinary[1], None);
        assert_eq!(ordinary[2], Some(b"new-096".to_vec()));
        assert_eq!(ordinary[2], ordinary[3]);
        assert_eq!(ordinary[4], None);

        let mut fitting = run
            .lookup_session(&lsm, TestPageKind::Primary, usize::MAX)
            .unwrap();
        let before = run.operation_stats();
        let mut session_values = Vec::new();
        for chunk in (0_u16..192)
            .map(|index| format!("key-{index:03}").into_bytes())
            .collect::<Vec<_>>()
            .chunks(64)
        {
            session_values.extend(
                run.lookup_many_with_session_and_absence_policy(
                    &mut fitting,
                    &lsm,
                    TestPageKind::Primary,
                    chunk,
                    || Ok(vec![false; chunk.len()]),
                )
                .unwrap(),
            );
        }
        let after = run.operation_stats();
        let all_keys = (0_u16..192)
            .map(|index| format!("key-{index:03}").into_bytes())
            .collect::<Vec<_>>();
        assert_eq!(
            session_values,
            run.lookup_many(&lsm, TestPageKind::Primary, &all_keys)
                .unwrap()
        );
        let stats = fitting.stats();
        assert_eq!(stats.misses, 2);
        assert_eq!(stats.hits, 2);
        assert_eq!(after.page_reads - before.page_reads, 2);
        assert_eq!(stats.evictions, 0);
        assert_eq!(stats.oversize, 0);
        assert!(stats.resident_bytes > 0);
        assert_eq!(stats.peak_resident_bytes, stats.resident_bytes);

        let segment_refs = lsm.levels.iter().flatten().collect::<Vec<_>>();
        assert_eq!(segment_refs.len(), 2);
        let charges = segment_refs
            .iter()
            .map(|segment_ref| {
                let segment = run
                    .read_segment(TestPageKind::Primary, segment_ref)
                    .unwrap();
                decoded_segment_charge(segment_ref, &segment)
            })
            .collect::<Vec<_>>();
        let one_segment_budget = *charges.iter().max().unwrap();
        let mut evicting = run
            .lookup_session(&lsm, TestPageKind::Primary, one_segment_budget)
            .unwrap();
        let overlap = vec![b"key-095".to_vec(), b"key-096".to_vec()];
        for _ in 0..2 {
            run.lookup_many_with_session_and_absence_policy(
                &mut evicting,
                &lsm,
                TestPageKind::Primary,
                &overlap,
                || Ok(vec![false; overlap.len()]),
            )
            .unwrap();
        }
        let stats = evicting.stats();
        assert!(stats.evictions >= 2);
        assert!(stats.resident_bytes <= one_segment_budget);
        assert!(stats.peak_resident_bytes <= one_segment_budget);

        let newer_ref = lsm.levels[0].as_ref().unwrap();
        let newer = run.read_segment(TestPageKind::Primary, newer_ref).unwrap();
        let oversize_budget = decoded_segment_charge(newer_ref, &newer) - 1;
        let mut oversize = run
            .lookup_session(&lsm, TestPageKind::Primary, oversize_budget)
            .unwrap();
        let key = vec![b"key-096".to_vec()];
        run.lookup_many_with_session_and_absence_policy(
            &mut oversize,
            &lsm,
            TestPageKind::Primary,
            &key,
            || Ok(vec![false]),
        )
        .unwrap();
        let stats = oversize.stats();
        assert_eq!(stats.oversize, 1);
        assert_eq!(stats.resident_bytes, 0);
        assert_eq!(stats.peak_resident_bytes, 0);
        assert_eq!(zero.stats().resident_bytes, 0);
        assert!(zero.stats().oversize > 0);

        drop(run);
        assert!(!root.join(SCRATCH_DIR).exists() || namespace_names(&root).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lookup_sessions_fail_closed_before_admission_and_snapshot_only_authenticated_bytes() {
        let tamper_root = scratch_root("lookup-session-tamper-before");
        let tamper_archive = archive(&tamper_root);
        let run = ScratchRun::create_ephemeral(&tamper_archive, TestOwner(Uuid::from_u128(0x52)))
            .unwrap();
        let lsm = run
            .insert_many(
                &ScratchLsmRoot::default(),
                TestPageKind::Primary,
                &BTreeMap::from([(b"key".to_vec(), Some(b"value".to_vec()))]),
            )
            .unwrap();
        let segment_ref = lsm.levels[0].as_ref().unwrap();
        let mut session = run
            .lookup_session(&lsm, TestPageKind::Primary, usize::MAX)
            .unwrap();
        run.with_pages(|pages| {
            pages.seek(SeekFrom::Start(segment_ref.page_ref.offset))?;
            let mut byte = [0_u8; 1];
            pages.read_exact(&mut byte)?;
            byte[0] ^= 0x80;
            pages.seek(SeekFrom::Start(segment_ref.page_ref.offset))?;
            pages.write_all(&byte)
        })
        .unwrap()
        .unwrap();
        let result = run.lookup_many_with_session_and_absence_policy(
            &mut session,
            &lsm,
            TestPageKind::Primary,
            &[b"key".to_vec()],
            || Ok(vec![false]),
        );
        assert!(matches!(
            result,
            Err(ScratchRunError::PageDigestMismatch(_))
        ));
        assert_eq!(session.stats().misses, 1);
        assert_eq!(session.stats().resident_bytes, 0);
        drop(run);
        fs::remove_dir_all(tamper_root).unwrap();

        let truncate_root = scratch_root("lookup-session-truncate-before");
        let truncate_archive = archive(&truncate_root);
        let run = ScratchRun::create_ephemeral(&truncate_archive, TestOwner(Uuid::from_u128(0x53)))
            .unwrap();
        let lsm = run
            .insert_many(
                &ScratchLsmRoot::default(),
                TestPageKind::Primary,
                &BTreeMap::from([(b"key".to_vec(), Some(b"value".to_vec()))]),
            )
            .unwrap();
        let mut session = run
            .lookup_session(&lsm, TestPageKind::Primary, usize::MAX)
            .unwrap();
        run.with_pages(|pages| pages.set_len(0)).unwrap().unwrap();
        let result = run.lookup_many_with_session_and_absence_policy(
            &mut session,
            &lsm,
            TestPageKind::Primary,
            &[b"key".to_vec()],
            || Ok(vec![false]),
        );
        assert_eq!(result, Err(ScratchRunError::MalformedPage));
        assert_eq!(session.stats().resident_bytes, 0);
        drop(run);
        fs::remove_dir_all(truncate_root).unwrap();

        let uncached_root = scratch_root("lookup-session-tamper-uncached");
        let uncached_archive = archive(&uncached_root);
        let run = ScratchRun::create_ephemeral(&uncached_archive, TestOwner(Uuid::from_u128(0x54)))
            .unwrap();
        let mut lsm = run
            .insert_many(
                &ScratchLsmRoot::default(),
                TestPageKind::Primary,
                &BTreeMap::from([(b"a".to_vec(), Some(b"one".to_vec()))]),
            )
            .unwrap();
        lsm = run
            .insert_many(
                &lsm,
                TestPageKind::Primary,
                &BTreeMap::from([(b"b".to_vec(), Some(b"two".to_vec()))]),
            )
            .unwrap();
        lsm = run
            .insert_many(
                &lsm,
                TestPageKind::Primary,
                &BTreeMap::from([(b"z".to_vec(), Some(b"three".to_vec()))]),
            )
            .unwrap();
        let mut session = run
            .lookup_session(&lsm, TestPageKind::Primary, usize::MAX)
            .unwrap();
        run.lookup_many_with_session_and_absence_policy(
            &mut session,
            &lsm,
            TestPageKind::Primary,
            &[b"a".to_vec()],
            || Ok(vec![false]),
        )
        .unwrap();
        let resident_before = session.stats().resident_bytes;
        let uncached_ref = lsm.levels[0].as_ref().unwrap();
        run.with_pages(|pages| {
            pages.seek(SeekFrom::Start(uncached_ref.page_ref.offset))?;
            let mut byte = [0_u8; 1];
            pages.read_exact(&mut byte)?;
            byte[0] ^= 0x40;
            pages.seek(SeekFrom::Start(uncached_ref.page_ref.offset))?;
            pages.write_all(&byte)
        })
        .unwrap()
        .unwrap();
        let result = run.lookup_many_with_session_and_absence_policy(
            &mut session,
            &lsm,
            TestPageKind::Primary,
            &[b"z".to_vec()],
            || Ok(vec![false]),
        );
        assert!(matches!(
            result,
            Err(ScratchRunError::PageDigestMismatch(_))
        ));
        assert_eq!(session.stats().resident_bytes, resident_before);
        drop(run);
        fs::remove_dir_all(uncached_root).unwrap();

        let cached_root = scratch_root("lookup-session-tamper-cached");
        let cached_archive = archive(&cached_root);
        let run = ScratchRun::create_ephemeral(&cached_archive, TestOwner(Uuid::from_u128(0x55)))
            .unwrap();
        let lsm = run
            .insert_many(
                &ScratchLsmRoot::default(),
                TestPageKind::Primary,
                &BTreeMap::from([(b"key".to_vec(), Some(b"value".to_vec()))]),
            )
            .unwrap();
        let mut session = run
            .lookup_session(&lsm, TestPageKind::Primary, usize::MAX)
            .unwrap();
        let lookup = |session: &mut ScratchLookupSession<TestPageKind>| {
            run.lookup_many_with_session_and_absence_policy(
                session,
                &lsm,
                TestPageKind::Primary,
                &[b"key".to_vec()],
                || Ok(vec![false]),
            )
        };
        assert_eq!(lookup(&mut session).unwrap(), vec![Some(b"value".to_vec())]);
        let cached_ref = lsm.levels[0].as_ref().unwrap();
        run.with_pages(|pages| {
            pages.seek(SeekFrom::Start(cached_ref.page_ref.offset))?;
            let mut byte = [0_u8; 1];
            pages.read_exact(&mut byte)?;
            byte[0] ^= 0x20;
            pages.seek(SeekFrom::Start(cached_ref.page_ref.offset))?;
            pages.write_all(&byte)
        })
        .unwrap()
        .unwrap();
        assert_eq!(lookup(&mut session).unwrap(), vec![Some(b"value".to_vec())]);
        assert_eq!(session.stats().hits, 1);
        let mut fresh = run
            .lookup_session(&lsm, TestPageKind::Primary, usize::MAX)
            .unwrap();
        assert!(matches!(
            lookup(&mut fresh),
            Err(ScratchRunError::PageDigestMismatch(_))
        ));
        assert_eq!(fresh.stats().resident_bytes, 0);
        drop(run);
        fs::remove_dir_all(cached_root).unwrap();
    }

    #[test]
    fn lookup_session_rejects_run_root_and_kind_rebinding() {
        let root = scratch_root("lookup-session-binding");
        let archive = archive(&root);
        let run = ScratchRun::create_ephemeral(&archive, TestOwner(Uuid::from_u128(0x56))).unwrap();
        let other_run =
            ScratchRun::create_ephemeral(&archive, TestOwner(Uuid::from_u128(0x56))).unwrap();
        let lsm = run
            .insert_many(
                &ScratchLsmRoot::default(),
                TestPageKind::Primary,
                &BTreeMap::from([(b"key".to_vec(), Some(b"value".to_vec()))]),
            )
            .unwrap();
        let rebound_root = run
            .insert_many(
                &lsm,
                TestPageKind::Primary,
                &BTreeMap::from([(b"other".to_vec(), Some(b"value".to_vec()))]),
            )
            .unwrap();
        let mut session = run
            .lookup_session(&lsm, TestPageKind::Primary, usize::MAX)
            .unwrap();
        let keys = [b"key".to_vec()];
        assert_eq!(
            other_run.lookup_many_with_session_and_absence_policy(
                &mut session,
                &lsm,
                TestPageKind::Primary,
                &keys,
                || Ok(vec![false]),
            ),
            Err(ScratchRunError::PageBindingMismatch)
        );
        assert_eq!(
            run.lookup_many_with_session_and_absence_policy(
                &mut session,
                &rebound_root,
                TestPageKind::Primary,
                &keys,
                || Ok(vec![false]),
            ),
            Err(ScratchRunError::PageBindingMismatch)
        );
        assert_eq!(
            run.lookup_many_with_session_and_absence_policy(
                &mut session,
                &lsm,
                TestPageKind::Other,
                &keys,
                || Ok(vec![false]),
            ),
            Err(ScratchRunError::PageBindingMismatch)
        );
        assert_eq!(session.stats(), ScratchLookupSessionStats::default());
        drop(other_run);
        drop(run);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn census_is_read_only_and_reclamation_deletes_only_proved_free_orphans() {
        let root = scratch_root("census-reclamation");
        let archive = archive(&root);
        let owner = TestOwner(Uuid::from_u128(1));
        let foreign_owner = TestOwner(Uuid::from_u128(2));

        let reachable = ScratchRun::create_retained(&archive, owner.clone()).unwrap();
        let reachable_id = reachable.run_id();
        drop(reachable);
        let orphan = ScratchRun::create_retained(&archive, owner.clone()).unwrap();
        let orphan_id = orphan.run_id();
        drop(orphan);
        let live = ScratchRun::create_retained(&archive, owner.clone()).unwrap();
        let live_id = live.run_id();
        let ephemeral = ScratchRun::create_ephemeral(&archive, owner.clone()).unwrap();
        let ephemeral_id = ephemeral.run_id();
        let foreign = ScratchRun::create_retained(&archive, foreign_owner).unwrap();
        let foreign_id = foreign.run_id();
        drop(foreign);
        let conflict = root
            .join(SCRATCH_DIR)
            .join(format!("run-{reachable_id} (1)"));
        fs::create_dir(&conflict).unwrap();
        fs::write(conflict.join(SCRATCH_MARKER_FILE), b"conflict copy").unwrap();

        let before = [reachable_id, orphan_id, live_id, ephemeral_id, foreign_id]
            .map(|run_id| (run_id, run_snapshot(&root, run_id)));
        let census = census_retained_runs(&archive, &owner).unwrap();
        assert_eq!(
            census,
            RetainedRunCensus {
                retained: 3,
                ephemeral: 1,
                unclassified: 2,
            }
        );
        for (run_id, bytes) in &before {
            assert_eq!(run_snapshot(&root, *run_id), *bytes);
        }
        assert_eq!(
            fs::read(conflict.join(SCRATCH_MARKER_FILE)).unwrap(),
            b"conflict copy"
        );

        // SAFETY: this synthetic test enumerated the complete authoritative
        // reachable set above; exactly `reachable_id` is reachable.
        let outcome = unsafe {
            reclaim_unreachable_retained_runs(&archive, &owner, |run_id| run_id == reachable_id)
        }
        .unwrap();
        assert_eq!(
            outcome,
            RetainedRunReclamation {
                retained_reachable: 1,
                retained_reclaimed: 1,
                retained_live_skipped: 1,
                ephemeral_preserved: 1,
                unclassified_preserved: 2,
            }
        );
        assert!(!run_path(&root, orphan_id).exists());
        for run_id in [reachable_id, live_id, ephemeral_id, foreign_id] {
            let baseline = before.iter().find(|(id, _)| *id == run_id).unwrap();
            assert_eq!(run_snapshot(&root, run_id), baseline.1);
        }
        assert_eq!(
            fs::read(conflict.join(SCRATCH_MARKER_FILE)).unwrap(),
            b"conflict copy"
        );

        drop(ephemeral);
        drop(live);
        fs::remove_dir_all(root).unwrap();
    }
}
